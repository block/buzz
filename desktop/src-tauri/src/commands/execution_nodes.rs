//! Desktop discovery of paired, runtime-neutral execution nodes.

use std::collections::{BTreeMap, BTreeSet};

use buzz_core_pkg::execution::{
    ExecutionCapability, ExecutionNodeLifecycle, ExecutionNodeStatus,
};
use buzz_core_pkg::kind::KIND_EXECUTION_NODE_ANNOUNCEMENT;
use chrono::{Duration, Utc};
use serde::Serialize;
use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

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
        let Ok(author_node_id) = buzz_core_pkg::execution::ExecutionNodeId::new(event.pubkey.to_hex())
        else {
            continue;
        };
        if status.node_id != author_node_id || !announcement_d_tag_matches(&event, status.node_id.as_str()) {
            continue;
        }
        let availability = if status.lifecycle != ExecutionNodeLifecycle::Ready {
            ExecutionNodeAvailability::Degraded
        } else if status.observed_at <= now
            && now - status.observed_at <= Duration::minutes(2)
        {
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
    use nostr::{EventBuilder, Kind, Keys, Tag};

    #[test]
    fn announcement_d_tag_must_match_once() {
        let keys = Keys::generate();
        let node_id = ExecutionNodeId::new(keys.public_key().to_hex()).expect("node id");
        let event = EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT as u16), "{}")
            .tags([Tag::parse(["d", node_id.as_str()]).expect("tag")])
            .sign_with_keys(&keys)
            .expect("event");
        assert!(announcement_d_tag_matches(&event, node_id.as_str()));
        let duplicate = EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT as u16), "{}")
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
