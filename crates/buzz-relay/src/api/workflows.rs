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
use sha2::{Digest, Sha256};
use uuid::Uuid;

use buzz_core::kind::{KIND_WORKFLOW_DEF, KIND_WORKFLOW_OWNER_RECEIPT};
use buzz_core::{StoredEvent, TenantContext};

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

/// `GET /workflows/{workflow_id}/owner-target` — minimal owner-management coordinate.
pub async fn workflow_owner_target(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/{workflow_id}/owner-target");
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "workflow not found"))?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path);
    let (owner, event_id) =
        bridge::verify_bridge_auth(&headers, "GET", &url, None, state.config.require_auth_token)?;
    bridge::enforce_http_admission(&state, &tenant, &owner).await?;
    bridge::check_nip98_replay(&state, &tenant, event_id).await?;
    let auth_tag = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok());
    super::relay_members::enforce_relay_membership(
        &state,
        tenant.community(),
        &owner.to_bytes(),
        auth_tag,
    )
    .await?;
    let target = state
        .db
        .get_workflow_owner_target(tenant.community(), workflow_id, owner.as_bytes())
        .await
        .map_err(|error| match error {
            buzz_db::error::DbError::NotFound(_) => {
                api_error(StatusCode::NOT_FOUND, "workflow not found")
            }
            other => internal_error(&format!("resolve workflow owner target: {other}")),
        })?;
    Ok(Json(serde_json::json!({
        "agent_pubkey": hex::encode(target.agent_pubkey),
        "expected_revision": hex::encode(target.expected_revision),
    })))
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

/// List pending owner update commands for the authenticated target agent.
pub async fn pending_owner_commands(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = "/workflows/owner-commands/pending";
    let (tenant, agent) = authorize_delivery_write(&state, &headers, path, &body).await?;
    let commands = state
        .db
        .list_pending_workflow_owner_commands(tenant.community(), agent.as_bytes(), 100)
        .await
        .map_err(|error| internal_error(&format!("list workflow owner commands: {error}")))?;
    Ok(Json(serde_json::json!({
        "commands": commands.iter().map(|command| serde_json::json!({
            "command_id": command.command_id,
            "event_id": hex::encode(&command.event_id),
            "owner_pubkey": hex::encode(&command.owner_pubkey),
            "agent_pubkey": hex::encode(&command.agent_pubkey),
            "workflow_id": command.workflow_id,
            "expected_revision": hex::encode(&command.expected_revision),
            "proposed_yaml": command.proposed_yaml,
            "channel_id": command.channel_id,
        })).collect::<Vec<_>>(),
    })))
}

/// Agent-authenticated terminal intent for one owner command.
#[derive(Debug, Deserialize)]
pub struct CompleteOwnerCommandRequest {
    replacement_event: Option<nostr::Event>,
    rejection_reason: Option<String>,
}

pub(crate) struct OwnerCommandReceiptInput<'a> {
    pub command_id: Uuid,
    pub owner_pubkey: &'a [u8],
    pub agent_pubkey: &'a nostr::PublicKey,
    pub workflow_id: Uuid,
    pub expected_revision: &'a [u8],
    pub status: &'a str,
    pub resulting_revision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

pub(crate) fn build_owner_command_receipt(
    state: &AppState,
    input: OwnerCommandReceiptInput<'_>,
) -> Result<nostr::Event, String> {
    let OwnerCommandReceiptInput {
        command_id,
        owner_pubkey,
        agent_pubkey,
        workflow_id,
        expected_revision,
        status,
        resulting_revision,
        reason,
    } = input;
    nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_WORKFLOW_OWNER_RECEIPT as u16),
        serde_json::json!({
            "status": status,
            "resulting_revision": resulting_revision,
            "reason": reason,
        })
        .to_string(),
    )
    .tags([
        nostr::Tag::parse(["d", &command_id.to_string()]).map_err(|e| e.to_string())?,
        nostr::Tag::parse(["p", &hex::encode(owner_pubkey)]).map_err(|e| e.to_string())?,
        nostr::Tag::parse(["p", &agent_pubkey.to_hex()]).map_err(|e| e.to_string())?,
        nostr::Tag::parse([
            "a",
            &format!(
                "{KIND_WORKFLOW_DEF}:{}:{workflow_id}",
                agent_pubkey.to_hex()
            ),
        ])
        .map_err(|e| e.to_string())?,
        nostr::Tag::parse(["revision", &hex::encode(expected_revision)])
            .map_err(|e| e.to_string())?,
        nostr::Tag::parse(["status", status]).map_err(|e| e.to_string())?,
    ])
    .custom_created_at(nostr::Timestamp::from(0))
    .sign_with_keys(&state.relay_keypair)
    .map_err(|e| e.to_string())
}

fn prepare_managed_definition(
    name: String,
    enabled: bool,
    definition_json: &str,
    existing: &serde_json::Value,
    is_webhook: bool,
) -> Result<(String, String, Vec<u8>, bool), String> {
    let mut definition: serde_json::Value = serde_json::from_str(definition_json)
        .map_err(|error| format!("replacement JSON: {error}"))?;
    crate::webhook_secret::prepare_definition(&mut definition, Some(existing), is_webhook, false)
        .map_err(str::to_string)?;
    let json =
        serde_json::to_string(&definition).map_err(|error| format!("replacement JSON: {error}"))?;
    let hash = Sha256::digest(json.as_bytes()).to_vec();
    Ok((name, json, hash, enabled))
}

/// Complete one pending owner update after agent-side verification/signing.
pub async fn complete_owner_command(
    State(state): State<Arc<AppState>>,
    Path(command_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = format!("/workflows/owner-commands/{command_id}/complete");
    let (tenant, agent) = authorize_delivery_write(&state, &headers, &path, &body).await?;
    let request = match serde_json::from_slice::<CompleteOwnerCommandRequest>(&body) {
        Ok(request)
            if request.replacement_event.is_some() != request.rejection_reason.is_some() =>
        {
            request
        }
        Ok(_) => CompleteOwnerCommandRequest {
            replacement_event: None,
            rejection_reason: Some("invalid_completion_shape".into()),
        },
        Err(_) => CompleteOwnerCommandRequest {
            replacement_event: None,
            rejection_reason: Some("invalid_completion_json".into()),
        },
    };

    let pending = state
        .db
        .get_workflow_owner_command(tenant.community(), command_id, agent.as_bytes())
        .await
        .map_err(|error| match error {
            buzz_db::DbError::NotFound(_) => api_error(
                StatusCode::NOT_FOUND,
                "owner command was not found for this agent",
            ),
            other => internal_error(&format!("read owner command: {other}")),
        })?;
    let workflow = state
        .db
        .get_workflow(tenant.community(), pending.workflow_id)
        .await
        .map_err(|error| internal_error(&format!("read owner command workflow: {error}")))?;
    let channel_id = workflow
        .channel_id
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "workflow is not channel-scoped"))?;

    let mut replacement_event = request.replacement_event;
    let mut rejection_reason = request.rejection_reason;
    let mut definition = None;
    if let Some(event) = replacement_event.as_ref() {
        let bindings_match = event.verify().is_ok()
            && event.kind.as_u16() as u32 == KIND_WORKFLOW_DEF
            && event.pubkey == agent
            && event.content == pending.proposed_yaml
            && event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
                .filter_map(|tag| tag.content())
                .collect::<Vec<_>>()
                == [pending.workflow_id.to_string()]
            && event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
                .filter_map(|tag| tag.content())
                .collect::<Vec<_>>()
                == [channel_id.to_string()]
            && event
                .tags
                .iter()
                .filter(|tag| {
                    tag.as_slice().first().map(String::as_str) == Some("expected-revision")
                })
                .filter_map(|tag| tag.content())
                .collect::<Vec<_>>()
                == [hex::encode(&pending.expected_revision)]
            && event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("command"))
                .filter_map(|tag| tag.content())
                .collect::<Vec<_>>()
                == [command_id.to_string()];
        if !bindings_match {
            replacement_event = None;
            rejection_reason = Some("replacement_binding_mismatch".into());
        } else {
            match buzz_workflow::WorkflowEngine::parse_yaml(&event.content) {
                Ok((def, json)) => {
                    let authority_ok = if def.requires_elevated_authority() {
                        let role = state
                            .db
                            .get_member_role(tenant.community(), channel_id, agent.as_bytes())
                            .await
                            .map_err(|error| {
                                internal_error(&format!("replacement authority lookup: {error}"))
                            })?;
                        matches!(role.as_deref(), Some("owner") | Some("admin"))
                    } else {
                        true
                    };
                    if authority_ok {
                        match prepare_managed_definition(
                            def.name.clone(),
                            def.enabled,
                            &json,
                            &workflow.definition,
                            matches!(def.trigger, buzz_workflow::TriggerDef::Webhook),
                        ) {
                            Ok(prepared) => definition = Some(prepared),
                            Err(reason) => {
                                replacement_event = None;
                                rejection_reason = Some(reason);
                            }
                        }
                    } else {
                        replacement_event = None;
                        rejection_reason = Some("insufficient_workflow_authority".into());
                    }
                }
                Err(_) => {
                    replacement_event = None;
                    rejection_reason = Some("invalid_workflow_yaml".into());
                }
            }
        }
    }

    let status = if replacement_event.is_some() {
        "applied"
    } else {
        "rejected"
    };
    let resulting_revision = replacement_event.as_ref().map(|event| event.id.to_hex());
    let receipt = build_owner_command_receipt(
        &state,
        OwnerCommandReceiptInput {
            command_id,
            owner_pubkey: &pending.owner_pubkey,
            agent_pubkey: &agent,
            workflow_id: pending.workflow_id,
            expected_revision: &pending.expected_revision,
            status,
            resulting_revision: resulting_revision.as_deref(),
            reason: rejection_reason.as_deref(),
        },
    )
    .map_err(|error| internal_error(&format!("sign owner command receipt: {error}")))?;
    let conflict_receipt = |reason| {
        build_owner_command_receipt(
            &state,
            OwnerCommandReceiptInput {
                command_id,
                owner_pubkey: &pending.owner_pubkey,
                agent_pubkey: &agent,
                workflow_id: pending.workflow_id,
                expected_revision: &pending.expected_revision,
                status: "rejected",
                resulting_revision: None,
                reason: Some(reason),
            },
        )
        .map_err(|error| internal_error(&format!("sign owner command conflict receipt: {error}")))
    };
    let coordinate_conflict_receipt = conflict_receipt("workflow_coordinate_changed")?;
    let revision_conflict_receipt = conflict_receipt("workflow_revision_changed")?;
    let result = state
        .db
        .complete_workflow_owner_command(
            tenant.community(),
            command_id,
            agent.as_bytes(),
            replacement_event.as_ref(),
            definition.as_ref().map(|value| value.0.as_str()),
            definition.as_ref().map(|value| value.1.as_str()),
            definition.as_ref().map(|value| value.2.as_slice()),
            definition.as_ref().map(|value| value.3),
            rejection_reason.as_deref(),
            &receipt,
            &coordinate_conflict_receipt,
            &revision_conflict_receipt,
        )
        .await
        .map_err(|error| match error {
            buzz_db::DbError::AccessDenied(_) => api_error(
                StatusCode::FORBIDDEN,
                "owner command targets a different agent",
            ),
            buzz_db::DbError::InvalidData(_) => {
                api_error(StatusCode::CONFLICT, "owner command completion conflict")
            }
            other => internal_error(&format!("complete owner command: {other}")),
        })?;

    if result.transitioned {
        if let Some(event) = replacement_event.as_ref() {
            state
                .workflow_engine
                .invalidate_channel_workflows(tenant.community(), channel_id);
            publish_internal_event(&state, &tenant, event, Some(channel_id)).await;
        }
        let published_receipt = if result.receipt_event_id == receipt.id.as_bytes() {
            &receipt
        } else if result.receipt_event_id == coordinate_conflict_receipt.id.as_bytes() {
            &coordinate_conflict_receipt
        } else {
            &revision_conflict_receipt
        };
        publish_internal_event(&state, &tenant, published_receipt, Some(channel_id)).await;
    }
    Ok(Json(serde_json::json!({
        "status": result.status,
        "resulting_revision": result.resulting_revision.as_deref().map(hex::encode),
        "reason": result.terminal_reason,
        "receipt_event_id": hex::encode(result.receipt_event_id),
        "transitioned": result.transitioned,
    })))
}

pub(crate) async fn publish_internal_event(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: &nostr::Event,
    channel_id: Option<Uuid>,
) {
    let topic = channel_id
        .map(buzz_pubsub::EventTopic::Channel)
        .unwrap_or(buzz_pubsub::EventTopic::Global);
    state.mark_local_event(tenant.community(), &event.id);
    if state
        .pubsub
        .publish_event(tenant, topic, event)
        .await
        .is_err()
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), event.id.to_bytes()));
    }
    let stored = StoredEvent::new(event.clone(), channel_id);
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;
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
    fn managed_webhook_update_preserves_existing_secret() {
        let existing = serde_json::json!({
            "name": "existing",
            "_webhook_secret": "keep-me",
        });
        let (_, json, hash, _) = prepare_managed_definition(
            "replacement".into(),
            true,
            r#"{"name":"replacement"}"#,
            &existing,
            true,
        )
        .unwrap();
        let prepared: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(prepared["_webhook_secret"], "keep-me");
        assert_eq!(hash, Sha256::digest(json.as_bytes()).to_vec());
    }

    #[test]
    fn managed_transition_to_webhook_rejects_undisclosable_secret() {
        let existing = serde_json::json!({"name": "existing"});
        assert_eq!(
            prepare_managed_definition(
                "replacement".into(),
                true,
                r#"{"name":"replacement"}"#,
                &existing,
                true,
            ),
            Err("webhook_secret_unavailable".into())
        );
        assert!(existing.get("_webhook_secret").is_none());
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
