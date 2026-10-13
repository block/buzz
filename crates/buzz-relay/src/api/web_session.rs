//! Web session tokens for the HTTP read bridge.
//!
//! A browser client signs ONE NIP-98 request to `POST /auth/session` and gets
//! back an opaque bearer token it then sends as `Authorization: BuzzSession
//! <token>` on `/query` and `/count`, instead of asking its (often slow,
//! remote) signer to sign every read.
//!
//! - **Short-lived:** tokens expire after [`SESSION_TTL_SECS`] (Redis `EX`).
//! - **Scoped:** read-only (`/query`, `/count`), bound to the minting pubkey
//!   AND the community resolved from the request host. Writes (`/events`) still
//!   need a signed event. Membership is re-checked on every request.
//! - **Revocable:** `DELETE /auth/session` deletes the token; losing relay
//!   membership also stops it working immediately.
//!
//! Only `sha256(token)` is stored server-side.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use buzz_core::TenantContext;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::api_error;
use super::bridge::{
    check_nip98_replay, enforce_http_admission, nip98_expected_url,
    verify_bridge_auth_with_options, VerifiedBridgeAuth,
};
use crate::state::AppState;

/// Token lifetime (8 hours).
pub const SESSION_TTL_SECS: u64 = 8 * 60 * 60;
/// Authorization scheme for session tokens.
pub const SCHEME: &str = "BuzzSession ";
/// Mint/revoke path.
pub const PATH: &str = "/auth/session";

type ApiErr = (StatusCode, Json<Value>);

fn session_key(tenant: &TenantContext, token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!(
        "buzz:websession:v1:{}:{}",
        tenant.community(),
        hex::encode(digest)
    )
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix(SCHEME))
        .map(str::trim)
}

/// Well-formed token: 43 chars of base64url (32 random bytes).
fn plausible(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn encode_value(pubkey_hex: &str, signed_created_at: Option<u64>) -> String {
    format!("{pubkey_hex}|{}", signed_created_at.unwrap_or(0))
}

fn decode_value(v: &str) -> Option<(nostr::PublicKey, Option<u64>)> {
    let (pk, sca) = v.split_once('|')?;
    let pubkey = nostr::PublicKey::from_hex(pk).ok()?;
    let sca: u64 = sca.parse().ok()?;
    Some((pubkey, (sca > 0).then_some(sca)))
}

/// Resolve a `BuzzSession` bearer. `None` when the request doesn't carry one
/// (the caller falls back to NIP-98). The returned auth has a zero event id,
/// so the NIP-98 replay check is skipped (the token is multi-use by design).
pub(crate) async fn resolve(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
) -> Option<Result<VerifiedBridgeAuth, ApiErr>> {
    let token = bearer(headers)?;
    Some(resolve_token(state, tenant, token).await)
}

async fn resolve_token(
    state: &AppState,
    tenant: &TenantContext,
    token: &str,
) -> Result<VerifiedBridgeAuth, ApiErr> {
    let expired = || api_error(StatusCode::UNAUTHORIZED, "session: expired or revoked");
    if !plausible(token) {
        return Err(expired());
    }
    let mut conn = state.redis_pool.get().await.map_err(|e| {
        tracing::warn!(error = %e, "web session: redis pool acquire failed");
        api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
    })?;
    let value: Option<String> = redis::cmd("GET")
        .arg(session_key(tenant, token))
        .query_async(&mut *conn)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "web session: redis GET failed");
            api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
        })?;
    let (pubkey, signed_created_at) = value
        .as_deref()
        .and_then(decode_value)
        .ok_or_else(expired)?;
    Ok(VerifiedBridgeAuth {
        pubkey,
        event_id_bytes: [0u8; 32],
        signed_created_at,
    })
}

fn bind_host(headers: &HeaderMap) -> &str {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

async fn bind(state: &AppState, headers: &HeaderMap) -> Result<TenantContext, ApiErr> {
    crate::tenant::bind_community(&state.db, bind_host(headers))
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })
}

/// `POST /auth/session` — NIP-98 signed (real signature required, no dev
/// X-Pubkey). Returns `{ token, expires_at }`.
pub async fn mint_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiErr> {
    let tenant = bind(&state, &headers).await?;
    let url = nip98_expected_url(&state.config.relay_url, &tenant, PATH);
    let auth = verify_bridge_auth_with_options(&headers, "POST", &url, Some(&body), true, false)?;
    enforce_http_admission(&state, &tenant, &auth.pubkey).await?;
    check_nip98_replay(&state, &tenant, auth.event_id_bytes).await?;
    let pubkey_bytes = auth.pubkey.to_bytes().to_vec();
    let auth_tag = super::relay_members::extract_auth_tag_header(&headers);
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        auth.signed_created_at,
    )
    .await?;

    let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let mut conn = state.redis_pool.get().await.map_err(|e| {
        tracing::warn!(error = %e, "web session: redis pool acquire failed");
        api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
    })?;
    let _: () = redis::cmd("SET")
        .arg(session_key(&tenant, &token))
        .arg(encode_value(&auth.pubkey.to_hex(), auth.signed_created_at))
        .arg("EX")
        .arg(SESSION_TTL_SECS)
        .query_async(&mut *conn)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "web session: redis SET failed");
            api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
        })?;
    let expires_at = chrono::Utc::now().timestamp() + SESSION_TTL_SECS as i64;
    tracing::info!(pubkey = %auth.pubkey.to_hex(), route = PATH, status = 200u16, "web session minted");
    Ok(Json(json!({ "token": token, "expires_at": expires_at })))
}

/// `DELETE /auth/session` with `Authorization: BuzzSession <token>` — revokes it.
pub async fn revoke_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiErr> {
    let tenant = bind(&state, &headers).await?;
    let Some(token) = bearer(&headers) else {
        return Err(api_error(StatusCode::UNAUTHORIZED, "missing session"));
    };
    if !plausible(token) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let mut conn = state.redis_pool.get().await.map_err(|e| {
        tracing::warn!(error = %e, "web session: redis pool acquire failed");
        api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
    })?;
    let _: i64 = redis::cmd("DEL")
        .arg(session_key(&tenant, token))
        .query_async(&mut *conn)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "web session: redis DEL failed");
            api_error(StatusCode::SERVICE_UNAVAILABLE, "session: unavailable")
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_round_trips() {
        let keys = nostr::Keys::generate();
        let hexpk = keys.public_key().to_hex();
        let (p, s) = decode_value(&encode_value(&hexpk, Some(1_700_000_000))).unwrap();
        assert_eq!(p.to_hex(), hexpk);
        assert_eq!(s, Some(1_700_000_000));
        let (_, s) = decode_value(&encode_value(&hexpk, None)).unwrap();
        assert_eq!(s, None);
    }

    #[test]
    fn token_shape() {
        let t = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        assert!(plausible(&t));
        assert!(!plausible("short"));
        assert!(!plausible(&format!("{}!", &t[..42])));
    }

    #[test]
    fn bearer_parses_scheme_only() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "BuzzSession abc".parse().unwrap());
        assert_eq!(bearer(&h), Some("abc"));
        h.insert("authorization", "Nostr abc".parse().unwrap());
        assert_eq!(bearer(&h), None);
    }
}
