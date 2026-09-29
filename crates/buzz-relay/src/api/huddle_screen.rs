//! Huddle screen-share LiveKit token mint (NIP-98 HTTP).
//!
//! Voice stays on `/huddle/{channel}/audio` (Opus). This module only mints
//! short-lived LiveKit JWTs so desktop can publish/subscribe **one** screen
//! video track. See `docs/huddle-screen-share.md`.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::audio::handler::ensure_membership;
use crate::state::AppState;

use super::{api_error, bridge, internal_error, relay_members};

/// Default LiveKit access-token lifetime.
pub(crate) const TOKEN_TTL_SECS: u64 = 15 * 60;

pub(crate) const SCREEN_TOKEN_PATH_SUFFIX: &str = "/screen-token";
pub(crate) const SCREEN_STOP_PATH_SUFFIX: &str = "/screen-stop";

/// Body for `POST /api/huddle/{channel_id}/screen-token`.
#[derive(Debug, Deserialize)]
pub struct ScreenTokenRequest {
    /// Parent channel for ephemeral huddles (same as audio join).
    #[serde(default)]
    pub parent_channel_id: Option<Uuid>,
    /// `subscribe` (default) or `publish` (claim one-sharer slot).
    #[serde(default = "default_intent")]
    pub intent: String,
}

fn default_intent() -> String {
    "subscribe".to_string()
}

/// Body for `POST /api/huddle/{channel_id}/screen-stop` (optional parent).
#[derive(Debug, Default, Deserialize)]
pub struct ScreenStopRequest {
    /// Parent channel for ephemeral huddles (same as audio join).
    #[serde(default)]
    pub parent_channel_id: Option<Uuid>,
}

/// LiveKit `video` grant claim subset we mint.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LiveKitVideoGrant {
    #[serde(rename = "roomJoin")]
    pub room_join: bool,
    pub room: String,
    #[serde(rename = "canSubscribe")]
    pub can_subscribe: bool,
    #[serde(rename = "canPublish")]
    pub can_publish: bool,
    #[serde(rename = "canPublishData")]
    pub can_publish_data: bool,
    #[serde(
        rename = "canPublishSources",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub can_publish_sources: Option<Vec<String>>,
}

/// LiveKit access-token claims (HS256, `iss` = API key).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct LiveKitClaims {
    pub exp: u64,
    pub iss: String,
    pub nbf: u64,
    pub sub: String,
    pub video: LiveKitVideoGrant,
}

/// Derive the LiveKit room name from the huddle ephemeral/backing channel id.
pub(crate) fn livekit_room_name(channel_id: Uuid) -> String {
    format!("huddle-{channel_id}")
}

/// Build LiveKit JWT claims for a participant.
pub(crate) fn build_livekit_claims(
    api_key: &str,
    identity: &str,
    room: &str,
    can_publish: bool,
    now_secs: u64,
    ttl_secs: u64,
) -> LiveKitClaims {
    LiveKitClaims {
        exp: now_secs.saturating_add(ttl_secs),
        iss: api_key.to_string(),
        nbf: now_secs.saturating_sub(10),
        sub: identity.to_string(),
        video: LiveKitVideoGrant {
            room_join: true,
            room: room.to_string(),
            can_subscribe: true,
            can_publish,
            can_publish_data: false,
            can_publish_sources: can_publish.then(|| vec!["screen_share".to_string()]),
        },
    }
}

/// Sign LiveKit claims with HS256 using the API secret.
pub(crate) fn encode_livekit_token(
    claims: &LiveKitClaims,
    api_secret: &str,
) -> Result<String, String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(claims.iss.clone());
    let key = EncodingKey::from_secret(api_secret.as_bytes());
    encode(&header, claims, &key).map_err(|e| format!("livekit jwt encode: {e}"))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

fn path_for_channel(channel_id: Uuid, suffix: &str) -> String {
    format!("/api/huddle/{channel_id}{suffix}")
}

async fn authenticate(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<(buzz_core::TenantContext, nostr::PublicKey), (StatusCode, Json<Value>)> {
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

    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        signed_created_at,
    } = bridge::verify_bridge_auth_with_options(
        headers,
        "POST",
        &url,
        Some(body),
        true,
        true,
    )?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    let _owner = relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        pubkey.as_bytes(),
        relay_members::extract_auth_tag_header(headers),
        signed_created_at,
    )
    .await?;

    Ok((tenant, pubkey))
}

fn require_livekit(
    state: &AppState,
) -> Result<&crate::config::LiveKitConfig, (StatusCode, Json<Value>)> {
    state.config.livekit.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "screen_share_unavailable",
                "message": "LiveKit is not configured on this relay"
            })),
        )
    })
}

/// Mint a LiveKit access token — `POST /api/huddle/{channel_id}/screen-token`.
pub async fn mint_screen_token(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = path_for_channel(channel_id, SCREEN_TOKEN_PATH_SUFFIX);
    let (tenant, pubkey) = authenticate(&state, &headers, &path, &body).await?;
    let livekit = require_livekit(&state)?;

    let request: ScreenTokenRequest = serde_json::from_slice(&body).map_err(|_| {
        api_error(StatusCode::BAD_REQUEST, "invalid JSON body")
    })?;

    let intent = request.intent.trim().to_ascii_lowercase();
    let want_publish = match intent.as_str() {
        "subscribe" | "" => false,
        "publish" => true,
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "intent must be subscribe or publish",
            ));
        }
    };

    ensure_membership(
        &state,
        &tenant,
        channel_id,
        pubkey.as_bytes(),
        request.parent_channel_id,
    )
    .await
    .map_err(|e| {
        let status = if e.contains("archived") || e.contains("not linked") || e.contains("not a member")
        {
            StatusCode::FORBIDDEN
        } else if e.contains("requires parent") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            internal_error(&format!("huddle screen membership: {e}"))
        } else {
            api_error(status, &e)
        }
    })?;

    let pubkey_hex = pubkey.to_hex();
    let sharer_key = (tenant.community(), channel_id);
    let current_sharer = state
        .huddle_screen_sharers
        .get(&sharer_key)
        .map(|entry| entry.value().clone());

    if want_publish {
        if let Some(ref sharer) = current_sharer {
            if sharer != &pubkey_hex {
                return Err((
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "screen_share_busy",
                        "message": "another participant is already sharing",
                        "current_sharer": sharer,
                    })),
                ));
            }
        }
        state
            .huddle_screen_sharers
            .insert(sharer_key, pubkey_hex.clone());
    }

    let room = livekit_room_name(channel_id);
    let claims = build_livekit_claims(
        &livekit.api_key,
        &pubkey_hex,
        &room,
        want_publish,
        now_unix_secs(),
        TOKEN_TTL_SECS,
    );
    let token = encode_livekit_token(&claims, livekit.api_secret())
        .map_err(|e| internal_error(&e))?;

    let current_sharer = state
        .huddle_screen_sharers
        .get(&(tenant.community(), channel_id))
        .map(|entry| entry.value().clone());

    Ok(Json(serde_json::json!({
        "url": livekit.url,
        "token": token,
        "room": room,
        "can_publish": want_publish,
        "current_sharer": current_sharer,
        "expires_in_secs": TOKEN_TTL_SECS,
    })))
}

/// Release the one-sharer slot — `POST /api/huddle/{channel_id}/screen-stop`.
pub async fn stop_screen_share(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = path_for_channel(channel_id, SCREEN_STOP_PATH_SUFFIX);
    let (tenant, pubkey) = authenticate(&state, &headers, &path, &body).await?;
    // When LiveKit is unset there is nothing to stop; match mint's unavailable.
    let _livekit = require_livekit(&state)?;

    let request: ScreenStopRequest = if body.is_empty() {
        ScreenStopRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            api_error(StatusCode::BAD_REQUEST, "invalid JSON body")
        })?
    };

    // Soft membership: allow stop even if channel archived so cleanup works.
    let _ = ensure_membership(
        &state,
        &tenant,
        channel_id,
        pubkey.as_bytes(),
        request.parent_channel_id,
    )
    .await;

    let pubkey_hex = pubkey.to_hex();
    let sharer_key = (tenant.community(), channel_id);
    let mut released = false;
    if let Some((_, held)) = state.huddle_screen_sharers.remove(&sharer_key) {
        if held == pubkey_hex {
            released = true;
        } else {
            // Put it back — only the current sharer may clear.
            state.huddle_screen_sharers.insert(sharer_key, held);
        }
    }

    Ok(Json(serde_json::json!({
        "released": released,
        "current_sharer": state
            .huddle_screen_sharers
            .get(&sharer_key)
            .map(|e| e.value().clone()),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_name_is_stable() {
        let id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        assert_eq!(
            livekit_room_name(id),
            "huddle-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
    }

    #[test]
    fn subscribe_claims_cannot_publish() {
        let claims = build_livekit_claims("APIkey", "pubkeyhex", "huddle-1", false, 1_700_000_000, 60);
        assert!(claims.video.room_join);
        assert!(claims.video.can_subscribe);
        assert!(!claims.video.can_publish);
        assert!(claims.video.can_publish_sources.is_none());
        assert_eq!(claims.iss, "APIkey");
        assert_eq!(claims.sub, "pubkeyhex");
        assert_eq!(claims.exp, 1_700_000_060);
    }

    #[test]
    fn publish_claims_limit_sources_to_screen_share() {
        let claims = build_livekit_claims("APIkey", "pubkeyhex", "huddle-1", true, 1_700_000_000, 60);
        assert!(claims.video.can_publish);
        assert_eq!(
            claims
                .video
                .can_publish_sources
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["screen_share"])
        );
    }

    #[test]
    fn encode_round_trips_header_and_claims_shape() {
        let claims = build_livekit_claims(
            "APItest",
            "deadbeef",
            "huddle-room",
            true,
            1_700_000_000,
            TOKEN_TTL_SECS,
        );
        let token = encode_livekit_token(&claims, "test-secret").expect("encode");
        let parts: Vec<_> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "jwt must have three segments");

        use base64::Engine;
        let header_json = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[0])
                .expect("header b64"),
        )
        .expect("utf8");
        assert!(header_json.contains("HS256"));
        assert!(header_json.contains("APItest"));

        let payload_json = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[1])
                .expect("payload b64"),
        )
        .expect("utf8");
        let parsed: LiveKitClaims = serde_json::from_str(&payload_json).expect("claims");
        assert_eq!(parsed.video.room, "huddle-room");
        assert_eq!(parsed.sub, "deadbeef");
        assert!(parsed.video.can_publish);
    }
}
