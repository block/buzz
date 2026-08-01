//! Desktop discovery of paired, runtime-neutral execution nodes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration as StdDuration;

use buzz_core_pkg::execution::{
    CredentialRef, ExecutionCapability, ExecutionCommand, ExecutionCommandEnvelope,
    ExecutionNodeId, ExecutionNodeLifecycle, ExecutionNodeStatus, ExecutionReceipt, WorkloadSpec,
};
use buzz_core_pkg::kind::{
    KIND_EXECUTION_NODE_ANNOUNCEMENT, KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT,
};
use chrono::{Duration, Utc};
use nostr::{nips::nip44, EventBuilder, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    app_state::AppState,
    relay::{query_relay, relay_ws_url_with_override, SubmitEventResponse},
};

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
}

/// Safe workload input for the first remote execution deployment flow.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployExecutionWorkloadInput {
    /// Target node public key.
    pub node_id: String,
    /// User-visible workload name.
    pub display_name: String,
    /// Runtime-neutral runtime identifier.
    pub runtime: String,
    /// Optional model identifier.
    pub model: Option<String>,
    /// Optional provider identifier.
    pub provider: Option<String>,
    /// Node-local credential references only.
    #[serde(default)]
    pub credential_refs: Vec<CredentialRef>,
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

/// Query the relay for the latest signed execution-node announcements.
#[tauri::command]
pub async fn list_execution_nodes(
    state: State<'_, AppState>,
) -> Result<Vec<ExecutionNodeTarget>, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_EXECUTION_NODE_ANNOUNCEMENT],
            "limit": 200,
        })],
    )
    .await?;

    let now = Utc::now();
    let mut nodes = BTreeMap::new();
    for event in events {
        if event.kind.as_u16() as u32 != KIND_EXECUTION_NODE_ANNOUNCEMENT
            || !event.verify_id()
            || !event.verify_signature()
        {
            continue;
        }
        let Ok(status) = serde_json::from_str::<ExecutionNodeStatus>(&event.content) else {
            continue;
        };
        let Ok(author_node_id) =
            buzz_core_pkg::execution::ExecutionNodeId::new(event.pubkey.to_hex())
        else {
            continue;
        };
        if status.node_id != author_node_id
            || !announcement_d_tag_matches(&event, status.node_id.as_str())
        {
            continue;
        }
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
        };
        nodes
            .entry(target.node_id.clone())
            .and_modify(|existing: &mut ExecutionNodeTarget| {
                if target.observed_at > existing.observed_at {
                    *existing = target.clone();
                }
            })
            .or_insert(target);
    }
    let mut nodes: Vec<_> = nodes.into_values().collect();
    nodes.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok(nodes)
}

/// Encrypt and publish one owner-authorized deploy command to a paired node.
///
/// The command contains only the safe shared workload projection. The node's
/// credentials and runtime implementation remain outside the event payload.
#[tauri::command]
pub async fn deploy_execution_workload(
    input: DeployExecutionWorkloadInput,
    state: State<'_, AppState>,
) -> Result<DeployExecutionWorkloadResponse, String> {
    let node_id = ExecutionNodeId::new(input.node_id.trim().to_string())
        .map_err(|error| format!("invalid execution node id: {error}"))?;
    let workload = WorkloadSpec::agent(
        buzz_core_pkg::execution::WorkloadId::random(),
        input.display_name,
        input.runtime,
        input.model,
        input.provider,
        input.credential_refs,
    )
    .map_err(|error| format!("invalid execution workload: {error}"))?;
    let issued_at = Utc::now();
    let envelope = ExecutionCommandEnvelope::new(
        node_id.clone(),
        issued_at,
        issued_at + Duration::minutes(5),
        ExecutionCommand::Deploy { workload },
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
        &relay_ws_url_with_override(&state),
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
                && receipt.is_terminal()
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
            "Onkie server",
            ExecutionNodeLifecycle::Ready,
            [ExecutionCapability::Deploy],
        )
        .expect("status");
    }
}
