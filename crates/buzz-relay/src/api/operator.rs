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
use uuid::Uuid;

use buzz_core::{CommunityId, TenantContext};

use crate::handlers::community_provisioning::{
    normalize_candidate_host, validate_pubkey_hex, ProvisionCommunityRequest,
    TransferCommunityRequest, TransferCommunityResponse,
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

const OPERATOR_REPLAY_SCOPE: &str = "operator-management";

/// Shared deployment-global operator auth prelude. The canonical management
/// origin and replay namespace are configuration, never tenant registry state
/// or an inbound proxy `Host` header.
async fn authorize_operator_request(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<nostr::PublicKey, (StatusCode, Json<Value>)> {
    let origin = state
        .config
        .relay_operator_api_origin
        .as_deref()
        .ok_or_else(|| internal_error("operator API origin is not configured"))?;
    let path_with_query = match raw_query {
        Some(q) if !q.is_empty() => format!("{path}?{q}"),
        _ => path.to_string(),
    };
    let url = format!("{origin}{path_with_query}");
    let (pubkey, event_id_bytes) = bridge::verify_bridge_auth_with_options(
        headers,
        method,
        &url,
        body,
        true, // operator endpoints always require NIP-98; no X-Pubkey dev fallback
        body.is_some(),
    )?;
    check_operator_replay(state, event_id_bytes).await?;

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

async fn check_operator_replay(
    state: &AppState,
    event_id_bytes: [u8; 32],
) -> Result<(), (StatusCode, Json<Value>)> {
    let event_id = nostr::EventId::from_byte_array(event_id_bytes);
    match state
        .nip98_replay
        .try_mark_in_scope(
            OPERATOR_REPLAY_SCOPE,
            &event_id,
            buzz_auth::DEFAULT_REPLAY_TTL_SECS,
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(api_error(
            StatusCode::UNAUTHORIZED,
            "NIP-98: replay detected",
        )),
        Err(error) => {
            tracing::warn!(
                scope = OPERATOR_REPLAY_SCOPE,
                error = %error,
                "operator NIP-98 replay guard failed; rejecting request fail-closed"
            );
            Err(api_error(
                StatusCode::UNAUTHORIZED,
                "NIP-98: replay check unavailable",
            ))
        }
    }
}

/// Create a community host and atomically bootstrap its initial owner.
///
/// `POST /operator/communities`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// { "host": "acme.communities.buzz.xyz", "initial_owner_pubkey": "<hex>" }
/// ```
///
/// The request is authenticated against `RELAY_OPERATOR_API_ORIGIN` and does
/// not bind the inbound host to a tenant. The operator allowlist is the
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
        Err(msg) if msg == "community already exists" => Err(api_error(StatusCode::CONFLICT, &msg)),
        Err(msg)
            if msg.starts_with("failed to create community:")
                || msg.starts_with("community provisioned but owner bootstrap failed:") =>
        {
            tracing::error!(error = %msg, "operator community persistence failed");
            Err(internal_error("operator community persistence failed"))
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

/// Transfer ownership of a community to a new owner pubkey.
///
/// `POST /operator/communities/transfer`, NIP-98 signed by a pubkey in
/// `RELAY_OPERATOR_PUBKEYS`, body:
///
/// ```json
/// { "community_id": "<uuid>", "new_owner_pubkey": "<hex>" }
/// ```
///
/// The previous owner is demoted to `member` (not `admin`). The transfer is
/// instant and atomic at the database layer. Publication of the updated
/// NIP-43 membership list is best-effort, matching the provision path.
pub async fn transfer_community(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _pubkey = authorize_operator_request(
        &state,
        &headers,
        "POST",
        "/operator/communities/transfer",
        None,
        Some(&body),
    )
    .await?;

    let request: TransferCommunityRequest = serde_json::from_slice(&body).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid transfer-community JSON: {e}"),
        )
    })?;

    let community_uuid = Uuid::parse_str(&request.community_id).map_err(|e| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid community_id: {e}"),
        )
    })?;

    let new_owner_pubkey = validate_pubkey_hex(&request.new_owner_pubkey).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid new_owner_pubkey: expected 64-char hex pubkey",
        )
    })?;

    let expected_owner_pubkey =
        validate_pubkey_hex(&request.expected_owner_pubkey).ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "invalid expected_owner_pubkey: expected 64-char hex pubkey",
            )
        })?;

    let community = CommunityId::from_uuid(community_uuid);

    let result = state
        .db
        .transfer_ownership(community, &new_owner_pubkey, &expected_owner_pubkey)
        .await
        .map_err(|e| internal_error(&format!("transfer ownership: {e}")))?;

    let (status, previous_owner) = match result {
        buzz_db::relay_members::TransferResult::Transferred { previous_owner } => {
            ("transferred", previous_owner)
        }
        buzz_db::relay_members::TransferResult::AlreadyOwner => ("already_owner", None),
        buzz_db::relay_members::TransferResult::NoOwner => {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                "community has no owner to transfer from",
            ));
        }
        buzz_db::relay_members::TransferResult::OwnerConflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "owner_conflict: the current owner no longer matches expected_owner_pubkey",
            ));
        }
        buzz_db::relay_members::TransferResult::LimitReached => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "limit_reached: transferee already owns the maximum number of communities",
            ));
        }
    };

    // Best-effort NIP-43 membership snapshot publication — same pattern as
    // provision_community. The DB mutation is already committed; a publication
    // failure must not turn a success into an HTTP error.
    if state.config.require_relay_membership {
        if let Some(host) = state
            .db
            .lookup_community_host(community)
            .await
            .map_err(|e| internal_error(&format!("lookup community host: {e}")))?
        {
            let tenant = TenantContext::resolved(community, host);
            if let Err(error) =
                crate::handlers::side_effects::publish_nip43_membership_list(&tenant, &state).await
            {
                tracing::warn!(
                    community = %community,
                    error = %error,
                    "ownership transferred but NIP-43 membership snapshot publication failed"
                );
            }
        }
    }

    let response = TransferCommunityResponse {
        community_id: request.community_id,
        new_owner_pubkey,
        status,
        previous_owner,
    };

    Ok(Json(serde_json::to_value(response).map_err(|e| {
        tracing::error!("failed to serialize transfer-community response: {e}");
        internal_error("operator transfer response serialization failed")
    })?))
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

    use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
    use buzz_db::event::EventQuery;

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
        config.relay_url = "wss://tenant.example".to_string();
        config.relay_operator_api_origin = Some(format!("http://{INGRESS_HOST}"));
        config.relay_operator_pubkeys = operator_keys
            .iter()
            .map(|keys| keys.public_key().to_hex())
            .collect();
        config.require_relay_membership = true;

        let pool = sqlx::PgPool::connect(TEST_DB_URL).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());

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
        let url = format!("http://{INGRESS_HOST}/operator/communities");
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
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = format!(
            r#"{{"host":"community-{}.example"}}"#,
            Uuid::new_v4().simple()
        );
        let url = format!("http://{INGRESS_HOST}/operator/communities");
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
    async fn unmapped_management_host_can_check_availability() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let query = format!("host={host}");
        let url = format!("http://{INGRESS_HOST}/operator/communities/availability?{query}");
        let auth = nip98_auth_header(&operator, &url, "GET", None);

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/operator/communities/availability?{query}"))
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(json.get("available").and_then(Value::as_bool), Some(true));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unmapped_management_host_can_list_owned_communities() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let owner_hex = owner.public_key().to_hex();
        let query = format!("owner_pubkey={owner_hex}");
        let url = format!("http://{INGRESS_HOST}/operator/communities?{query}");
        let auth = nip98_auth_header(&operator, &url, "GET", None);

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/operator/communities?{query}"))
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(
            json.get("owner_pubkey").and_then(Value::as_str),
            Some(owner_hex.as_str())
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn happy_path_create_returns_created_and_bootstraps_owner() {
        let operator = Keys::generate();
        let owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": owner.public_key().to_hex(),
            "create_only": true,
        })
        .to_string();
        let url = format!("http://{INGRESS_HOST}/operator/communities");
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
        let owner_hex = owner.public_key().to_hex();
        let member = state
            .db
            .get_relay_member(community.id, &owner_hex)
            .await
            .expect("lookup owner role")
            .expect("owner member exists");
        assert_eq!(member.role, "owner");

        let membership_events = state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![KIND_NIP43_MEMBERSHIP_LIST as i32]),
                global_only: true,
                limit: Some(1),
                ..EventQuery::for_community(community.id)
            })
            .await
            .expect("query membership snapshot");
        let snapshot = membership_events
            .first()
            .expect("membership snapshot published during provisioning");
        assert!(snapshot.event.tags.iter().any(|tag| {
            tag.as_slice()
                .first()
                .is_some_and(|value| value == "member")
                && tag
                    .as_slice()
                    .get(1)
                    .is_some_and(|value| value == &owner_hex)
                && tag.as_slice().get(2).is_some_and(|value| value == "owner")
        }));
    }

    /// Happy path: POST /operator/communities/transfer swaps ownership, demotes
    /// the old owner to `member`, and publishes a NIP-43 snapshot reflecting the
    /// new roles.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn happy_path_transfer_swaps_owner_and_demotes_old_to_member() {
        let operator = Keys::generate();
        let initial_owner = Keys::generate();
        let new_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };

        // Create a community with initial_owner.
        let host = format!("community-{}.example", Uuid::new_v4().simple());
        let create_body = serde_json::json!({
            "host": host,
            "initial_owner_pubkey": initial_owner.public_key().to_hex(),
            "create_only": true,
        })
        .to_string();
        let create_url = format!("http://{INGRESS_HOST}/operator/communities");
        let create_auth =
            nip98_auth_header(&operator, &create_url, "POST", Some(create_body.as_bytes()));
        build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, create_auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(create_body))
                    .expect("create request"),
            )
            .await
            .expect("create response");

        let community = state
            .db
            .lookup_community_by_host(&host)
            .await
            .expect("lookup community")
            .expect("community exists");
        let community_id = community.id.to_string();
        let initial_owner_hex = initial_owner.public_key().to_hex();
        let new_owner_hex = new_owner.public_key().to_hex();

        // Transfer ownership.
        let transfer_body = serde_json::json!({
            "community_id": community_id,
            "new_owner_pubkey": new_owner_hex,
            "expected_owner_pubkey": initial_owner_hex,
        })
        .to_string();
        let transfer_url = format!("http://{INGRESS_HOST}/operator/communities/transfer");
        let transfer_auth = nip98_auth_header(
            &operator,
            &transfer_url,
            "POST",
            Some(transfer_body.as_bytes()),
        );

        let response = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities/transfer")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, transfer_auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(transfer_body))
                    .expect("transfer request"),
            )
            .await
            .expect("transfer response");

        assert_eq!(response.status(), StatusCode::OK);
        let json = read_json(response).await;
        assert_eq!(
            json.get("status").and_then(Value::as_str),
            Some("transferred")
        );
        assert_eq!(
            json.get("new_owner_pubkey").and_then(Value::as_str),
            Some(new_owner_hex.as_str())
        );
        assert_eq!(
            json.get("previous_owner").and_then(Value::as_str),
            Some(initial_owner_hex.as_str())
        );

        // New owner is owner.
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &new_owner_hex)
                .await
                .expect("get new owner")
                .expect("new owner exists")
                .role,
            "owner"
        );
        // Old owner is member (not admin).
        assert_eq!(
            state
                .db
                .get_relay_member(community.id, &initial_owner_hex)
                .await
                .expect("get old owner")
                .expect("old owner exists")
                .role,
            "member"
        );

        // NIP-43 snapshot reflects the new roles.
        let membership_events = state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![KIND_NIP43_MEMBERSHIP_LIST as i32]),
                global_only: true,
                limit: Some(1),
                ..EventQuery::for_community(community.id)
            })
            .await
            .expect("query membership snapshot");
        let snapshot = membership_events
            .first()
            .expect("membership snapshot published after transfer");
        assert!(snapshot.event.tags.iter().any(|tag| {
            tag.as_slice().first().is_some_and(|v| v == "member")
                && tag.as_slice().get(1).is_some_and(|v| v == &new_owner_hex)
                && tag.as_slice().get(2).is_some_and(|v| v == "owner")
        }));
        assert!(snapshot.event.tags.iter().any(|tag| {
            tag.as_slice().first().is_some_and(|v| v == "member")
                && tag
                    .as_slice()
                    .get(1)
                    .is_some_and(|v| v == &initial_owner_hex)
                && tag.as_slice().get(2).is_some_and(|v| v == "member")
        }));
    }

    /// Transfer with an invalid community_id returns 400.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_with_invalid_community_id_returns_400() {
        let operator = Keys::generate();
        let new_owner = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = serde_json::json!({
            "community_id": "not-a-uuid",
            "new_owner_pubkey": new_owner.public_key().to_hex(),
            "expected_owner_pubkey": new_owner.public_key().to_hex(),
        })
        .to_string();
        let url = format!("http://{INGRESS_HOST}/operator/communities/transfer");
        let auth = nip98_auth_header(&operator, &url, "POST", Some(body.as_bytes()));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities/transfer")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Transfer with an invalid new_owner_pubkey returns 400.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_with_invalid_pubkey_returns_400() {
        let operator = Keys::generate();
        let Some(state) = operator_test_state(std::slice::from_ref(&operator)).await else {
            return;
        };
        let body = serde_json::json!({
            "community_id": Uuid::new_v4().to_string(),
            "new_owner_pubkey": "not-a-pubkey",
            "expected_owner_pubkey": "not-a-pubkey",
        })
        .to_string();
        let url = format!("http://{INGRESS_HOST}/operator/communities/transfer");
        let auth = nip98_auth_header(&operator, &url, "POST", Some(body.as_bytes()));

        let response = build_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/operator/communities/transfer")
                    .header(header::HOST, INGRESS_HOST)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
