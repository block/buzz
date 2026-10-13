//! Web push prefetch: a short-lived, HMAC-scoped read token minted per web
//! wake so the service worker can warm the opened channel/thread before the
//! user taps. Disabled unless both `BUZZ_PUSH_PREFETCH_SECRET` and
//! `BUZZ_PUSH_PREFETCH_URL` are set. Tokens are bearer-only (no cookies),
//! expire after five minutes, are limited to a few uses, and every read
//! re-checks channel membership.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use buzz_core::tenant::CommunityId;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// Token lifetime.
pub const TOKEN_TTL_SECS: i64 = 300;
/// Reads allowed per token.
pub const MAX_USES: u32 = 3;
/// Maximum events returned per read.
pub const MAX_EVENTS: i64 = 50;

/// Signed token payload.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Claims {
    /// Version (1).
    pub v: u8,
    /// Lease author (hex pubkey) whose membership is re-checked on read.
    pub a: String,
    /// Community.
    pub c: Uuid,
    /// Channel.
    pub ch: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Thread root (hex), when the wake was a thread reply.
    pub r: Option<String>,
    /// Wake event id (hex); results start at its created_at.
    pub s: String,
    /// Expiry, Unix seconds.
    pub e: i64,
}

/// Token rejection reason.
#[derive(Debug, PartialEq)]
pub enum TokenError {
    /// Not a well-formed v1 token.
    Malformed,
    /// HMAC mismatch.
    BadSignature,
    /// Past `e`.
    Expired,
}

struct Settings {
    secret: Vec<u8>,
    url: String,
}

fn settings() -> Option<&'static Settings> {
    static S: OnceLock<Option<Settings>> = OnceLock::new();
    S.get_or_init(|| {
        let secret = std::env::var("BUZZ_PUSH_PREFETCH_SECRET").ok()?;
        let url = std::env::var("BUZZ_PUSH_PREFETCH_URL").ok()?;
        let url = url.trim().to_string();
        if secret.len() < 32 || !url.starts_with("https://") || url.len() > 256 {
            tracing::warn!(
                "push prefetch disabled: secret must be >=32 bytes and url https (<=256 chars)"
            );
            return None;
        }
        Some(Settings {
            secret: secret.into_bytes(),
            url,
        })
    })
    .as_ref()
}

/// Sign claims into `b64url(payload).b64url(hmac)`.
pub fn mint(secret: &[u8], claims: &Claims) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims serialize"));
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(payload.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{payload}.{sig}")
}

/// Verify signature and expiry.
pub fn verify(secret: &[u8], token: &str, now: i64) -> Result<Claims, TokenError> {
    if token.len() > 512 {
        return Err(TokenError::Malformed);
    }
    let (payload, sig) = token.split_once('.').ok_or(TokenError::Malformed)?;
    let sig = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|_| TokenError::Malformed)?;
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig)
        .map_err(|_| TokenError::BadSignature)?;
    let raw = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| TokenError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&raw).map_err(|_| TokenError::Malformed)?;
    if claims.v != 1 {
        return Err(TokenError::Malformed);
    }
    if claims.e < now {
        return Err(TokenError::Expired);
    }
    Ok(claims)
}

/// Hint attached to web wakes: `{url, token}`. None when disabled or the wake
/// has no channel.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct PrefetchHint {
    /// Absolute https prefetch endpoint.
    pub url: String,
    /// Bearer token.
    pub token: String,
}

/// Hint from process configuration.
pub fn hint_for(
    wake: &buzz_db::push::ClaimedWake,
    thread_root: Option<&[u8]>,
) -> Option<PrefetchHint> {
    let s = settings()?;
    hint_with(
        &s.secret,
        &s.url,
        wake,
        thread_root,
        chrono::Utc::now().timestamp(),
    )
}

/// Hint from explicit key/url (testable).
pub fn hint_with(
    secret: &[u8],
    url: &str,
    wake: &buzz_db::push::ClaimedWake,
    thread_root: Option<&[u8]>,
    now: i64,
) -> Option<PrefetchHint> {
    let ch = wake.channel_id?;
    let claims = Claims {
        v: 1,
        a: hex::encode(&wake.author),
        c: *wake.community.as_uuid(),
        ch,
        r: thread_root.map(hex::encode),
        s: hex::encode(&wake.event_id),
        e: now + TOKEN_TTL_SECS,
    };
    Some(PrefetchHint {
        url: url.to_string(),
        token: mint(secret, &claims),
    })
}

/// Returns false once a token has been used `MAX_USES` times.
fn take_use(token: &str, exp: i64, now: i64) -> bool {
    static USES: OnceLock<Mutex<HashMap<String, (u32, i64)>>> = OnceLock::new();
    let mut map = USES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if map.len() > 10_000 {
        map.retain(|_, (_, e)| *e >= now);
    }
    let sig = token.rsplit('.').next().unwrap_or(token).to_string();
    let entry = map.entry(sig).or_insert((0, exp));
    entry.0 += 1;
    entry.0 <= MAX_USES
}

fn cors(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// CORS preflight.
pub async fn options_handler() -> Response {
    cors(StatusCode::NO_CONTENT.into_response())
}

/// `GET /push/prefetch`.
pub async fn prefetch_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    cors(prefetch_inner(&state, &headers).await)
}

async fn prefetch_inner(state: &AppState, headers: &HeaderMap) -> Response {
    let Some(s) = settings() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let now = chrono::Utc::now().timestamp();
    let claims = match verify(&s.secret, token, now) {
        Ok(c) => c,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    if !take_use(token, claims.e, now) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let (Ok(author), Ok(since)) = (hex::decode(&claims.a), hex::decode(&claims.s)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let root = match claims.r.as_deref().map(hex::decode).transpose() {
        Ok(r) => r,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let community = CommunityId::from_uuid(claims.c);
    match state.db.is_member(community, claims.ch, &author).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::FORBIDDEN.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
    let ids = match state
        .db
        .push_prefetch_event_ids(community, claims.ch, root.as_deref(), &since, MAX_EVENTS)
        .await
    {
        Ok(ids) => ids,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let refs: Vec<&[u8]> = ids.iter().map(Vec::as_slice).collect();
    let events = match state.db.get_events_by_ids(community, &refs).await {
        Ok(e) => e,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut events: Vec<_> = events.into_iter().map(|e| e.event).collect();
    events.sort_by_key(|e| e.created_at);
    axum::Json(serde_json::json!({
        "channel_id": claims.ch,
        "thread_root_id": claims.r,
        "events": events,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn claims(e: i64) -> Claims {
        Claims {
            v: 1,
            a: "ab".into(),
            c: Uuid::nil(),
            ch: Uuid::nil(),
            r: None,
            s: "cd".into(),
            e,
        }
    }

    #[test]
    fn roundtrip() {
        let t = mint(K, &claims(100));
        assert_eq!(verify(K, &t, 50).unwrap(), claims(100));
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
        assert!(t.len() <= 512);
    }

    #[test]
    fn rejects_expired_tampered_wrong_key() {
        let t = mint(K, &claims(100));
        assert_eq!(verify(K, &t, 101), Err(TokenError::Expired));
        assert_eq!(
            verify(b"another-key-another-key-another!", &t, 50),
            Err(TokenError::BadSignature)
        );
        let (p, sig) = t.split_once('.').unwrap();
        let other = mint(
            K,
            &Claims {
                ch: Uuid::from_u128(9),
                ..claims(100)
            },
        );
        let (p2, _) = other.split_once('.').unwrap();
        assert_ne!(p, p2);
        assert_eq!(
            verify(K, &format!("{p2}.{sig}"), 50),
            Err(TokenError::BadSignature)
        );
        assert_eq!(verify(K, "nodot", 50), Err(TokenError::Malformed));
    }

    #[test]
    fn use_limit() {
        let t = mint(K, &claims(i64::MAX));
        for _ in 0..MAX_USES {
            assert!(take_use(&t, i64::MAX, 0));
        }
        assert!(!take_use(&t, i64::MAX, 0));
    }
}
