//! Relay invite HTTP API — mint and claim stateless invite codes.
//!
//! Routes (both NIP-98 signed, outside the Nostr event data plane):
//!
//! - `POST /api/invites` — mint an invite code. Caller must hold the `owner`
//!   or `admin` role in the tenant community (mirrors the kind:9030 authz).
//! - `POST /api/invites/claim` — claim an invite code. Deliberately **exempt
//!   from the relay-membership gate**: the whole point is that the caller is
//!   not a member yet. NIP-98 proves control of the joining pubkey; the HMAC
//!   on the code proves an admin authorized the join.
//!
//! Token format, key derivation, and security trade-offs live in
//! [`crate::invite_token`].

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Response},
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::handlers::side_effects::{publish_nip43_member_added, publish_nip43_membership_list};
use buzz_core::invite::{
    hash_v2_code, validate_v2_code, DEFAULT_INVITE_TTL_SECS, MAX_INVITE_TTL_SECS, MAX_INVITE_USES,
    MIN_INVITE_TTL_SECS, V2_PREFIX,
};

use crate::invite_token;
use crate::state::AppState;

use super::{api_error, bridge, internal_error};

/// Fixed-window size for the per-pubkey claim rate limiter.
pub(crate) const CLAIM_RATE_WINDOW: Duration = Duration::from_secs(60);
/// Max claim attempts per pubkey per window. Claims are idempotent and a real
/// user performs exactly one, so this only bounds brute-force probing.
const CLAIM_RATE_LIMIT: u32 = 10;
/// Maximum distinct pubkeys retained by the process-local claim limiter.
/// NIP-98 proves key ownership, not that a key is costly to create, so this
/// bound is required in addition to expiry.
pub(crate) const CLAIM_RATE_CACHE_CAPACITY: u64 = 10_000;

/// V3 identity-handoff protocol marker carried on mint responses and claims.
pub const IDENTITY_HANDOFF_PROTOCOL: &str = "identity-handoff-v3";
const IDENTITY_HANDOFF_MAX_BODY_BYTES: usize = 4 * 1024;
const IDENTITY_HANDOFF_MINT_RATE_LIMIT: u32 = 10;
const IDENTITY_HANDOFF_STATUS_RATE_LIMIT: u32 = 60;
const IDENTITY_HANDOFF_CLAIM_RATE_LIMIT: u32 = 10;
pub(crate) const IDENTITY_HANDOFF_RATE_CACHE_CAPACITY: u64 = 20_000;
pub(crate) const IDENTITY_HANDOFF_MINT_RATE_WINDOW: Duration = Duration::from_secs(60 * 60);
pub(crate) const IDENTITY_HANDOFF_STATUS_RATE_WINDOW: Duration = Duration::from_secs(60);
pub(crate) const IDENTITY_HANDOFF_CLAIM_RATE_WINDOW: Duration = Duration::from_secs(10 * 60);
const STATUS_KEY_DIMENSION: u8 = 1;
const STATUS_HANDOFF_DIMENSION: u8 = 2;
const CLAIM_CLAIMANT_DIMENSION: u8 = 1;
const CLAIM_TOKEN_DIMENSION: u8 = 2;

type ApiError = (StatusCode, Json<Value>);
type IdentityHandoffResult = Result<Json<Value>, IdentityHandoffApiError>;

enum IdentityHandoffApiError {
    Standard(ApiError),
    RateLimited {
        retry_after: u64,
        reason: &'static str,
    },
}

impl From<ApiError> for IdentityHandoffApiError {
    fn from(error: ApiError) -> Self {
        Self::Standard(error)
    }
}

impl IntoResponse for IdentityHandoffApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Standard(error) => error.into_response(),
            Self::RateLimited {
                retry_after,
                reason,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, retry_after.to_string())],
                Json(serde_json::json!({ "error": reason })),
            )
                .into_response(),
        }
    }
}

/// Body for `POST /api/invites`.
#[derive(Debug, Default, Deserialize)]
pub struct MintInviteRequest {
    /// Requested lifetime in seconds. Must be between
    /// [`MIN_INVITE_TTL_SECS`] and
    /// [`invite_token::MAX_INVITE_TTL_SECS`]; defaults to 72 h.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
    /// Maximum number of uses before the invite is exhausted. `None` (omitted
    /// or `null`) means unlimited — preserves current behavior. When present,
    /// must be an integer from 1 through [`MAX_INVITE_USES`].
    #[serde(default)]
    pub max_uses: Option<i32>,
}

fn validate_mint_request(
    request: &MintInviteRequest,
) -> Result<(u64, Option<i32>), (StatusCode, Json<Value>)> {
    let ttl = request.ttl_secs.unwrap_or(DEFAULT_INVITE_TTL_SECS);
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&ttl) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "ttl_secs must be between {} and {MAX_INVITE_TTL_SECS}",
                MIN_INVITE_TTL_SECS
            ),
        ));
    }

    if let Some(max_uses) = request.max_uses {
        if !(1..=MAX_INVITE_USES).contains(&max_uses) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                &format!("max_uses must be between 1 and {MAX_INVITE_USES}"),
            ));
        }
    }

    Ok((ttl, request.max_uses))
}

/// Body for `POST /api/invites/claim`.
#[derive(Debug, Deserialize)]
pub struct ClaimInviteRequest {
    /// The invite code to redeem.
    pub code: String,
    /// Relay-issued proof of accepting the configured terms, when required.
    #[serde(default)]
    pub policy_receipt: Option<String>,
    /// Required exact marker for v3 identity-handoff claims. Generic v1/v2
    /// callers may continue omitting it.
    #[serde(default)]
    pub protocol: Option<String>,
}

/// Body for `POST /api/identity-handoffs`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MintIdentityHandoffRequest {
    /// Public key the handoff is bound to.
    pub expected_pubkey: String,
    /// Non-secret Identity link incarnation identifier.
    pub link_incarnation_id: String,
    /// Fixed one-hour lifetime. Other values are rejected.
    pub ttl_secs: u64,
}

/// Body for `POST /api/identity-handoffs/status`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityHandoffStatusRequest {
    /// Opaque, non-authorizing handoff locator.
    pub handoff_id: String,
    /// Linked public key used to reauthorize the lookup.
    pub expected_pubkey: String,
    /// Link incarnation used to reauthorize the lookup.
    pub link_incarnation_id: String,
}

/// Body for `POST /api/identity-handoffs/invalidate`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvalidateIdentityHandoffsRequest {
    /// Linked public key whose active handoffs must be invalidated.
    pub expected_pubkey: String,
    /// Link incarnation that must be permanently fenced.
    pub link_incarnation_id: String,
}

/// Body for `POST /api/invites/accept-policy`.
#[derive(Debug, Deserialize)]
pub struct AcceptPolicyRequest {
    /// Invite code the acceptance receipt will be bound to.
    pub code: String,
    /// Policy revision displayed by the client.
    pub policy_version: String,
    /// Minimum-age assertion, required only when configured by the operator.
    #[serde(default)]
    pub age_confirmed: bool,
}

/// Public join policy shared by every client-side join surface.
pub async fn join_policy(State(state): State<Arc<AppState>>) -> Json<Value> {
    match &state.config.join_policy {
        Some(policy) => Json(serde_json::json!({
            "policy": {
                "terms_markdown": policy.terms_markdown,
                "privacy_markdown": policy.privacy_markdown,
                "age_attestation_required": policy.age_attestation_required,
                "version": policy.version
            }
        })),
        None => Json(serde_json::json!({})),
    }
}

/// `GET /api/join-policy/terms` — Terms of Service as a standalone HTML page.
///
/// Serves the operator-configured Markdown as a real browser page so desktop
/// clients can hand the link to the system browser instead of rendering the
/// document inside the webview (which requires app chrome the onboarding
/// surfaces don't have). 404 when no terms document is configured.
pub async fn join_policy_terms(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, Json<Value>)> {
    policy_document_page(&state, "Terms of Service", |policy| {
        policy.terms_markdown.as_deref()
    })
}

/// `GET /api/join-policy/privacy` — Privacy Policy as a standalone HTML page.
pub async fn join_policy_privacy(
    State(state): State<Arc<AppState>>,
) -> Result<Html<String>, (StatusCode, Json<Value>)> {
    policy_document_page(&state, "Privacy Policy", |policy| {
        policy.privacy_markdown.as_deref()
    })
}

fn policy_document_page(
    state: &AppState,
    title: &str,
    select: impl Fn(&crate::config::JoinPolicyConfig) -> Option<&str>,
) -> Result<Html<String>, (StatusCode, Json<Value>)> {
    let markdown = state
        .config
        .join_policy
        .as_ref()
        .and_then(select)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "join_policy_not_configured"))?;
    Ok(Html(render_policy_document(title, markdown)))
}

/// Render operator Markdown into a minimal self-contained HTML page.
///
/// Raw HTML embedded in the Markdown is escaped and rendered as text — the
/// operator authors a policy document, not a web page, and this keeps the
/// endpoint from serving arbitrary operator-controlled markup.
fn render_policy_document(title: &str, markdown: &str) -> String {
    use pulldown_cmark::{html, Event, Parser};

    let mut body = String::new();
    html::push_html(
        &mut body,
        Parser::new(markdown).map(|event| match event {
            Event::Html(raw) => Event::Text(raw.into_string().into()),
            Event::InlineHtml(raw) => Event::Text(raw.into_string().into()),
            other => other,
        }),
    );

    // Titles are fixed literals today; escape anyway so a future caller
    // can't accidentally inject markup through this seam.
    let escaped_title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{escaped_title}</title>\n\
         <style>body{{max-width:42rem;margin:2rem auto;padding:0 1rem;\
         font-family:system-ui,sans-serif;line-height:1.6}}</style>\n\
         </head>\n<body>\n{body}</body>\n</html>\n"
    )
}

/// Exchange explicit policy acceptance for a short-lived, invite-bound receipt.
pub async fn accept_policy(
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let Some(policy) = &state.config.join_policy else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "join_policy_not_configured",
        ));
    };
    let request: AcceptPolicyRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid policy acceptance JSON: {e}"),
        )
    })?;
    if request.policy_version != policy.version
        || (policy.age_attestation_required && !request.age_confirmed)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "join_policy_not_accepted",
        ));
    }
    let key = invite_token::derive_invite_key(&state.relay_keypair);
    let receipt = invite_token::mint_policy_acceptance(&key, &request.code, &policy.version);
    Ok(Json(serde_json::json!({ "receipt": receipt })))
}

/// Shared prelude: bind the tenant from the Host header and verify the NIP-98
/// signature + replay for `path`.
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
    let (pubkey, event_id_bytes) = bridge::verify_bridge_auth_with_options(
        headers,
        "POST",
        &url,
        Some(body),
        true, // invites always require NIP-98; no X-Pubkey dev fallback
        true, // POST bodies must be covered by a payload tag
    )?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    Ok((tenant, pubkey))
}

fn require_identity_handoff_nonce(headers: &HeaderMap) -> Result<(), ApiError> {
    let encoded = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Nostr "))
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "missing Nostr auth"))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid NIP-98 nonce"))?;
    let event: Value = serde_json::from_slice(&decoded)
        .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "invalid NIP-98 nonce"))?;
    let nonce_tags = event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(|parts| parts.first().and_then(Value::as_str) == Some("nonce"))
        .collect::<Vec<_>>();
    let valid_nonce = nonce_tags.len() == 1
        && nonce_tags[0].len() == 2
        && nonce_tags[0]
            .get(1)
            .and_then(Value::as_str)
            .is_some_and(|nonce| uuid::Uuid::parse_str(nonce).is_ok());
    if !valid_nonce {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "NIP-98: fresh nonce required",
        ));
    }
    Ok(())
}

async fn authenticate_identity_handoff(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<(buzz_core::TenantContext, nostr::PublicKey), ApiError> {
    let authenticated = authenticate(state, headers, path, body).await?;
    require_identity_handoff_nonce(headers)?;
    Ok(authenticated)
}

async fn require_owner_or_admin(
    state: &AppState,
    tenant: &buzz_core::TenantContext,
    pubkey: &nostr::PublicKey,
) -> Result<(), ApiError> {
    let sender_hex = pubkey.to_hex();
    let member = state
        .db
        .get_relay_member(tenant.community(), &sender_hex)
        .await
        .map_err(|error| internal_error(&format!("identity handoff role lookup: {error}")))?;
    if member
        .as_ref()
        .is_some_and(|member| member.role == "owner" || member.role == "admin")
    {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "identity_handoff_admin_required",
        ))
    }
}

fn parse_identity_handoff_request<T: serde::de::DeserializeOwned>(
    body: &[u8],
) -> Result<T, ApiError> {
    if body.is_empty() || body.len() > IDENTITY_HANDOFF_MAX_BODY_BYTES {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "identity_handoff_invalid_request",
        ));
    }
    serde_json::from_slice(body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request"))
}

fn parse_identity_handoff_pubkey(raw: &str) -> Result<nostr::PublicKey, ApiError> {
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "identity_handoff_invalid_request",
        ));
    }
    nostr::PublicKey::from_hex(raw)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request"))
}

fn validate_link_incarnation_id(link_incarnation_id: &str) -> Result<(), ApiError> {
    if !(16..=256).contains(&link_incarnation_id.len()) || !link_incarnation_id.is_ascii() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "identity_handoff_invalid_request",
        ));
    }
    Ok(())
}

fn validate_identity_handoff_mint_request(
    request: &MintIdentityHandoffRequest,
) -> Result<nostr::PublicKey, ApiError> {
    if request.ttl_secs != buzz_db::relay_invite::IDENTITY_HANDOFF_TTL_SECS as u64 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "identity_handoff_ttl_must_be_3600",
        ));
    }
    validate_link_incarnation_id(&request.link_incarnation_id)?;
    parse_identity_handoff_pubkey(&request.expected_pubkey)
}

fn valid_identity_handoff_client_marker(marker: Option<&str>) -> bool {
    marker == Some(IDENTITY_HANDOFF_PROTOCOL)
}

fn quota_exceeded<K>(
    cache: &moka::sync::Cache<K, Arc<std::sync::atomic::AtomicU32>>,
    key: K,
    limit: u32,
) -> bool
where
    K: std::hash::Hash + Eq + Send + Sync + Clone + 'static,
{
    let counter = cache.get_with(key, || Arc::new(std::sync::atomic::AtomicU32::new(0)));
    counter.fetch_add(1, Ordering::Relaxed) >= limit
}

fn handoff_id_quota_key(handoff_id: uuid::Uuid) -> [u8; 32] {
    Sha256::digest(handoff_id.as_bytes()).into()
}

fn rate_limited(retry_after: Duration) -> IdentityHandoffApiError {
    IdentityHandoffApiError::RateLimited {
        retry_after: retry_after.as_secs(),
        reason: "identity_handoff_rate_limited",
    }
}

/// Mint an invite code — `POST /api/invites`, NIP-98 signed by an owner/admin.
///
/// Returns the code, its expiry, and a shareable landing-page URL on the
/// tenant host.
pub async fn mint_invite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (tenant, pubkey) = authenticate(&state, &headers, "/api/invites", &body).await?;

    // Authz mirrors kind:9030 (add member): owner or admin only.
    let sender_hex = pubkey.to_hex();
    let member = state
        .db
        .get_relay_member(tenant.community(), &sender_hex)
        .await
        .map_err(|e| internal_error(&format!("invite mint role lookup: {e}")))?;
    let role = member.map(|m| m.role).unwrap_or_default();
    if role != "owner" && role != "admin" {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "only relay owners and admins can create invites",
        ));
    }

    let request: MintInviteRequest = if body.is_empty() {
        MintInviteRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|e| {
            api_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid invite JSON: {e}"),
            )
        })?
    };

    let (ttl, max_uses) = validate_mint_request(&request)?;

    // Mint a v2 opaque, database-backed invite.
    let invite = state
        .db
        .mint_relay_invite(tenant.community(), &sender_hex, ttl, max_uses)
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(message) => api_error(StatusCode::BAD_REQUEST, &message),
            error => internal_error(&format!("invite mint: {error}")),
        })?;

    // Same TLS-posture logic as nip98_expected_url: wss deployments get an
    // https landing page URL, ws dev/test deployments get http.
    let scheme = if state.config.relay_url.trim_start().starts_with("wss://") {
        "https"
    } else {
        "http"
    };

    tracing::info!(
        community = %tenant.community(),
        minted_by = %sender_hex,
        invite_id = %invite.invite_id,
        expires_at = %invite.expires_at,
        max_uses = ?invite.max_uses,
        "relay invite minted"
    );

    // expires_at as unix seconds for the response contract.
    let expires_at_unix = invite.expires_at.timestamp() as u64;

    Ok(Json(serde_json::json!({
        "code": invite.code,
        "expires_at": expires_at_unix,
        "max_uses": invite.max_uses,
        "uses_remaining": invite.uses_remaining,
        "url": format!("{scheme}://{}/invite/{}", tenant.host(), invite.code),
    })))
}

/// Mint a one-hour public-key-bound identity handoff.
pub async fn mint_identity_handoff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match mint_identity_handoff_inner(state, headers, body).await {
        Ok(response) => response.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn mint_identity_handoff_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> IdentityHandoffResult {
    let (tenant, signer) =
        authenticate_identity_handoff(&state, &headers, "/api/identity-handoffs", &body).await?;
    require_owner_or_admin(&state, &tenant, &signer).await?;

    let request: MintIdentityHandoffRequest = parse_identity_handoff_request(&body)?;
    let expected_pubkey = validate_identity_handoff_mint_request(&request)?;
    let quota_key = (tenant.community(), expected_pubkey.to_bytes());
    if quota_exceeded(
        &state.identity_handoff_mint_rate_limiter,
        quota_key,
        IDENTITY_HANDOFF_MINT_RATE_LIMIT,
    ) {
        return Err(rate_limited(IDENTITY_HANDOFF_MINT_RATE_WINDOW));
    }

    let outcome = state
        .db
        .mint_identity_handoff(
            tenant.community(),
            &expected_pubkey.to_hex(),
            &request.link_incarnation_id,
            &signer.to_hex(),
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(_) => {
                api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request")
            }
            error => internal_error(&format!("identity handoff mint: {error}")),
        })?;

    let buzz_db::relay_invite::MintIdentityHandoffOutcome::Minted(handoff) = outcome else {
        return Err(api_error(StatusCode::CONFLICT, "identity_handoff_incarnation_revoked").into());
    };
    let scheme = if state.config.relay_url.trim_start().starts_with("wss://") {
        "https"
    } else {
        "http"
    };

    tracing::info!(
        community = %tenant.community(),
        minted_by = %signer.to_hex(),
        handoff_id = %handoff.handoff_id,
        expires_at = %handoff.expires_at,
        "identity handoff minted"
    );

    Ok(Json(serde_json::json!({
        "protocol": IDENTITY_HANDOFF_PROTOCOL,
        "invite_url": format!("{scheme}://{}/invite#code={}", tenant.host(), handoff.code),
        "handoff_id": handoff.handoff_id.to_string(),
        "expires_at": handoff.expires_at.timestamp() as u64,
    })))
}

/// Read one identity handoff after reauthorizing its public-key and incarnation binding.
pub async fn identity_handoff_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match identity_handoff_status_inner(state, headers, body).await {
        Ok(response) => response.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn identity_handoff_status_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> IdentityHandoffResult {
    let (tenant, signer) =
        authenticate_identity_handoff(&state, &headers, "/api/identity-handoffs/status", &body)
            .await?;
    require_owner_or_admin(&state, &tenant, &signer).await?;

    let request: IdentityHandoffStatusRequest = parse_identity_handoff_request(&body)?;
    validate_link_incarnation_id(&request.link_incarnation_id)?;
    let expected_pubkey = parse_identity_handoff_pubkey(&request.expected_pubkey)?;
    if request.handoff_id.len() != 36 {
        return Err(api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request").into());
    }
    let handoff_id = uuid::Uuid::parse_str(&request.handoff_id)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request"))?;

    let pubkey_limited = quota_exceeded(
        &state.identity_handoff_status_rate_limiter,
        (
            tenant.community(),
            STATUS_KEY_DIMENSION,
            expected_pubkey.to_bytes(),
        ),
        IDENTITY_HANDOFF_STATUS_RATE_LIMIT,
    );
    let handoff_limited = quota_exceeded(
        &state.identity_handoff_status_rate_limiter,
        (
            tenant.community(),
            STATUS_HANDOFF_DIMENSION,
            handoff_id_quota_key(handoff_id),
        ),
        IDENTITY_HANDOFF_STATUS_RATE_LIMIT,
    );
    if pubkey_limited || handoff_limited {
        return Err(rate_limited(IDENTITY_HANDOFF_STATUS_RATE_WINDOW));
    }

    let state_value = state
        .db
        .identity_handoff_status(
            tenant.community(),
            handoff_id,
            &expected_pubkey.to_hex(),
            &request.link_incarnation_id,
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(_) => {
                api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request")
            }
            error => internal_error(&format!("identity handoff status: {error}")),
        })?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "identity_handoff_not_found"))?;

    let state_name = match state_value {
        buzz_db::relay_invite::IdentityHandoffState::Active => "active",
        buzz_db::relay_invite::IdentityHandoffState::Claimed => "claimed",
        buzz_db::relay_invite::IdentityHandoffState::Superseded => "superseded",
        buzz_db::relay_invite::IdentityHandoffState::Invalidated => "invalidated",
        buzz_db::relay_invite::IdentityHandoffState::Expired => "expired",
    };
    Ok(Json(serde_json::json!({ "state": state_name })))
}

/// Install a durable incarnation fence and invalidate its active identity handoffs.
pub async fn invalidate_identity_handoffs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match invalidate_identity_handoffs_inner(state, headers, body).await {
        Ok(response) => response.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn invalidate_identity_handoffs_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> IdentityHandoffResult {
    let (tenant, signer) =
        authenticate_identity_handoff(&state, &headers, "/api/identity-handoffs/invalidate", &body)
            .await?;
    require_owner_or_admin(&state, &tenant, &signer).await?;

    let request: InvalidateIdentityHandoffsRequest = parse_identity_handoff_request(&body)?;
    validate_link_incarnation_id(&request.link_incarnation_id)?;
    let expected_pubkey = parse_identity_handoff_pubkey(&request.expected_pubkey)?;
    let invalidation = state
        .db
        .invalidate_identity_handoffs(
            tenant.community(),
            &expected_pubkey.to_hex(),
            &request.link_incarnation_id,
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::InvalidData(_) => {
                api_error(StatusCode::BAD_REQUEST, "identity_handoff_invalid_request")
            }
            error => internal_error(&format!("identity handoff invalidation: {error}")),
        })?;

    tracing::info!(
        community = %tenant.community(),
        invalidated_by = %signer.to_hex(),
        expected_pubkey = %expected_pubkey.to_hex(),
        fence_created = invalidation.fence_created,
        invalidated_count = invalidation.invalidated_count,
        "identity handoffs invalidated"
    );

    Ok(Json(serde_json::json!({
        "fence_created": invalidation.fence_created,
        "invalidated_count": invalidation.invalidated_count,
    })))
}

/// Claim an invite code — `POST /api/invites/claim`, NIP-98 signed by the
/// *joining* pubkey. Exempt from the relay-membership gate by design.
///
/// Routing is by exact prefix: `v2.` codes go to the database-backed
/// redemption path; every other code goes to the v1 HMAC verifier. A `v2.`
/// code is never fallen back to v1 verification.
pub async fn claim_invite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    match claim_invite_inner(state, headers, body).await {
        Ok(response) => response.into_response(),
        Err(error) => error.into_response(),
    }
}

async fn claim_invite_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> IdentityHandoffResult {
    let (tenant, pubkey) = authenticate(&state, &headers, "/api/invites/claim", &body).await?;

    let parsed_request = serde_json::from_slice::<ClaimInviteRequest>(&body);
    let is_identity_handoff = parsed_request.as_ref().is_ok_and(|request| {
        request
            .code
            .starts_with(buzz_db::relay_invite::IDENTITY_HANDOFF_PREFIX)
    });
    if !is_identity_handoff && claim_rate_limited(&state, tenant.community(), &pubkey) {
        return Err(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many invite claim attempts, slow down",
        )
        .into());
    }

    let request = parsed_request
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid claim JSON: {e}")))?;

    let claimer_hex = pubkey.to_hex();
    let key = invite_token::derive_invite_key(&state.relay_keypair);

    // --- v3 identity-handoff path ---
    //
    // Route by exact prefix before the generic paths. A compatible client must
    // explicitly mark the payload and provide a fresh nonce; legacy clients
    // get a typed upgrade response without touching handoff or membership state.
    if request
        .code
        .starts_with(buzz_db::relay_invite::IDENTITY_HANDOFF_PREFIX)
    {
        if !valid_identity_handoff_client_marker(request.protocol.as_deref()) {
            return Err(api_error(
                StatusCode::UPGRADE_REQUIRED,
                "invite_client_upgrade_required",
            )
            .into());
        }
        if body.len() > IDENTITY_HANDOFF_MAX_BODY_BYTES
            || !buzz_db::relay_invite::validate_identity_handoff_code(&request.code)
        {
            return Err(api_error(StatusCode::FORBIDDEN, "invite_invalid").into());
        }
        require_identity_handoff_nonce(&headers)?;

        let token_hash = buzz_db::relay_invite::hash_identity_handoff_code(&request.code);
        let claimant_limited = quota_exceeded(
            &state.identity_handoff_claim_rate_limiter,
            (
                tenant.community(),
                CLAIM_CLAIMANT_DIMENSION,
                pubkey.to_bytes(),
            ),
            IDENTITY_HANDOFF_CLAIM_RATE_LIMIT,
        );
        let token_limited = quota_exceeded(
            &state.identity_handoff_claim_rate_limiter,
            (tenant.community(), CLAIM_TOKEN_DIMENSION, token_hash),
            IDENTITY_HANDOFF_CLAIM_RATE_LIMIT,
        );
        if claimant_limited || token_limited {
            return Err(rate_limited(IDENTITY_HANDOFF_CLAIM_RATE_WINDOW));
        }

        // Policy acceptance is verified before the DB is allowed to compare
        // identities or mutate either handoff or membership state.
        if let Some(policy) = &state.config.join_policy {
            let receipt = request
                .policy_receipt
                .as_deref()
                .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
            invite_token::verify_policy_acceptance(&key, receipt, &request.code, &policy.version)
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
        }

        let outcome = state
            .db
            .claim_identity_handoff(
                tenant.community(),
                &token_hash,
                &claimer_hex,
                state
                    .config
                    .join_policy
                    .as_ref()
                    .map(|policy| policy.version.as_str()),
            )
            .await
            .map_err(|error| internal_error(&format!("identity handoff claim: {error}")))?;

        return match outcome {
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::Claimed { membership } => {
                let added = matches!(
                    membership,
                    buzz_db::relay_invite::IdentityHandoffMembershipOutcome::Added
                );
                if added {
                    tracing::info!(
                        community = %tenant.community(),
                        member = %claimer_hex,
                        "relay member added via identity handoff"
                    );
                    if let Err(error) =
                        publish_nip43_member_added(&tenant, &state, &claimer_hex).await
                    {
                        tracing::warn!(
                            "failed to publish NIP-43 member-added delta after identity handoff: {error}"
                        );
                    }
                    if let Err(error) = publish_nip43_membership_list(&tenant, &state).await {
                        tracing::warn!(
                            "failed to publish NIP-43 membership list after identity handoff: {error}"
                        );
                    }
                }
                Ok(Json(serde_json::json!({
                    "status": if added { "joined" } else { "already_member" },
                    "community_id": tenant.community().to_string(),
                    "host": tenant.host(),
                    "role": "member",
                })))
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::AlreadyClaimed => {
                Err(api_error(StatusCode::CONFLICT, "invite_already_claimed").into())
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::IdentityMismatch => {
                Err(api_error(StatusCode::CONFLICT, "invite_identity_mismatch").into())
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::Expired => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_expired").into())
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::Superseded => {
                Err(api_error(StatusCode::CONFLICT, "invite_superseded").into())
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::Invalidated => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_invalidated").into())
            }
            buzz_db::relay_invite::IdentityHandoffClaimOutcome::Invalid => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_invalid").into())
            }
        };
    }

    // --- v2 database-backed path ---
    //
    // Route by exact prefix: v2. codes use the durable invite table. No
    // fallback to v1 HMAC verification for malformed v2 input.
    if request.code.starts_with(V2_PREFIX) {
        validate_v2_code(&request.code)
            .map_err(|_| api_error(StatusCode::FORBIDDEN, "invite_invalid"))?;

        // Join-policy receipt verification, same mechanism as v1: the receipt
        // is bound to the code string by SHA-256, so it works for v2 codes.
        if let Some(policy) = &state.config.join_policy {
            let receipt = request
                .policy_receipt
                .as_deref()
                .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
            invite_token::verify_policy_acceptance(&key, receipt, &request.code, &policy.version)
                .map_err(|_| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
        }

        let token_hash = hash_v2_code(&request.code);
        let outcome = state
            .db
            .claim_relay_invite(
                tenant.community(),
                &token_hash,
                &claimer_hex,
                state
                    .config
                    .join_policy
                    .as_ref()
                    .map(|policy| policy.version.as_str()),
            )
            .await
            .map_err(|e| internal_error(&format!("v2 invite claim: {e}")))?;

        return match outcome {
            buzz_db::relay_invite::ClaimOutcome::Joined { .. } => {
                tracing::info!(
                    community = %tenant.community(),
                    member = %claimer_hex,
                    "relay member added via v2 invite"
                );
                // NIP-43 side effects only on Joined, never on other outcomes.
                if let Err(e) = publish_nip43_member_added(&tenant, &state, &claimer_hex).await {
                    tracing::warn!(
                        "failed to publish NIP-43 member-added delta after v2 claim: {e}"
                    );
                }
                if let Err(e) = publish_nip43_membership_list(&tenant, &state).await {
                    tracing::warn!("failed to publish NIP-43 membership list after v2 claim: {e}");
                }
                Ok(Json(serde_json::json!({
                    "status": "joined",
                    "community_id": tenant.community().to_string(),
                    "host": tenant.host(),
                    "role": "member",
                })))
            }
            buzz_db::relay_invite::ClaimOutcome::AlreadyMember { .. } => {
                Ok(Json(serde_json::json!({
                    "status": "already_member",
                    "community_id": tenant.community().to_string(),
                    "host": tenant.host(),
                    "role": "member",
                })))
            }
            buzz_db::relay_invite::ClaimOutcome::Expired => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_expired").into())
            }
            buzz_db::relay_invite::ClaimOutcome::Exhausted => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_exhausted").into())
            }
            buzz_db::relay_invite::ClaimOutcome::Invalid => {
                Err(api_error(StatusCode::FORBIDDEN, "invite_invalid").into())
            }
        };
    }

    // --- v1 HMAC path (stateless tokens, drain window) ---
    let payload = invite_token::verify_invite(&key, tenant.community(), &request.code).map_err(
        |e| match e {
            // Expired is post-MAC: revealing it helps the UX without helping a forger.
            invite_token::InviteError::Expired => {
                api_error(StatusCode::FORBIDDEN, "invite_expired")
            }
            // Everything else stays coarse so the endpoint is a poor oracle.
            _ => api_error(StatusCode::FORBIDDEN, "invite_invalid"),
        },
    )?;

    if let Some(policy) = &state.config.join_policy {
        let receipt = request
            .policy_receipt
            .as_deref()
            .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
        invite_token::verify_policy_acceptance(&key, receipt, &request.code, &policy.version)
            .map_err(|_| api_error(StatusCode::FORBIDDEN, "join_policy_required"))?;
    }

    let was_inserted = state
        .db
        .claim_relay_membership(
            tenant.community(),
            &claimer_hex,
            &payload.r,
            state
                .config
                .join_policy
                .as_ref()
                .map(|policy| policy.version.as_str()),
        )
        .await
        .map_err(|e| internal_error(&format!("invite claim insert: {e}")))?;

    if was_inserted {
        tracing::info!(
            community = %tenant.community(),
            member = %claimer_hex,
            "relay member added via invite"
        );
        if let Err(e) = publish_nip43_member_added(&tenant, &state, &claimer_hex).await {
            tracing::warn!("failed to publish NIP-43 member-added delta after claim: {e}");
        }
        if let Err(e) = publish_nip43_membership_list(&tenant, &state).await {
            tracing::warn!("failed to publish NIP-43 membership list after claim: {e}");
        }
    }

    Ok(Json(serde_json::json!({
        "status": if was_inserted { "joined" } else { "already_member" },
        "community_id": tenant.community().to_string(),
        "host": tenant.host(),
        "role": payload.r,
    })))
}

/// Fixed-window rate limit on claim attempts, keyed by community and claimer
/// pubkey so traffic for one tenant cannot consume another tenant's allowance.
///
/// Entries expire after one window and the cache has a hard capacity. Both are
/// important because a pre-membership caller can cheaply create fresh Nostr
/// keypairs; retaining one immortal entry per key would make the limiter itself
/// an unbounded-memory denial-of-service vector.
fn claim_rate_limited(
    state: &AppState,
    community: buzz_core::tenant::CommunityId,
    pubkey: &nostr::PublicKey,
) -> bool {
    claim_key_rate_limited(
        &state.invite_claim_rate_limiter,
        (community, pubkey.to_bytes()),
    )
}

fn claim_key_rate_limited(
    cache: &moka::sync::Cache<crate::state::ScopedPubkeyKey, Arc<std::sync::atomic::AtomicU32>>,
    key: crate::state::ScopedPubkeyKey,
) -> bool {
    let counter = cache.get_with(key, || Arc::new(std::sync::atomic::AtomicU32::new(0)));
    counter.fetch_add(1, Ordering::Relaxed) >= CLAIM_RATE_LIMIT
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        claim_key_rate_limited, CLAIM_RATE_LIMIT, IDENTITY_HANDOFF_CLAIM_RATE_LIMIT,
        IDENTITY_HANDOFF_MINT_RATE_LIMIT, IDENTITY_HANDOFF_STATUS_RATE_LIMIT, MAX_INVITE_USES,
        MIN_INVITE_TTL_SECS,
    };
    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use hmac::{Hmac, KeyInit, Mac};
    use nostr::{EventBuilder, EventId, Keys, Kind, Tag};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::sync::Mutex;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::invite_token::{derive_invite_key, InvitePayload, MAX_INVITE_TTL_SECS};

    use crate::router::build_router;
    use crate::state::AppState;

    struct AlwaysFreshReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
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

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    fn claim_cache(
        capacity: u64,
        ttl: Duration,
    ) -> moka::sync::Cache<crate::state::ScopedPubkeyKey, Arc<std::sync::atomic::AtomicU32>> {
        moka::sync::Cache::builder()
            .max_capacity(capacity)
            .time_to_live(ttl)
            .build()
    }

    #[test]
    fn claim_limiter_rejects_after_limit() {
        let cache = claim_cache(100, Duration::from_secs(60));
        let key = (buzz_core::CommunityId::from_uuid(Uuid::nil()), [7; 32]);

        for _ in 0..CLAIM_RATE_LIMIT {
            assert!(!claim_key_rate_limited(&cache, key));
        }
        assert!(claim_key_rate_limited(&cache, key));
    }

    #[test]
    fn claim_limiter_expires_entries() {
        let cache = claim_cache(100, Duration::from_millis(10));
        let key = (buzz_core::CommunityId::from_uuid(Uuid::nil()), [8; 32]);
        assert!(!claim_key_rate_limited(&cache, key));
        assert!(cache.get(&key).is_some());

        std::thread::sleep(Duration::from_millis(25));
        cache.run_pending_tasks();

        assert!(cache.get(&key).is_none());
        assert!(!claim_key_rate_limited(&cache, key));
    }

    #[test]
    fn claim_limiter_isolates_communities_for_same_pubkey() {
        let cache = claim_cache(100, Duration::from_secs(60));
        let pubkey = [9; 32];
        let community_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));

        for _ in 0..CLAIM_RATE_LIMIT {
            assert!(!claim_key_rate_limited(&cache, (community_a, pubkey)));
        }
        assert!(claim_key_rate_limited(&cache, (community_a, pubkey)));
        assert!(!claim_key_rate_limited(&cache, (community_b, pubkey)));
    }

    #[test]
    fn claim_limiter_bounds_distinct_pubkeys() {
        let capacity = 10;
        let cache = claim_cache(capacity, Duration::from_secs(60));
        for id in 0..100_u64 {
            let mut pubkey = [0; 32];
            pubkey[..8].copy_from_slice(&id.to_le_bytes());
            let key = (buzz_core::CommunityId::from_uuid(Uuid::nil()), pubkey);
            assert!(!claim_key_rate_limited(&cache, key));
        }
        cache.run_pending_tasks();

        assert!(cache.entry_count() <= capacity);
    }

    #[test]
    fn identity_handoff_request_contracts_are_strict_and_bounded() {
        let pubkey = "ab".repeat(32);
        let incarnation = Uuid::new_v4().to_string();

        let mint = serde_json::json!({
            "expected_pubkey": pubkey,
            "link_incarnation_id": incarnation,
            "ttl_secs": 3600,
        });
        let parsed: super::MintIdentityHandoffRequest =
            serde_json::from_value(mint).expect("valid v3 mint request");
        super::validate_identity_handoff_mint_request(&parsed).expect("valid v3 mint contract");

        for invalid in [
            serde_json::json!({
                "expected_pubkey": "ab".repeat(32),
                "link_incarnation_id": Uuid::new_v4().to_string(),
                "ttl_secs": 3599,
            }),
            serde_json::json!({
                "expected_pubkey": "ab".repeat(32),
                "link_incarnation_id": Uuid::new_v4().to_string(),
                "ttl_secs": 3600,
                "unexpected": true,
            }),
            serde_json::json!({
                "expected_pubkey": "not-a-pubkey",
                "link_incarnation_id": Uuid::new_v4().to_string(),
                "ttl_secs": 3600,
            }),
        ] {
            let parsed = serde_json::from_value::<super::MintIdentityHandoffRequest>(invalid);
            assert!(
                !parsed.as_ref().is_ok_and(|request| {
                    super::validate_identity_handoff_mint_request(request).is_ok()
                }),
                "invalid v3 mint request was accepted"
            );
        }
    }

    #[test]
    fn identity_handoff_client_marker_is_exact() {
        assert!(super::valid_identity_handoff_client_marker(Some(
            "identity-handoff-v3"
        )));
        for marker in [
            None,
            Some(""),
            Some("identity-handoff-v2"),
            Some("IDENTITY-HANDOFF-V3"),
        ] {
            assert!(!super::valid_identity_handoff_client_marker(marker));
        }
    }

    fn nip98_auth_header_with_nonce(
        keys: &Keys,
        url: &str,
        body: &[u8],
        include_nonce: bool,
    ) -> String {
        let hash: [u8; 32] = Sha256::digest(body).into();
        let mut tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", "POST"]).expect("method tag"),
            Tag::parse(["payload", hex::encode(hash).as_str()]).expect("payload tag"),
        ];
        if include_nonce {
            tags.push(
                Tag::parse(["nonce", Uuid::new_v4().to_string().as_str()]).expect("nonce tag"),
            );
        }
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
    }

    fn nip98_auth_header(keys: &Keys, url: &str, body: &[u8]) -> String {
        nip98_auth_header_with_nonce(keys, url, body, true)
    }

    /// Build a closed-relay (`require_relay_membership = true`) test state with
    /// a fresh community on `host`; returns `None` when Postgres is unavailable.
    async fn invite_test_state(host: &str) -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_string());
        config.database_url = database_url.clone();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_url = format!("wss://{host}");
        // The claim route must work on relays where membership is enforced —
        // that is the entire point of an invite.
        config.require_relay_membership = true;

        let pool = sqlx::PgPool::connect(&database_url).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community(host).await.ok()?;

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
        let (mut state, _audit_shutdown) = AppState::new(
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

    async fn post_json(
        state: Arc<AppState>,
        host: &str,
        path: &str,
        keys: &Keys,
        body: String,
    ) -> axum::response::Response {
        let url = format!("https://{host}{path}");
        let auth = nip98_auth_header(keys, &url, body.as_bytes());
        build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::HOST, host)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn post_json_without_nonce(
        state: Arc<AppState>,
        host: &str,
        path: &str,
        keys: &Keys,
        body: String,
    ) -> axum::response::Response {
        let url = format!("https://{host}{path}");
        let auth = nip98_auth_header_with_nonce(keys, &url, body.as_bytes(), false);
        build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::HOST, host)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn read_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response JSON")
    }

    async fn mint_code(state: Arc<AppState>, host: &str, owner: &Keys, request: Value) -> String {
        let response = post_json(state, host, "/api/invites", owner, request.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        read_json(response)
            .await
            .get("code")
            .and_then(Value::as_str)
            .expect("minted code")
            .to_string()
    }

    async fn mint_identity_handoff(
        state: Arc<AppState>,
        host: &str,
        owner: &Keys,
        expected_pubkey: &str,
        incarnation: &str,
    ) -> Value {
        let response = post_json(
            state,
            host,
            "/api/identity-handoffs",
            owner,
            serde_json::json!({
                "expected_pubkey": expected_pubkey,
                "link_incarnation_id": incarnation,
                "ttl_secs": 3600,
            })
            .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        read_json(response).await
    }

    fn identity_handoff_code(response: &Value) -> String {
        response
            .get("invite_url")
            .and_then(Value::as_str)
            .and_then(|url| url.split_once("#code="))
            .map(|(_, code)| code.to_owned())
            .expect("fragment-held identity handoff code")
    }

    async fn event_count(state: &AppState, community: buzz_core::CommunityId, kind: i32) -> i64 {
        state
            .db
            .count_events(&buzz_db::EventQuery {
                kinds: Some(vec![kind]),
                global_only: true,
                ..buzz_db::EventQuery::for_community(community)
            })
            .await
            .expect("count side-effect events")
    }

    #[test]
    fn mint_request_deserialization_is_strict() {
        for valid in [
            serde_json::json!({}),
            serde_json::json!({ "max_uses": null }),
            serde_json::json!({ "max_uses": 1 }),
            serde_json::json!({ "max_uses": MAX_INVITE_USES }),
        ] {
            serde_json::from_value::<super::MintInviteRequest>(valid).expect("valid request");
        }

        for invalid in [
            serde_json::json!({ "max_uses": 1.5 }),
            serde_json::json!({ "max_uses": "10" }),
            serde_json::json!({ "ttl_secs": -1 }),
            serde_json::json!({ "ttl_secs": 1.5 }),
            serde_json::json!({ "ttl_secs": "3600" }),
        ] {
            assert!(
                serde_json::from_value::<super::MintInviteRequest>(invalid.clone()).is_err(),
                "accepted wrong JSON type: {invalid}"
            );
        }
    }

    #[test]
    fn mint_request_validation_enforces_bounds_without_a_database() {
        use super::validate_mint_request;

        for (request, expected) in [
            (
                super::MintInviteRequest::default(),
                (crate::invite_token::DEFAULT_INVITE_TTL_SECS, None),
            ),
            (
                super::MintInviteRequest {
                    ttl_secs: Some(MIN_INVITE_TTL_SECS),
                    max_uses: Some(1),
                },
                (MIN_INVITE_TTL_SECS, Some(1)),
            ),
            (
                super::MintInviteRequest {
                    ttl_secs: Some(MAX_INVITE_TTL_SECS),
                    max_uses: Some(MAX_INVITE_USES),
                },
                (MAX_INVITE_TTL_SECS, Some(MAX_INVITE_USES)),
            ),
        ] {
            assert_eq!(
                validate_mint_request(&request).expect("valid request"),
                expected
            );
        }

        for request in [
            super::MintInviteRequest {
                ttl_secs: None,
                max_uses: Some(0),
            },
            super::MintInviteRequest {
                ttl_secs: None,
                max_uses: Some(-1),
            },
            super::MintInviteRequest {
                ttl_secs: None,
                max_uses: Some(MAX_INVITE_USES + 1),
            },
            super::MintInviteRequest {
                ttl_secs: Some(MIN_INVITE_TTL_SECS - 1),
                max_uses: None,
            },
            super::MintInviteRequest {
                ttl_secs: Some(MAX_INVITE_TTL_SECS + 1),
                max_uses: None,
            },
        ] {
            assert_eq!(
                validate_mint_request(&request)
                    .expect_err("invalid request")
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mint_validates_max_uses_and_ttl_bounds() {
        let host = format!("invites-validation-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        for body in [
            serde_json::json!({ "max_uses": 0 }),
            serde_json::json!({ "max_uses": -1 }),
            serde_json::json!({ "max_uses": MAX_INVITE_USES + 1 }),
            serde_json::json!({ "ttl_secs": MIN_INVITE_TTL_SECS - 1 }),
            serde_json::json!({ "ttl_secs": MAX_INVITE_TTL_SECS + 1 }),
        ] {
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites",
                &owner,
                body.to_string(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        }

        for body in [
            serde_json::json!({}),
            serde_json::json!({ "max_uses": null }),
            serde_json::json!({ "max_uses": 1 }),
            serde_json::json!({ "max_uses": MAX_INVITE_USES }),
            serde_json::json!({ "ttl_secs": MIN_INVITE_TTL_SECS }),
            serde_json::json!({ "ttl_secs": MAX_INVITE_TTL_SECS }),
        ] {
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites",
                &owner,
                body.to_string(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{body}");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_routes_enforce_auth_binding_and_terminal_contracts() {
        let host = format!("identity-handoff-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let admin = Keys::generate();
        let member = Keys::generate();
        let expected = Keys::generate();
        let mismatch = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");
        state
            .db
            .add_relay_member(community.id, &admin.public_key().to_hex(), "admin", None)
            .await
            .expect("seed admin");
        state
            .db
            .add_relay_member(community.id, &member.public_key().to_hex(), "member", None)
            .await
            .expect("seed member");

        let incarnation = Uuid::new_v4().to_string();
        let mint_body = serde_json::json!({
            "expected_pubkey": expected.public_key().to_hex(),
            "link_incarnation_id": incarnation,
            "ttl_secs": 3600,
        })
        .to_string();
        let unauthenticated = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/identity-handoffs")
                    .header(header::HOST, &host)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(mint_body.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        let no_nonce = post_json_without_nonce(
            state.clone(),
            &host,
            "/api/identity-handoffs",
            &owner,
            mint_body.clone(),
        )
        .await;
        assert_eq!(no_nonce.status(), StatusCode::UNAUTHORIZED);

        for unauthorized in [&member, &mismatch] {
            let response = post_json(
                state.clone(),
                &host,
                "/api/identity-handoffs",
                unauthorized,
                mint_body.clone(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }

        let mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &expected.public_key().to_hex(),
            &incarnation,
        )
        .await;
        assert_eq!(
            mint.get("protocol").and_then(Value::as_str),
            Some("identity-handoff-v3")
        );
        assert!(mint.get("code").is_none(), "standalone code leaked: {mint}");
        let invite_url = mint
            .get("invite_url")
            .and_then(Value::as_str)
            .expect("invite URL");
        assert!(invite_url.starts_with(&format!("https://{host}/invite#code=v3.")));
        let code = identity_handoff_code(&mint);
        let handoff_id = mint
            .get("handoff_id")
            .and_then(Value::as_str)
            .expect("handoff id")
            .to_owned();

        let status_body = serde_json::json!({
            "handoff_id": handoff_id,
            "expected_pubkey": expected.public_key().to_hex(),
            "link_incarnation_id": incarnation,
        })
        .to_string();
        let response = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/status",
            &admin,
            status_body.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let status = read_json(response).await;
        assert_eq!(status.get("state").and_then(Value::as_str), Some("active"));
        assert!(status.get("invite_url").is_none());
        assert!(status.get("code").is_none());

        let legacy_claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &expected,
            serde_json::json!({ "code": code }).to_string(),
        )
        .await;
        assert_eq!(legacy_claim.status(), StatusCode::UPGRADE_REQUIRED);
        assert_eq!(
            read_json(legacy_claim)
                .await
                .get("error")
                .and_then(Value::as_str),
            Some("invite_client_upgrade_required")
        );

        let mismatch_claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &mismatch,
            serde_json::json!({
                "code": code,
                "protocol": "identity-handoff-v3",
            })
            .to_string(),
        )
        .await;
        assert_eq!(mismatch_claim.status(), StatusCode::CONFLICT);
        let mismatch_error = read_json(mismatch_claim).await;
        assert_eq!(
            mismatch_error.get("error").and_then(Value::as_str),
            Some("invite_identity_mismatch")
        );
        assert!(
            !mismatch_error
                .to_string()
                .contains(&expected.public_key().to_hex()),
            "mismatch response disclosed the bound key"
        );
        assert!(!state
            .db
            .is_relay_member(community.id, &mismatch.public_key().to_hex())
            .await
            .expect("mismatch membership"));

        let matching_claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &expected,
            serde_json::json!({
                "code": code,
                "protocol": "identity-handoff-v3",
            })
            .to_string(),
        )
        .await;
        assert_eq!(matching_claim.status(), StatusCode::OK);
        assert_eq!(
            read_json(matching_claim)
                .await
                .get("status")
                .and_then(Value::as_str),
            Some("joined")
        );

        let response = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/status",
            &admin,
            status_body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            read_json(response)
                .await
                .get("state")
                .and_then(Value::as_str),
            Some("claimed")
        );

        let existing = Keys::generate();
        state
            .db
            .add_relay_member(
                community.id,
                &existing.public_key().to_hex(),
                "member",
                None,
            )
            .await
            .expect("seed pre-existing member");
        let existing_incarnation = Uuid::new_v4().to_string();
        let existing_mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &existing.public_key().to_hex(),
            &existing_incarnation,
        )
        .await;
        let existing_claim = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &existing,
            serde_json::json!({
                "code": identity_handoff_code(&existing_mint),
                "protocol": "identity-handoff-v3",
            })
            .to_string(),
        )
        .await;
        assert_eq!(existing_claim.status(), StatusCode::OK);
        assert_eq!(read_json(existing_claim).await["status"], "already_member");
        let existing_status = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/status",
            &admin,
            serde_json::json!({
                "handoff_id": existing_mint["handoff_id"],
                "expected_pubkey": existing.public_key().to_hex(),
                "link_incarnation_id": existing_incarnation,
            })
            .to_string(),
        )
        .await;
        assert_eq!(existing_status.status(), StatusCode::OK);
        assert_eq!(read_json(existing_status).await["state"], "claimed");

        let pending = Keys::generate();
        let pending_incarnation = Uuid::new_v4().to_string();
        let pending_mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &pending.public_key().to_hex(),
            &pending_incarnation,
        )
        .await;
        let pending_code = identity_handoff_code(&pending_mint);
        let invalidate_body = serde_json::json!({
            "expected_pubkey": pending.public_key().to_hex(),
            "link_incarnation_id": pending_incarnation,
        })
        .to_string();
        let first = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/invalidate",
            &owner,
            invalidate_body.clone(),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first = read_json(first).await;
        assert_eq!(first["fence_created"], true);
        assert_eq!(first["invalidated_count"], 1);

        let second = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/invalidate",
            &owner,
            invalidate_body,
        )
        .await;
        assert_eq!(second.status(), StatusCode::OK);
        let second = read_json(second).await;
        assert_eq!(second["fence_created"], false);
        assert_eq!(second["invalidated_count"], 0);

        let invalidated_claim = post_json(
            state,
            &host,
            "/api/invites/claim",
            &pending,
            serde_json::json!({
                "code": pending_code,
                "protocol": "identity-handoff-v3",
            })
            .to_string(),
        )
        .await;
        assert_eq!(invalidated_claim.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            read_json(invalidated_claim)
                .await
                .get("error")
                .and_then(Value::as_str),
            Some("invite_invalidated")
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_policy_receipt_precedes_identity_comparison() {
        let host = format!(
            "identity-handoff-policy-{}.example",
            Uuid::new_v4().simple()
        );
        let owner = Keys::generate();
        let expected = Keys::generate();
        let mismatch = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let mut state_inner = (*state).clone();
        let mut config = state_inner.config.as_ref().clone();
        let policy_version = "a".repeat(64);
        config.join_policy = Some(crate::config::JoinPolicyConfig {
            terms_markdown: Some("# Terms".to_owned()),
            privacy_markdown: None,
            age_attestation_required: false,
            version: policy_version.clone(),
        });
        state_inner.config = Arc::new(config);
        let state = Arc::new(state_inner);
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        let incarnation = Uuid::new_v4().to_string();
        let mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &expected.public_key().to_hex(),
            &incarnation,
        )
        .await;
        let code = identity_handoff_code(&mint);
        let handoff_id = mint["handoff_id"].as_str().expect("handoff id");
        let key = crate::invite_token::derive_invite_key(&state.relay_keypair);
        let wrong_code_receipt =
            crate::invite_token::mint_policy_acceptance(&key, "v3.wrong-code", &policy_version);
        let stale_receipt =
            crate::invite_token::mint_policy_acceptance(&key, &code, &"b".repeat(64));

        for receipt in [None, Some(wrong_code_receipt), Some(stale_receipt)] {
            let mut body = serde_json::json!({
                "code": code,
                "protocol": "identity-handoff-v3",
            });
            if let Some(receipt) = receipt {
                body["policy_receipt"] = Value::String(receipt);
            }
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites/claim",
                &mismatch,
                body.to_string(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                read_json(response)
                    .await
                    .get("error")
                    .and_then(Value::as_str),
                Some("join_policy_required")
            );
        }

        let status = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/status",
            &owner,
            serde_json::json!({
                "handoff_id": handoff_id,
                "expected_pubkey": expected.public_key().to_hex(),
                "link_incarnation_id": incarnation,
            })
            .to_string(),
        )
        .await;
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(read_json(status).await["state"], "active");

        let valid_receipt =
            crate::invite_token::mint_policy_acceptance(&key, &code, &policy_version);
        let response = post_json(
            state,
            &host,
            "/api/invites/claim",
            &mismatch,
            serde_json::json!({
                "code": code,
                "protocol": "identity-handoff-v3",
                "policy_receipt": valid_receipt,
            })
            .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            read_json(response).await["error"],
            "invite_identity_mismatch"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_endpoint_quotas_are_typed_and_dimensioned() {
        let host = format!("identity-handoff-quota-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let expected = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        let incarnation = Uuid::new_v4().to_string();
        for _ in 0..IDENTITY_HANDOFF_MINT_RATE_LIMIT {
            mint_identity_handoff(
                state.clone(),
                &host,
                &owner,
                &expected.public_key().to_hex(),
                &incarnation,
            )
            .await;
        }
        let mint_limited = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs",
            &owner,
            serde_json::json!({
                "expected_pubkey": expected.public_key().to_hex(),
                "link_incarnation_id": incarnation,
                "ttl_secs": 3600,
            })
            .to_string(),
        )
        .await;
        assert_eq!(mint_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            mint_limited
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("3600")
        );
        assert_eq!(
            read_json(mint_limited).await["error"],
            "identity_handoff_rate_limited"
        );

        let status_expected = Keys::generate();
        let status_incarnation = Uuid::new_v4().to_string();
        let status_mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &status_expected.public_key().to_hex(),
            &status_incarnation,
        )
        .await;
        let status_body = serde_json::json!({
            "handoff_id": status_mint["handoff_id"],
            "expected_pubkey": status_expected.public_key().to_hex(),
            "link_incarnation_id": status_incarnation,
        })
        .to_string();
        for _ in 0..IDENTITY_HANDOFF_STATUS_RATE_LIMIT {
            let response = post_json(
                state.clone(),
                &host,
                "/api/identity-handoffs/status",
                &owner,
                status_body.clone(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let status_limited = post_json(
            state.clone(),
            &host,
            "/api/identity-handoffs/status",
            &owner,
            status_body,
        )
        .await;
        assert_eq!(status_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            status_limited
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("60")
        );

        let token_expected = Keys::generate();
        let token_mint = mint_identity_handoff(
            state.clone(),
            &host,
            &owner,
            &token_expected.public_key().to_hex(),
            &Uuid::new_v4().to_string(),
        )
        .await;
        let token_code = identity_handoff_code(&token_mint);
        for _ in 0..IDENTITY_HANDOFF_CLAIM_RATE_LIMIT {
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites/claim",
                &Keys::generate(),
                serde_json::json!({
                    "code": token_code,
                    "protocol": "identity-handoff-v3",
                })
                .to_string(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
        let token_limited = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &Keys::generate(),
            serde_json::json!({
                "code": token_code,
                "protocol": "identity-handoff-v3",
            })
            .to_string(),
        )
        .await;
        assert_eq!(token_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            token_limited
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("600")
        );

        let repeated_claimant = Keys::generate();
        for attempt in 0..=IDENTITY_HANDOFF_CLAIM_RATE_LIMIT {
            let bound = Keys::generate();
            let mint = mint_identity_handoff(
                state.clone(),
                &host,
                &owner,
                &bound.public_key().to_hex(),
                &Uuid::new_v4().to_string(),
            )
            .await;
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites/claim",
                &repeated_claimant,
                serde_json::json!({
                    "code": identity_handoff_code(&mint),
                    "protocol": "identity-handoff-v3",
                })
                .to_string(),
            )
            .await;
            if attempt < IDENTITY_HANDOFF_CLAIM_RATE_LIMIT {
                assert_eq!(response.status(), StatusCode::CONFLICT);
            } else {
                assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(
                    read_json(response).await["error"],
                    "identity_handoff_rate_limited"
                );
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn malformed_and_unknown_v2_codes_are_forbidden_without_v1_fallback() {
        let host = format!("invites-v2-invalid-{}.example", Uuid::new_v4().simple());
        let joiner = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let unknown = format!("v2.{}", URL_SAFE_NO_PAD.encode([9_u8; 32]));

        for code in [
            "v2.".to_string(),
            "v2.not-base64!".to_string(),
            format!("v2.{}", URL_SAFE_NO_PAD.encode([9_u8; 31])),
            unknown,
        ] {
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites/claim",
                &joiner,
                serde_json::json!({ "code": code }).to_string(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{code}");
            assert_eq!(
                read_json(response)
                    .await
                    .get("error")
                    .and_then(Value::as_str),
                Some("invite_invalid")
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bounded_v2_claims_publish_side_effects_only_for_joined() {
        let host = format!(
            "invites-v2-side-effects-{}.example",
            Uuid::new_v4().simple()
        );
        let owner = Keys::generate();
        let first = Keys::generate();
        let second = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");
        let before_delta_count = event_count(
            &state,
            community.id,
            buzz_core::kind::KIND_NIP43_MEMBER_ADDED as i32,
        )
        .await;
        let before_list_count = event_count(
            &state,
            community.id,
            buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
        )
        .await;
        let code = mint_code(
            state.clone(),
            &host,
            &owner,
            serde_json::json!({ "max_uses": 1 }),
        )
        .await;
        let claim_body = serde_json::json!({ "code": code }).to_string();

        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &first,
            claim_body.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            read_json(response)
                .await
                .get("status")
                .and_then(Value::as_str),
            Some("joined")
        );
        let delta_count = event_count(
            &state,
            community.id,
            buzz_core::kind::KIND_NIP43_MEMBER_ADDED as i32,
        )
        .await;
        let list_count = event_count(
            &state,
            community.id,
            buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
        )
        .await;
        assert_eq!(delta_count, before_delta_count + 1);
        assert_eq!(list_count, before_list_count + 1);

        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &first,
            claim_body.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            read_json(response)
                .await
                .get("status")
                .and_then(Value::as_str),
            Some("already_member")
        );
        assert_eq!(
            event_count(
                &state,
                community.id,
                buzz_core::kind::KIND_NIP43_MEMBER_ADDED as i32,
            )
            .await,
            delta_count
        );
        assert_eq!(
            event_count(
                &state,
                community.id,
                buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
            )
            .await,
            list_count
        );

        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &second,
            claim_body,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            read_json(response)
                .await
                .get("error")
                .and_then(Value::as_str),
            Some("invite_exhausted")
        );
        assert_eq!(
            event_count(
                &state,
                community.id,
                buzz_core::kind::KIND_NIP43_MEMBER_ADDED as i32,
            )
            .await,
            delta_count
        );
        assert_eq!(
            event_count(
                &state,
                community.id,
                buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
            )
            .await,
            list_count
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_mints_and_new_pubkey_claims() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let joiner = Keys::generate();
        let Some(state) = invite_test_state(&host).await else {
            return;
        };
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        let community_id = community.id;
        state
            .db
            .add_relay_member(community_id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        // Mint.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites",
            &owner,
            "{}".to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        let code = json.get("code").and_then(Value::as_str).expect("code");
        let url = json.get("url").and_then(Value::as_str).expect("url");
        assert!(url.contains("/invite/"), "unexpected url: {url}");

        // Claim on a closed relay by a pubkey that is not yet a member.
        let claim_body = serde_json::json!({ "code": code }).to_string();
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            claim_body.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(json.get("status").and_then(Value::as_str), Some("joined"));
        assert_eq!(json.get("role").and_then(Value::as_str), Some("member"));

        let member = state
            .db
            .get_relay_member(community_id, &joiner.public_key().to_hex())
            .await
            .expect("member lookup")
            .expect("joiner is now a member");
        assert_eq!(member.role, "member");

        // Second claim is idempotent.
        let response = post_json(state, &host, "/api/invites/claim", &joiner, claim_body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(
            json.get("status").and_then(Value::as_str),
            Some("already_member")
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn join_policy_gate_end_to_end() {
        let host = format!("invites-policy-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let joiner = Keys::generate();
        let Some(state) = invite_test_state(&host).await else {
            return;
        };
        // Force the join policy on regardless of env.
        let mut state_inner = (*state).clone();
        let mut config = state_inner.config.as_ref().clone();
        config.join_policy = Some(crate::config::JoinPolicyConfig {
            terms_markdown: Some("# Terms".to_string()),
            privacy_markdown: Some("# Privacy".to_string()),
            age_attestation_required: true,
            version: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        });
        state_inner.config = Arc::new(config);
        let state = Arc::new(state_inner);

        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        // Mint an invite.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites",
            &owner,
            "{}".to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        let code = json
            .get("code")
            .and_then(Value::as_str)
            .expect("code")
            .to_string();

        // 1. Claim WITHOUT receipt -> 403 (checkbox bypass).
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            serde_json::json!({ "code": code }).to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "no-receipt claim must fail"
        );

        // 2. Forged receipt (wrong key) -> 403.
        let forged = crate::invite_token::mint_policy_acceptance(
            &[9u8; 32],
            &code,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            serde_json::json!({ "code": code, "policy_receipt": forged }).to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "forged receipt must fail"
        );

        // 3. Receipt bound to a DIFFERENT invite code -> 403.
        let key = crate::invite_token::derive_invite_key(&state.relay_keypair);
        let other = crate::invite_token::mint_policy_acceptance(
            &key,
            "some-other-code",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            serde_json::json!({ "code": code, "policy_receipt": other }).to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "cross-invite receipt must fail"
        );

        // 4. Receipt for a STALE policy version -> 403.
        let stale = crate::invite_token::mint_policy_acceptance(
            &key,
            &code,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            serde_json::json!({ "code": code, "policy_receipt": stale }).to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "stale-version receipt must fail"
        );

        // 5. accept-policy without age confirmation -> 400.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/accept-policy",
            &joiner,
            serde_json::json!({ "code": code, "policy_version": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "age_confirmed": false })
                .to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "age not confirmed must be rejected when required"
        );

        // 5b. accept-policy with stale version -> 400.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/accept-policy",
            &joiner,
            serde_json::json!({ "code": code, "policy_version": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "age_confirmed": true })
                .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // 6. Legit flow: accept-policy -> receipt -> claim OK.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/accept-policy",
            &joiner,
            serde_json::json!({ "code": code, "policy_version": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "age_confirmed": true })
                .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let receipt = read_json(response)
            .await
            .get("receipt")
            .and_then(Value::as_str)
            .expect("receipt")
            .to_string();

        let response = post_json(
            state.clone(),
            &host,
            "/api/invites/claim",
            &joiner,
            serde_json::json!({ "code": code, "policy_receipt": receipt }).to_string(),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "legit receipt claim must succeed"
        );
        let json = read_json(response).await;
        assert_eq!(json.get("status").and_then(Value::as_str), Some("joined"));

        let member = state
            .db
            .get_relay_member(community.id, &joiner.public_key().to_hex())
            .await
            .expect("member lookup")
            .expect("joiner is now a member");
        assert_eq!(member.role, "member");
        assert!(
            state
                .db
                .has_join_policy_acceptance(
                    community.id,
                    &joiner.public_key().to_hex(),
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .await
                .expect("policy acceptance lookup"),
            "accepted policy version must be persisted",
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn non_admin_cannot_mint() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let member = Keys::generate();
        let outsider = Keys::generate();
        let Some(state) = invite_test_state(&host).await else {
            return;
        };
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        let community_id = community.id;
        state
            .db
            .add_relay_member(community_id, &member.public_key().to_hex(), "member", None)
            .await
            .expect("seed member");

        for keys in [&member, &outsider] {
            let response =
                post_json(state.clone(), &host, "/api/invites", keys, "{}".to_string()).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rejects_invalid_code() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let joiner = Keys::generate();
        let Some(state) = invite_test_state(&host).await else {
            return;
        };

        let body = serde_json::json!({ "code": "garbage.code" }).to_string();
        let response = post_json(state.clone(), &host, "/api/invites/claim", &joiner, body).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = read_json(response).await;
        assert_eq!(
            json.get("error").and_then(Value::as_str),
            Some("invite_invalid")
        );

        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        let is_member = state
            .db
            .is_relay_member(community.id, &joiner.public_key().to_hex())
            .await
            .expect("member check");
        assert!(!is_member, "invalid code must not admit anyone");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn code_minted_for_one_community_fails_on_another() {
        let host_a = format!("invites-a-{}.example", Uuid::new_v4().simple());
        let host_b = format!("invites-b-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let joiner = Keys::generate();
        let Some(state) = invite_test_state(&host_a).await else {
            return;
        };
        state
            .db
            .ensure_configured_community(&host_b)
            .await
            .expect("second community");
        let community_a = state
            .db
            .lookup_community_by_host(&host_a)
            .await
            .expect("lookup")
            .expect("community a");
        state
            .db
            .add_relay_member(community_a.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        let response = post_json(
            state.clone(),
            &host_a,
            "/api/invites",
            &owner,
            "{}".to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        let code = json.get("code").and_then(Value::as_str).expect("code");

        // Present community A's code on community B's host.
        let body = serde_json::json!({ "code": code }).to_string();
        let response = post_json(state, &host_b, "/api/invites/claim", &joiner, body).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Forge an already-expired invite payload signed with the relay's derived
    /// invite key. `mint_invite` clamps ttl to 60s minimum, so the only way to
    /// produce an expired code is to build the payload by hand at the token
    /// layer.
    fn forge_expired_invite_code(
        state: &AppState,
        community: buzz_core::CommunityId,
        seconds_ago: u64,
    ) -> String {
        let key = derive_invite_key(&state.relay_keypair);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_secs();
        let payload = InvitePayload {
            c: community.as_uuid().to_string(),
            r: "member".to_string(),
            e: now.saturating_sub(seconds_ago),
            n: "test-nonce".to_string(),
        };
        let payload_bytes = serde_json::to_vec(&payload).expect("payload serializes");
        let mut mac =
            <Hmac<Sha256> as KeyInit>::new_from_slice(&key).expect("HMAC accepts any key size");
        mac.update(&payload_bytes);
        let mac_bytes = mac.finalize().into_bytes();
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload_bytes),
            URL_SAFE_NO_PAD.encode(mac_bytes),
        )
    }

    /// Endpoint-level proof that expired codes (with a valid MAC) are
    /// rejected by `/api/invites/claim` with the distinguishable
    /// `invite_expired` body, and do not admit the caller.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rejects_expired_code() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let joiner = Keys::generate();
        let state = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        let code = forge_expired_invite_code(&state, community.id, 10);

        let body = serde_json::json!({ "code": code }).to_string();
        let response = post_json(state.clone(), &host, "/api/invites/claim", &joiner, body).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = read_json(response).await;
        // The expired branch is deliberately distinguishable from the generic
        // `invite_invalid` so the UX can prompt the user for a fresh link
        // without becoming a MAC oracle.
        assert_eq!(
            json.get("error").and_then(Value::as_str),
            Some("invite_expired"),
            "expired branch must be distinguishable from generic invalid",
        );

        let is_member = state
            .db
            .is_relay_member(community.id, &joiner.public_key().to_hex())
            .await
            .expect("member check");
        assert!(!is_member, "expired code must not admit anyone");
    }

    /// NIP-98 replay guard that returns `Ok(true)` the first time a given
    /// event id is seen and `Ok(false)` on every subsequent call — mirrors
    /// what the Redis guard does after a `SET NX` succeeds and then fails.
    struct SeenOnceReplayGuard {
        seen: Mutex<std::collections::HashSet<[u8; 32]>>,
    }

    impl SeenOnceReplayGuard {
        fn new() -> Self {
            Self {
                seen: Mutex::new(std::collections::HashSet::new()),
            }
        }
    }

    impl buzz_auth::Nip98ReplayGuard for SeenOnceReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            event_id: &'a EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            let bytes = *event_id.as_bytes();
            let inserted = self.seen.lock().expect("replay set").insert(bytes);
            Box::pin(async move { Ok(inserted) })
        }
    }

    struct UnavailableReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for UnavailableReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async {
                Err(buzz_auth::AuthError::Internal(
                    "test replay storage unavailable".to_owned(),
                ))
            })
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_handoff_management_rejects_replay_tampering_and_guard_outage() {
        let host = format!("identity-handoff-auth-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let expected = Keys::generate();
        let state_arc = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        let mut state_owned =
            Arc::try_unwrap(state_arc).unwrap_or_else(|_| panic!("sole owner of AppState"));
        state_owned.nip98_replay = Arc::new(SeenOnceReplayGuard::new());
        let state = Arc::new(state_owned);
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        let body = serde_json::json!({
            "expected_pubkey": expected.public_key().to_hex(),
            "link_incarnation_id": Uuid::new_v4().to_string(),
            "ttl_secs": 3600,
        })
        .to_string();
        let path = "/api/identity-handoffs";
        let url = format!("https://{host}{path}");
        let auth = nip98_auth_header(&owner, &url, body.as_bytes());
        let send = |state: Arc<AppState>, auth: String, body: String| {
            let host = host.clone();
            async move {
                build_router(state)
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(path)
                            .header(header::HOST, host)
                            .header(header::AUTHORIZATION, auth)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body))
                            .expect("request"),
                    )
                    .await
                    .expect("response")
            }
        };

        let first = send(state.clone(), auth.clone(), body.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let replay = send(state.clone(), auth, body.clone()).await;
        assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(read_json(replay).await["error"], "NIP-98: replay detected");

        let tamper_auth = nip98_auth_header(&owner, &url, body.as_bytes());
        let mut tampered: Value = serde_json::from_str(&body).expect("mint JSON");
        tampered["ttl_secs"] = Value::from(3599);
        let tampered = send(state.clone(), tamper_auth, tampered.to_string()).await;
        assert_eq!(tampered.status(), StatusCode::UNAUTHORIZED);

        let wrong_path_auth = nip98_auth_header(
            &owner,
            &format!("https://{host}/api/identity-handoffs/status"),
            body.as_bytes(),
        );
        let wrong_path = send(state.clone(), wrong_path_auth, body.clone()).await;
        assert_eq!(wrong_path.status(), StatusCode::UNAUTHORIZED);

        let mut unavailable = (*state).clone();
        unavailable.nip98_replay = Arc::new(UnavailableReplayGuard);
        let unavailable = Arc::new(unavailable);
        let response = post_json(unavailable, &host, path, &owner, body).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            read_json(response).await["error"],
            "NIP-98: replay check unavailable"
        );
    }

    /// Endpoint-level proof that a replayed NIP-98 auth event on a claim POST
    /// is rejected — the first claim succeeds, but reusing the exact same
    /// Authorization header (same signed NIP-98 event id) is rejected as
    /// replay before the invite verification ever runs.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rejects_replayed_nip98_auth() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let owner = Keys::generate();
        let joiner = Keys::generate();
        let state_arc = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        // Swap the always-fresh guard for one that fires the second time the
        // same event id is presented — the code path we're pinning.
        let mut state_owned =
            Arc::try_unwrap(state_arc).unwrap_or_else(|_| panic!("sole owner of AppState"));
        state_owned.nip98_replay = Arc::new(SeenOnceReplayGuard::new());
        let state = Arc::new(state_owned);

        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup")
            .expect("community exists");
        state
            .db
            .add_relay_member(community.id, &owner.public_key().to_hex(), "owner", None)
            .await
            .expect("seed owner");

        // Mint a valid code so the replay under test is on the claim path.
        let response = post_json(
            state.clone(),
            &host,
            "/api/invites",
            &owner,
            "{}".to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        let code = json.get("code").and_then(Value::as_str).expect("code");

        // Build one NIP-98 header and reuse it verbatim on two claim POSTs.
        let claim_body = serde_json::json!({ "code": code }).to_string();
        let claim_url = format!("https://{host}/api/invites/claim");
        let claim_auth = nip98_auth_header(&joiner, &claim_url, claim_body.as_bytes());

        let send_claim = |auth: String, body: String| {
            let state = state.clone();
            let host = host.clone();
            async move {
                build_router(state)
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/invites/claim")
                            .header(header::HOST, host.as_str())
                            .header(header::AUTHORIZATION, auth)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body))
                            .expect("request"),
                    )
                    .await
                    .expect("response")
            }
        };

        let first = send_claim(claim_auth.clone(), claim_body.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);

        // Same signed auth event, sent again → replay guard fires.
        let second = send_claim(claim_auth, claim_body).await;
        assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
        let json = read_json(second).await;
        assert_eq!(
            json.get("error").and_then(Value::as_str),
            Some("NIP-98: replay detected"),
        );
    }

    /// Endpoint-level proof that `/api/invites/claim` enforces the per-pubkey
    /// fixed-window rate limit — the same joiner probing the endpoint hits
    /// 429 on the `CLAIM_RATE_LIMIT + 1`th attempt inside the window.
    ///
    /// We use invalid codes throughout so no membership state can change; the
    /// limiter runs before code verification, so the transition from
    /// `invite_invalid` (403) to `too many invite claim attempts` (429) proves
    /// the limiter guard is on the request path and fires on repeat pubkey.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn claim_rate_limit_fires_on_repeat_pubkey() {
        let host = format!("invites-{}.example", Uuid::new_v4().simple());
        let joiner = Keys::generate();
        let state_arc = invite_test_state(&host)
            .await
            .expect("requires reachable Postgres and relay test state");
        // Fresh limiter with the production limit so the assertion pins the
        // in-endpoint threshold, not a test-only budget.
        let mut state_owned =
            Arc::try_unwrap(state_arc).unwrap_or_else(|_| panic!("sole owner of AppState"));
        state_owned.invite_claim_rate_limiter = Arc::new(claim_cache(
            super::CLAIM_RATE_CACHE_CAPACITY,
            super::CLAIM_RATE_WINDOW,
        ));
        let state = Arc::new(state_owned);

        let body = serde_json::json!({ "code": "garbage.code" }).to_string();
        for _ in 0..CLAIM_RATE_LIMIT {
            let response = post_json(
                state.clone(),
                &host,
                "/api/invites/claim",
                &joiner,
                body.clone(),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "attempts up to the limit should reach code verification and be rejected as invalid",
            );
            let json = read_json(response).await;
            assert_eq!(
                json.get("error").and_then(Value::as_str),
                Some("invite_invalid"),
            );
        }

        let over_limit = post_json(state, &host, "/api/invites/claim", &joiner, body).await;
        assert_eq!(over_limit.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            over_limit.headers().get(header::RETRY_AFTER).is_none(),
            "generic v1/v2 rate-limit headers are a compatibility contract"
        );
        let json = read_json(over_limit).await;
        assert_eq!(
            json.get("error").and_then(Value::as_str),
            Some("too many invite claim attempts, slow down"),
        );
    }

    #[test]
    fn policy_document_renders_markdown_and_escapes_raw_html() {
        let page = super::render_policy_document(
            "Terms of Service",
            "# Terms\n\nBe kind & honest.\n\n<script>alert(1)</script>",
        );
        assert!(page.contains("<title>Terms of Service</title>"), "{page}");
        assert!(page.contains("<h1>Terms</h1>"), "{page}");
        // `&` inside prose must be entity-encoded by the HTML writer.
        assert!(page.contains("Be kind &amp; honest."), "{page}");
        // Raw HTML in operator Markdown renders as escaped text, never markup.
        assert!(!page.contains("<script>"), "{page}");
        assert!(
            page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "{page}"
        );
    }

    /// The document routes are public (no NIP-98) and 404 until configured,
    /// exactly like the JSON policy endpoint they sit beside.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn join_policy_document_pages_serve_configured_markdown() {
        let host = format!("invites-docs-{}.example", Uuid::new_v4().simple());
        let Some(state) = invite_test_state(&host).await else {
            return;
        };

        let get_page = |state: Arc<crate::state::AppState>, path: &'static str| {
            let host = host.clone();
            async move {
                build_router(state)
                    .oneshot(
                        Request::builder()
                            .method("GET")
                            .uri(path)
                            .header(header::HOST, host)
                            .body(Body::empty())
                            .expect("request"),
                    )
                    .await
                    .expect("response")
            }
        };

        // Unconfigured relay: both documents 404.
        let response = get_page(state.clone(), "/api/join-policy/terms").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = get_page(state.clone(), "/api/join-policy/privacy").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Configure terms only — terms serves HTML, privacy still 404s.
        let mut state_inner = (*state).clone();
        let mut config = state_inner.config.as_ref().clone();
        config.join_policy = Some(crate::config::JoinPolicyConfig {
            terms_markdown: Some("# Terms\n\nNo funny business.".to_string()),
            privacy_markdown: None,
            age_attestation_required: false,
            version: "v".repeat(64),
        });
        state_inner.config = Arc::new(config);
        let state = Arc::new(state_inner);

        let response = get_page(state.clone(), "/api/join-policy/terms").await;
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/html"), "{content_type}");
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read body");
        let page = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(page.contains("<h1>Terms</h1>"), "{page}");
        assert!(page.contains("No funny business."), "{page}");

        let response = get_page(state, "/api/join-policy/privacy").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
