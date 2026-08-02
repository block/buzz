//! Desktop discovery of paired, runtime-neutral execution nodes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration as StdDuration;

use buzz_core_pkg::execution::{
    AgentRuntimeSettings, AgentWorkloadContext, ExecutionCapability, ExecutionCommand,
    ExecutionCommandEnvelope, ExecutionNodeId, ExecutionNodeLifecycle, ExecutionNodeStatus,
    ExecutionReceipt, ProviderAuthResponse, ProviderAuthSession, WorkloadSpec, WorkloadStatus,
};
use buzz_core_pkg::kind::{
    KIND_EXECUTION_NODE_ANNOUNCEMENT, KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT,
};
use chrono::{Duration, Utc};
use nostr::{nips::nip44, Event, EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        load_global_agent_config, load_managed_agents, load_personas, save_managed_agents,
        BackendKind, ManagedAgentRecord,
    },
    relay::{query_relay, relay_ws_url_with_override, SubmitEventResponse},
};
use buzz_core_pkg::tenant::relay_url_authority;

const RECEIPT_POLL_INTERVAL: StdDuration = StdDuration::from_millis(250);
const RECEIPT_POLL_TIMEOUT: StdDuration = StdDuration::from_secs(10);

/// Availability derived from the most recent signed node announcement.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeAvailability {
    /// The node announced itself recently.
    Connected,
    /// The node has not announced itself within the reconnect window.
    Unavailable,
    /// The node is announcing but its lifecycle is not ready.
    Degraded,
}

/// Safe execution-node target returned to the Desktop frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionNodeTarget {
    /// Stable Nostr identity of the node.
    pub node_id: String,
    /// User-visible name chosen by the node operator.
    pub display_name: String,
    /// Current node lifecycle.
    pub lifecycle: ExecutionNodeLifecycle,
    /// Explicit operations advertised by the node.
    pub capabilities: BTreeSet<ExecutionCapability>,
    /// Last signed observation time.
    pub observed_at: chrono::DateTime<Utc>,
    /// Client-facing connectivity classification.
    pub availability: ExecutionNodeAvailability,
    /// Durable workloads currently known by the node.
    pub workloads: Vec<WorkloadStatus>,
}

/// Input for deploying an existing managed-agent identity to an execution node.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployManagedAgentToExecutionNodeInput {
    /// Managed-agent public identity whose configuration is the source of truth.
    pub pubkey: String,
    /// Target paired execution node.
    pub node_id: String,
    /// Channel selected by the create flow, when one exists.
    pub channel_id: Option<String>,
}

/// Result of publishing a deploy command and, when online, receiving its receipt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployExecutionWorkloadResponse {
    /// Command correlation identity.
    pub command_id: buzz_core_pkg::execution::CommandId,
    /// Request correlation identity.
    pub request_id: buzz_core_pkg::execution::RequestId,
    /// Workload identity assigned by Desktop.
    pub workload_id: buzz_core_pkg::execution::WorkloadId,
    /// Target node identity.
    pub node_id: String,
    /// Relay publish response.
    pub publication: SubmitEventResponse,
    /// Terminal or latest receipt observed before the polling timeout.
    pub receipt: Option<ExecutionReceipt>,
}

/// Input for a lifecycle command targeting an existing workload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWorkloadCommandInput {
    /// Target node public key.
    pub node_id: String,
    /// Existing workload identity.
    pub workload_id: buzz_core_pkg::execution::WorkloadId,
}

/// Input for starting a provider-authentication session on a node.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartExecutionAuthenticationInput {
    /// Target node public key.
    pub node_id: String,
    /// Existing workload identity.
    pub workload_id: buzz_core_pkg::execution::WorkloadId,
    /// Provider namespace to authenticate.
    pub provider: String,
}

/// Input for submitting or cancelling a provider-authentication session.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionAuthenticationSessionInput {
    /// Target node public key.
    pub node_id: String,
    /// Existing workload identity.
    pub workload_id: buzz_core_pkg::execution::WorkloadId,
    /// Session returned by the start command.
    pub session_id: String,
}

/// Encrypted provider response submitted by Desktop.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitExecutionAuthenticationInput {
    /// Target node public key.
    pub node_id: String,
    /// Existing workload identity.
    pub workload_id: buzz_core_pkg::execution::WorkloadId,
    /// Session returned by the start command.
    pub session_id: String,
    /// Provider response; this is only placed in the encrypted command.
    pub response: String,
}

/// Resolve the durable cleanup identity for an execution-node managed agent.
///
/// The stable fallback is intentional: deployment is a remote side effect
/// followed by a local projection, so a save failure or a concurrent delete
/// can leave the record without `backend_agent_id` even though the node has
/// accepted the workload. Deletion must still address the same workload.
pub(crate) fn managed_agent_execution_target(
    record: &ManagedAgentRecord,
) -> Result<Option<(String, String)>, String> {
    let BackendKind::ExecutionNode { node_id } = &record.backend else {
        return Ok(None);
    };
    let workload_id = match record.backend_agent_id.as_deref() {
        Some(value) => buzz_core_pkg::execution::WorkloadId::new(value.to_string())
            .map_err(|error| format!("invalid stored execution workload id: {error}"))?,
        None => buzz_core_pkg::execution::WorkloadId::stable_for_agent(&record.pubkey)
            .map_err(|error| format!("invalid managed-agent identity: {error}"))?,
    };
    Ok(Some((node_id.clone(), workload_id.as_str().to_string())))
}

#[derive(Debug, Clone, Copy)]
enum WorkloadLifecycleOperation {
    Start,
    Stop,
    Restart,
    Remove,
}

/// Query the relay for the latest signed execution-node announcements.
#[tauri::command]
pub async fn list_execution_nodes(
    state: State<'_, AppState>,
) -> Result<Vec<ExecutionNodeTarget>, String> {
    let owner_keys = state.signing_keys()?;
    let owner_pubkey = owner_keys.public_key().to_hex();
    let relay_authority = relay_url_authority(&relay_ws_url_with_override(&state));
    if relay_authority.is_empty() {
        return Err("the configured relay URL has no valid authority".into());
    }
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_EXECUTION_NODE_ANNOUNCEMENT],
            "limit": 200,
        })],
    )
    .await?;

    let now = Utc::now();
    let mut nodes: BTreeMap<String, (u64, String, ExecutionNodeTarget)> = BTreeMap::new();
    for event in events {
        let Some(status) = trusted_execution_node_status(&event, &relay_authority, &owner_pubkey)
        else {
            continue;
        };
        let availability = if status.lifecycle != ExecutionNodeLifecycle::Ready {
            ExecutionNodeAvailability::Degraded
        } else if status.observed_at <= now && now - status.observed_at <= Duration::minutes(2) {
            ExecutionNodeAvailability::Connected
        } else {
            ExecutionNodeAvailability::Unavailable
        };
        let target = ExecutionNodeTarget {
            node_id: status.node_id.into(),
            display_name: status.display_name,
            lifecycle: status.lifecycle,
            capabilities: status.capabilities,
            observed_at: status.observed_at,
            availability,
            workloads: status.workloads,
        };
        let version = (event.created_at.as_secs(), event.id.to_hex());
        nodes
            .entry(target.node_id.clone())
            .and_modify(|existing| {
                if version > (existing.0, existing.1.clone()) {
                    *existing = (version.0, version.1.clone(), target.clone());
                }
            })
            .or_insert((version.0, version.1, target));
    }
    let mut nodes: Vec<_> = nodes.into_values().map(|(_, _, target)| target).collect();
    nodes.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok(nodes)
}

/// Deploy an already-created managed-agent identity to a paired execution node.
///
/// The workload projection is reconstructed from the durable managed-agent
/// record, so remote execution uses the same identity, prompt, relay/auth
/// configuration, audience policy, and channel context as the Desktop agent.
#[tauri::command]
pub async fn deploy_managed_agent_to_execution_node(
    input: DeployManagedAgentToExecutionNodeInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let _execution_guard = state.managed_agent_execution_transition.lock().await;
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    let workload = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == input.pubkey)
            .ok_or_else(|| format!("managed agent {} not found", input.pubkey))?;
        let personas = load_personas(&app)?;
        let global_config = load_global_agent_config(&app)?;
        let effective_config = crate::managed_agents::effective_config::resolve_effective_config(
            record,
            &personas,
            &global_config,
        )
        .require_resolved()?;
        let effective_harness = crate::managed_agents::resolve_effective_harness_descriptor(
            record,
            &personas,
            &global_config,
        )?;
        let runtime_settings = AgentRuntimeSettings::new(
            effective_harness.args,
            record.idle_timeout_seconds,
            record.max_turn_duration_seconds,
            record.parallelism,
        )
        .map_err(|error| format!("invalid managed-agent runtime settings: {error}"))?;
        let runtime = record
            .runtime
            .clone()
            .ok_or_else(|| "managed agent has no persisted runtime".to_string())?;
        let (bound_node_id, stored_workload_id) = managed_agent_execution_target(record)?
            .ok_or_else(|| "managed agent is not configured for an execution node".to_string())?;
        if bound_node_id != node_id.as_str() {
            return Err("managed agent is bound to a different execution node".into());
        }
        let relay_url = crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(&state),
        );
        let workload_id = buzz_core_pkg::execution::WorkloadId::new(stored_workload_id)
            .map_err(|error| format!("invalid managed-agent workload id: {error}"))?;
        let mut workload = WorkloadSpec::agent(
            workload_id,
            record.name.clone(),
            runtime,
            effective_config.model.value,
            effective_config.provider.value,
            Vec::new(),
        )
        .map_err(|error| format!("invalid managed-agent workload: {error}"))?;
        workload.agent = Some(
            AgentWorkloadContext::new(
                record.pubkey.clone(),
                effective_config.system_prompt.value,
                Some(relay_url),
                record.auth_tag.clone(),
                Some(record.respond_to.as_str().to_string()),
                record.respond_to_allowlist.clone(),
                input.channel_id.clone(),
            )
            .map_err(|error| format!("invalid managed-agent context: {error}"))?
            .with_runtime_settings(runtime_settings)
            .map_err(|error| format!("invalid managed-agent runtime settings: {error}"))?
            .with_private_key(record.private_key_nsec.clone())
            .map_err(|error| format!("managed-agent key is unavailable: {error}"))?,
        );
        workload
            .validate()
            .map_err(|error| format!("invalid managed-agent workload: {error}"))?;
        workload
    };
    let response = send_execution_command(
        &state,
        node_id.clone(),
        ExecutionCommand::Deploy { workload },
    )
    .await?;
    match response.receipt.as_ref() {
        Some(receipt)
            if matches!(
                receipt.outcome,
                buzz_core_pkg::execution::ReceiptOutcome::Succeeded
            ) => {}
        Some(receipt) => {
            return Err(format!(
                "execution node did not deploy workload: {:?}",
                receipt.outcome
            ))
        }
        None => return Err("execution node did not confirm workload deployment".into()),
    }

    let persist_result = (|| -> Result<(), String> {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let record = records
            .iter_mut()
            .find(|record| record.pubkey == input.pubkey)
            .ok_or_else(|| format!("managed agent {} disappeared", input.pubkey))?;
        record.backend_agent_id = Some(response.workload_id.as_str().to_string());
        save_managed_agents(&app, &records)
    })();
    if let Err(persist_error) = persist_result {
        let cleanup_result = remove_execution_workload_for_managed_agent(
            &state,
            node_id.as_str(),
            response.workload_id.as_str(),
        )
        .await;
        return match cleanup_result {
            Ok(()) => Err(persist_error),
            Err(cleanup_error) => Err(format!(
                "{persist_error}; remote workload cleanup also failed: {cleanup_error}"
            )),
        };
    }
    Ok(response)
}

/// Start a durable workload through the encrypted execution command path.
#[tauri::command]
pub async fn start_execution_workload(
    input: ExecutionWorkloadCommandInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    send_lifecycle_command(&state, input, WorkloadLifecycleOperation::Start).await
}

/// Stop a durable workload through the encrypted execution command path.
#[tauri::command]
pub async fn stop_execution_workload(
    input: ExecutionWorkloadCommandInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    send_lifecycle_command(&state, input, WorkloadLifecycleOperation::Stop).await
}

/// Restart a durable workload through the encrypted execution command path.
#[tauri::command]
pub async fn restart_execution_workload(
    input: ExecutionWorkloadCommandInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    send_lifecycle_command(&state, input, WorkloadLifecycleOperation::Restart).await
}

/// Remove a durable workload through the encrypted execution command path.
#[tauri::command]
pub async fn remove_execution_workload(
    input: ExecutionWorkloadCommandInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    send_lifecycle_command(&state, input, WorkloadLifecycleOperation::Remove).await
}

/// Remove a managed-agent workload and require a confirmed node-side result.
pub(crate) async fn remove_execution_workload_for_managed_agent(
    state: &State<'_, AppState>,
    node_id: &str,
    workload_id: &str,
) -> Result<(), String> {
    let response = send_lifecycle_command_unlocked(
        state,
        ExecutionWorkloadCommandInput {
            node_id: node_id.to_string(),
            workload_id: buzz_core_pkg::execution::WorkloadId::new(workload_id.to_string())
                .map_err(|error| format!("invalid managed-agent workload id: {error}"))?,
        },
        WorkloadLifecycleOperation::Remove,
    )
    .await?;
    match response.receipt {
        Some(receipt)
            if matches!(
                receipt.outcome,
                buzz_core_pkg::execution::ReceiptOutcome::Succeeded
                    | buzz_core_pkg::execution::ReceiptOutcome::Failed {
                        error: buzz_core_pkg::execution::SafeErrorCode::WorkloadNotFound,
                    }
            ) =>
        {
            Ok(())
        }
        Some(receipt) => Err(format!(
            "execution node did not remove workload: {:?}",
            receipt.outcome
        )),
        None => Err("execution node did not confirm workload removal".into()),
    }
}

pub(crate) async fn start_execution_workload_for_managed_agent(
    state: &State<'_, AppState>,
    node_id: &str,
    workload_id: &str,
) -> Result<(), String> {
    let response = send_lifecycle_command(
        state,
        ExecutionWorkloadCommandInput {
            node_id: node_id.to_string(),
            workload_id: buzz_core_pkg::execution::WorkloadId::new(workload_id.to_string())
                .map_err(|error| format!("invalid managed-agent workload id: {error}"))?,
        },
        WorkloadLifecycleOperation::Start,
    )
    .await?;
    ensure_lifecycle_command_accepted(response)
}

fn ensure_lifecycle_command_accepted(
    response: DeployExecutionWorkloadResponse,
) -> Result<(), String> {
    match response.receipt {
        Some(receipt)
            if matches!(
                receipt.outcome,
                buzz_core_pkg::execution::ReceiptOutcome::Accepted
                    | buzz_core_pkg::execution::ReceiptOutcome::Progress
                    | buzz_core_pkg::execution::ReceiptOutcome::Succeeded
            ) =>
        {
            Ok(())
        }
        Some(receipt) => Err(format!(
            "execution node rejected workload lifecycle command: {:?}",
            receipt.outcome
        )),
        None => Err("execution node did not confirm workload lifecycle command".into()),
    }
}

/// Start a provider-authentication session on a paired execution node.
#[tauri::command]
pub async fn start_execution_authentication(
    input: StartExecutionAuthenticationInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    let session = ProviderAuthSession::new(
        input.workload_id,
        input.provider,
        Uuid::new_v4().to_string(),
        Utc::now() + Duration::minutes(10),
    )
    .map_err(|error| format!("could not build authentication session: {error}"))?;
    send_execution_command(
        &state,
        node_id,
        ExecutionCommand::AuthenticateProvider { session },
    )
    .await
}

/// Submit a provider-authentication response through the encrypted command path.
#[tauri::command]
pub async fn submit_execution_authentication(
    input: SubmitExecutionAuthenticationInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    let response = ProviderAuthResponse::new(input.workload_id, input.session_id, input.response)
        .map_err(|error| format!("invalid authentication response: {error}"))?;
    send_execution_command(
        &state,
        node_id,
        ExecutionCommand::SubmitProviderAuthentication { response },
    )
    .await
}

/// Cancel a provider-authentication session on a paired execution node.
#[tauri::command]
pub async fn cancel_execution_authentication(
    input: ExecutionAuthenticationSessionInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    send_execution_command(
        &state,
        node_id,
        ExecutionCommand::CancelProviderAuthentication {
            workload_id: input.workload_id,
            session_id: input.session_id,
        },
    )
    .await
}

async fn send_lifecycle_command(
    state: &State<'_, AppState>,
    input: ExecutionWorkloadCommandInput,
    operation: WorkloadLifecycleOperation,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let _execution_guard = state.managed_agent_execution_transition.lock().await;
    send_lifecycle_command_unlocked(state, input, operation).await
}

async fn send_lifecycle_command_unlocked(
    state: &State<'_, AppState>,
    input: ExecutionWorkloadCommandInput,
    operation: WorkloadLifecycleOperation,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    let command = match operation {
        WorkloadLifecycleOperation::Start => ExecutionCommand::Start {
            workload_id: input.workload_id,
        },
        WorkloadLifecycleOperation::Stop => ExecutionCommand::Stop {
            workload_id: input.workload_id,
        },
        WorkloadLifecycleOperation::Restart => ExecutionCommand::Restart {
            workload_id: input.workload_id,
        },
        WorkloadLifecycleOperation::Remove => ExecutionCommand::Remove {
            workload_id: input.workload_id,
        },
    };
    send_execution_command(state, node_id, command).await
}

async fn send_execution_command(
    state: &State<'_, AppState>,
    node_id: ExecutionNodeId,
    command: ExecutionCommand,
) -> Result<DeployExecutionWorkloadResponse, String> {
    ensure_trusted_execution_node(state, &node_id).await?;
    let issued_at = Utc::now();
    let envelope = ExecutionCommandEnvelope::new(
        node_id.clone(),
        issued_at,
        issued_at + Duration::minutes(5),
        command,
    )
    .map_err(|error| format!("could not build execution command: {error}"))?;
    let keys = state.signing_keys()?;
    let node_pubkey = PublicKey::from_hex(node_id.as_str())
        .map_err(|error| format!("invalid execution node public key: {error}"))?;
    let plaintext = serde_json::to_string(&envelope)
        .map_err(|error| format!("could not serialize execution command: {error}"))?;
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &node_pubkey,
        &plaintext,
        nip44::Version::V2,
    )
    .map_err(|error| format!("could not encrypt execution command: {error}"))?;
    let builder = EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_COMMAND as u16), ciphertext)
        .tags([Tag::parse(["p", node_id.as_str()])
            .map_err(|error| format!("could not build execution target tag: {error}"))?]);
    let mut connection = buzz_ws_client_pkg::NostrWsConnection::connect_authenticated(
        &relay_ws_url_with_override(state),
        &keys,
        None,
    )
    .await
    .map_err(|error| format!("could not connect to relay for execution: {error}"))?;
    connection
        .send_raw(&serde_json::json!([
            "REQ",
            "execution-receipts",
            {
                "kinds": [KIND_EXECUTION_NODE_RECEIPT],
                "#p": [keys.public_key().to_hex()],
                "since": issued_at.timestamp(),
                "limit": 100,
            }
        ]))
        .await
        .map_err(|error| format!("could not subscribe to execution receipts: {error}"))?;
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|error| format!("could not sign execution command: {error}"))?;
    let publication_ack = connection
        .send_event(event)
        .await
        .map_err(|error| format!("could not publish execution command: {error}"))?;
    let publication = SubmitEventResponse {
        event_id: publication_ack.event_id,
        accepted: publication_ack.accepted,
        message: publication_ack.message,
    };
    if !publication.accepted {
        return Err(format!(
            "relay rejected execution command: {}",
            publication.message
        ));
    }
    let receipt =
        wait_for_execution_receipt(&mut connection, &keys, &node_id, envelope.command_id()).await?;

    Ok(DeployExecutionWorkloadResponse {
        command_id: envelope.command_id(),
        request_id: envelope.request_id(),
        workload_id: envelope.command.workload_id().clone(),
        node_id: node_id.into(),
        publication,
        receipt,
    })
}

async fn ensure_trusted_execution_node(
    state: &State<'_, AppState>,
    expected_node_id: &ExecutionNodeId,
) -> Result<(), String> {
    let owner_keys = state.signing_keys()?;
    let owner_pubkey = owner_keys.public_key().to_hex();
    let relay_authority = relay_url_authority(&relay_ws_url_with_override(state));
    if relay_authority.is_empty() {
        return Err("the configured relay URL has no valid authority".into());
    }
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [KIND_EXECUTION_NODE_ANNOUNCEMENT],
            "limit": 200,
        })],
    )
    .await?;
    let trusted = events.into_iter().any(|event| {
        trusted_execution_node_status(&event, &relay_authority, &owner_pubkey)
            .is_some_and(|status| status.node_id == *expected_node_id)
    });
    if trusted {
        Ok(())
    } else {
        Err("execution node is not paired with this workspace owner and relay".into())
    }
}

fn trusted_execution_node_status(
    event: &Event,
    relay_authority: &str,
    owner_pubkey: &str,
) -> Option<ExecutionNodeStatus> {
    if event.kind.as_u16() as u32 != KIND_EXECUTION_NODE_ANNOUNCEMENT
        || !event.verify_id()
        || !event.verify_signature()
    {
        return None;
    }
    let status = serde_json::from_str::<ExecutionNodeStatus>(&event.content).ok()?;
    status.validate().ok()?;
    let author_node_id = ExecutionNodeId::new(event.pubkey.to_hex()).ok()?;
    if status.node_id != author_node_id
        || !announcement_d_tag_matches(event, status.node_id.as_str())
    {
        return None;
    }
    status
        .owner_attestations
        .iter()
        .any(|attestation| {
            attestation
                .verify(&status.node_id, relay_authority, Some(owner_pubkey))
                .is_ok()
        })
        .then_some(status)
}

async fn wait_for_execution_receipt(
    connection: &mut buzz_ws_client_pkg::NostrWsConnection,
    keys: &nostr::Keys,
    node_id: &ExecutionNodeId,
    command_id: buzz_core_pkg::execution::CommandId,
) -> Result<Option<ExecutionReceipt>, String> {
    let deadline = tokio::time::Instant::now() + RECEIPT_POLL_TIMEOUT;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        let next = connection
            .next_event(RECEIPT_POLL_INTERVAL.min(deadline - now))
            .await;
        let buzz_ws_client_pkg::RelayMessage::Event { event, .. } = (match next {
            Ok(event) => event,
            Err(buzz_ws_client_pkg::WsClientError::Timeout) => continue,
            Err(error) => return Err(format!("execution receipt stream failed: {error}")),
        }) else {
            continue;
        };
        if event.kind.as_u16() as u32 != KIND_EXECUTION_NODE_RECEIPT
            || !event.verify_id()
            || !event.verify_signature()
            || !has_exact_p_tag(&event, &keys.public_key().to_hex())
        {
            continue;
        }
        if let Ok(receipt) = decrypt_execution_receipt(keys, &event) {
            if receipt.command_id == command_id
                && &receipt.node_id == node_id
                && (receipt.is_terminal() || receipt.detail.is_some())
            {
                return Ok(Some(receipt));
            }
        }
    }
}

fn decrypt_execution_receipt(
    keys: &nostr::Keys,
    event: &nostr::Event,
) -> Result<ExecutionReceipt, String> {
    let plaintext = nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content)
        .map_err(|error| format!("could not decrypt execution receipt: {error}"))?;
    let receipt: ExecutionReceipt = serde_json::from_str(&plaintext)
        .map_err(|error| format!("could not decode execution receipt: {error}"))?;
    let event_node_id = ExecutionNodeId::new(event.pubkey.to_hex())
        .map_err(|error| format!("invalid receipt event identity: {error}"))?;
    if receipt.node_id != event_node_id {
        return Err("receipt node identity does not match its event signer".into());
    }
    receipt
        .validate()
        .map_err(|error| format!("invalid execution receipt: {error}"))?;
    Ok(receipt)
}

fn has_exact_p_tag(event: &nostr::Event, expected: &str) -> bool {
    let tags: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "p")
        .filter_map(|tag| tag.content())
        .collect();
    tags.len() == 1 && tags[0] == expected
}

fn announcement_d_tag_matches(event: &nostr::Event, node_id: &str) -> bool {
    let mut matches = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "d")
        .filter_map(|tag| tag.content())
        .filter(|value| *value == node_id);
    matches.next().is_some() && matches.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::execution::{ExecutionNodeId, ExecutionNodeStatus};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn announcement_d_tag_must_match_once() {
        let keys = Keys::generate();
        let node_id = ExecutionNodeId::new(keys.public_key().to_hex()).expect("node id");
        let event = EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT as u16), "{}")
            .tags([Tag::parse(["d", node_id.as_str()]).expect("tag")])
            .sign_with_keys(&keys)
            .expect("event");
        assert!(announcement_d_tag_matches(&event, node_id.as_str()));
        let duplicate =
            EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT as u16), "{}")
                .tags([
                    Tag::parse(["d", node_id.as_str()]).expect("tag"),
                    Tag::parse(["d", node_id.as_str()]).expect("tag"),
                ])
                .sign_with_keys(&keys)
                .expect("event");
        assert!(!announcement_d_tag_matches(&duplicate, node_id.as_str()));
    }

    #[test]
    fn target_projection_uses_shared_status_types() {
        let _status = ExecutionNodeStatus::new(
            ExecutionNodeId::new("a".repeat(64)).expect("node id"),
            "Example execution node",
            ExecutionNodeLifecycle::Ready,
            [ExecutionCapability::Deploy],
        )
        .expect("status");
    }
}
