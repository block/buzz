//! Transcription session endpoint — proxies OpenAI Realtime API client-secret minting.
//!
//! When `BUZZ_OPENAI_API_KEY` is configured, the relay can mint ephemeral client
//! secrets for the OpenAI Realtime API. The desktop app uses these to establish a
//! WebRTC connection for real-time speech-to-text dictation.
//!
//! Both endpoints require NIP-98 auth (same as `/events`, `/query`, `/count`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Serialize;
use serde_json::Value;

use buzz_core::CommunityId;

use crate::state::AppState;

use super::api_error;

const OPENAI_REALTIME_CLIENT_SECRETS_URL: &str =
    "https://api.openai.com/v1/realtime/client_secrets";

/// Rate-limit window for transcription session minting.
const TRANSCRIBE_RATE_WINDOW: Duration = Duration::from_secs(60);

/// Response for `GET /transcribe/status`.
#[derive(Serialize)]
pub struct TranscribeStatus {
    configured: bool,
    model: String,
}

/// Response for `POST /transcribe/session`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeSession {
    client_secret: String,
    model: String,
}

/// `GET /transcribe/status` — check if transcription is configured.
///
/// Requires NIP-98 auth. Returns whether the relay has an OpenAI API key
/// configured for real-time transcription **and** the caller is allowed to
/// mint sessions (i.e. is a relay member). This prevents non-members from
/// being prompted for microphone access only to hit a 403 on session creation.
pub async fn transcribe_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<TranscribeStatus>, (StatusCode, Json<Value>)> {
    let (pubkey, _community) = authenticate(&state, &headers, "/transcribe/status", "GET").await?;

    // Check both: API key configured AND caller can actually mint sessions.
    let configured = if state.config.openai_api_key.is_some() {
        // On membership-required relays, `authenticate` already verified
        // membership. On open relays, we need to check explicitly since
        // `/transcribe/session` enforces hard membership regardless.
        if state.config.require_relay_membership {
            true
        } else {
            can_mint_session(&state, &headers, &pubkey).await
        }
    } else {
        false
    };

    Ok(Json(TranscribeStatus {
        configured,
        model: state.config.transcription_model.clone(),
    }))
}

/// `POST /transcribe/session` — create an ephemeral OpenAI Realtime session.
///
/// Requires NIP-98 auth **and** relay membership (even on open relays).
/// Each session mints a metered OpenAI Realtime connection on the relay
/// operator's bill, so we require the caller to be an actual relay member
/// regardless of `BUZZ_REQUIRE_RELAY_MEMBERSHIP`.
pub async fn create_transcribe_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<TranscribeSession>, (StatusCode, Json<Value>)> {
    let (pubkey, community) = authenticate(&state, &headers, "/transcribe/session", "POST").await?;

    // Hard membership gate — billable endpoint requires actual relay membership
    // even on open relays where `enforce_relay_membership` would short-circuit.
    require_relay_member(&state, &headers, &pubkey).await?;

    // Per-(community, pubkey) rate limit — each session mints a metered OpenAI
    // Realtime connection on the operator's bill.
    if transcribe_rate_limited(&state, community, &pubkey) {
        metrics::counter!("buzz_transcribe_session_rejections_total", "reason" => "rate_limit")
            .increment(1);
        return Err(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "transcription session rate limit exceeded — try again shortly",
        ));
    }

    let api_key = state.config.openai_api_key.as_deref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "transcription is not configured on this relay",
        )
    })?;

    let model = state.config.transcription_model.clone();

    let client = openai_client().map_err(|(status, msg)| api_error(status, msg))?;

    // Build the session payload — VAD configuration depends on the model.
    // `gpt-realtime-whisper` requires manual audio commit (no turn detection),
    // while other models (e.g. `whisper-1`) use server-side VAD.
    let session_payload = build_session_payload(&model);

    let response = client
        .post(OPENAI_REALTIME_CLIENT_SECRETS_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&session_payload)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("OpenAI realtime session request failed: {e}");
            api_error(
                StatusCode::BAD_GATEWAY,
                "failed to create transcription session",
            )
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("OpenAI realtime session error ({status}): {body}");
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "OpenAI rejected the transcription session request",
        ));
    }

    let body: Value = response.json().await.map_err(|e| {
        tracing::error!("OpenAI realtime session response parse error: {e}");
        api_error(
            StatusCode::BAD_GATEWAY,
            "invalid response from transcription service",
        )
    })?;

    let client_secret = extract_client_secret(&body).ok_or_else(|| {
        tracing::error!("OpenAI realtime session response missing client_secret: {body}");
        api_error(
            StatusCode::BAD_GATEWAY,
            "transcription service returned unexpected response",
        )
    })?;

    Ok(Json(TranscribeSession {
        client_secret,
        model,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Check whether the caller can mint a transcription session (i.e. is a relay
/// member or delegated by one). Used by `/transcribe/status` to avoid showing
/// the mic button to users who would get a 403 on session creation.
async fn can_mint_session(
    state: &AppState,
    headers: &HeaderMap,
    pubkey: &nostr::PublicKey,
) -> bool {
    // Try the same membership check that `require_relay_member` uses.
    require_relay_member(state, headers, pubkey).await.is_ok()
}

/// Build the OpenAI Realtime session payload with model-appropriate VAD config.
///
/// - `gpt-realtime-whisper` (live transcription model): omits `turn_detection`
///   entirely — audio is committed manually by the client per OpenAI guidance.
/// - Other models (e.g. `whisper-1`): use `server_vad` for automatic turn
///   detection.
fn build_session_payload(model: &str) -> Value {
    let uses_manual_commit = model.contains("realtime-whisper");

    let audio_input = if uses_manual_commit {
        // Manual commit mode — omit turn_detection entirely.
        serde_json::json!({
            "transcription": { "model": model }
        })
    } else {
        // Server VAD mode — include turn_detection.
        serde_json::json!({
            "transcription": { "model": model },
            "turn_detection": { "type": "server_vad" }
        })
    };

    serde_json::json!({
        "session": {
            "type": "transcription",
            "audio": {
                "input": audio_input
            }
        },
        "expires_after": {
            "anchor": "created_at",
            "seconds": 60
        }
    })
}

/// Authenticate the request using the same NIP-98 / X-Pubkey pattern as the
/// bridge endpoints, plus replay detection and relay membership enforcement.
/// Returns the authenticated pubkey and resolved community on success.
async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    method: &str,
) -> Result<(nostr::PublicKey, CommunityId), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get("host")
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

    let url = super::bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    // Always require NIP-98 signed auth for transcribe endpoints — these mint
    // billable OpenAI sessions, so we cannot trust the unauthenticated X-Pubkey
    // dev fallback (which is spoofable) regardless of BUZZ_REQUIRE_AUTH_TOKEN.
    let (pubkey, event_id_bytes) = super::bridge::verify_bridge_auth(
        headers, method, &url, None, true, // force NIP-98 — billable endpoint
    )?;
    super::bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    // Enforce relay membership (with NIP-OA fallback via x-auth-tag header).
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    Ok((pubkey, tenant.community()))
}

/// Hard relay-membership check for billable endpoints.
///
/// Unlike `enforce_relay_membership` (which short-circuits on open relays),
/// this always verifies that the pubkey is an actual relay member or is
/// delegated via NIP-OA by a member. This prevents arbitrary NIP-98 signers
/// from minting metered sessions on the operator's bill.
async fn require_relay_member(
    state: &AppState,
    headers: &HeaderMap,
    pubkey: &nostr::PublicKey,
) -> Result<(), (StatusCode, Json<Value>)> {
    // If the relay already requires membership globally, the `authenticate`
    // call above handled it — no need to double-check.
    if state.config.require_relay_membership {
        return Ok(());
    }

    // On open relays we still require membership for this billable endpoint.
    let raw_host = headers
        .get("host")
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

    let pubkey_hex = pubkey.to_hex();
    let is_member = state
        .db
        .is_relay_member(tenant.community(), &pubkey_hex)
        .await
        .map_err(|e| {
            tracing::error!("transcribe membership check failed: {e}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "membership check failed")
        })?;

    if is_member {
        return Ok(());
    }

    // NIP-OA fallback: check if the agent's owner is a member.
    if state.config.allow_nip_oa_auth {
        let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
        if let Some(owner) =
            super::relay_members::extract_nip_oa_owner(&pubkey.to_bytes(), auth_tag)
        {
            let owner_hex = owner.to_hex();
            let owner_is_member = state
                .db
                .is_relay_member(tenant.community(), &owner_hex)
                .await
                .map_err(|e| {
                    tracing::error!("transcribe owner membership check failed: {e}");
                    api_error(StatusCode::INTERNAL_SERVER_ERROR, "membership check failed")
                })?;
            if owner_is_member {
                return Ok(());
            }
        }
    }

    Err(api_error(
        StatusCode::FORBIDDEN,
        "relay membership required for transcription sessions",
    ))
}

/// Per-(community, pubkey) sliding-window rate limiter for transcription session minting.
fn transcribe_rate_limited(state: &AppState, community: CommunityId, pubkey: &nostr::PublicKey) -> bool {
    let key = (community, pubkey.to_bytes());
    let now = Instant::now();
    let limit = state.config.transcribe_sessions_per_minute;
    let mut entry = state.transcribe_rate_limiter.entry(key).or_insert((0, now));
    let (count, window_start) = entry.value_mut();
    if now.duration_since(*window_start) >= TRANSCRIBE_RATE_WINDOW {
        *count = 1;
        *window_start = now;
        return false;
    }
    if *count >= limit {
        return true;
    }
    *count += 1;
    false
}

/// Shared HTTP client for OpenAI requests (connection pooling).
fn openai_client() -> Result<&'static reqwest::Client, (StatusCode, &'static str)> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    // `get_or_init` only runs the closure once; if TLS backend init fails we
    // propagate an API error instead of panicking in production.
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to initialize HTTP client",
            )
        })?;
    // Another thread may have raced us — `get_or_init` is fine here since we
    // already proved construction succeeds.
    Ok(CLIENT.get_or_init(|| client))
}

fn extract_client_secret(value: &Value) -> Option<String> {
    // Shape 1: { "client_secret": { "value": "..." } }
    if let Some(cs) = value.get("client_secret") {
        if let Some(v) = cs.get("value").and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
        // Shape 2: { "client_secret": "..." }
        if let Some(v) = cs.as_str() {
            return Some(v.to_string());
        }
    }
    // Shape 3: { "value": "..." }
    value
        .get("value")
        .and_then(|v| v.as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::extract_client_secret;
    use serde_json::json;

    #[test]
    fn parses_nested_client_secret() {
        let body = json!({ "client_secret": { "value": "sec_abc123", "expires_at": 9999 } });
        assert_eq!(extract_client_secret(&body), Some("sec_abc123".to_string()));
    }

    #[test]
    fn parses_direct_string_client_secret() {
        let body = json!({ "client_secret": "sec_direct" });
        assert_eq!(extract_client_secret(&body), Some("sec_direct".to_string()));
    }

    #[test]
    fn parses_top_level_value() {
        let body = json!({ "value": "sec_toplevel" });
        assert_eq!(
            extract_client_secret(&body),
            Some("sec_toplevel".to_string())
        );
    }

    #[test]
    fn returns_none_for_missing_secret() {
        let body = json!({ "id": "sess_123", "model": "gpt-4o" });
        assert_eq!(extract_client_secret(&body), None);
    }

    #[test]
    fn build_session_payload_whisper1_includes_server_vad() {
        use super::build_session_payload;
        let payload = build_session_payload("whisper-1");
        let td = &payload["session"]["audio"]["input"]["turn_detection"];
        assert_eq!(td["type"], "server_vad");
    }

    #[test]
    fn build_session_payload_sets_short_expires_after() {
        use super::build_session_payload;
        let payload = build_session_payload("whisper-1");
        assert_eq!(payload["expires_after"]["anchor"], "created_at");
        assert_eq!(payload["expires_after"]["seconds"], 60);
        let payload2 = build_session_payload("gpt-realtime-whisper");
        assert_eq!(payload2["expires_after"]["anchor"], "created_at");
        assert_eq!(payload2["expires_after"]["seconds"], 60);
    }

    #[test]
    fn build_session_payload_realtime_whisper_omits_turn_detection() {
        use super::build_session_payload;
        let payload = build_session_payload("gpt-realtime-whisper");
        let td = &payload["session"]["audio"]["input"]["turn_detection"];
        assert!(
            td.is_null(),
            "turn_detection should be absent for realtime-whisper model"
        );
    }

    #[test]
    fn build_session_payload_realtime_whisper_variant_omits_turn_detection() {
        use super::build_session_payload;
        let payload = build_session_payload("gpt-4o-realtime-whisper-20250512");
        let td = &payload["session"]["audio"]["input"]["turn_detection"];
        assert!(
            td.is_null(),
            "turn_detection should be absent for any realtime-whisper variant"
        );
    }
}
