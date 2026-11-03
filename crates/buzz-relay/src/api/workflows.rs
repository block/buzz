//! Authorized structured reads for workflow execution state.
//!
//! Runs and approvals are relay-owned database rows, not Nostr events. These
//! endpoints expose those read models without inventing synthetic events.

use std::sync::Arc;

use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use buzz_core::TenantContext;

use crate::{
    api::{api_error, bridge, internal_error, parse_query_or_400},
    nip_fi_http::admit_nip_fi_http_on_state,
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

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn authorize_workflow_read(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    path: &str,
    raw_query: Option<&str>,
    workflow_id: Uuid,
    method: &'static str,
) -> Result<(TenantContext, Vec<u8>), Response> {
    let tenant = crate::nip_fi_shadow::bind_tenant(state, headers)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
            .into_response()
        })?;

    let path_with_query = request_path(path, raw_query);
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path_with_query);
    // In NIP-FI enforce/deny-protected mode a real NIP-98 event is mandatory —
    // the X-Pubkey dev-mode fallback must never satisfy the pairing requirement.
    // [NIP-FI.md:594-607, FI-TRACE-HTTP-INGRESS]
    let nip_fi_active = state.config.nip_fi.mode.restricts();

    // NIP-FI admission. [FI-TRACE-AUTHORITY-UNIFORM]
    let require_auth = state.config.require_auth_token || nip_fi_active;
    let admission = admit_nip_fi_http_on_state(
        state,
        headers,
        bridge::make_nip98_closure_for_admission(
            headers.clone(),
            method,
            url,
            None,
            require_auth,
            false,
        ),
    )
    .await?;
    let pubkey = *admission.proven_pubkey();
    let token_owner = admission.token_owner();
    let (event_id_bytes, signed_created_at) = admission.into_extra();

    bridge::enforce_http_admission(state, &tenant, &pubkey)
        .await
        .map_err(|e| e.into_response())?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes)
        .await
        .map_err(|e| e.into_response())?;

    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let auth_tag = super::relay_members::extract_auth_tag_header(headers);
    super::relay_members::enforce_relay_membership(
        state,
        &tenant,
        &pubkey_bytes,
        auth_tag,
        signed_created_at,
        token_owner,
    )
    .await
    .map_err(|e| e.into_response())?;

    // A workflow the caller cannot read answers exactly like a missing one, so
    // the response never reveals which workflow IDs exist.
    let not_found = || api_error(StatusCode::NOT_FOUND, "workflow not found").into_response();
    let workflow = match state.db.get_workflow(tenant.community(), workflow_id).await {
        Ok(workflow) => workflow,
        Err(buzz_db::error::DbError::NotFound(_)) => return Err(not_found()),
        Err(other) => {
            return Err(
                internal_error(&format!("get workflow for run read: {other}")).into_response(),
            )
        }
    };
    let channel_id = workflow.channel_id.ok_or_else(not_found)?;
    let accessible = state
        .get_accessible_channel_ids_cached(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| {
            internal_error(&format!("workflow channel access lookup: {error}")).into_response()
        })?;
    // A workflow names its channel, so a readable named-only channel (a
    // system channel, or a type added later) counts.
    let readable = accessible.contains(&channel_id)
        || state
            .db
            .get_readable_named_only_channels(tenant.community(), &pubkey_bytes, &[channel_id])
            .await
            .map_err(|error| {
                internal_error(&format!(
                    "workflow named-only channel access lookup: {error}"
                ))
                .into_response()
            })?
            .iter()
            .any(|channel| channel.id == channel_id);
    if !readable {
        return Err(not_found());
    }

    Ok((tenant, pubkey_bytes))
}

/// `GET /workflows/{workflow_id}/runs` — one authorized, keyset-paginated page.
pub async fn workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<Uuid>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Response {
    workflow_runs_inner(state, workflow_id, headers, raw_query)
        .await
        .into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn workflow_runs_inner(
    state: Arc<AppState>,
    workflow_id: Uuid,
    headers: HeaderMap,
    raw_query: Option<String>,
) -> Result<Json<Value>, Response> {
    // Admission first: NIP-98 + NIP-FI must fire before any application-level
    // validation — including query-string parsing — so the denial contract wins
    // over request-validation errors. [FI-TRACE-HTTP-INGRESS]
    // Raw query is preserved here; parsing happens after admission.
    let path = format!("/workflows/{workflow_id}/runs");
    let (tenant, _) =
        authorize_workflow_read(&state, &headers, &path, raw_query.as_deref(), workflow_id, "GET").await?;

    // Parse query string after admission so malformed params (e.g. ?limit=abc)
    // cannot 400 before the NIP-FI gate fires.  A parse failure after
    // admission is a caller error (400); defaulting silently would change
    // query semantics.  [FI-TRACE-HTTP-INGRESS]
    let query: RunsQuery =
        parse_query_or_400(raw_query.as_deref()).map_err(|e| e.into_response())?;

    if query.before.is_some() != query.before_id.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "before and before_id must be supplied together",
        )
        .into_response());
    }
    let limit = query.limit.unwrap_or(DEFAULT_RUN_LIMIT);
    if !(1..=MAX_RUN_LIMIT).contains(&limit) {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "limit must be between 1 and 100").into_response(),
        );
    }

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
        .map_err(|error| internal_error(&format!("list workflow runs: {error}")).into_response())?;

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
) -> Response {
    run_approvals_inner(state, workflow_id, run_id, headers)
        .await
        .into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn run_approvals_inner(
    state: Arc<AppState>,
    workflow_id: Uuid,
    run_id: Uuid,
    headers: HeaderMap,
) -> Result<Json<Value>, Response> {
    let path = format!("/workflows/{workflow_id}/runs/{run_id}/approvals");
    let (tenant, _) = authorize_workflow_read(&state, &headers, &path, None, workflow_id, "GET").await?;

    let run = state
        .db
        .get_workflow_run(tenant.community(), run_id)
        .await
        .map_err(|error| match error {
            buzz_db::error::DbError::NotFound(_) => {
                api_error(StatusCode::NOT_FOUND, "workflow run not found").into_response()
            }
            other => internal_error(&format!("get workflow run for approval read: {other}"))
                .into_response(),
        })?;
    if run.workflow_id != workflow_id {
        return Err(api_error(StatusCode::NOT_FOUND, "workflow run not found").into_response());
    }

    let approvals = state
        .db
        .get_run_approvals(tenant.community(), workflow_id, run_id)
        .await
        .map_err(|error| internal_error(&format!("list run approvals: {error}")).into_response())?;
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

/// Body for the approval decide endpoint.
#[derive(Debug, Deserialize)]
pub struct DecideApprovalBody {
    /// `granted` or `denied`.
    decision: String,
    /// Optional note recorded on the approval.
    note: Option<String>,
}

/// `POST /workflows/{workflow_id}/runs/{run_id}/approvals/{approval_ref}/decide`
///
/// Grants or denies a pending approval gate (WF-08). On grant the run
/// resumes after the gate step; on deny the run fails. `approval_ref` is the
/// hex-encoded stored token hash from the approvals listing.
pub async fn decide_approval(
    State(state): State<Arc<AppState>>,
    Path((workflow_id, run_id, approval_ref)): Path<(Uuid, Uuid, String)>,
    headers: HeaderMap,
    axum::extract::Json(body): axum::extract::Json<DecideApprovalBody>,
) -> Response {
    decide_approval_inner(state, workflow_id, run_id, approval_ref, headers, body)
        .await
        .into_response()
}

async fn decide_approval_inner(
    state: Arc<AppState>,
    workflow_id: Uuid,
    run_id: Uuid,
    approval_ref: String,
    headers: HeaderMap,
    body: DecideApprovalBody,
) -> Result<Json<Value>, Response> {
    let path = format!("/workflows/{workflow_id}/runs/{run_id}/approvals/{approval_ref}/decide");
    let (tenant, pubkey_bytes) =
        authorize_workflow_read(&state, &headers, &path, None, workflow_id, "POST").await?;
    let community = tenant.community();

    let granted = match body.decision.as_str() {
        "granted" => true,
        "denied" => false,
        _ => {
            return Err(
                api_error(StatusCode::BAD_REQUEST, "decision must be granted or denied")
                    .into_response(),
            )
        }
    };

    let token_hash = hex::decode(approval_ref.trim()).map_err(|_| {
        api_error(StatusCode::BAD_REQUEST, "approval_ref is not hex").into_response()
    })?;
    if token_hash.len() != 32 {
        return Err(
            api_error(StatusCode::BAD_REQUEST, "approval_ref must be a 32-byte hash").into_response(),
        );
    }

    let run = state.db.get_workflow_run(community, run_id).await.map_err(|error| match error {
        buzz_db::error::DbError::NotFound(_) => {
            api_error(StatusCode::NOT_FOUND, "workflow run not found").into_response()
        }
        other => internal_error(&format!("get workflow run for decide: {other}")).into_response(),
    })?;
    if run.workflow_id != workflow_id {
        return Err(api_error(StatusCode::NOT_FOUND, "workflow run not found").into_response());
    }

    let approval = state
        .db
        .get_approval_by_stored_hash(community, &token_hash)
        .await
        .map_err(|error| match error {
            buzz_db::error::DbError::NotFound(_) => {
                api_error(StatusCode::NOT_FOUND, "approval not found").into_response()
            }
            other => internal_error(&format!("get approval for decide: {other}")).into_response(),
        })?;
    if approval.run_id != run_id || approval.workflow_id != workflow_id {
        return Err(api_error(StatusCode::NOT_FOUND, "approval not found").into_response());
    }
    if !matches!(
        approval.status,
        buzz_db::workflow::ApprovalStatus::Pending
    ) {
        return Err(api_error(StatusCode::CONFLICT, "approval is already decided").into_response());
    }
    if !matches!(run.status, buzz_db::workflow::RunStatus::WaitingApproval) {
        return Err(api_error(StatusCode::CONFLICT, "run is not waiting for approval").into_response());
    }
    if approval.expires_at <= Utc::now() {
        let _ = state
            .db
            .update_approval_by_stored_hash(
                community,
                &token_hash,
                buzz_db::workflow::ApprovalStatus::Expired,
                None,
                None,
            )
            .await;
        let _ = state
            .db
            .update_workflow_run(
                community,
                run_id,
                buzz_db::workflow::RunStatus::Failed,
                approval.step_index,
                &run.execution_trace,
                Some(buzz_db::workflow::WorkflowRunFailure {
                    code: "approval_expired",
                    message: "approval window elapsed without a decision",
                }),
            )
            .await;
        return Err(api_error(StatusCode::GONE, "approval window has expired").into_response());
    }

    if !approver_spec_satisfied(&approval.approver_spec, &hex::encode(&pubkey_bytes)) {
        return Err(
            api_error(StatusCode::FORBIDDEN, "approver is not authorized for this gate")
                .into_response(),
        );
    }

    let new_status = if granted {
        buzz_db::workflow::ApprovalStatus::Granted
    } else {
        buzz_db::workflow::ApprovalStatus::Denied
    };
    let updated = state
        .db
        .update_approval_by_stored_hash(
            community,
            &token_hash,
            new_status,
            Some(&pubkey_bytes),
            body.note.as_deref(),
        )
        .await
        .map_err(|error| internal_error(&format!("update approval: {error}")).into_response())?;
    if !updated {
        return Err(api_error(StatusCode::CONFLICT, "approval is already decided").into_response());
    }

    if !granted {
        state
            .db
            .update_workflow_run(
                community,
                run_id,
                buzz_db::workflow::RunStatus::Failed,
                approval.step_index,
                &run.execution_trace,
                Some(buzz_db::workflow::WorkflowRunFailure {
                    code: "approval_denied",
                    message: body.note.as_deref().unwrap_or("approval denied"),
                }),
            )
            .await
            .map_err(|error| internal_error(&format!("fail denied run: {error}")).into_response())?;
        return Ok(Json(serde_json::json!({
            "status": "denied",
            "run_id": run_id,
            "step_index": approval.step_index,
        })));
    }

    // Granted: resume after the gate step with pre-approval outputs restored.
    let workflow = state.db.get_workflow(community, workflow_id).await.map_err(|error| {
        internal_error(&format!("get workflow for resume: {error}")).into_response()
    })?;
    let def: buzz_workflow::WorkflowDef = serde_json::from_value(workflow.definition.clone())
        .map_err(|error| internal_error(&format!("parse workflow definition: {error}")).into_response())?;
    let trigger_ctx: buzz_workflow::executor::TriggerContext = match run.trigger_context.clone() {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| internal_error(&format!("parse trigger context: {error}")).into_response())?,
        None => buzz_workflow::executor::TriggerContext::default(),
    };
    let mut outputs = step_outputs_from_trace(&run.execution_trace);
    outputs.insert(
        approval.step_id.clone(),
        serde_json::json!({
            "approved": true,
            "approver": hex::encode(&pubkey_bytes),
            "note": body.note,
        }),
    );
    let existing_trace: Vec<Value> = run.execution_trace.as_array().cloned().unwrap_or_default();
    let resume_from = (approval.step_index + 1).max(0) as usize;

    let engine = Arc::clone(&state.workflow_engine);
    tokio::spawn(async move {
        let result = buzz_workflow::executor::execute_from_step(
            &engine,
            community,
            run_id,
            &def,
            &trigger_ctx,
            resume_from,
            Some(outputs),
        )
        .await;
        engine.finalize_run(community, run_id, result, Some(existing_trace)).await;
    });

    Ok(Json(serde_json::json!({
        "status": "resumed",
        "run_id": run_id,
        "resume_from_step": resume_from,
    })))
}

/// True when `proven_pubkey_hex` may act on a gate with this approver spec.
///
/// A 64-hex-char spec (optionally `@`-prefixed) names one pubkey and must
/// match exactly. Anything else is a role spec with no resolver yet, so any
/// admitted community reader may decide. Follow-up: resolve roles to pubkeys.
fn approver_spec_satisfied(spec: &str, proven_pubkey_hex: &str) -> bool {
    let trimmed = spec.strip_prefix('@').unwrap_or(spec).trim();
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return trimmed.eq_ignore_ascii_case(proven_pubkey_hex);
    }
    true
}

/// Rebuild pre-gate step outputs from the persisted execution trace.
///
/// Trace entries written by the executor carry `step_id`, `status`, and
/// `output`; only completed steps contribute outputs for `{{steps.*}}`.
fn step_outputs_from_trace(trace: &Value) -> std::collections::HashMap<String, Value> {
    let mut outputs = std::collections::HashMap::new();
    for entry in trace.as_array().cloned().unwrap_or_default() {
        let step_id = entry.get("step_id").and_then(Value::as_str);
        let status = entry.get("status").and_then(Value::as_str);
        if let (Some(id), Some("completed")) = (step_id, status) {
            if let Some(output) = entry.get("output").cloned() {
                outputs.insert(id.to_string(), output);
            }
        }
    }
    outputs
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

    #[test]
    fn approver_spec_pubkey_must_match_exactly() {
        let key = "ab".repeat(32);
        assert!(approver_spec_satisfied(&key, &key));
        assert!(approver_spec_satisfied(&format!("@{key}"), &key));
        assert!(approver_spec_satisfied(&key.to_uppercase(), &key));
        assert!(!approver_spec_satisfied(&"cd".repeat(32), &key));
    }

    #[test]
    fn approver_role_spec_falls_back_to_membership() {
        // No role resolver yet: any admitted reader may decide.
        assert!(approver_spec_satisfied("@release-manager", &"ab".repeat(32)));
        assert!(approver_spec_satisfied("any", &"ab".repeat(32)));
    }

    #[test]
    fn trace_outputs_rebuild_only_completed_steps() {
        let trace = serde_json::json!([
            {"step_id": "a", "status": "completed", "output": {"x": 1}},
            {"step_id": "b", "status": "skipped"},
            {"step_id": "c", "status": "completed", "output": [1, 2]},
        ]);
        let outputs = step_outputs_from_trace(&trace);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs["a"], serde_json::json!({"x": 1}));
        assert_eq!(outputs["c"], serde_json::json!([1, 2]));
        assert!(step_outputs_from_trace(&serde_json::json!({})).is_empty());
    }
}
