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
    http::{HeaderMap, StatusCode},
    response::{Html, Json},
};
use serde::Deserialize;
use serde_json::Value;

use crate::handlers::side_effects::{publish_nip43_member_added, publish_nip43_membership_list};
use crate::invite_token::{self, DEFAULT_INVITE_TTL_SECS};
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

/// Body for `POST /api/invites`.
#[derive(Debug, Default, Deserialize)]
pub struct MintInviteRequest {
    /// Requested lifetime in seconds. Clamped to
    /// [`invite_token::MAX_INVITE_TTL_SECS`]; defaults to 72 h.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Body for `POST /api/invites/claim`.
#[derive(Debug, Deserialize)]
pub struct ClaimInviteRequest {
    /// The invite code to redeem.
    pub code: String,
    /// Relay-issued proof of accepting the configured terms, when required.
    #[serde(default)]
    pub policy_receipt: Option<String>,
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

    let key = invite_token::derive_invite_key(&state.relay_keypair);
    let ttl = request.ttl_secs.unwrap_or(DEFAULT_INVITE_TTL_SECS);
    let (code, expires_at) = invite_token::mint_invite(&key, tenant.community(), ttl);

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
        expires_at,
        "relay invite minted"
    );

    Ok(Json(serde_json::json!({
        "code": code,
        "expires_at": expires_at,
        "url": format!("{scheme}://{}/invite/{}", tenant.host(), code),
    })))
}

/// Claim an invite code — `POST /api/invites/claim`, NIP-98 signed by the
/// *joining* pubkey. Exempt from the relay-membership gate by design.
pub async fn claim_invite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (tenant, pubkey) = authenticate(&state, &headers, "/api/invites/claim", &body).await?;

    if claim_rate_limited(&state, tenant.community(), &pubkey) {
        return Err(api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many invite claim attempts, slow down",
        ));
    }

    let request: ClaimInviteRequest = serde_json::from_slice(&body)
        .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid claim JSON: {e}")))?;

    let key = invite_token::derive_invite_key(&state.relay_keypair);
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

    let claimer_hex = pubkey.to_hex();
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

    use super::{claim_key_rate_limited, CLAIM_RATE_LIMIT};
    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use uuid::Uuid;

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

    fn nip98_auth_header(keys: &Keys, url: &str, body: &[u8]) -> String {
        let hash: [u8; 32] = Sha256::digest(body).into();
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", "POST"]).expect("method tag"),
            Tag::parse(["payload", hex::encode(hash).as_str()]).expect("payload tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
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

    async fn read_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response JSON")
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
