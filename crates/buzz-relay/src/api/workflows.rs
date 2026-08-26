//! Authorized structured reads for workflow execution state.
//!
//! Runs and approvals are relay-owned database rows, not Nostr events. These
//! endpoints expose those read models without inventing synthetic events.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use buzz_core::{
    workflow_delivery::{
        WorkflowDeliveryBinding, WorkflowDeliveryCause, WorkflowDeliveryId, WorkflowDeliveryReceipt,
    },
    TenantContext,
};

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
/// This narrow endpoint does not grant channel visibility; it returns only the
/// exact owner-signed definition event needed to construct a revision-bound
/// manual trigger.
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
    Ok(Json(serde_json::to_value(event.event).map_err(
        |error| internal_error(&format!("serialize workflow revision: {error}")),
    )?))
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

const DEFAULT_DELIVERY_LEASE_SECONDS: i64 = 60;
const MIN_DELIVERY_LEASE_SECONDS: i64 = 15;
const MAX_DELIVERY_LEASE_SECONDS: i64 = 300;

/// Full immutable selector for a specific durable delivery.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryBindingRequest {
    community_id: Uuid,
    workflow_id: Uuid,
    run_id: Uuid,
    step_id: String,
    target_pubkey: String,
    definition_event_id: String,
    message_event_id: String,
    cause: DeliveryCauseRequest,
}

/// Trigger authority identity carried by a delivery binding.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeliveryCauseRequest {
    /// Exact signed event which caused the run.
    Event {
        /// Trigger event identifier.
        event_id: String,
    },
    /// Exact scheduled firing slot which caused the run.
    Schedule {
        /// Authoritative Unix-second schedule slot.
        scheduled_for_unix_seconds: i64,
    },
    /// Opaque server-side webhook invocation identity.
    Webhook {
        /// Invocation identifier; no webhook secret or payload.
        invocation_id: Uuid,
    },
}

/// Request to claim a specific delivery or poll the oldest pending delivery.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimDeliveryRequest {
    #[serde(default)]
    delivery_id: Option<Uuid>,
    #[serde(default)]
    expected: Option<DeliveryBindingRequest>,
    #[serde(default = "default_delivery_lease_seconds")]
    lease_seconds: i64,
}

/// Fenced lease capability presented to read, renew, or finish a delivery.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryLeaseRequest {
    lease_generation: i64,
    binding: DeliveryBindingRequest,
}

/// Request to extend a delivery lease.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenewDeliveryRequest {
    lease_generation: i64,
    binding: DeliveryBindingRequest,
    #[serde(default = "default_delivery_lease_seconds")]
    lease_seconds: i64,
}

/// Terminal disposition for a delivery.
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryDisposition {
    /// Delivery work completed successfully.
    Finished,
    /// Delivery work failed permanently.
    Failed,
}

/// Request to settle a delivery under its fenced lease.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishDeliveryRequest {
    lease_generation: i64,
    binding: DeliveryBindingRequest,
    disposition: DeliveryDisposition,
}

fn default_delivery_lease_seconds() -> i64 {
    DEFAULT_DELIVERY_LEASE_SECONDS
}

fn validate_lease_seconds(value: i64) -> Result<i64, (StatusCode, Json<Value>)> {
    if !(MIN_DELIVERY_LEASE_SECONDS..=MAX_DELIVERY_LEASE_SECONDS).contains(&value) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "lease_seconds must be between 15 and 300",
        ));
    }
    Ok(value)
}

fn parse_event_id(value: &str, field: &str) -> Result<nostr::EventId, (StatusCode, Json<Value>)> {
    nostr::EventId::from_hex(value)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, &format!("invalid {field}")))
}

fn parse_delivery_binding(
    request: DeliveryBindingRequest,
    tenant: &TenantContext,
    authenticated: nostr::PublicKey,
) -> Result<WorkflowDeliveryBinding, (StatusCode, Json<Value>)> {
    if request.community_id != *tenant.community().as_uuid() {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    }
    let target = nostr::PublicKey::from_hex(&request.target_pubkey)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid target_pubkey"))?;
    if target != authenticated {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    }
    let cause = match request.cause {
        DeliveryCauseRequest::Event { event_id } => {
            WorkflowDeliveryCause::Event(parse_event_id(&event_id, "cause.event_id")?)
        }
        DeliveryCauseRequest::Schedule {
            scheduled_for_unix_seconds,
        } => WorkflowDeliveryCause::Schedule {
            scheduled_for_unix_seconds,
        },
        DeliveryCauseRequest::Webhook { invocation_id } => {
            WorkflowDeliveryCause::Webhook { invocation_id }
        }
    };
    WorkflowDeliveryBinding::new(
        tenant.community(),
        request.workflow_id,
        request.run_id,
        request.step_id,
        target,
        parse_event_id(&request.definition_event_id, "definition_event_id")?,
        parse_event_id(&request.message_event_id, "message_event_id")?,
        cause,
    )
    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid delivery binding"))
}

async fn authorize_delivery_request(
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
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "delivery not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let (pubkey, event_id) = bridge::verify_bridge_auth_with_options(
        headers,
        "POST",
        &url,
        Some(body),
        state.config.require_auth_token,
        true,
    )?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id).await?;
    let pubkey_bytes = pubkey.to_bytes();
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
    let (_, owner) = state
        .db
        .get_agent_channel_policy(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| internal_error(&format!("delivery target lookup: {error}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "delivery not found"))?;
    if owner.is_none() {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    }
    Ok((tenant, pubkey))
}

async fn validate_binding_channel(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    binding: &WorkflowDeliveryBinding,
) -> Result<(), (StatusCode, Json<Value>)> {
    let message = state
        .db
        .get_event_by_id(tenant.community(), binding.message_event_id().as_bytes())
        .await
        .map_err(|error| internal_error(&format!("delivery message lookup: {error}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "delivery not found"))?;
    let Some(channel_id) = message.channel_id else {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    };
    let workflow = state
        .db
        .get_workflow(tenant.community(), binding.workflow_id())
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "delivery not found"))?;
    if workflow.channel_id != Some(channel_id) {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    }
    Ok(())
}

async fn current_bound_delivery(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    target: nostr::PublicKey,
    delivery_id: WorkflowDeliveryId,
    lease_generation: i64,
    requested: DeliveryBindingRequest,
    require_live_claim: bool,
) -> Result<buzz_db::workflow::WorkflowAgentDeliveryRecord, (StatusCode, Json<Value>)> {
    let delivery = state
        .db
        .get_workflow_agent_delivery(tenant.community(), delivery_id)
        .await
        .map_err(|error| internal_error(&format!("read workflow delivery: {error}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "delivery not found"))?;
    let binding = parse_delivery_binding(requested, tenant, target)?;
    let live_claim = delivery.status == buzz_db::workflow::WorkflowDeliveryStatus::Claimed
        && delivery
            .lease_until
            .is_some_and(|until| until >= Utc::now());
    if delivery.binding != binding
        || delivery.lease_generation != lease_generation
        || (require_live_claim && !live_claim)
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "delivery lease is not current",
        ));
    }
    // Resolve message/channel authority only after the caller has demonstrated
    // the complete delivery binding. Otherwise this endpoint becomes an event
    // existence oracle for arbitrary managed-agent identities.
    validate_binding_channel(state, tenant, &binding).await?;
    Ok(delivery)
}

fn lease_for(
    tenant: &TenantContext,
    target: nostr::PublicKey,
    delivery_id: WorkflowDeliveryId,
    lease_generation: i64,
) -> buzz_db::workflow::WorkflowDeliveryLease {
    buzz_db::workflow::WorkflowDeliveryLease {
        community_id: tenant.community(),
        delivery_id,
        target_pubkey: target,
        lease_generation,
        // C's mutation queries fence on community/id/target/generation and the
        // database clock. This field is returned state, not client authority.
        lease_until: Utc::now(),
    }
}

/// `POST /workflows/agent-deliveries/claim` — claim only as the immutable
/// managed-agent target. A wake can supply identifiers but grants no authority.
pub async fn claim_agent_delivery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = "/workflows/agent-deliveries/claim";
    let (tenant, target) = authorize_delivery_request(&state, &headers, path, &body).await?;
    let request: ClaimDeliveryRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid delivery claim JSON"))?;
    let lease_seconds = validate_lease_seconds(request.lease_seconds)?;
    if request.delivery_id.is_none() && request.expected.is_some() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "expected binding requires delivery_id",
        ));
    }
    let delivery_id = request.delivery_id.map(WorkflowDeliveryId::from_uuid);
    let expected = request
        .expected
        .map(|expected| parse_delivery_binding(expected, &tenant, target))
        .transpose()?;
    if let Some(binding) = expected.as_ref() {
        validate_binding_channel(&state, &tenant, binding).await?;
    }
    let claimed = state
        .db
        .claim_workflow_agent_delivery(
            tenant.community(),
            &target,
            delivery_id,
            expected.as_ref(),
            lease_seconds,
        )
        .await
        .map_err(|error| internal_error(&format!("claim workflow delivery: {error}")))?;
    let Some((lease, delivery)) = claimed else {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    };
    delivery_response(&state, &tenant, &target, &lease, &delivery).await
}

/// `POST /workflows/agent-deliveries/{id}/read` — return private execution
/// inputs only to the exact target holding the current live fencing token.
pub async fn read_agent_delivery(
    State(state): State<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/agent-deliveries/{delivery_id}/read");
    let (tenant, target) = authorize_delivery_request(&state, &headers, &path, &body).await?;
    let request: DeliveryLeaseRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid delivery read JSON"))?;
    let id = WorkflowDeliveryId::from_uuid(delivery_id);
    let delivery = current_bound_delivery(
        &state,
        &tenant,
        target,
        id,
        request.lease_generation,
        request.binding,
        true,
    )
    .await?;
    let lease = lease_for(&tenant, target, id, request.lease_generation);
    delivery_response(&state, &tenant, &target, &lease, &delivery).await
}

/// `POST /workflows/agent-deliveries/{id}/renew` — extend the current fenced lease.
pub async fn renew_agent_delivery(
    State(state): State<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/agent-deliveries/{delivery_id}/renew");
    let (tenant, target) = authorize_delivery_request(&state, &headers, &path, &body).await?;
    let request: RenewDeliveryRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid delivery renewal JSON"))?;
    let lease_seconds = validate_lease_seconds(request.lease_seconds)?;
    let id = WorkflowDeliveryId::from_uuid(delivery_id);
    current_bound_delivery(
        &state,
        &tenant,
        target,
        id,
        request.lease_generation,
        request.binding,
        true,
    )
    .await?;
    let lease = lease_for(&tenant, target, id, request.lease_generation);
    match state
        .db
        .renew_workflow_agent_delivery(&lease, lease_seconds)
        .await
        .map_err(|error| internal_error(&format!("renew workflow delivery: {error}")))?
    {
        buzz_db::workflow::WorkflowDeliveryRenewOutcome::Renewed(lease_until) => Ok(Json(
            serde_json::json!({"renewed": true, "lease_until": lease_until}),
        )),
        buzz_db::workflow::WorkflowDeliveryRenewOutcome::LeaseLost => Err(api_error(
            StatusCode::CONFLICT,
            "delivery lease is not current",
        )),
    }
}

/// `POST /workflows/agent-deliveries/{id}/finish` — settle once, with stable
/// acknowledgement only when a terminal replay has the identical disposition.
pub async fn finish_agent_delivery(
    State(state): State<Arc<AppState>>,
    Path(delivery_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/agent-deliveries/{delivery_id}/finish");
    let (tenant, target) = authorize_delivery_request(&state, &headers, &path, &body).await?;
    let request: FinishDeliveryRequest = serde_json::from_slice(&body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid delivery finish JSON"))?;
    let id = WorkflowDeliveryId::from_uuid(delivery_id);
    current_bound_delivery(
        &state,
        &tenant,
        target,
        id,
        request.lease_generation,
        request.binding,
        false,
    )
    .await?;
    let requested = match request.disposition {
        DeliveryDisposition::Finished => buzz_db::workflow::WorkflowDeliveryOutcome::Finished,
        DeliveryDisposition::Failed => buzz_db::workflow::WorkflowDeliveryOutcome::Failed,
    };
    let lease = lease_for(&tenant, target, id, request.lease_generation);
    match state
        .db
        .finish_workflow_agent_delivery(&lease, requested)
        .await
        .map_err(|error| internal_error(&format!("finish workflow delivery: {error}")))?
    {
        buzz_db::workflow::WorkflowDeliveryFinishOutcome::Settled(_) => Ok(Json(
            serde_json::json!({"finished": true, "replayed": false}),
        )),
        buzz_db::workflow::WorkflowDeliveryFinishOutcome::AlreadyTerminal(status)
            if terminal_matches(status, request.disposition) =>
        {
            Ok(Json(
                serde_json::json!({"finished": true, "replayed": true}),
            ))
        }
        buzz_db::workflow::WorkflowDeliveryFinishOutcome::AlreadyTerminal(_)
        | buzz_db::workflow::WorkflowDeliveryFinishOutcome::LeaseLost => Err(api_error(
            StatusCode::CONFLICT,
            "delivery lease is not current",
        )),
    }
}

fn terminal_matches(
    status: buzz_db::workflow::WorkflowDeliveryStatus,
    disposition: DeliveryDisposition,
) -> bool {
    matches!(
        (status, disposition),
        (
            buzz_db::workflow::WorkflowDeliveryStatus::Finished,
            DeliveryDisposition::Finished
        ) | (
            buzz_db::workflow::WorkflowDeliveryStatus::Failed,
            DeliveryDisposition::Failed
        )
    )
}

async fn validate_delivery_cause_authority(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    binding: &WorkflowDeliveryBinding,
) -> Result<(), (StatusCode, Json<Value>)> {
    let authoritative_run = match binding.cause() {
        WorkflowDeliveryCause::Event(event_id) => {
            let event = state
                .db
                .get_event_by_id(tenant.community(), event_id.as_bytes())
                .await
                .map_err(|error| internal_error(&format!("delivery cause lookup: {error}")))?
                .ok_or_else(|| {
                    api_error(StatusCode::CONFLICT, "delivery authority is unavailable")
                })?;
            if event.event.verify().is_err() {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "delivery authority is unavailable",
                ));
            }
            return Ok(());
        }
        WorkflowDeliveryCause::Schedule {
            scheduled_for_unix_seconds,
        } => {
            let scheduled_for =
                chrono::DateTime::<Utc>::from_timestamp(*scheduled_for_unix_seconds, 0)
                    .ok_or_else(|| {
                        api_error(StatusCode::CONFLICT, "delivery authority is unavailable")
                    })?;
            state
                .db
                .get_scheduled_workflow_fire_run(
                    tenant.community(),
                    binding.workflow_id(),
                    scheduled_for,
                )
                .await
                .map_err(|error| internal_error(&format!("schedule authority lookup: {error}")))?
        }
        WorkflowDeliveryCause::Webhook { invocation_id } => state
            .db
            .get_workflow_webhook_invocation_run(
                tenant.community(),
                binding.workflow_id(),
                *invocation_id,
            )
            .await
            .map_err(|error| internal_error(&format!("webhook authority lookup: {error}")))?,
    };
    if authoritative_run != Some(binding.run_id()) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "delivery authority is unavailable",
        ));
    }
    Ok(())
}

async fn delivery_response(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    target: &nostr::PublicKey,
    lease: &buzz_db::workflow::WorkflowDeliveryLease,
    delivery: &buzz_db::workflow::WorkflowAgentDeliveryRecord,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if delivery.binding.community_id() != tenant.community()
        || delivery.binding.target_pubkey() != *target
        || delivery.id != lease.delivery_id
        || delivery.lease_generation != lease.lease_generation
    {
        return Err(api_error(StatusCode::NOT_FOUND, "delivery not found"));
    }
    let run = state
        .db
        .get_workflow_run(tenant.community(), delivery.binding.run_id())
        .await
        .map_err(|error| internal_error(&format!("delivery run lookup: {error}")))?;
    if run.workflow_id != delivery.binding.workflow_id()
        || run.definition_event_id.as_deref()
            != Some(delivery.binding.definition_event_id().as_bytes())
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "delivery binding is unavailable",
        ));
    }
    let definition = state
        .db
        .get_event_by_id(
            tenant.community(),
            delivery.binding.definition_event_id().as_bytes(),
        )
        .await
        .map_err(|error| internal_error(&format!("delivery definition lookup: {error}")))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "delivery binding is unavailable"))?;
    let message = state
        .db
        .get_event_by_id(
            tenant.community(),
            delivery.binding.message_event_id().as_bytes(),
        )
        .await
        .map_err(|error| internal_error(&format!("delivery message lookup: {error}")))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "delivery binding is unavailable"))?;
    validate_delivery_cause_authority(state, tenant, &delivery.binding).await?;
    let receipt =
        WorkflowDeliveryReceipt::new(delivery.id, delivery.binding.clone(), &message.event)
            .and_then(|receipt| receipt.sign(&state.relay_keypair, &message.event))
            .map_err(|error| internal_error(&format!("sign delivery receipt: {error}")))?;
    Ok(Json(serde_json::json!({
        "delivery": delivery_json(delivery, lease),
        "definition_event": definition.event,
        "message_event": message.event,
        "receipt_event": receipt,
    })))
}

fn delivery_json(
    delivery: &buzz_db::workflow::WorkflowAgentDeliveryRecord,
    lease: &buzz_db::workflow::WorkflowDeliveryLease,
) -> Value {
    let binding = &delivery.binding;
    serde_json::json!({
        "id": delivery.id.as_uuid(),
        "community_id": binding.community_id().as_uuid(),
        "workflow_id": binding.workflow_id(),
        "run_id": binding.run_id(),
        "step_id": binding.step_id(),
        "target_pubkey": binding.target_pubkey().to_hex(),
        "definition_event_id": binding.definition_event_id().to_hex(),
        "message_event_id": binding.message_event_id().to_hex(),
        "cause": delivery_cause_json(binding.cause()),
        "lease_generation": lease.lease_generation,
        "lease_until": lease.lease_until,
    })
}

fn delivery_cause_json(cause: &WorkflowDeliveryCause) -> Value {
    match cause {
        WorkflowDeliveryCause::Event(event_id) => {
            serde_json::json!({"kind": "event", "event_id": event_id.to_hex()})
        }
        WorkflowDeliveryCause::Schedule {
            scheduled_for_unix_seconds,
        } => serde_json::json!({
            "kind": "schedule",
            "scheduled_for_unix_seconds": scheduled_for_unix_seconds,
        }),
        WorkflowDeliveryCause::Webhook { invocation_id } => {
            serde_json::json!({"kind": "webhook", "invocation_id": invocation_id})
        }
    }
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
    use std::sync::Arc;

    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{header, Request},
    };
    use base64::Engine;
    use buzz_core::workflow_delivery::WorkflowDeliveryBinding;
    use buzz_db::{
        channel::{ChannelType, ChannelVisibility},
        workflow::WorkflowAgentDelivery,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};
    use sqlx::PgPool;
    use tower::ServiceExt;

    use crate::{router::build_router, state::AppState};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

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

    struct DeliveryRouteFixture {
        state: Arc<AppState>,
        pool: PgPool,
        host: String,
        community: buzz_core::CommunityId,
        target: Keys,
        other_target: Keys,
        binding: WorkflowDeliveryBinding,
        delivery_id: WorkflowDeliveryId,
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
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&event).expect("serialize NIP-98 event"));
        format!("Nostr {encoded}")
    }

    async fn post_delivery_json(
        fixture: &DeliveryRouteFixture,
        keys: &Keys,
        path: &str,
        body: Value,
    ) -> axum::response::Response {
        let body = body.to_string();
        let url = format!("https://{}{path}", fixture.host);
        let auth = nip98_auth_header(keys, &url, body.as_bytes());
        build_router(Arc::clone(&fixture.state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::HOST, &fixture.host)
                    .header(header::AUTHORIZATION, auth)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("response JSON")
    }

    fn binding_json(binding: &WorkflowDeliveryBinding) -> Value {
        let cause = match binding.cause() {
            WorkflowDeliveryCause::Event(event_id) => {
                serde_json::json!({"kind": "event", "event_id": event_id.to_hex()})
            }
            WorkflowDeliveryCause::Schedule {
                scheduled_for_unix_seconds,
            } => serde_json::json!({
                "kind": "schedule",
                "scheduled_for_unix_seconds": scheduled_for_unix_seconds,
            }),
            WorkflowDeliveryCause::Webhook { invocation_id } => {
                serde_json::json!({"kind": "webhook", "invocation_id": invocation_id})
            }
        };
        serde_json::json!({
            "community_id": binding.community_id().as_uuid(),
            "workflow_id": binding.workflow_id(),
            "run_id": binding.run_id(),
            "step_id": binding.step_id(),
            "target_pubkey": binding.target_pubkey().to_hex(),
            "definition_event_id": binding.definition_event_id().to_hex(),
            "message_event_id": binding.message_event_id().to_hex(),
            "cause": cause,
        })
    }

    async fn delivery_route_fixture() -> Option<DeliveryRouteFixture> {
        let mut config = crate::config::Config::from_env().ok()?;
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        config.database_url = database_url.clone();
        config.redis_url = "redis://127.0.0.1:6379".to_owned();
        let host = format!("workflow-delivery-{}.example", Uuid::new_v4().simple());
        config.relay_url = format!("wss://{host}");
        config.require_relay_membership = false;

        let pool = PgPool::connect(&database_url).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        db.migrate().await.ok()?;
        let community = db.ensure_configured_community(&host).await.ok()?.id;
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
            db.clone(),
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

        let owner = Keys::generate();
        let target = Keys::generate();
        let other_target = Keys::generate();
        for keys in [&owner, &target, &other_target] {
            db.ensure_user(community, keys.public_key().as_bytes())
                .await
                .ok()?;
        }
        db.set_agent_owner(
            community,
            target.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .ok()?;
        db.set_agent_owner(
            community,
            other_target.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .ok()?;
        let channel = db
            .create_channel(
                community,
                "delivery-route-test",
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                owner.public_key().as_bytes(),
                Some(3600),
            )
            .await
            .ok()?;
        let workflow_id = db
            .create_workflow(
                community,
                Some(channel.id),
                owner.public_key().as_bytes(),
                "delivery-route-test",
                r#"{"name":"delivery-route-test","on":{"manual":{}},"steps":[]}"#,
                &[0x44; 32],
            )
            .await
            .ok()?;
        let definition = EventBuilder::new(Kind::Custom(30620), "signed definition")
            .tag(Tag::parse(["d", workflow_id.to_string().as_str()]).ok()?)
            .sign_with_keys(&owner)
            .ok()?;
        let message = EventBuilder::new(Kind::Custom(9), "private execution input")
            .tags([
                Tag::parse(["h", channel.id.to_string().as_str()]).ok()?,
                Tag::parse([
                    "p",
                    target.public_key().to_hex().as_str(),
                    "",
                    buzz_core::workflow_delivery::WORKFLOW_DELIVERY_TARGET_MARKER,
                ])
                .ok()?,
            ])
            .sign_with_keys(&owner)
            .ok()?;
        db.insert_event(community, &definition, Some(channel.id))
            .await
            .ok()?;
        db.insert_event(community, &message, Some(channel.id))
            .await
            .ok()?;
        sqlx::query(
            "UPDATE workflows SET definition_event_id = $3 WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(workflow_id)
        .bind(definition.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .ok()?;
        let run_id = db
            .create_workflow_run(
                community,
                workflow_id,
                definition.id.as_bytes(),
                Some(message.id.as_bytes()),
                None,
            )
            .await
            .ok()?;
        let binding = WorkflowDeliveryBinding::new(
            community,
            workflow_id,
            run_id,
            "notify",
            target.public_key(),
            definition.id,
            message.id,
            WorkflowDeliveryCause::Event(message.id),
        )
        .ok()?;
        let delivery_id = WorkflowDeliveryId::from_uuid(Uuid::new_v4());
        let (transaction, existing) = buzz_db::workflow::lock_workflow_agent_delivery_identity(
            &pool, community, run_id, "notify",
        )
        .await
        .ok()?;
        if existing {
            return None;
        }
        let stored_message = db
            .get_event_by_id(community, message.id.as_bytes())
            .await
            .ok()??;
        buzz_db::workflow::commit_workflow_agent_deliveries(
            transaction,
            community,
            DateTime::<Utc>::from_timestamp(stored_message.event.created_at.as_secs() as i64, 0)
                .expect("message timestamp"),
            &[WorkflowAgentDelivery {
                id: delivery_id,
                binding: binding.clone(),
            }],
        )
        .await
        .ok()?;

        Some(DeliveryRouteFixture {
            state: Arc::new(state),
            pool,
            host,
            community,
            target,
            other_target,
            binding,
            delivery_id,
        })
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authenticated_delivery_routes_complete_the_fenced_lifecycle() {
        let Some(fixture) = delivery_route_fixture().await else {
            eprintln!("skipping: Postgres unavailable");
            return;
        };
        let binding = binding_json(&fixture.binding);
        let claim = post_delivery_json(
            &fixture,
            &fixture.target,
            "/workflows/agent-deliveries/claim",
            serde_json::json!({
                "delivery_id": fixture.delivery_id.as_uuid(),
                "expected": binding,
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(claim.status(), StatusCode::OK);
        let claimed = response_json(claim).await;
        assert_eq!(claimed["delivery"]["lease_generation"], 1);
        assert_eq!(claimed["definition_event"]["content"], "signed definition");
        assert_eq!(
            claimed["message_event"]["content"],
            "private execution input"
        );
        assert!(claimed.get("execution_trace").is_none());
        assert!(claimed.get("trigger_context").is_none());
        assert!(claimed.get("webhook_fields").is_none());
        let receipt: nostr::Event =
            serde_json::from_value(claimed["receipt_event"].clone()).expect("signed receipt event");
        assert_eq!(receipt.pubkey, fixture.state.relay_keypair.public_key());
        assert!(receipt.verify().is_ok());
        assert!(receipt.content.is_empty());
        assert!(!claimed["receipt_event"]
            .to_string()
            .contains("private execution input"));

        let lease = serde_json::json!({
            "lease_generation": 1,
            "binding": binding_json(&fixture.binding),
        });
        let path = format!("/workflows/agent-deliveries/{}/read", fixture.delivery_id);
        let read = post_delivery_json(&fixture, &fixture.target, &path, lease.clone()).await;
        assert_eq!(read.status(), StatusCode::OK);

        let path = format!("/workflows/agent-deliveries/{}/renew", fixture.delivery_id);
        let renew = post_delivery_json(
            &fixture,
            &fixture.target,
            &path,
            serde_json::json!({
                "lease_generation": 1,
                "binding": binding_json(&fixture.binding),
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(renew.status(), StatusCode::OK);
        assert_eq!(response_json(renew).await["renewed"], true);

        let path = format!("/workflows/agent-deliveries/{}/finish", fixture.delivery_id);
        let finish_body = serde_json::json!({
            "lease_generation": 1,
            "binding": binding_json(&fixture.binding),
            "disposition": "finished",
        });
        let finish =
            post_delivery_json(&fixture, &fixture.target, &path, finish_body.clone()).await;
        assert_eq!(finish.status(), StatusCode::OK);
        assert_eq!(response_json(finish).await["replayed"], false);
        let replay = post_delivery_json(&fixture, &fixture.target, &path, finish_body).await;
        assert_eq!(replay.status(), StatusCode::OK);
        assert_eq!(response_json(replay).await["replayed"], true);
        let conflicting = post_delivery_json(
            &fixture,
            &fixture.target,
            &path,
            serde_json::json!({
                "lease_generation": 1,
                "binding": binding_json(&fixture.binding),
                "disposition": "failed",
            }),
        )
        .await;
        assert_eq!(conflicting.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authenticated_delivery_routes_reject_wrong_authority_and_stale_fences() {
        let Some(fixture) = delivery_route_fixture().await else {
            eprintln!("skipping: Postgres unavailable");
            return;
        };
        let claim_path = "/workflows/agent-deliveries/claim";
        let wrong_target = post_delivery_json(
            &fixture,
            &fixture.other_target,
            claim_path,
            serde_json::json!({
                "delivery_id": fixture.delivery_id.as_uuid(),
                "expected": binding_json(&fixture.binding),
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(wrong_target.status(), StatusCode::NOT_FOUND);

        let mut wrong_tenant = binding_json(&fixture.binding);
        wrong_tenant["community_id"] = serde_json::json!(Uuid::new_v4());
        let response = post_delivery_json(
            &fixture,
            &fixture.target,
            claim_path,
            serde_json::json!({
                "delivery_id": fixture.delivery_id.as_uuid(),
                "expected": wrong_tenant,
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let mut wrong_binding = binding_json(&fixture.binding);
        wrong_binding["step_id"] = serde_json::json!("other-step");
        let response = post_delivery_json(
            &fixture,
            &fixture.target,
            claim_path,
            serde_json::json!({
                "delivery_id": fixture.delivery_id.as_uuid(),
                "expected": wrong_binding,
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let claim = post_delivery_json(
            &fixture,
            &fixture.target,
            claim_path,
            serde_json::json!({
                "delivery_id": fixture.delivery_id.as_uuid(),
                "expected": binding_json(&fixture.binding),
                "lease_seconds": 60,
            }),
        )
        .await;
        assert_eq!(claim.status(), StatusCode::OK);
        let read_path = format!("/workflows/agent-deliveries/{}/read", fixture.delivery_id);
        let stale_generation = post_delivery_json(
            &fixture,
            &fixture.target,
            &read_path,
            serde_json::json!({
                "lease_generation": 0,
                "binding": binding_json(&fixture.binding),
            }),
        )
        .await;
        assert_eq!(stale_generation.status(), StatusCode::CONFLICT);

        sqlx::query(
            "UPDATE workflow_agent_deliveries SET lease_until = NOW() - INTERVAL '1 second' WHERE community_id = $1 AND id = $2",
        )
        .bind(fixture.community.as_uuid())
        .bind(fixture.delivery_id.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("expire delivery lease");
        let expired = post_delivery_json(
            &fixture,
            &fixture.target,
            &read_path,
            serde_json::json!({
                "lease_generation": 1,
                "binding": binding_json(&fixture.binding),
            }),
        )
        .await;
        assert_eq!(expired.status(), StatusCode::CONFLICT);
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
    fn delivery_wire_excludes_private_run_state() {
        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());
        let target = nostr::Keys::generate().public_key();
        let binding = WorkflowDeliveryBinding::new(
            community,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "notify".to_owned(),
            target,
            nostr::EventId::from_byte_array([0x11; 32]),
            nostr::EventId::from_byte_array([0x22; 32]),
            WorkflowDeliveryCause::Webhook {
                invocation_id: Uuid::new_v4(),
            },
        )
        .expect("valid delivery binding");
        let delivery = buzz_db::workflow::WorkflowAgentDeliveryRecord {
            id: WorkflowDeliveryId::from_uuid(Uuid::new_v4()),
            binding,
            status: buzz_db::workflow::WorkflowDeliveryStatus::Claimed,
            lease_generation: 4,
            lease_until: Some(Utc::now()),
            claimed_at: Some(Utc::now()),
            finished_at: None,
            created_at: Utc::now(),
        };
        let lease = buzz_db::workflow::WorkflowDeliveryLease {
            community_id: community,
            delivery_id: delivery.id,
            target_pubkey: target,
            lease_generation: 4,
            lease_until: Utc::now(),
        };

        let wire = serde_json::json!({
            "delivery": delivery_json(&delivery, &lease),
            "definition_event": {"content": "signed definition"},
            "message_event": {"content": "visible message"},
        });
        assert!(wire.get("execution_trace").is_none());
        assert!(wire.get("trigger_context").is_none());
        assert!(wire.get("webhook_fields").is_none());
        assert_eq!(wire["delivery"]["lease_generation"], 4);
    }

    #[test]
    fn delivery_binding_rejects_cross_tenant_and_cross_target() {
        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());
        let tenant = TenantContext::resolved(community, "agent.example");
        let authenticated = nostr::Keys::generate().public_key();
        let other = nostr::Keys::generate().public_key();
        let request = |community_id, target_pubkey: String| DeliveryBindingRequest {
            community_id,
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "notify".to_owned(),
            target_pubkey,
            definition_event_id: "11".repeat(32),
            message_event_id: "22".repeat(32),
            cause: DeliveryCauseRequest::Webhook {
                invocation_id: Uuid::new_v4(),
            },
        };

        let (status, _) = parse_delivery_binding(
            request(Uuid::new_v4(), authenticated.to_hex()),
            &tenant,
            authenticated,
        )
        .expect_err("body tenant cannot override host-resolved tenant");
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = parse_delivery_binding(
            request(*community.as_uuid(), other.to_hex()),
            &tenant,
            authenticated,
        )
        .expect_err("authenticated target must match the binding");
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn malformed_binding_and_lease_bounds_fail_closed() {
        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());
        let tenant = TenantContext::resolved(community, "agent.example");
        let authenticated = nostr::Keys::generate().public_key();
        let malformed = DeliveryBindingRequest {
            community_id: *community.as_uuid(),
            workflow_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            step_id: "notify".to_owned(),
            target_pubkey: authenticated.to_hex(),
            definition_event_id: "not-an-event-id".to_owned(),
            message_event_id: "22".repeat(32),
            cause: DeliveryCauseRequest::Webhook {
                invocation_id: Uuid::new_v4(),
            },
        };
        let (status, _) = parse_delivery_binding(malformed, &tenant, authenticated)
            .expect_err("malformed signed authority must fail closed");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(validate_lease_seconds(MIN_DELIVERY_LEASE_SECONDS - 1).is_err());
        assert!(validate_lease_seconds(MAX_DELIVERY_LEASE_SECONDS + 1).is_err());
    }

    #[test]
    fn terminal_replay_requires_the_same_disposition() {
        assert!(terminal_matches(
            buzz_db::workflow::WorkflowDeliveryStatus::Finished,
            DeliveryDisposition::Finished,
        ));
        assert!(terminal_matches(
            buzz_db::workflow::WorkflowDeliveryStatus::Failed,
            DeliveryDisposition::Failed,
        ));
        assert!(!terminal_matches(
            buzz_db::workflow::WorkflowDeliveryStatus::Finished,
            DeliveryDisposition::Failed,
        ));
        assert!(!terminal_matches(
            buzz_db::workflow::WorkflowDeliveryStatus::Failed,
            DeliveryDisposition::Finished,
        ));
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
