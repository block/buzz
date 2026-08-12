//! Nostr HTTP bridge — POST /events, /query, /count with NIP-98 auth.
//!
//! These endpoints provide HTTP access to the relay's Nostr protocol,
//! authenticated via NIP-98 signed events.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use base64::Engine;
use serde_json::Value;

use buzz_auth::{
    LimitType, Nip98ReplayGuard, ProofTransport, RouteCapability, DEFAULT_REPLAY_TTL_SECS,
};
use buzz_core::TenantContext;

use crate::handlers::ingest::{IngestAuth, IngestError};
use crate::state::AppState;

use super::{api_error, internal_error, not_found};

async fn enforce_http_admission(
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
            metrics::counter!("buzz_admission_rejections_total", "transport" => "http", "reason" => "quota").increment(1);
            Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("rate-limited: quota exceeded; retry in {reset_in_secs}s"),
            ))
        }
        Err(crate::admission::AdmissionError::Unavailable) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "http", "reason" => "unavailable").increment(1);
            Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "rate-limited: shared admission unavailable",
            ))
        }
    }
}

/// Verify bridge auth: NIP-98 (production) or X-Pubkey (dev mode).
///
/// Returns the authenticated public key and an event ID for replay detection.
/// For X-Pubkey dev mode, the event ID is a zero hash (no replay concern).
pub(crate) fn verify_bridge_auth(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
) -> Result<(nostr::PublicKey, [u8; 32]), (StatusCode, Json<Value>)> {
    verify_bridge_auth_with_options(headers, method, url, body, require_auth_token, false)
}

pub(crate) fn verify_bridge_auth_with_options(
    headers: &HeaderMap,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    require_auth_token: bool,
    require_payload: bool,
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

pub(crate) fn exact_nip98_authorization_event(headers: &HeaderMap) -> Option<Arc<str>> {
    let mut values = headers.get_all("authorization").iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let encoded = value.strip_prefix("Nostr ")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    String::from_utf8(bytes).ok().map(Arc::<str>::from)
}

/// Corporate identity enrollment must always start from cryptographic proof of
/// the Nostr key. The development-only `X-Pubkey` fallback is caller-controlled
/// and therefore cannot safely participate in a durable identity binding.
fn bridge_requires_nip98(require_auth_token: bool, require_corporate_identity: bool) -> bool {
    require_auth_token || require_corporate_identity
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

async fn verify_bridge_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    auth_tag: Option<&str>,
) -> Result<crate::corporate_identity::CorporateIdentityProof, (StatusCode, Json<Value>)> {
    let identity_jwt = crate::corporate_identity::identity_jwt_from_headers(
        headers,
        &state.config.corporate_identity,
    );
    crate::corporate_identity::verify_corporate_identity(
        state,
        tenant.community(),
        pubkey,
        identity_jwt.as_deref(),
        auth_tag,
    )
    .await
    .map_err(|e| e.into_api_error())
}

async fn finalize_bridge_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    pubkey: nostr::PublicKey,
    proof: crate::corporate_identity::CorporateIdentityProof,
) -> Result<(), (StatusCode, Json<Value>)> {
    crate::corporate_identity::finalize_corporate_identity(state, tenant.community(), pubkey, proof)
        .await
        .map(|_| ())
        .map_err(|e| e.into_api_error())
}

async fn enforce_bridge_membership(
    state: &AppState,
    tenant: &TenantContext,
    pubkey_bytes: &[u8],
    auth_tag: Option<&str>,
) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<Value>)> {
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        pubkey_bytes,
        auth_tag,
    )
    .await
    .map(|owner| {
        owner.or_else(|| {
            if state.config.require_relay_membership {
                None
            } else {
                super::relay_members::extract_nip_oa_owner(pubkey_bytes, auth_tag)
            }
        })
    })
}

async fn verify_bridge_identity_for_mode(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    auth_tag: Option<&str>,
) -> Result<Option<crate::corporate_identity::CorporateIdentityProof>, (StatusCode, Json<Value>)> {
    match state.config.nip_fi_mode {
        buzz_auth::NipFiMode::Off => {
            verify_bridge_corporate_identity(state, tenant, headers, pubkey, auth_tag)
                .await
                .map(Some)
        }
        buzz_auth::NipFiMode::Enforce => Ok(None),
        buzz_auth::NipFiMode::DenyProtected => Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: protected ingress denied",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn finalize_bridge_identity_for_mode(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    legacy_proof: Option<crate::corporate_identity::CorporateIdentityProof>,
    method: &str,
    expected_url: &str,
    body: Option<&[u8]>,
    capability: RouteCapability,
    ingress: crate::authorization_runtime::ProtectedIngress,
) -> Result<(), (StatusCode, Json<Value>)> {
    match state.config.nip_fi_mode {
        buzz_auth::NipFiMode::Off => {
            let proof = legacy_proof.ok_or_else(|| {
                api_error(StatusCode::UNAUTHORIZED, "identity verification required")
            })?;
            finalize_bridge_corporate_identity(state, tenant, pubkey, proof).await
        }
        buzz_auth::NipFiMode::Enforce => {
            authorize_canonical_bridge(
                state,
                tenant,
                headers,
                pubkey,
                method,
                expected_url,
                body,
                capability,
                ingress,
            )
            .await
        }
        buzz_auth::NipFiMode::DenyProtected => Err(api_error(
            StatusCode::FORBIDDEN,
            "restricted: protected ingress denied",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
async fn authorize_canonical_bridge(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    method: &str,
    expected_url: &str,
    body: Option<&[u8]>,
    capability: RouteCapability,
    ingress: crate::authorization_runtime::ProtectedIngress,
) -> Result<(), (StatusCode, Json<Value>)> {
    let domain = tenant.community();
    let object =
        crate::protected_ingress::domain_object(domain).map_err(map_protected_bridge_error)?;
    let body_bytes = match body {
        Some(body) => body,
        None => &[],
    };
    let body_digest =
        crate::protected_ingress::fingerprint(b"buzz:nip-fi:bridge-body:v1", &[body_bytes]);
    let request_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:bridge-request:v1",
        &[
            domain.as_uuid().as_bytes(),
            pubkey.as_bytes(),
            method.as_bytes(),
            expected_url.as_bytes(),
            &body_digest,
        ],
    );
    let transport_context_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:bridge-transport:v1",
        &[
            domain.as_uuid().as_bytes(),
            tenant.host().as_bytes(),
            method.as_bytes(),
            expected_url.as_bytes(),
        ],
    );
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        ingress,
        domain,
        capability,
        object,
        ProofTransport::Nip98,
        request_fingerprint,
        transport_context_fingerprint,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion_token = crate::protected_ingress::exact_assertion(
        headers,
        &state.config.corporate_identity.jwt_header,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion =
        crate::protected_ingress::verify_assertion(state, &assertion_token, coordinates)
            .await
            .map_err(map_protected_bridge_error)?;
    let event_json = exact_nip98_authorization_event(headers).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    let proof = buzz_auth::verify_nip98_authorization_proof(
        &event_json,
        expected_url,
        method,
        body,
        &assertion,
        ProofTransport::Nip98,
        request_fingerprint,
        *object.key(),
        transport_context_fingerprint,
    )
    .map_err(|_| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    crate::protected_ingress::authorize_read(state, coordinates, assertion, proof)
        .await
        .map(|_| ())
        .map_err(map_protected_bridge_error)
}

async fn prepare_canonical_bridge_mutation(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    expected_url: &str,
    body: &[u8],
    target: (
        crate::authorization_runtime::ProtectedIngress,
        RouteCapability,
        buzz_db::authorization_admission::AdmissionObject,
    ),
) -> Result<buzz_db::authorization_admission::AdmissionCommitRequest, (StatusCode, Json<Value>)> {
    let domain = tenant.community();
    let (ingress, capability, object) = target;
    let body_digest = crate::protected_ingress::fingerprint(b"buzz:nip-fi:bridge-body:v1", &[body]);
    let request_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:bridge-request:v1",
        &[
            domain.as_uuid().as_bytes(),
            pubkey.as_bytes(),
            b"POST",
            expected_url.as_bytes(),
            &body_digest,
            object.key(),
        ],
    );
    let transport_context_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:bridge-transport:v1",
        &[
            domain.as_uuid().as_bytes(),
            tenant.host().as_bytes(),
            b"POST",
            expected_url.as_bytes(),
        ],
    );
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        ingress,
        domain,
        capability,
        object,
        ProofTransport::Nip98,
        request_fingerprint,
        transport_context_fingerprint,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion_token = crate::protected_ingress::exact_assertion(
        headers,
        &state.config.corporate_identity.jwt_header,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion =
        crate::protected_ingress::verify_assertion(state, &assertion_token, coordinates)
            .await
            .map_err(map_protected_bridge_error)?;
    let event_json = exact_nip98_authorization_event(headers).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    let proof = buzz_auth::verify_nip98_authorization_proof(
        &event_json,
        expected_url,
        "POST",
        Some(body),
        &assertion,
        ProofTransport::Nip98,
        request_fingerprint,
        *object.key(),
        transport_context_fingerprint,
    )
    .map_err(|_| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    crate::protected_ingress::prepare_mutation(state, coordinates, assertion, proof)
        .await
        .map_err(map_protected_bridge_error)
}

fn map_protected_bridge_error(
    error: crate::protected_ingress::ProtectedIngressError,
) -> (StatusCode, Json<Value>) {
    match error {
        crate::protected_ingress::ProtectedIngressError::Denied => {
            api_error(StatusCode::FORBIDDEN, "restricted: authorization denied")
        }
        crate::protected_ingress::ProtectedIngressError::Expired => {
            api_error(StatusCode::UNAUTHORIZED, error.code())
        }
        crate::protected_ingress::ProtectedIngressError::Unavailable => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "restricted: authorization unavailable",
        ),
    }
}

async fn authorize_canonical_moderation_read(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
    path: &str,
    expected_url: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let domain = tenant.community();
    let target_key = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:moderation-read-target:v1",
        &[domain.as_uuid().as_bytes(), path.as_bytes()],
    );
    let object = buzz_db::authorization_admission::AdmissionObject::new(
        buzz_db::authorization_admission::AdmissionObjectKind::ModerationTarget,
        target_key,
    )
    .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "restricted: authorization denied"))?;
    let request_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:moderation-read-request:v1",
        &[
            domain.as_uuid().as_bytes(),
            pubkey.as_bytes(),
            path.as_bytes(),
            expected_url.as_bytes(),
        ],
    );
    let transport_context_fingerprint = crate::protected_ingress::fingerprint(
        b"buzz:nip-fi:moderation-read-transport:v1",
        &[
            domain.as_uuid().as_bytes(),
            tenant.host().as_bytes(),
            expected_url.as_bytes(),
            b"GET",
        ],
    );
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::ModerationRead,
        domain,
        RouteCapability::Moderation,
        object,
        ProofTransport::Nip98,
        request_fingerprint,
        transport_context_fingerprint,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion_token = crate::protected_ingress::exact_assertion(
        headers,
        &state.config.corporate_identity.jwt_header,
    )
    .map_err(map_protected_bridge_error)?;
    let assertion =
        crate::protected_ingress::verify_assertion(state, &assertion_token, coordinates)
            .await
            .map_err(map_protected_bridge_error)?;
    let event_json = exact_nip98_authorization_event(headers).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    let proof = buzz_auth::verify_nip98_authorization_proof(
        &event_json,
        expected_url,
        "GET",
        None,
        &assertion,
        ProofTransport::Nip98,
        request_fingerprint,
        *object.key(),
        transport_context_fingerprint,
    )
    .map_err(|_| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "invalid canonical NIP-98 authorization",
        )
    })?;
    crate::protected_ingress::authorize_read(state, coordinates, assertion, proof)
        .await
        .map(|_| ())
        .map_err(map_protected_bridge_error)
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
            let mut aux_query = buzz_db::EventQuery::for_community(tenant.community());
            aux_query.kinds = Some(hop_kinds.iter().map(|k| *k as i32).collect());
            aux_query.e_tags = Some(std::mem::take(&mut hop_ids));
            aux_query.limit = Some(1000);
            let aux_events = session
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
        bridge_requires_nip98(
            state.config.require_auth_token,
            state.config.corporate_identity.require,
        ),
    )?;
    let pubkey_hex = pubkey.to_hex();

    // Everything after auth — admission, replay, membership, parse, ingest —
    // runs inside the helper.  The thin wrapper here owns the single terminal
    // attribution line so it fires for every outcome, including admission/
    // replay/membership failures that previously returned before any log fired.
    let outcome =
        submit_event_authed(&state, &tenant, &headers, &body, pubkey, event_id_bytes).await;

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
        SubmitOutcome::Rejected { kind, reason, .. } => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                route = "/events",
                status = 400u16,
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

    outcome.into_response()
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
    /// IngestError::Rejected — log kind + truncated reason.
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
) -> SubmitOutcome {
    // Off retains its exact legacy quota/replay behavior. Enforce uses the
    // canonical PostgreSQL receipt and typed result, so no Redis mutation can
    // precede final admission or turn a committed event into a 429 response.
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
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
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_proof =
        match verify_bridge_identity_for_mode(state, tenant, headers, pubkey, auth_tag).await {
            Ok(proof) => proof,
            Err(e) => {
                return SubmitOutcome::Err {
                    status: e.0,
                    response: e,
                };
            }
        };
    let mut nip_oa_owner = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
        match enforce_bridge_membership(state, tenant, &pubkey_bytes, auth_tag).await {
            Ok(owner) => owner,
            Err(e) => {
                return SubmitOutcome::Err {
                    status: e.0,
                    response: e,
                };
            }
        }
    } else {
        None
    };
    let expected_url = nip98_expected_url(&state.config.relay_url, tenant, "/events");
    let kind_u32 = buzz_core::kind::event_kind_u32(&event);
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce
        && !buzz_core::kind::is_moderation_command_kind(kind_u32)
        && !crate::handlers::ingest::canonical_bridge_kind_supported(kind_u32)
    {
        let e = api_error(
            StatusCode::BAD_REQUEST,
            "restricted: event kind requires a dedicated canonical mutation owner",
        );
        return SubmitOutcome::Err {
            status: e.0,
            response: e,
        };
    }
    let canonical_admission = match state.config.nip_fi_mode {
        buzz_auth::NipFiMode::Off => {
            if let Err(e) = finalize_bridge_identity_for_mode(
                state,
                tenant,
                headers,
                pubkey,
                identity_proof,
                "POST",
                &expected_url,
                Some(body),
                RouteCapability::MessagesWrite,
                crate::authorization_runtime::ProtectedIngress::BridgeEvent,
            )
            .await
            {
                return SubmitOutcome::Err {
                    status: e.0,
                    response: e,
                };
            }
            None
        }
        buzz_auth::NipFiMode::Enforce => {
            let target = if buzz_core::kind::is_moderation_command_kind(kind_u32) {
                match crate::handlers::moderation_commands::prepare_moderation_application_effect(
                    tenant, &event,
                ) {
                    Ok(effect) => (
                        crate::authorization_runtime::ProtectedIngress::ModerationWrite,
                        RouteCapability::Moderation,
                        effect.admission_object(),
                    ),
                    Err(message) => {
                        let e = api_error(StatusCode::BAD_REQUEST, &message);
                        return SubmitOutcome::Err {
                            status: e.0,
                            response: e,
                        };
                    }
                }
            } else {
                let object = match buzz_db::authorization_admission::AdmissionObject::event(
                    event.id.to_bytes(),
                ) {
                    Some(object) => object,
                    None => {
                        let e = api_error(StatusCode::BAD_REQUEST, "invalid event identifier");
                        return SubmitOutcome::Err {
                            status: e.0,
                            response: e,
                        };
                    }
                };
                (
                    crate::authorization_runtime::ProtectedIngress::BridgeEvent,
                    RouteCapability::MessagesWrite,
                    object,
                )
            };
            match prepare_canonical_bridge_mutation(
                state,
                tenant,
                headers,
                pubkey,
                &expected_url,
                body,
                target,
            )
            .await
            {
                Ok(request) => Some(request),
                Err(e) => {
                    return SubmitOutcome::Err {
                        status: e.0,
                        response: e,
                    };
                }
            }
        }
        buzz_auth::NipFiMode::DenyProtected => {
            let e = api_error(
                StatusCode::FORBIDDEN,
                "restricted: protected ingress denied",
            );
            return SubmitOutcome::Err {
                status: e.0,
                response: e,
            };
        }
    };
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        nip_oa_owner = match enforce_bridge_membership(state, tenant, &pubkey_bytes, auth_tag).await
        {
            Ok(owner) => owner,
            Err(e) => {
                return SubmitOutcome::Err {
                    status: e.0,
                    response: e,
                };
            }
        };
    }

    let moderation_evidence = buzz_core::kind::is_moderation_command_kind(kind_u32)
        .then(|| {
            exact_nip98_authorization_event(headers).map(|authorization_event| {
                crate::handlers::ingest::ModerationTransportEvidence::Nip98 {
                    authorization_event,
                    body: axum::body::Bytes::copy_from_slice(body),
                }
            })
        })
        .flatten();
    let auth = IngestAuth::Http {
        pubkey,
        scopes: buzz_auth::Scope::all_known(), // Pure Nostr: full scopes, channel access via membership
        auth_method: crate::handlers::ingest::HttpAuthMethod::Nip98,
        moderation_evidence,
    };

    let ingested = match canonical_admission {
        Some(request) => {
            crate::handlers::ingest::ingest_event_with_canonical_admission(
                state, tenant, event, auth, request,
            )
            .await
        }
        None => crate::handlers::ingest::ingest_event(state, tenant, event, auth)
            .await
            .map(|result| {
                (
                    result,
                    crate::handlers::ingest::CanonicalIngestDisposition::Legacy,
                )
            }),
    };
    match ingested {
        Ok((result, disposition)) => {
            if matches!(
                disposition,
                crate::handlers::ingest::CanonicalIngestDisposition::Committed
                    | crate::handlers::ingest::CanonicalIngestDisposition::Legacy
            ) {
                if let Some(owner) = nip_oa_owner {
                    super::relay_members::materialize_nip_oa_owner(state, tenant, &pubkey, &owner)
                        .await;
                }
            }
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
        bridge_requires_nip98(
            state.config.require_auth_token,
            state.config.corporate_identity.require,
        ),
    )?;
    let pubkey_hex = pubkey.to_hex();

    // Admission, replay, membership, and filter execution all run inside the
    // helper.  The single terminal attribution line fires here from the Result
    // so every outcome — including admission/replay/membership failures that
    // previously returned before any log — is attributed.
    let result =
        query_events_authed(&state, &tenant, &headers, &body, pubkey, event_id_bytes).await;
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
    result
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
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_proof =
        verify_bridge_identity_for_mode(state, tenant, headers, pubkey, auth_tag).await?;
    let expected_url = nip98_expected_url(&state.config.relay_url, tenant, "/query");
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        finalize_bridge_identity_for_mode(
            state,
            tenant,
            headers,
            pubkey,
            None,
            "POST",
            &expected_url,
            Some(body),
            RouteCapability::MessagesRead,
            crate::authorization_runtime::ProtectedIngress::BridgeQuery,
        )
        .await?;
    }
    enforce_http_admission(state, tenant, &pubkey).await?;
    check_nip98_replay(state, tenant, event_id_bytes).await?;
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;
    // Two-pass parse: preserve raw JSON for custom extension fields (before_id,
    // depth_limit, feed_types) that nostr::Filter silently drops.
    let raw_filters: Vec<Value> = serde_json::from_slice(body)
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
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
        finalize_bridge_identity_for_mode(
            state,
            tenant,
            headers,
            pubkey,
            identity_proof,
            "POST",
            &expected_url,
            Some(body),
            RouteCapability::MessagesRead,
            crate::authorization_runtime::ProtectedIngress::BridgeQuery,
        )
        .await?;
    }

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

    if let Some(presence_events) = synthesize_presence(state, tenant, &filters).await {
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
            state,
            tenant.community(),
        )
        .await;
        crate::handlers::req::apply_access_scope_to_query(
            &mut query,
            extract_channel_from_filter(filter),
            &accessible_channels,
        );
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

        catchall_queries.push((idx, query));
    }

    // Phase 2 — DB reads, bounded-concurrent, order-preserving (`buffered`).
    // Phase 3 consumes results in original filter order, so response ordering
    // and error semantics match the previous serial loop.
    use futures_util::stream::{self, StreamExt};
    let db = state.db.clone();
    let mut catchall_results = stream::iter(catchall_queries.into_iter().map(|(idx, query)| {
        let db = db.clone();
        async move { (idx, db.query_events_routed("bridge_query", &query).await) }
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
        bridge_requires_nip98(
            state.config.require_auth_token,
            state.config.corporate_identity.require,
        ),
    )?;
    let pubkey_hex = pubkey.to_hex();

    // Admission, replay, membership, and count execution all run inside the
    // helper.  The single terminal attribution line fires here from the Result
    // so every outcome — including admission/replay/membership failures that
    // previously returned before any log — is attributed.
    let result =
        count_events_authed(&state, &tenant, &headers, &body, pubkey, event_id_bytes).await;
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
    result
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
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_proof =
        verify_bridge_identity_for_mode(state, tenant, headers, pubkey, auth_tag).await?;
    let expected_url = nip98_expected_url(&state.config.relay_url, tenant, "/count");
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        finalize_bridge_identity_for_mode(
            state,
            tenant,
            headers,
            pubkey,
            None,
            "POST",
            &expected_url,
            Some(body),
            RouteCapability::MessagesRead,
            crate::authorization_runtime::ProtectedIngress::BridgeCount,
        )
        .await?;
    }
    enforce_http_admission(state, tenant, &pubkey).await?;
    check_nip98_replay(state, tenant, event_id_bytes).await?;
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;
    let filters: Vec<nostr::Filter> = serde_json::from_slice(body)
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
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
        finalize_bridge_identity_for_mode(
            state,
            tenant,
            headers,
            pubkey,
            identity_proof,
            "POST",
            &expected_url,
            Some(body),
            RouteCapability::MessagesRead,
            crate::authorization_runtime::ProtectedIngress::BridgeCount,
        )
        .await?;
    }

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
        if let Some(ch_id) = extract_channel_from_filter(filter) {
            if !accessible_channels.contains(&ch_id) {
                continue; // Skip filters targeting inaccessible channels.
            }
            // Channel is accessible — count with pushability check.
            let mut query = crate::handlers::req::build_event_query_from_filter(
                filter,
                &pubkey_bytes,
                state,
                tenant.community(),
            )
            .await;
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
async fn authorize_moderation_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
) -> Result<TenantContext, (StatusCode, Json<Value>)> {
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

    let path_with_query = match raw_query {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let url = nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let (pubkey, event_id_bytes) = verify_bridge_auth(
        headers,
        "GET",
        &url,
        None,
        bridge_requires_nip98(
            state.config.require_auth_token,
            state.config.corporate_identity.require,
        ),
    )?;
    let pubkey_bytes = pubkey.to_bytes().to_vec();

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_proof =
        verify_bridge_identity_for_mode(state, &tenant, headers, pubkey, auth_tag).await?;
    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        authorize_canonical_moderation_read(state, &tenant, headers, pubkey, path, &url).await?;
    }
    check_nip98_replay(state, &tenant, event_id_bytes).await?;
    // Canonical Enforce admission precedes this distributed legacy quota.
    // Fresh admitted signatures still bound queue work before any queue query.
    enforce_http_admission(state, &tenant, &pubkey).await?;

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
    })?;
    match state.config.nip_fi_mode {
        buzz_auth::NipFiMode::Off => {
            let proof = identity_proof.ok_or_else(|| {
                api_error(StatusCode::UNAUTHORIZED, "identity verification required")
            })?;
            finalize_bridge_corporate_identity(state, &tenant, pubkey, proof).await?;
        }
        buzz_auth::NipFiMode::Enforce => {}
        buzz_auth::NipFiMode::DenyProtected => {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "restricted: protected ingress denied",
            ));
        }
    }

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
    Query(q): Query<ModerationReadQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let tenant = authorize_moderation_read(
        &state,
        &headers,
        "/moderation/reports",
        raw_query.as_deref(),
    )
    .await?;
    let rows = state
        .db
        .list_moderation_reports(
            tenant.community(),
            q.status.as_deref(),
            clamp_limit(q.limit),
        )
        .await
        .map_err(|e| internal_error(&format!("list reports: {e}")))?;
    Ok(Json(Value::Array(rows.iter().map(report_json).collect())))
}

/// `GET /moderation/audit` — the moderation audit log (NIP-98 + mod-authz).
pub async fn moderation_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(q): Query<ModerationReadQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let tenant =
        authorize_moderation_read(&state, &headers, "/moderation/audit", raw_query.as_deref())
            .await?;
    let rows = state
        .db
        .list_moderation_actions(tenant.community(), clamp_limit(q.limit))
        .await
        .map_err(|e| internal_error(&format!("list actions: {e}")))?;
    Ok(Json(Value::Array(rows.iter().map(action_json).collect())))
}

/// `GET /moderation/restricted` — currently banned/timed-out members.
pub async fn moderation_restricted(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let tenant =
        authorize_moderation_read(&state, &headers, "/moderation/restricted", None).await?;
    let rows = state
        .db
        .list_community_restrictions(tenant.community())
        .await
        .map_err(|e| internal_error(&format!("list restrictions: {e}")))?;
    Ok(Json(Value::Array(rows.iter().map(ban_json).collect())))
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
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nostr::{Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag};
    use std::sync::{Arc, Mutex};

    fn test_status_writer(
        send_tx: tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
    ) -> crate::connection::StatusWriter {
        let (status_tx, mut status_rx) =
            tokio::sync::mpsc::channel::<crate::connection::StatusWrite>(8);
        tokio::spawn(async move {
            while let Some(status) = status_rx.recv().await {
                let acknowledgement = send_tx
                    .send(axum::extract::ws::Message::Text(status.text.into()))
                    .await
                    .map(|()| crate::connection::StatusWriteAck {
                        identity: status.identity,
                    })
                    .map_err(|_| ());
                let succeeded = acknowledgement.is_ok();
                let _ = status.flushed.send(acknowledgement);
                if !succeeded {
                    break;
                }
            }
        });
        crate::connection::StatusWriter::new(status_tx)
    }

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
    fn corporate_identity_disables_x_pubkey_bridge_fallback() {
        let keys = Keys::generate();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-pubkey",
            keys.public_key()
                .to_hex()
                .parse()
                .expect("valid pubkey header"),
        );

        assert!(!bridge_requires_nip98(false, false));
        assert!(bridge_requires_nip98(true, false));
        assert!(bridge_requires_nip98(false, true));

        let (status, _) = verify_bridge_auth(
            &headers,
            "POST",
            "https://relay.example/events",
            Some(b"{}"),
            bridge_requires_nip98(false, true),
        )
        .expect_err("corporate identity enrollment must require a signed NIP-98 event");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
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

        let (pubkey, _event_id_bytes) =
            verify_bridge_auth(&headers, "POST", &expected_url, Some(b""), true)
                .expect("matching-host NIP-98 event must verify");
        assert_eq!(
            pubkey,
            keys.public_key(),
            "returned pubkey must be the signer's"
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

        let (pubkey, _event_id_bytes) =
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

        let (pubkey, _event_id_bytes) =
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

        let (pubkey, _event_id_bytes) =
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

    fn test_database_url() -> Option<String> {
        std::env::var("BUZZ_TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("DATABASE_URL").ok())
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
    async fn bridge_handler_test_state() -> Option<Arc<crate::state::AppState>> {
        bridge_handler_test_state_with_corporate_identity(false).await
    }

    async fn bridge_handler_test_state_with_corporate_identity(
        require_corporate_identity: bool,
    ) -> Option<Arc<crate::state::AppState>> {
        bridge_handler_test_state_with_rate_limit(require_corporate_identity, None).await
    }

    async fn bridge_handler_test_state_with_rate_limit(
        require_corporate_identity: bool,
        human_api_calls_per_min: Option<u64>,
    ) -> Option<Arc<crate::state::AppState>> {
        build_bridge_handler_test_state(
            require_corporate_identity,
            human_api_calls_per_min,
            None,
            None,
        )
        .await
        .map(|(state, _)| state)
    }

    async fn build_bridge_handler_test_state(
        require_corporate_identity: bool,
        human_api_calls_per_min: Option<u64>,
        canonical_runtime: Option<(
            crate::authorization_runtime::ProviderFreeRuntimeConfig,
            crate::authorization_runtime::InstalledAuthorizationRuntime,
        )>,
        database_url: Option<&str>,
    ) -> Option<(Arc<crate::state::AppState>, sqlx::PgPool)> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = database_url.map(str::to_owned).or_else(test_database_url)?;
        // Use the real local Redis so enforce_http_admission can pass.
        config.redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        config.relay_url = "wss://bridge-test.local".to_string();
        config.require_auth_token = false;
        config.require_relay_membership = false;
        if let Some((runtime_config, _)) = canonical_runtime.as_ref() {
            config.nip_fi_mode = buzz_auth::NipFiMode::Enforce;
            config.nip_fi = runtime_config.clone();
        }
        if let Some(limit) = human_api_calls_per_min {
            config.auth.rate_limits.human_api_calls_per_min = limit;
        }
        config.corporate_identity.require = require_corporate_identity;
        if require_corporate_identity {
            config.corporate_identity.jwks_uri = "http://127.0.0.1:9/jwks".to_string();
            config.corporate_identity.issuer = "https://idp.example".to_string();
            config.corporate_identity.audience = "buzz-relay".to_string();
        }

        let pool = sqlx::PgPool::connect(&config.database_url).await.ok()?;
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

        let (mut state, _audit_shutdown) = match canonical_runtime {
            Some((_, runtime)) => crate::state::AppState::new_with_authorization_runtime(
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
                runtime,
            ),
            None => crate::state::AppState::new(
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
            ),
        };
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        Some((Arc::new(state), pool))
    }

    struct StaticBridgeJwksLoader(Vec<u8>);

    #[async_trait]
    impl crate::authorization_runtime::JwksDocumentLoader for StaticBridgeJwksLoader {
        async fn load(
            &self,
            _source: &crate::authorization_runtime::JwksSourceConfig,
            _expected_issuer: &str,
            _policy: crate::authorization_runtime::JwksRefreshPolicy,
        ) -> Result<Vec<u8>, crate::authorization_runtime::RuntimeAuthorizationError> {
            Ok(self.0.clone())
        }
    }

    struct BlockingUnavailableStatusEvidence {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl crate::authorization_runtime::CurrentStatusEvidenceSource
        for BlockingUnavailableStatusEvidence
    {
        async fn current(
            &self,
            _request: &buzz_auth::CurrentBindingStatusEvidenceRequest,
        ) -> Result<
            buzz_core::CanonicalCurrentBindingEvidence,
            crate::authorization_runtime::StatusSessionError,
        > {
            self.entered.notify_one();
            self.release.notified().await;
            Err(crate::authorization_runtime::StatusSessionError::EvidenceUnavailable)
        }

        async fn recheck(
            &self,
            _evidence: &buzz_core::CanonicalCurrentBindingEvidence,
        ) -> Result<
            (
                buzz_core::CanonicalCurrentBindingEvidence,
                chrono::DateTime<chrono::Utc>,
            ),
            crate::authorization_runtime::StatusSessionError,
        > {
            Err(crate::authorization_runtime::StatusSessionError::EvidenceUnavailable)
        }
    }

    async fn install_bridge_binding(
        pool: &sqlx::PgPool,
        community: buzz_core::CommunityId,
        actor: nostr::PublicKey,
        issuer: &str,
        subject: &str,
    ) {
        let operation_id = uuid::Uuid::new_v4();
        let history_id = uuid::Uuid::new_v4();
        let binding_id = uuid::Uuid::new_v4();
        let request_fingerprint = [71_u8; 32];
        let actor_bytes = actor.to_bytes();
        let mut transaction = pool.begin().await.expect("begin bridge binding fixture");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,2,$2,transaction_timestamp()-interval '1 second') \
             ON CONFLICT (community_id,policy_revision) DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind([72_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge policy");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0) ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge invalidation domain");
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
             VALUES ($1,32,2097152,16384) ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge audit capacity");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,1,$4,1,$5)",
        )
        .bind(community.as_uuid())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(actor_bytes.as_slice())
        .bind([73_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge binding receipt");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
              actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
              occurred_at,canonical_envelope,envelope_digest) \
             VALUES ($1,$2,1,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(community.as_uuid())
        .bind(uuid::Uuid::new_v4())
        .bind(actor_bytes.as_slice())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(uuid::Uuid::new_v4())
        .bind(uuid::Uuid::new_v4())
        .bind([1_u8].as_slice())
        .bind([74_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge binding event");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,$3,$4,$5,$6,1,1,2,1,$7,$8,$9,$10) RETURNING binding_version",
        )
        .bind(community.as_uuid())
        .bind(binding_id)
        .bind(issuer)
        .bind(subject)
        .bind([75_u8; 32].as_slice())
        .bind(actor_bytes.as_slice())
        .bind([76_u8; 32].as_slice())
        .bind(history_id)
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .fetch_one(&mut *transaction)
        .await
        .expect("insert bridge binding");
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
              successor_binding_version,successor_lifecycle_revision,successor_state, \
              operation_id,request_fingerprint,transition_digest) \
             VALUES ($1,$2,1,1,$3,$4,1,1,$5,$6,$7)",
        )
        .bind(community.as_uuid())
        .bind(history_id)
        .bind(binding_id)
        .bind(binding_version)
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind([77_u8; 32].as_slice())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge binding history");
        sqlx::query(
            "INSERT INTO relay_members (community_id,pubkey,role,added_by) \
             VALUES ($1,$2,'owner',NULL)",
        )
        .bind(community.as_uuid())
        .bind(actor.to_hex())
        .execute(&mut *transaction)
        .await
        .expect("insert bridge moderation owner");
        transaction
            .commit()
            .await
            .expect("commit bridge binding fixture");
    }

    #[tokio::test]
    #[ignore = "requires disposable PostgreSQL and Redis"]
    async fn live_enforce_bridge_event_co_commits_and_replays_without_legacy_redis_mutation() {
        use axum::body::{to_bytes, Body};
        use axum::http::{header, Request};
        use base64::Engine as _;
        use buzz_auth::LocalStatusEvidenceResolver as _;
        use sha2::{Digest as _, Sha256};
        use tower::ServiceExt;

        const KID: &str = "bridge-canonical-test";
        const ISSUER: &str = "https://bridge-issuer.example";
        const AUDIENCE: &str = "buzz-bridge-test";
        let test_db_url = test_database_url()
            .expect("BUZZ_TEST_DATABASE_URL must name a disposable PostgreSQL database");
        let admin_url = test_db_url
            .rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/postgres"))
            .expect("test database URL has a database name");
        let admin = sqlx::PgPool::connect(&admin_url)
            .await
            .expect("connect PostgreSQL admin database");
        let stale_databases: Vec<String> = sqlx::query_scalar(
            "SELECT datname FROM pg_database WHERE datname LIKE 'buzz_reachability_%'",
        )
        .fetch_all(&admin)
        .await
        .expect("list stale disposable bridge databases");
        for stale_database in stale_databases {
            let suffix = stale_database
                .strip_prefix("buzz_reachability_")
                .expect("query prefix is exact");
            if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE \"{stale_database}\" WITH (FORCE)"
            )))
            .execute(&admin)
            .await
            .expect("drop stale disposable bridge database");
        }
        let database_name = format!("buzz_reachability_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{database_name}\""
        )))
        .execute(&admin)
        .await
        .expect("create disposable bridge database");
        let database_url = test_db_url
            .rsplit_once('/')
            .map(|(prefix, _)| format!("{prefix}/{database_name}"))
            .expect("derive disposable bridge database URL");

        let actor = Keys::generate();
        let subject = format!("bridge-user-{}", uuid::Uuid::new_v4());
        let jwk = crate::corporate_identity::canonical_test_support::jwk(0, KID);
        let jwks = serde_json::to_vec(&jsonwebtoken::jwk::JwkSet { keys: vec![jwk] })
            .expect("serialize bridge JWKS");
        let runtime_config = crate::authorization_runtime::ProviderFreeRuntimeConfig::from_optional_json(Some(
            r#"{
                "issuer":"https://bridge-issuer.example",
                "audience":"buzz-bridge-test",
                "subject_claim":"sub",
                "event_author_claim":"event_author",
                "maximum_token_lifetime_seconds":600,
                "jwks":{"jwks_uri":"https://bridge-issuer.example/keys"},
                "lease":{"maximum_seconds":300},
                "policy_revision":1,
                "audit":{"max_events_per_domain":32,"max_bytes_per_domain":2097152,"max_envelope_bytes":16384},
                "client_status_admission":{"max_presentations_per_domain":32,"max_presentations_per_actor":8,"max_presentations_per_peer":8},
                "transport":{"kind":"sealed_test_transport"},
                "enrollment":{"kind":"canonical_admission"},
                "restore":{"kind":"operation_manifest"}
            }"#,
        ))
        .expect("parse bridge runtime config");
        let enforce = runtime_config.enforce().expect("enforce config");
        let verifier = Arc::new(
            crate::authorization_runtime::DynamicVerifier::new(
                enforce.verifier_policy().clone(),
                enforce.issuer().to_owned(),
                enforce.jwks_source().clone(),
                crate::authorization_runtime::JwksRefreshPolicy::new(
                    64 * 1024,
                    std::time::Duration::from_secs(2),
                    std::time::Duration::from_secs(300),
                )
                .expect("bridge refresh policy"),
                Arc::new(StaticBridgeJwksLoader(jwks)),
            )
            .expect("bridge dynamic verifier"),
        );
        let snapshot = verifier
            .refresh(chrono::Utc::now())
            .await
            .expect("publish bridge JWKS");
        let runtime =
            crate::authorization_runtime::InstalledAuthorizationRuntime::for_canonical_assertion_test(
                verifier,
                snapshot,
            );
        let (state, pool) = build_bridge_handler_test_state(
            false,
            None,
            Some((runtime_config, runtime)),
            Some(&database_url),
        )
        .await
        .expect("build live canonical bridge state");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("run bridge migrations");
        let host = format!("bridge-canonical-{}.local", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("create bridge community")
            .id;
        install_bridge_binding(&pool, community, actor.public_key(), ISSUER, &subject).await;

        let event = EventBuilder::new(Kind::TextNote, "canonical bridge event")
            .sign_with_keys(&actor)
            .expect("sign bridge event");
        let body = serde_json::to_vec(&event).expect("serialize bridge event");
        let expected_url = format!("https://{host}/events");
        let payload = hex::encode(Sha256::digest(&body));
        let authorization_event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", expected_url.as_str()]).expect("bridge u tag"),
                Tag::parse(["method", "POST"]).expect("bridge method tag"),
                Tag::parse(["payload", payload.as_str()]).expect("bridge payload tag"),
            ])
            .sign_with_keys(&actor)
            .expect("sign bridge NIP-98 event");
        let authorization_json =
            serde_json::to_string(&authorization_event).expect("serialize NIP-98 event");
        let authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(authorization_json.as_bytes())
        );
        let now = chrono::Utc::now().timestamp();
        let assertion = crate::corporate_identity::canonical_test_support::signed_jwt(
            &serde_json::json!({
                "iss": ISSUER,
                "aud": AUDIENCE,
                "sub": subject,
                "event_author": actor.public_key().to_hex(),
                "iat": now - 1,
                "nbf": now - 1,
                "exp": now + 300,
            }),
            0,
            KID,
        );
        let assertion_header = state.config.corporate_identity.jwt_header.clone();
        let request = || {
            Request::builder()
                .method("POST")
                .uri("/events")
                .header(header::HOST, &host)
                .header(header::AUTHORIZATION, &authorization)
                .header(&assertion_header, &assertion)
                .body(Body::from(body.clone()))
                .expect("build canonical bridge request")
        };
        let first = crate::router::build_router(state.clone())
            .oneshot(request())
            .await
            .expect("first bridge response");
        let first_status = first.status();
        let first_body = to_bytes(first.into_body(), 64 * 1024)
            .await
            .expect("read first bridge response");
        assert_eq!(
            first_status,
            StatusCode::OK,
            "unexpected bridge response: {}",
            String::from_utf8_lossy(&first_body)
        );
        let second = crate::router::build_router(state.clone())
            .oneshot(request())
            .await
            .expect("replayed bridge response");
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = to_bytes(second.into_body(), 64 * 1024)
            .await
            .expect("read replayed bridge response");
        assert_eq!(first_body, second_body);

        let stored: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(event.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count stored bridge event");
        assert_eq!(stored, 1);
        let canonical_results: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_admission_results \
             WHERE community_id=$1 AND object_kind=7 AND object_key=$2 \
               AND application_code=1",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count canonical bridge result");
        assert_eq!(canonical_results, 1);

        let moderation_target = Keys::generate().public_key();
        let moderation_event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_MODERATION_BAN as u16),
            "",
        )
        .tag(Tag::parse(["p", moderation_target.to_hex().as_str()]).expect("moderation target tag"))
        .sign_with_keys(&actor)
        .expect("sign bridge moderation command");
        let moderation_body =
            serde_json::to_vec(&moderation_event).expect("serialize moderation command");
        let moderation_payload = hex::encode(Sha256::digest(&moderation_body));
        let moderation_authorization_event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", expected_url.as_str()]).expect("moderation u tag"),
                Tag::parse(["method", "POST"]).expect("moderation method tag"),
                Tag::parse(["payload", moderation_payload.as_str()])
                    .expect("moderation payload tag"),
            ])
            .sign_with_keys(&actor)
            .expect("sign moderation NIP-98 event");
        let moderation_authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(
                serde_json::to_string(&moderation_authorization_event)
                    .expect("serialize moderation NIP-98 event")
                    .as_bytes()
            )
        );
        let moderation_request = || {
            Request::builder()
                .method("POST")
                .uri("/events")
                .header(header::HOST, &host)
                .header(header::AUTHORIZATION, &moderation_authorization)
                .header(&assertion_header, &assertion)
                .body(Body::from(moderation_body.clone()))
                .expect("build canonical moderation request")
        };
        let first_moderation = crate::router::build_router(state.clone())
            .oneshot(moderation_request())
            .await
            .expect("first bridge moderation response");
        assert_eq!(first_moderation.status(), StatusCode::OK);
        let first_moderation_body = to_bytes(first_moderation.into_body(), 64 * 1024)
            .await
            .expect("read first moderation response");
        let replayed_moderation = crate::router::build_router(state.clone())
            .oneshot(moderation_request())
            .await
            .expect("replayed bridge moderation response");
        assert_eq!(replayed_moderation.status(), StatusCode::OK);
        let replayed_moderation_body = to_bytes(replayed_moderation.into_body(), 64 * 1024)
            .await
            .expect("read replayed moderation response");
        assert_eq!(first_moderation_body, replayed_moderation_body);
        let moderation_actions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM moderation_actions WHERE community_id=$1 \
             AND target_pubkey=$2 AND action='ban'",
        )
        .bind(community.as_uuid())
        .bind(moderation_target.to_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count co-committed moderation actions");
        assert_eq!(moderation_actions, 1);
        let moderation_results: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_admission_results \
             WHERE community_id=$1 AND object_kind=5",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count canonical moderation result");
        assert_eq!(moderation_results, 1);

        let invite_body = serde_json::to_vec(&serde_json::json!({
            "ttl_secs": 3600,
            "max_uses": 2,
        }))
        .expect("serialize invite mint request");
        let invite_url = format!("https://{host}/api/invites");
        let invite_payload = hex::encode(Sha256::digest(&invite_body));
        let invite_authorization_event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", invite_url.as_str()]).expect("invite u tag"),
                Tag::parse(["method", "POST"]).expect("invite method tag"),
                Tag::parse(["payload", invite_payload.as_str()]).expect("invite payload tag"),
            ])
            .sign_with_keys(&actor)
            .expect("sign invite NIP-98 event");
        let invite_authorization = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(
                serde_json::to_string(&invite_authorization_event)
                    .expect("serialize invite NIP-98 event")
                    .as_bytes()
            )
        );
        let invite_request = || {
            Request::builder()
                .method("POST")
                .uri("/api/invites")
                .header(header::HOST, &host)
                .header(header::AUTHORIZATION, &invite_authorization)
                .header(&assertion_header, &assertion)
                .body(Body::from(invite_body.clone()))
                .expect("build canonical invite request")
        };
        let first_invite = crate::router::build_router(state.clone())
            .oneshot(invite_request())
            .await
            .expect("first invite mint response");
        assert_eq!(first_invite.status(), StatusCode::OK);
        let first_invite_body = to_bytes(first_invite.into_body(), 64 * 1024)
            .await
            .expect("read first invite response");
        let replayed_invite = crate::router::build_router(state.clone())
            .oneshot(invite_request())
            .await
            .expect("replayed invite mint response");
        assert_eq!(replayed_invite.status(), StatusCode::OK);
        let replayed_invite_body = to_bytes(replayed_invite.into_body(), 64 * 1024)
            .await
            .expect("read replayed invite response");
        assert_eq!(first_invite_body, replayed_invite_body);
        let invite_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM relay_invites WHERE community_id=$1 AND created_by=$2",
        )
        .bind(community.as_uuid())
        .bind(actor.public_key().to_hex())
        .fetch_one(&pool)
        .await
        .expect("count co-committed invite rows");
        assert_eq!(invite_rows, 1);
        let invite_results: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_admission_results \
             WHERE community_id=$1 AND object_kind=9 AND application_code=1",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count canonical invite results");
        assert_eq!(invite_results, 1);

        // Drive the real AUTH owner with an opaque peer and opted-in status
        // scope. This proves the transport peer survives canonical AUTH into
        // the frozen connection context and that live Enforce AUTH activates
        // status rather than merely leaving the helper reachable in tests.
        let challenge = format!("bridge-auth-{}", uuid::Uuid::new_v4());
        let relay_url = format!("wss://{host}");
        let auth_event = EventBuilder::auth(
            &challenge,
            nostr::RelayUrl::parse(&relay_url).expect("parse AUTH relay URL"),
        )
        .tag(
            Tag::parse([
                buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG,
                "1",
                uuid::Uuid::new_v4().to_string().as_str(),
                state.relay_keypair.public_key().to_hex().as_str(),
            ])
            .expect("build binding-status scope"),
        )
        .sign_with_keys(&actor)
        .expect("sign canonical AUTH event");
        let authenticated_peer = buzz_auth::AuthenticatedClientPeer::for_test([0x92; 32]);
        let evidence = buzz_auth::SealedTransportEvidence::for_test(
            community,
            assertion.clone(),
            b"GET",
            host.as_bytes(),
            b"/",
            [0; 32],
            buzz_auth::ProofTransport::Nip42,
            chrono::Utc::now() + chrono::Duration::seconds(300),
            authenticated_peer,
        );
        let (send_tx, mut send_rx) = tokio::sync::mpsc::channel(16);
        let (ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::channel(8);
        let status_writer = test_status_writer(send_tx.clone());
        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            tenant: TenantContext::resolved(community, &host),
            corporate_identity_jwt: None,
            canonical_transport_evidence: tokio::sync::Mutex::new(Some(evidence)),
            canonical_authorization: tokio::sync::RwLock::new(None),
            auth_state: tokio::sync::RwLock::new(crate::connection::AuthState::Pending {
                challenge: challenge.clone(),
            }),
            status_scope: tokio::sync::RwLock::new(None),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx,
            status_writer,
            ctrl_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
        });
        crate::handlers::auth::handle_auth(auth_event, Arc::clone(&conn), Arc::clone(&state)).await;
        let mut frames = Vec::new();
        while let Ok(frame) = send_rx.try_recv() {
            frames.push(format!("{frame:?}"));
        }
        let mut control_frames = Vec::new();
        while let Ok(frame) = ctrl_rx.try_recv() {
            control_frames.push(format!("{frame:?}"));
        }
        let status_results: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_admission_results \
             WHERE community_id=$1 AND object_kind=8",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count canonical AUTH status results");
        {
            let auth = conn.auth_state.read().await;
            let crate::connection::AuthState::Authenticated(context) = &*auth else {
                panic!(
                    "canonical AUTH did not authenticate the connection: status_results={status_results} data={frames:?} control={control_frames:?}"
                );
            };
            assert_eq!(
                context.authenticated_client_peer(),
                Some(&authenticated_peer)
            );
            assert_eq!(context.authorization().pubkey, actor.public_key());
        }
        assert_eq!(status_results, 1);
        let durable_status: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM client_status_delivery_capacity \
                    WHERE community_id=$1 AND healthy), \
                 (SELECT count(*) FROM client_status_transitions \
                    WHERE community_id=$1 AND delivery_kind=1 AND status_revision=1), \
                 (SELECT count(*) FROM client_status_delivery_outbox outbox \
                    JOIN client_status_transitions transition \
                      ON transition.community_id=outbox.community_id \
                     AND transition.transition_id=outbox.transition_id \
                    WHERE outbox.community_id=$1 AND outbox.delivery_state=2 \
                      AND transition.delivery_kind=1)",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read physically acknowledged current-binding status");
        assert_eq!(durable_status, (1, 1, 1));

        // The live WS EVENT handler must prepare a fresh operation-bound
        // admission from the canonical session. An exact replay returns the
        // immutable typed result without a second event write or dispatch.
        let websocket_event = EventBuilder::new(
            Kind::TextNote,
            format!("canonical websocket event {}", uuid::Uuid::new_v4()),
        )
        .sign_with_keys(&actor)
        .expect("sign canonical WebSocket event");
        crate::handlers::event::handle_event(
            websocket_event.clone(),
            Arc::clone(&conn),
            Arc::clone(&state),
        )
        .await;
        crate::handlers::event::handle_event(
            websocket_event.clone(),
            Arc::clone(&conn),
            Arc::clone(&state),
        )
        .await;
        let websocket_event_counts: (i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM events WHERE community_id=$1 AND id=$2), \
                 (SELECT count(*) FROM authorization_admission_results \
                    WHERE community_id=$1 AND object_kind=7 AND object_key=$2)",
        )
        .bind(community.as_uuid())
        .bind(websocket_event.id.as_bytes())
        .fetch_one(&pool)
        .await
        .expect("read canonical WebSocket event result");
        assert_eq!(websocket_event_counts, (1, 1));
        let status_evidence_request =
            buzz_auth::CurrentBindingStatusEvidenceRequest::new(community, actor.public_key())
                .expect("build current-status evidence request");
        let first_status_evidence = state
            .db
            .current_status_evidence(&status_evidence_request)
            .await
            .expect("resolve first connection status evidence");
        assert!(conn.canonical_authorization.read().await.is_some());
        let bootstrap_index = frames
            .iter()
            .position(|frame| {
                frame.contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_BOOTSTRAP_SUB_ID)
            })
            .expect("real AUTH must deliver the status bootstrap");
        let current_index = frames
            .iter()
            .position(|frame| {
                frame.contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID)
            })
            .expect("real AUTH must deliver authoritative current status");
        let acknowledgement_index = frames
            .iter()
            .position(|frame| frame.contains("true"))
            .expect("real AUTH must acknowledge after activation");
        assert!(
            bootstrap_index < current_index && current_index < acknowledgement_index,
            "AUTH ordering must be bootstrap, authoritative current, then success: {frames:?}"
        );
        assert!(control_frames.is_empty());
        assert!(conn.clear_client_binding_status_task().await);
        conn.cancel.cancel();
        let prior_connection = match Arc::try_unwrap(conn) {
            Ok(connection) => connection,
            Err(_) => panic!("first status connection remained shared after task shutdown"),
        };

        // A second opted-in connection for the same actor must bind its own
        // socket target while advancing the canonical object epoch. This
        // catches accidental equality between the connection-local evidence
        // epoch and the independently monotonic admission epoch.
        let reconnect_challenge = format!("bridge-auth-reconnect-{}", uuid::Uuid::new_v4());
        let reconnect_event = EventBuilder::auth(
            &reconnect_challenge,
            nostr::RelayUrl::parse(&relay_url).expect("parse reconnect relay URL"),
        )
        .tag(
            Tag::parse([
                buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG,
                "1",
                uuid::Uuid::new_v4().to_string().as_str(),
                state.relay_keypair.public_key().to_hex().as_str(),
            ])
            .expect("build reconnect binding-status scope"),
        )
        .sign_with_keys(&actor)
        .expect("sign reconnect AUTH event");
        let reconnect_peer = buzz_auth::AuthenticatedClientPeer::for_test([0x93; 32]);
        let reconnect_evidence = buzz_auth::SealedTransportEvidence::for_test_with_nonce(
            community,
            assertion.clone(),
            b"GET",
            host.as_bytes(),
            b"/",
            [0; 32],
            buzz_auth::ProofTransport::Nip42,
            chrono::Utc::now() + chrono::Duration::seconds(300),
            reconnect_peer,
            [0x92; 32],
        );
        let (reconnect_send_tx, mut reconnect_send_rx) = tokio::sync::mpsc::channel(16);
        let (reconnect_ctrl_tx, mut reconnect_ctrl_rx) = tokio::sync::mpsc::channel(8);
        let reconnect_status_writer = test_status_writer(reconnect_send_tx.clone());
        let reconnect = Arc::new(crate::connection::ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            canonical_transport_evidence: tokio::sync::Mutex::new(Some(reconnect_evidence)),
            canonical_authorization: tokio::sync::RwLock::new(None),
            auth_state: tokio::sync::RwLock::new(crate::connection::AuthState::Pending {
                challenge: reconnect_challenge,
            }),
            status_scope: tokio::sync::RwLock::new(None),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx: reconnect_send_tx,
            status_writer: reconnect_status_writer,
            ctrl_tx: reconnect_ctrl_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            ..prior_connection
        });
        crate::handlers::auth::handle_auth(
            reconnect_event,
            Arc::clone(&reconnect),
            Arc::clone(&state),
        )
        .await;
        let mut reconnect_frames = Vec::new();
        while let Ok(frame) = reconnect_send_rx.try_recv() {
            reconnect_frames.push(format!("{frame:?}"));
        }
        let mut reconnect_control_frames = Vec::new();
        while let Ok(frame) = reconnect_ctrl_rx.try_recv() {
            reconnect_control_frames.push(format!("{frame:?}"));
        }
        {
            let auth = reconnect.auth_state.read().await;
            let crate::connection::AuthState::Authenticated(context) = &*auth else {
                panic!(
                    "reconnect did not authenticate: data={reconnect_frames:?} control={reconnect_control_frames:?}"
                );
            };
            assert_eq!(context.authenticated_client_peer(), Some(&reconnect_peer));
        }
        assert!(reconnect.canonical_authorization.read().await.is_some());
        assert!(reconnect_frames.iter().any(|frame| frame.contains("true")));
        assert!(reconnect_frames.iter().any(|frame| frame
            .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_BOOTSTRAP_SUB_ID)));
        assert!(reconnect_frames.iter().any(|frame| frame
            .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID)));
        assert!(reconnect_control_frames.is_empty());
        let reconnect_status: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM authorization_admission_results \
                    WHERE community_id=$1 AND object_kind=8), \
                 (SELECT count(*) FROM client_status_transitions \
                    WHERE community_id=$1 AND delivery_kind=1 AND status_revision=1), \
                 (SELECT count(*) FROM client_status_delivery_outbox outbox \
                    JOIN client_status_transitions transition \
                      ON transition.community_id=outbox.community_id \
                     AND transition.transition_id=outbox.transition_id \
                    WHERE outbox.community_id=$1 AND outbox.delivery_state=2 \
                      AND transition.delivery_kind=1)",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read reconnect status completion");
        assert_eq!(reconnect_status, (2, 2, 2));
        let reconnect_status_evidence = state
            .db
            .current_status_evidence(&status_evidence_request)
            .await
            .expect("resolve reconnect status evidence");
        assert_eq!(
            (
                first_status_evidence.binding_id(),
                first_status_evidence.binding_version(),
                first_status_evidence.policy_revision(),
                first_status_evidence.invalidation_generation(),
                first_status_evidence.authority_epoch(),
                first_status_evidence.fence(),
            ),
            (
                reconnect_status_evidence.binding_id(),
                reconnect_status_evidence.binding_version(),
                reconnect_status_evidence.policy_revision(),
                reconnect_status_evidence.invalidation_generation(),
                reconnect_status_evidence.authority_epoch(),
                reconnect_status_evidence.fence(),
            ),
            "a later connection must preserve the stable connection-status evidence",
        );
        assert!(reconnect.clear_client_binding_status_task().await);
        reconnect.cancel.cancel();
        let prior_reconnect = match Arc::try_unwrap(reconnect) {
            Ok(connection) => connection,
            Err(_) => panic!("reconnect remained shared after task shutdown"),
        };

        // The connection-status scope remains an opt-in. A canonical Enforce
        // AUTH without that tag must authenticate normally and must not create
        // a bootstrap or status delivery.
        let unscoped_challenge = format!("bridge-auth-unscoped-{}", uuid::Uuid::new_v4());
        let unscoped_event = EventBuilder::auth(
            &unscoped_challenge,
            nostr::RelayUrl::parse(&relay_url).expect("parse unscoped relay URL"),
        )
        .sign_with_keys(&actor)
        .expect("sign unscoped AUTH event");
        let unscoped_peer = buzz_auth::AuthenticatedClientPeer::for_test([0x94; 32]);
        let unscoped_evidence = buzz_auth::SealedTransportEvidence::for_test_with_nonce(
            community,
            assertion.clone(),
            b"GET",
            host.as_bytes(),
            b"/",
            [0; 32],
            buzz_auth::ProofTransport::Nip42,
            chrono::Utc::now() + chrono::Duration::seconds(300),
            unscoped_peer,
            [0x93; 32],
        );
        let (unscoped_send_tx, mut unscoped_send_rx) = tokio::sync::mpsc::channel(8);
        let (unscoped_ctrl_tx, mut unscoped_ctrl_rx) = tokio::sync::mpsc::channel(8);
        let unscoped_status_writer = test_status_writer(unscoped_send_tx.clone());
        let unscoped = Arc::new(crate::connection::ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            canonical_transport_evidence: tokio::sync::Mutex::new(Some(unscoped_evidence)),
            canonical_authorization: tokio::sync::RwLock::new(None),
            auth_state: tokio::sync::RwLock::new(crate::connection::AuthState::Pending {
                challenge: unscoped_challenge,
            }),
            status_scope: tokio::sync::RwLock::new(None),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx: unscoped_send_tx,
            status_writer: unscoped_status_writer,
            ctrl_tx: unscoped_ctrl_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            ..prior_reconnect
        });
        crate::handlers::auth::handle_auth(
            unscoped_event,
            Arc::clone(&unscoped),
            Arc::clone(&state),
        )
        .await;
        let mut unscoped_frames = Vec::new();
        while let Ok(frame) = unscoped_send_rx.try_recv() {
            unscoped_frames.push(format!("{frame:?}"));
        }
        assert!(matches!(
            &*unscoped.auth_state.read().await,
            crate::connection::AuthState::Authenticated(context)
                if context.authenticated_client_peer() == Some(&unscoped_peer)
        ));
        assert!(unscoped.canonical_authorization.read().await.is_some());
        assert!(unscoped_frames.iter().any(|frame| frame.contains("true")));
        assert!(unscoped_frames.iter().all(|frame| !frame
            .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_BOOTSTRAP_SUB_ID)));
        assert!(unscoped_frames.iter().all(|frame| !frame
            .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID)));
        assert!(unscoped_ctrl_rx.try_recv().is_err());
        let unscoped_counts: (i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM authorization_admission_results \
                    WHERE community_id=$1 AND object_kind=8), \
                 (SELECT count(*) FROM client_status_transitions \
                    WHERE community_id=$1 AND delivery_kind=1)",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read unscoped canonical AUTH state");
        assert_eq!(unscoped_counts, (3, 2));
        let unscoped_status_evidence = state
            .db
            .current_status_evidence(&status_evidence_request)
            .await
            .expect("resolve status evidence after unscoped AUTH");
        assert_eq!(
            (
                first_status_evidence.binding_id(),
                first_status_evidence.binding_version(),
                first_status_evidence.policy_revision(),
                first_status_evidence.invalidation_generation(),
                first_status_evidence.authority_epoch(),
                first_status_evidence.fence(),
            ),
            (
                unscoped_status_evidence.binding_id(),
                unscoped_status_evidence.binding_version(),
                unscoped_status_evidence.policy_revision(),
                unscoped_status_evidence.invalidation_generation(),
                unscoped_status_evidence.authority_epoch(),
                unscoped_status_evidence.fence(),
            ),
            "unscoped AUTH must preserve the stable connection-status evidence",
        );
        unscoped.cancel.cancel();
        let prior_unscoped = match Arc::try_unwrap(unscoped) {
            Ok(connection) => connection,
            Err(_) => panic!("unscoped connection remained shared after AUTH"),
        };

        // Keep canonical admission live but fail the evidence source that the
        // real AUTH owner awaits before it may acknowledge success.
        let unavailable_status = Arc::new(BlockingUnavailableStatusEvidence {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        *state.client_status_evidence_override.write().await = Some(unavailable_status.clone());
        let failed_actor = Keys::generate();
        let failed_subject = format!("bridge-status-failure-{}", uuid::Uuid::new_v4());
        let failed_host = format!(
            "bridge-status-failure-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        let failed_community = state
            .db
            .ensure_configured_community(&failed_host)
            .await
            .expect("create failed-status community")
            .id;
        install_bridge_binding(
            &pool,
            failed_community,
            failed_actor.public_key(),
            ISSUER,
            &failed_subject,
        )
        .await;
        let failed_challenge = format!("bridge-auth-failure-{}", uuid::Uuid::new_v4());
        let failed_relay_url = format!("wss://{failed_host}");
        let failed_auth_event = EventBuilder::auth(
            &failed_challenge,
            nostr::RelayUrl::parse(&failed_relay_url).expect("parse failed AUTH relay URL"),
        )
        .tag(
            Tag::parse([
                buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG,
                "1",
                uuid::Uuid::new_v4().to_string().as_str(),
                state.relay_keypair.public_key().to_hex().as_str(),
            ])
            .expect("build failed binding-status scope"),
        )
        .sign_with_keys(&failed_actor)
        .expect("sign failing canonical AUTH event");
        let failed_now = chrono::Utc::now().timestamp();
        let failed_assertion = crate::corporate_identity::canonical_test_support::signed_jwt(
            &serde_json::json!({
                "iss": ISSUER,
                "aud": AUDIENCE,
                    "sub": failed_subject,
                    "event_author": failed_actor.public_key().to_hex(),
                "iat": failed_now - 1,
                "nbf": failed_now - 1,
                "exp": failed_now + 300,
                "nonce": uuid::Uuid::new_v4().to_string(),
            }),
            0,
            KID,
        );
        let failed_evidence = buzz_auth::SealedTransportEvidence::for_test(
            failed_community,
            failed_assertion,
            b"GET",
            failed_host.as_bytes(),
            b"/",
            [0; 32],
            buzz_auth::ProofTransport::Nip42,
            chrono::Utc::now() + chrono::Duration::seconds(300),
            authenticated_peer,
        );
        let (failed_send_tx, mut failed_send_rx) = tokio::sync::mpsc::channel(16);
        let (failed_ctrl_tx, mut failed_ctrl_rx) = tokio::sync::mpsc::channel(8);
        let failed_status_writer = test_status_writer(failed_send_tx.clone());
        let failed_conn = Arc::new(crate::connection::ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            tenant: TenantContext::resolved(failed_community, &failed_host),
            corporate_identity_jwt: None,
            canonical_transport_evidence: tokio::sync::Mutex::new(Some(failed_evidence)),
            canonical_authorization: tokio::sync::RwLock::new(None),
            auth_state: tokio::sync::RwLock::new(crate::connection::AuthState::Pending {
                challenge: failed_challenge,
            }),
            status_scope: tokio::sync::RwLock::new(None),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx: failed_send_tx,
            status_writer: failed_status_writer,
            ctrl_tx: failed_ctrl_tx,
            cancel: tokio_util::sync::CancellationToken::new(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
        });
        let failed_auth_task = tokio::spawn(crate::handlers::auth::handle_auth(
            failed_auth_event,
            Arc::clone(&failed_conn),
            Arc::clone(&state),
        ));
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            unavailable_status.entered.notified(),
        )
        .await
        .expect("failed evidence source must be reached");
        assert!(matches!(
            &*failed_conn.auth_state.read().await,
            crate::connection::AuthState::Pending { .. }
        ));
        assert!(failed_conn.canonical_authorization.read().await.is_none());
        unavailable_status.release.notify_one();
        failed_auth_task.await.expect("join failed AUTH task");
        let mut failed_frames = Vec::new();
        while let Ok(frame) = failed_send_rx.try_recv() {
            failed_frames.push(format!("{frame:?}"));
        }
        let mut failed_control_frames = Vec::new();
        while let Ok(frame) = failed_ctrl_rx.try_recv() {
            failed_control_frames.push(format!("{frame:?}"));
        }
        assert!(
            failed_frames.iter().all(|frame| !frame.contains("true")),
            "failed PG status evidence must precede and suppress AUTH success: {failed_frames:?}"
        );
        assert!(
            failed_frames.iter().any(|frame| frame.contains(
                buzz_core::client_binding_bootstrap::CLIENT_BINDING_BOOTSTRAP_SUB_ID
            )),
            "the negative case must reach live status activation before PG evidence fails: {failed_frames:?}"
        );
        assert!(
            failed_frames
                .iter()
                .chain(failed_control_frames.iter())
                .any(|frame| frame.contains("false")),
            "failed activation must send a negative AUTH result: data={failed_frames:?} control={failed_control_frames:?}"
        );
        assert!(matches!(
            &*failed_conn.auth_state.read().await,
            crate::connection::AuthState::Failed
        ));
        assert!(failed_conn.canonical_authorization.read().await.is_none());
        assert!(failed_conn.cancel.is_cancelled());
        assert!(!failed_conn.clear_client_binding_status_task().await);
        let prior_failed = match Arc::try_unwrap(failed_conn) {
            Ok(connection) => connection,
            Err(_) => panic!("failed connection remained shared after AUTH"),
        };
        *state.client_status_evidence_override.write().await = None;

        // Two independent AUTH events for the same actor must not compete for
        // one actor-global protected-object epoch. Start two opted-in live
        // status owners in the same scheduler turn and require both physical
        // status activations plus two authenticated sessions.
        let concurrent_connection = |marker: u8, prior: crate::connection::ConnectionState| {
            let challenge = format!("bridge-auth-concurrent-{marker}-{}", uuid::Uuid::new_v4());
            let relay_signer = state.relay_keypair.public_key().to_hex();
            let connection_epoch = uuid::Uuid::new_v4().to_string();
            let auth_event = EventBuilder::auth(
                &challenge,
                nostr::RelayUrl::parse(&relay_url).expect("parse concurrent relay URL"),
            )
            .tag(
                Tag::parse([
                    buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG,
                    "1",
                    connection_epoch.as_str(),
                    relay_signer.as_str(),
                ])
                .expect("build concurrent binding-status scope"),
            )
            .sign_with_keys(&actor)
            .expect("sign concurrent AUTH event");
            let peer = buzz_auth::AuthenticatedClientPeer::for_test([marker; 32]);
            let evidence = buzz_auth::SealedTransportEvidence::for_test_with_nonce(
                community,
                assertion.clone(),
                b"GET",
                host.as_bytes(),
                b"/",
                [0; 32],
                buzz_auth::ProofTransport::Nip42,
                chrono::Utc::now() + chrono::Duration::seconds(300),
                peer,
                [marker.wrapping_add(1); 32],
            );
            let (send_tx, send_rx) = tokio::sync::mpsc::channel(16);
            let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::channel(8);
            let connection = Arc::new(crate::connection::ConnectionState {
                conn_id: uuid::Uuid::new_v4(),
                tenant: TenantContext::resolved(community, &host),
                canonical_transport_evidence: tokio::sync::Mutex::new(Some(evidence)),
                canonical_authorization: tokio::sync::RwLock::new(None),
                auth_state: tokio::sync::RwLock::new(crate::connection::AuthState::Pending {
                    challenge,
                }),
                status_scope: tokio::sync::RwLock::new(None),
                subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                status_writer: test_status_writer(send_tx.clone()),
                send_tx,
                ctrl_tx,
                cancel: tokio_util::sync::CancellationToken::new(),
                backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                grace_limit: 3,
                ..prior
            });
            (auth_event, peer, connection, send_rx, ctrl_rx)
        };
        let (
            concurrent_event_a,
            concurrent_peer_a,
            concurrent_a,
            mut concurrent_send_rx_a,
            mut concurrent_ctrl_rx_a,
        ) = concurrent_connection(0xa1, prior_unscoped);
        let (
            concurrent_event_b,
            concurrent_peer_b,
            concurrent_b,
            mut concurrent_send_rx_b,
            mut concurrent_ctrl_rx_b,
        ) = concurrent_connection(0xb1, prior_failed);
        tokio::join!(
            crate::handlers::auth::handle_auth(
                concurrent_event_a,
                Arc::clone(&concurrent_a),
                Arc::clone(&state),
            ),
            crate::handlers::auth::handle_auth(
                concurrent_event_b,
                Arc::clone(&concurrent_b),
                Arc::clone(&state),
            ),
        );
        assert!(matches!(
            &*concurrent_a.auth_state.read().await,
            crate::connection::AuthState::Authenticated(context)
                if context.authenticated_client_peer() == Some(&concurrent_peer_a)
        ));
        assert!(matches!(
            &*concurrent_b.auth_state.read().await,
            crate::connection::AuthState::Authenticated(context)
                if context.authenticated_client_peer() == Some(&concurrent_peer_b)
        ));
        assert!(concurrent_a.canonical_authorization.read().await.is_some());
        assert!(concurrent_b.canonical_authorization.read().await.is_some());
        for frames in [&mut concurrent_send_rx_a, &mut concurrent_send_rx_b] {
            let mut delivered = Vec::new();
            while let Ok(frame) = frames.try_recv() {
                delivered.push(format!("{frame:?}"));
            }
            assert!(delivered.iter().any(|frame| frame.contains("true")));
            assert!(delivered.iter().any(|frame| frame
                .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_BOOTSTRAP_SUB_ID)));
            assert!(delivered.iter().any(|frame| frame
                .contains(buzz_core::client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID)));
        }
        assert!(concurrent_ctrl_rx_a.try_recv().is_err());
        assert!(concurrent_ctrl_rx_b.try_recv().is_err());
        let concurrent_results: (i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM authorization_admission_results \
                    WHERE community_id=$1 AND object_kind=8), \
                 (SELECT count(*) FROM client_status_transitions \
                    WHERE community_id=$1 AND delivery_kind=1 AND status_revision=1)",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count concurrent canonical AUTH and status results");
        assert_eq!(concurrent_results, (5, 4));
        concurrent_a.cancel.cancel();
        concurrent_b.cancel.cancel();

        let tenant = TenantContext::resolved(community, &host);
        let rate_key = buzz_auth::rate_limit::rate_limit_key(
            &tenant,
            &actor.public_key(),
            &buzz_auth::LimitType::ApiCalls,
        );
        let replay_key = buzz_auth::nip98_replay_key(&tenant, &authorization_event.id);
        let moderation_replay_key =
            buzz_auth::nip98_replay_key(&tenant, &moderation_authorization_event.id);
        let invite_replay_key =
            buzz_auth::nip98_replay_key(&tenant, &invite_authorization_event.id);
        let mut redis = state
            .redis_pool
            .get()
            .await
            .expect("borrow live bridge Redis");
        let legacy_keys: i64 = redis::cmd("EXISTS")
            .arg(&[
                rate_key,
                replay_key,
                moderation_replay_key,
                invite_replay_key,
            ])
            .query_async(&mut *redis)
            .await
            .expect("read legacy bridge Redis keys");
        assert_eq!(legacy_keys, 0);

        drop(redis);
        drop(state);
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE \"{database_name}\" WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop exact disposable bridge database");
    }

    #[test]
    #[ignore = "requires Postgres"]
    fn moderation_reads_require_corporate_identity_after_nip98_proof() {
        use axum::body::Body;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");
        let state = rt
            .block_on(bridge_handler_test_state_with_corporate_identity(true))
            .expect("local Postgres not reachable");
        let host = format!("bridge-moderation-{}.local", uuid::Uuid::new_v4().simple());
        rt.block_on(state.db.ensure_configured_community(&host))
            .expect("ensure community");

        let keys = Keys::generate();
        let signed_url = format!("https://{host}/moderation/reports");
        let event_json = build_nip98_event_json(&keys, &signed_url, "GET");
        let auth = nip98_auth_headers(&event_json)
            .get(header::AUTHORIZATION)
            .cloned()
            .expect("authorization header");
        let response = rt
            .block_on(
                crate::router::build_router(state).oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/moderation/reports")
                        .header(header::HOST, host)
                        .header(header::AUTHORIZATION, auth)
                        .body(Body::empty())
                        .expect("build request"),
                ),
            )
            .expect("router oneshot");

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "a valid NIP-98 moderator request without an identity JWT must fail before role authorization"
        );
    }

    #[test]
    fn repeated_moderation_reads_are_bounded_before_identity_binding_work() {
        use axum::body::Body;
        use axum::http::{header, Request};
        use tower::ServiceExt;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");
        let Some(state) = rt.block_on(bridge_handler_test_state_with_rate_limit(false, Some(1)))
        else {
            return;
        };
        let host = format!(
            "bridge-moderation-limit-{}.local",
            uuid::Uuid::new_v4().simple()
        );
        let community = rt
            .block_on(state.db.ensure_configured_community(&host))
            .expect("ensure moderation limit community")
            .id;
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_bytes();
        assert!(rt
            .block_on(
                state
                    .db
                    .get_active_identity_binding_by_pubkey(community, &pubkey)
            )
            .expect("read initial identity binding")
            .is_none());

        let signed_url = format!("https://{host}/moderation/reports");
        let event_json = build_nip98_event_json(&keys, &signed_url, "GET");
        let auth = nip98_auth_headers(&event_json)
            .get(header::AUTHORIZATION)
            .cloned()
            .expect("authorization header");
        let request = || {
            Request::builder()
                .method("GET")
                .uri("/moderation/reports")
                .header(header::HOST, &host)
                .header(header::AUTHORIZATION, auth.clone())
                .body(Body::empty())
                .expect("build moderation request")
        };

        let first = rt
            .block_on(crate::router::build_router(state.clone()).oneshot(request()))
            .expect("first moderation response");
        assert_eq!(
            first.status(),
            StatusCode::FORBIDDEN,
            "the first request must pass admission and reach moderator authorization"
        );

        let second = rt
            .block_on(crate::router::build_router(state.clone()).oneshot(request()))
            .expect("second moderation response");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            second
                .headers()
                .get("x-buzz-error-code")
                .and_then(|value| value.to_str().ok()),
            Some("rate_limited")
        );
        assert!(
            rt.block_on(
                state
                    .db
                    .get_active_identity_binding_by_pubkey(community, &pubkey)
            )
            .expect("read final identity binding")
            .is_none(),
            "bounded moderation reads must not enroll or persist an identity binding"
        );
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
}
