//! Dependency graph extraction (ported from claude-code-cli-ui `relationships.ts`).

use serde::{Deserialize, Serialize};

/// Entity type in the agent/command/skill graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphNodeType {
    /// Claude Code agent definition.
    Agent,
    /// Slash command.
    Command,
    /// Skill pack.
    Skill,
    /// MCP server (read-only node).
    Mcp,
}

/// Edge semantics between graph nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationshipType {
    /// Command body spawns or references an agent.
    Spawns,
    /// Frontmatter declares a dependency.
    AgentFrontmatter,
    /// Inverse spawn reference.
    SpawnedBy,
}

/// A directed edge in the agent studio dependency graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    /// Source node kind.
    pub source_type: GraphNodeType,
    /// Source slug (agent name, command path, skill id).
    pub source_slug: String,
    /// Target node kind.
    pub target_type: GraphNodeType,
    /// Target slug.
    pub target_slug: String,
    /// How the edge was detected.
    pub relationship_type: RelationshipType,
    /// Human-readable evidence (frontmatter key, matched snippet).
    pub evidence: String,
}

/// Agent markdown entry scanned from `.claude/agents/*.md`.
#[derive(Clone, Debug)]
pub struct AgentEntry {
    /// Agent slug (filename without `.md`).
    pub slug: String,
    /// Markdown body after frontmatter.
    pub body: String,
    /// Parsed YAML frontmatter values.
    pub skills: Vec<String>,
}

/// Command markdown entry.
#[derive(Clone, Debug)]
pub struct CommandEntry {
    /// Command slug (may include `--` for nested paths).
    pub slug: String,
    /// Markdown body.
    pub body: String,
    /// Optional `agent:` frontmatter reference.
    pub agent_ref: Option<String>,
}

/// Skill entry.
#[derive(Clone, Debug)]
pub struct SkillEntry {
    /// Skill directory name / slug.
    pub slug: String,
}

/// Build dependency edges from scanned Claude Code config entries.
pub fn extract_relationships(
    agents: &[AgentEntry],
    commands: &[CommandEntry],
    skills: &[SkillEntry],
    extra_skill_slugs: &[String],
) -> Vec<Relationship> {
    let agent_names: std::collections::HashSet<&str> =
        agents.iter().map(|a| a.slug.as_str()).collect();
    let mut skill_slugs: std::collections::HashSet<&str> =
        skills.iter().map(|s| s.slug.as_str()).collect();
    for slug in extra_skill_slugs {
        skill_slugs.insert(slug.as_str());
    }

    let mut relationships = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut add = |rel: Relationship| {
        let key = format!(
            "{:?}:{}->{:?}:{}",
            rel.source_type, rel.source_slug, rel.target_type, rel.target_slug
        );
        if seen.insert(key) {
            relationships.push(rel);
        }
    };

    for agent in agents {
        for skill_slug in &agent.skills {
            if skill_slugs.contains(skill_slug.as_str()) {
                add(Relationship {
                    source_type: GraphNodeType::Agent,
                    source_slug: agent.slug.clone(),
                    target_type: GraphNodeType::Skill,
                    target_slug: skill_slug.clone(),
                    relationship_type: RelationshipType::AgentFrontmatter,
                    evidence: format!("preloads skill: {skill_slug}"),
                });
            }
        }
    }

    for cmd in commands {
        if let Some(agent_ref) = &cmd.agent_ref {
            if agent_names.contains(agent_ref.as_str()) {
                add(Relationship {
                    source_type: GraphNodeType::Command,
                    source_slug: cmd.slug.clone(),
                    target_type: GraphNodeType::Agent,
                    target_slug: agent_ref.clone(),
                    relationship_type: RelationshipType::AgentFrontmatter,
                    evidence: format!("agent: {agent_ref}"),
                });
            }
        }

        if cmd.body.to_ascii_lowercase().contains("subagent_type") {
            if let Some(idx) = cmd.body.to_ascii_lowercase().find("subagent_type") {
                let tail = &cmd.body[idx..];
                if let Some(spawned) = parse_quoted_slug_after_colon(tail) {
                    if agent_names.contains(spawned.as_str()) {
                        add(Relationship {
                            source_type: GraphNodeType::Command,
                            source_slug: cmd.slug.clone(),
                            target_type: GraphNodeType::Agent,
                            target_slug: spawned,
                            relationship_type: RelationshipType::Spawns,
                            evidence: "subagent_type reference".into(),
                        });
                    }
                }
            }
        }

        for spawn in find_spawn_patterns(&cmd.body) {
            if agent_names.contains(spawn.as_str()) {
                add(Relationship {
                    source_type: GraphNodeType::Command,
                    source_slug: cmd.slug.clone(),
                    target_type: GraphNodeType::Agent,
                    target_slug: spawn,
                    relationship_type: RelationshipType::Spawns,
                    evidence: "spawn pattern".into(),
                });
            }
        }
    }

    relationships
}

/// Serialize graph as `{ nodes, edges }` for HTTP clients.
pub fn graph_json(
    agents: &[AgentEntry],
    commands: &[CommandEntry],
    skills: &[SkillEntry],
    extra_skill_slugs: &[String],
) -> serde_json::Value {
    let edges = extract_relationships(agents, commands, skills, extra_skill_slugs);

    let mut node_ids = std::collections::BTreeSet::new();
    for agent in agents {
        node_ids.insert(format!("agent:{}", agent.slug));
    }
    for cmd in commands {
        node_ids.insert(format!("command:{}", cmd.slug));
    }
    for skill in skills {
        node_ids.insert(format!("skill:{}", skill.slug));
    }
    for edge in &edges {
        node_ids.insert(format!(
            "{}:{}",
            node_kind_label(&edge.source_type),
            edge.source_slug
        ));
        node_ids.insert(format!(
            "{}:{}",
            node_kind_label(&edge.target_type),
            edge.target_slug
        ));
    }

    let nodes: Vec<serde_json::Value> = node_ids
        .into_iter()
        .map(|id| {
            let (kind, slug) = id.split_once(':').unwrap_or((&id, ""));
            serde_json::json!({ "id": id, "kind": kind, "slug": slug })
        })
        .collect();

    serde_json::json!({ "nodes": nodes, "edges": edges })
}

fn node_kind_label(kind: &GraphNodeType) -> &'static str {
    match kind {
        GraphNodeType::Agent => "agent",
        GraphNodeType::Command => "command",
        GraphNodeType::Skill => "skill",
        GraphNodeType::Mcp => "mcp",
    }
}

fn parse_quoted_slug_after_colon(text: &str) -> Option<String> {
    let after = text.split(':').nth(1)?.trim();
    let slug = after
        .trim_start_matches(['"', '\''])
        .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        .next()?
        .trim();
    if slug.is_empty() || !slug.starts_with(|c: char| c.is_ascii_lowercase()) {
        return None;
    }
    Some(slug.to_string())
}

fn find_spawn_patterns(body: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lower = body.to_ascii_lowercase();
    for prefix in ["spawn ", "spawns ", "spawned "] {
        let mut start = 0;
        while let Some(idx) = lower[start..].find(prefix) {
            let abs = start + idx + prefix.len();
            let tail = body.get(abs..).unwrap_or("");
            let trimmed = tail
                .trim_start()
                .strip_prefix("the ")
                .or_else(|| tail.trim_start().strip_prefix("The "))
                .unwrap_or(tail.trim_start());
            if let Some(slug) = trimmed
                .trim_start_matches(['"', '\''])
                .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .next()
            {
                if slug.starts_with(|c: char| c.is_ascii_lowercase()) {
                    found.push(slug.to_string());
                }
            }
            start = abs;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_skill_frontmatter_edge() {
        let agents = vec![AgentEntry {
            slug: "reviewer".into(),
            body: String::new(),
            skills: vec!["lint".into()],
        }];
        let skills = vec![SkillEntry {
            slug: "lint".into(),
        }];
        let edges = extract_relationships(&agents, &[], &skills, &[]);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_slug, "lint");
    }

    #[test]
    fn command_agent_frontmatter_edge() {
        let agents = vec![AgentEntry {
            slug: "worker".into(),
            body: String::new(),
            skills: vec![],
        }];
        let commands = vec![CommandEntry {
            slug: "ship".into(),
            body: String::new(),
            agent_ref: Some("worker".into()),
        }];
        let edges = extract_relationships(&agents, &commands, &[], &[]);
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0].relationship_type,
            RelationshipType::AgentFrontmatter
        );
    }
}
