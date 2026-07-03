//! Nostr HTTP bridge — POST /events, /query, /count with NIP-98 auth.
//!
//! These endpoints provide HTTP access to the relay's Nostr protocol,
//! authenticated via NIP-98 signed events.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use base64::Engine;
use serde_json::Value;

use buzz_auth::{Nip98ReplayGuard, DEFAULT_REPLAY_TTL_SECS};
use buzz_core::TenantContext;

use crate::handlers::ingest::{IngestAuth, IngestError};
use crate::state::AppState;

use super::{api_error, internal_error, not_found};

/// Verify bridge auth: NIP-98 (production) or X-Pubkey (dev mode).
///
/// Returns the authenticated public key and an event ID for replay detection.
/// For X-Pubkey dev mode, the event ID is a zero hash (no replay concern).
fn verify_bridge_auth(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
) -> Result<(nostr::PublicKey, [u8; 32]), (StatusCode, Json<Value>)> {
    // Try NIP-98 first (Authorization: Nostr <base64>)
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

        let pubkey = buzz_auth::verify_nip98_event(&event_json, url, method, body)
            .map_err(|e| api_error(StatusCode::UNAUTHORIZED, &format!("NIP-98: {e}")))?;

        return Ok((pubkey, event_id_bytes));
    }

    // Dev-mode fallback: X-Pubkey header (only when require_auth_token is false)
    if !require_auth_token {
        if let Some(hex_val) = headers.get("x-pubkey").and_then(|v| v.to_str().ok()) {
            let pubkey = nostr::PublicKey::from_hex(hex_val)
                .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid X-Pubkey hex"))?;
            // Zero event ID — no replay detection needed for dev mode
            return Ok((pubkey, [0u8; 32]));
        }
    }

    Err(api_error(StatusCode::UNAUTHORIZED, "missing Nostr auth"))
}

/// Check NIP-98 replay and record the event ID atomically.
///
/// The correctness boundary is the shared, community-scoped Redis seen-set on
/// `AppState`, not process-local memory. Any Redis/guard error fails closed:
/// without the shared `SET NX EX` proof, a stateless worker cannot admit the
/// NIP-98 request safely.
async fn check_nip98_replay(
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
fn nip98_expected_url(config_relay_url: &str, tenant: &TenantContext, path: &str) -> String {
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

    let window = state
        .db
        .get_channel_window(
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
    //    One round trip for the client instead of an #e fan-out.
    if extension_flag(raw, "include_aux") && !row_ids_hex.is_empty() {
        let mut seen_aux: std::collections::HashSet<nostr::EventId> =
            std::collections::HashSet::new();
        let mut hop_ids = row_ids_hex.clone();
        for hop_kinds in [&WINDOW_AUX_KINDS[..], &WINDOW_AUX_DELETE_KINDS[..]] {
            let mut aux_query = buzz_db::EventQuery::for_community(tenant.community());
            aux_query.kinds = Some(hop_kinds.iter().map(|k| *k as i32).collect());
            aux_query.e_tags = Some(std::mem::take(&mut hop_ids));
            aux_query.limit = Some(1000);
            let aux_events = state
                .db
                .query_events(&aux_query)
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

/// Submit a signed Nostr event via HTTP bridge (NIP-98 auth).
pub async fn submit_event(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
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
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/events");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, &tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    // Enforce relay membership (with NIP-OA fallback via x-auth-tag header).
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    let event: nostr::Event = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid event JSON: {e}")))?;

    // Mesh signaling kinds (24620 status report, 24621 connect request) are
    // ephemeral and deliberately absent from ingest_event's per-kind allowlist.
    // The desktop's Rust coordinator publishes them via this bridge, so route
    // them to the mesh handlers — the HTTP twin of the WS door's special-casing
    // in handlers::event. Membership was enforced above; the handlers re-check
    // it fail-closed.
    let kind_u32 = buzz_core::kind::event_kind_u32(&event);
    if kind_u32 == buzz_core::kind::KIND_MESH_STATUS_REPORT
        || kind_u32 == buzz_core::kind::KIND_MESH_CONNECT_REQUEST
    {
        let event_id = event.id.to_hex();
        return match crate::handlers::mesh_signaling::handle_mesh_event_http(
            &state, &tenant, &pubkey, &event,
        )
        .await
        {
            Ok(()) => Ok(Json(serde_json::json!({
                "event_id": event_id,
                "accepted": true,
                "message": "",
            }))),
            Err(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
        };
    }

    let auth = IngestAuth::Http {
        pubkey,
        scopes: buzz_auth::Scope::all_known(), // Pure Nostr: full scopes, channel access via membership
        auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
    };

    match crate::handlers::ingest::ingest_event(&state, &tenant, event, auth).await {
        Ok(result) => Ok(Json(serde_json::json!({
            "event_id": result.event_id,
            "accepted": result.accepted,
            "message": result.message,
        }))),
        Err(e) => match e {
            IngestError::Rejected(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
            IngestError::AuthFailed(msg) => Err(api_error(StatusCode::FORBIDDEN, &msg)),
            IngestError::Internal(msg) => Err(internal_error(&msg)),
        },
    }
}

/// Query events via HTTP bridge (NIP-98 auth). Returns JSON array of events.
///
/// Enforces channel access: results are filtered to channels the user can access.
pub async fn query_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
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
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/query");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, &tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    // Two-pass parse: preserve raw JSON for custom extension fields (before_id,
    // depth_limit, feed_types) that nostr::Filter silently drops.
    let raw_filters: Vec<Value> = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;
    let filters: Vec<nostr::Filter> = raw_filters
        .iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<_, _>>()
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;

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

    // Get channels this user can access — same enforcement as WS REQ handler.
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;

    if filters.iter().any(|f| f.search.is_some()) {
        if has_mixed_search_filters(&filters) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "mixed search and non-search filters not supported",
            ));
        }
        return handle_bridge_search(
            &state,
            &raw_filters,
            &filters,
            &accessible_channels,
            &tenant,
            &authed_pubkey_hex,
            &pubkey_bytes,
        )
        .await;
    }

    if let Some(presence_events) = synthesize_presence(&state, &tenant, &filters).await {
        return Ok(Json(Value::Array(presence_events)));
    }

    let mut events: Vec<Value> = Vec::new();
    let mut handled: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Channel-window filters (`top_level: true`) — the GUI read-model surface.
    // Dispatched first: a window filter is never a feed/thread/catchall query.
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if !extension_flag(raw, "top_level") {
            continue;
        }
        handle_channel_window_filter(
            &state,
            &tenant,
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
                    .query_feed_mentions(
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
                    .query_feed_needs_action(
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
                    .query_feed_activity(tenant.community(), &accessible_channels, since, remaining)
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
            if let Ok(v) = serde_json::to_value(&se.event) {
                events.push(v);
            }
        }
        handled.insert(idx);
    }

    // Phase 1 — pure construction + validation, in filter order. Access-scope
    // skips and the `before_id` BAD_REQUEST are decided here, before any DB
    // work is issued (validation errors are deterministic client mistakes, so
    // surfacing them ahead of transient DB errors is strictly more predictable).
    let mut catchall_queries: Vec<(usize, buzz_db::EventQuery)> = Vec::new();
    for (idx, (raw, filter)) in raw_filters.iter().zip(filters.iter()).enumerate() {
        if handled.contains(&idx) {
            continue;
        }

        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue;
            }
        }

        let mut query = crate::handlers::req::build_event_query_from_filter(
            filter,
            &pubkey_bytes,
            &state,
            tenant.community(),
        )
        .await;

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

        catchall_queries.push((idx, query));
    }

    // Phase 2 — DB reads, bounded-concurrent, order-preserving (`buffered`).
    // Phase 3 consumes results in original filter order, so response ordering
    // and error semantics match the previous serial loop.
    use futures_util::stream::{self, StreamExt};
    let db = state.db.clone();
    let mut catchall_results = stream::iter(catchall_queries.into_iter().map(|(idx, query)| {
        let db = db.clone();
        async move { (idx, db.query_events(&query).await) }
    }))
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
                    if !buzz_core::filter::reader_authorized_for_event(
                        &se.event,
                        &authed_pubkey_hex,
                    ) {
                        continue;
                    }
                    if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes) {
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

/// Count events via HTTP bridge (NIP-98 auth). Returns `{"count": N}`.
///
/// Enforces channel access: only counts events in channels the user can access.
/// For filters without a `#h` tag, falls back to per-event counting with access checks.
pub async fn count_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
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
        })?;

    let url = nip98_expected_url(&state.config.relay_url, &tenant, "/count");
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        &headers,
        "POST",
        &url,
        Some(&body),
        state.config.require_auth_token,
    )?;
    check_nip98_replay(&state, &tenant, event_id_bytes).await?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    let filters: Vec<nostr::Filter> = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid filters: {e}")))?;

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
    let accessible_channels = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|e| internal_error(&format!("channel access lookup: {e}")))?;

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

        // If filter targets a specific channel, verify access.
        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue; // Skip filters targeting inaccessible channels.
            }
            // Channel is accessible — count with pushability check.
            let query = crate::handlers::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                &state,
                tenant.community(),
            )
            .await;
            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
            {
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query + post-filter for non-pushable constraints.
                let mut q = query;
                crate::handlers::req::apply_count_fallback_limit(&mut q);
                match state.db.query_events(&q).await {
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
                            if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes)
                            {
                                continue;
                            }
                            if !buzz_core::filter::reader_authorized_for_event(
                                &se.event,
                                &authed_pubkey_hex,
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
                &state,
                tenant.community(),
            )
            .await;
            query.channel_ids = Some(accessible_channels.to_vec());

            let author_is_self = filter.authors.as_ref().is_some_and(|authors| {
                !authors.is_empty()
                    && authors
                        .iter()
                        .all(|a| a.to_hex().eq_ignore_ascii_case(&authed_pubkey_hex))
            });
            if crate::handlers::req::filter_fully_pushable(filter)
                && (!needs_author_only_filtering || author_is_self)
                && !needs_result_gated_filtering
            {
                query.limit = None;
                match state.db.count_events(&query).await {
                    Ok(n) => total += n as u64,
                    Err(e) => {
                        return Err(internal_error(&format!("count error: {e}")));
                    }
                }
            } else {
                // Fallback: query a bounded candidate set + post-filter.
                crate::handlers::req::apply_count_fallback_limit(&mut query);
                match state.db.query_events(&query).await {
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
                            if crate::handlers::req::is_author_only_event(&se.event, &pubkey_bytes)
                            {
                                continue;
                            }
                            if !buzz_core::filter::reader_authorized_for_event(
                                &se.event,
                                &authed_pubkey_hex,
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
            .get_events_by_ids(tenant.community(), &id_refs)
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
            if crate::handlers::req::is_author_only_event(&stored.event, pubkey_bytes) {
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
                        Some(&format!("definition parse error: {e}")),
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
/// Returns `Some(events)` if handled, `None` to fall through to normal query.
async fn synthesize_presence(
    state: &AppState,
    tenant: &buzz_core::tenant::TenantContext,
    filters: &[nostr::Filter],
) -> Option<Vec<Value>> {
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
        return Some(Vec::new());
    }

    // Dedup pubkeys.
    all_pubkeys.sort_by_key(|pk| pk.to_hex());
    all_pubkeys.dedup();

    // Look up Redis.
    let presence_map = state
        .pubsub
        .get_presence_bulk(tenant, &all_pubkeys)
        .await
        .unwrap_or_default();

    if presence_map.is_empty() {
        return Some(Vec::new());
    }

    // Synthesize kind:20001 events signed by the relay.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut events = Vec::with_capacity(presence_map.len());
    for (pubkey_hex, status) in &presence_map {
        // Build a synthetic event: relay-signed, content = status, p-tag = subject.
        let tags = vec![nostr::Tag::parse(["p", pubkey_hex]).ok()?];
        let event =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_PRESENCE_UPDATE as u16), status)
                .tags(tags)
                .custom_created_at(nostr::Timestamp::from(now))
                .sign_with_keys(&state.relay_keypair)
                .ok()?;

        if let Ok(v) = serde_json::to_value(&event) {
            events.push(v);
        }
    }

    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag};

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
    #[tokio::test]
    #[ignore = "requires Redis"]
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
    #[tokio::test]
    #[ignore = "requires Redis"]
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
            fn try_mark<'a>(
                &'a self,
                _ctx: &'a buzz_core::TenantContext,
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

        let (pubkey, _event_id_bytes) =
            verify_bridge_auth(&headers, "POST", &expected_url, Some(b""), true)
                .expect("matching-host NIP-98 event must verify");
        assert_eq!(
            pubkey,
            keys.public_key(),
            "returned pubkey must be the signer's"
        );
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
}
