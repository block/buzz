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
use nostr::{Event, JsonUtil, Keys, Tag};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::oidc::{self, OidcConfig};
use crate::roster::Roster;
use crate::token::{self, ConsumedNonces, MagicToken, TokenError, VerifiedToken};

/// How long an OIDC `state` is valid between `/oidc/start` and the callback.
const OIDC_STATE_TTL_SECS: u64 = 600;
/// How long a post-callback finalize `code` is valid. The first valid
/// redemption binds it to one key; only that key may retry.
const OIDC_CODE_TTL_SECS: u64 = 300;
/// Bound unauthenticated `/oidc/start` memory use (and the pending-code map).
/// Operators should also rate-limit the public endpoint at their reverse proxy.
const MAX_PENDING_OIDC_STATES: usize = 4096;

/// A callback code becomes permanently bound to the first valid claimant key
/// that redeems it. Replays by that same key are safe because both relay writes
/// are idempotent; a different key can never take over a partially completed
/// migration.
pub struct OidcFinalizeCode {
    subject: String,
    pubkey: Option<String>,
    expires_at: u64,
}

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
    /// Slack workspace id — the `<team>` in every `slack:<team>:<user>` subject.
    pub team_id: String,
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
    /// Live OIDC `state` → (nonce, expiry). CSRF and replay protection for the
    /// Slack round-trip. The claimant pubkey is deliberately NOT bound here — it
    /// is proven only at `/oidc/finalize`, so an unauthenticated `/oidc/start`
    /// can never pin an attacker's key to a victim's Slack login.
    pub oidc_states: Mutex<HashMap<String, (String, u64)>>,
    /// Post-callback `code` → verified subject + first proven claimant key.
    /// Codes are retryable only by that same key until expiry.
    pub oidc_codes: Mutex<HashMap<String, OidcFinalizeCode>>,
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
        .route("/oidc/finalize", post(oidc_finalize))
        .route("/oidc/dev-complete", get(oidc_dev_complete))
        // Buzz Desktop runs in a WebView origin. The public migration
        // endpoints are already protected by OIDC state, short-lived bearer
        // codes, and signed claims; CORS is transport compatibility, not an
        // authorization boundary.
        .layer(CorsLayer::permissive())
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

#[derive(Debug, Serialize)]
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

/// Sign and publish the owner/admin attestation `subject → pubkey`.
///
/// This does not grant community membership. Email claims are a recovery
/// channel for an already-admitted member; only the dedicated Slack OAuth join
/// path below is allowed to admit a new member.
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

/// Admit an OAuth-verified Slack user, then attest their imported identity.
///
/// Both relay writes are idempotent: adding an existing member is a no-op and
/// the attestation is parameterized-replaceable. A callback retry therefore
/// cannot duplicate membership or attribution.
async fn publish_join_attestation_for(
    inner: &Inner,
    subject: &str,
    pubkey: &str,
    channel: &str,
) -> Result<String, ApiError> {
    let member_event_id = crate::attest::publish_add_member(
        &inner.relay_url,
        &inner.admin,
        pubkey,
        inner.auth_tag.as_ref(),
    )
    .await
    .map_err(|e| ApiError::upstream(&e.to_string()))?;
    tracing::info!(pubkey, member_event_id, channel, "ensured community member");
    publish_attestation_for(inner, subject, pubkey, channel).await
}

// ---- OIDC channel (Sign in with Slack) --------------------------------------

/// Begin Sign in with Slack: store a fresh `state`/`nonce` and redirect to
/// Slack. The claimant's Buzz pubkey is intentionally not accepted here — it is
/// proven at `/oidc/finalize`. This closes the takeover where an attacker starts
/// OIDC bound to their own key and has a victim complete the Slack login.
async fn oidc_start(State(state): State<AppState>) -> Result<Redirect, ApiError> {
    let inner = state.i();
    let cfg = inner.oidc.as_ref().ok_or_else(oidc_unconfigured)?;

    let st = hex::encode(random_nonce());
    let nonce = hex::encode(random_nonce());
    {
        let now = now_secs();
        let mut states = lock_or_recover(&inner.oidc_states, "oidc states");
        states.retain(|_, (_, exp)| *exp > now);
        if states.len() >= MAX_PENDING_OIDC_STATES {
            return Err(ApiError::too_many_requests(
                "too many pending Slack sign-ins; try again shortly",
            ));
        }
        states.insert(
            st.clone(),
            (nonce.clone(), now.saturating_add(OIDC_STATE_TTL_SECS)),
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

/// Slack's redirect target: resolve `state`, exchange the code for the verified
/// Slack user id, mint a short-lived finalize `code`, and hand back to the app.
///
/// It deliberately does NOT publish anything. The attestation is signed only at
/// `/oidc/finalize`, once the app proves control of the pubkey to be attested —
/// so a victim's Slack login can never mint an attestation for someone else's
/// key.
async fn oidc_callback(
    State(state): State<AppState>,
    Query(q): Query<OidcCallbackQuery>,
) -> Result<Redirect, ApiError> {
    let inner = state.i();
    let cfg = inner.oidc.as_ref().ok_or_else(oidc_unconfigured)?;

    // Consume the state (CSRF + replay) → the nonce we sent Slack.
    let nonce = {
        let now = now_secs();
        let mut states = lock_or_recover(&inner.oidc_states, "oidc states");
        states.retain(|_, (_, exp)| *exp > now);
        states.remove(&q.state).map(|(nonce, _)| nonce)
    }
    .ok_or_else(|| ApiError::bad("unknown or expired OIDC state"))?;

    let subject = oidc::exchange_code_for_subject(&inner.http, cfg, &q.code, &nonce)
        .await
        .map_err(|e| ApiError::upstream(&e.to_string()))?;

    // Mint a short-lived code bound to the verified subject. The first valid
    // self-claim also binds it permanently to that claimant key.
    let code = hex::encode(random_nonce());
    {
        let now = now_secs();
        let mut codes = lock_or_recover(&inner.oidc_codes, "oidc codes");
        codes.retain(|_, pending| pending.expires_at > now);
        if codes.len() >= MAX_PENDING_OIDC_STATES {
            return Err(ApiError::too_many_requests(
                "too many pending Slack sign-ins; try again shortly",
            ));
        }
        codes.insert(
            code.clone(),
            OidcFinalizeCode {
                subject: subject.clone(),
                pubkey: None,
                expires_at: now.saturating_add(OIDC_CODE_TTL_SECS),
            },
        );
    }
    Ok(Redirect::to(&oidc_app_return_url(inner, &subject, &code)))
}

fn oidc_app_return_url(inner: &Inner, subject: &str, code: &str) -> String {
    format!(
        "buzz://import-claim?subject={}&via=oidc&code={}&relay={}&service={}",
        urlencode(subject),
        urlencode(code),
        urlencode(&inner.relay_url),
        urlencode(&inner.base_url),
    )
}

#[derive(Deserialize)]
struct OidcFinalizeReq {
    /// Short-lived code from the `/oidc/callback` redirect.
    code: String,
    /// The claimant's signed self-claim (kind `KIND_IMPORT_IDENTITY_CLAIM`). Its
    /// valid signature proves control of the pubkey to be attested; its `d` tag
    /// must equal the code's OIDC-verified subject.
    claim: serde_json::Value,
}

/// The app's proof-of-possession finalize: redeem a post-OIDC `code` and publish
/// the owner/admin attestation — bound to the pubkey that *signed* the
/// accompanying self-claim, never to an attacker-suppliable parameter.
///
/// The claim event is fully verified (id hash + Schnorr signature); its `d` tag
/// must equal the Slack-verified subject the code stands for. Because the code
/// reaches only the app that completed the Slack login, and the attested pubkey
/// is proven by signature, a phisher who makes a victim authenticate can neither
/// obtain the code nor forge a self-claim for a key they do not hold.
async fn oidc_finalize(
    State(state): State<AppState>,
    Json(req): Json<OidcFinalizeReq>,
) -> Result<Json<CompleteResp>, ApiError> {
    let inner = state.i();

    // Reject the wrong event kind up front (cheap), then fully verify id+sig
    // before trusting any field read off the event.
    let kind_ok = req.claim.get("kind").and_then(|k| k.as_u64())
        == Some(u64::from(buzz_core::kind::KIND_IMPORT_IDENTITY_CLAIM));
    if !kind_ok {
        return Err(ApiError::bad("claim must be an identity-claim event"));
    }
    let claim_json = serde_json::to_string(&req.claim)
        .map_err(|_| ApiError::bad("claim must be a JSON event"))?;
    let event = Event::from_json(&claim_json)
        .map_err(|_| ApiError::bad("claim is not a valid Nostr event"))?;
    let to_verify = event.clone();
    tokio::task::spawn_blocking(move || buzz_core::verification::verify_event(&to_verify))
        .await
        .map_err(|_| ApiError::upstream("claim verification task failed"))?
        .map_err(|_| ApiError::bad("claim signature is invalid"))?;

    let claim_subject = event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) == Some("d") {
            parts.get(1).cloned()
        } else {
            None
        }
    });

    // Bind the code to the first key that proves possession. Keeping that
    // binding until expiry lets the same app retry if the relay accepted the
    // attestation but the subsequent client-side claim publish failed.
    let pubkey = event.pubkey.to_hex();
    let subject = bind_oidc_finalize_code(
        inner,
        &req.code,
        claim_subject.as_deref(),
        &pubkey,
        now_secs(),
    )?;
    let event_id = publish_join_attestation_for(inner, &subject, &pubkey, "oidc").await?;
    Ok(Json(CompleteResp {
        subject,
        attestation_event_id: event_id,
    }))
}

fn bind_oidc_finalize_code(
    inner: &Inner,
    code: &str,
    claim_subject: Option<&str>,
    pubkey: &str,
    now: u64,
) -> Result<String, ApiError> {
    let mut codes = lock_or_recover(&inner.oidc_codes, "oidc codes");
    codes.retain(|_, pending| pending.expires_at > now);
    let pending = codes
        .get_mut(code)
        .ok_or_else(|| ApiError::bad("unknown or expired finalize code"))?;
    if claim_subject != Some(pending.subject.as_str()) {
        return Err(ApiError::bad(
            "self-claim does not match the verified Slack identity",
        ));
    }
    match pending.pubkey.as_deref() {
        Some(bound) if bound != pubkey => {
            return Err(ApiError::bad(
                "finalize code is already bound to a different Buzz account",
            ));
        }
        Some(_) => {}
        None => pending.pubkey = Some(pubkey.to_string()),
    }
    Ok(pending.subject.clone())
}

#[derive(Deserialize)]
struct OidcDevCompleteQuery {
    /// Simulated Slack user id (e.g. `U060`).
    sub: String,
}

#[derive(Serialize)]
struct DevCompleteResp {
    subject: String,
    /// The `buzz://import-claim` URL a real Slack callback would redirect to,
    /// carrying the short-lived finalize code — follow it to drive the app.
    app_return_url: String,
}

/// Dev-only: simulate a verified OIDC result. Mints the same finalize code a
/// real callback would and returns the app deep link, so the finalize path can
/// be exercised without a Slack app. Returns 404 unless started with --dev.
async fn oidc_dev_complete(
    State(state): State<AppState>,
    Query(q): Query<OidcDevCompleteQuery>,
) -> Result<Json<DevCompleteResp>, ApiError> {
    let inner = state.i();
    if !inner.dev {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "not found".into(),
        });
    }
    let sub = q.sub.trim();
    if sub.is_empty() {
        return Err(ApiError::bad("sub must not be empty"));
    }
    let subject = format!("slack:{}:{sub}", inner.team_id);
    let code = hex::encode(random_nonce());
    {
        let now = now_secs();
        let mut codes = lock_or_recover(&inner.oidc_codes, "oidc codes");
        codes.insert(
            code.clone(),
            OidcFinalizeCode {
                subject: subject.clone(),
                pubkey: None,
                expires_at: now.saturating_add(OIDC_CODE_TTL_SECS),
            },
        );
    }
    let app_return_url = oidc_app_return_url(inner, &subject, &code);
    Ok(Json(DevCompleteResp {
        subject,
        app_return_url,
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
    fn too_many_requests(msg: &str) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
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
            roster: Roster::from_users_json(users.as_bytes(), "T060").unwrap(),
            token_secret: b"secret".to_vec(),
            consumed: Mutex::new(ConsumedNonces::new()),
            admin: Keys::generate(),
            team_id: "T060".into(),
            relay_url: "ws://127.0.0.1:1".into(),
            auth_tag: None,
            base_url: "http://localhost:8787".into(),
            token_ttl_secs: 3600,
            mailer: Mailer::Dev,
            http: reqwest::Client::new(),
            oidc: None,
            oidc_states: Mutex::new(HashMap::new()),
            oidc_codes: Mutex::new(HashMap::new()),
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

    #[tokio::test]
    async fn disabled_mailer_does_not_expose_known_addresses() {
        let mut inner = test_inner();
        inner.mailer = Mailer::Disabled;
        let state = AppState(Arc::new(inner));

        let known = email_start(
            State(state.clone()),
            Json(EmailStartReq {
                email: "alice@corp.com".into(),
            }),
        )
        .await
        .expect("generic response");
        let unknown = email_start(
            State(state),
            Json(EmailStartReq {
                email: "unknown@corp.com".into(),
            }),
        )
        .await
        .expect("generic response");

        assert!(known.0.dev_link.is_none());
        assert!(unknown.0.dev_link.is_none());
        assert_eq!(known.0.message, unknown.0.message);
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
    fn oidc_return_carries_code_relay_and_service() {
        let inner = test_inner();
        assert_eq!(
            oidc_app_return_url(&inner, "slack:U060", "c0de"),
            "buzz://import-claim?subject=slack%3AU060&via=oidc&code=c0de&relay=ws%3A%2F%2F127.0.0.1%3A1&service=http%3A%2F%2Flocalhost%3A8787"
        );
    }

    #[tokio::test]
    async fn oidc_start_rejects_when_pending_state_capacity_is_reached() {
        let mut inner = test_inner();
        inner.oidc = Some(OidcConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://migrate.example/oidc/callback".into(),
            team_id: "T060".into(),
        });
        inner.oidc_states = Mutex::new(
            (0..MAX_PENDING_OIDC_STATES)
                .map(|index| (format!("state-{index}"), ("nonce".into(), u64::MAX)))
                .collect(),
        );
        let state = AppState(Arc::new(inner));
        let error = match oidc_start(State(state)).await {
            Ok(_) => panic!("capacity must reject"),
            Err(error) => error,
        };
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
    }

    /// Build a signed self-claim (kind `KIND_IMPORT_IDENTITY_CLAIM`) with the
    /// given `d` subject, as JSON — what the app POSTs to `/oidc/finalize`.
    fn signed_claim(subject: &str) -> (serde_json::Value, Keys) {
        use nostr::{EventBuilder, Kind};
        let keys = Keys::generate();
        let event = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_IMPORT_IDENTITY_CLAIM as u16),
            "",
        )
        .tags([Tag::parse(["d", subject]).unwrap()])
        .sign_with_keys(&keys)
        .expect("sign");
        (serde_json::from_str(&event.as_json()).expect("json"), keys)
    }

    fn inner_with_code(code: &str, subject: &str) -> Inner {
        let inner = test_inner();
        inner.oidc_codes.lock().unwrap().insert(
            code.to_string(),
            OidcFinalizeCode {
                subject: subject.to_string(),
                pubkey: None,
                expires_at: u64::MAX,
            },
        );
        inner
    }

    #[test]
    fn finalize_code_allows_same_key_retry_but_rejects_a_different_key() {
        let inner = inner_with_code("retry-code", "slack:U060");
        let first = bind_oidc_finalize_code(
            &inner,
            "retry-code",
            Some("slack:U060"),
            &"aa".repeat(32),
            100,
        )
        .unwrap();
        assert_eq!(first, "slack:U060");
        assert!(bind_oidc_finalize_code(
            &inner,
            "retry-code",
            Some("slack:U060"),
            &"aa".repeat(32),
            101,
        )
        .is_ok());
        let error = bind_oidc_finalize_code(
            &inner,
            "retry-code",
            Some("slack:U060"),
            &"bb".repeat(32),
            102,
        )
        .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn mismatched_subject_does_not_consume_or_bind_finalize_code() {
        let inner = inner_with_code("subject-code", "slack:U060");
        assert!(bind_oidc_finalize_code(
            &inner,
            "subject-code",
            Some("slack:U999"),
            &"aa".repeat(32),
            100,
        )
        .is_err());
        assert!(bind_oidc_finalize_code(
            &inner,
            "subject-code",
            Some("slack:U060"),
            &"bb".repeat(32),
            101,
        )
        .is_ok());
    }

    #[tokio::test]
    async fn finalize_rejects_claim_for_a_different_subject() {
        // Slack verified U060, but the self-claim consents to U999. Even with a
        // valid signature and a live code, the mismatch is refused — no attacker
        // can redirect a code onto a different identity.
        let inner = inner_with_code("code1", "slack:U060");
        let (claim, _keys) = signed_claim("slack:U999");
        let state = AppState(Arc::new(inner));
        let err = oidc_finalize(
            State(state),
            Json(OidcFinalizeReq {
                code: "code1".into(),
                claim,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finalize_rejects_tampered_signature() {
        // A claim whose signature doesn't verify never reaches the code redemption
        // or the relay — the attested pubkey must be genuinely proven.
        let inner = inner_with_code("code2", "slack:U060");
        let (mut claim, _keys) = signed_claim("slack:U060");
        claim["sig"] = serde_json::Value::String("0".repeat(128));
        let state = AppState(Arc::new(inner));
        let err = oidc_finalize(
            State(state),
            Json(OidcFinalizeReq {
                code: "code2".into(),
                claim,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finalize_rejects_unknown_code() {
        let inner = inner_with_code("code3", "slack:U060");
        let (claim, _keys) = signed_claim("slack:U060");
        let state = AppState(Arc::new(inner));
        let err = oidc_finalize(
            State(state),
            Json(OidcFinalizeReq {
                code: "not-the-code".into(),
                claim,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finalize_rejects_wrong_kind() {
        let inner = inner_with_code("code4", "slack:U060");
        let keys = Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(1), "")
            .tags([Tag::parse(["d", "slack:U060"]).unwrap()])
            .sign_with_keys(&keys)
            .expect("sign");
        let claim: serde_json::Value = serde_json::from_str(&event.as_json()).expect("json");
        let state = AppState(Arc::new(inner));
        let err = oidc_finalize(
            State(state),
            Json(OidcFinalizeReq {
                code: "code4".into(),
                claim,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn verify_page_escapes_export_supplied_subject() {
        let page = verify_page(r#"slack:<script>"x"</script>"#, "buzz://safe");
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(page.contains("&quot;x&quot;"));
    }
}
