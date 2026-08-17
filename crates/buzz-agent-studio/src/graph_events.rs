//! Build graph nodes/edges from Nostr Agent Studio events (kinds 47200+, 47350+).

use crate::events::{AgentConfigCreated, AgentSkillImported};
use crate::graph::{
    graph_json, AgentEntry, CommandEntry, GraphNodeType, RelationshipType, SkillEntry,
};

/// Parsed graph edge from kind 47350 event content.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GraphEdgeEvent {
    /// Source node kind.
    pub source_type: GraphNodeType,
    /// Source slug.
    pub source_slug: String,
    /// Target node kind.
    pub target_type: GraphNodeType,
    /// Target slug.
    pub target_slug: String,
    /// Edge semantics.
    pub relationship_type: RelationshipType,
    /// Detection evidence.
    pub evidence: String,
}

/// Merge scanned filesystem entries with persisted Nostr events.
pub fn graph_from_events(
    agents: &[AgentEntry],
    commands: &[CommandEntry],
    skills: &[SkillEntry],
    configs: &[AgentConfigCreated],
    imported_skills: &[AgentSkillImported],
    edge_events: &[GraphEdgeEvent],
) -> serde_json::Value {
    let mut skill_entries: Vec<SkillEntry> = skills.to_vec();
    for imported in imported_skills {
        if !skill_entries.iter().any(|s| s.slug == imported.skill_id) {
            skill_entries.push(SkillEntry {
                slug: imported.skill_id.clone(),
            });
        }
    }

    let mut agent_entries: Vec<AgentEntry> = agents.to_vec();
    for cfg in configs {
        if !agent_entries.iter().any(|a| a.slug == cfg.agent_id) {
            agent_entries.push(AgentEntry {
                slug: cfg.agent_id.clone(),
                body: String::new(),
                skills: vec![],
            });
        }
    }

    let mut extra_skills: Vec<String> = Vec::new();
    extra_skills.sort();
    extra_skills.dedup();

    let mut base = graph_json(&agent_entries, commands, &skill_entries, &extra_skills);
    if let Some(edges) = base.get_mut("edges").and_then(|v| v.as_array_mut()) {
        for ev in edge_events {
            edges.push(serde_json::to_value(ev).unwrap_or_default());
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_skills_add_nodes() {
        let imported = vec![AgentSkillImported {
            skill_id: "lint-rust".into(),
            source_repo: None,
            source_commit: None,
        }];
        let graph = graph_from_events(&[], &[], &[], &[], &imported, &[]);
        let nodes = graph["nodes"].as_array().expect("nodes");
        assert!(nodes.iter().any(|n| n["slug"] == "lint-rust"));
    }
}
