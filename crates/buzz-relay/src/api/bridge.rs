//! Nostr HTTP bridge — POST /events, /query, /count with NIP-98 auth.
//!
//! These endpoints provide HTTP access to the relay's Nostr protocol,
//! authenticated via NIP-98 signed events.

mod read_state_snapshot;

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use base64::Engine;
use serde_json::Value;

use buzz_auth::{LimitType, Nip98ReplayGuard, NipFiMode, DEFAULT_REPLAY_TTL_SECS};
use buzz_core::TenantContext;

use crate::handlers::ingest::{IngestAuth, IngestError};
use crate::nip_fi_http::{admit_nip_fi_http_on_state, Nip98Proof};
use crate::state::AppState;

use super::{api_error, internal_error, not_found, parse_query_or_400};

#[cfg(test)]
#[path = "bridge/ephemeral_postgres_tests.rs"]
mod ephemeral_postgres_tests;
mod thread_roots;
mod thread_window;

pub(crate) async fn enforce_http_admission(
    state: &AppState,
    tenant: &TenantContext,
    pubkey: &nostr::PublicKey,
) -> Result<(), (StatusCode, Json<Value>)> {
    let limit = state.auth.config().rate_limits.human_api_calls_per_min;
    match crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        tenant,
        pubkey,
        LimitType::ApiCalls,
        60,
        limit,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(crate::admission::AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "http", "reason" => "quota", "bucket" => "api_calls").increment(1);
            Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("rate-limited: quota exceeded; retry in {reset_in_secs}s"),
            ))
        }
        Err(crate::admission::AdmissionError::Unavailable) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "http", "reason" => "unavailable", "bucket" => "api_calls").increment(1);
            Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "rate-limited: shared admission unavailable",
            ))
        }
    }
}

/// Values retained from an already-verified bridge authentication event.
#[derive(Debug)]
pub(crate) struct VerifiedBridgeAuth {
    pub(crate) pubkey: nostr::PublicKey,
    pub(crate) event_id_bytes: [u8; 32],
    pub(crate) signed_created_at: Option<u64>,
}

type BridgeAuthResult = Result<VerifiedBridgeAuth, (StatusCode, Json<Value>)>;

/// Verify bridge auth: NIP-98 (production) or X-Pubkey (dev mode).
///
/// Returns the authenticated public key, an event ID for replay detection, and
/// the verified signed auth timestamp. For X-Pubkey dev mode, the event ID is
/// a zero hash and the timestamp is absent.
///
/// Most callers use [`make_nip98_closure_for_admission`] (admitted surfaces),
/// [`verify_nip98_exempt_invite_claim`] / [`verify_nip98_exempt_operator`]
/// (explicitly-named exempt paths), or the `pub(crate)` form below for
/// git-settings and other crate-local specialized handlers.
pub(crate) fn verify_bridge_auth(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
) -> BridgeAuthResult {
    verify_bridge_auth_with_options(headers, method, url, body, require_auth_token, false)
}

pub(crate) fn verify_bridge_auth_with_options(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
    require_payload: bool,
) -> BridgeAuthResult {
    // Try NIP-98 first (Authorization: Nostr <base64>)
    //
    // Cardinality is enforced at the NIP-FI admission boundary
    // (`admit_nip_fi_http`) for Enforce mode. Off-mode passes
    // through legacy first-value behavior per [FI-INV-15].

    if let Some(auth_str) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Nostr "))
    {
        let event_json = {
            use base64::engine::general_purpose::STANDARD as BASE64;
            let bytes = BASE64
                .decode(auth_str)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid base64 in Nostr auth"))?;
            String::from_utf8(bytes)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid UTF-8 in Nostr auth"))?
        };

        let event: nostr::Event = serde_json::from_str(&event_json)
            .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid NIP-98 event JSON"))?;
        let event_id_bytes = event.id.to_bytes();

        if require_payload
            && !event
                .tags
                .iter()
                .any(|tag| tag.kind() == nostr::TagKind::Payload)
        {
            return Err(api_error(
                StatusCode::UNAUTHORIZED,
                "NIP-98: missing payload tag",
            ));
        }

        let pubkey = buzz_auth::verify_nip98_event(&event_json, url, method, body)
            .map_err(|e| api_error(StatusCode::UNAUTHORIZED, &format!("NIP-98: {e}")))?;

        return Ok(VerifiedBridgeAuth {
            pubkey,
            event_id_bytes,
            signed_created_at: Some(event.created_at.as_secs()),
        });
    }

    // Dev-mode fallback: X-Pubkey header (only when require_auth_token is false)
    if !require_auth_token {
        if let Some(hex_val) = headers.get("x-pubkey").and_then(|v| v.to_str().ok()) {
            let pubkey = nostr::PublicKey::from_hex(hex_val)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid X-Pubkey hex"))?;
            // Zero event ID — no replay detection needed for dev mode
            return Ok(VerifiedBridgeAuth {
                pubkey,
                event_id_bytes: [0u8; 32],
                signed_created_at: None,
            });
        }
    }

    Err(api_error(StatusCode::UNAUTHORIZED, "missing Nostr auth"))
}

// ── NIP-FI Authority boundary ─────────────────────────────────────────────────
//
// The two functions below are the ONLY `pub(crate)` entry points to the raw
// NIP-98 verifier.  All other callers must use one of:
//
//   • `make_nip98_closure_for_admission` — for HTTP surfaces under NIP-FI
//     admission. The closure is passed directly to `admit_nip_fi_http_on_state`
//     and its result is never projected outside a `NipFiAdmission`.
//
//   • `verify_nip98_exempt_invite_claim` / `verify_nip98_exempt_operator` —
//     for the two explicitly NIP-FI-exempt paths that pre-date NIP-FI and must
//     continue to run independently of the NIP-FI state machine.
//
// [FI-TRACE-AUTHORITY-EXEMPT]: grep this tag to audit all exempt call sites.

/// Build a NIP-98 extraction closure suitable for passing directly to
/// [`crate::nip_fi_http::admit_nip_fi_http_on_state`].
///
/// The closure captures all needed parameters by value and, when called,
/// runs the full NIP-98 verification (including optional payload-tag check and
/// X-Pubkey dev-mode fallback) with the same semantics as the private
/// `verify_bridge_auth_with_options`.
///
/// Callers outside `bridge.rs` MUST use this instead of calling the private
/// verifier directly. The pubkey in the closure's result is only accessible
/// through the `NipFiAdmission` produced by `admit_nip_fi_http_on_state` —
/// it cannot be projected without completing the mode-appropriate admission
/// path (pairing and deny-map run only in Enforce).
///
/// [FI-TRACE-AUTHORITY-UNIFORM]
// Response<Body> is intentionally large (axum's design); see nip_fi_http.rs allow blocks.
#[allow(clippy::result_large_err)]
#[allow(clippy::type_complexity)] // The return type IS the admission closure contract; a type alias cannot name impl Trait
pub(crate) fn make_nip98_closure_for_admission(
    headers: HeaderMap,
    method: &'static str,
    url: String,
    body: Option<Vec<u8>>,
    require_auth_token: bool,
    require_payload: bool,
) -> impl FnOnce() -> Result<Nip98Proof<([u8; 32], Option<u64>)>, axum::http::Response<axum::body::Body>>
{
    move || {
        verify_bridge_auth_with_options(
            &headers,
            method,
            &url,
            body.as_deref(),
            require_auth_token,
            require_payload,
        )
        .map(|auth| Nip98Proof::new(auth.pubkey, (auth.event_id_bytes, auth.signed_created_at)))
        .map_err(|e| e.into_response())
    }
}

/// NIP-FI-exempt NIP-98 verifier for the invite-claim path.
///
/// Invite claims run before a tenant's NIP-FI config is consulted and are
/// structurally outside the NIP-FI state machine. This function makes the
/// exemption nameable and greppable. [FI-TRACE-AUTHORITY-EXEMPT]
pub(crate) fn verify_nip98_exempt_invite_claim(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> BridgeAuthResult {
    verify_bridge_auth_with_options(
        headers, method, url, body,
        true, // invite-claim always requires NIP-98; no X-Pubkey dev fallback
        true, // POST bodies must be covered by a payload tag
    )
}

/// NIP-FI-exempt NIP-98 verifier for operator-management endpoints.
///
/// Operator endpoints use a separate auth origin and are structurally outside
/// the per-tenant NIP-FI state machine. [FI-TRACE-AUTHORITY-EXEMPT]
pub(crate) fn verify_nip98_exempt_operator(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> BridgeAuthResult {
    verify_bridge_auth_with_options(
        headers,
        method,
        url,
        body,
        true, // operator endpoints always require NIP-98; no X-Pubkey dev fallback
        body.is_some(),
    )
}

/// Check NIP-98 replay and record the event ID atomically.
///
/// The correctness boundary is the shared, community-scoped Redis seen-set on
/// `AppState`, not process-local memory. Any Redis/guard error fails closed:
/// without the shared `SET NX EX` proof, a stateless worker cannot admit the
/// NIP-98 request safely.
pub(crate) async fn check_nip98_replay(
    state: &AppState,
    tenant: &TenantContext,
    event_id_bytes: [u8; 32],
) -> Result<(), (StatusCode, Json<Value>)> {
    check_nip98_replay_with_guard(state.nip98_replay.as_ref(), tenant, event_id_bytes).await
}

async fn check_nip98_replay_with_guard(
    replay_guard: &dyn Nip98ReplayGuard,
    tenant: &TenantContext,
    event_id_bytes: [u8; 32],
) -> Result<(), (StatusCode, Json<Value>)> {
    // Skip replay detection for dev-mode X-Pubkey auth (zero hash).
    if event_id_bytes == [0u8; 32] {
        return Ok(());
    }

    let event_id = nostr::EventId::from_byte_array(event_id_bytes);
    match replay_guard
        .try_mark(tenant, &event_id, DEFAULT_REPLAY_TTL_SECS)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(api_error(
            StatusCode::UNAUTHORIZED,
            "NIP-98: replay detected",
        )),
        Err(e) => {
            tracing::warn!(
                community = %tenant.community(),
                error = %e,
                "NIP-98 replay guard failed; rejecting request fail-closed"
            );
            Err(api_error(
                StatusCode::UNAUTHORIZED,
                "NIP-98: replay check unavailable",
            ))
        }
    }
}

/// Construct the NIP-98 `u`-tag expected URL for a request bound to `tenant`.
///
/// Conformance row 44 obligation: "NIP-98 `u` URL host must match
/// `req.community`." Host comes from the resolved [`TenantContext`] — the
/// same host the row-zero seam already bound from the request `Host` header —
/// and the scheme comes from the deployment's configured relay URL so
/// `ws`/`wss` deployments map to `http`/`https` consistently with how the
/// client signs the URL it is actually hitting.
///
/// Critically, this does NOT use `config_relay_url`'s host. `config.relay_url`
/// is one static string per deployment; under multi-tenant a relay serves many
/// hosts, only one of which would match. Using it as the URL match key would
/// (a) accept a NIP-98 event signed for community A's host when the request
/// arrives at community B's host (host-binding side door — verify_nip98 would
/// pass and the relay would proceed against the wrong tenant's auth context),
/// and (b) reject every legitimate request whose community host isn't the
/// single configured one. Substituting `tenant.host()` closes both directions.
pub(crate) fn nip98_expected_url(
    config_relay_url: &str,
    tenant: &TenantContext,
    path: &str,
) -> String {
    let scheme = if config_relay_url.trim_start().starts_with("wss://") {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{}{path}", tenant.host())
}

/// Construct the NIP-42 expected `relay` URL for a connection bound to `tenant`.
///
/// NIP-42 (WebSocket AUTH) sibling of [`nip98_expected_url`]. Conformance row 44
/// obligation extends to the WS auth side: the AUTH event's `relay` tag must
/// match the per-tenant host the connection arrived on, not the deployment-wide
/// `config.relay_url`. Same hole the NIP-98 fix closed for HTTP — `config.relay_url`
/// is one static string per deployment, so verifying against it (a) admits an
/// AUTH event signed against community A's host on a connection bound to
/// community B (cross-host token reuse), and (b) rejects every legitimate AUTH
/// whose tenant host isn't the single configured one.
///
/// Scheme is `ws`/`wss` (not `http`/`https`) because the value being matched is
/// the client's connect URL embedded in the signed AUTH event; the helper
/// preserves the deployment's TLS posture from `config_relay_url`'s prefix so
/// `wss://` deployments stay `wss://` and `ws://` dev/test stays `ws://`.
/// Path is empty — clients put the bare WS origin (`ws://host[:port]`) in the
/// `relay` tag, matching how `EventBuilder::auth` accepts a [`nostr::RelayUrl`].
pub(crate) fn nip42_expected_relay_url(config_relay_url: &str, tenant: &TenantContext) -> String {
    let scheme = if config_relay_url.trim_start().starts_with("wss://") {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{}", tenant.host())
}

/// Extract a channel UUID from a single filter's `#h` tag.
fn extract_channel_from_filter(filter: &nostr::Filter) -> Option<uuid::Uuid> {
    let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
    filter.generic_tags.get(&h_tag).and_then(|vs| {
        if vs.len() == 1 {
            vs.iter().next()?.parse::<uuid::Uuid>().ok()
        } else {
            None
        }
    })
}

//
// The CLI injects extension fields (before_id, depth_limit, feed_types) into
// Nostr filter JSON. nostr::Filter silently drops unknown fields during
// deserialization, so we extract them from the raw JSON Value first.

const BRIDGE_FEED_MAX_LIMIT: i64 = 100;
const BRIDGE_THREAD_MAX_LIMIT: u32 = 500;

/// The `before_id` extension field, with "present but malformed" kept distinct
/// from "absent": NIP-CW's cursor grammar says a malformed value MUST reject
/// the request, never silently demote it to a half cursor or a head request.
enum BeforeId {
    Absent,
    Valid(Vec<u8>),
    Malformed,
}

fn extract_before_id(raw: &Value) -> BeforeId {
    let Some(value) = raw.get("before_id") else {
        return BeforeId::Absent;
    };
    match value
        .as_str()
        .filter(|hex_str| hex_str.len() == 64)
        .and_then(|hex_str| hex::decode(hex_str).ok())
    {
        Some(id) => BeforeId::Valid(id),
        None => BeforeId::Malformed,
    }
}

/// The `consistency` extension field: a read-your-writes opt-in. A
/// write-influencing read (a canvas save's head precondition or its post-write
/// ancestry verification) sets `"consistency": "strong"` so the relay serves it
/// from the writer pool, never a replica that may lag behind the caller's own
/// just-accepted write. Absent = the default routed path (replica-eligible when
/// `BUZZ_REPLICA_READ_MAX_AGE_MS` is set).
///
/// This only ever forces the *writer*, which is always the sound direction (a
/// replica can be stale, the writer never is), so it cannot be abused to skip
/// data — there is deliberately no inverse "force replica" value. Any value
/// other than the single accepted `"strong"` is rejected, so a typo fails loud
/// rather than silently degrading to routed.
enum Consistency {
    /// Absent: route normally (replica-eligible under the read budget).
    Default,
    /// `"strong"`: pin this filter's read to the writer pool.
    Strong,
    /// Present but not `"strong"`: reject the request.
    Malformed,
}

fn extract_consistency(raw: &Value) -> Consistency {
    let Some(value) = raw.get("consistency") else {
        return Consistency::Default;
    };
    match value.as_str() {
        Some("strong") => Consistency::Strong,
        _ => Consistency::Malformed,
    }
}

/// Which pool a catchall filter's read is dispatched to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadRoute {
    /// The default replica-eligible path (`query_events_routed`).
    Routed,
    /// The writer pool (`query_events`), pinned by `"consistency": "strong"`.
    Writer,
}

/// Resolve the pool a filter reads from, folding the `consistency` extension
/// into a routing direction. `"strong"` pins the writer; absent routes
/// normally; any other value is a client error (`Err`), rejected before any DB
/// work. This is the single seam that maps client-carried intent to a pool, so
/// a refactor that drops the field flips the mapping this function returns and
/// its tests fail.
fn resolve_read_route(raw: &Value) -> Result<ReadRoute, ()> {
    match extract_consistency(raw) {
        Consistency::Default => Ok(ReadRoute::Routed),
        Consistency::Strong => Ok(ReadRoute::Writer),
        Consistency::Malformed => Err(()),
    }
}

fn extract_buzz_channel(raw: &Value) -> Option<&str> {
    raw.get("#buzz-channel")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 1)
        .and_then(|values| values.first())
        .and_then(Value::as_str)
}

/// True when the raw filter opts into a bridge extension flag (`top_level`,
/// `include_summaries`, `include_aux`). Absent or non-boolean = false.
fn extension_flag(raw: &Value, key: &str) -> bool {
    raw.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn extract_depth_limit(raw: &Value) -> Option<u32> {
    raw.get("depth_limit")?
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
}

/// Extract a thread pagination cursor from the raw filter JSON.
///
/// The desktop pages `get_thread_replies` forward with a keyset cursor derived
/// transparently from the last reply it has already loaded — no server-issued
/// token. The cursor is a composite of that reply's `created_at` (Unix seconds,
/// field `thread_cursor`/`threadCursor`) and its hex event id (field
/// `thread_cursor_id`/`threadCursorId`). The event id is the tiebreak that lets
/// pagination cross replies sharing the same `created_at` second — without it,
/// a timestamp-only cursor silently drops every tied reply past the page limit
/// (the exact "missed messages" bug this work exists to fix).
///
/// Wire → DB encoding: 8-byte big-endian i64 seconds, followed by the raw
/// event-id bytes when present. `get_thread_replies` decodes this layout back
/// into its `(timestamp, event_id)` keyset. A bare timestamp (no id) is still
/// accepted and paginates on time alone (unsafe across same-second ties).
fn extract_thread_cursor(raw: &Value) -> Option<Vec<u8>> {
    let secs = raw
        .get("thread_cursor")
        .or_else(|| raw.get("threadCursor"))?
        .as_i64()?;
    let mut bytes = secs.to_be_bytes().to_vec();

    if let Some(id_hex) = raw
        .get("thread_cursor_id")
        .or_else(|| raw.get("threadCursorId"))
        .and_then(Value::as_str)
    {
        if let Ok(id_bytes) = hex::decode(id_hex) {
            bytes.extend_from_slice(&id_bytes);
        }
    }

    Some(bytes)
}

fn extract_feed_types(raw: &Value) -> Option<Vec<String>> {
    let arr = raw.get("feed_types")?.as_array()?;
    let types: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if types.is_empty() {
        None
    } else {
        Some(types)
    }
}

fn extract_search_mode(raw: &Value) -> buzz_search::SearchMode {
    match raw
        .get("search_mode")
        .or_else(|| raw.get("searchMode"))
        .and_then(Value::as_str)
    {
        Some("prefix") => buzz_search::SearchMode::Prefix,
        _ => buzz_search::SearchMode::FullText,
    }
}

fn extract_search_page(raw: &Value) -> u32 {
    raw.get("page")
        .or_else(|| raw.get("search_page"))
        .or_else(|| raw.get("searchPage"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

/// Compute the SQL `OFFSET` for a raw `page` extension on a non-search general
/// query, or `None` if paging shouldn't apply.
///
/// `page` is 1-based: page 1 → offset 0 (no change), page N → `(N-1) * limit`.
/// Returns `None` when `page` is absent or ≤ 1 (so unrelated general queries
/// keep their default offset) and when `limit` is missing (can't size a page).
/// This mirrors the FTS path's `page`/`per_page` for the non-search directory
/// listing (empty-query kind:0), whose deterministic `created_at DESC, id ASC`
/// ordering in `query_events` makes offset paging stable.
fn extract_page_offset(raw: &Value, limit: Option<i64>) -> Option<i64> {
    let page = raw
        .get("page")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok())
        .filter(|value| *value > 1)?;
    let per_page = limit.filter(|l| *l > 0)?;
    page.checked_sub(1)?.checked_mul(per_page)
}

/// Default and maximum row budget for a channel-window request. The budget
/// counts row events only; summary/bounds overlays and the aux closure never
/// consume it (docs/bridge-channel-window.md).
const BRIDGE_WINDOW_DEFAULT_LIMIT: u32 = 50;
const BRIDGE_WINDOW_MAX_LIMIT: u32 = 200;

/// Aux closure kinds: reactions, deletions (NIP-09 + NIP-29), edits.
const WINDOW_AUX_KINDS: [u32; 4] = [
    buzz_core::kind::KIND_DELETION,
    buzz_core::kind::KIND_REACTION,
    buzz_core::kind::KIND_NIP29_DELETE_EVENT,
    buzz_core::kind::KIND_STREAM_MESSAGE_EDIT,
];
/// Second-hop kinds: deletions targeting aux events (delete-of-a-reaction).
const WINDOW_AUX_DELETE_KINDS: [u32; 2] = [
    buzz_core::kind::KIND_DELETION,
    buzz_core::kind::KIND_NIP29_DELETE_EVENT,
];

/// Page size for one aux-closure hop. Matches the DB clamp
/// (`buzz_db::DEFAULT_MAX_PAGE_LIMIT`) so each page is one full query.
const AUX_PAGE_LIMIT: i64 = buzz_db::DEFAULT_MAX_PAGE_LIMIT;
/// Upper bound on pages drained per hop: 64k aux events referencing one page
/// of rows is far past any real thread; past it we log and stop rather than
/// loop forever against a pathological write pattern.
const AUX_MAX_PAGES: usize = 64;

fn build_aux_query(
    community: buzz_core::CommunityId,
    target_ids: Vec<String>,
    kinds: &[u32],
) -> buzz_db::EventQuery {
    let mut query = buzz_db::EventQuery::for_community(community);
    query.kinds = Some(kinds.iter().map(|kind| *kind as i32).collect());
    query.e_tags = Some(target_ids);
    query
}

/// Where an aux hop reads from: the window path pins the request's proved
/// read session; the thread path takes the routed display-read fast path.
enum AuxReader<'a> {
    Session(&'a mut buzz_db::ReadSession),
    Routed(&'a buzz_db::Db, &'static str),
    #[cfg(test)]
    Fake(&'a mut (dyn FnMut(&buzz_db::EventQuery) -> Vec<buzz_core::StoredEvent> + Send)),
}

impl AuxReader<'_> {
    async fn fetch(
        &mut self,
        query: &buzz_db::EventQuery,
    ) -> buzz_db::Result<Vec<buzz_core::StoredEvent>> {
        match self {
            AuxReader::Session(session) => session.query_events(query).await,
            AuxReader::Routed(db, path) => db.query_events_routed(path, query).await,
            #[cfg(test)]
            AuxReader::Fake(fetch) => Ok(fetch(query)),
        }
    }
}

/// Drain every event matching `query`, walking the `(created_at, id)` keyset
/// cursor `query_events` already orders by until a short page. An aux hop
/// over a reaction-heavy page can exceed a single page clamp, and because
/// results are newest-first a one-shot query silently drops the *oldest*
/// edits and deletions — rendering original or deleted content, not merely
/// losing decoration.
async fn query_all_pages(
    mut query: buzz_db::EventQuery,
    page_limit: i64,
    reader: &mut AuxReader<'_>,
) -> buzz_db::Result<Vec<buzz_core::StoredEvent>> {
    query.limit = Some(page_limit);
    let mut events = Vec::new();
    for _ in 0..AUX_MAX_PAGES {
        let page = reader.fetch(&query).await?;
        let next = if page.len() as i64 >= page_limit {
            page.last().map(|se| (se.event.created_at, se.event.id))
        } else {
            None
        };
        events.extend(page);
        let Some((created_at, id)) = next else {
            return Ok(events);
        };
        query.until = chrono::DateTime::from_timestamp(created_at.as_secs() as i64, 0);
        query.before_id = Some(id.to_bytes().to_vec());
    }
    tracing::warn!(
        pages = AUX_MAX_PAGES,
        events = events.len(),
        "aux closure hop exceeded page cap; returning truncated closure"
    );
    Ok(events)
}

/// Serve one `top_level: true` channel-window filter on the bridge `/query`
/// path (docs/bridge-channel-window.md). Appends, in order: row events, the
/// aux closure (`include_aux`), `39005` thread-summary overlays
/// (`include_summaries`), and exactly one `39006` window-bounds overlay.
///
/// Validation errors (missing `#h`, half a cursor) are deterministic client
/// mistakes and return `400`; an inaccessible channel is an access-scope skip
/// that still emits nothing, matching every other read path here.
async fn handle_channel_window_filter(
    state: &AppState,
    tenant: &buzz_core::TenantContext,
    raw: &Value,
    filter: &nostr::Filter,
    accessible_channels: &[uuid::Uuid],
    events: &mut Vec<Value>,
) -> Result<(), (StatusCode, Json<Value>)> {
    use buzz_core::kind::{KIND_THREAD_SUMMARY, KIND_WINDOW_BOUNDS};

    let Some(ch_id) = extract_channel_from_filter(filter) else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "top_level requires exactly one #h channel",
        ));
    };
    if !accessible_channels.contains(&ch_id) {
        return Ok(());
    }

    // Composite request cursor: `until` + `before_id`, both or neither. The
    // window path has no timestamp-only fallback — that ambiguity is the
    // dense-second dup/loss bug this surface exists to kill. A malformed
    // `before_id` is likewise rejected outright (NIP-CW cursor grammar),
    // never demoted to a half cursor or a head request.
    let before_id = match extract_before_id(raw) {
        BeforeId::Malformed => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "top_level: before_id must be a 64-hex event id",
            ));
        }
        BeforeId::Valid(id) => Some(id),
        BeforeId::Absent => None,
    };
    let cursor = match (filter.until, before_id) {
        (Some(ts), Some(id)) => {
            let ts = chrono::DateTime::from_timestamp(ts.as_secs() as i64, 0).ok_or_else(|| {
                api_error(StatusCode::BAD_REQUEST, "top_level: until is out of range")
            })?;
            Some((ts, id))
        }
        (None, None) => None,
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "top_level cursor requires both until and before_id, or neither",
            ));
        }
    };

    let limit = filter
        .limit
        .map(|l| (l as u32).min(BRIDGE_WINDOW_MAX_LIMIT))
        .unwrap_or(BRIDGE_WINDOW_DEFAULT_LIMIT)
        .max(1);
    let kind_filter: Option<Vec<u32>> = filter
        .kinds
        .as_ref()
        .map(|ks| ks.iter().map(|k| k.as_u16() as u32).collect());

    let (window, mut session) = state
        .db
        .get_channel_window_with_session(
            tenant.community(),
            ch_id,
            limit,
            cursor.clone(),
            kind_filter.as_deref(),
        )
        .await
        .map_err(|e| internal_error(&format!("channel window error: {e}")))?;

    // 1. Rows, in keyset order.
    let mut row_ids_hex = Vec::with_capacity(window.rows.len());
    for row in &window.rows {
        row_ids_hex.push(row.stored_event.event.id.to_hex());
        let v = serde_json::to_value(&row.stored_event.event)
            .map_err(|e| internal_error(&format!("window row serialize: {e}")))?;
        events.push(v);
    }

    // 2. Aux closure: reactions/deletions/edits targeting retained rows, plus
    //    deletions targeting those aux events (the transitive second hop).
    //    One round trip for the client instead of an #e fan-out. Runs in the
    //    SAME request transaction that served the window: when the page came
    //    from a proved replica session, the heartbeat observation anchored a
    //    REPEATABLE READ snapshot, so the aux hops see exactly the state the
    //    proof covered — another pooled session (or even another autocommit
    //    statement) could sit at a different replay position.
    if extension_flag(raw, "include_aux") && !row_ids_hex.is_empty() {
        let mut seen_aux: std::collections::HashSet<nostr::EventId> =
            std::collections::HashSet::new();
        let mut hop_ids = row_ids_hex.clone();
        for hop_kinds in [&WINDOW_AUX_KINDS[..], &WINDOW_AUX_DELETE_KINDS[..]] {
            let aux_query =
                build_aux_query(tenant.community(), std::mem::take(&mut hop_ids), hop_kinds);
            let aux_events = query_all_pages(
                aux_query,
                AUX_PAGE_LIMIT,
                &mut AuxReader::Session(&mut session),
            )
            .await
            .map_err(|e| internal_error(&format!("window aux error: {e}")))?;
            for se in aux_events {
                if !seen_aux.insert(se.event.id) {
                    continue;
                }
                // Deletions can be stored channel-less; access-check instead
                // of channel-constraining so they aren't silently dropped.
                if !event_in_accessible_channel(&se, accessible_channels) {
                    continue;
                }
                hop_ids.push(se.event.id.to_hex());
                let v = serde_json::to_value(&se.event)
                    .map_err(|e| internal_error(&format!("window aux serialize: {e}")))?;
                events.push(v);
            }
            if hop_ids.is_empty() {
                break;
            }
        }
    }

    let sign_overlay = |kind: u32, tags: Vec<nostr::Tag>, content: String| {
        nostr::EventBuilder::new(nostr::Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(&state.relay_keypair)
            .map_err(|e| internal_error(&format!("window overlay sign: {e}")))
    };
    let parse_tag = |parts: [&str; 2]| {
        nostr::Tag::parse(parts).map_err(|e| internal_error(&format!("window overlay tag: {e}")))
    };
    let ch_hex = ch_id.to_string();

    // 3. Thread-summary overlays: one relay-signed 39005 per row with replies.
    if extension_flag(raw, "include_summaries") {
        for row in &window.rows {
            let Some(summary) = &row.thread_summary else {
                continue;
            };
            let root_hex = row.stored_event.event.id.to_hex();
            let content = serde_json::json!({
                "reply_count": summary.reply_count,
                "descendant_count": summary.descendant_count,
                "last_reply_at": summary.last_reply_at.map(|t| t.timestamp()),
                "participants": summary.participants.iter().map(hex::encode).collect::<Vec<_>>(),
            });
            let tags = vec![
                parse_tag(["e", &root_hex])?,
                parse_tag(["d", &root_hex])?,
                parse_tag(["h", &ch_hex])?,
            ];
            let overlay = sign_overlay(KIND_THREAD_SUMMARY, tags, content.to_string())?;
            let v = serde_json::to_value(&overlay)
                .map_err(|e| internal_error(&format!("window overlay serialize: {e}")))?;
            events.push(v);
        }
    }

    // 4. Window bounds: exactly one 39006 per window response — the only
    //    authority on exhaustion. `rows < limit` proves nothing on an
    //    exact-multiple final page.
    let cursor_suffix = match &cursor {
        Some((ts, id)) => format!("{}:{}", ts.timestamp(), hex::encode(id)),
        None => "head".to_owned(),
    };
    let d_val = format!("{ch_hex}:{cursor_suffix}");
    let content = serde_json::json!({
        "has_more": window.has_more,
        "next_cursor": window.next_cursor.as_ref().map(|(ts, id)| serde_json::json!({
            "created_at": ts.timestamp(),
            "id": hex::encode(id),
        })),
    });
    let tags = vec![parse_tag(["d", &d_val])?, parse_tag(["h", &ch_hex])?];
    let overlay = sign_overlay(KIND_WINDOW_BOUNDS, tags, content.to_string())?;
    let v = serde_json::to_value(&overlay)
        .map_err(|e| internal_error(&format!("window overlay serialize: {e}")))?;
    events.push(v);

    Ok(())
}

fn event_in_accessible_channel(se: &buzz_core::StoredEvent, accessible: &[uuid::Uuid]) -> bool {
    match se.channel_id {
        Some(ch_id) => accessible.contains(&ch_id),
        None => true,
    }
}

/// Hard cap on the `reason` field logged for a rejected `/events` request.
///
/// The reject message can embed event-controlled content (e.g. a submitted
/// channel's `visibility`/`channel_type` tag values, or a raw tag pubkey) —
/// attacker-controlled text that must never reach Datadog unbounded.
const REJECT_REASON_MAX_BYTES: usize = 256;

/// Truncate `s` to at most `max_bytes`, cutting at the nearest UTF-8 character
/// boundary so a multi-byte codepoint straddling the cutoff is never split.
/// Bounds attacker-controlled text before it enters a structured log line —
/// the line's size must stay bounded regardless of the triggering input size.
fn truncate_reason(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Submit a signed Nostr event via HTTP bridge (NIP-98 auth).
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
pub async fn submit_event(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, axum::response::Response> {
    use axum::response::IntoResponse as _;
    // Row zero: bind this HTTP request to its community from the request host
    // before any tenant-scoped write, identical to the WS door in `router.rs`.
    // Unmapped host or lookup failure fails closed with a generic 404 — never a
    // default tenant, never echoing the host.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
            .into_response()
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/events");
    // In NIP-FI enforce/deny-protected mode, a real NIP-98 event is mandatory —
    // the X-Pubkey dev-mode fallback must never satisfy the pairing requirement.
    // [NIP-FI.md:594-607, FI-TRACE-HTTP-INGRESS]
    let nip_fi_active = !matches!(state.config.nip_fi.mode, NipFiMode::Off);
    // POST /events carries an authorization-relevant body (the event determines
    // resource, effect, and state change), so a payload tag is required in
    // NIP-FI enforce mode. [NIP-FI.md:619-637]
    let nip_fi_enforce = matches!(state.config.nip_fi.mode, NipFiMode::Enforce);

    // NIP-FI admission: NIP-98 extraction runs inside the closure, followed by
    // assertion verify → pair → deny-map in fixed order. The proven pubkey is
    // only available through the returned NipFiAdmission. [FI-TRACE-AUTHORITY-UNIFORM]
    let admission = admit_nip_fi_http_on_state(&state, &headers, || {
        verify_bridge_auth_with_options(
            &headers,
            "POST",
            &url,
            Some(&body),
            state.config.require_auth_token || nip_fi_active,
            nip_fi_enforce,
        )
        .map(|auth| Nip98Proof::new(auth.pubkey, (auth.event_id_bytes, auth.signed_created_at)))
        .map_err(|e| e.into_response())
    })?;
    let pubkey = *admission.proven_pubkey();
    let (event_id_bytes, signed_created_at) = admission.into_extra();
    let pubkey_hex = pubkey.to_hex();

    // Everything after auth — admission, replay, membership, parse, ingest —
    // runs inside the helper.  The thin wrapper here owns the single terminal
    // attribution line so it fires for every outcome, including admission/
    // replay/membership failures that previously returned before any log fired.
    let outcome = submit_event_authed(
        &state,
        &tenant,
        &headers,
        &body,
        pubkey,
        event_id_bytes,
        signed_created_at,
    )
    .await;

    match &outcome {
        SubmitOutcome::Ok { accepted, kind, .. } => {
            tracing::info!(
                pubkey = %pubkey_hex,
                route = "/events",
                status = 200u16,
                accepted,
                kind,
                "HTTP bridge request"
            );
        }
        SubmitOutcome::ParseFail {
            category,
            line,
            column,
            ..
        } => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/events",
                status = 400u16,
                accepted = false,
                category,
                line,
                column,
                "HTTP bridge request"
            );
        }
        SubmitOutcome::Rejected {
            kind,
            reason,
            response,
        } => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/events",
                status = response.0.as_u16(),
                accepted = false,
                kind,
                reason = %reason,
                "HTTP bridge request"
            );
        }
        SubmitOutcome::Err { status, .. } => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/events",
                status = status.as_u16(),
                accepted = false,
                "HTTP bridge request"
            );
        }
    }

    Ok(outcome.into_response().into_response())
}

/// Log-context outcome for a single [`submit_event`] call.
///
/// Carries enough structured data for the terminal attribution log while also
/// holding the HTTP response so the thin wrapper can return it unchanged.
enum SubmitOutcome {
    /// Ingest pipeline ran and returned a result (accepted or not).
    Ok {
        accepted: bool,
        kind: u32,
        response: Json<Value>,
    },
    /// JSON parse failure before ingest — log category/line/column, not msg.
    ParseFail {
        category: &'static str,
        line: usize,
        column: usize,
        response: (StatusCode, Json<Value>),
    },
    /// IngestError::Rejected or IngestError::CanvasConflict — log kind + truncated reason.
    ///
    /// Generic rejections yield HTTP 400; canvas CAS conflicts yield HTTP 409.
    /// The logged `status` reflects the actual response status carried in `response`.
    Rejected {
        kind: u32,
        reason: String,
        response: (StatusCode, Json<Value>),
    },
    /// Any other error (admission, replay, membership, auth, internal) —
    /// only the HTTP status is logged; the response body is returned as-is.
    Err {
        status: StatusCode,
        response: (StatusCode, Json<Value>),
    },
}

impl SubmitOutcome {
    fn into_response(self) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        match self {
            SubmitOutcome::Ok { response, .. } => Ok(response),
            SubmitOutcome::ParseFail { response, .. } => Err(response),
            SubmitOutcome::Rejected { response, .. } => Err(response),
            SubmitOutcome::Err { response, .. } => Err(response),
        }
    }
}

/// Post-auth execution for [`submit_event`]: admission, replay, membership,
/// parse, and ingest.  Returns a [`SubmitOutcome`] that carries both the log
/// fields and the HTTP response so the thin wrapper can emit exactly one
/// terminal attribution line covering every outcome.
async fn submit_event_authed(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    headers: &HeaderMap,
    body: &[u8],
    pubkey: nostr::PublicKey,
    event_id_bytes: [u8; 32],
    signed_auth_created_at: Option<u64>,
) -> SubmitOutcome {
    // Admission and replay checks fire before body parse — a 429 or replay
    // reject on a malformed body must still be attributed.
    if let Err(e) = enforce_http_admission(state, tenant, &pubkey).await {
        return SubmitOutcome::Err {
            status: e.0,
            response: e,
        };
    }
    if let Err(e) = check_nip98_replay(state, tenant, event_id_bytes).await {
        return SubmitOutcome::Err {
            status: e.0,
            response: e,
        };
    }
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let event: nostr::Event = match serde_json::from_slice(body) {
        Ok(ev) => ev,
        Err(e) => {
            // Never log `e`'s Display string: serde_json embeds the offending
            // input verbatim in its error message, so a malformed field of
            // arbitrary size (the router allows 1 MiB bodies) would otherwise
            // reflect attacker-controlled text into a log line at full size.
            // `category`/`line`/`column` are bounded, structured, and still
            // enough to tell "which parse failure" apart at a glance.
            crate::handlers::ingest::reject_with_transport("http", "invalid");
            return SubmitOutcome::ParseFail {
                category: match e.classify() {
                    serde_json::error::Category::Io => "io",
                    serde_json::error::Category::Syntax => "syntax",
                    serde_json::error::Category::Data => "data",
                    serde_json::error::Category::Eof => "eof",
                },
                line: e.line(),
                column: e.column(),
                response: api_error(StatusCode::BAD_REQUEST, &format!("invalid event JSON: {e}")),
            };
        }
    };

    // Enforce relay membership (with NIP-OA fallback via x-auth-tag header).
    let auth_tag = super::relay_members::extract_auth_tag_header(headers);
    let nip_oa_owner = match super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        signed_auth_created_at,
    )
    .await
    {
        Ok(owner) => owner.or_else(|| {
            if !state.config.require_relay_membership {
                super::relay_members::extract_nip_oa_owner(
                    &pubkey_bytes,
                    auth_tag,
                    signed_auth_created_at,
                )
            } else {
                None
            }
        }),
        Err(e) => {
            return SubmitOutcome::Err {
                status: e.0,
                response: e,
            };
        }
    };
    if let Some(owner) = nip_oa_owner {
        super::relay_members::materialize_nip_oa_owner(state, tenant, &pubkey, &owner).await;
    }

    let kind_u32 = buzz_core::kind::event_kind_u32(&event);
    let auth = IngestAuth::Http {
        pubkey,
        scopes: buzz_auth::Scope::all_known(), // Pure Nostr: full scopes, channel access via membership
        auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
    };

    let result = if buzz_core::kind::is_ephemeral(kind_u32) {
        let event_id = event.id.to_hex();
        crate::handlers::ephemeral::submit(state, tenant, event, &auth)
            .await
            .map(|()| crate::handlers::ingest::IngestResult {
                event_id,
                accepted: true,
                message: String::new(),
            })
    } else {
        crate::handlers::ingest::ingest_event(state, tenant, event, auth).await
    };
    match result {
        Ok(result) => {
            let response = Json(serde_json::json!({
                "event_id": result.event_id,
                "accepted": result.accepted,
                "message": result.message,
            }));
            SubmitOutcome::Ok {
                accepted: result.accepted,
                kind: kind_u32,
                response,
            }
        }
        Err(IngestError::Rejected(msg)) => {
            // `msg` can embed event-controlled content (e.g. a channel
            // create's raw `visibility`/`channel_type` tag values, or a raw
            // tag pubkey) — truncate before logging, but return the full msg
            // in the HTTP response body (unchanged from prior behaviour).
            let reason = truncate_reason(&msg, REJECT_REASON_MAX_BYTES).to_owned();
            crate::handlers::ingest::reject_with_transport("http", "invalid");
            SubmitOutcome::Rejected {
                kind: kind_u32,
                reason,
                response: api_error(StatusCode::BAD_REQUEST, &msg),
            }
        }
        Err(IngestError::CanvasConflict(msg)) => {
            // Canvas CAS precondition failures are a distinct HTTP 409 so the
            // CLI's reconciliation branch (which gates on `status == 409`) is
            // reachable against the live relay.  The message body is unchanged;
            // the desktop TypeScript layer matches on message text, not status.
            let reason = truncate_reason(&msg, REJECT_REASON_MAX_BYTES).to_owned();
            crate::handlers::ingest::reject_with_transport("http", "invalid");
            SubmitOutcome::Rejected {
                kind: kind_u32,
                reason,
                response: api_error(StatusCode::CONFLICT, &msg),
            }
        }
        Err(IngestError::AuthFailed(msg)) => {
            crate::handlers::ingest::reject_with_transport("http", "auth");
            let e = api_error(StatusCode::FORBIDDEN, &msg);
            SubmitOutcome::Err {
                status: e.0,
                response: e,
            }
        }
        Err(IngestError::Internal(msg)) => {
            crate::handlers::ingest::reject_with_transport("http", "error");
            let e = internal_error(&msg);
            SubmitOutcome::Err {
                status: e.0,
                response: e,
            }
        }
    }
}

/// Query events via HTTP bridge (NIP-98 auth). Returns JSON array of events.
///
/// Enforces channel access: results are filtered to channels the user can access.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
pub async fn query_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, axum::response::Response> {
    use axum::response::IntoResponse as _;
    // Row zero: bind this HTTP request to its community from the request host
    // before any tenant-scoped read, identical to the WS door in `router.rs`.
    // An unmapped host or lookup failure fails closed with a generic 404 — never
    // a default tenant, never echoing the host (so an unauthenticated caller
    // cannot probe which communities exist on this deployment).
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
            .into_response()
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/query");
    // In NIP-FI enforce/deny-protected mode, a real NIP-98 event is mandatory.
    // [NIP-FI.md:594-607, FI-TRACE-HTTP-INGRESS]
    let nip_fi_active = !matches!(state.config.nip_fi.mode, NipFiMode::Off);
    // POST /query carries an authorization-relevant body (filter selects the
    // resources returned), so a payload tag is required in enforce mode.
    // [NIP-FI.md:619-637]
    let nip_fi_enforce = matches!(state.config.nip_fi.mode, NipFiMode::Enforce);

    // NIP-FI admission. [FI-TRACE-AUTHORITY-UNIFORM]
    let admission = admit_nip_fi_http_on_state(&state, &headers, || {
        verify_bridge_auth_with_options(
            &headers,
            "POST",
            &url,
            Some(&body),
            state.config.require_auth_token || nip_fi_active,
            nip_fi_enforce,
        )
        .map(|auth| Nip98Proof::new(auth.pubkey, (auth.event_id_bytes, auth.signed_created_at)))
        .map_err(|e| e.into_response())
    })?;
    let pubkey = *admission.proven_pubkey();
    let (event_id_bytes, signed_created_at) = admission.into_extra();
    let pubkey_hex = pubkey.to_hex();

    // Admission, replay, membership, and filter execution all run inside the
    // helper.  The single terminal attribution line fires here from the Result
    // so every outcome — including admission/replay/membership failures that
    // previously returned before any log — is attributed.
    let result = query_events_authed(
        &state,
        &tenant,
        &headers,
        &body,
        pubkey,
        event_id_bytes,
        signed_created_at,
    )
    .await;
    match &result {
        Ok(Json(Value::Array(events))) => {
            tracing::info!(
                pubkey = %pubkey_hex,
                route = "/query",
                status = 200u16,
                result_count = events.len(),
                "HTTP bridge request"
            );
        }
        Ok(_) => {
            tracing::info!(pubkey = %pubkey_hex, route = "/query", status = 200u16, "HTTP bridge request");
        }
        Err((status, _)) => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/query",
                status = status.as_u16(),
                "HTTP bridge request"
            );
        }
    }
    Ok(result.into_response())
}

/// Filter execution for [`query_events`], run once NIP-98 auth succeeds.
/// Handles admission, replay, membership, and all filter paths so the thin
/// wrapper above can emit exactly one terminal attribution line from the Result.
async fn query_events_authed(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    headers: &HeaderMap,
    body: &[u8],
    pubkey: nostr::PublicKey,
    event_id_bytes: [u8; 32],
    signed_auth_created_at: Option<u64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    enforce_http_admission(state, tenant, &pubkey).await?;
    check_nip98_replay(state, tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = super::relay_members::extract_auth_tag_header(headers);
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        signed_auth_created_at,
    )
    .await?;

    // Two-pass parse: preserve raw JSON for custom extension fields (before_id,
    // depth_limit, feed_types) that nostr::Filter silently drops.
    let raw_filters: Vec<Value> = serde_json::from_slice(body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;
    let thread_windows = thread_window::parse(&raw_filters)?;
    let filters: Vec<nostr::Filter> = raw_filters
        .iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<_, _>>()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;
    crate::handlers::req::extract_channel_ids_from_filters_limited(&filters)
        .map_err(|()| api_error(StatusCode::BAD_REQUEST, "too many explicit channels"))?;

    // P-gated kinds (gift wraps, member notifications, observer frames) require
    // the caller's own pubkey in the #p tag — same enforcement as WS REQ handler.
    let authed_pubkey_hex = pubkey.to_hex();
    if !crate::handlers::req::p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: p-gated kinds require #p tag matching your pubkey",
        ));
    }
    if !crate::handlers::req::engram_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: agent-engram reads require authors=[self] or #p=[self]",
        ));
    }
    if !crate::handlers::req::author_only_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: author-only kinds require authors=[self]",
        ));
    }

    if thread_windows.iter().any(Option::is_some) {
        if thread_windows.iter().any(Option::is_none) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "thread_window cannot mix with other query modes",
            ));
        }
        return tokio::time::timeout(
            thread_window::DEADLINE,
            thread_window::query_batch(state, tenant, &pubkey, thread_windows.iter().flatten()),
        )
        .await
        .map_err(|_| {
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "thread window deadline exceeded",
            )
        })?
        .map(|events| Json(Value::Array(events)));
    }
    if read_state_snapshot::requested(&raw_filters) {
        return read_state_snapshot::query(state, tenant, &pubkey, &raw_filters).await;
    }

    // Get channels this user can access — same enforcement as WS REQ handler.
    let mut accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;
    repair_requested_channel_access(
        state,
        tenant,
        &filters,
        &pubkey_bytes,
        &mut accessible_channels,
    )
    .await?;

    if filters.iter().any(|f| f.search.is_some()) {
        if has_mixed_search_filters(&filters) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "mixed search and non-search filters not supported",
            ));
        }
        return handle_bridge_search(
            state,
            &raw_filters,
            &filters,
            &accessible_channels,
            tenant,
            &authed_pubkey_hex,
            &pubkey_bytes,
        )
        .await;
    }

    if let Some(presence_result) =
        synthesize_presence(&state.pubsub, &state.relay_keypair, tenant, &filters).await
    {
        return presence_result.map(|events| Json(Value::Array(events)));
    }

    let mut events: Vec<Value> = Vec::new();
    let mut handled: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let ownership_targets: usize = raw_filters
        .iter()
        .zip(&filters)
        .filter(|(raw, _)| extension_flag(raw, "resolve_thread_roots"))
        .map(|(_, filter)| filter.ids.as_ref().map_or(0, |ids| ids.len()))
        .sum();
    if ownership_targets > 100 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "resolve_thread_roots permits at most 100 targets per request",
        ));
    }
    // Resolve reply owners from retained thread metadata, including tombstones.
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if extension_flag(raw, "resolve_thread_roots") {
            events.extend(thread_roots::query(state, tenant, filter, &accessible_channels).await?);
            handled.insert(idx);
        }
    }

    // Channel-window filters (`top_level: true`) — the GUI read-model surface.
    // Dispatched first: a window filter is never a feed/thread/catchall query.
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) || !extension_flag(raw, "top_level") {
            continue;
        }
        handle_channel_window_filter(
            state,
            tenant,
            raw,
            filter,
            &accessible_channels,
            &mut events,
        )
        .await?;
        handled.insert(idx);
    }

    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }
        let feed_types = match extract_feed_types(raw) {
            Some(t) => t,
            None => continue,
        };

        let limit = filter
            .limit
            .map(|l| (l as i64).min(BRIDGE_FEED_MAX_LIMIT))
            .unwrap_or(20);
        let since = filter
            .since
            .and_then(|s| chrono::DateTime::from_timestamp(s.as_secs() as i64, 0));

        let mut seen_types = std::collections::HashSet::new();
        let mut seen = std::collections::HashSet::new();
        let mut feed_count = 0i64;
        for feed_type in &feed_types {
            let canonical = if feed_type == "agent_activity" {
                "activity"
            } else {
                feed_type.as_str()
            };
            if !seen_types.insert(canonical) {
                continue;
            }
            if feed_count >= limit {
                break;
            }
            let remaining = limit - feed_count;
            let type_events = match canonical {
                "mentions" => state
                    .db
                    .query_feed_mentions_routed(
                        "bridge_feed",
                        tenant.community(),
                        &pubkey_bytes,
                        &accessible_channels,
                        since,
                        remaining,
                    )
                    .await
                    .map_err(|e| internal_error(&format!("feed mentions error: {e}")))?,
                "needs_action" => state
                    .db
                    .query_feed_needs_action_routed(
                        "bridge_feed",
                        tenant.community(),
                        &pubkey_bytes,
                        &accessible_channels,
                        since,
                        remaining,
                    )
                    .await
                    .map_err(|e| internal_error(&format!("feed needs_action error: {e}")))?,
                "activity" => state
                    .db
                    .query_feed_activity_routed(
                        "bridge_feed",
                        tenant.community(),
                        &accessible_channels,
                        since,
                        remaining,
                    )
                    .await
                    .map_err(|e| internal_error(&format!("feed activity error: {e}")))?,
                _ => continue,
            };
            for se in type_events {
                if !seen.insert(se.event.id) {
                    continue;
                }
                if !event_in_accessible_channel(&se, &accessible_channels) {
                    continue;
                }
                // Defense-in-depth: never deliver a result-gated event (e.g. kind:44200
                // or kind:30622) to a non-owner via the feed path, even though feed SQL
                // kind allowlists already exclude these kinds.
                if !buzz_core::filter::reader_authorized_for_event(&se.event, &authed_pubkey_hex) {
                    continue;
                }
                if let Ok(v) = serde_json::to_value(&se.event) {
                    events.push(v);
                    feed_count += 1;
                }
            }
        }
        handled.insert(idx);
    }

    let e_tag_key = nostr::SingleLetterTag::lowercase(nostr::Alphabet::E);
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }
        let depth = match extract_depth_limit(raw) {
            Some(d) => d,
            None => continue,
        };
        let e_values = match filter.generic_tags.get(&e_tag_key) {
            Some(vs) if vs.len() == 1 => vs,
            _ => continue,
        };
        let root_hex = match e_values.iter().next() {
            Some(h) => h,
            None => continue,
        };
        let root_bytes = match hex::decode(root_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => continue,
        };

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                handled.insert(idx);
                continue;
            }
        }

        let limit = filter
            .limit
            .unwrap_or(100)
            .min(BRIDGE_THREAD_MAX_LIMIT as usize) as u32;
        let thread_cursor = extract_thread_cursor(raw);
        let thread_replies = state
            .db
            .get_thread_replies(
                tenant.community(),
                &root_bytes,
                Some(depth),
                limit,
                thread_cursor.as_deref(),
            )
            .await
            .map_err(|e| internal_error(&format!("thread query error: {e}")))?;

        let mut thread_row_ids = Vec::with_capacity(thread_replies.len() + 1);
        thread_row_ids.push(root_hex.to_string());
        for reply in thread_replies {
            let se = reply.stored_event;
            if !event_in_accessible_channel(&se, &accessible_channels) {
                continue;
            }
            // Defense-in-depth: never deliver a result-gated event (e.g. kind:44200
            // or kind:30622) to a non-owner via the thread path, even though
            // requires_h_channel_scope already excludes these kinds from thread metadata.
            if !buzz_core::filter::reader_authorized_for_event(&se.event, &authed_pubkey_hex) {
                continue;
            }
            thread_row_ids.push(se.event.id.to_hex());
            if let Ok(v) = serde_json::to_value(&se.event) {
                events.push(v);
            }
        }

        if extension_flag(raw, "include_aux") && !thread_row_ids.is_empty() {
            let mut seen_aux = std::collections::HashSet::new();
            let mut hop_ids = thread_row_ids;
            for hop_kinds in [&WINDOW_AUX_KINDS[..], &WINDOW_AUX_DELETE_KINDS[..]] {
                let aux_query =
                    build_aux_query(tenant.community(), std::mem::take(&mut hop_ids), hop_kinds);
                let aux_events = query_all_pages(
                    aux_query,
                    AUX_PAGE_LIMIT,
                    &mut AuxReader::Routed(&state.db, "bridge_thread_aux"),
                )
                .await
                .map_err(|e| internal_error(&format!("thread aux query error: {e}")))?;
                for se in aux_events {
                    if !seen_aux.insert(se.event.id)
                        || !event_in_accessible_channel(&se, &accessible_channels)
                        || !buzz_core::filter::reader_authorized_for_event(
                            &se.event,
                            &authed_pubkey_hex,
                        )
                    {
                        continue;
                    }
                    hop_ids.push(se.event.id.to_hex());
                    if let Ok(value) = serde_json::to_value(&se.event) {
                        events.push(value);
                    }
                }
                if hop_ids.is_empty() {
                    break;
                }
            }
        }
        handled.insert(idx);
    }

    // Phase 1 — pure construction + validation, in filter order. Access-scope
    // skips and the `before_id` BAD_REQUEST are decided here, before any DB
    // work is issued (validation errors are deterministic client mistakes, so
    // surfacing them ahead of transient DB errors is strictly more predictable).
    let mut catchall_queries: Vec<(usize, buzz_db::EventQuery, ReadRoute)> = Vec::new();
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue;
            }
        }

        // Read-your-writes opt-in: a write-influencing read pins to the writer
        // pool so a lagging replica cannot hide the caller's own just-accepted
        // write. Rejected before any DB work, like the `before_id` grammar
        // error below — a malformed value is a deterministic client mistake.
        let read_route = resolve_read_route(raw).map_err(|()| {
            api_error(
                StatusCode::BAD_REQUEST,
                "consistency must be \"strong\" when present",
            )
        })?;

        let mut query = crate::handlers::req::build_event_query_from_filter(
            filter,
            &pubkey_bytes,
            state,
            tenant.community(),
        )
        .await;
        crate::handlers::req::apply_channel_scope_to_query(
            &mut query,
            filter,
            extract_channel_from_filter(filter),
            &accessible_channels,
        );
        if let Some(channel) = extract_buzz_channel(raw) {
            query.custom_tag = Some(("buzz-channel".into(), channel.into()));
        }
        // Shared-gated visibility pushdown: must mirror WS REQ so that a page of
        // newer private events does not starve older shared ones off the page.
        if crate::handlers::req::filter_can_match_shared_gated_kinds(filter) {
            query.shared_gated_reader = Some(pubkey_bytes.clone());
        }

        match extract_before_id(raw) {
            BeforeId::Malformed => {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "before_id must be a 64-char hex event id",
                ));
            }
            BeforeId::Valid(bid) => {
                if query.until.is_none() {
                    return Err(api_error(
                        StatusCode::BAD_REQUEST,
                        "before_id requires until to be set",
                    ));
                }
                query.before_id = Some(bid);
            }
            BeforeId::Absent => {}
        }

        // Honor `page` on non-search general queries so offset paging works for
        // the empty-query people directory (kind:0 listing). The FTS path
        // (`handle_bridge_search`) has its own `page`/`per_page`; a filter with
        // no `search` field lands here instead, where paging would otherwise be
        // dropped and the directory would terminate at its first page. Deterministic
        // ordering in `query_events` (`created_at DESC, id ASC`) makes offset paging
        // stable. `page` defaults to 1 → offset 0, so unrelated general queries are
        // unaffected.
        if let Some(offset) = extract_page_offset(raw, query.limit) {
            query.offset = Some(offset);
        }

        catchall_queries.push((idx, query, read_route));
    }

    // Phase 2 — DB reads, bounded-concurrent, order-preserving (`buffered`).
    // Phase 3 consumes results in original filter order, so response ordering
    // and error semantics match the previous serial loop.
    use futures_util::stream::{self, StreamExt};
    let db = state.db.clone();
    let mut catchall_results = stream::iter(catchall_queries.into_iter().map(
        |(idx, query, read_route)| {
            let db = db.clone();
            async move {
                // The route was resolved from client-carried `consistency`
                // intent in phase 1 (`resolve_read_route`). `Writer` pins the
                // read to the writer pool (`query_events`); `Routed` takes the
                // replica-eligible path. Only these two directions exist — the
                // inverse "force replica" is deliberately unrepresentable.
                let result = match read_route {
                    ReadRoute::Writer => db.query_events(&query).await,
                    ReadRoute::Routed => db.query_events_routed("bridge_query", &query).await,
                };
                (idx, result)
            }
        },
    ))
    .buffered(crate::handlers::req::FILTER_QUERY_CONCURRENCY);

    // Phase 3 — post-processing, strictly in filter order.
    while let Some((idx, filter_events)) = catchall_results.next().await {
        let filter = &filters[idx];
        match filter_events {
            Ok(stored_events) => {
                for se in stored_events {
                    if !event_in_accessible_channel(&se, &accessible_channels) {
                        continue;
                    }
                    if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se) {
                        continue;
                    }
                    // Result-level read auth: never hand a viewer-private snapshot
                    // (kind:30622) to anyone but its owner, even via kindless `ids`.
                    // Also enforces author-only kinds (30300/30350) and the persona
                    // shared-gate (kind:30175 without ["shared","true"]). Single call
                    // covers all three gated event classes.
                    if !crate::handlers::req::event_visible_to_reader(&se.event, &pubkey_bytes) {
                        continue;
                    }
                    if let Ok(v) = serde_json::to_value(&se.event) {
                        events.push(v);
                    }
                }
            }
            Err(e) => {
                return Err(internal_error(&format!("query error: {e}")));
            }
        }
    }

    Ok(Json(Value::Array(events)))
}

async fn repair_requested_channel_access(
    state: &AppState,
    tenant: &TenantContext,
    filters: &[nostr::Filter],
    pubkey_bytes: &[u8],
    accessible_channels: &mut Vec<uuid::Uuid>,
) -> Result<(), (StatusCode, Json<Value>)> {
    for filter in filters {
        let Some(requested) =
            crate::handlers::req::extract_channel_ids_from_filters(std::slice::from_ref(filter))
        else {
            continue;
        };
        for channel_id in requested {
            if accessible_channels.contains(&channel_id) {
                continue;
            }
            let is_member = state
                .db
                .is_member(tenant.community(), channel_id, pubkey_bytes)
                .await
                .map_err(|e| internal_error(&format!("channel membership confirmation: {e}")))?;
            crate::handlers::req::resolve_request_local_access(
                accessible_channels,
                channel_id,
                true,
                Some(is_member),
            );
        }
    }
    Ok(())
}

/// Count events via HTTP bridge (NIP-98 auth). Returns `{"count": N}`.
///
/// Enforces channel access: only counts events in channels the user can access.
/// For filters without a `#h` tag, falls back to per-event counting with access checks.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
pub async fn count_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, axum::response::Response> {
    use axum::response::IntoResponse as _;
    // Row zero: bind this HTTP request to its community from the request host
    // before any tenant-scoped read, identical to the WS door in `router.rs`
    // and `query_events`/`submit_event` above. Fail-closed; never a default
    // tenant, never echoing the host.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
            .into_response()
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/count");
    // In NIP-FI enforce/deny-protected mode, a real NIP-98 event is mandatory.
    // [NIP-FI.md:594-607, FI-TRACE-HTTP-INGRESS]
    let nip_fi_active = !matches!(state.config.nip_fi.mode, NipFiMode::Off);
    // POST /count carries an authorization-relevant body (filter selects what
    // is counted), so a payload tag is required in enforce mode.
    // [NIP-FI.md:619-637]
    let nip_fi_enforce = matches!(state.config.nip_fi.mode, NipFiMode::Enforce);

    // NIP-FI admission. [FI-TRACE-AUTHORITY-UNIFORM]
    let admission = admit_nip_fi_http_on_state(&state, &headers, || {
        verify_bridge_auth_with_options(
            &headers,
            "POST",
            &url,
            Some(&body),
            state.config.require_auth_token || nip_fi_active,
            nip_fi_enforce,
        )
        .map(|auth| Nip98Proof::new(auth.pubkey, (auth.event_id_bytes, auth.signed_created_at)))
        .map_err(|e| e.into_response())
    })?;
    let pubkey = *admission.proven_pubkey();
    let (event_id_bytes, signed_created_at) = admission.into_extra();
    let pubkey_hex = pubkey.to_hex();

    // Admission, replay, membership, and count execution all run inside the
    // helper.  The single terminal attribution line fires here from the Result
    // so every outcome — including admission/replay/membership failures that
    // previously returned before any log — is attributed.
    let result = count_events_authed(
        &state,
        &tenant,
        &headers,
        &body,
        pubkey,
        event_id_bytes,
        signed_created_at,
    )
    .await;
    match &result {
        Ok(Json(value)) => {
            let count = value.get("count").and_then(Value::as_u64);
            tracing::info!(
                pubkey = %pubkey_hex,
                route = "/count",
                status = 200u16,
                result_count = count,
                "HTTP bridge request"
            );
        }
        Err((status, _)) => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/count",
                status = status.as_u16(),
                "HTTP bridge request"
            );
        }
    }
    Ok(result.into_response())
}

/// Filter execution for [`count_events`], run once NIP-98 auth succeeds.
/// Handles admission, replay, membership, and count execution so the thin
/// wrapper above can emit exactly one terminal attribution line from the Result.
async fn count_events_authed(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    headers: &HeaderMap,
    body: &[u8],
    pubkey: nostr::PublicKey,
    event_id_bytes: [u8; 32],
    signed_auth_created_at: Option<u64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    enforce_http_admission(state, tenant, &pubkey).await?;
    check_nip98_replay(state, tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = super::relay_members::extract_auth_tag_header(headers);
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        signed_auth_created_at,
    )
    .await?;

    let filters: Vec<nostr::Filter> = serde_json::from_slice(body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;
    crate::handlers::req::extract_channel_ids_from_filters_limited(&filters)
        .map_err(|()| api_error(StatusCode::BAD_REQUEST, "too many explicit channels"))?;

    // P-gated kinds enforcement — same as WS REQ and /query.
    let authed_pubkey_hex = pubkey.to_hex();
    if !crate::handlers::req::p_gated_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: p-gated kinds require #p tag matching your pubkey",
        ));
    }
    if !crate::handlers::req::engram_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: agent-engram reads require authors=[self] or #p=[self]",
        ));
    }
    if !crate::handlers::req::author_only_filters_authorized(&filters, &authed_pubkey_hex) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: author-only kinds require authors=[self]",
        ));
    }

    // Get channels this user can access.
    let mut accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;
    repair_requested_channel_access(
        state,
        tenant,
        &filters,
        &pubkey_bytes,
        &mut accessible_channels,
    )
    .await?;

    let mut total: u64 = 0;
    for filter in &filters {
        let needs_author_only_filtering =
            crate::handlers::req::filter_can_match_author_only_kinds(filter);
        // Same result-gated guard as the WS COUNT handler: force the per-event
        // fallback for filters that can match 44200 or 30622 unless #p=[self]
        // is safely pushed down (existence leak otherwise).
        let needs_result_gated_filtering =
            crate::handlers::req::filter_can_match_result_gated_kinds(filter)
                && !crate::handlers::req::result_gated_count_safe_for_pushdown(
                    filter,
                    &authed_pubkey_hex,
                );
        // Force per-event fallback for filters that can match a shared-gated
        // kind — the fast SQL count_events() path has no per-event gate and
        // would over-count foreign unshared events (existence leak).
        let needs_shared_gate_filtering =
            crate::handlers::req::filter_can_match_shared_gated_kinds(filter);

        // If filter targets a specific channel, verify access.
        if crate::handlers::req::extract_channel_ids_from_filters(std::slice::from_ref(filter))
            .is_some()
        {
            let ch_id = extract_channel_from_filter(filter);
            let requested = crate::handlers::req::extract_channel_ids_from_filters(
                std::slice::from_ref(filter),
            )
            .unwrap_or_default();
            if !requested
                .iter()
                .any(|channel_id| accessible_channels.contains(channel_id))
            {
                continue;
            }
            // Channel is accessible — count with pushability check.
            let mut query = crate::handlers::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                state,
                tenant.community(),
            )
            .await;
            crate::handlers::req::apply_channel_scope_to_query(
                &mut query,
                filter,
                ch_id,
                &accessible_channels,
            );
            // Shared-gated visibility pushdown: same as REQ and /query paths, so
            // the fallback's query_events call doesn't over-fetch private rows.
            if needs_shared_gate_filtering {
                query.shared_gated_reader = Some(pubkey_bytes.clone());
            }
            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
                && !needs_shared_gate_filtering
            {
                match state.db.count_events_routed("bridge_count", &query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query + post-filter for non-pushable constraints.
                let mut q = query;
                crate::handlers::req::apply_count_fallback_limit(&mut q);
                match state
                    .db
                    .query_events_routed_bounded("bridge_count_fallback", &q)
                    .await
                {
                    Ok(stored_events) => {
                        if crate::handlers::req::count_fallback_exceeded(stored_events.len()) {
                            metrics::counter!("buzz_count_fallback_rejections_total").increment(1);
                            return Err(api_error(
                                StatusCode::BAD_REQUEST,
                                "count filter requires narrower constraints",
                            ));
                        }
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if !crate::handlers::req::event_visible_to_reader(
                                &se.event,
                                &pubkey_bytes,
                            ) {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            }
        } else {
            // No channel filter — use SQL-level channel_ids pushdown to count
            // only events in accessible channels (+ global events).
            let mut query = crate::handlers::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                state,
                tenant.community(),
            )
            .await;
            query.channel_ids = Some(accessible_channels.to_vec());
            // Shared-gated visibility pushdown: pre-filter before ORDER/LIMIT on
            // the fallback query_events path.
            if needs_shared_gate_filtering {
                query.shared_gated_reader = Some(pubkey_bytes.clone());
            }

            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
                && !needs_shared_gate_filtering
            {
                query.limit = None;
                match state.db.count_events_routed("bridge_count", &query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query a bounded candidate set + post-filter.
                crate::handlers::req::apply_count_fallback_limit(&mut query);
                match state
                    .db
                    .query_events_routed_bounded("bridge_count_fallback", &query)
                    .await
                {
                    Ok(stored_events) => {
                        if crate::handlers::req::count_fallback_exceeded(stored_events.len()) {
                            metrics::counter!("buzz_count_fallback_rejections_total").increment(1);
                            return Err(api_error(
                                StatusCode::BAD_REQUEST,
                                "count filter requires narrower constraints",
                            ));
                        }
                        for se in stored_events {
                            if !buzz_core::filter::filters_match(std::slice::from_ref(filter), &se)
                            {
                                continue;
                            }
                            if !crate::handlers::req::event_visible_to_reader(
                                &se.event,
                                &pubkey_bytes,
                            ) {
                                continue;
                            }
                            total += 1;
                        }
                    }
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "count": total })))
}

fn has_mixed_search_filters(filters: &[nostr::Filter]) -> bool {
    filters.iter().any(|f| f.search.is_some()) && filters.iter().any(|f| f.search.is_none())
}

/// Decide whether a search hit should be returned to the caller.
///
/// Mirrors the WS NIP-50 path's post-filter step in `handlers/req.rs`:
/// the FTS backend receives only the kind/authors/time pushdown, so any other filter
/// constraint (`#p`, `#h`, `#e`, `#d`, `ids`, …) must be enforced here against
/// the full stored event. Without this, an authorized engram search such as
/// `{"kinds":[30174],"#p":[self]}` would leak text-matching envelopes whose
/// `#p` belongs to a different owner — the NIP-AE read gate at the filter
/// layer would be bypassed for `/query`.
///
/// `accessible_channels` is the caller's channel scope; channel-scoped hits
/// outside that set are rejected regardless of NIP-01 match.
fn search_hit_accepted(
    filter: &nostr::Filter,
    stored: &buzz_core::StoredEvent,
    accessible_channels: &[uuid::Uuid],
    reader_pubkey_hex: &str,
) -> bool {
    if !buzz_core::filter::filters_match(std::slice::from_ref(filter), stored) {
        return false;
    }
    if let Some(ch_id) = stored.channel_id {
        if !accessible_channels.contains(&ch_id) {
            return false;
        }
    }
    if !buzz_core::filter::reader_authorized_for_event(&stored.event, reader_pubkey_hex) {
        return false;
    }
    true
}

/// Handle search filters by routing to Postgres FTS, then fetching full events
/// from DB. Supports a bridge-only `page` extension over the FTS result set.
async fn handle_bridge_search(
    state: &AppState,
    raw_filters: &[Value],
    filters: &[nostr::Filter],
    accessible_channels: &[uuid::Uuid],
    tenant: &buzz_core::tenant::TenantContext,
    reader_pubkey_hex: &str,
    pubkey_bytes: &[u8],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Bridge always includes global (channel-less) events — same as WS with
    // full scopes. `None` means no accessible channels and no global access →
    // empty result set (the caller short-circuits exactly as the WS door EOSEs).
    let channel_scope = match crate::handlers::req::build_search_channel_scope_filter(
        accessible_channels,
        true, // include_global
    ) {
        Some(scope) => scope,
        None => return Ok(Json(Value::Array(Vec::new()))),
    };

    let mut events: Vec<Value> = Vec::new();
    let mut seen_ids: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();

    for (raw, filter) in raw_filters.iter().zip(filters) {
        let search_mode = extract_search_mode(raw);
        let search_page = extract_search_page(raw);
        let search_text = match &filter.search {
            Some(s) if !s.is_empty() => s.clone(),
            _ => continue,
        };

        let limit = filter.limit.unwrap_or(100).min(500) as u32;
        if limit == 0 {
            continue;
        }

        // Scope by channel — push the #h tag (intersected with accessible
        // channels) if present, else the community-wide scope.
        let h_tag = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
        let filter_channel_scope =
            if let Some(vs) = filter.generic_tags.get(&h_tag).filter(|vs| !vs.is_empty()) {
                let valid: Vec<uuid::Uuid> = vs
                    .iter()
                    .filter_map(|v| v.parse::<uuid::Uuid>().ok())
                    .filter(|id| accessible_channels.contains(id))
                    .collect();
                if valid.is_empty() {
                    continue; // All #h values inaccessible — skip filter.
                }
                buzz_search::ChannelScope::Channels(valid)
            } else {
                channel_scope.clone()
            };

        let kinds = filter.kinds.as_ref().and_then(|ks| {
            if ks.is_empty() {
                None
            } else {
                Some(ks.iter().map(|k| k.as_u16() as i32).collect::<Vec<_>>())
            }
        });
        let authors = filter.authors.as_ref().and_then(|au| {
            if au.is_empty() {
                None
            } else {
                Some(au.iter().map(|a| a.to_bytes().to_vec()).collect::<Vec<_>>())
            }
        });
        let since = filter.since.map(|s| s.as_secs() as i64);
        let until = filter.until.map(|u| u.as_secs() as i64);

        let search_query = buzz_search::SearchQuery {
            community: tenant.community(),
            q: search_text,
            channel_scope: filter_channel_scope,
            kinds,
            authors,
            since,
            until,
            page: search_page,
            per_page: limit,
            mode: search_mode,
        };

        let search_result = state
            .search
            .search(&search_query)
            .await
            .map_err(|e| internal_error(&format!("search error: {e}")))?;

        // Fetch full events from DB by ID. Hit ids are already raw 32-byte
        // arrays from the FTS layer — no hex decode.
        let hit_ids: Vec<[u8; 32]> = search_result.hits.into_iter().map(|h| h.event_id).collect();

        if hit_ids.is_empty() {
            continue;
        }

        let id_refs: Vec<&[u8]> = hit_ids.iter().map(|b| b.as_slice()).collect();
        let stored_events = state
            .db
            .get_events_by_ids_routed("bridge_search_hydrate", tenant.community(), &id_refs)
            .await
            .map_err(|e| internal_error(&format!("search fetch error: {e}")))?;

        // Build lookup map to preserve FTS relevance ordering.
        let event_map: std::collections::HashMap<[u8; 32], &buzz_core::StoredEvent> = stored_events
            .iter()
            .map(|ev| (ev.event.id.to_bytes(), ev))
            .collect();

        for id_array in &hit_ids {
            let stored = match event_map.get(id_array) {
                Some(ev) => ev,
                None => continue,
            };
            if !search_hit_accepted(filter, stored, accessible_channels, reader_pubkey_hex) {
                continue;
            }
            // Defense-in-depth: apply the full per-event visibility gate, which
            // covers author-only kinds, the persona shared-gate (kind:30175), and
            // result-gated kinds. Kind:30175 is not in the FTS positive allowlist
            // today (migration 8 indexes only 0,9,40002,45001,45003), so this
            // branch cannot currently return unshared persona content — but the
            // check here ensures that a future FTS allowlist change cannot silently
            // reopen the bypass.
            if !crate::handlers::req::event_visible_to_reader(&stored.event, pubkey_bytes) {
                continue;
            }
            // Dedup across filters.
            if !seen_ids.insert(*id_array) {
                continue;
            }
            if let Ok(v) = serde_json::to_value(&stored.event) {
                events.push(v);
            }
        }
    }

    Ok(Json(Value::Array(events)))
}

/// Query parameters for the webhook trigger endpoint.
#[derive(serde::Deserialize)]
pub struct WebhookQuery {
    /// Webhook secret for authentication. Prefer the `X-Webhook-Secret` header instead.
    pub secret: Option<String>,
}

/// Webhook trigger endpoint. No user auth — the webhook secret authenticates the caller.
///
/// Prefers `X-Webhook-Secret` header over `?secret=` query param (headers aren't logged
/// by most proxies). Returns 202 Accepted; execution is async.
pub async fn workflow_webhook(
    State(state): State<Arc<AppState>>,
    Path(id_str): Path<String>,
    Query(query): Query<WebhookQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let id = uuid::Uuid::parse_str(&id_str)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workflow UUID"))?;

    // Row zero: bind this webhook to its community from the request host before
    // any tenant-scoped lookup or write. The host — not the workflow row —
    // determines the tenant: a request for community A's host may only reach
    // community A's workflows, even when the same workflow UUID also exists in
    // community B. Unmapped host, lookup failure, and a workflow that does not
    // exist in *this* community all fail closed with the same generic 404, so a
    // caller cannot probe which hosts or workflow ids exist on other tenants.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| not_found("workflow not found"))?;
    let community_id = tenant.community();

    let workflow = state
        .db
        .get_workflow(community_id, id)
        .await
        .map_err(|_| not_found("workflow not found"))?;

    let def: buzz_workflow::WorkflowDef = serde_json::from_value(workflow.definition.clone())
        .map_err(|e| super::internal_error(&format!("corrupt workflow definition: {e}")))?;

    if !matches!(def.trigger, buzz_workflow::TriggerDef::Webhook) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "workflow does not have a webhook trigger",
        ));
    }

    // Verify webhook secret. Prefer header (not logged by proxies); fall back to query param.
    let stored_secret = crate::webhook_secret::extract_secret(&workflow.definition);
    let provided_secret = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| query.secret.clone())
        .unwrap_or_default();

    match &stored_secret {
        Some(secret) => {
            if !crate::webhook_secret::verify_secret(&provided_secret, secret) {
                tracing::warn!("webhook: invalid secret for workflow {id}");
                return Err(api_error(StatusCode::UNAUTHORIZED, "authentication failed"));
            }
        }
        None => {
            return Err(api_error(
                StatusCode::UNAUTHORIZED,
                "webhook secret required but not configured — re-save the workflow to generate one",
            ));
        }
    }

    // Parse optional JSON body as trigger context.
    let body_json: Option<Value> =
        if body.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&body).map_err(|e| {
                api_error(StatusCode::BAD_REQUEST, &format!("invalid JSON body: {e}"))
            })?)
        };

    // Build trigger context from webhook body fields.
    let mut trigger_ctx = buzz_workflow::executor::TriggerContext {
        channel_id: workflow
            .channel_id
            .map(|ch| ch.to_string())
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(Value::Object(ref map)) = body_json {
        for (k, v) in map {
            let val_str = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            trigger_ctx.webhook_fields.insert(k.clone(), val_str);
        }
    }
    let trigger_ctx_json = serde_json::to_value(&trigger_ctx).ok();

    // SEC-006: the webhook secret authenticates the *caller*, but the run
    // executes with the workflow **owner's** standing authority — so the
    // secret alone is insufficient. Immediately before run creation, reject
    // disabled/inactive workflows and recheck the owner's current channel
    // membership (and role, for exfiltration-capable definitions). Fail
    // closed with the same generic 404 as the lookups above so a
    // revoked-owner workflow is indistinguishable from a nonexistent one.
    if !workflow.enabled || workflow.status != buzz_db::workflow::WorkflowStatus::Active {
        return Err(not_found("workflow not found"));
    }
    let Some(wf_channel_id) = workflow.channel_id else {
        // No channel scope means no channel authority to verify — fail closed.
        return Err(not_found("workflow not found"));
    };
    state
        .workflow_engine
        .check_owner_authority(community_id, wf_channel_id, &workflow.owner_pubkey, &def)
        .await
        .map_err(|_| not_found("workflow not found"))?;

    let run_id = state
        .db
        .create_workflow_run(community_id, id, None, trigger_ctx_json.as_ref())
        .await
        .map_err(|e| super::internal_error(&format!("db error: {e}")))?;

    // Spawn workflow execution asynchronously.
    let engine = Arc::clone(&state.workflow_engine);
    let db = state.db.clone();
    let def_value = workflow.definition.clone();
    let trigger_ctx_clone = trigger_ctx.clone();
    tokio::spawn(async move {
        let def: buzz_workflow::WorkflowDef = match serde_json::from_value(def_value) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("webhook: failed to parse definition: {e}");
                if let Err(db_err) = db
                    .update_workflow_run(
                        community_id,
                        run_id,
                        buzz_db::workflow::RunStatus::Failed,
                        0,
                        &serde_json::json!([]),
                        Some(buzz_db::workflow::WorkflowRunFailure {
                            code: "invalid_definition",
                            message: &format!("definition parse error: {e}"),
                        }),
                    )
                    .await
                {
                    tracing::error!("webhook: failed to mark run as failed: {db_err}");
                }
                return;
            }
        };

        let result = buzz_workflow::executor::execute_from_step(
            &engine,
            community_id,
            run_id,
            &def,
            &trigger_ctx_clone,
            0,
            None,
        )
        .await;
        engine
            .finalize_run(community_id, run_id, result, None)
            .await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "run_id": run_id.to_string(),
            "workflow_id": id.to_string(),
            "status": "pending",
        })),
    ))
}

/// If all filters target kind:20001 or kind:40902 with authors, synthesize
/// presence from Redis instead of querying the DB (ephemeral events are never
/// stored, and kind:40902 snapshots are relay-generated on demand).
///
/// Returns `None` when the filters are not a presence query (fall through to
/// the normal query path). Returns `Some(Ok(events))` when a presence snapshot
/// was produced — an empty vec is an authoritative "all offline" answer.
/// Returns `Some(Err(_))` when the backing Redis lookup failed: callers must
/// propagate that as an error response rather than a fake-empty success, so a
/// consumer cannot mistake a backend outage for an authoritative snapshot.
#[allow(clippy::type_complexity)]
async fn synthesize_presence(
    pubsub: &buzz_pubsub::PubSubManager,
    relay_keypair: &nostr::Keys,
    tenant: &buzz_core::tenant::TenantContext,
    filters: &[nostr::Filter],
) -> Option<Result<Vec<Value>, (StatusCode, Json<Value>)>> {
    use buzz_core::kind::{KIND_PRESENCE_SNAPSHOT, KIND_PRESENCE_UPDATE};

    // Only intercept if every filter targets kind:20001 or 40902 with authors.
    let mut all_pubkeys: Vec<nostr::PublicKey> = Vec::new();
    for filter in filters {
        let kinds = filter.kinds.as_ref()?;
        let only_kind = kinds.iter().next()?;
        let k = only_kind.as_u16() as u32;
        if kinds.len() != 1 || (k != KIND_PRESENCE_UPDATE && k != KIND_PRESENCE_SNAPSHOT) {
            return None;
        }
        let authors = filter.authors.as_ref()?;
        if authors.is_empty() {
            return None;
        }
        all_pubkeys.extend(authors.iter().copied());
    }

    if all_pubkeys.is_empty() {
        return Some(Ok(Vec::new()));
    }

    // Dedup pubkeys.
    all_pubkeys.sort_by_key(|pk| pk.to_hex());
    all_pubkeys.dedup();

    // Look up Redis. A lookup failure must surface as an error, not a
    // fake-empty success — otherwise a Redis outage is indistinguishable from
    // an authoritative all-offline snapshot to the consumer.
    let presence_map = match pubsub.get_presence_bulk(tenant, &all_pubkeys).await {
        Ok(map) => map,
        Err(e) => return Some(Err(internal_error(&format!("presence lookup: {e}")))),
    };

    if presence_map.is_empty() {
        return Some(Ok(Vec::new()));
    }

    // Synthesize kind:20001 events signed by the relay.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut events = Vec::with_capacity(presence_map.len());
    for (pubkey_hex, status) in &presence_map {
        // Build a synthetic event: relay-signed, content = status, p-tag = subject.
        // A build/sign failure here is an internal fault, not a "not a presence
        // query" signal, so surface it as an error rather than falling through.
        let tags = match nostr::Tag::parse(["p", pubkey_hex]) {
            Ok(tag) => vec![tag],
            Err(e) => return Some(Err(internal_error(&format!("presence tag: {e}")))),
        };
        let event = match nostr::EventBuilder::new(
            nostr::Kind::Custom(KIND_PRESENCE_UPDATE as u16),
            status,
        )
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(now))
        .sign_with_keys(relay_keypair)
        {
            Ok(event) => event,
            Err(e) => return Some(Err(internal_error(&format!("presence sign: {e}")))),
        };

        if let Ok(v) = serde_json::to_value(&event) {
            events.push(v);
        }
    }

    Some(Ok(events))
}

// ── Moderation queue reads (L6 — Quinn) ───────────────────────────────────────
//
// Mod-only structured rows (`moderation_reports`/`moderation_actions`/
// `community_bans`) are not nostr events, so they are served over dedicated
// NIP-98-authed GET endpoints rather than the REQ/`/query` path (which would
// force a synthetic event shape and thread a privileged branch onto the shared
// read hot path). Gated on `ModerationAction::ViewQueue` via the one capability
// helper — never an inline role check. Host-scoped: community from the request
// host, no channel context (queue reads are community-wide).

/// Shared prelude for a moderation read: bind tenant, verify NIP-98 GET auth,
/// replay-check, and confirm the caller may view the queue.
///
/// `raw_query` is the request's raw query string (from [`axum::extract::RawQuery`]),
/// e.g. `Some("limit=20&status=open")`. NIP-98 signs the *full* request URL, so the
/// client's `u` tag includes any query string; the expected URL reconstructed here
/// must therefore append the same query verbatim or query-bearing reads
/// (`reports?limit=…`, `audit?limit=…`) 401 on a URL mismatch. Query-less reads
/// (`restricted`) pass `None` and keep the bare-path expectation. The verbatim
/// request query is used (not a re-serialized parse) so the match stays byte-exact
/// with what the client signed regardless of param order or encoding.
#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn authorize_moderation_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
) -> Result<TenantContext, Response> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
            .into_response()
        })?;

    let path_with_query = match raw_query {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let url = nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    // In NIP-FI enforce/deny-protected mode a real NIP-98 event is mandatory —
    // the X-Pubkey dev-mode fallback must never satisfy the pairing requirement.
    // [NIP-FI.md:594-607, FI-TRACE-HTTP-INGRESS]
    let nip_fi_active = !matches!(state.config.nip_fi.mode, NipFiMode::Off);

    // NIP-FI admission. [FI-TRACE-AUTHORITY-UNIFORM]
    let admission = admit_nip_fi_http_on_state(state, headers, || {
        verify_bridge_auth(
            headers,
            "GET",
            &url,
            None,
            state.config.require_auth_token || nip_fi_active,
        )
        .map(|auth| Nip98Proof::new(auth.pubkey, auth.event_id_bytes))
        .map_err(|e| e.into_response())
    })?;
    let pubkey = *admission.proven_pubkey();
    let event_id_bytes = admission.into_extra();

    check_nip98_replay(state, &tenant, event_id_bytes)
        .await
        .map_err(|e| e.into_response())?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    crate::handlers::moderation_authz::authorize_moderation_action(
        &tenant,
        state,
        &pubkey_bytes,
        None,
        crate::handlers::moderation_authz::ModerationTarget::None,
        crate::handlers::moderation_authz::ModerationAction::ViewQueue,
    )
    .await
    .map_err(|_| {
        api_error(
            StatusCode::FORBIDDEN,
            "restricted: moderator access required",
        )
        .into_response()
    })?;

    Ok(tenant)
}

/// Cap on rows returned by a single moderation read.
const MODERATION_READ_LIMIT: i64 = 500;

/// Optional `?status=` and `?limit=` query for moderation reads.
#[derive(serde::Deserialize, Default)]
pub struct ModerationReadQuery {
    status: Option<String>,
    limit: Option<i64>,
}

fn clamp_limit(requested: Option<i64>) -> i64 {
    requested
        .filter(|n| *n > 0)
        .map(|n| n.min(MODERATION_READ_LIMIT))
        .unwrap_or(MODERATION_READ_LIMIT)
}

/// `GET /moderation/reports` — the moderation queue (NIP-98 + mod-authz).
pub async fn moderation_reports(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let tenant = match authorize_moderation_read(
        &state,
        &headers,
        "/moderation/reports",
        raw_query.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(r) => return r,
    };
    // Parse query after admission so malformed params cannot 400 before the
    // NIP-FI gate fires.  A parse failure after admission is a caller error
    // (400), not an auth failure; defaulting silently would change query
    // semantics (e.g. drop a valid `status=` together with a bad `limit=`).
    // [FI-TRACE-HTTP-INGRESS]
    let q: ModerationReadQuery = match parse_query_or_400(raw_query.as_deref()) {
        Ok(q) => q,
        Err(e) => return e.into_response(),
    };
    match state
        .db
        .list_moderation_reports(
            tenant.community(),
            q.status.as_deref(),
            clamp_limit(q.limit),
        )
        .await
    {
        Ok(rows) => Json(Value::Array(rows.iter().map(report_json).collect())).into_response(),
        Err(e) => internal_error(&format!("list reports: {e}")).into_response(),
    }
}

/// `GET /moderation/audit` — the moderation audit log (NIP-98 + mod-authz).
pub async fn moderation_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let tenant = match authorize_moderation_read(
        &state,
        &headers,
        "/moderation/audit",
        raw_query.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(r) => return r,
    };
    // Parse query after admission so malformed params cannot 400 before the
    // NIP-FI gate fires.  A parse failure after admission is a caller error
    // (400), not an auth failure; defaulting silently would change query
    // semantics.  [FI-TRACE-HTTP-INGRESS]
    let q: ModerationReadQuery = match parse_query_or_400(raw_query.as_deref()) {
        Ok(q) => q,
        Err(e) => return e.into_response(),
    };
    match state
        .db
        .list_moderation_actions(tenant.community(), clamp_limit(q.limit))
        .await
    {
        Ok(rows) => Json(Value::Array(rows.iter().map(action_json).collect())).into_response(),
        Err(e) => internal_error(&format!("list actions: {e}")).into_response(),
    }
}

/// `GET /moderation/restricted` — currently banned/timed-out members.
pub async fn moderation_restricted(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let tenant =
        match authorize_moderation_read(&state, &headers, "/moderation/restricted", None).await {
            Ok(t) => t,
            Err(r) => return r,
        };
    match state
        .db
        .list_community_restrictions(tenant.community())
        .await
    {
        Ok(rows) => Json(Value::Array(rows.iter().map(ban_json).collect())).into_response(),
        Err(e) => internal_error(&format!("list restrictions: {e}")).into_response(),
    }
}

fn report_json(r: &buzz_db::moderation::ReportRecord) -> Value {
    let (target_kind, target) = match &r.target {
        buzz_db::moderation::ReportTarget::Event(id) => ("event", hex::encode(id)),
        buzz_db::moderation::ReportTarget::Pubkey(pk) => ("pubkey", hex::encode(pk)),
        buzz_db::moderation::ReportTarget::Blob(sha) => ("blob", hex::encode(sha)),
    };
    serde_json::json!({
        "id": r.id,
        "report_event_id": hex::encode(&r.report_event_id),
        "reporter_pubkey": hex::encode(&r.reporter_pubkey),
        "target_kind": target_kind,
        "target": target,
        "channel_id": r.channel_id,
        "report_type": r.report_type,
        "note": r.note,
        "status": r.status,
        "resolved_by": r.resolved_by.as_ref().map(hex::encode),
        "resolved_at": r.resolved_at,
        "action_id": r.action_id,
        "created_at": r.created_at,
    })
}

fn action_json(a: &buzz_db::moderation::ActionRecord) -> Value {
    serde_json::json!({
        "id": a.id,
        "actor_pubkey": hex::encode(&a.actor_pubkey),
        "action": a.action,
        "target_pubkey": a.target_pubkey.as_ref().map(hex::encode),
        "target_event_id": a.target_event_id.as_ref().map(hex::encode),
        "channel_id": a.channel_id,
        "reason_code": a.reason_code,
        "public_reason": a.public_reason,
        "private_reason": a.private_reason,
        "matched_principal": a.matched_principal,
        "created_at": a.created_at,
    })
}

fn ban_json(b: &buzz_db::moderation::BanRecord) -> Value {
    serde_json::json!({
        "pubkey": hex::encode(&b.pubkey),
        "banned": b.banned,
        "ban_expires_at": b.ban_expires_at,
        "ban_reason": b.ban_reason,
        "muted_until": b.muted_until,
        "mute_reason": b.mute_reason,
        "actor_pubkey": hex::encode(&b.actor_pubkey),
        "updated_at": b.updated_at,
    })
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::{Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag};
    use std::sync::Mutex;

    fn redis_pool() -> deadpool_redis::Pool {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        deadpool_redis::Config::from_url(url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("create redis pool")
    }

    fn fresh_tenant(host: &str) -> TenantContext {
        TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(uuid::Uuid::new_v4()),
            host,
        )
    }

    fn fresh_nip98_event_id_bytes() -> [u8; 32] {
        EventBuilder::new(Kind::HttpAuth, "")
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event")
            .id
            .to_bytes()
    }

    #[test]
    fn bridge_detects_mixed_search_and_non_search_filters() {
        let filters = vec![
            nostr::Filter::new().search("hello"),
            nostr::Filter::new().kind(Kind::TextNote),
        ];

        assert!(has_mixed_search_filters(&filters));
    }

    #[test]
    fn bridge_accepts_all_search_filters() {
        let filters = vec![
            nostr::Filter::new().search("hello"),
            nostr::Filter::new().search("world"),
        ];

        assert!(!has_mixed_search_filters(&filters));
    }

    #[test]
    fn bridge_accepts_all_non_search_filters() {
        let filters = vec![
            nostr::Filter::new().kind(Kind::TextNote),
            nostr::Filter::new().kind(Kind::Metadata),
        ];

        assert!(!has_mixed_search_filters(&filters));
    }

    /// Production-wiring seam for the Redis-outage boundary. Drives the real
    /// `synthesize_presence` with a `PubSubManager` whose pool points at a
    /// closed port, so the `get_presence_bulk` lookup fails. A presence-snapshot
    /// filter must yield `Some(Err(500))` — never `Some(Ok([]))`, which would
    /// let a consumer mistake a backend outage for an authoritative all-offline
    /// snapshot. Restoring `unwrap_or_default()` inside `synthesize_presence`
    /// turns this red (it would return `Some(Ok([]))`), which is what protects
    /// the error-mapping seam Thufir found otherwise mutation-unprotected.
    #[tokio::test]
    async fn synthesize_presence_surfaces_redis_failure_as_error_response() {
        use buzz_core::kind::KIND_PRESENCE_SNAPSHOT;

        // Pool at a closed port: get_presence_bulk's connection attempt fails.
        let dead_pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("pool builds lazily");
        let pubsub = buzz_pubsub::PubSubManager::new("redis://127.0.0.1:1", dead_pool)
            .await
            .expect("PubSubManager::new performs no IO");
        let relay_keypair = Keys::generate();
        let tenant = fresh_tenant("relay.example");

        // A presence-snapshot query for a concrete author reaches the Redis
        // lookup (an empty author set would short-circuit to an empty snapshot).
        let filters = vec![nostr::Filter::new()
            .kind(Kind::Custom(KIND_PRESENCE_SNAPSHOT as u16))
            .author(Keys::generate().public_key())];

        let result = synthesize_presence(&pubsub, &relay_keypair, &tenant, &filters).await;

        match result {
            Some(Err((status, _))) => assert_eq!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "a Redis lookup failure must surface as HTTP 500"
            ),
            other => panic!(
                "a Redis outage must yield Some(Err(500)), not a fake-empty success: {other:?}"
            ),
        }
    }

    #[test]
    fn thread_aux_query_targets_root_and_replies() {
        let tenant = fresh_tenant("relay.example");
        let targets = vec!["root".to_string(), "reply".to_string()];
        let query = build_aux_query(tenant.community(), targets.clone(), &WINDOW_AUX_KINDS);

        assert_eq!(query.e_tags, Some(targets));
        assert_eq!(
            query.kinds,
            Some(WINDOW_AUX_KINDS.iter().map(|kind| *kind as i32).collect())
        );
        assert_eq!(query.limit, None);
        assert_eq!(query.until, None);
        assert_eq!(query.before_id, None);
    }

    fn aux_event(keys: &Keys, created_at: u64, content: &str) -> buzz_core::StoredEvent {
        let ev = EventBuilder::new(Kind::Custom(7), content)
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap();
        buzz_core::StoredEvent::new(ev, None)
    }

    /// Carl/#6572: a one-shot `limit=1000` aux query is newest-first, so the
    /// oldest reactions/edits/deletions past the clamp vanished. The paged
    /// drain must walk the keyset cursor until a short page and return every
    /// event exactly once.
    #[tokio::test]
    async fn query_all_pages_drains_past_the_page_clamp() {
        let keys = Keys::generate();
        // Newest-first store: 5 events, two sharing a second so the id
        // tiebreak is exercised.
        let mut store = [
            aux_event(&keys, 50, "e"),
            aux_event(&keys, 40, "d1"),
            aux_event(&keys, 40, "d2"),
            aux_event(&keys, 30, "c"),
            aux_event(&keys, 10, "a"),
        ];
        store.sort_by(|l, r| {
            r.event
                .created_at
                .cmp(&l.event.created_at)
                .then(l.event.id.cmp(&r.event.id))
        });
        let expected: Vec<_> = store.iter().map(|se| se.event.id).collect();
        let mut calls = Vec::new();

        let tenant = fresh_tenant("relay.example");
        let query = build_aux_query(tenant.community(), vec!["root".into()], &WINDOW_AUX_KINDS);
        let mut fetch = |q: &buzz_db::EventQuery| {
            calls.push((q.limit, q.until, q.before_id.clone()));
            // Emulate `query_events_on`: `created_at < until OR
            // (created_at = until AND id > before_id)`, newest-first, limit.
            let page: Vec<_> = store
                .iter()
                .filter(|se| match (q.until, q.before_id.as_deref()) {
                    (Some(until), Some(before)) => {
                        let ts = se.event.created_at.as_secs() as i64;
                        ts < until.timestamp()
                            || (ts == until.timestamp()
                                && se.event.id.as_bytes().as_slice() > before)
                    }
                    _ => true,
                })
                .take(q.limit.unwrap() as usize)
                .cloned()
                .collect();
            page
        };
        let events = query_all_pages(query, 2, &mut AuxReader::Fake(&mut fetch))
            .await
            .unwrap();

        assert_eq!(
            events.iter().map(|se| se.event.id).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(calls.len(), 3, "2 full pages + 1 short page");
        assert!(calls.iter().all(|(limit, _, _)| *limit == Some(2)));
        assert_eq!(calls[0].1, None);
        // Second page resumes from the last row of the first (ts 40, larger id).
        assert_eq!(calls[1].1.unwrap().timestamp(), 40);
        assert_eq!(
            calls[1].2.as_deref(),
            Some(store[1].event.id.as_bytes().as_slice())
        );
        assert_eq!(calls[2].1.unwrap().timestamp(), 30);
    }

    #[tokio::test]
    async fn query_all_pages_stops_at_one_short_page() {
        let tenant = fresh_tenant("relay.example");
        let query = build_aux_query(tenant.community(), vec!["root".into()], &WINDOW_AUX_KINDS);
        let mut calls = 0;
        let mut fetch = |_q: &buzz_db::EventQuery| {
            calls += 1;
            Vec::new()
        };
        let events = query_all_pages(query, 1000, &mut AuxReader::Fake(&mut fetch))
            .await
            .unwrap();
        assert!(events.is_empty());
        assert_eq!(calls, 1);
    }

    #[test]
    fn bridge_search_mode_extension_defaults_to_full_text() {
        assert_eq!(
            extract_search_mode(&serde_json::json!({ "search": "pro" })),
            buzz_search::SearchMode::FullText
        );
        assert_eq!(
            extract_search_mode(&serde_json::json!({ "search": "pro", "search_mode": "word" })),
            buzz_search::SearchMode::FullText
        );
    }

    #[test]
    fn bridge_search_mode_extension_accepts_prefix_snake_or_camel_case() {
        assert_eq!(
            extract_search_mode(&serde_json::json!({ "search": "pro", "search_mode": "prefix" })),
            buzz_search::SearchMode::Prefix
        );
        assert_eq!(
            extract_search_mode(&serde_json::json!({ "search": "pro", "searchMode": "prefix" })),
            buzz_search::SearchMode::Prefix
        );
    }

    /// Attack 3 proof: two stateless relay pods sharing Redis must share one
    /// community-scoped NIP-98 seen-set. Pod A's first claim succeeds; pod B's
    /// replay of the same event id in the same community is rejected. The same
    /// id in a different community still succeeds, proving the key is scoped by
    /// server-resolved tenant rather than global process memory.
    async fn nip98_replay_guard_rejects_cross_pod_replay_on_bridge_path() {
        let pool = redis_pool();
        let pod_a = buzz_pubsub::RedisNip98ReplayGuard::new(pool.clone());
        let pod_b = buzz_pubsub::RedisNip98ReplayGuard::new(pool);
        let tenant_a = fresh_tenant("relay-a.example");
        let tenant_b = fresh_tenant("relay-b.example");
        let event_id_bytes = fresh_nip98_event_id_bytes();

        check_nip98_replay_with_guard(&pod_a, &tenant_a, event_id_bytes)
            .await
            .expect("first pod should claim fresh NIP-98 event id");

        let (status, _) = check_nip98_replay_with_guard(&pod_b, &tenant_a, event_id_bytes)
            .await
            .expect_err("second pod must reject same-community replay");
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        check_nip98_replay_with_guard(&pod_b, &tenant_b, event_id_bytes)
            .await
            .expect("same event id in a different community uses a distinct seen-set");
    }

    /// Attack 3 same-pod regression guard: replacing the process-local moka
    /// cache with a shared Redis seen-set must not weaken same-pod replay
    /// rejection. A single guard instance, called twice with the same
    /// `TenantContext` and the same event id, MUST reject the second call.
    /// Bites if `try_mark`'s admit/reject mapping is reversed or no-op'd.
    async fn nip98_replay_guard_rejects_same_pod_same_community_replay() {
        let pool = redis_pool();
        let pod = buzz_pubsub::RedisNip98ReplayGuard::new(pool);
        let tenant = fresh_tenant("relay-a.example");
        let event_id_bytes = fresh_nip98_event_id_bytes();

        check_nip98_replay_with_guard(&pod, &tenant, event_id_bytes)
            .await
            .expect("first claim on a fresh event id must succeed");

        let (status, _) = check_nip98_replay_with_guard(&pod, &tenant, event_id_bytes)
            .await
            .expect_err("same-pod replay of the same id+community must reject");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    mod external_infra_redis_tests {
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn nip98_replay_guard_rejects_cross_pod_replay_on_bridge_path() {
            super::nip98_replay_guard_rejects_cross_pod_replay_on_bridge_path().await;
        }

        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn nip98_replay_guard_rejects_same_pod_same_community_replay() {
            super::nip98_replay_guard_rejects_same_pod_same_community_replay().await;
        }
    }

    /// Attack 3 fail-closed guard: a stateless worker that loses Redis MUST
    /// reject the request, never admit it. The shared seen-set is the
    /// freshness fence; degrading to "best effort, allow on error" forfeits
    /// the proof (per the `Nip98ReplayGuard` trait contract,
    /// `buzz-auth/src/nip98_replay.rs:70-73`).
    ///
    /// This test does not require Redis — it injects a guard that always
    /// returns `Err`, exercising the `Err =>` arm in
    /// `check_nip98_replay_with_guard` directly. Bites if the arm is changed
    /// to admit (`Ok(())` / `Ok(true)`) instead of returning 401.
    #[tokio::test]
    async fn nip98_replay_check_fails_closed_when_guard_errors() {
        use buzz_auth::AuthError;
        use nostr::EventId;
        use std::future::Future;
        use std::pin::Pin;

        struct AlwaysErrGuard;
        impl Nip98ReplayGuard for AlwaysErrGuard {
            fn try_mark_in_scope<'a>(
                &'a self,
                _scope: &'a str,
                _event_id: &'a EventId,
                _ttl_secs: u64,
            ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
                Box::pin(async {
                    Err(AuthError::Internal(
                        "simulated Redis pool acquire failure".into(),
                    ))
                })
            }
        }

        let guard = AlwaysErrGuard;
        let tenant = fresh_tenant("relay-a.example");
        let event_id_bytes = fresh_nip98_event_id_bytes();

        let (status, body) = check_nip98_replay_with_guard(&guard, &tenant, event_id_bytes)
            .await
            .expect_err("guard error MUST fail closed, never admit");
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "fail-closed must return 401"
        );
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            msg.contains("replay check unavailable"),
            "fail-closed body must carry the unavailable signal so callers can \
             distinguish unavailability from replay; got body = {body:?}"
        );
    }

    /// Build a signed NIP-98 event JSON string for `url` + `method`, mirroring
    /// `buzz_auth::nip98::tests::make_nip98_event` so the bridge tests don't
    /// reach into buzz-auth's test scope.
    fn build_nip98_event_json(keys: &Keys, url: &str, method: &str) -> String {
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        serde_json::to_string(&event).expect("serialize")
    }

    /// Build a `HeaderMap` with the NIP-98 event base64-encoded in
    /// `Authorization: Nostr <base64>`, matching the production bridge auth
    /// header shape.
    fn nip98_auth_headers(event_json: &str) -> axum::http::HeaderMap {
        use base64::engine::general_purpose::STANDARD as BASE64;
        let mut headers = axum::http::HeaderMap::new();
        let value = format!("Nostr {}", BASE64.encode(event_json.as_bytes()));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            value.parse().expect("valid header value"),
        );
        headers
    }

    /// Row 44 obligation: a NIP-98 event signed against community A's host
    /// MUST be rejected at the bridge when the request resolves to community
    /// B's host. The conformance text in `docs/multi-tenant-conformance.md`
    /// states: "NIP-98 `u` URL host must match `req.community`". Before this
    /// gap closed, `expected_url` was derived from `state.config.relay_url`
    /// (one static string per deployment), so any request to *any* host on a
    /// multi-tenant deployment would verify against community A's URL — both
    /// admitting cross-host forgeries (event signed for A presented at B) and
    /// rejecting every legitimate request whose community host wasn't the
    /// single configured one.
    ///
    /// This test bites if `nip98_expected_url` is reverted to use
    /// `config.relay_url`'s host (the original `canonical_url` behavior).
    #[test]
    fn verify_bridge_auth_rejects_nip98_event_signed_for_wrong_communitys_host() {
        let keys = Keys::generate();
        // Client signs an event for community A's host, then presents it at a
        // request whose `Host` header resolved to community B.
        let signed_url = "https://host-a.example/events";
        let event_json = build_nip98_event_json(&keys, signed_url, "POST");
        let headers = nip98_auth_headers(&event_json);

        let config_relay_url = "wss://host-a.example"; // doesn't matter — only used for scheme.
        let tenant_b = fresh_tenant("host-b.example");
        let expected_url = nip98_expected_url(config_relay_url, &tenant_b, "/events");

        let (status, body) = verify_bridge_auth(&headers, "POST", &expected_url, Some(b""), true)
            .expect_err(
                "cross-host NIP-98 event MUST be rejected — row 44: `u` URL host \
                 must match req.community",
            );
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "cross-host rejection must be a 401, not silently admitted"
        );
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            msg.contains("URL mismatch"),
            "rejection must carry the URL-mismatch signal so callers can \
             distinguish it from other auth failures; got body = {body:?}"
        );
    }

    #[test]
    fn verify_bridge_auth_can_require_payload_tag_for_json_body_endpoints() {
        let keys = Keys::generate();
        let signed_url = "https://host-a.example/operator/communities";
        let event_json = build_nip98_event_json(&keys, signed_url, "POST");
        let headers = nip98_auth_headers(&event_json);

        let (status, body) = verify_bridge_auth_with_options(
            &headers,
            "POST",
            signed_url,
            Some(br#"{"host":"created.example"}"#),
            true,
            true,
        )
        .expect_err("body-bearing operator requests must require a payload tag");

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            msg.contains("missing payload tag"),
            "rejection should explain the payload binding failure; got body = {body:?}"
        );
    }

    /// Positive control for the cross-host test: a NIP-98 event signed for
    /// host A MUST be accepted at a request whose tenant resolved to host A.
    /// Without this, the cross-host test could be passing vacuously (e.g. if
    /// `nip98_expected_url` always produced a URL no event could match).
    #[test]
    fn verify_bridge_auth_accepts_nip98_event_signed_for_matching_host() {
        let keys = Keys::generate();
        let signed_url = "https://host-a.example/events";
        let event_json = build_nip98_event_json(&keys, signed_url, "POST");
        let headers = nip98_auth_headers(&event_json);

        // Configured relay URL deliberately differs in host from the request's
        // tenant host — proving the helper uses `tenant.host()`, not the config.
        let config_relay_url = "wss://other-config-host.example";
        let tenant_a = fresh_tenant("host-a.example");
        let expected_url = nip98_expected_url(config_relay_url, &tenant_a, "/events");

        let VerifiedBridgeAuth {
            pubkey,
            signed_created_at,
            ..
        } = verify_bridge_auth(&headers, "POST", &expected_url, Some(b""), true)
            .expect("matching-host NIP-98 event must verify");
        assert_eq!(
            pubkey,
            keys.public_key(),
            "returned pubkey must be the signer's"
        );
        assert!(
            signed_created_at.is_some(),
            "verified NIP-98 auth must retain its signed timestamp"
        );
    }

    /// Mirror of the query-reconstruction `authorize_moderation_read` performs
    /// before calling [`nip98_expected_url`], so the tests below pin the exact
    /// seam without a DB harness. Kept in lockstep with the production match arm.
    fn moderation_read_expected_url(
        config_relay_url: &str,
        tenant: &TenantContext,
        path: &str,
        raw_query: Option<&str>,
    ) -> String {
        let path_with_query = match raw_query {
            Some(q) if !q.is_empty() => format!("{path}?{q}"),
            _ => path.to_string(),
        };
        nip98_expected_url(config_relay_url, tenant, &path_with_query)
    }

    /// L7 read-auth blocker (Wren, #1591 sweep): the CLI signs the *full*
    /// request URL — including `?limit=…&status=…` — but the relay used to
    /// reconstruct the expected URL from the bare path only, so
    /// `buzz moderation reports` / `audit` 401'd on a NIP-98 URL mismatch in
    /// normal use. This pins that a query-bearing GET verifies iff the expected
    /// URL carries the same query verbatim. Bites if the query is ever dropped
    /// from `authorize_moderation_read`'s expected-URL reconstruction.
    #[test]
    fn moderation_read_query_bearing_nip98_event_verifies_with_matching_query() {
        let keys = Keys::generate();
        // CLI signs the URL it actually requests, query and all.
        let signed_url = "https://host-a.example/moderation/reports?limit=20&status=open";
        let event_json = build_nip98_event_json(&keys, signed_url, "GET");
        let headers = nip98_auth_headers(&event_json);

        let tenant_a = fresh_tenant("host-a.example");
        let expected_url = moderation_read_expected_url(
            "wss://config-host.example",
            &tenant_a,
            "/moderation/reports",
            Some("limit=20&status=open"),
        );

        let VerifiedBridgeAuth { pubkey, .. } =
            verify_bridge_auth(&headers, "GET", &expected_url, None, true)
                .expect("query-bearing moderation read must verify against the same query");
        assert_eq!(pubkey, keys.public_key());
    }

    /// Anti-regression control proving the fix is load-bearing: the same
    /// query-bearing event MUST be rejected when the expected URL omits the
    /// query — the pre-fix behavior. If this ever passes, the relay has
    /// silently reverted to bare-path reconstruction.
    #[test]
    fn moderation_read_query_bearing_nip98_event_rejected_against_bare_path() {
        let keys = Keys::generate();
        let signed_url = "https://host-a.example/moderation/reports?limit=20&status=open";
        let event_json = build_nip98_event_json(&keys, signed_url, "GET");
        let headers = nip98_auth_headers(&event_json);

        let tenant_a = fresh_tenant("host-a.example");
        // No query — the broken pre-fix reconstruction.
        let bare_url = moderation_read_expected_url(
            "wss://config-host.example",
            &tenant_a,
            "/moderation/reports",
            None,
        );

        let (status, body) = verify_bridge_auth(&headers, "GET", &bare_url, None, true)
            .expect_err("query-signed event MUST NOT match a bare-path expected URL");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            msg.contains("URL mismatch"),
            "rejection must be a URL mismatch; got body = {body:?}"
        );
    }

    /// `audit?limit=20` — the second query-bearing read path — verifies the
    /// same way. Pins that the reconstruction is generic over the path, not
    /// special-cased to `reports`.
    #[test]
    fn moderation_read_audit_query_bearing_nip98_event_verifies() {
        let keys = Keys::generate();
        let signed_url = "https://host-a.example/moderation/audit?limit=20";
        let event_json = build_nip98_event_json(&keys, signed_url, "GET");
        let headers = nip98_auth_headers(&event_json);

        let tenant_a = fresh_tenant("host-a.example");
        let expected_url = moderation_read_expected_url(
            "wss://config-host.example",
            &tenant_a,
            "/moderation/audit",
            Some("limit=20"),
        );

        let VerifiedBridgeAuth { pubkey, .. } =
            verify_bridge_auth(&headers, "GET", &expected_url, None, true)
                .expect("audit query-bearing read must verify");
        assert_eq!(pubkey, keys.public_key());
    }

    /// `restricted` has no query and passes `None`, so its expected URL stays
    /// the bare path — a query-less signed event verifies. Pins Wren's
    /// "preserve restricted no-query behavior" checklist item.
    #[test]
    fn moderation_read_restricted_no_query_still_verifies() {
        let keys = Keys::generate();
        let signed_url = "https://host-a.example/moderation/restricted";
        let event_json = build_nip98_event_json(&keys, signed_url, "GET");
        let headers = nip98_auth_headers(&event_json);

        let tenant_a = fresh_tenant("host-a.example");
        let expected_url = moderation_read_expected_url(
            "wss://config-host.example",
            &tenant_a,
            "/moderation/restricted",
            None,
        );
        assert_eq!(expected_url, "https://host-a.example/moderation/restricted");

        let VerifiedBridgeAuth { pubkey, .. } =
            verify_bridge_auth(&headers, "GET", &expected_url, None, true)
                .expect("query-less restricted read must verify against the bare path");
        assert_eq!(pubkey, keys.public_key());
    }

    /// `nip98_expected_url` derives host from `tenant`, not from
    /// `config_relay_url`. Pin both directions: changing the tenant's host
    /// changes the output; changing the config's host does NOT.
    #[test]
    fn nip98_expected_url_uses_tenant_host_not_config_host() {
        let tenant_a = fresh_tenant("host-a.example");
        let tenant_b = fresh_tenant("host-b.example");

        let url_a = nip98_expected_url("wss://config-host.example", &tenant_a, "/events");
        let url_b = nip98_expected_url("wss://config-host.example", &tenant_b, "/events");
        assert_eq!(url_a, "https://host-a.example/events");
        assert_eq!(url_b, "https://host-b.example/events");

        // Same tenant, two different config hosts → output is identical.
        // (If config-host ever leaked into the URL, this assertion would bite.)
        let url_a_alt_config =
            nip98_expected_url("wss://different-config.example", &tenant_a, "/events");
        assert_eq!(
            url_a, url_a_alt_config,
            "config-relay-url's host MUST NOT influence the NIP-98 expected URL — \
             only its scheme contributes"
        );
    }

    /// `nip98_expected_url` derives scheme from `config_relay_url`'s prefix:
    /// `wss://` → `https`, everything else → `http`. Deployments that run
    /// `ws://` in dev/test still need a NIP-98 URL the client can sign against.
    #[test]
    fn nip98_expected_url_derives_scheme_from_config() {
        let tenant = fresh_tenant("host-a.example");
        assert_eq!(
            nip98_expected_url("wss://config.example", &tenant, "/events"),
            "https://host-a.example/events",
            "wss:// production config → https:// URL"
        );
        assert_eq!(
            nip98_expected_url("ws://config.example", &tenant, "/events"),
            "http://host-a.example/events",
            "ws:// dev config → http:// URL"
        );
    }

    // ----- NIP-42 host-binding tests (sibling of NIP-98 row 44 obligation) -----

    /// Sign a NIP-42 AUTH event with `relay` tag = `relay_url`, then verify
    /// it against `expected_relay_url`. Returns the `verify_nip42_event` result.
    fn verify_nip42_with_urls(
        challenge: &str,
        signed_relay_url: &str,
        expected_relay_url: &str,
    ) -> Result<(), buzz_auth::AuthError> {
        let keys = Keys::generate();
        let parsed = nostr::RelayUrl::parse(signed_relay_url).expect("valid relay url");
        let event = EventBuilder::auth(challenge, parsed)
            .sign_with_keys(&keys)
            .expect("sign auth event");
        buzz_auth::nip42::verify_nip42_event(&event, challenge, expected_relay_url)
    }

    /// Row 44 obligation (WS side): a NIP-42 AUTH event signed against
    /// community A's host MUST be rejected on a connection whose tenant
    /// resolved to community B's host. Before this gap closed, `handle_auth`
    /// verified against `state.config.relay_url` (one static string per
    /// deployment), so a token-of-A presented on B's connection would pass —
    /// the cross-host hole `nip98_expected_url` already closed on the HTTP
    /// side, mirrored here on the WS side.
    ///
    /// Scenario: a multi-tenant deployment whose `config.relay_url` is set to
    /// community A's host (a realistic accident — config can only hold one
    /// host). An attacker on a B-bound connection signs an AUTH event matching
    /// that config URL (publicly knowable). Pre-fix: expected = config = A's
    /// URL = the signed URL → ACCEPT (cross-host hole). Post-fix: expected
    /// derives from `tenant.host() = B` ≠ signed (A) → REJECT.
    ///
    /// This test bites if `nip42_expected_relay_url` is reverted to return
    /// `config.relay_url` verbatim — the exact regression the helper guards.
    #[test]
    fn verify_nip42_rejects_event_signed_for_wrong_communitys_host() {
        let challenge = "fixed-challenge-for-test";
        // Config URL is A's host (deployment-wide static), and the attacker
        // signs an AUTH event with that same URL. Both are knowable to the
        // attacker. Connection arrived at community B.
        let config_relay_url = "ws://host-a.example:3100";
        let signed_relay_url = "ws://host-a.example:3100";
        let tenant_b = fresh_tenant("host-b.example:3100");
        let expected = nip42_expected_relay_url(config_relay_url, &tenant_b);

        let err = verify_nip42_with_urls(challenge, signed_relay_url, &expected).expect_err(
            "cross-host NIP-42 AUTH event MUST be rejected — row 44 sibling: \
             `relay` URL host must match the per-tenant host, NOT the \
             deployment-wide config URL",
        );
        assert!(
            matches!(err, buzz_auth::AuthError::RelayUrlMismatch),
            "rejection must carry RelayUrlMismatch (not a generic failure) so \
             callers can distinguish it from other auth failures; got {err:?}"
        );
    }

    /// Positive control: a NIP-42 AUTH event signed for host A MUST be
    /// accepted on a connection whose tenant resolved to host A. Without
    /// this, the cross-host test could be passing vacuously (e.g. if
    /// `nip42_expected_relay_url` always produced a URL no event could match).
    #[test]
    fn verify_nip42_accepts_event_signed_for_matching_host() {
        let challenge = "fixed-challenge-for-test";
        let signed_relay_url = "ws://host-a.example:3100";
        // Configured relay URL deliberately differs in host from the tenant's
        // host — proving the helper uses `tenant.host()`, not the config.
        let config_relay_url = "ws://other-config-host.example";
        let tenant_a = fresh_tenant("host-a.example:3100");
        let expected = nip42_expected_relay_url(config_relay_url, &tenant_a);

        verify_nip42_with_urls(challenge, signed_relay_url, &expected)
            .expect("matching-host NIP-42 AUTH event must verify");
    }

    /// `nip42_expected_relay_url` derives host from `tenant`, not from
    /// `config_relay_url`. Pin both directions: changing the tenant's host
    /// changes the output; changing the config's host does NOT.
    #[test]
    fn nip42_expected_relay_url_uses_tenant_host_not_config_host() {
        let tenant_a = fresh_tenant("host-a.example:3100");
        let tenant_b = fresh_tenant("host-b.example:3100");

        let url_a = nip42_expected_relay_url("ws://config-host.example", &tenant_a);
        let url_b = nip42_expected_relay_url("ws://config-host.example", &tenant_b);
        assert_eq!(url_a, "ws://host-a.example:3100");
        assert_eq!(url_b, "ws://host-b.example:3100");

        // Same tenant, two different config hosts → output is identical.
        // (If config-host ever leaked into the URL, this assertion would bite —
        // catches the exact "reverted to config host" regression.)
        let url_a_alt_config = nip42_expected_relay_url("ws://different-config.example", &tenant_a);
        assert_eq!(
            url_a, url_a_alt_config,
            "config-relay-url's host MUST NOT influence the NIP-42 expected URL — \
             only its scheme contributes"
        );
    }

    /// `nip42_expected_relay_url` derives scheme from `config_relay_url`'s
    /// prefix: `wss://` → `wss`, everything else → `ws`. Deployments that run
    /// `ws://` in dev/test must produce a `ws://` URL that matches what
    /// tungstenite clients put in the AUTH event's `relay` tag.
    #[test]
    fn nip42_expected_relay_url_derives_scheme_from_config() {
        let tenant = fresh_tenant("host-a.example:3100");
        assert_eq!(
            nip42_expected_relay_url("wss://config.example", &tenant),
            "wss://host-a.example:3100",
            "wss:// production config → wss:// URL"
        );
        assert_eq!(
            nip42_expected_relay_url("ws://config.example", &tenant),
            "ws://host-a.example:3100",
            "ws:// dev config → ws:// URL"
        );
    }

    /// Build a kind:30174 engram envelope authored by `agent`, tagged with `owner`.
    fn engram_envelope(agent: &Keys, owner_hex: &str) -> buzz_core::StoredEvent {
        let d_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::D)),
            ["abcd1234"],
        );
        let p_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
            [owner_hex],
        );
        let ev = EventBuilder::new(Kind::Custom(30174), "engram body")
            .tags([d_tag, p_tag])
            .sign_with_keys(agent)
            .expect("sign engram");
        buzz_core::StoredEvent::new(ev, None)
    }

    /// Regression test for the NIP-AE `/query` search leak (PR #593 review).
    ///
    /// Setup: two engram envelopes by different agents for different owners.
    /// An authorized search for `{kinds:[30174], #p:[owner_a]}` would be
    /// approved by the engram gate (owner_a is querying engrams addressed to
    /// them). The FTS pushdown only carries `kind:=[30174]`, so the
    /// envelope for owner_b can come back as a text-match hit. The post-filter
    /// in `search_hit_accepted` must reject it.
    #[test]
    fn search_hit_rejects_envelope_with_mismatched_p_tag() {
        let agent_a = Keys::generate();
        let agent_b = Keys::generate();
        let owner_a = Keys::generate().public_key().to_hex();
        let owner_b = Keys::generate().public_key().to_hex();

        let env_for_a = engram_envelope(&agent_a, &owner_a);
        let env_for_b = engram_envelope(&agent_b, &owner_b);

        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .custom_tags(p_tag, [&owner_a]);

        // 30174 is not owner-gated, so any reader hex is fine here.
        let reader = Keys::generate().public_key().to_hex();
        assert!(
            search_hit_accepted(&filter, &env_for_a, &[], &reader),
            "envelope addressed to owner_a must be returned"
        );
        assert!(
            !search_hit_accepted(&filter, &env_for_b, &[], &reader),
            "envelope addressed to owner_b must NOT be returned for a #p=[owner_a] search"
        );
    }

    /// `authors=[agent_a]` search must not return an envelope authored by agent_b,
    /// even if the FTS text match would otherwise surface it. (The FTS query does
    /// carry an `authors` pushdown today, so this is defence-in-depth; mirroring
    /// the WS contract.)
    #[test]
    fn search_hit_rejects_event_with_mismatched_author() {
        let agent_a = Keys::generate();
        let agent_b = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();

        let env_a = engram_envelope(&agent_a, &owner);
        let env_b = engram_envelope(&agent_b, &owner);

        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .author(agent_a.public_key());

        let reader = Keys::generate().public_key().to_hex();
        assert!(search_hit_accepted(&filter, &env_a, &[], &reader));
        assert!(
            !search_hit_accepted(&filter, &env_b, &[], &reader),
            "authors=[agent_a] search must not return events authored by agent_b"
        );
    }

    /// Channel-scoped events outside the caller's accessible-channel set are
    /// rejected by the post-filter regardless of NIP-01 match.
    #[test]
    fn search_hit_rejects_inaccessible_channel() {
        let agent = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let mut stored = engram_envelope(&agent, &owner);
        let scoped_channel = uuid::Uuid::new_v4();
        stored.channel_id = Some(scoped_channel);

        let p_tag = SingleLetterTag::lowercase(Alphabet::P);
        let filter = nostr::Filter::new()
            .kind(Kind::Custom(30174))
            .custom_tags(p_tag, [&owner]);

        let reader = Keys::generate().public_key().to_hex();
        assert!(
            !search_hit_accepted(&filter, &stored, &[], &reader),
            "channel-scoped hit must be rejected when caller has no channel access"
        );
        assert!(
            search_hit_accepted(&filter, &stored, &[scoped_channel], &reader),
            "channel-scoped hit must be accepted when caller has access to that channel"
        );
    }

    #[test]
    fn extract_buzz_channel_requires_one_string_value() {
        assert_eq!(
            extract_buzz_channel(&serde_json::json!({"#buzz-channel": ["channel-a"]})),
            Some("channel-a")
        );
        assert_eq!(
            extract_buzz_channel(&serde_json::json!({"#buzz-channel": ["channel-a", "channel-b"]})),
            None
        );
        assert_eq!(
            extract_buzz_channel(&serde_json::json!({"#buzz-channel": [42]})),
            None
        );
    }

    #[test]
    fn extract_before_id_valid_hex() {
        let hex = "a".repeat(64);
        let raw = serde_json::json!({ "before_id": hex });
        match extract_before_id(&raw) {
            BeforeId::Valid(id) => assert_eq!(id.len(), 32),
            _ => panic!("64-char hex must parse as Valid"),
        }
    }

    #[test]
    fn extract_before_id_short_hex() {
        let raw = serde_json::json!({ "before_id": "a".repeat(63) });
        assert!(matches!(extract_before_id(&raw), BeforeId::Malformed));
    }

    #[test]
    fn extract_before_id_long_hex() {
        let raw = serde_json::json!({ "before_id": "a".repeat(65) });
        assert!(matches!(extract_before_id(&raw), BeforeId::Malformed));
    }

    #[test]
    fn extract_before_id_invalid_hex_chars() {
        let raw = serde_json::json!({ "before_id": "z".repeat(64) });
        assert!(matches!(extract_before_id(&raw), BeforeId::Malformed));
    }

    #[test]
    fn extract_before_id_absent() {
        let raw = serde_json::json!({});
        assert!(matches!(extract_before_id(&raw), BeforeId::Absent));
    }

    #[test]
    fn extract_before_id_non_string() {
        let raw = serde_json::json!({ "before_id": 12345 });
        assert!(matches!(extract_before_id(&raw), BeforeId::Malformed));
    }

    #[test]
    fn extract_consistency_strong_pins_to_writer() {
        let raw = serde_json::json!({ "consistency": "strong" });
        assert!(matches!(extract_consistency(&raw), Consistency::Strong));
    }

    #[test]
    fn extract_consistency_absent_is_default_routed() {
        let raw = serde_json::json!({ "kinds": [40100] });
        assert!(matches!(extract_consistency(&raw), Consistency::Default));
    }

    #[test]
    fn extract_consistency_unknown_value_is_malformed() {
        // A typo or an attempt to name the inverse "force replica" direction
        // must reject the request, never silently degrade to routed.
        for bad in [
            serde_json::json!({ "consistency": "weak" }),
            serde_json::json!({ "consistency": "replica" }),
            serde_json::json!({ "consistency": "eventual" }),
            serde_json::json!({ "consistency": "STRONG" }),
            serde_json::json!({ "consistency": true }),
            serde_json::json!({ "consistency": 1 }),
        ] {
            assert!(
                matches!(extract_consistency(&bad), Consistency::Malformed),
                "{bad} must be rejected as malformed"
            );
        }
    }

    /// The routing direction the catchall loop dispatches on. A filter carrying
    /// `"consistency": "strong"` MUST resolve to the writer pool
    /// (`ReadRoute::Writer` → `query_events`); one without MUST resolve to the
    /// replica-eligible path (`ReadRoute::Routed` → `query_events_routed`).
    /// Both directions are pinned here so a refactor that drops the field on
    /// the floor — reading every filter from one pool — flips one of these and
    /// fails. The writer-vs-replica pool divergence itself is exercised by the
    /// two-pool `routed_reads_are_confined_to_the_requested_community` test in
    /// buzz-db (`#[ignore]`, requires Postgres).
    #[test]
    fn resolve_read_route_pins_strong_to_writer() {
        let strong = serde_json::json!({ "consistency": "strong" });
        assert_eq!(resolve_read_route(&strong), Ok(ReadRoute::Writer));
    }

    #[test]
    fn resolve_read_route_defaults_to_routed_replica() {
        let absent = serde_json::json!({ "kinds": [40100], "limit": 1 });
        assert_eq!(resolve_read_route(&absent), Ok(ReadRoute::Routed));
    }

    #[test]
    fn resolve_read_route_rejects_unknown_values() {
        // Malformed never degrades to a pool — it is a client error, so the
        // catchall loop turns this `Err` into a BAD_REQUEST before any DB work.
        for bad in [
            serde_json::json!({ "consistency": "weak" }),
            serde_json::json!({ "consistency": "replica" }),
            serde_json::json!({ "consistency": "STRONG" }),
            serde_json::json!({ "consistency": true }),
        ] {
            assert_eq!(
                resolve_read_route(&bad),
                Err(()),
                "{bad} must be a client error, never a pool"
            );
        }
    }

    /// Extension flags opt in only on a literal JSON `true` — absent,
    /// non-boolean, and truthy-but-not-bool values all read as false, so a
    /// malformed filter degrades to a normal query instead of a wrong window.
    #[test]
    fn extension_flag_only_true_on_literal_bool() {
        assert!(extension_flag(
            &serde_json::json!({ "top_level": true }),
            "top_level"
        ));
        assert!(!extension_flag(
            &serde_json::json!({ "top_level": false }),
            "top_level"
        ));
        assert!(!extension_flag(&serde_json::json!({}), "top_level"));
        assert!(!extension_flag(
            &serde_json::json!({ "top_level": "true" }),
            "top_level"
        ));
        assert!(!extension_flag(
            &serde_json::json!({ "top_level": 1 }),
            "top_level"
        ));
    }

    #[test]
    fn extract_page_offset_absent_is_none() {
        // No `page` → default offset (unrelated general queries untouched).
        let raw = serde_json::json!({ "kinds": [0], "limit": 50 });
        assert_eq!(extract_page_offset(&raw, Some(50)), None);
    }

    #[test]
    fn extract_page_offset_page_one_is_none() {
        // Page 1 is the first page → offset 0, expressed as no override.
        let raw = serde_json::json!({ "kinds": [0], "limit": 50, "page": 1 });
        assert_eq!(extract_page_offset(&raw, Some(50)), None);
    }

    #[test]
    fn extract_page_offset_computes_offset_from_page_and_limit() {
        // Empty people-directory contract: page N → (N-1) * limit.
        let raw = serde_json::json!({ "kinds": [0], "limit": 50, "page": 3 });
        assert_eq!(extract_page_offset(&raw, Some(50)), Some(100));
    }

    #[test]
    fn extract_page_offset_missing_limit_is_none() {
        // Can't size a page without a limit.
        let raw = serde_json::json!({ "kinds": [0], "page": 2 });
        assert_eq!(extract_page_offset(&raw, None), None);
    }

    /// Offsets are sized from the *clamped* limit the DB will honor, not from
    /// what the client asked for. `filter_to_query_params` clamps an absent or
    /// over-ceiling `limit` to `DEFAULT_MAX_PAGE_LIMIT` (guarded in
    /// `handlers::req::tests::req_filter_limit_clamps_to_advertised_nip11_max_limit`)
    /// and that clamped value is what arrives here — so page N starts exactly
    /// N-1 full pages in. Sizing from an unclamped limit would step past rows
    /// the previous page never returned.
    #[test]
    fn extract_page_offset_sizes_pages_from_clamped_limit() {
        let clamped = buzz_db::DEFAULT_MAX_PAGE_LIMIT;

        assert_eq!(
            extract_page_offset(&serde_json::json!({ "page": 2 }), Some(clamped)),
            Some(clamped)
        );
        assert_eq!(
            extract_page_offset(&serde_json::json!({ "page": 3 }), Some(clamped)),
            Some(clamped * 2)
        );
    }

    #[test]
    fn extract_depth_limit_valid() {
        let raw = serde_json::json!({ "depth_limit": 3 });
        assert_eq!(extract_depth_limit(&raw), Some(3));
    }

    #[test]
    fn extract_thread_cursor_valid() {
        // Timestamp-only cursor: 8-byte BE seconds, no tiebreak id.
        let raw = serde_json::json!({ "thread_cursor": 1_782_866_946_i64 });
        assert_eq!(
            extract_thread_cursor(&raw),
            Some(1_782_866_946_i64.to_be_bytes().to_vec())
        );
    }

    #[test]
    fn extract_thread_cursor_camel_case() {
        let raw = serde_json::json!({ "threadCursor": 42_i64 });
        assert_eq!(
            extract_thread_cursor(&raw),
            Some(42_i64.to_be_bytes().to_vec())
        );
    }

    #[test]
    fn extract_thread_cursor_composite() {
        // Composite cursor: 8-byte BE seconds followed by the raw event-id bytes.
        let id_hex = "aa".repeat(32);
        let raw = serde_json::json!({
            "thread_cursor": 1_782_866_946_i64,
            "thread_cursor_id": id_hex,
        });
        let mut expected = 1_782_866_946_i64.to_be_bytes().to_vec();
        expected.extend_from_slice(&[0xaa; 32]);
        assert_eq!(extract_thread_cursor(&raw), Some(expected));
    }

    #[test]
    fn extract_thread_cursor_composite_camel_case() {
        let id_hex = "bb".repeat(32);
        let raw = serde_json::json!({
            "threadCursor": 7_i64,
            "threadCursorId": id_hex,
        });
        let mut expected = 7_i64.to_be_bytes().to_vec();
        expected.extend_from_slice(&[0xbb; 32]);
        assert_eq!(extract_thread_cursor(&raw), Some(expected));
    }

    #[test]
    fn extract_thread_cursor_ignores_bad_id_hex() {
        // A malformed id falls back to timestamp-only rather than erroring.
        let raw = serde_json::json!({
            "thread_cursor": 5_i64,
            "thread_cursor_id": "not-hex",
        });
        assert_eq!(
            extract_thread_cursor(&raw),
            Some(5_i64.to_be_bytes().to_vec())
        );
    }

    #[test]
    fn extract_thread_cursor_absent() {
        let raw = serde_json::json!({ "depth_limit": 3 });
        assert!(extract_thread_cursor(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_zero() {
        let raw = serde_json::json!({ "depth_limit": 0 });
        assert_eq!(extract_depth_limit(&raw), Some(0));
    }

    #[test]
    fn extract_depth_limit_u32_max() {
        let raw = serde_json::json!({ "depth_limit": u32::MAX });
        assert_eq!(extract_depth_limit(&raw), Some(u32::MAX));
    }

    #[test]
    fn extract_depth_limit_overflow() {
        let raw = serde_json::json!({ "depth_limit": (u32::MAX as u64) + 1 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_negative() {
        let raw = serde_json::json!({ "depth_limit": -1 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_absent() {
        let raw = serde_json::json!({});
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_depth_limit_float() {
        let raw = serde_json::json!({ "depth_limit": 3.5 });
        assert!(extract_depth_limit(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_valid() {
        let raw = serde_json::json!({ "feed_types": ["mentions", "activity"] });
        assert_eq!(
            extract_feed_types(&raw),
            Some(vec!["mentions".to_string(), "activity".to_string()])
        );
    }

    #[test]
    fn extract_feed_types_empty_array() {
        let raw = serde_json::json!({ "feed_types": [] });
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_mixed_types() {
        let raw = serde_json::json!({ "feed_types": ["mentions", 42, "activity"] });
        assert_eq!(
            extract_feed_types(&raw),
            Some(vec!["mentions".to_string(), "activity".to_string()])
        );
    }

    #[test]
    fn extract_feed_types_absent() {
        let raw = serde_json::json!({});
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn extract_feed_types_non_array() {
        let raw = serde_json::json!({ "feed_types": "mentions" });
        assert!(extract_feed_types(&raw).is_none());
    }

    #[test]
    fn event_accessible_no_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let se = buzz_core::StoredEvent::new(ev, None);
        assert!(event_in_accessible_channel(&se, &[]));
    }

    #[test]
    fn event_accessible_matching_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let ch = uuid::Uuid::new_v4();
        let mut se = buzz_core::StoredEvent::new(ev, None);
        se.channel_id = Some(ch);
        assert!(event_in_accessible_channel(&se, &[ch]));
    }

    #[test]
    fn event_inaccessible_channel() {
        let keys = Keys::generate();
        let ev = EventBuilder::new(Kind::Custom(1), "test")
            .sign_with_keys(&keys)
            .unwrap();
        let ch = uuid::Uuid::new_v4();
        let other = uuid::Uuid::new_v4();
        let mut se = buzz_core::StoredEvent::new(ev, None);
        se.channel_id = Some(ch);
        assert!(!event_in_accessible_channel(&se, &[other]));
    }

    /// NIP-DV regression: a relay-signed kind:30622 snapshot must not leak via
    /// search through a kindless `ids:[snapshot_id]` filter that carries no #p.
    /// `filters_match` passes (id matches), channel check passes (channel_id =
    /// None), so only the result-level `reader_authorized_for_event` check
    /// stands between a third party and the owner's private hide set.
    #[test]
    fn search_hit_rejects_dm_visibility_for_kindless_ids_third_party() {
        let relay = Keys::generate();
        let viewer = Keys::generate().public_key().to_hex();
        let third_party = Keys::generate().public_key().to_hex();

        let d_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::D)),
            [&viewer],
        );
        let p_tag = Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P)),
            [&viewer],
        );
        let ev = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_DM_VISIBILITY as u16), "")
            .tags([d_tag, p_tag])
            .sign_with_keys(&relay)
            .expect("sign snapshot");
        let stored = buzz_core::StoredEvent::new(ev.clone(), None);

        // Kindless filter — the exact bypass shape: no #p, just the id.
        let filter = nostr::Filter::new().id(ev.id);

        assert!(
            !search_hit_accepted(&filter, &stored, &[], &third_party),
            "third party must not receive a DM-visibility snapshot via kindless ids search"
        );
        assert!(
            search_hit_accepted(&filter, &stored, &[], &viewer),
            "owner must still receive their own snapshot"
        );
    }

    // ──────────────────────────────────────────────────────────────────────────
    // truncate_reason regression tests
    //
    // Required by T1: prove a near-limit malformed input cannot produce a
    // near-limit log payload.  No infrastructure needed — pure unit tests.
    // ──────────────────────────────────────────────────────────────────────────

    /// Near-limit ASCII input (1 MiB) must be capped at exactly `max_bytes`.
    #[test]
    fn truncate_reason_large_ascii_input_is_bounded() {
        let big = "a".repeat(1024 * 1024); // 1 MiB
        let result = truncate_reason(&big, 256);
        assert_eq!(result.len(), 256);
        assert!(result.is_ascii());
    }

    /// A multi-byte codepoint that straddles the byte boundary must not be
    /// split: the output must end at the last complete codepoint before the
    /// cap, keeping the slice valid UTF-8.
    #[test]
    fn truncate_reason_multibyte_codepoint_at_boundary_is_not_split() {
        // Build a string of 255 ASCII bytes followed by a 3-byte codepoint (€, U+20AC).
        // Naïve cutoff at byte 256 would land inside the 3-byte sequence.
        let mut s = "a".repeat(255);
        s.push('€'); // 3 bytes: 0xE2 0x82 0xAC — spans bytes 255..258
        assert_eq!(s.len(), 258);

        let result = truncate_reason(&s, 256);
        // Must end before the multi-byte codepoint.
        assert_eq!(result.len(), 255);
        assert_eq!(result, "a".repeat(255));
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    /// Short input well under the cap must be returned unchanged.
    #[test]
    fn truncate_reason_short_input_returned_unchanged() {
        let s = "invalid: kind 24620 rejected";
        let result = truncate_reason(s, 256);
        assert_eq!(result, s);
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Handler-level tests: submit_event HTTP-counter seam
    //
    // These tests drive real HTTP requests through the axum router to prove
    // that the bridge code path (not just the shared helper) actually
    // increments buzz_events_rejected_total{transport="http"}.  They are
    // discriminating: removing either bridge call site causes the corresponding
    // test to fail.
    //
    // Why `#[ignore = "requires Postgres"]`: submit_event calls bind_community
    // (needs a real communities row) and enforce_http_admission (needs Redis).
    // Both services run locally in dev.  The admission check succeeds for a
    // freshly generated pubkey (first request, far below quota), so no Redis
    // seeding is required beyond the default local instance.
    //
    // Why `#[test]` + manual runtime instead of `#[tokio::test]`:
    // metrics::with_local_recorder stores the recorder in a thread-local.  An
    // async test uses a multi-thread scheduler by default; when submit_event
    // runs, it may land on a different thread and miss the recorder entirely.
    // Using a current_thread runtime with rt.block_on() inside the recorder
    // closure guarantees the handler runs on the same thread as the recorder.
    // ──────────────────────────────────────────────────────────────────────────

    struct AlwaysFreshReplayGuard;

    impl Nip98ReplayGuard for AlwaysFreshReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    /// Build an AppState suitable for handler-level bridge tests.
    ///
    /// - `require_auth_token = false` → X-Pubkey dev-mode fallback active.
    /// - `require_relay_membership = false` → membership check short-circuits to
    ///   OpenRelay without a DB lookup.
    /// - `nip98_replay` replaced with an always-fresh guard → no Redis needed
    ///   for replay detection.
    /// - Redis pool points at the local dev instance for the admission check.
    ///
    /// Returns `None` when local Postgres is not reachable.
    pub(super) async fn bridge_handler_test_state() -> Option<Arc<crate::state::AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        // Use the real local Redis so enforce_http_admission can pass.
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "wss://bridge-test.local".to_string();
        config.require_auth_token = false;
        config.require_relay_membership = false;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    /// Drive a single POST /events request through the router and return the
    /// HTTP status code.
    async fn post_events(
        state: Arc<crate::state::AppState>,
        host: &str,
        pubkey_hex: &str,
        body: &[u8],
    ) -> axum::http::StatusCode {
        use axum::body::Body;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        crate::router::build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/events")
                    .header(header::HOST, host)
                    .header("x-pubkey", pubkey_hex)
                    .body(Body::from(body.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot")
            .status()
    }

    /// Like `post_events` but also returns the UTF-8 response body.
    async fn post_events_with_body(
        state: Arc<crate::state::AppState>,
        host: &str,
        pubkey_hex: &str,
        body: &[u8],
    ) -> (axum::http::StatusCode, String) {
        use axum::body::Body;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let resp = crate::router::build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/events")
                    .header(header::HOST, host)
                    .header("x-pubkey", pubkey_hex)
                    .body(Body::from(body.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read response body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Collect buzz_events_rejected_total with (transport, reason) labels from
    /// a DebuggingRecorder snapshot.
    fn http_reject_counts(
        snapshotter: &metrics_util::debugging::Snapshotter,
    ) -> std::collections::HashMap<(String, String), u64> {
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_events_rejected_total")
            .map(|(key, _, _, value)| {
                let metrics_util::debugging::DebugValue::Counter(n) = value else {
                    panic!("buzz_events_rejected_total must be a counter");
                };
                let labels: Vec<_> = key.key().labels().collect();
                let transport = labels
                    .iter()
                    .find(|l| l.key() == "transport")
                    .map(|l| l.value().to_owned())
                    .unwrap_or_default();
                let reason = labels
                    .iter()
                    .find(|l| l.key() == "reason")
                    .map(|l| l.value().to_owned())
                    .unwrap_or_default();
                ((transport, reason), n)
            })
            .collect()
    }

    /// T2a — pre-parse 400 arm: a POST /events with an invalid JSON body must
    /// increment buzz_events_rejected_total{transport="http",reason="invalid"}.
    ///
    /// Discriminating: if the `reject_with_transport` call in bridge.rs's
    /// `serde_json::from_slice` map_err closure is removed, this test fails.
    #[test]
    #[ignore = "requires Postgres"]
    fn submit_event_invalid_json_body_increments_http_transport_counter() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(bridge_handler_test_state()) else {
            panic!("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");
        };

        // Provision a fresh community so bind_community succeeds.
        let host = {
            let h = format!("bridge-test-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&h))
                .expect("ensure community");
            h
        };

        let pubkey_hex = Keys::generate().public_key().to_hex();

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            let status = rt.block_on(post_events(
                state.clone(),
                &host,
                &pubkey_hex,
                b"not valid json at all",
            ));
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "malformed body must yield 400"
            );
        });

        let counts = http_reject_counts(&snapshotter);
        assert_eq!(
            counts.get(&("http".to_owned(), "invalid".to_owned())),
            Some(&1),
            "pre-parse 400 arm must increment transport=http,reason=invalid"
        );
    }

    /// T2b — post-parse IngestError::Rejected arm: a POST /events with a
    /// valid but relay-only-kind event (kind 13534 = membership snapshot) must
    /// increment buzz_events_rejected_total{transport="http",reason="invalid"}.
    ///
    /// Kind 13534 is rejected in ingest_event before signature verification,
    /// so any properly signed Nostr event of this kind triggers the arm.
    ///
    /// Discriminating: if the `reject_with_transport` call in bridge.rs's
    /// IngestError::Rejected match arm is removed, this test fails.
    #[test]
    #[ignore = "requires Postgres"]
    fn submit_event_relay_only_kind_increments_http_transport_counter() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(bridge_handler_test_state()) else {
            panic!("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");
        };

        let host = {
            let h = format!("bridge-test-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&h))
                .expect("ensure community");
            h
        };

        let client_keys = Keys::generate();
        let pubkey_hex = client_keys.public_key().to_hex();

        // Kind 13534 (membership snapshot) is relay-only; ingest_event rejects
        // it before reaching signature verification.
        let relay_only_event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as u16),
            "",
        )
        .sign_with_keys(&client_keys)
        .expect("sign relay-only event");
        let event_json = serde_json::to_vec(&relay_only_event).expect("serialize event");

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            let status = rt.block_on(post_events(state.clone(), &host, &pubkey_hex, &event_json));
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "relay-only-kind event must yield 400"
            );
        });

        let counts = http_reject_counts(&snapshotter);
        assert_eq!(
            counts.get(&("http".to_owned(), "invalid".to_owned())),
            Some(&1),
            "IngestError::Rejected arm must increment transport=http,reason=invalid"
        );
    }

    /// Canvas ingest wiring regression: the kind-40100-specific future-timestamp
    /// guard in `ingest_event_inner` is actually wired to the shipping call path.
    ///
    /// Calls `post_events_with_body` → router → `submit_event` → `ingest_event_inner`:
    /// - A canvas event with `created_at = relay_now + 600` is rejected 400 with
    ///   the canvas-specific error "canvas event timestamp too far in the future".
    ///
    /// The +600 offset sits 300 s above the canvas ceiling (300 s) and 300 s
    /// below the general drift bound (900 s). Scheduler latency between test
    /// setup and production's independent `Utc::now()` re-sample would need to
    /// exceed 300 s to erode the margin — not possible under any realistic load.
    /// Exact 300/301 boundary coverage lives in the pure `validate_canvas_future_timestamp`
    /// tests (`canvas_ingest_numeric_contract`, `canvas_ingest_future_timestamp_boundary`),
    /// which pass fixed arguments and have no clock race.
    ///
    /// Discriminating: deleting the `if kind_u32 == KIND_CANVAS { … }` call in
    /// `ingest_event_inner` removes the guard. The event then passes the general
    /// ±900 s drift check (600 s < 900 s) and reaches the channel membership
    /// check (no h-tag channel exists → "restricted: not a channel member"),
    /// making the message assertion below fail with a different body.
    #[test]
    #[ignore = "requires Postgres"]
    fn canvas_ingest_future_timestamp_guard_is_wired() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(bridge_handler_test_state()) else {
            panic!("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");
        };

        let host = {
            let h = format!(
                "canvas-ingest-wiring-{}.local",
                uuid::Uuid::new_v4().simple()
            );
            rt.block_on(state.db.ensure_configured_community(&h))
                .expect("ensure community");
            h
        };

        let client_keys = Keys::generate();
        let pubkey_hex = client_keys.public_key().to_hex();

        // A canvas event 600 s in the future. The +600 offset sits 300 s above
        // the canvas ceiling and 300 s below the general ±900 s drift bound, so
        // only the canvas guard can produce a rejection here. Scheduler latency
        // between this Utc::now() call and production's independent re-sample
        // would need to exceed 300 s to erode the margin — impossible in practice.
        // Exact 300/301 boundary assertions live in the pure fixed-literal tests.
        let relay_now = chrono::Utc::now().timestamp();
        // Use a random channel UUID that does NOT exist in the DB. If the canvas
        // guard is correctly wired, it fires first; if deleted, the event passes
        // the general ±900 s check (600 < 900) and reaches the membership check,
        // producing "not a channel member" instead of the canvas rejection.
        let channel_id = uuid::Uuid::new_v4().to_string();
        let event_past_ceiling =
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "")
                .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
                .custom_created_at(nostr::Timestamp::from(
                    (relay_now + 600).try_into().unwrap_or(0u64),
                ))
                .sign_with_keys(&client_keys)
                .expect("sign canvas event past ceiling");
        let body_bytes =
            serde_json::to_vec(&event_past_ceiling).expect("serialize event past ceiling");

        let (status, body) = rt.block_on(post_events_with_body(
            state.clone(),
            &host,
            &pubkey_hex,
            &body_bytes,
        ));

        // Must be 400 AND the body must name the canvas guard (not the membership check).
        // Mutation oracle: deleting the `if kind_u32 == KIND_CANVAS { … }` guard
        // makes the body say "not a channel member" instead, failing both assertions.
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "canvas event at relay_now+600 must be rejected 400; body: {body}",
        );
        assert!(
            body.contains("canvas event timestamp too far in the future"),
            "rejection body must name the canvas guard (not the membership check). \
             Got: {body}; mutation oracle: delete the guard call site → body becomes \
             'not a channel member'",
        );
    }

    /// Wire-pinning test: a canvas CAS conflict must reach the HTTP client as
    /// **409 CONFLICT**, not 400.
    ///
    /// The relay's `IngestError::CanvasConflict` variant maps to `409` via the
    /// `bridge.rs` HTTP handler.  The CLI reconciliation branch gates on
    /// `status == 409`; if the bridge emits `400` instead the reconciliation
    /// path is dead code against the live relay.
    ///
    /// Scenario:
    /// 1. POST canvas event A (no `expected-revision` tag) → 200, head = A.
    /// 2. POST canvas event B with `expected-revision: <A-id>` → 200, head = B.
    /// 3. POST canvas event C with `expected-revision: <A-id>` (stale, A ≠ B)
    ///    → 409 with a body containing `"canvas changed since it was loaded"`.
    ///
    /// Mutation oracle: mapping `IngestError::CanvasConflict` to
    /// `StatusCode::BAD_REQUEST` (reverting the fix) makes step 3 return 400
    /// and fails the status assertion. The body assertion separately pins the
    /// exact `error` envelope value.
    #[test]
    #[ignore = "requires Postgres"]
    fn canvas_cas_conflict_yields_409_through_http_bridge() {
        use buzz_core::kind::KIND_CANVAS;
        use buzz_db::channel::{ChannelType, ChannelVisibility};
        use uuid::Uuid;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(bridge_handler_test_state()) else {
            panic!("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");
        };

        let (host, channel_id) = rt.block_on(async {
            let h = format!("canvas-cas-409-wiring-{}.local", Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&h)
                .await
                .expect("ensure community");
            let creator_keys = Keys::generate();
            let (channel, _) = state
                .db
                .create_channel_with_id(
                    community.id,
                    Uuid::new_v4(),
                    &format!("canvas-cas-409-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    creator_keys.public_key().to_bytes().as_slice(),
                    None,
                )
                .await
                .expect("create test channel");
            (h, channel.id.to_string())
        });

        let author_keys = Keys::generate();
        let pubkey_hex = author_keys.public_key().to_hex();

        let relay_now = chrono::Utc::now().timestamp() as u64;

        // Step 1: unconditional first write — establishes head A.
        let event_a = EventBuilder::new(Kind::Custom(KIND_CANVAS as u16), "# first canvas")
            .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
            .custom_created_at(nostr::Timestamp::from(relay_now))
            .sign_with_keys(&author_keys)
            .expect("sign canvas event A");
        let event_a_id = event_a.id.to_hex();
        let body_a = serde_json::to_vec(&event_a).expect("serialize event A");

        let (status_a, _) = rt.block_on(post_events_with_body(
            state.clone(),
            &host,
            &pubkey_hex,
            &body_a,
        ));
        assert_eq!(
            status_a,
            axum::http::StatusCode::OK,
            "first canvas write must be accepted"
        );

        // Step 2: write B on top of A — advances head so A is no longer current.
        let event_b = EventBuilder::new(Kind::Custom(KIND_CANVAS as u16), "# second canvas (on A)")
            .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
            .tag(
                Tag::parse(["expected-revision", event_a_id.as_str()])
                    .expect("expected-revision tag"),
            )
            .custom_created_at(nostr::Timestamp::from(relay_now + 1))
            .sign_with_keys(&author_keys)
            .expect("sign canvas event B");
        let body_b = serde_json::to_vec(&event_b).expect("serialize event B");

        let (status_b, _) = rt.block_on(post_events_with_body(
            state.clone(),
            &host,
            &pubkey_hex,
            &body_b,
        ));
        assert_eq!(
            status_b,
            axum::http::StatusCode::OK,
            "second canvas write (B on A) must be accepted"
        );

        // Step 3: stale write C with the same `expected-revision: A` — A is no
        // longer the head (B is), so this must be a CAS conflict → HTTP 409.
        // Mutation oracle: reverting IngestError::CanvasConflict → BAD_REQUEST
        // in bridge.rs makes this return 400 and fails the status assertion.
        let event_c = EventBuilder::new(
            Kind::Custom(KIND_CANVAS as u16),
            "# stale write (still on A)",
        )
        .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
        .tag(Tag::parse(["expected-revision", event_a_id.as_str()]).expect("expected-revision tag"))
        .custom_created_at(nostr::Timestamp::from(relay_now + 2))
        .sign_with_keys(&author_keys)
        .expect("sign canvas event C");
        let body_c = serde_json::to_vec(&event_c).expect("serialize event C");

        let (status_c, body_text) = rt.block_on(post_events_with_body(
            state.clone(),
            &host,
            &pubkey_hex,
            &body_c,
        ));

        assert_eq!(
            status_c,
            axum::http::StatusCode::CONFLICT,
            "stale canvas CAS write must yield 409 CONFLICT (not 400); body: {body_text}"
        );
        // Parse the response body and assert the exact canonical `error` value to
        // pin the byte-preservation contract. A substring check would pass even if
        // the message were embedded elsewhere; this ensures the envelope is intact.
        let body_json: serde_json::Value =
            serde_json::from_str(&body_text).expect("response body must be valid JSON");
        assert_eq!(
            body_json.get("error").and_then(|v| v.as_str()),
            Some("conflict: canvas changed since it was loaded"),
            "409 body must carry the exact canonical error value. Got: {body_text}"
        );
    }

    // ──────────────────────────────────────────────────────────────────────────
    // Log-capture helpers and attribution-invariant tests
    //
    // These tests assert that exactly ONE "HTTP bridge request" log line
    // appears per authed request, pinning the W1 single-terminal-log invariant
    // and the E1 no-double-log rule.
    //
    // Infrastructure: same `#[ignore = "requires Postgres"]` + current_thread
    // runtime discipline as the HTTP-counter tests above.
    // ──────────────────────────────────────────────────────────────────────────

    /// Shared buffer that collects all bytes written by the tracing fmt layer.
    #[derive(Clone)]
    struct CapturingMakeWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    struct CapturingWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for CapturingWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturingMakeWriter {
        type Writer = CapturingWriter;
        fn make_writer(&'a self) -> Self::Writer {
            CapturingWriter {
                buf: Arc::clone(&self.buf),
            }
        }
    }

    /// Run `post_events` with a capturing subscriber and return `(status_code, captured_log)`.
    fn run_and_capture(
        rt: &tokio::runtime::Runtime,
        state: Arc<crate::state::AppState>,
        host: &str,
        pubkey_hex: &str,
        body: &[u8],
    ) -> (axum::http::StatusCode, String) {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let make_writer = CapturingMakeWriter {
            buf: Arc::clone(&buf),
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(make_writer)
            .with_ansi(false)
            .finish();

        let status = tracing::subscriber::with_default(subscriber, || {
            rt.block_on(post_events(state, host, pubkey_hex, body))
        });

        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap_or_default();
        (status, captured)
    }

    /// Count lines that contain the terminal attribution marker.
    fn count_attribution_lines(log: &str) -> usize {
        log.lines()
            .filter(|l| l.contains("HTTP bridge request"))
            .count()
    }

    /// T3a — exactly-once invariant, pre-parse 400 arm (invalid JSON).
    ///
    /// An authenticated client that submits a non-JSON body must produce
    /// exactly ONE "HTTP bridge request" log line.  This pins W1 (early exits
    /// are attributed) and E1 (no double line even for the parse-fail arm).
    ///
    /// Discriminating: if the attribution log is removed from the ParseFail
    /// arm in submit_event, this test fails.
    #[test]
    #[ignore = "requires Postgres"]
    fn submit_event_invalid_json_emits_exactly_one_attribution_line() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let state = rt
            .block_on(bridge_handler_test_state())
            .expect("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");

        let host = {
            let h = format!("bridge-attr-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&h))
                .expect("ensure community");
            h
        };

        let pubkey_hex = Keys::generate().public_key().to_hex();

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let (status, log) = metrics::with_local_recorder(&recorder, || {
            run_and_capture(&rt, state, &host, &pubkey_hex, b"not valid json at all")
        });

        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "invalid JSON must yield 400"
        );

        let n = count_attribution_lines(&log);
        assert_eq!(
            n, 1,
            "expected exactly 1 attribution line for invalid-JSON arm, got {n};\nlog:\n{log}"
        );
        assert!(
            log.contains(&pubkey_hex[..16]),
            "attribution line must carry the pubkey;\nlog:\n{log}"
        );
    }

    /// T3b — exactly-once invariant, post-parse IngestError::Rejected arm (relay-only kind).
    ///
    /// An authenticated client that submits a relay-only-kind event (kind 13534)
    /// must produce exactly ONE "HTTP bridge request" log line — not two
    /// (the old code emitted both an info! attribution and a warn! reason line).
    ///
    /// Discriminating: if two log lines are emitted (old double-log bug), this
    /// test fails.
    #[test]
    #[ignore = "requires Postgres"]
    fn submit_event_relay_only_kind_emits_exactly_one_attribution_line() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let state = rt
            .block_on(bridge_handler_test_state())
            .expect("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");

        let host = {
            let h = format!("bridge-attr-{}.local", uuid::Uuid::new_v4().simple());
            rt.block_on(state.db.ensure_configured_community(&h))
                .expect("ensure community");
            h
        };

        let client_keys = Keys::generate();
        let pubkey_hex = client_keys.public_key().to_hex();

        let relay_only_event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as u16),
            "",
        )
        .sign_with_keys(&client_keys)
        .expect("sign relay-only event");
        let event_json = serde_json::to_vec(&relay_only_event).expect("serialize event");

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let (status, log) = metrics::with_local_recorder(&recorder, || {
            run_and_capture(&rt, state, &host, &pubkey_hex, &event_json)
        });

        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "relay-only-kind event must yield 400"
        );

        let n = count_attribution_lines(&log);
        assert_eq!(
            n, 1,
            "expected exactly 1 attribution line for IngestError::Rejected arm, got {n};\nlog:\n{log}"
        );
        assert!(
            log.contains(&pubkey_hex[..16]),
            "attribution line must carry the pubkey;\nlog:\n{log}"
        );
    }

    // ── NIP-FI production-seam tests (F4) ────────────────────────────────────
    //
    // These tests drive real HTTP requests through the axum router with NIP-FI
    // in Enforce mode and a valid NIP-98 event but NO assertion header.  Each
    // test must go red if the `admit_nip_fi_http_on_state` call is deleted or
    // inverted at the corresponding production call site.
    //
    // Falsifiability: a request with valid NIP-98 + no assertion in Enforce
    // mode → NIP-FI gate fires → 401 (MissingEvidence). If the gate is removed,
    // the request proceeds past NIP-FI to community lookup → succeeds (community
    // is provisioned) → further processing → some other status (200, 400, etc.)
    // that is NOT 401.  The assert_eq fires.
    //
    // Why `#[ignore = "requires Postgres"]`: the handlers call bind_community
    // before the NIP-FI gate; the community must exist for the NIP-98 URL to
    // match. All four protected surfaces need Postgres for the NIP-FI seam test
    // to be exercised (vs. bailing at community lookup with 404 before NIP-FI).
    //
    // ## NIP-FI route classification
    //
    // Route classification (PROTECTED vs. EXEMPT) is now owned by
    // `router.rs::NIP_FI_EXEMPT_PREFIXES` and enforced by the
    // `nip_fi_assertion_guard` middleware layer.  See the comment block at the
    // top of `router.rs` for the complete classification and the rationale.
    //
    // The tests below exercise the *outer* assertion guard in `router.rs`
    // (router.rs:232-234): in Enforce mode, a missing or crypto-invalid
    // `Nostr-Federated-Identity` token is rejected BEFORE the handler runs.
    //
    // These tests do NOT prove per-handler `admit_nip_fi_http_on_state` wiring
    // — deleting a handler's admission call would not change these results.
    // The cardinality test (`r3_cardinality_actual_caller_query_off_passes_enforce_denies`)
    // exercises the handler-level gate with a valid assertion.  Per-handler
    // key-pairing and deny-map are tested in `settings_tests.rs` and the
    // crypto-seam test above.

    /// Build an AppState with NIP-FI in Enforce mode for production-seam tests.
    ///
    /// Sets `nip_fi.mode = Enforce` while leaving `nip_fi_verifier = None`
    /// (startup race: no issuers configured → verifier not built).  This is
    /// sufficient for the seam test because the NIP-FI gate fires with 401
    /// (MissingEvidence) when the assertion header is absent, BEFORE any
    /// verifier lookup.  `require_auth_token = true` forces real NIP-98.
    ///
    /// Returns `None` when local Postgres is not reachable.
    async fn nip_fi_enforce_test_state() -> Option<Arc<crate::state::AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "wss://nip-fi-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        // Pin the GIF provider absent: `Config::from_env()` imports
        // `BUZZ_KLIPY_API_KEY`, and the GIF positive control's exact 404
        // (`gifs.rs` "GIF search is not configured") depends on `klipy = None`.
        config.klipy = None;
        // No issuers configured → nip_fi_verifier = None (startup-race path).
        // The seam test fires before verifier is needed (missing assertion → 401).

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    // EC P-256 test key constants shared by positive-control and cardinality tests.
    // Private key (PKCS#8 PEM) + public key coordinates (JWK x/y/kid).
    // Used by `nip_fi_enforce_test_state_with_verifier()` and
    // `signed_assertion_for_pubkey()`.
    const HANDLER_TEST_ISSUER: &str = "https://issuer.example";
    const HANDLER_TEST_AUDIENCE: &str = "https://relay.example";
    const HANDLER_TEST_KID: &str = "test-key-1";
    const HANDLER_TEST_EC_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
        WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
        zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
        -----END PRIVATE KEY-----\n";

    /// Build a NIP-FI Enforce AppState with a real injected P-256 verifier.
    ///
    /// Used by positive-control tests (same-key admission proves the handler
    /// was reached, not just deny-all).  Same key material as used in the
    /// cardinality test and the `signed_assertion_for_pubkey` helper below.
    async fn nip_fi_enforce_test_state_with_verifier() -> Option<Arc<crate::state::AppState>> {
        use buzz_auth::{
            AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
            IssuerRegistry, StaticIssuerKeySource, TokenClass, VerifyAssertion,
        };
        use jsonwebtoken::{jwk::JwkSet, Algorithm};

        let mut state = (*nip_fi_enforce_test_state().await?).clone();

        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                "kid": HANDLER_TEST_KID,
                "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
                "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA"
            }]
        }))
        .expect("valid test JWKS");
        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set =
            AssertionKeySet::new_for_test(HANDLER_TEST_ISSUER.to_owned(), 1, jwks, hard_deadline)
                .expect("valid test key set");
        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{HANDLER_TEST_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");
        let policy = IssuerPolicy::new(
            HANDLER_TEST_ISSUER.to_owned(),
            vec![HANDLER_TEST_AUDIENCE.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");
        let mut registry = IssuerRegistry::new();
        registry.insert(policy);
        let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
        ));
        state.nip_fi_verifier = Some(verifier);
        Some(Arc::new(state))
    }

    /// Mint a signed NIP-FI assertion whose `nostr_pubkey` = `pubkey_hex`,
    /// using the shared HANDLER_TEST_* key material.
    fn signed_assertion_for_pubkey(pubkey_hex: &str) -> String {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": HANDLER_TEST_ISSUER,
            "aud": HANDLER_TEST_AUDIENCE,
            "iat": now,
            "exp": now + 600,
            "sub": "test-subject",
            "nostr_pubkey": pubkey_hex,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(HANDLER_TEST_KID.to_owned());
        header.typ = Some("nip-fi+jwt".to_owned());
        let key =
            EncodingKey::from_ec_pem(HANDLER_TEST_EC_PEM.as_bytes()).expect("valid test EC PEM");
        jsonwebtoken::encode(&header, &claims, &key).expect("sign assertion")
    }

    /// Build a HeaderMap containing a valid NIP-98 Authorization header +
    /// a valid NIP-FI assertion for the same key.
    fn same_key_nip98_and_assertion_headers(
        keys: &Keys,
        url: &str,
        method: &str,
        body: &[u8],
    ) -> axum::http::HeaderMap {
        let mut headers = make_nip98_headers(keys, url, method, body);
        let assertion = signed_assertion_for_pubkey(&keys.public_key().to_hex());
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {assertion}").parse().expect("valid header"),
        );
        headers
    }

    /// Build an AppState with NIP-FI in Off mode for production-seam regression tests.
    ///
    /// `require_auth_token = false` so requests without NIP-98 auth still reach
    /// the application logic rather than rejecting at the NIP-98 layer.
    async fn nip_fi_off_test_state() -> Option<Arc<crate::state::AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "wss://nip-fi-test.local".to_string();
        config.require_auth_token = false;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Off;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    /// Build an AppState with NIP-FI in DenyProtected mode.
    async fn nip_fi_deny_protected_test_state() -> Option<Arc<crate::state::AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = crate::test_support::database_url();
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "wss://nip-fi-test.local".to_string();
        config.require_auth_token = true;
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::DenyProtected;

        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

        let (mut state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some(Arc::new(state))
    }

    /// Sign a NIP-98 event for a given URL and method, returning a valid
    /// `Authorization: Nostr <base64>` header map.
    ///
    /// Includes a `payload` tag for the given body bytes so the event passes
    /// the payload-binding check in NIP-FI Enforce mode. For GET or empty
    /// bodies pass `b""` — the SHA-256 of an empty body is included regardless,
    /// keeping the event unconditionally valid through `verify_bridge_auth_with_options`.
    fn make_nip98_headers(
        keys: &Keys,
        url: &str,
        method: &str,
        body: &[u8],
    ) -> axum::http::HeaderMap {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use sha2::{Digest, Sha256};
        let payload_hex = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
            Tag::parse(["payload", &payload_hex]).expect("payload tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let value = format!("Nostr {}", BASE64.encode(event_json.as_bytes()));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            value.parse().expect("valid header"),
        );
        headers
    }

    /// Drive a single oneshot request through the full relay router and return
    /// the HTTP status.
    async fn oneshot_request(
        state: Arc<crate::state::AppState>,
        method: &str,
        uri: &str,
        host: &str,
        headers: axum::http::HeaderMap,
        body: &[u8],
    ) -> axum::http::StatusCode {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", host);
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }
        crate::router::build_router(state)
            .oneshot(
                builder
                    .body(Body::from(body.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot")
            .status()
    }

    /// Drive a single oneshot request through the full relay router and return
    /// `(status, response_headers, body_bytes)` for exact-byte assertions.
    async fn oneshot_request_full(
        state: Arc<crate::state::AppState>,
        method: &str,
        uri: &str,
        host: &str,
        headers: axum::http::HeaderMap,
        body: &[u8],
    ) -> (axum::http::StatusCode, axum::http::HeaderMap, bytes::Bytes) {
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use tower::ServiceExt;

        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", host);
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }
        let resp = crate::router::build_router(state)
            .oneshot(
                builder
                    .body(Body::from(body.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot");
        let status = resp.status();
        let resp_headers = resp.headers().clone();
        let resp_body = to_bytes(resp.into_body(), 8192).await.unwrap_or_default();
        (status, resp_headers, resp_body)
    }

    // ── F4: bridge POST /events — enforce mode, no assertion → 401 ──────────
    //
    // Exercises the OUTER assertion guard in `router.rs` (not the per-handler
    // gate): build_router with no Nostr-Federated-Identity header → guard fires
    // before the handler runs → 401 `authentication required\n`.
    //
    // Falsifying mutation: removing the outer `nip_fi_assertion_guard` layer
    // from `build_router` does NOT change this test — the per-handler
    // `admit_nip_fi_http_on_state` call in `submit_event` also denies 401
    // `authentication required\n` when no assertion is present. This test
    // proves the outer guard fires (and its error path is exercised), not
    // that it is the sole denial point for missing-assertion requests.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_bridge_events_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}/events");
        let auth_headers = make_nip98_headers(&keys, &url, "POST", b"{}");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            "/events",
            &host,
            auth_headers,
            b"{}",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: POST /events with valid NIP-98 + no assertion MUST deny 401 \
             [FI-TRACE-HTTP-INGRESS]; if this fails the admit_nip_fi_http_on_state gate was \
             removed from submit_event"
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "/events: exact MissingEvidence body must be 'authentication required\\n'"
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "/events: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "/events: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── F4: bridge POST /query — enforce mode, no assertion → 401 ───────────
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_bridge_query_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}/query");
        let auth_headers = make_nip98_headers(&keys, &url, "POST", b"[]");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            "/query",
            &host,
            auth_headers,
            b"[]",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: POST /query with valid NIP-98 + no assertion MUST deny 401 \
             [FI-TRACE-HTTP-INGRESS]; if this fails the admit_nip_fi_http_on_state gate was \
             removed from query_events"
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "/query: exact MissingEvidence body must be 'authentication required\\n'"
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "/query: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "/query: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── F4: bridge POST /count — enforce mode, no assertion → 401 ───────────
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_bridge_count_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}/count");
        let auth_headers = make_nip98_headers(&keys, &url, "POST", b"[]");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            "/count",
            &host,
            auth_headers,
            b"[]",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: POST /count with valid NIP-98 + no assertion MUST deny 401 \
             [FI-TRACE-HTTP-INGRESS]; if this fails the admit_nip_fi_http_on_state gate was \
             removed from count_events"
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "/count: exact MissingEvidence body must be 'authentication required\\n'"
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "/count: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "/count: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── F4: moderation GET — enforce mode, no assertion → 401 ───────────────
    //
    // Shared witness for all three moderation routes: they share
    // `authorize_moderation_read` which calls `admit_nip_fi_http_on_state`.
    //
    // The 401 is produced by the OUTER `nip_fi_assertion_guard` layer in
    // `build_router`: no `Nostr-Federated-Identity` header → MissingEvidence →
    // 401 `authentication required\n`.
    //
    // Note: removing only the outer guard does NOT change this test — the
    // handler's own `admit_nip_fi_http_on_state` also fires 401 on missing
    // assertion.  This test witnesses the outer guard fires first and its
    // error path is exercised; it does not claim the outer guard is the sole
    // denial point.  The same-key positive (below) is the complement witness.
    //
    // The exact body/CT/challenge oracles discriminate any implementation that
    // returns a different status or body (e.g. application-level 403 if both
    // admission layers were removed).
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_moderation_reports_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}/moderation/reports");
        let auth_headers = make_nip98_headers(&keys, &url, "GET", b"");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "GET",
            "/moderation/reports",
            &host,
            auth_headers,
            b"",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: GET /moderation/reports with valid NIP-98 + no assertion MUST \
             deny 401 [FI-TRACE-HTTP-INGRESS]; outer nip_fi_assertion_guard fires on missing \
             Nostr-Federated-Identity header → MissingEvidence → 401."
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "/moderation/reports: exact MissingEvidence body must be 'authentication required\\n'"
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "/moderation/reports: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "/moderation/reports: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── F4: GIF search — enforce mode, no assertion → 401 ───────────────────
    //
    // Shared witness for both GIF routes (search + share both go through
    // `authenticate` which calls `admit_nip_fi_http_on_state`).
    //
    // The 401 is produced by the OUTER `nip_fi_assertion_guard` layer in
    // `build_router`: no `Nostr-Federated-Identity` header → MissingEvidence →
    // 401 `authentication required\n`.
    //
    // Note: removing only the outer guard does NOT change this test — the
    // handler's own `admit_nip_fi_http_on_state` in `gifs::authenticate` also
    // fires 401 on missing assertion.  This test witnesses the outer guard fires
    // first and its error path is exercised; it does not claim the outer guard is
    // the sole denial point.  The same-key positive (below) is the complement witness.
    //
    // The exact body/CT/challenge oracles discriminate any implementation that
    // returns a different status or body (e.g. 404 if BOTH admission layers were
    // removed and Klipy config was absent).
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_gif_search_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}{}", crate::api::gifs::SEARCH_PATH);
        let auth_headers = make_nip98_headers(&keys, &url, "POST", b"{}");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            crate::api::gifs::SEARCH_PATH,
            &host,
            auth_headers,
            b"{}",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: POST {} with valid NIP-98 + no assertion MUST deny 401 \
             [FI-TRACE-HTTP-INGRESS]; outer nip_fi_assertion_guard fires on missing \
             Nostr-Federated-Identity header → MissingEvidence → 401.",
            crate::api::gifs::SEARCH_PATH
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "GIF search: exact MissingEvidence body must be 'authentication required\\n'. \
             Note: GIF 404 → NIP-FI 401 is a known exception (gifs.rs → bridge → api_error() \
             for the Off path); Enforce must still produce exact NIP-FI bytes."
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "GIF search: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "GIF search: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── F4: workflow runs — enforce mode, no assertion → 401 ────────────────
    //
    // Shared witness for both workflow routes (`authorize_workflow_read`
    // calls `admit_nip_fi_http_on_state`).
    //
    // The 401 is produced by the OUTER `nip_fi_assertion_guard` layer in
    // `build_router`: no `Nostr-Federated-Identity` header → MissingEvidence →
    // 401 `authentication required\n`.  The per-handler gate is unreachable.
    //
    // Removing only the outer guard does not change this 401: the request then
    // reaches `authorize_workflow_read`, whose `admit_nip_fi_http_on_state`
    // (workflows.rs) denies the same missing assertion with the same
    // MissingEvidence bytes.  The handler-level gate is witnessed separately by
    // the same-key positive and mismatched-key controls below.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_workflow_runs_no_assertion_is_401() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let workflow_id = uuid::Uuid::new_v4();
        let keys = Keys::generate();
        let path = format!("/workflows/{workflow_id}/runs");
        let url = format!("https://{host}{path}");
        let auth_headers = make_nip98_headers(&keys, &url, "GET", b"");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "GET",
            &path,
            &host,
            auth_headers,
            b"",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI enforce mode: GET {path} with valid NIP-98 + no assertion MUST deny 401 \
             [FI-TRACE-HTTP-INGRESS]; outer nip_fi_assertion_guard fires on missing \
             Nostr-Federated-Identity header → MissingEvidence → 401."
        );
        assert_eq!(
            body.as_ref(),
            b"authentication required\n",
            "{path}: exact MissingEvidence body must be 'authentication required\\n'"
        );
        let ct = resp_headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "{path}: 401 content-type must be text/plain; charset=utf-8"
        );
        let www_auth = resp_headers
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            www_auth, "Nostr",
            "{path}: 401 must carry WWW-Authenticate: Nostr"
        );
    }

    // ── GIF search — Enforce mode, same-key admission → reaches handler ───────
    //
    // Positive control for `nip_fi_enforce_gif_search_no_assertion_is_401`:
    // a valid NIP-FI assertion + valid same-key NIP-98 MUST pass admission and
    // reach the GIF handler.  The handler returns 404 (GIF search not configured)
    // — which is NOT 401/403, proving the NIP-FI gate did not deny the request.
    //
    // Falsifying mutation: make the NIP-FI verifier always-deny → same-key
    // request returns 403 AuthorizationDenied → 404 assertion fires.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_gif_search_same_key_admission_succeeds() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-gif-positive-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // `nip_fi_enforce_test_state` pins `config.klipy = None`.
        let keys = Keys::generate();
        let url = format!("https://{host}{}", crate::api::gifs::SEARCH_PATH);
        let headers = same_key_nip98_and_assertion_headers(&keys, &url, "POST", b"{}");

        let (status, _resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            crate::api::gifs::SEARCH_PATH,
            &host,
            headers,
            b"{}",
        ));

        // Admission passes → handler fires → GIF config absent → exact 404.
        // Falsifying mutation: make verifier always-deny → 403 AuthorizationDenied.
        assert_eq!(
            status,
            axum::http::StatusCode::NOT_FOUND,
            "GIF search same-key positive: NIP-FI MUST admit and handler MUST return 404 \
             (GIF provider not configured). \
             If 401: NIP-FI MissingEvidence — outer guard or assertion check denying. \
             If 403: NIP-FI AuthorizationDenied — verifier or pairing denying. \
             Body: {body:?}"
        );
        // Verify exact body: api_error(NOT_FOUND, "GIF search is not configured") → JSON.
        let body_json: serde_json::Value =
            serde_json::from_slice(&body).expect("404 body must be valid JSON");
        assert_eq!(
            body_json.get("error").and_then(|v| v.as_str()),
            Some("GIF search is not configured"),
            "GIF search same-key positive: exact 404 body must be JSON \
             {{\"error\":\"GIF search is not configured\"}}. \
             Falsifying mutation: make handler always-deny → 403 body differs."
        );
    }

    // ── Moderation reports — Enforce mode, same-key admission → reaches handler ─
    //
    // Positive control: a valid NIP-FI assertion + same-key NIP-98 on the
    // registered `/moderation/reports` route passes admission and reaches
    // `authorize_moderation_action`.  The unprivileged caller gets the
    // application 403 JSON `{"error":"restricted: moderator access required"}`
    // (`authorize_moderation_read` → `api_error`), which is distinguishable
    // from the NIP-FI text/plain `authorization denied\n` denial.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_moderation_reports_same_key_admission_succeeds() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-mod-positive-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // The caller holds no moderation role (`ensure_user` creates a member
        // row only), so `authorize_moderation_action(ViewQueue)` fails and
        // `authorize_moderation_read` maps it to an application JSON 403.
        let keys = Keys::generate();
        rt.block_on(async {
            state
                .db
                .ensure_user(
                    state
                        .db
                        .ensure_configured_community(&host)
                        .await
                        .expect("community")
                        .id,
                    keys.public_key().as_bytes(),
                )
                .await
                .expect("ensure_user");
        });

        let path = "/moderation/reports";
        let url = format!("https://{host}{path}");
        let headers = same_key_nip98_and_assertion_headers(&keys, &url, "GET", b"");

        let (status, resp_headers, body) = rt.block_on(oneshot_request_full(
            state, "GET", path, &host, headers, b"",
        ));

        // Admission passes — the caller is NOT a moderator so moderation returns
        // 403 with exact application body "restricted: moderator access required".
        // This is an application-level 403, not a NIP-FI denial.
        //
        // Distinguishing mutations:
        // - NIP-FI always-deny → "authorization denied\n" (text/plain) ≠ JSON body.
        // - Remove moderation authz check → 200 with empty results ≠ 403.
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "Moderation same-key positive: handler MUST reach moderation authz → \
             403 (caller is not a moderator). \
             If 401: NIP-FI MissingEvidence — assertion check denying. \
             If 200: moderation authz check was removed."
        );
        assert_eq!(
            resp_headers
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "Moderation same-key positive: application 403 must be JSON"
        );
        assert!(
            resp_headers.get("www-authenticate").is_none(),
            "Moderation same-key positive: application 403 carries no challenge"
        );
        assert_eq!(
            body.as_ref(),
            br#"{"error":"restricted: moderator access required"}"#,
            "Moderation same-key positive: exact 403 body must be JSON \
             {{\"error\":\"restricted: moderator access required\"}}. \
             If 'authorization denied\\n': NIP-FI AuthDenied — verifier or pairing denying. \
             Falsifying mutation: make verifier always-deny → text/plain body."
        );
    }

    // ── Workflow runs — Enforce mode, same-key admission → reaches handler ────
    //
    // Positive control for `nip_fi_enforce_workflow_runs_no_assertion_is_401`:
    // valid NIP-FI assertion + same-key NIP-98 passes admission and reaches the
    // workflow handler.  The handler returns 404 (no workflow with this UUID).
    //
    // Falsifying mutation: make the NIP-FI verifier always-deny → 403 instead
    // of 404 → assertion fires.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_workflow_runs_same_key_admission_succeeds() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-wf-positive-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let workflow_id = uuid::Uuid::new_v4();
        let keys = Keys::generate();
        let path = format!("/workflows/{workflow_id}/runs");
        let url = format!("https://{host}{path}");
        let headers = same_key_nip98_and_assertion_headers(&keys, &url, "GET", b"");

        let (status, _resp_headers, _body) = rt.block_on(oneshot_request_full(
            state, "GET", &path, &host, headers, b"",
        ));

        // Admission passes → workflow not found → 404.
        // NIP-FI denial would return 401 or 403 — neither is expected here.
        assert_eq!(
            status,
            axum::http::StatusCode::NOT_FOUND,
            "Workflow runs same-key positive: admission MUST pass and handler MUST \
             return 404 (workflow not found). \
             401 = NIP-FI MissingEvidence; 403 = NIP-FI AuthDenied/Cardinality. \
             Falsifying mutation: make verifier always-deny → 403 instead of 404."
        );
    }

    // ── Caller key-pairing witness: GIF mismatched key → 403 AuthorizationDenied ─
    //
    // Valid assertion signed for key_a, NIP-98 signed by key_b.  The key-pairing
    // check in `admit_nip_fi_http_on_state` fires → 403 `authorization denied\n`.
    //
    // This is the handler-level denial witness: the same-key positive above proves
    // admission passes when keys match; this proves the pairing check fires when
    // they don't.  Together they bound removing the pairing check from both sides.
    //
    // Falsifying mutation: remove key-pairing check from `admit_nip_fi_http` →
    // mismatched keys pass admission → 404 (GIF not configured) ≠ 403.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_gif_search_mismatched_key_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-gif-mismatch-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // key_nip98: signs the NIP-98 Authorization header.
        // key_assertion: signs the NIP-FI assertion (different pubkey → pairing mismatch).
        let key_nip98 = Keys::generate();
        let key_assertion = Keys::generate();
        let url = format!("https://{host}{}", crate::api::gifs::SEARCH_PATH);
        let assertion = signed_assertion_for_pubkey(&key_assertion.public_key().to_hex());

        let mut headers = make_nip98_headers(&key_nip98, &url, "POST", b"{}");
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {assertion}").parse().expect("valid header"),
        );

        let (status, _resp_headers, body) = rt.block_on(oneshot_request_full(
            state,
            "POST",
            crate::api::gifs::SEARCH_PATH,
            &host,
            headers,
            b"{}",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "GIF mismatched key MUST return 403 AuthorizationDenied. \
             Falsifying mutation: remove key-pairing check → admission passes \
             → 404 (GIF not configured) returned instead."
        );
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "GIF mismatched key 403 MUST carry exact body 'authorization denied\\n'."
        );
    }

    // ── Caller key-pairing witness: moderation mismatched key → 403 ─────────
    //
    // Mirror of the GIF case through the moderation route.
    // Falsifying mutation: remove key-pairing check → admission passes → 403 from
    // moderation authz (not NIP-FI) with different JSON body.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_moderation_mismatched_key_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!(
            "nip-fi-mod-mismatch-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key_nip98 = Keys::generate();
        let key_assertion = Keys::generate();
        let path = "/moderation/reports";
        let url = format!("https://{host}{path}");
        let assertion = signed_assertion_for_pubkey(&key_assertion.public_key().to_hex());

        let mut headers = make_nip98_headers(&key_nip98, &url, "GET", b"");
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {assertion}").parse().expect("valid header"),
        );

        let (status, _resp_headers, body) = rt.block_on(oneshot_request_full(
            state, "GET", path, &host, headers, b"",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "Moderation mismatched key MUST return 403 AuthorizationDenied. \
             Falsifying mutation: remove key-pairing check → admission passes \
             → 403 from moderation authz (different JSON body)."
        );
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "Moderation mismatched key 403 MUST carry exact body 'authorization denied\\n'. \
             If JSON 403: key-pairing was skipped, moderation authz fired instead."
        );
    }

    // ── Caller key-pairing witness: workflow mismatched key → 403 ───────────
    //
    // Mirror of the GIF/moderation cases through the workflow route.
    // Falsifying mutation: remove key-pairing check → admission passes → 404 (no
    // such workflow) rather than 403 AuthorizationDenied.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_workflow_mismatched_key_is_403() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state_with_verifier()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-wf-mismatch-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let key_nip98 = Keys::generate();
        let key_assertion = Keys::generate();
        let workflow_id = uuid::Uuid::new_v4();
        let path = format!("/workflows/{workflow_id}/runs");
        let url = format!("https://{host}{path}");
        let assertion = signed_assertion_for_pubkey(&key_assertion.public_key().to_hex());

        let mut headers = make_nip98_headers(&key_nip98, &url, "GET", b"");
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {assertion}").parse().expect("valid header"),
        );

        let (status, _resp_headers, body) = rt.block_on(oneshot_request_full(
            state, "GET", &path, &host, headers, b"",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "Workflow mismatched key MUST return 403 AuthorizationDenied. \
             Falsifying mutation: remove key-pairing check → admission passes \
             → 404 (workflow not found) returned instead."
        );
        assert_eq!(
            body.as_ref(),
            b"authorization denied\n",
            "Workflow mismatched key 403 MUST carry exact body 'authorization denied\\n'."
        );
    }

    // ── F4: bridge POST /query — off mode, no assertion → reaches application ─
    //
    // Regression guard [FI-INV-15]: in Off mode the NIP-FI gate MUST be
    // transparent. The request has no assertion header and no auth at all
    // (require_auth_token=false in off state). It MUST NOT produce a NIP-FI
    // denial (401/403/503). Any application-level response (even 404 or 500) is
    // acceptable — the gate was not the source.
    //
    // Falsifying mutation: enabling NIP-FI mode in the Off state would cause the
    // gate to fire; the response would be 401, not the downstream 401 from
    // missing auth. Wait — Off state has require_auth_token=false, so an
    // anonymous /query without any assertion would reach the application layer
    // and produce a non-NIP-FI response (could be 200 [] on an open relay). The
    // key observable: the status MUST NOT be produced by the NIP-FI gate in Off
    // mode. We verify by checking the response body is NOT the NIP-FI contract
    // text ("authentication required\n").
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_off_bridge_query_no_assertion_is_not_nip_fi_denied() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_off_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // X-Pubkey dev-mode auth: passes verify_bridge_auth (require_auth_token=false
        // in Off state) and reaches admit_nip_fi_http_on_state, which MUST admit
        // unconditionally in Off mode.
        //
        // We cannot use no-auth-at-all because verify_bridge_auth returns 401
        // ("missing Nostr auth") before the NIP-FI gate is reached, making the
        // assert_ne!(_, UNAUTHORIZED) trivially falsifiable for the wrong reason.
        // X-Pubkey is the correct dev-mode bypass when require_auth_token=false.
        let keys = nostr::Keys::generate();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-pubkey",
            keys.public_key().to_hex().parse().expect("valid header"),
        );

        let status = rt.block_on(oneshot_request(
            state, "POST", "/query", &host, headers, b"[]",
        ));

        // In Off mode the NIP-FI gate is transparent — any downstream status
        // (200, 400, 500) is acceptable.  The forbidden outcomes are NIP-FI
        // gate denials: 401 (Enforce missing_evidence) and 503 (DenyProtected).
        //
        // Mutation evidence: changing the test state to Enforce mode causes the
        // gate to fire (no Nostr-Federated-Identity header) returning 401, which
        // falsifies the first assert_ne.
        assert_ne!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI Off mode MUST NOT produce 401 from the gate [FI-INV-15]"
        );
        assert_ne!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "NIP-FI Off mode MUST NOT produce 503 from the gate [FI-INV-15]"
        );
    }

    // ── T2-seam: admitted malformed query through real handler → 400 ─────────
    //
    // Thufir's required seam test: one admitted malformed-query request through
    // a real affected handler (`moderation_reports`) asserting 400.
    //
    // ## What this proves
    //
    // With the old `.ok().unwrap_or_default()` behavior: `?status=open&limit=abc`
    // silently discarded ALL query fields (the entire `ModerationReadQuery`
    // became `Default`) and the handler returned 200 with all reports.
    // With `parse_query_or_400`: the handler returns 400 after admission.
    //
    // The test would fail against the old code because the handler would return
    // 200 (list all reports) rather than 400.
    //
    // ## Setup
    //
    // NIP-FI Off mode + `require_auth_token = false` allows X-Pubkey dev-mode
    // auth to bypass NIP-98 and NIP-FI gates, admitting the request to the
    // application layer.  The actor is seeded as community "owner" so the
    // moderation authz check passes without requiring real relay member rows.
    //
    // ## Falsifying mutation
    //
    // Revert `parse_query_or_400` to `.ok().unwrap_or_default()` in
    // `moderation_reports`.  The handler returns 200 (all reports for the
    // freshly created community — an empty array `[]`) instead of 400.
    // The `assert_eq!(status, BAD_REQUEST)` assertion panics.
    #[test]
    #[ignore = "requires Postgres"]
    fn t2_admitted_malformed_query_through_moderation_reports_is_400() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        // Off mode: NIP-FI gate is transparent; require_auth_token=false allows
        // X-Pubkey dev-mode auth to admit the request.
        let Some(state) = rt.block_on(nip_fi_off_test_state()) else {
            panic!("local Postgres not reachable");
        };

        let host = format!("t2-seam-{}.local", uuid::Uuid::new_v4().simple());
        let community = rt
            .block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // Seed the test actor as "owner" so moderation authz passes.
        let actor_keys = Keys::generate();
        let actor_hex = actor_keys.public_key().to_hex();
        rt.block_on(
            state
                .db
                .add_relay_member(community.id, &actor_hex, "owner", None),
        )
        .expect("seed actor as owner");

        // Build headers: X-Pubkey dev-mode admission (require_auth_token=false).
        // No Nostr-Federated-Identity header — NIP-FI is Off, so the guard is
        // transparent and the per-handler check admits unconditionally.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-pubkey", actor_hex.parse().expect("valid header"));

        // Malformed query: `status=open` is valid but `limit=abc` is not.
        // Old behavior: `.ok().unwrap_or_default()` → status=None, limit=None
        //   (all fields dropped), handler returns 200.
        // New behavior: `parse_query_or_400` → 400 BAD_REQUEST.
        let status = rt.block_on(oneshot_request(
            state,
            "GET",
            "/moderation/reports?status=open&limit=abc",
            &host,
            headers,
            b"",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "T2 seam: GET /moderation/reports?status=open&limit=abc after admission MUST \
             return 400; if this returns 200 the handler is still using .ok().unwrap_or_default() \
             which silently discards all query fields on parse error [FI-TRACE-HTTP-INGRESS T2]"
        );
    }

    // ── T1-IMP2: POST /internal/git/policy — Enforce mode → NOT 401 ─────────
    //
    // Verifies that `/internal/git/policy` is exempt from the NIP-FI guard in
    // Enforce mode.  The pre-receive hook callback carries no
    // Nostr-Federated-Identity assertion and must reach the policy handler's
    // own authorization layer, not be rejected by the guard.
    //
    // ## What this proves
    //
    // In Enforce mode, every non-exempt route without an assertion header gets
    // 401 (MissingEvidence) from `nip_fi_assertion_guard`.  `/internal/git/policy`
    // appears in `NIP_FI_EXEMPT_PREFIXES`, so the guard forwards it instead.
    // `require_localhost` then rejects (403) because Tower's `oneshot` does not
    // inject `ConnectInfo`.  A 403 proves the NIP-FI guard was NOT the rejector;
    // a 401 would mean the guard fired and the exempt entry is broken.
    //
    // ## Falsifying mutation
    //
    // Remove `"/internal/git/policy"` from `NIP_FI_EXEMPT_PREFIXES` in
    // `router.rs`.  The guard fires, returns 401, and the `assert_ne!(401)`
    // assertion panics.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_enforce_git_policy_callback_reaches_own_auth_not_nip_fi_guard() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_enforce_test_state()) else {
            panic!("local Postgres not reachable");
        };

        // Minimal syntactically-valid payload — the HMAC will fail (no real
        // hook secret), so the policy handler returns 403.  We only care that
        // the NIP-FI guard does NOT produce a 401 first.
        let body = br#"{
            "repo_id": "test-repo",
            "repo_owner": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "community_id": "test",
            "pusher_pubkey": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "ref_updates": [],
            "timestamp": 1234567890,
            "signature": "0000000000000000000000000000000000000000000000000000000000000000"
        }"#;

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().expect("valid header"),
        );
        // No Nostr-Federated-Identity header — the guard must pass this through.

        let status = rt.block_on(oneshot_request(
            state,
            "POST",
            "/internal/git/policy",
            "test.local",
            headers,
            body,
        ));

        assert_ne!(
            status,
            axum::http::StatusCode::UNAUTHORIZED,
            "NIP-FI Enforce mode: POST /internal/git/policy with no assertion must NOT \
             be denied by the NIP-FI guard (401); the pre-receive hook does not carry an \
             assertion and must reach the policy handler's own auth layer \
             [FI-TRACE-HTTP-INGRESS T1-IMP2]"
        );
        // The policy handler returns 403 (require_localhost check, since
        // Tower's oneshot does not inject ConnectInfo) — not 401 from the guard.
        // 403 proves the NIP-FI guard was not the rejector.
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "POST /internal/git/policy must reach its own authorization layer (403), \
             not be blocked at the NIP-FI guard layer (which would return 401)"
        );
    }

    // ── F4: bridge POST /query — deny_protected mode → 503 ──────────────────
    //
    // DenyProtected fires the gate unconditionally before any NIP-98 check,
    // returning 503 authorization_unavailable.
    //
    // Falsifying mutation: switching DenyProtected to Off or Enforce changes the
    // status — Off admits (non-401), Enforce needs assertion (401). Either way
    // this assert fails.
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_deny_protected_bridge_query_is_503() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let Some(state) = rt.block_on(nip_fi_deny_protected_test_state()) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // The request carries a valid NIP-98 event signed for the community's
        // actual URL (https://{host}/query), so the 503 cannot be attributed to
        // a proof failure.
        //
        // In DenyProtected the router's `nip_fi_assertion_guard` returns 503 for
        // this non-exempt route before the handler runs; `admit_nip_fi_http`
        // would also return 503 as its first step, before NIP-98 or the
        // assertion verifier.  Neither path runs NIP-98 first.
        let keys = Keys::generate();
        let url = format!("https://{host}/query");
        let auth_headers = make_nip98_headers(&keys, &url, "POST", b"[]");

        let status = rt.block_on(oneshot_request(
            state,
            "POST",
            "/query",
            &host,
            auth_headers,
            b"[]",
        ));

        // Mutation evidence: switching DenyProtected to Enforce causes the gate
        // to return 401 (no Nostr-Federated-Identity header present); switching
        // to Off causes the gate to admit and return a downstream status.  Either
        // change falsifies this assert_eq.
        assert_eq!(
            status,
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "NIP-FI DenyProtected mode: POST /query MUST deny 503 authorization_unavailable \
             [FI-TRACE-HTTP-INGRESS]; if this fails the admit_nip_fi_http_on_state gate was \
             removed or mode was changed"
        );
    }

    // ── T1-IMP1 (final): guard performs crypto verification, not just transport ──
    //
    // ## What this proves
    //
    // `nip_fi_assertion_guard` now performs the full offline assertion
    // verification — not just transport-level shape validation.  A structurally
    // valid but cryptographically invalid assertion (wrong signature) MUST be
    // denied by the guard with 403 `evidence_rejected`, before the handler fires.
    //
    // ## Why the test distinguishes guard vs per-handler
    //
    // The request carries a bad-sig assertion token but NO NIP-98
    // `Authorization: Nostr ...` header.  With `require_auth_token = true`:
    //
    //   • Guard intact: `verifier.verify_assertion(bad_token)` → EvidenceRejected
    //     → 403 (guard denies before handler fires).
    //
    //   • Guard mutated (step 2 removed): guard forwards.  Handler's NIP-98
    //     auth layer fires first → missing auth → 401.
    //
    // 403 ≠ 401, so the mutation turns this test RED.
    //
    // ## What "mandatory wiring" means
    //
    // The removed wiring in the falsifying mutation is the
    // `verifier.verify_assertion(token)` call in `nip_fi_assertion_guard`
    // (`router.rs`).  Removing it restores the old transport-only guard, which
    // forwards any structurally valid token to the handler.  That is the
    // "forgotten-gate" failure class: a handler that omits
    // `admit_nip_fi_http_on_state` would admit with an invalidly-signed
    // assertion if the guard doesn't verify.
    //
    // ## Verifier construction
    //
    // To get a distinguishable outcome, this test injects a real
    // `StaticIssuerKeySource`-backed verifier into the state (rather than
    // `nip_fi_verifier = None`), so that a bad-sig token produces a definite
    // 403 (not a startup-race 503 that a handler check would also produce).
    #[test]
    #[ignore = "requires Postgres"]
    fn nip_fi_guard_rejects_crypto_invalid_assertion_before_handler_fires() {
        use buzz_auth::{
            AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
            IssuerRegistry, StaticIssuerKeySource, TokenClass, VerifyAssertion,
        };
        use jsonwebtoken::{jwk::JwkSet, Algorithm};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        // ── 1. Build the test state with a real injected verifier ─────────────

        let Some(mut state) = rt.block_on(async {
            // Clone nip_fi_enforce_test_state setup, but return the state
            // before Arc-wrapping so we can inject the verifier.
            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://nip-fi-test.local".to_string();
            config.require_auth_token = true;
            config.require_relay_membership = false;
            config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;

            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
            Some(state)
        }) else {
            panic!("local Postgres not reachable");
        };

        // ── 2. Build the verifier with StaticIssuerKeySource + test key ───────
        //
        // The verifier is seeded with a known P-256 public key.  Tokens that
        // claim `iss=https://issuer.test` will be verified against this key.
        // A token with an all-zero signature will fail `InvalidSignatureOrClaims`
        // → DenialClass::EvidenceRejected → 403.
        //
        // Key constants match the canonical test key in buzz-auth
        // (verifier/tests.rs): TEST_JWK_X / TEST_JWK_Y / TEST_KID / ISSUER.
        const TEST_ISSUER: &str = "https://issuer.example";
        const TEST_AUDIENCE: &str = "https://relay.example";
        const TEST_KID: &str = "test-key-1";

        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": TEST_KID,
                "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
                "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA"
            }]
        }))
        .expect("valid test JWKS");

        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set = AssertionKeySet::new_for_test(TEST_ISSUER.to_owned(), 1, jwks, hard_deadline)
            .expect("valid test key set");

        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{TEST_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");

        let policy = IssuerPolicy::new(
            TEST_ISSUER.to_owned(),
            vec![TEST_AUDIENCE.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,   // skew_seconds
            3600, // max_assertion_age_seconds
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");

        let mut registry = IssuerRegistry::new();
        registry.insert(policy);

        let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
        ));

        state.nip_fi_verifier = Some(verifier);
        let state = Arc::new(state);

        let host = format!("nip-fi-seam-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        // ── 3. Build a structurally valid but cryptographically invalid token ─
        //
        // Header and claims match the verifier's expectations (correct issuer,
        // audience, exp, nostr_pubkey).  The signature is 64 zero bytes —
        // structurally valid base64url for an ES256 DER signature, but
        // cryptographically invalid.  The verifier will parse through to the
        // signature check and fail with EvidenceRejected (403).
        const BAD_SIG_TOKEN: &str = concat!(
            // Header: {"alg":"ES256","kid":"test-key-1"}
            "eyJhbGciOiJFUzI1NiIsImtpZCI6InRlc3Qta2V5LTEifQ",
            ".",
            // Claims: {"iss":"https://issuer.example","aud":"https://relay.example",
            //          "iat":1700000000,"exp":9999999999,
            //          "nostr_pubkey":"1234...cdef","sub":"test-subject"}
            "eyJpc3MiOiJodHRwczovL2lzc3Vlci5leGFtcGxlIiwiYXVkIjoiaHR0cHM6Ly9yZWxheS5leGFtcGxlIiwiaWF0IjoxNzAwMDAwMDAwLCJleHAiOjk5OTk5OTk5OTksIm5vc3RyX3B1YmtleSI6IjEyMzQ1Njc4OTBhYmNkZWYxMjM0NTY3ODkwYWJjZGVmMTIzNDU2Nzg5MGFiY2RlZjEyMzQ1Njc4OTBhYmNkZWYiLCJzdWIiOiJ0ZXN0LXN1YmplY3QifQ",
            ".",
            // Signature: 64 zero bytes (invalid)
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        );

        // Verify the token is structurally valid (3 dots, valid base64url segments)
        // but is actually rejected by the verifier:
        let verifier_check = state
            .nip_fi_verifier
            .as_deref()
            .expect("verifier injected")
            .verify_assertion(BAD_SIG_TOKEN);
        assert!(
            verifier_check.is_err(),
            "pre-condition: the bad-sig token MUST be rejected by the verifier; \
             if it passes, the test cannot distinguish guard-deny from handler-deny"
        );

        // ── 4. Send the request through the production router ─────────────────
        //
        // The request carries:
        //   • Nostr-Federated-Identity: Bearer <bad-sig token>  (structurally valid, bad sig)
        //   • NO Authorization: Nostr ...  (no NIP-98)
        //
        // Expected with guard verifying (current code):
        //   Guard calls verifier.verify_assertion(bad_token) → EvidenceRejected
        //   → 403 evidence_rejected before handler fires.
        //
        // Falsifying mutation (remove verifier.verify_assertion from guard):
        //   Guard forwards (step 2 removed) → handler's NIP-98 auth fires first
        //   → missing NIP-98 → 401.  403 ≠ 401 → test fails.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {BAD_SIG_TOKEN}")
                .parse()
                .expect("valid header"),
        );
        // Deliberately NO Authorization header (no NIP-98).

        let status = rt.block_on(oneshot_request(
            state, "POST", "/events", &host, headers, b"{}",
        ));

        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "NIP-FI enforce mode: POST /events with cryptographically invalid assertion \
             (bad sig) MUST deny 403 evidence_rejected from the guard before the handler \
             fires [FI-TRACE-AUTHORITY-UNIFORM, T1-IMP1]. \
             Falsifying mutation: remove verifier.verify_assertion from nip_fi_assertion_guard \
             → guard forwards → missing NIP-98 → 401 ≠ 403 → test fails."
        );
    }

    // ── R3 cardinality regression: actual-caller (/query) ────────────────────
    //
    // Proves that the cardinality gate in `admit_nip_fi_http` fires on actual
    // HTTP routes, not just the unit-level `admit_nip_fi_http` tests.
    //
    // The unit tests in nip_fi_http.rs prove the gate logic; this test proves
    // the gate is actually wired into the `/query` route through the full router.
    //
    // ## Off-mode auth-required compatibility control
    //
    // Off mode must NOT reject duplicate Authorization headers — `verify_bridge_auth`
    // used `.get()` (first-value) before NIP-FI.  FI-INV-15 requires that Off mode
    // preserves this behavior.  The state is built with `require_auth_token = true`
    // so the NIP-98 layer is active; single valid NIP-98 succeeds (200 []); a
    // valid-first / malformed-second duplicate also uses the first value and
    // succeeds (same 200 []).  The cardinality gate is bypassed in Off mode:
    // neither the single nor the duplicate case returns 403.
    //
    // Falsifying mutation: add a cardinality check before legacy auth in Off
    // mode → duplicate case returns 403 EvidenceRejected → assertion fires.
    //
    // ## Enforce-mode cardinality denial
    //
    // Enforce mode + a valid assertion + two Authorization headers must return
    // 403 EvidenceRejected from the cardinality gate BEFORE NIP-98 is parsed.
    // The assertion guard passes with a valid signed token; the cardinality check
    // inside `admit_nip_fi_http` then fires because `auth_count == 2`.
    //
    // Negative control: without a valid assertion the middleware 401s first and
    // the cardinality gate is never reached — the old test exercised the wrong path.
    //
    // Falsifying mutation: remove the cardinality gate in Enforce mode → the
    // two-header request passes cardinality, NIP-98 proceeds with keys2/url2
    // matching → pairing succeeds → handler returns 200 [] (same as single-header
    // positive control) → body "evidence rejected\n" assertion fires.
    //
    // ## Single-header same-key positive control
    //
    // A single Authorization header with the same valid assertion must NOT produce
    // a cardinality denial.  Without this, an always-denying implementation passes
    // the two-header test.  Exact success: status 200, body [].
    #[test]
    #[ignore = "requires Postgres and Redis"]
    fn r3_cardinality_actual_caller_query_off_passes_enforce_denies() {
        use buzz_auth::{
            AssertionKeySet, FederatedAssertionVerifier, FreshnessClass, IssuerPolicy,
            IssuerRegistry, StaticIssuerKeySource, TokenClass, VerifyAssertion,
        };
        use jsonwebtoken::{jwk::JwkSet, Algorithm};

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        // ── Off mode: require_auth_token=true, first-value semantics ─────────
        //
        // Build an Off state with `require_auth_token = true` so the NIP-98 gate
        // is active and can actually validate the token.  This differs from the
        // shared `nip_fi_off_test_state()` helper which uses `require_auth_token=false`.
        //
        // With auth required:
        //   Case 1: single valid NIP-98 → auth passes → handler → 200 [].
        //   Case 2: valid-first + malformed-second ("Nostr AAAA") → Off mode uses
        //           first-value semantics (.get() on Authorization) → same 200 [].
        //
        // Identity of the two results proves first-value semantics preserved.
        // Neither is 403: proves cardinality gate is not applied in Off mode.
        let Some(off_state) = rt.block_on(async {
            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://nip-fi-test.local".to_string();
            config.require_auth_token = true;
            config.require_relay_membership = false;
            config.nip_fi.mode = buzz_auth::NipFiMode::Off;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
            Some(Arc::new(state))
        }) else {
            panic!("local Postgres not reachable");
        };
        let host = format!("nip-fi-cardinality-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(off_state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let url = format!("https://{host}/query");
        // Build a valid single NIP-98 header value.
        let nip98_header_value = {
            let mut h = make_nip98_headers(&keys, &url, "POST", b"[]");
            h.remove(axum::http::header::AUTHORIZATION)
                .expect("authorization header")
        };
        // Malformed second header: valid Nostr scheme prefix, invalid payload.
        // "Nostr AAAA" decodes as 3 zero bytes — not a valid JSON Nostr event.
        let malformed_nostr_header: axum::http::HeaderValue =
            "Nostr AAAA".parse().expect("valid header bytes");

        // Case 1: single valid NIP-98 → auth passes → 200 [].
        let (single_off_status, _, single_off_body) = rt.block_on(oneshot_request_full(
            Arc::clone(&off_state),
            "POST",
            "/query",
            &host,
            {
                let mut h = axum::http::HeaderMap::new();
                h.append(
                    axum::http::header::AUTHORIZATION,
                    nip98_header_value.clone(),
                );
                h
            },
            b"[]",
        ));
        assert_eq!(
            single_off_status,
            axum::http::StatusCode::OK,
            "Off mode: single valid NIP-98 MUST reach the handler and return 200. \
             [FI-INV-15]"
        );
        assert_eq!(
            single_off_body.as_ref(),
            b"[]",
            "Off mode: single valid NIP-98 MUST return empty events array for empty filter set."
        );

        // Case 2: valid-first + malformed-second → Off uses first-value → same 200 [].
        // Identical values cannot distinguish first-value from last-value selection —
        // valid-first/invalid-second proves the first value is used, not the last.
        let (dup_off_status, _, dup_off_body) = rt.block_on(oneshot_request_full(
            off_state,
            "POST",
            "/query",
            &host,
            {
                let mut h = axum::http::HeaderMap::new();
                h.append(
                    axum::http::header::AUTHORIZATION,
                    nip98_header_value.clone(),
                );
                h.append(
                    axum::http::header::AUTHORIZATION,
                    malformed_nostr_header.clone(),
                );
                h
            },
            b"[]",
        ));
        assert_ne!(
            dup_off_status,
            axum::http::StatusCode::FORBIDDEN,
            "Off mode: duplicate Authorization headers MUST NOT produce 403 EvidenceRejected \
             from the cardinality gate [FI-INV-15]. Off mode must preserve first-value legacy \
             behavior — cardinality denial is an Enforce-only contract. \
             Falsifying mutation: add cardinality check in Off mode → 403 → assertion fires."
        );
        assert_eq!(
            single_off_status, dup_off_status,
            "Off mode: duplicate-header result must equal single-header result — \
             the first valid header is used (first-value semantics), \
             not treated as a cardinality violation."
        );
        assert_eq!(
            single_off_body, dup_off_body,
            "Off mode: single and dup bodies must match — first-value semantics \
             means the malformed second header is silently discarded."
        );

        // ── Enforce mode: build a state with a real injected verifier ─────────
        //
        // The verifier is required so the assertion guard can validate the signed
        // token and forward the request.  Without a verifier, the middleware 401s
        // before the cardinality gate inside `admit_nip_fi_http` can fire.
        let Some(mut enforce_state) = rt.block_on(async {
            let mut config = crate::config::Config::from_env().ok()?;
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://nip-fi-test.local".to_string();
            config.require_auth_token = true;
            config.require_relay_membership = false;
            config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .ok()?;
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .ok()?;
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .ok()?,
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
            let (mut state, _) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
            Some(state)
        }) else {
            panic!("local Postgres not reachable (enforce)");
        };

        // Inject the real verifier with the static test key.
        const TEST_ISSUER: &str = "https://issuer.example";
        const TEST_AUDIENCE: &str = "https://relay.example";
        const TEST_KID: &str = "test-key-1";
        // PKCS#8 private key matching TEST_JWK_X/Y — same key used by
        // nip_fi_guard_rejects_crypto_invalid_assertion_before_handler_fires.
        const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
            MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
            WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
            zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
            -----END PRIVATE KEY-----\n";

        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256",
                "kid": TEST_KID,
                "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
                "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA"
            }]
        }))
        .expect("valid test JWKS");

        let hard_deadline = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let key_set = AssertionKeySet::new_for_test(TEST_ISSUER.to_owned(), 1, jwks, hard_deadline)
            .expect("valid test key set");
        let jwks_contract = buzz_auth::JwksSourceContract::new(
            format!("{TEST_ISSUER}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid jwks contract");
        let policy = IssuerPolicy::new(
            TEST_ISSUER.to_owned(),
            vec![TEST_AUDIENCE.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .expect("valid issuer policy");
        let mut registry = IssuerRegistry::new();
        registry.insert(policy);
        let verifier: Arc<dyn VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
        ));
        enforce_state.nip_fi_verifier = Some(verifier);
        let enforce_state = Arc::new(enforce_state);

        let host2 = format!(
            "nip-fi-cardinality-enf-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        rt.block_on(enforce_state.db.ensure_configured_community(&host2))
            .expect("ensure community");

        // Mint a valid signed assertion for an arbitrary test pubkey.
        let assertion_pubkey_hex = nostr::Keys::generate().public_key().to_hex();
        let valid_assertion = {
            use jsonwebtoken::{Algorithm, EncodingKey, Header};
            let now = chrono::Utc::now().timestamp();
            let claims = serde_json::json!({
                "iss": TEST_ISSUER,
                "aud": TEST_AUDIENCE,
                "iat": now,
                "exp": now + 600,
                "sub": "test-subject",
                "nostr_pubkey": assertion_pubkey_hex,
            });
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(TEST_KID.to_owned());
            header.typ = Some("nip-fi+jwt".to_owned());
            let key =
                EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).expect("valid test EC PEM");
            jsonwebtoken::encode(&header, &claims, &key).expect("sign assertion")
        };

        // Pre-condition: verifier accepts the token.
        assert!(
            enforce_state
                .nip_fi_verifier
                .as_deref()
                .expect("verifier injected")
                .verify_assertion(&valid_assertion)
                .is_ok(),
            "pre-condition: valid assertion must be accepted by the verifier"
        );

        // Use the same key for both NIP-98 and the assertion's nostr_pubkey so
        // the pairing check succeeds and the request reaches the query handler.
        let keys2 = Keys::generate();
        let url2 = format!("https://{host2}/query");
        let nip98_val2 = {
            let mut h = make_nip98_headers(&keys2, &url2, "POST", b"[]");
            h.remove(axum::http::header::AUTHORIZATION)
                .expect("authorization header")
        };

        // Mint a same-key assertion: nostr_pubkey = keys2's public key.
        let same_key_assertion = {
            use jsonwebtoken::{Algorithm, EncodingKey, Header};
            let now = chrono::Utc::now().timestamp();
            let claims = serde_json::json!({
                "iss": TEST_ISSUER,
                "aud": TEST_AUDIENCE,
                "iat": now,
                "exp": now + 600,
                "sub": "test-subject",
                "nostr_pubkey": keys2.public_key().to_hex(),
            });
            let mut header = Header::new(Algorithm::ES256);
            header.kid = Some(TEST_KID.to_owned());
            header.typ = Some("nip-fi+jwt".to_owned());
            let key =
                EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).expect("valid test EC PEM");
            jsonwebtoken::encode(&header, &claims, &key).expect("sign same-key assertion")
        };
        // Pre-condition: same-key assertion is accepted.
        assert!(
            enforce_state
                .nip_fi_verifier
                .as_deref()
                .expect("verifier injected")
                .verify_assertion(&same_key_assertion)
                .is_ok(),
            "pre-condition: same-key assertion must be accepted"
        );

        // ── Same-key positive control: 1 Authorization header + same-key assertion ─
        //
        // One Authorization header passes the cardinality gate; the NIP-98 key
        // matches the assertion's nostr_pubkey → pairing succeeds → handler reached.
        //
        // Falsifying mutation: always return 403 from cardinality → this test
        // returns 403 EvidenceRejected → assertion fires.
        let mut single_headers = axum::http::HeaderMap::new();
        single_headers.append(axum::http::header::AUTHORIZATION, nip98_val2.clone());
        single_headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {same_key_assertion}")
                .parse()
                .expect("valid header"),
        );

        let single_resp = rt.block_on(async {
            use axum::body::{to_bytes, Body};
            use tower::ServiceExt;
            let mut builder = axum::http::Request::builder()
                .method("POST")
                .uri("/query")
                .header("host", &host2);
            for (name, value) in &single_headers {
                builder = builder.header(name, value);
            }
            let resp = crate::router::build_router(Arc::clone(&enforce_state))
                .oneshot(
                    builder
                        .body(Body::from(b"[]".to_vec()))
                        .expect("build request"),
                )
                .await
                .expect("router oneshot");
            let status = resp.status();
            let body = to_bytes(resp.into_body(), 4096).await.unwrap_or_default();
            (status, body)
        });

        // Single same-key: cardinality passes, pairing passes; handler reached.
        // Exact success: 200 [] (empty filter set on fresh community has no events).
        // Falsifying mutation: always-denying cardinality → 403 EvidenceRejected.
        assert_eq!(
            single_resp.0,
            axum::http::StatusCode::OK,
            "Single Authorization header + same-key assertion MUST return 200. \
             Falsifying mutation: lower the gate threshold to 1 → 403 EvidenceRejected."
        );
        assert_eq!(
            single_resp.1.as_ref(),
            b"[]",
            "Single Authorization header + same-key assertion MUST return empty events array \
             for empty filter set on a fresh community."
        );

        // ── Enforce mode: duplicate header + same-key assertion → cardinality 403 ─
        let mut enforce_headers = axum::http::HeaderMap::new();
        enforce_headers.append(axum::http::header::AUTHORIZATION, nip98_val2.clone());
        enforce_headers.append(axum::http::header::AUTHORIZATION, nip98_val2.clone());
        enforce_headers.insert(
            buzz_auth::CLIENT_ATTACHED_HEADER,
            format!("Bearer {same_key_assertion}")
                .parse()
                .expect("valid header"),
        );

        let (enforce_status, enforce_resp_headers, enforce_body) =
            rt.block_on(oneshot_request_full(
                Arc::clone(&enforce_state),
                "POST",
                "/query",
                &host2,
                enforce_headers,
                b"[]",
            ));

        assert_eq!(
            enforce_status,
            axum::http::StatusCode::FORBIDDEN,
            "Enforce mode: duplicate Authorization headers must yield 403 EvidenceRejected \
             from cardinality gate [FI-TRACE-DENIAL-ORACLE]. \
             Falsifying mutation: remove cardinality gate → NIP-98 closure runs → \
             handler returns 200 [] (same as single-header positive control)."
        );
        assert_eq!(
            enforce_body.as_ref(),
            b"evidence rejected\n",
            "Enforce mode: cardinality denial body must be exact contract bytes 'evidence rejected\\n'"
        );
        let enforce_ct = enforce_resp_headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            enforce_ct, "text/plain; charset=utf-8",
            "Enforce mode: cardinality 403 Content-Type must be 'text/plain; charset=utf-8'. \
             [FI-TRACE-DENIAL-ORACLE]"
        );
        assert!(
            enforce_resp_headers
                .get(axum::http::header::WWW_AUTHENTICATE)
                .is_none(),
            "Enforce mode: cardinality 403 MUST NOT emit WWW-Authenticate — \
             the client has a token but it is malformed, not absent. \
             [FI-TRACE-DENIAL-ORACLE]"
        );
    }

    /// T3c — log fidelity for canvas CAS conflict: the terminal attribution line
    /// must log `status=409`, not 400, when the relay emits a canvas CAS 409.
    ///
    /// Before the fix, `SubmitOutcome::Rejected` hardcoded `status = 400u16` in
    /// its logging arm, so every canvas CAS conflict — which now correctly
    /// returns HTTP 409 to the client — was misattributed as 400 in the relay
    /// log.  This test pins both the log fidelity and the 400 control so the
    /// distinction is exercised in the same run.
    ///
    /// - **CAS branch:** a stale canvas write (RevisionMismatch) must log `status=409`.
    /// - **Generic-rejection control:** a relay-only-kind event must log `status=400`.
    ///
    /// Discriminating: restoring `status = 400u16` in bridge.rs's `Rejected` logging
    /// arm causes the CAS `status=409` assertion to fail while the 400 control
    /// continues to pass — the test is split so the regression direction is unambiguous.
    #[test]
    #[ignore = "requires Postgres"]
    fn canvas_cas_conflict_logs_status_409_not_400() {
        use buzz_core::kind::KIND_CANVAS;
        use buzz_db::channel::{ChannelType, ChannelVisibility};
        use uuid::Uuid;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let state = rt
            .block_on(bridge_handler_test_state())
            .expect("local Postgres not reachable — start Postgres on 127.0.0.1:5432 before running ignored bridge handler tests");

        let (host, channel_id) = rt.block_on(async {
            let h = format!("canvas-cas-log-{}.local", Uuid::new_v4().simple());
            let community = state
                .db
                .ensure_configured_community(&h)
                .await
                .expect("ensure community");
            let creator_keys = nostr::Keys::generate();
            let (channel, _) = state
                .db
                .create_channel_with_id(
                    community.id,
                    Uuid::new_v4(),
                    &format!("log-test-{}", Uuid::new_v4().simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    creator_keys.public_key().to_bytes().as_slice(),
                    None,
                )
                .await
                .expect("create test channel");
            (h, channel.id.to_string())
        });

        let author_keys = Keys::generate();
        let pubkey_hex = author_keys.public_key().to_hex();
        let relay_now = chrono::Utc::now().timestamp() as u64;

        // Establish head A with an unconditional write.
        let event_a = EventBuilder::new(Kind::Custom(KIND_CANVAS as u16), "# head")
            .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
            .custom_created_at(nostr::Timestamp::from(relay_now))
            .sign_with_keys(&author_keys)
            .expect("sign event A");
        let event_a_id = event_a.id.to_hex();
        let body_a = serde_json::to_vec(&event_a).expect("serialize event A");
        // Accept A silently (no log assertion here).
        rt.block_on(post_events(state.clone(), &host, &pubkey_hex, &body_a));

        // Advance head to B.
        let event_b = EventBuilder::new(Kind::Custom(KIND_CANVAS as u16), "# head B")
            .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
            .tag(
                Tag::parse(["expected-revision", event_a_id.as_str()])
                    .expect("expected-revision tag"),
            )
            .custom_created_at(nostr::Timestamp::from(relay_now + 1))
            .sign_with_keys(&author_keys)
            .expect("sign event B");
        let body_b = serde_json::to_vec(&event_b).expect("serialize event B");
        rt.block_on(post_events(state.clone(), &host, &pubkey_hex, &body_b));

        // Stale write C: still expects A, but B is now head → RevisionMismatch → 409.
        // Capture the log to assert the logged status.
        let event_c = EventBuilder::new(Kind::Custom(KIND_CANVAS as u16), "# stale")
            .tag(Tag::parse(["h", channel_id.as_str()]).expect("h tag"))
            .tag(
                Tag::parse(["expected-revision", event_a_id.as_str()])
                    .expect("expected-revision tag"),
            )
            .custom_created_at(nostr::Timestamp::from(relay_now + 2))
            .sign_with_keys(&author_keys)
            .expect("sign event C");
        let body_c = serde_json::to_vec(&event_c).expect("serialize event C");

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let (status_cas, log_cas) = metrics::with_local_recorder(&recorder, || {
            run_and_capture(&rt, state.clone(), &host, &pubkey_hex, &body_c)
        });

        assert_eq!(
            status_cas,
            axum::http::StatusCode::CONFLICT,
            "canvas CAS conflict must yield HTTP 409"
        );
        // The log must record the real response status, not the former hardcoded 400.
        // Discriminating: restoring `status = 400u16` in the Rejected logging arm
        // makes this assertion fail while the generic-rejection control below still passes.
        assert!(
            log_cas.contains("status=409"),
            "terminal attribution line must log status=409 for canvas CAS conflict;\nlog:\n{log_cas}"
        );
        assert_eq!(
            count_attribution_lines(&log_cas),
            1,
            "exactly one attribution line for canvas CAS conflict;\nlog:\n{log_cas}"
        );

        // ── Generic-rejection control ────────────────────────────────────────
        // A relay-only-kind event is still a Rejected outcome → HTTP 400.
        // This control confirms the fix does not break generic-rejection logging.
        let relay_only_event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as u16),
            "",
        )
        .sign_with_keys(&author_keys)
        .expect("sign relay-only event");
        let relay_only_json = serde_json::to_vec(&relay_only_event).expect("serialize");

        let recorder2 = metrics_util::debugging::DebuggingRecorder::new();
        let (status_generic, log_generic) = metrics::with_local_recorder(&recorder2, || {
            run_and_capture(&rt, state.clone(), &host, &pubkey_hex, &relay_only_json)
        });

        assert_eq!(
            status_generic,
            axum::http::StatusCode::BAD_REQUEST,
            "generic rejection must still yield HTTP 400"
        );
        assert!(
            log_generic.contains("status=400"),
            "generic rejection must log status=400;\nlog:\n{log_generic}"
        );
        assert_eq!(
            count_attribution_lines(&log_generic),
            1,
            "exactly one attribution line for generic rejection;\nlog:\n{log_generic}"
        );
    }

    /// Drive a single POST /query request through the router and return the
    /// HTTP status code + body bytes.
    async fn post_query(
        state: Arc<crate::state::AppState>,
        host: &str,
        pubkey_hex: &str,
        body: &[u8],
    ) -> (axum::http::StatusCode, axum::body::Bytes) {
        use axum::body::Body;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let resp = crate::router::build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query")
                    .header(header::HOST, host)
                    .header("x-pubkey", pubkey_hex)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_vec()))
                    .expect("build request"),
            )
            .await
            .expect("router oneshot");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        (status, bytes)
    }

    // ── Bridge dispatch test: writer-pin routes to the writer pool ────────────
    //
    // Exercises the catchall dispatch loop's `ReadRoute` match at the shipping
    // seam — the production `match read_route { Writer => db.query_events(...),
    // Routed => db.query_events_routed(...) }` block.
    //
    // The test stages divergent data: a kind-40100 canvas event is inserted into
    // the writer pool only. The replica pool starts empty for that channel. With
    // the fence open and a bounded-staleness budget set, `query_events_routed`
    // routes to the replica and sees nothing. `query_events` reads the writer
    // and sees the event.
    //
    // DoD sequence:
    //   1. Routed read (no consistency field) → replica → event absent. This
    //      proves the replica path is genuinely live in this harness; otherwise
    //      the strong-read probe proves nothing.
    //   2. Strong read (consistency=strong) → writer → event present.
    //   3. Malformed consistency value → 400.
    //
    // Mutation oracle: changing the `ReadRoute::Writer` arm to call
    // `db.query_events_routed` makes probe 2 return empty (same as probe 1) →
    // the `assert_eq!(strong_events.len(), 1)` assertion fails. This is the
    // direct evidence Thufir required: the dispatch IS the seam, and breaking
    // the arm breaks this test.
    //
    // Infrastructure: two scratch Postgres databases on the local instance.
    // Requires the same local Postgres as the other `#[ignore]` bridge tests.
    #[test]
    #[ignore = "requires Postgres"]
    fn strong_consistency_dispatches_to_writer_pool_not_replica() {
        use buzz_core::CommunityId;
        use buzz_db::channel::{ChannelType, ChannelVisibility};
        use sqlx::PgPool;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");

        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| crate::test_support::database_url());

        // Create a scratch database and run migrations on it.
        async fn scratch_db(admin: &PgPool, admin_url: &str, suffix: &str) -> (PgPool, String) {
            let name = format!(
                "bridge_dispatch_{}_{}",
                suffix,
                uuid::Uuid::new_v4().simple()
            );
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                .execute(admin)
                .await
                .unwrap_or_else(|e| panic!("create scratch db {name}: {e}"));
            let slash = admin_url
                .rfind('/')
                .expect("URL must have a path component");
            let url = format!("{}/{name}", &admin_url[..slash]);
            let pool = PgPool::connect(&url)
                .await
                .unwrap_or_else(|e| panic!("connect scratch db {name}: {e}"));
            buzz_db::migration::run_migrations(&pool)
                .await
                .unwrap_or_else(|e| panic!("migrate scratch db {name}: {e}"));
            (pool, name)
        }

        async fn drop_scratch(admin: &PgPool, pool: PgPool, name: &str) {
            drop(pool);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
            )))
            .execute(admin)
            .await;
        }

        // --- setup -----------------------------------------------------------
        let admin = rt.block_on(PgPool::connect(&admin_url)).expect(
            "connect admin pool — start local Postgres before running ignored bridge tests",
        );

        let (writer_pool, writer_name) = rt.block_on(scratch_db(&admin, &admin_url, "w"));
        let (replica_pool, replica_name) = rt.block_on(scratch_db(&admin, &admin_url, "r"));

        let community = uuid::Uuid::new_v4();
        let channel_id = uuid::Uuid::new_v4();
        let host = format!("dispatch-test-{}.local", community.simple());
        let author = Keys::generate();

        // Seed community + open channel on both writer and replica.
        rt.block_on(async {
            for pool in [&writer_pool, &replica_pool] {
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(community)
                    .bind(&host)
                    .execute(pool)
                    .await
                    .expect("seed community");
                buzz_db::channel::create_channel_with_id(
                    pool,
                    CommunityId::from_uuid(community),
                    channel_id,
                    &format!("canvas-{}", channel_id.simple()),
                    ChannelType::Stream,
                    ChannelVisibility::Open,
                    None,
                    author.public_key().to_bytes().as_slice(),
                    None,
                )
                .await
                .expect("create channel");
            }
        });

        // Writer-only canvas event: inserted on writer, NOT replicated.
        let canvas_ev = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_CANVAS as u16),
            "writer-only canvas content",
        )
        .tag(Tag::custom(
            nostr::TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::H)),
            [channel_id.to_string()],
        ))
        .sign_with_keys(&author)
        .expect("sign canvas event");

        rt.block_on(async {
            let db_w = buzz_db::Db::from_pool(writer_pool.clone());
            db_w.insert_event(
                CommunityId::from_uuid(community),
                &canvas_ev,
                Some(channel_id),
            )
            .await
            .expect("insert canvas event on writer");
            // replica_pool deliberately receives no canvas events.
        });

        // --- build AppState with two-pool Db ---------------------------------
        let state = rt.block_on(async {
            let mut config = crate::config::Config::from_env()
                .expect("Config::from_env required — set DATABASE_URL, REDIS_URL, etc.");
            config.database_url = crate::test_support::database_url();
            config.redis_url =
                std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
            config.relay_url = "wss://dispatch-test.local".to_string();
            config.require_auth_token = false;
            config.require_relay_membership = false;

            let mut db = buzz_db::Db::from_pools(writer_pool.clone(), replica_pool.clone());
            // Open the freshness fence and set a bounded-staleness budget so
            // `query_events_routed` actually routes to the replica pool.
            db.fence().force_open_for_tests(chrono::Utc::now());
            db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));

            let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .expect("redis pool");
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .expect("pubsub manager"),
            );
            let audit = buzz_audit::AuditService::new(writer_pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(writer_pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");

            let (mut state, _audit_shutdown) = crate::state::AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                Keys::generate(),
                media_storage,
            );
            state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
            Arc::new(state)
        });

        let pubkey_hex = author.public_key().to_hex();
        let channel_str = channel_id.to_string();

        // Probe 1: routed read (no consistency) → replica → event absent.
        // This proves the replica path is genuinely live in this harness.
        let body = serde_json::to_vec(&serde_json::json!([{
            "kinds": [buzz_core::kind::KIND_CANVAS as u64],
            "#h": [&channel_str],
            "limit": 10,
        }]))
        .expect("serialize routed filter");
        let (status, resp_body) = rt.block_on(post_query(state.clone(), &host, &pubkey_hex, &body));
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "Probe 1: routed query must return 200: {}",
            String::from_utf8_lossy(&resp_body)
        );
        let routed_events: Vec<serde_json::Value> =
            serde_json::from_slice(&resp_body).expect("parse routed response");
        assert!(
            routed_events.is_empty(),
            "Probe 1 FAIL — routed read must NOT see writer-only canvas event \
             (replica pool is empty for this channel): {routed_events:?}"
        );

        // Probe 2: writer-pinned read (consistency=strong) → writer pool → event present.
        // Mutation oracle: changing Writer arm to query_events_routed → probe 2 returns
        // empty → assertion fails.
        let body = serde_json::to_vec(&serde_json::json!([{
            "kinds": [buzz_core::kind::KIND_CANVAS as u64],
            "#h": [&channel_str],
            "limit": 10,
            "consistency": "strong",
        }]))
        .expect("serialize strong filter");
        let (status, resp_body) = rt.block_on(post_query(state.clone(), &host, &pubkey_hex, &body));
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "Probe 2: strong query must return 200: {}",
            String::from_utf8_lossy(&resp_body)
        );
        let strong_events: Vec<serde_json::Value> =
            serde_json::from_slice(&resp_body).expect("parse strong response");
        assert_eq!(
            strong_events.len(),
            1,
            "Probe 2 FAIL — strong-consistency read MUST see the writer-only canvas event. \
             Mutation oracle: if ReadRoute::Writer dispatches to query_events_routed instead \
             of query_events, this returns empty and this assertion fails: {strong_events:?}"
        );
        assert_eq!(
            strong_events[0].get("content").and_then(|v| v.as_str()),
            Some("writer-only canvas content"),
            "strong read must return the canvas event inserted into the writer pool"
        );

        // Probe 3: malformed consistency value must 400.
        let body = serde_json::to_vec(&serde_json::json!([{
            "kinds": [buzz_core::kind::KIND_CANVAS as u64],
            "#h": [&channel_str],
            "consistency": "weak",
        }]))
        .expect("serialize bad filter");
        let (status, _) = rt.block_on(post_query(state.clone(), &host, &pubkey_hex, &body));
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "Probe 3 FAIL — unknown consistency value must be rejected with 400"
        );

        // --- teardown --------------------------------------------------------
        rt.block_on(async {
            let admin2 = PgPool::connect(&admin_url)
                .await
                .expect("reconnect admin for teardown");
            drop_scratch(&admin2, writer_pool, &writer_name).await;
            drop_scratch(&admin2, replica_pool, &replica_name).await;
        });
    }
}
