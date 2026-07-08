//! Deployment-operator HTTP APIs.
//!
//! These routes are outside the Nostr event data plane. They still use NIP-98
//! request signing and replay protection, but they do not run through event
//! ingest, relay membership, channel scoping, storage, or fan-out.

use std::sync::Arc;

use axum::{
    extract::{Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::handlers::community_provisioning::{
    normalize_candidate_host, validate_pubkey_hex, ProvisionCommunityRequest,
};
use crate::state::AppState;

use super::{api_error, bridge, internal_error};

/// Query parameters for `GET /operator/communities`.
#[derive(Debug, Deserialize)]
pub struct ListCommunitiesQuery {
    owner_pubkey: String,
}

/// Query parameters for `GET /operator/communities/availability`.
#[derive(Debug, Deserialize)]
pub struct CommunityAvailabilityQuery {
    host: String,
}

/// Shared operator auth prelude: bind an ingress host for NIP-98 URL/replay
/// scoping, verify the signed request, then gate on `RELAY_OPERATOR_PUBKEYS`.
async fn authorize_operator_request(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<nostr::PublicKey, (StatusCode, Json<Value>)> {
    // Bind to an existing ingress community only for NIP-98 URL/replay scoping.
    // This is not a tenant data-plane operation, so do not run relay-membership
    // checks and do not route through event ingest.
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
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let (pubkey, event_id_bytes) = bridge::verify_bridge_auth_with_options(
        headers,
        method,
        &url,
        body,
        true, // operator endpoints always require NIP-98; no X-Pubkey dev fallback
        body.is_some(),
    )?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    let pubkey_hex = pubkey.to_hex();
    if !state
        .config
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &pubkey_hex)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "actor not authorized: not a relay operator",
        ));
    }

    Ok(pubkey)
}

/// Provision or converge a community host.
///
/// `POST /operator/communities`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// { "host": "acme.communities.buzz.xyz", "initial_owner_pubkey": "<hex>" }
/// ```
///
/// The request is authenticated against the host it arrives on (so NIP-98 `u`
/// still binds to the request authority) but it intentionally does not require
/// relay membership in that host's community. The operator allowlist is the
/// authority for this deployment-root control-plane surface.
pub async fn provision_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pubkey = authorize_operator_request(
        &state,
        &headers,
        "POST",
        "/operator/communities",
        None,
        Some(&body),
    )
    .await?;

    let request: ProvisionCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid provision-community JSON: {e}"),
        )
    })?;

    match crate::handlers::community_provisioning::provision_community(&state, &pubkey, request)
        .await
    {
        Ok(response) => Ok(Json(serde_json::to_value(response).map_err(|e| {
            tracing::error!("failed to serialize provision-community response: {e}");
            internal_error("operator provision response serialization failed")
        })?)),
        Err(msg) if msg.starts_with("actor not authorized") => {
            Err(api_error(StatusCode::FORBIDDEN, &msg))
        }
        Err(msg) => Err(api_error(StatusCode::BAD_REQUEST, &msg)),
    }
}

/// List communities where a pubkey currently holds the `owner` role.
pub async fn list_owned_communities(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<ListCommunitiesQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authorize_operator_request(
        &state,
        &headers,
        "GET",
        "/operator/communities",
        raw_query.as_deref(),
        None,
    )
    .await?;

    let owner_pubkey = validate_pubkey_hex(&query.owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid owner_pubkey: expected 64-char hex pubkey",
        )
    })?;

    let rows = state
        .db
        .list_communities_owned_by(&owner_pubkey)
        .await
        .map_err(|e| internal_error(&format!("list owned communities: {e}")))?;

    Ok(Json(serde_json::json!({
        "owner_pubkey": owner_pubkey,
        "communities": rows.into_iter().map(|row| serde_json::json!({
            "community_id": row.id.to_string(),
            "host": row.host,
            "created_at": row.created_at,
        })).collect::<Vec<_>>(),
    })))
}

/// Check whether a community host is available, returning the relay-canonical
/// normalized authority used by create.
pub async fn community_availability(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<CommunityAvailabilityQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authorize_operator_request(
        &state,
        &headers,
        "GET",
        "/operator/communities/availability",
        raw_query.as_deref(),
        None,
    )
    .await?;

    let normalized_host = normalize_candidate_host(&query.host)
        .map_err(|msg| api_error(StatusCode::BAD_REQUEST, &msg))?;
    let existing = state
        .db
        .lookup_community_by_host(&normalized_host)
        .await
        .map_err(|e| internal_error(&format!("check community availability: {e}")))?;

    Ok(Json(serde_json::json!({
        "host": query.host,
        "normalized_host": normalized_host,
        "available": existing.is_none(),
        "community_id": existing.map(|record| record.id.to_string()),
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
        fn try_mark<'a>(
            &'a self,
            _ctx: &'a buzz_core::TenantContext,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1
    const INGRESS_HOST: &str = "operator-ingress.example";

    fn nip98_auth_header(keys: &Keys, url: &str, method: &str, body: Option<&[u8]>) -> String {
        let mut tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
        ];
        if let Some(body) = body {
            let hash: [u8; 32] = Sha256::digest(body).into();
            let hash_hex = hex::encode(hash);
            tags.push(Tag::parse(["payload", hash_hex.as_str()]).expect("payload tag"));
        }
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
    }

    fn nip98_auth_header_without_payload(keys: &Keys, url: &str, method: &str) -> String {
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let event_json = serde_json::to_string(&event).expect("serialize NIP-98 event");
        let encoded = base64::engine::general_purpose::STANDARD.encode(event_json.as_bytes());
        format!("Nostr {encoded}")
    }

    async fn operator_test_state(operator_keys: &[Keys]) -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.database_url = TEST_DB_URL.to_string();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_url = format!("wss://{INGRESS_HOST}");
        config.relay_operator_pubkeys = operator_keys
            .iter()
            .map(|keys| keys.public_key().to_hex())
            .collect();

        let pool = sqlx::PgPool::connect(TEST_DB_URL).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community(INGRESS_HOST).await.ok()?;

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

    async fn read_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response JSON")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn non_allowlisted_operator_key_gets_403() {
        let operator = Keys::generate();
        let outsider = Keys::generate();
        let Some(state) = operator_test_state(&[operator]).await else {
            return;
        };
        let body = format!(
            r#"{{"host":"community-{}.example"}}"#,
            Uuid::new_v4().simple()
        );
        let url = format!("https://{INGRESS_HOST}/operator/communities");
        let auth = nip98_auth_header(&outsider, &url, "POST", Some(body.as_bytes()));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn post_operator_body_requires_payload_tag() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(&[operator.clone()]).await else {
            return;
        };
        let body = format!(
            r#"{{"host":"community-{}.example"}}"#,
            Uuid::new_v4().simple()
        );
        let url = format!("https://{INGRESS_HOST}/operator/communities");
        let auth = nip98_auth_header_without_payload(&operator, &url, "POST");

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let json = read_json(response).await;
        assert!(
            json.get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("missing payload tag"),
            "unexpected response: {json:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn happy_path_create_returns_created_and_bootstraps_owner() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(&[operator.clone()]).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": owner.public_key().to_hex(),
        })
        .to_string();
        let url = format!("https://{INGRESS_HOST}/operator/communities");
        let auth = nip98_auth_header(&operator, &url, "POST", Some(body.as_bytes()));

        let response = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(json.get("status").and_then(Value::as_str), Some("created"));
        assert_eq!(
            json.get("host").and_then(Value::as_str),
            Some(host.as_str())
        );
        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        let member = state
            .db
            .get_relay_member(community.id, &owner.public_key().to_hex())
            .await
            .expect("lookup owner role")
            .expect("owner member exists");
        assert_eq!(member.role, "owner");
    }
}
