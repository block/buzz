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
use buzz_core::workflow_delivery::{
    DEFAULT_LEASE_SECONDS as DEFAULT_DELIVERY_LEASE_SECONDS,
    MAX_LEASE_SECONDS as MAX_DELIVERY_LEASE_SECONDS,
};

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
    let (pubkey, event_id_bytes) =
        bridge::verify_bridge_auth(headers, "GET", &url, None, state.config.require_auth_token)?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;

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
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "workflow is not accessible",
        ));
    }

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

/// Authenticated request to claim either a specific or the oldest due delivery.
#[derive(Debug, Deserialize)]
pub struct ClaimDeliveryRequest {
    #[serde(default)]
    delivery_id: Option<Uuid>,
    #[serde(default)]
    expected: Option<ClaimDeliveryBindingRequest>,
    #[serde(default = "default_delivery_lease_seconds")]
    lease_seconds: i64,
}

/// Immutable delivery bindings authenticated from a relay-authored live wake.
#[derive(Debug, Deserialize)]
pub struct ClaimDeliveryBindingRequest {
    run_id: Uuid,
    step_id: String,
    definition_event_id: String,
    message_event_id: String,
    channel_id: Uuid,
}

fn default_delivery_lease_seconds() -> i64 {
    DEFAULT_DELIVERY_LEASE_SECONDS
}

/// Authenticated request to extend a fenced delivery lease.
#[derive(Debug, Deserialize)]
pub struct RenewDeliveryRequest {
    claim_token: Uuid,
    lease_seconds: i64,
}

/// Authenticated completion result for a fenced delivery lease.
#[derive(Debug, Deserialize)]
pub struct FinishDeliveryRequest {
    claim_token: Uuid,
    delivered: bool,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    failure_code: Option<String>,
    #[serde(default)]
    failure_message: Option<String>,
}

async fn authorize_delivery_write(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    body: &[u8],
) -> Result<(TenantContext, nostr::PublicKey), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "relay community not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let (pubkey, event_id) = bridge::verify_bridge_auth(
        headers,
        "POST",
        &url,
        Some(body),
        state.config.require_auth_token,
    )?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id).await?;
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey.to_bytes(),
        auth_tag,
    )
    .await?;
    Ok((tenant, pubkey))
}

/// Claim the oldest due workflow delivery for the authenticated managed agent.
pub async fn claim_agent_delivery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = "/workflows/agent-deliveries/claim";
    let (tenant, agent) = authorize_delivery_write(&state, &headers, path, &body).await?;
    let request: ClaimDeliveryRequest = serde_json::from_slice(&body).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid claim JSON: {error}"),
        )
    })?;
    if !(DEFAULT_DELIVERY_LEASE_SECONDS..=MAX_DELIVERY_LEASE_SECONDS)
        .contains(&request.lease_seconds)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "lease_seconds is outside the supported range",
        ));
    }
    if request.delivery_id.is_none() && request.expected.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "expected wake bindings require a specific delivery_id",
        ));
    }
    let expected = request
        .expected
        .map(|binding| {
            let definition_event_id = hex::decode(&binding.definition_event_id)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid definition_event_id"))?;
            let message_event_id = hex::decode(&binding.message_event_id)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid message_event_id"))?;
            if definition_event_id.len() != 32
                || message_event_id.len() != 32
                || binding.step_id.is_empty()
            {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid expected wake bindings",
                ));
            }
            Ok(buzz_db::workflow::WorkflowAgentDeliveryBinding {
                run_id: binding.run_id,
                step_id: binding.step_id,
                definition_event_id,
                message_event_id,
                channel_id: binding.channel_id,
            })
        })
        .transpose()?;
    let delivery = state
        .db
        .claim_workflow_agent_delivery(
            tenant.community(),
            &agent.to_bytes(),
            request.delivery_id,
            expected.as_ref(),
            request.lease_seconds,
        )
        .await
        .map_err(|error| internal_error(&format!("claim workflow delivery: {error}")))?;
    Ok(Json(serde_json::json!({
        "delivery": delivery.as_ref().map(delivery_json),
    })))
}

/// Extend a workflow delivery lease using its owner/token fencing stamp.
pub async fn renew_agent_delivery(
    State(state): State<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/agent-deliveries/{delivery_id}/renew");
    let (tenant, agent) = authorize_delivery_write(&state, &headers, &path, &body).await?;
    let request: RenewDeliveryRequest = serde_json::from_slice(&body).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid renewal JSON: {error}"),
        )
    })?;
    if !(DEFAULT_DELIVERY_LEASE_SECONDS..=MAX_DELIVERY_LEASE_SECONDS)
        .contains(&request.lease_seconds)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "lease_seconds is outside the supported range",
        ));
    }
    let claim_expires_at = state
        .db
        .renew_workflow_agent_delivery(
            tenant.community(),
            delivery_id,
            &agent.to_bytes(),
            request.claim_token,
            request.lease_seconds,
        )
        .await
        .map_err(|error| internal_error(&format!("renew workflow delivery: {error}")))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "delivery claim is stale or expired"))?;
    Ok(Json(serde_json::json!({
        "renewed": true,
        "claim_expires_at": claim_expires_at,
    })))
}

/// Complete a workflow delivery using its lease token as a fencing stamp.
pub async fn finish_agent_delivery(
    State(state): State<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/agent-deliveries/{delivery_id}/finish");
    let (tenant, agent) = authorize_delivery_write(&state, &headers, &path, &body).await?;
    let request: FinishDeliveryRequest = serde_json::from_slice(&body).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            &format!("invalid finish JSON: {error}"),
        )
    })?;
    let completed = state
        .db
        .finish_workflow_agent_delivery(
            tenant.community(),
            delivery_id,
            &agent.to_bytes(),
            request.claim_token,
            request.delivered,
            request.retryable,
            request.failure_code.as_deref(),
            request.failure_message.as_deref(),
        )
        .await
        .map_err(|error| internal_error(&format!("finish workflow delivery: {error}")))?;
    if !completed {
        return Err(api_error(
            StatusCode::CONFLICT,
            "delivery claim is stale or expired",
        ));
    }
    Ok(Json(serde_json::json!({"completed": true})))
}

fn delivery_json(delivery: &buzz_db::workflow::WorkflowAgentDeliveryRecord) -> Value {
    serde_json::json!({
        "id": delivery.id,
        "workflow_id": delivery.workflow_id,
        "run_id": delivery.run_id,
        "step_id": delivery.step_id,
        "definition_event_id": hex::encode(&delivery.definition_event_id),
        "message_event_id": hex::encode(&delivery.message_event_id),
        "channel_id": delivery.channel_id,
        "target_pubkey": hex::encode(&delivery.target_pubkey),
        "attempt": delivery.attempt,
        "claim_token": delivery.claim_token,
        "claim_expires_at": delivery.claim_expires_at,
        "expires_at": delivery.expires_at,
        "execution_trace": delivery.execution_trace,
        "trigger_context": delivery.trigger_context,
    })
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
