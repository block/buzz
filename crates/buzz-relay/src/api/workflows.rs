//! Authorized structured reads for workflow execution state.
//!
//! Runs and approvals are relay-owned database rows, not Nostr events. These
//! endpoints expose those read models without inventing synthetic events.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use buzz_core::TenantContext;

use crate::{
    api::{api_error, bridge, internal_error},
    state::AppState,
};

const DEFAULT_RUN_LIMIT: i64 = 20;
const MAX_RUN_LIMIT: i64 = 100;

/// Pagination query for workflow run history.
#[derive(Debug, Deserialize, Default)]
pub struct RunsQuery {
    before: Option<DateTime<Utc>>,
    before_id: Option<Uuid>,
    limit: Option<i64>,
}

fn request_path(path: &str, raw_query: Option<&str>) -> String {
    match raw_query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_string(),
    }
}

async fn authorize_workflow_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
    workflow_id: Uuid,
    allow_immutable_owner: bool,
) -> Result<TenantContext, (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;

    let path_with_query = request_path(path, raw_query);
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    let bridge::VerifiedBridgeAuth {
        pubkey,
        event_id_bytes,
        signed_created_at,
    } = bridge::verify_bridge_auth(headers, "GET", &url, None, state.config.require_auth_token)?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = super::relay_members::extract_auth_tag_header(headers);
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
        signed_created_at,
    )
    .await?;

    let workflow = state
        .db
        .get_workflow(tenant.community(), workflow_id)
        .await
        .map_err(|error| match error {
            buzz_db::error::DbError::NotFound(_) => {
                api_error(StatusCode::NOT_FOUND, "workflow not found")
            }
            other => internal_error(&format!("get workflow for run read: {other}")),
        })?;
    let channel_id = workflow
        .channel_id
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "workflow is not channel-scoped"))?;
    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| internal_error(&format!("workflow channel access lookup: {error}")))?;
    if !accessible.contains(&channel_id) {
        let controls = allow_immutable_owner
            && (workflow.owner_pubkey == pubkey_bytes
                || state
                    .db
                    .is_agent_owner(tenant.community(), &workflow.owner_pubkey, &pubkey_bytes)
                    .await
                    .map_err(|error| internal_error(&format!("workflow owner lookup: {error}")))?);
        if !controls {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "workflow is not accessible",
            ));
        }
    }

    Ok(tenant)
}

/// `GET /workflows/{workflow_id}/revision` — current signed revision for an
/// authorized channel reader or the managed agent's immutable human owner.
///
/// This narrow endpoint does not grant channel visibility. It returns only the
/// revision event ID needed to construct a revision-bound manual trigger; the
/// signed definition remains subject to normal channel-read authorization.
pub async fn workflow_revision(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/{workflow_id}/revision");
    let tenant = authorize_workflow_read(&state, &headers, &path, None, workflow_id, true).await?;
    let workflow = state
        .db
        .get_workflow(tenant.community(), workflow_id)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "workflow not found"))?;
    let revision = workflow.definition_event_id.as_deref().ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "owner-signed workflow revision is unavailable",
        )
    })?;
    let event = state
        .db
        .get_event_by_id(tenant.community(), revision)
        .await
        .map_err(|error| internal_error(&format!("get workflow revision: {error}")))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "workflow revision is unavailable"))?;
    Ok(Json(serde_json::json!({ "id": event.event.id.to_hex() })))
}

/// `GET /workflows/{workflow_id}/runs` — one authorized, keyset-paginated page.
pub async fn workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<Uuid>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    Query(query): Query<RunsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if query.before.is_some() != query.before_id.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "before and before_id must be supplied together",
        ));
    }
    let limit = query.limit.unwrap_or(DEFAULT_RUN_LIMIT);
    if !(1..=MAX_RUN_LIMIT).contains(&limit) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 100",
        ));
    }

    let path = format!("/workflows/{workflow_id}/runs");
    let tenant = authorize_workflow_read(
        &state,
        &headers,
        &path,
        raw_query.as_deref(),
        workflow_id,
        false,
    )
    .await?;
    let mut rows = state
        .db
        .list_workflow_runs_page(
            tenant.community(),
            workflow_id,
            query.before,
            query.before_id,
            limit + 1,
        )
        .await
        .map_err(|error| internal_error(&format!("list workflow runs: {error}")))?;

    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if has_more {
        rows.last().map(|last| {
            serde_json::json!({
                "before": last.created_at,
                "before_id": last.id,
            })
        })
    } else {
        None
    };

    Ok(Json(serde_json::json!({
        "runs": rows.iter().map(run_json).collect::<Vec<_>>(),
        "next": next,
    })))
}

/// `GET /workflows/{workflow_id}/runs/{run_id}/approvals` — approvals for a run.
pub async fn run_approvals(
    State(state): State<Arc<AppState>>,
    Path((workflow_id, run_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/{workflow_id}/runs/{run_id}/approvals");
    let tenant = authorize_workflow_read(&state, &headers, &path, None, workflow_id, false).await?;

    let run = state
        .db
        .get_workflow_run(tenant.community(), run_id)
        .await
        .map_err(|error| match error {
            buzz_db::error::DbError::NotFound(_) => {
                api_error(StatusCode::NOT_FOUND, "workflow run not found")
            }
            other => internal_error(&format!("get workflow run for approval read: {other}")),
        })?;
    if run.workflow_id != workflow_id {
        return Err(api_error(StatusCode::NOT_FOUND, "workflow run not found"));
    }

    let approvals = state
        .db
        .get_run_approvals(tenant.community(), workflow_id, run_id)
        .await
        .map_err(|error| internal_error(&format!("list run approvals: {error}")))?;
    Ok(Json(serde_json::json!({
        "approvals": approvals.iter().map(approval_json).collect::<Vec<_>>(),
    })))
}

fn run_json(run: &buzz_db::workflow::WorkflowRunRecord) -> Value {
    serde_json::json!({
        "id": run.id,
        "workflow_id": run.workflow_id,
        "status": run.status,
        "current_step": run.current_step,
        "execution_trace": run.execution_trace,
        "started_at": run.started_at.map(|value| value.timestamp()),
        "completed_at": run.completed_at.map(|value| value.timestamp()),
        "error_code": run.error_code,
        "error_message": run.error_message,
        "created_at": run.created_at.timestamp(),
    })
}

fn approval_json(approval: &buzz_db::workflow::ApprovalRecord) -> Value {
    serde_json::json!({
        "approval_ref": hex::encode(&approval.token),
        "workflow_id": approval.workflow_id,
        "run_id": approval.run_id,
        "step_id": approval.step_id,
        "step_index": approval.step_index,
        "approver_spec": approval.approver_spec,
        "status": approval.status,
        "approver_pubkey": approval.approver_pubkey.as_ref().map(hex::encode),
        "note": approval.note,
        "expires_at": approval.expires_at,
        "created_at": approval.created_at.timestamp(),
    })
}

#[cfg(test)]
mod postgres_tests {
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::Digest;
    use tower::ServiceExt;

    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn workflow_test_state(host: &str) -> Arc<AppState> {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_string());
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let mut config = crate::config::Config::from_env().expect("config from env");
        config.database_url = database_url.clone();
        config.redis_url = redis_url.clone();
        config.relay_url = format!("wss://{host}");
        config.require_auth_token = false;
        config.require_relay_membership = false;

        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect workflow API test database");
        // The harness prepares the schema (CI uses pgschema, not SQLx history).
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community(host)
            .await
            .expect("create workflow API test community");
        let redis_pool = deadpool_redis::Config::from_url(&redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool config");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool);
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
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
        Arc::new(state)
    }

    #[test]
    fn request_path_preserves_signed_query_verbatim() {
        assert_eq!(
            request_path("/workflows/id/runs", Some("limit=20&before_id=abc")),
            "/workflows/id/runs?limit=20&before_id=abc"
        );
        assert_eq!(
            request_path("/workflows/id/runs", None),
            "/workflows/id/runs"
        );
    }

    #[test]
    fn approval_wire_does_not_expose_hash_as_token() {
        let approval = buzz_db::workflow::ApprovalRecord {
            token: vec![0xab; 32],
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "review".to_string(),
            step_index: 1,
            approver_spec: "any".to_string(),
            status: buzz_db::workflow::ApprovalStatus::Pending,
            approver_pubkey: None,
            note: None,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };
        let wire = approval_json(&approval);
        assert!(wire.get("token").is_none());
        assert_eq!(wire["approval_ref"], hex::encode([0xab; 32]));
    }

    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn revoked_immutable_owner_receives_only_revision_id() {
        use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};

        let host = format!("workflow-revision-{}.example", Uuid::new_v4().simple());
        let state = workflow_test_state(&host).await;
        let tenant = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("load workflow API test community");
        let community = tenant.id;
        let owner = Keys::generate();
        let agent = Keys::generate();
        let owner_bytes = owner.public_key().to_bytes();
        let agent_bytes = agent.public_key().to_bytes();
        state
            .db
            .ensure_user(community, &owner_bytes)
            .await
            .expect("ensure immutable owner");
        state
            .db
            .ensure_user(community, &agent_bytes)
            .await
            .expect("ensure managed agent");
        assert!(state
            .db
            .set_agent_owner(community, &agent_bytes, &owner_bytes)
            .await
            .expect("set immutable owner"));
        let channel = state
            .db
            .create_channel(
                community,
                "revision-secret-boundary",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                &agent_bytes,
                None,
            )
            .await
            .expect("create workflow channel");
        state
            .db
            .add_member(
                community,
                channel.id,
                &owner_bytes,
                MemberRole::Member,
                Some(&agent_bytes),
            )
            .await
            .expect("add immutable owner to workflow channel");

        let workflow_id = Uuid::new_v4();
        let secret = format!("secret-{}", Uuid::new_v4().simple());
        let yaml = format!(
            "name: guarded\ntrigger:\n  on: webhook\nsteps:\n  - id: call\n    action: call_webhook\n    url: https://example.invalid\n    headers:\n      Authorization: Bearer {secret}\n    body: '{secret}'\n"
        );
        let revision = EventBuilder::new(Kind::Custom(30620), &yaml)
            .tags(vec![
                Tag::parse(["d", workflow_id.to_string().as_str()]).expect("d tag"),
                Tag::parse(["h", channel.id.to_string().as_str()]).expect("h tag"),
            ])
            .sign_with_keys(&agent)
            .expect("sign secret-bearing workflow revision");
        let (_, definition_json) =
            buzz_workflow::WorkflowEngine::parse_yaml(&yaml).expect("parse workflow YAML");
        let definition_hash = sha2::Sha256::digest(definition_json.as_bytes());
        let mut tx = state
            .db
            .begin_transaction()
            .await
            .expect("begin workflow seed");
        buzz_db::event::insert_event_in_transaction(
            &mut tx,
            community,
            &revision,
            Some(channel.id),
        )
        .await
        .expect("persist signed workflow revision");
        state
            .db
            .upsert_workflow(
                &mut tx,
                community,
                workflow_id,
                Some(channel.id),
                &agent_bytes,
                "guarded",
                &definition_json,
                definition_hash.as_slice(),
                revision.id.as_bytes(),
            )
            .await
            .expect("materialize workflow revision");
        tx.commit().await.expect("commit workflow seed");
        state
            .db
            .remove_member(community, channel.id, &owner_bytes, &agent_bytes)
            .await
            .expect("revoke immutable owner's channel access");
        let tenant_context = TenantContext::resolved(community, host.clone());
        state.invalidate_membership(&tenant_context, channel.id, &owner_bytes);

        let path = format!("/workflows/{workflow_id}/revision");
        let response = crate::router::build_router(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&path)
                    .header(header::HOST, &host)
                    .header("x-pubkey", owner.public_key().to_hex())
                    .body(Body::empty())
                    .expect("workflow revision request"),
            )
            .await
            .expect("workflow revision response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read workflow revision response");
        let body: Value = serde_json::from_slice(&bytes).expect("revision response JSON");
        assert_eq!(body, serde_json::json!({ "id": revision.id.to_hex() }));
        let serialized = String::from_utf8(bytes.to_vec()).expect("UTF-8 response");
        assert!(!serialized.contains(&secret));
        assert!(body.get("content").is_none());
        assert!(body.get("tags").is_none());
    }
}
