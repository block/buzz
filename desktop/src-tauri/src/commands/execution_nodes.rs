//! Desktop discovery of paired, runtime-neutral execution nodes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration as StdDuration;

use buzz_core_pkg::execution::{
    AgentWorkloadContext, ExecutionCapability, ExecutionCommand, ExecutionCommandEnvelope,
    ExecutionNodeId, ExecutionNodeLifecycle, ExecutionNodeStatus, ExecutionReceipt,
    ProviderAuthResponse, ProviderAuthSession, WorkloadSpec, WorkloadStatus,
};
use buzz_core_pkg::kind::{
    KIND_EXECUTION_NODE_ANNOUNCEMENT, KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT,
    KIND_PRESENCE_UPDATE,
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

/// Availability derived from the node's live kind:20001 presence heartbeat
/// combined with the lifecycle in its most recent signed announcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionNodeAvailability {
    /// The node is ready and its presence heartbeat is live on the relay.
    Connected,
    /// The node announced Ready but has no live presence heartbeat.
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

/// Relay publish acknowledgement in the execution seam's camelCase shape.
///
/// [`SubmitEventResponse`] itself serializes `event_id` (snake_case) and is
/// read that way by other TS callers (e.g. `social.ts`), so it cannot be
/// renamed globally; this local projection matches the camelCase
/// `publication` field the TS `DeployExecutionWorkloadResponse` declares.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPublicationAck {
    /// Relay-assigned event identity.
    pub event_id: String,
    /// Whether the relay accepted the command event.
    pub accepted: bool,
    /// Relay acknowledgement message.
    pub message: String,
}

impl From<SubmitEventResponse> for ExecutionPublicationAck {
    fn from(response: SubmitEventResponse) -> Self {
        Self {
            event_id: response.event_id,
            accepted: response.accepted,
            message: response.message,
        }
    }
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
    pub publication: ExecutionPublicationAck,
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

    let mut nodes: BTreeMap<String, (u64, String, ExecutionNodeTarget)> = BTreeMap::new();
    for event in events {
        let Some(status) = trusted_execution_node_status(&event, &relay_authority, &owner_pubkey)
        else {
            continue;
        };
        let target = ExecutionNodeTarget {
            node_id: status.node_id.into(),
            display_name: status.display_name,
            lifecycle: status.lifecycle,
            capabilities: status.capabilities,
            observed_at: status.observed_at,
            // Placeholder until presence is resolved for all nodes at once below.
            availability: ExecutionNodeAvailability::Unavailable,
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
    let node_ids: Vec<String> = nodes.keys().cloned().collect();
    let online = online_execution_node_pubkeys(&state, &node_ids).await;
    let mut nodes: Vec<_> = nodes
        .into_values()
        .map(|(_, _, mut target)| {
            target.availability = derive_execution_node_availability(
                &target.lifecycle,
                online.contains(&target.node_id),
            );
            target
        })
        .collect();
    nodes.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok(nodes)
}

/// Node pubkeys whose latest kind:20001 presence resolves to a live status.
///
/// Nodes heartbeat presence the same way members and managed agents do, so
/// liveness rides the relay's Redis presence store (short TTL, cleared on
/// clean disconnect) instead of announcement freshness. The relay's HTTP
/// bridge intercepts `kinds:[20001]` + `authors` filters and synthesizes
/// presence events from Redis. A lookup failure degrades to "no presence"
/// rather than failing the node listing.
async fn online_execution_node_pubkeys(
    state: &State<'_, AppState>,
    node_ids: &[String],
) -> BTreeSet<String> {
    if node_ids.is_empty() {
        return BTreeSet::new();
    }
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [KIND_PRESENCE_UPDATE],
            "authors": node_ids,
        })],
    )
    .await
    .unwrap_or_default();
    online_presence_pubkeys(&events)
}

/// Extract the subjects whose most recent presence event is a live status.
///
/// Relay-synthesized presence events (built from Redis on query) are signed
/// by the relay and carry the subject in a `p` tag; self-signed live events
/// use the event author directly — same distinction `get_presence` handles.
fn online_presence_pubkeys(events: &[Event]) -> BTreeSet<String> {
    let mut latest: BTreeMap<String, (u64, bool)> = BTreeMap::new();
    for event in events {
        let subject = event
            .tags
            .iter()
            .find_map(|tag| {
                let slice = tag.as_slice();
                (slice.len() >= 2 && slice[0] == "p").then(|| slice[1].clone())
            })
            .unwrap_or_else(|| event.pubkey.to_hex());
        let online = match event.content.trim() {
            "online" | "away" => true,
            "offline" => false,
            _ => continue,
        };
        let ts = event.created_at.as_secs();
        match latest.get(&subject) {
            Some((prev_ts, _)) if *prev_ts >= ts => {}
            _ => {
                latest.insert(subject, (ts, online));
            }
        }
    }
    latest
        .into_iter()
        .filter_map(|(pubkey, (_, online))| online.then_some(pubkey))
        .collect()
}

/// Classify node availability from announcement lifecycle plus live presence.
fn derive_execution_node_availability(
    lifecycle: &ExecutionNodeLifecycle,
    presence_online: bool,
) -> ExecutionNodeAvailability {
    if *lifecycle != ExecutionNodeLifecycle::Ready {
        ExecutionNodeAvailability::Degraded
    } else if presence_online {
        ExecutionNodeAvailability::Connected
    } else {
        ExecutionNodeAvailability::Unavailable
    }
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
        let teams = crate::managed_agents::load_teams(&app).unwrap_or_default();
        let owner_pubkey = state.signing_keys()?.public_key().to_hex();
        // The same resolver providers consume — then stripped of provider
        // credential variables at the node boundary: nodes inject those from
        // the operator's own environment, and the launch env is persisted in
        // the node's workload ledger.
        let launch = crate::managed_agents::launch::resolve_launch_spec(
            record,
            &effective_harness,
            &teams,
            effective_config.system_prompt.value.as_deref(),
            effective_config.model.value.as_deref(),
            effective_config.provider.value.as_deref(),
            Some(&owner_pubkey),
        )?
        .without_provider_credentials();
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
            launch,
        )
        .map_err(|error| format!("invalid managed-agent workload: {error}"))?;
        workload.agent = Some(
            AgentWorkloadContext::new(
                record.pubkey.clone(),
                Some(relay_url),
                record.auth_tag.clone(),
                input.channel_id.clone(),
            )
            .map_err(|error| format!("invalid managed-agent context: {error}"))?
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
        ExecutionCommand::Deploy {
            workload: Box::new(workload),
            supersedes_removal: None,
        },
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
    let node_status = ensure_trusted_execution_node(state, &node_id).await?;
    let mut command = command;
    // Deploys echo the highest node-assigned receipt sequence observed for
    // the workload. The node's stable workload ids deliberately survive
    // remove-then-redeploy, and its removal tombstones only yield to deploys
    // that prove they were issued after the removal was observed — the echoed
    // sequence is that proof (a stale replayed deploy cannot carry it).
    if let ExecutionCommand::Deploy {
        workload,
        supersedes_removal,
    } = &mut command
    {
        if supersedes_removal.is_none() {
            *supersedes_removal = node_status
                .workloads()
                .iter()
                .find(|status| status.workload_id == workload.workload_id)
                .map(|status| status.sequence);
        }
    }
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
        publication: publication.into(),
        receipt,
    })
}

async fn ensure_trusted_execution_node(
    state: &State<'_, AppState>,
    expected_node_id: &ExecutionNodeId,
) -> Result<ExecutionNodeStatus, String> {
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
    let mut newest: Option<(u64, String, ExecutionNodeStatus)> = None;
    for event in events {
        let Some(status) = trusted_execution_node_status(&event, &relay_authority, &owner_pubkey)
        else {
            continue;
        };
        if status.node_id != *expected_node_id {
            continue;
        }
        let version = (event.created_at.as_secs(), event.id.to_hex());
        if newest
            .as_ref()
            .is_none_or(|(seconds, id, _)| version > (*seconds, id.clone()))
        {
            newest = Some((version.0, version.1, status));
        }
    }
    newest.map(|(_, _, status)| status).ok_or_else(|| {
        "execution node is not paired with this workspace owner and relay".to_string()
    })
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
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn presence_event(keys: &Keys, content: &str, subject: Option<&str>, created_at: u64) -> Event {
        let builder = EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), content)
            .custom_created_at(Timestamp::from(created_at));
        let builder = match subject {
            Some(subject) => builder.tags([Tag::parse(["p", subject]).expect("p tag")]),
            None => builder.tags([]),
        };
        builder.sign_with_keys(keys).expect("presence event")
    }

    #[test]
    fn availability_requires_ready_lifecycle_and_live_presence() {
        assert_eq!(
            derive_execution_node_availability(&ExecutionNodeLifecycle::Ready, true),
            ExecutionNodeAvailability::Connected
        );
        assert_eq!(
            derive_execution_node_availability(&ExecutionNodeLifecycle::Ready, false),
            ExecutionNodeAvailability::Unavailable
        );
        // Non-ready lifecycles are degraded regardless of presence.
        assert_eq!(
            derive_execution_node_availability(&ExecutionNodeLifecycle::Draining, true),
            ExecutionNodeAvailability::Degraded
        );
        assert_eq!(
            derive_execution_node_availability(&ExecutionNodeLifecycle::Connecting, false),
            ExecutionNodeAvailability::Degraded
        );
    }

    #[test]
    fn presence_subject_prefers_p_tag_over_author() {
        let relay_keys = Keys::generate();
        let node_keys = Keys::generate();
        let node_pubkey = node_keys.public_key().to_hex();

        // Relay-synthesized presence: signed by the relay, subject in a p tag.
        let synthesized = presence_event(&relay_keys, "online", Some(&node_pubkey), 100);
        let online = online_presence_pubkeys(&[synthesized]);
        assert!(online.contains(&node_pubkey));
        assert!(!online.contains(&relay_keys.public_key().to_hex()));

        // Self-signed live presence: no p tag, subject is the author.
        let self_signed = presence_event(&node_keys, "online", None, 100);
        let online = online_presence_pubkeys(&[self_signed]);
        assert!(online.contains(&node_pubkey));
    }

    #[test]
    fn presence_uses_latest_status_per_subject() {
        let node_keys = Keys::generate();
        let node_pubkey = node_keys.public_key().to_hex();

        let stale_online = presence_event(&node_keys, "online", None, 100);
        let fresh_offline = presence_event(&node_keys, "offline", None, 200);
        let online = online_presence_pubkeys(&[stale_online.clone(), fresh_offline.clone()]);
        assert!(!online.contains(&node_pubkey));

        // Order independence: the newest event wins either way.
        let online = online_presence_pubkeys(&[fresh_offline, stale_online]);
        assert!(!online.contains(&node_pubkey));

        // Unknown statuses are ignored rather than treated as offline.
        let garbage = presence_event(&node_keys, "not-a-status", None, 300);
        let recovered = presence_event(&node_keys, "online", None, 250);
        let online = online_presence_pubkeys(&[garbage, recovered]);
        assert!(online.contains(&node_pubkey));
    }

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
