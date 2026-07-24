//! The claim-service HTTP surface (email channel).
//!
//! Three endpoints implement the email proof:
//!
//! - `POST /email/start {email}` — mint a magic-link token for the Slack user
//!   that email belongs to and mail it there. Deliberately takes **no pubkey**
//!   and always answers the same way whether or not the email is known, so it
//!   can neither be used to enumerate the workspace nor to smuggle an attacker
//!   pubkey (see [`crate::token`]).
//! - `GET /email/verify?token=…` — the link target. Validates the token and
//!   hands off to the recipient's Buzz app via a `buzz://import-claim` deep
//!   link. Does not consume the token (the app completes the claim).
//! - `POST /email/complete {token, pubkey}` — the app calls this with **its
//!   own** key. Re-verifies, atomically consumes (single use), then publishes
//!   the owner/admin attestation `subject → pubkey`. The app separately
//!   publishes the matching self-claim; only then is history attributed.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use nostr::{Keys, Tag};
use serde::{Deserialize, Serialize};

use crate::roster::Roster;
use crate::token::{self, ConsumedNonces, MagicToken, TokenError};

/// How the service delivers magic links.
#[derive(Clone)]
pub enum Mailer {
    /// Development: don't send anything; the link is logged and returned in the
    /// `/email/start` response so a local tester can follow it by hand.
    Dev,
}

/// Immutable service configuration + shared mutable single-use ledger.
pub struct Inner {
    pub roster: Roster,
    pub token_secret: Vec<u8>,
    pub consumed: Mutex<ConsumedNonces>,
    pub admin: Keys,
    pub relay_url: String,
    pub auth_tag: Option<Tag>,
    /// Public base URL of this service, used to build magic links.
    pub base_url: String,
    pub token_ttl_secs: u64,
    pub mailer: Mailer,
}

/// Cloneable handle to the service state (an `Arc` under the hood).
#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl AppState {
    fn i(&self) -> &Inner {
        &self.0
    }
}

/// Build the router for the email channel.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/email/start", post(email_start))
        .route("/email/verify", get(email_verify))
        .route("/email/complete", post(email_complete))
        .with_state(state)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_nonce() -> [u8; 16] {
    rand::random()
}

/// Normalize a caller-supplied Buzz pubkey to canonical lowercase hex, or
/// reject it. Only 64-char hex is accepted (the app sends hex, never an nsec).
fn normalize_pubkey(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s.to_lowercase())
    } else {
        None
    }
}

// ---- POST /email/start -------------------------------------------------------

#[derive(Deserialize)]
struct EmailStartReq {
    email: String,
}

#[derive(Serialize)]
struct EmailStartResp {
    /// Always the same generic message, regardless of whether the email is
    /// known — do not leak workspace membership.
    message: String,
    /// Only populated in [`Mailer::Dev`]: the magic link to follow by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    dev_link: Option<String>,
}

const GENERIC_START_MSG: &str =
    "If that address belongs to a workspace member, a migration link has been sent to it.";

async fn email_start(
    State(state): State<AppState>,
    Json(req): Json<EmailStartReq>,
) -> Json<EmailStartResp> {
    let inner = state.i();
    let dev_link = match inner.roster.subject_for_email(&req.email) {
        Some(subject) => {
            let tok = token::mint(
                &inner.token_secret,
                subject,
                now_secs(),
                inner.token_ttl_secs,
                &random_nonce(),
            );
            let link = format!("{}/email/verify?token={}", inner.base_url, tok.as_str());
            match inner.mailer {
                Mailer::Dev => {
                    tracing::info!(subject, %link, "dev mailer: magic link (not emailed)");
                    Some(link)
                }
            }
        }
        None => {
            // Unknown address: do the same amount of visible work, send nothing.
            tracing::info!(email = %req.email, "email/start for unknown address (ignored)");
            None
        }
    };
    Json(EmailStartResp {
        message: GENERIC_START_MSG.to_string(),
        dev_link,
    })
}

// ---- GET /email/verify -------------------------------------------------------

#[derive(Deserialize)]
struct VerifyQuery {
    token: String,
}

/// The magic-link target. Validates integrity + expiry (not single use) and
/// renders a page that hands the token to the recipient's Buzz app.
async fn email_verify(
    State(state): State<AppState>,
    Query(q): Query<VerifyQuery>,
) -> impl IntoResponse {
    let inner = state.i();
    let tok = MagicToken::from_wire(q.token.clone());
    match token::verify(&inner.token_secret, &tok, now_secs()) {
        Ok(v) => {
            let deep_link = format!(
                "buzz://import-claim?subject={}&token={}&service={}",
                urlencode(&v.subject),
                urlencode(tok.as_str()),
                urlencode(&inner.base_url),
            );
            Html(verify_page(&v.subject, &deep_link)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Html(error_page(&e.to_string()))).into_response(),
    }
}

// ---- POST /email/complete ----------------------------------------------------

#[derive(Deserialize)]
struct CompleteReq {
    token: String,
    /// The claimant's Buzz pubkey (64-hex) — supplied by their own app.
    pubkey: String,
}

#[derive(Serialize)]
struct CompleteResp {
    subject: String,
    /// The published attestation's event id.
    attestation_event_id: String,
}

async fn email_complete(
    State(state): State<AppState>,
    Json(req): Json<CompleteReq>,
) -> Result<Json<CompleteResp>, ApiError> {
    let inner = state.i();
    let pubkey = normalize_pubkey(&req.pubkey)
        .ok_or_else(|| ApiError::bad("pubkey must be 64-char hex (the app's own key)"))?;

    // Verify + atomically consume (single use). Pure, network-free.
    let subject = redeem_email_token(inner, &req.token, now_secs())?;

    // Publish the owner/admin attestation subject → pubkey.
    let event_id = crate::attest::publish_attestation(
        &inner.relay_url,
        &inner.admin,
        &subject,
        &pubkey,
        inner.auth_tag.as_ref(),
    )
    .await
    .map_err(|e| ApiError::upstream(&e.to_string()))?;

    tracing::info!(
        subject,
        pubkey,
        event_id,
        "published attestation for email claim"
    );
    Ok(Json(CompleteResp {
        subject,
        attestation_event_id: event_id,
    }))
}

/// Verify a token and, on success, atomically mark it used, returning the
/// proven subject. Factored out so the security-critical redeem path is unit
/// tested without a relay.
fn redeem_email_token(inner: &Inner, wire: &str, now: u64) -> Result<String, ApiError> {
    let tok = MagicToken::from_wire(wire.to_string());
    let verified =
        token::verify(&inner.token_secret, &tok, now).map_err(|e| token_err_to_api(&e))?;
    {
        let mut ledger = inner.consumed.lock().expect("consumed ledger not poisoned");
        ledger
            .try_consume(&verified, now)
            .map_err(|e| token_err_to_api(&e))?;
    }
    Ok(verified.subject)
}

fn token_err_to_api(e: &TokenError) -> ApiError {
    match e {
        TokenError::Expired | TokenError::AlreadyUsed => ApiError {
            status: StatusCode::GONE,
            message: e.to_string(),
        },
        _ => ApiError::bad(&e.to_string()),
    }
}

// ---- shared error + small helpers -------------------------------------------

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad(msg: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.to_string(),
        }
    }
    fn upstream(msg: &str) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: msg.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        #[derive(Serialize)]
        struct Body {
            error: String,
        }
        (
            self.status,
            Json(Body {
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// Minimal percent-encoding for the query values we build (no external dep).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn verify_page(subject: &str, deep_link: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>Complete your migration</title>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1>You're verified</h1>\
         <p>Open Buzz to finish linking your imported history for <code>{subject}</code>.</p>\
         <p><a href=\"{deep_link}\" style=\"display:inline-block;padding:.6rem 1rem;\
         background:#5b21b6;color:#fff;border-radius:.5rem;text-decoration:none\">\
         Open in Buzz</a></p>\
         <p style=\"color:#666;font-size:.85rem\">This link is single-use and expires soon. \
         It links your history to the Buzz account on this device — only continue if you \
         started this in Buzz.</p></body>"
    )
}

fn error_page(msg: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>Link problem</title>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1>This link can't be used</h1><p>{msg}</p>\
         <p style=\"color:#666\">Ask your operator to send a fresh migration link.</p></body>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_inner() -> Inner {
        let users =
            r#"[{"id":"U060","profile":{"email":"alice@corp.com","display_name":"Alice"}}]"#;
        Inner {
            roster: Roster::from_users_json(users.as_bytes()).unwrap(),
            token_secret: b"secret".to_vec(),
            consumed: Mutex::new(ConsumedNonces::new()),
            admin: Keys::generate(),
            relay_url: "ws://127.0.0.1:1".into(),
            auth_tag: None,
            base_url: "http://localhost:8787".into(),
            token_ttl_secs: 3600,
            mailer: Mailer::Dev,
        }
    }

    #[test]
    fn redeem_happy_path_returns_subject_and_consumes() {
        let inner = test_inner();
        let tok = token::mint(&inner.token_secret, "slack:U060", 1000, 3600, &[3u8; 16]);
        let subject = redeem_email_token(&inner, tok.as_str(), 1500).expect("redeems");
        assert_eq!(subject, "slack:U060");
        // Second redeem of the same token is rejected as used.
        let err = redeem_email_token(&inner, tok.as_str(), 1500).unwrap_err();
        assert_eq!(err.status, StatusCode::GONE);
    }

    #[test]
    fn redeem_rejects_forged_token() {
        let inner = test_inner();
        let forged = token::mint(b"other-secret", "slack:U060", 1000, 3600, &[3u8; 16]);
        let err = redeem_email_token(&inner, forged.as_str(), 1500).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn redeem_rejects_expired_token() {
        let inner = test_inner();
        let tok = token::mint(&inner.token_secret, "slack:U060", 1000, 10, &[3u8; 16]);
        let err = redeem_email_token(&inner, tok.as_str(), 2000).unwrap_err();
        assert_eq!(err.status, StatusCode::GONE);
    }

    #[test]
    fn pubkey_normalization() {
        assert_eq!(normalize_pubkey(&"AB".repeat(32)).unwrap(), "ab".repeat(32));
        assert!(normalize_pubkey("npub1xyz").is_none());
        assert!(normalize_pubkey("tooshort").is_none());
        assert!(normalize_pubkey(&"g".repeat(64)).is_none()); // non-hex
    }

    #[test]
    fn urlencode_escapes_reserved() {
        assert_eq!(urlencode("slack:U060"), "slack%3AU060");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }
}
