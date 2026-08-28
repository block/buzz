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

fn ensure_channel_access(
    accessible: &[Uuid],
    channel_id: Uuid,
    error: &'static str,
) -> Result<(), (StatusCode, Json<Value>)> {
    if accessible.contains(&channel_id) {
        Ok(())
    } else {
        Err(api_error(StatusCode::FORBIDDEN, error))
    }
}

async fn enforce_current_channel_read(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: &nostr::PublicKey,
    channel_id: Uuid,
    error: &'static str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey_bytes,
        auth_tag,
    )
    .await?;

    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| internal_error(&format!("workflow channel access lookup: {error}")))?;
    ensure_channel_access(&accessible, channel_id, error)
}

async fn authorize_workflow_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
    workflow_id: Uuid,
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
    enforce_current_channel_read(
        state,
        &tenant,
        headers,
        &pubkey,
        channel_id,
        "workflow is not accessible",
    )
    .await?;

    Ok(tenant)
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
    let tenant =
        authorize_workflow_read(&state, &headers, &path, raw_query.as_deref(), workflow_id).await?;
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
    let tenant = authorize_workflow_read(&state, &headers, &path, None, workflow_id).await?;

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

// A missing/revoked authority is terminal; unavailable storage is not. Keep
// this distinction at the endpoint so ACP's bounded transport retries can work.
pub(crate) fn wake_lookup_error(error: buzz_db::DbError) -> (StatusCode, Json<Value>) {
    use buzz_db::DbError;
    let status = match &error {
        DbError::NotFound(_) => StatusCode::NOT_FOUND,
        DbError::Sqlx(sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        DbError::Sqlx(sqlx::Error::Database(error))
            if error.code().is_some_and(|code| {
                code.starts_with("08")
                    || matches!(
                        code.as_ref(),
                        "40001"
                            | "40P01"
                            | "53300"
                            | "55P03"
                            | "57014"
                            | "57P01"
                            | "57P02"
                            | "57P03"
                    )
            }) =>
        {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    tracing::warn!(%error, %status, "workflow wake authority lookup failed");
    api_error(status, "workflow wake authority unavailable")
}

/// `GET /workflow-wakes/{run_id}/{message_id}` — exact authority bundle for one wake.
pub async fn workflow_wake_authority(
    State(state): State<Arc<AppState>>,
    Path((run_id, message_id)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflow-wakes/{run_id}/{message_id}");
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "workflow wake not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path);
    let (recipient, auth_event_id) =
        bridge::verify_bridge_auth(&headers, "GET", &url, None, state.config.require_auth_token)?;
    bridge::enforce_http_admission(&state, &tenant, &recipient).await?;
    bridge::check_nip98_replay(&state, &tenant, auth_event_id).await?;

    let run = state
        .db
        .get_workflow_run(tenant.community(), run_id)
        .await
        .map_err(wake_lookup_error)?;
    let workflow = state
        .db
        .get_workflow(tenant.community(), run.workflow_id)
        .await
        .map_err(wake_lookup_error)?;
    let definition_id = run
        .definition_event_id
        .as_deref()
        .filter(|id| id.len() == 32)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workflow wake not found"))?;
    let message_id = nostr::EventId::from_hex(&message_id)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid message id"))?;
    let definition = state
        .db
        .get_workflow_revision(tenant.community(), definition_id)
        .await
        .map_err(wake_lookup_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workflow wake not found"))?;
    let message = state
        .db
        .get_event_by_id(tenant.community(), message_id.as_bytes())
        .await
        .map_err(wake_lookup_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workflow wake not found"))?;

    let recipient_hex = recipient.to_hex();
    let exact_tag = |event: &nostr::Event, name: &str, value: &str| {
        event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.len() == 2 && values[0] == name && values[1].eq_ignore_ascii_case(value)
        })
    };
    let message_channel = message
        .channel_id
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workflow wake not found"))?;
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &recipient.to_bytes(),
        auth_tag,
    )
    .await
    .map_err(|(status, body)| {
        // This shared boundary exposes database lookup failures as 500, while
        // explicit roster/authentication denials retain their terminal status.
        let status = if status == StatusCode::INTERNAL_SERVER_ERROR {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            status
        };
        (status, body)
    })?;
    if !state
        .db
        .is_member(tenant.community(), message_channel, &recipient.to_bytes())
        .await
        .map_err(wake_lookup_error)?
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "workflow wake not accessible",
        ));
    }
    if workflow.owner_pubkey != definition.event.pubkey.to_bytes()
        || workflow.channel_id != Some(message_channel)
        || !exact_tag(&definition.event, "h", &message_channel.to_string())
        || !exact_tag(&definition.event, "d", &run.workflow_id.to_string())
        || !exact_tag(&message.event, "h", &message_channel.to_string())
        || !message.event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.len() == 2 && values[0] == "p" && values[1].eq_ignore_ascii_case(&recipient_hex)
        })
        || !exact_tag(&message.event, "workflow-run", &run_id.to_string())
        || !exact_tag(
            &message.event,
            "workflow-definition",
            &definition.event.id.to_hex(),
        )
    {
        return Err(api_error(StatusCode::NOT_FOUND, "workflow wake not found"));
    }

    Ok(Json(serde_json::json!({
        "run_id": run.id,
        "channel_id": message_channel,
        "workflow_id": run.workflow_id,
        "definition_event_id": definition.event.id.to_hex(),
        "workflow_owner": hex::encode(&workflow.owner_pubkey),
        "definition": definition.event,
        "message": message.event,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn wake_lookup_failure_is_not_authority_revocation() {
        use buzz_db::DbError;
        for error in [
            sqlx::Error::PoolClosed,
            sqlx::Error::PoolTimedOut,
            sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ] {
            assert_eq!(
                wake_lookup_error(DbError::Sqlx(error)).0,
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        assert_eq!(
            wake_lookup_error(DbError::NotFound("run".into())).0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            wake_lookup_error(DbError::InvalidData("bad row".into())).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            wake_lookup_error(DbError::Sqlx(sqlx::Error::ColumnNotFound("bad".into()))).0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn channel_access_is_required_at_authority_read_time() {
        let channel = Uuid::new_v4();
        assert!(ensure_channel_access(&[channel], channel, "revoked").is_ok());

        let (status, _) = ensure_channel_access(&[], channel, "revoked")
            .expect_err("removed member must not retain authority read access");
        assert_eq!(status, StatusCode::FORBIDDEN);
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
}
