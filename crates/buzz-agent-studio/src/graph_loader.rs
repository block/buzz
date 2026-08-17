//! Parse stored Nostr events into Agent Studio graph JSON.

use buzz_core::kind::{
    KIND_AGENT_CONFIG_CREATED, KIND_AGENT_CONFIG_UPDATED, KIND_AGENT_GRAPH_EDGE,
    KIND_AGENT_SKILL_IMPORTED,
};

use crate::events::{AgentConfigCreated, AgentGraphEdge, AgentSkillImported};
use crate::graph_events::{graph_from_events, GraphEdgeEvent};

/// Minimal stored event fields needed for graph projection.
#[derive(Clone, Debug)]
pub struct StoredAgentStudioEvent {
    /// Nostr event kind.
    pub kind: u32,
    /// Event content JSON.
    pub content: String,
}

/// Build `{ nodes, edges }` from relay-stored Agent Studio events.
pub fn graph_from_stored_events(events: &[StoredAgentStudioEvent]) -> serde_json::Value {
    let mut configs = Vec::new();
    let mut imported_skills = Vec::new();
    let mut edge_events = Vec::new();

    for event in events {
        match event.kind {
            KIND_AGENT_CONFIG_CREATED | KIND_AGENT_CONFIG_UPDATED => {
                if let Ok(payload) = serde_json::from_str::<AgentConfigCreated>(&event.content) {
                    configs.push(payload);
                }
            }
            KIND_AGENT_SKILL_IMPORTED => {
                if let Ok(payload) = serde_json::from_str::<AgentSkillImported>(&event.content) {
                    imported_skills.push(payload);
                }
            }
            KIND_AGENT_GRAPH_EDGE => {
                if let Ok(raw) = serde_json::from_str::<AgentGraphEdge>(&event.content) {
                    edge_events.push(parse_graph_edge(&raw));
                }
            }
            _ => {}
        }
    }

    graph_from_events(&[], &[], &[], &configs, &imported_skills, &edge_events)
}

fn parse_graph_edge(raw: &AgentGraphEdge) -> GraphEdgeEvent {
    use crate::graph::{GraphNodeType, RelationshipType};

    let parse_kind = |label: &str| -> GraphNodeType {
        match label {
            "command" => GraphNodeType::Command,
            "skill" => GraphNodeType::Skill,
            "mcp" => GraphNodeType::Mcp,
            _ => GraphNodeType::Agent,
        }
    };
    let parse_rel = |label: &str| -> RelationshipType {
        match label {
            "spawns" => RelationshipType::Spawns,
            "spawned-by" => RelationshipType::SpawnedBy,
            _ => RelationshipType::AgentFrontmatter,
        }
    };

    GraphEdgeEvent {
        source_type: parse_kind(&raw.source_type),
        source_slug: raw.source_slug.clone(),
        target_type: parse_kind(&raw.target_type),
        target_slug: raw.target_slug.clone(),
        relationship_type: parse_rel(&raw.relationship_type),
        evidence: raw.evidence.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_event_adds_agent_node() {
        let content = r#"{"agent_id":"reviewer","config_json":"{}"}"#;
        let graph = graph_from_stored_events(&[StoredAgentStudioEvent {
            kind: KIND_AGENT_CONFIG_CREATED,
            content: content.into(),
        }]);
        let nodes = graph["nodes"].as_array().expect("nodes");
        assert!(nodes.iter().any(|n| n["slug"] == "reviewer"));
    }
}
