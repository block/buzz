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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use nostr::{Keys, Tag};
use serde::{Deserialize, Serialize};

use crate::oidc::{self, OidcConfig};
use crate::roster::Roster;
use crate::token::{self, ConsumedNonces, MagicToken, TokenError, VerifiedToken};

/// How long an OIDC `state` is valid between `/oidc/start` and the callback.
const OIDC_STATE_TTL_SECS: u64 = 600;

/// How the service delivers magic links.
#[derive(Clone)]
pub enum Mailer {
    /// Production-safe placeholder until an actual email delivery backend is
    /// configured. `/email/start` stays enumeration-safe and sends nothing.
    Disabled,
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
    /// Shared HTTP client for the OIDC exchange.
    pub http: reqwest::Client,
    /// Slack OIDC credentials; `None` disables the `/oidc/*` routes.
    pub oidc: Option<OidcConfig>,
    /// Live OIDC `state` → (claimant pubkey, nonce, expiry). CSRF, replay
    /// protection, and pubkey binding.
    pub oidc_states: Mutex<HashMap<String, (String, String, u64)>>,
    /// Dev mode: enables `/oidc/dev-complete`, which simulates a verified OIDC
    /// result so the publish path can be exercised without a real Slack app.
    pub dev: bool,
}

/// Cloneable handle to the service state (an `Arc` under the hood).
#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl AppState {
    fn i(&self) -> &Inner {
        &self.0
    }
}

/// Build the router for both claim channels.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        // Email channel.
        .route("/email/start", post(email_start))
        .route("/email/verify", get(email_verify))
        .route("/email/complete", post(email_complete))
        // OIDC channel (Sign in with Slack).
        .route("/oidc/start", get(oidc_start))
        .route("/oidc/callback", get(oidc_callback))
        .route("/oidc/dev-complete", get(oidc_dev_complete))
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
) -> Result<Json<EmailStartResp>, ApiError> {
    let inner = state.i();
    let dev_link = match (&inner.mailer, inner.roster.subject_for_email(&req.email)) {
        (Mailer::Dev, Some(subject)) => {
            let tok = token::mint(
                &inner.token_secret,
                subject,
                now_secs(),
                inner.token_ttl_secs,
                &random_nonce(),
            )
            .map_err(|e| token_err_to_api(&e))?;
            let link = format!("{}/email/verify?token={}", inner.base_url, tok.as_str());
            tracing::info!(subject, %link, "dev mailer: magic link (not emailed)");
            Some(link)
        }
        (Mailer::Dev, None) => {
            // Unknown address: do the same amount of visible work, send nothing.
            tracing::info!(email = %req.email, "email/start for unknown address (ignored)");
            None
        }
        (Mailer::Disabled, _) => None,
    };
    Ok(Json(EmailStartResp {
        message: GENERIC_START_MSG.to_string(),
        dev_link,
    }))
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

    // Reserve before the network write to block concurrent redemption. Commit
    // only after relay acceptance; a transient relay failure releases the
    // reservation so the legitimate claimant can retry the same link.
    let verified = reserve_email_token(inner, &req.token, now_secs())?;
    let subject = verified.subject.clone();
    let event_id = match publish_attestation_for(inner, &subject, &pubkey, "email").await {
        Ok(event_id) => {
            consumed_ledger(inner).commit(&verified);
            event_id
        }
        Err(error) => {
            consumed_ledger(inner).release(&verified);
            return Err(error);
        }
    };
    Ok(Json(CompleteResp {
        subject,
        attestation_event_id: event_id,
    }))
}

/// Sign + publish the owner/admin attestation `subject → pubkey`. Shared by
/// every channel; `channel` is only for logging.
async fn publish_attestation_for(
    inner: &Inner,
    subject: &str,
    pubkey: &str,
    channel: &str,
) -> Result<String, ApiError> {
    let event_id = crate::attest::publish_attestation(
        &inner.relay_url,
        &inner.admin,
        subject,
        pubkey,
        inner.auth_tag.as_ref(),
    )
    .await
    .map_err(|e| ApiError::upstream(&e.to_string()))?;
    tracing::info!(subject, pubkey, event_id, channel, "published attestation");
    Ok(event_id)
}

// ---- OIDC channel (Sign in with Slack) --------------------------------------

#[derive(Deserialize)]
struct OidcStartQuery {
    /// The claimant's own Buzz pubkey (64-hex), supplied by their app.
    pubkey: String,
}

/// Begin Sign in with Slack: bind `state → pubkey` and redirect to Slack.
async fn oidc_start(
    State(state): State<AppState>,
    Query(q): Query<OidcStartQuery>,
) -> Result<Redirect, ApiError> {
    let inner = state.i();
    let cfg = inner.oidc.as_ref().ok_or_else(oidc_unconfigured)?;
    let pubkey =
        normalize_pubkey(&q.pubkey).ok_or_else(|| ApiError::bad("pubkey must be 64-char hex"))?;

    let st = hex::encode(random_nonce());
    let nonce = hex::encode(random_nonce());
    {
        let now = now_secs();
        let mut states = lock_or_recover(&inner.oidc_states, "oidc states");
        states.retain(|_, (_, _, exp)| *exp > now);
        states.insert(
            st.clone(),
            (
                pubkey,
                nonce.clone(),
                now.saturating_add(OIDC_STATE_TTL_SECS),
            ),
        );
    }
    Ok(Redirect::to(&oidc::authorize_url(cfg, &st, &nonce)))
}

#[derive(Deserialize)]
struct OidcCallbackQuery {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
}

/// Slack's redirect target: resolve `state → pubkey`, exchange the code for the
/// verified Slack user id, publish the attestation, and hand back to the app.
async fn oidc_callback(
    State(state): State<AppState>,
    Query(q): Query<OidcCallbackQuery>,
) -> Result<Redirect, ApiError> {
    let inner = state.i();
    let cfg = inner.oidc.as_ref().ok_or_else(oidc_unconfigured)?;

    // Consume the state → the pubkey the app started with (CSRF + binding).
    let (pubkey, nonce) = {
        let now = now_secs();
        let mut states = lock_or_recover(&inner.oidc_states, "oidc states");
        states.retain(|_, (_, _, exp)| *exp > now);
        states.remove(&q.state).map(|(pk, nonce, _)| (pk, nonce))
    }
    .ok_or_else(|| ApiError::bad("unknown or expired OIDC state"))?;

    let subject = oidc::exchange_code_for_subject(&inner.http, cfg, &q.code, &nonce)
        .await
        .map_err(|e| ApiError::upstream(&e.to_string()))?;

    publish_attestation_for(inner, &subject, &pubkey, "oidc").await?;
    Ok(Redirect::to(&format!(
        "buzz://import-claim?subject={}&via=oidc",
        urlencode(&subject)
    )))
}

#[derive(Deserialize)]
struct OidcDevCompleteQuery {
    pubkey: String,
    /// Simulated Slack user id (e.g. `U060`).
    sub: String,
}

/// Dev-only: simulate a verified OIDC result to exercise the publish path
/// without a Slack app. Returns 404 unless the service was started with --dev.
async fn oidc_dev_complete(
    State(state): State<AppState>,
    Query(q): Query<OidcDevCompleteQuery>,
) -> Result<Json<CompleteResp>, ApiError> {
    let inner = state.i();
    if !inner.dev {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "not found".into(),
        });
    }
    let pubkey =
        normalize_pubkey(&q.pubkey).ok_or_else(|| ApiError::bad("pubkey must be 64-char hex"))?;
    if q.sub.trim().is_empty() {
        return Err(ApiError::bad("sub must not be empty"));
    }
    let subject = format!("slack:{}", q.sub.trim());
    let event_id = publish_attestation_for(inner, &subject, &pubkey, "oidc-dev").await?;
    Ok(Json(CompleteResp {
        subject,
        attestation_event_id: event_id,
    }))
}

fn oidc_unconfigured() -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "OIDC channel is not configured on this service".into(),
    }
}

/// Verify a token and atomically reserve it for one in-flight relay write.
fn reserve_email_token(inner: &Inner, wire: &str, now: u64) -> Result<VerifiedToken, ApiError> {
    let tok = MagicToken::from_wire(wire.to_string());
    let verified =
        token::verify(&inner.token_secret, &tok, now).map_err(|e| token_err_to_api(&e))?;
    consumed_ledger(inner)
        .try_reserve(&verified, now)
        .map_err(|e| token_err_to_api(&e))?;
    Ok(verified)
}

fn consumed_ledger(inner: &Inner) -> MutexGuard<'_, ConsumedNonces> {
    lock_or_recover(&inner.consumed, "consumed nonce ledger")
}

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::error!(name, "recovering poisoned migration-service mutex");
            poisoned.into_inner()
        }
    }
}

fn token_err_to_api(e: &TokenError) -> ApiError {
    match e {
        TokenError::Expired | TokenError::AlreadyUsed => ApiError {
            status: StatusCode::GONE,
            message: e.to_string(),
        },
        TokenError::Unavailable => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
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
    let subject = escape_html(subject);
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
    let msg = escape_html(msg);
    format!(
        "<!doctype html><meta charset=utf-8><title>Link problem</title>\
         <body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1>This link can't be used</h1><p>{msg}</p>\
         <p style=\"color:#666\">Ask your operator to send a fresh migration link.</p></body>"
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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
            http: reqwest::Client::new(),
            oidc: None,
            oidc_states: Mutex::new(HashMap::new()),
            dev: false,
        }
    }

    #[test]
    fn reserve_happy_path_blocks_concurrent_redemption() {
        let inner = test_inner();
        let tok =
            token::mint(&inner.token_secret, "slack:U060", 1000, 3600, &[3u8; 16]).expect("mint");
        let verified = reserve_email_token(&inner, tok.as_str(), 1500).expect("reserves");
        assert_eq!(verified.subject, "slack:U060");
        let err = reserve_email_token(&inner, tok.as_str(), 1500).unwrap_err();
        assert_eq!(err.status, StatusCode::GONE);
    }

    #[test]
    fn reserve_rejects_forged_token() {
        let inner = test_inner();
        let forged =
            token::mint(b"other-secret", "slack:U060", 1000, 3600, &[3u8; 16]).expect("mint");
        let err = reserve_email_token(&inner, forged.as_str(), 1500).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn reserve_rejects_expired_token() {
        let inner = test_inner();
        let tok =
            token::mint(&inner.token_secret, "slack:U060", 1000, 10, &[3u8; 16]).expect("mint");
        let err = reserve_email_token(&inner, tok.as_str(), 2000).unwrap_err();
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

    #[test]
    fn verify_page_escapes_export_supplied_subject() {
        let page = verify_page(r#"slack:<script>"x"</script>"#, "buzz://safe");
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(page.contains("&quot;x&quot;"));
    }
}
