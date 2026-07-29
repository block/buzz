use std::collections::{BTreeMap, BTreeSet};

use nostr::Event;
use serde::Serialize;
use serde_json::Value;

use super::profile::{ProfileEnvironment, ResolvedProfile};
use super::runtime::{query_events, ContextRuntime};
use crate::error::CliError;
use crate::ContextGraphFormat;

const GRAPH_KINDS: [u16; 37] = [
    1, 7, 9, 1_984, 9_000, 9_001, 9_002, 9_003, 9_004, 9_005, 9_006, 9_007, 9_008, 9_009, 9_010,
    9_011, 9_012, 9_013, 9_014, 9_015, 9_016, 9_017, 9_018, 9_019, 9_020, 20_700, 30_023, 30_174,
    30_700, 39_000, 39_001, 39_002, 39_003, 40_100, 45_001, 45_003, 45_010,
];

#[derive(Debug, Serialize)]
struct GraphProjection {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize)]
struct GraphNode {
    id: String,
    kind: u16,
    author: String,
    created_at: u64,
    label: String,
}

#[derive(Debug, Serialize, Ord, PartialOrd, Eq, PartialEq)]
struct GraphEdge {
    from: String,
    to: String,
    relation: String,
}

pub async fn pulse(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    relay: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let filter = [serde_json::json!({
        "kinds": [20_700],
        "limit": 20,
    })];
    let mut observations = if let Some(relay) = relay {
        let role = if profile.file.identities.replication_transport.is_some() {
            "replication_transport"
        } else {
            "journal_author"
        };
        query_events(&profile.client_for(role, relay, environment)?, &filter).await?
    } else {
        runtime.query_union(&filter).await?
    };
    observations.sort_by_key(|event| std::cmp::Reverse((event.created_at, event.id)));
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&observations)
                .map_err(|error| CliError::Other(format!("pulse JSON failed: {error}")))?
        );
        return Ok(());
    }
    if observations.is_empty() {
        return Err(CliError::NotFound(
            "no signed kind-20700 pulse was observed".into(),
        ));
    }
    for event in observations {
        render_pulse(&event);
    }
    Ok(())
}

pub async fn graph(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    format: ContextGraphFormat,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let events = runtime
        .query_union(&[serde_json::json!({
            "kinds": GRAPH_KINDS.as_slice(),
            "limit": 5000,
        })])
        .await?;
    let projection = project(&events);
    match format {
        ContextGraphFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&projection)
                .map_err(|error| CliError::Other(format!("graph JSON failed: {error}")))?
        ),
        ContextGraphFormat::Tree => render_tree(&projection),
        ContextGraphFormat::Dot => render_dot(&projection),
        ContextGraphFormat::Mermaid => render_mermaid(&projection),
    }
    Ok(())
}

fn project(events: &[Event]) -> GraphProjection {
    let mut nodes = BTreeMap::new();
    let known = events
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<BTreeSet<_>>();
    let mut edges = BTreeSet::new();
    for event in events {
        let id = event.id.to_hex();
        nodes.insert(
            id.clone(),
            GraphNode {
                id: id.clone(),
                kind: event.kind.as_u16(),
                author: event.pubkey.to_hex(),
                created_at: event.created_at.as_secs(),
                label: event_label(event),
            },
        );
        for relation in ["e", "x"] {
            for target in tag_values(event, relation) {
                if relation == "e" && !known.contains(&target) {
                    continue;
                }
                edges.insert(GraphEdge {
                    from: id.clone(),
                    to: target,
                    relation: relation.into(),
                });
            }
        }
    }
    GraphProjection {
        nodes: nodes.into_values().collect(),
        edges: edges.into_iter().collect(),
    }
}

fn render_tree(graph: &GraphProjection) {
    println!(
        "context graph: {} events, {} links",
        graph.nodes.len(),
        graph.edges.len()
    );
    for node in &graph.nodes {
        println!(
            "├─ {}  kind {}  {}  {}",
            short(&node.id),
            node.kind,
            short(&node.author),
            node.label
        );
        for edge in graph.edges.iter().filter(|edge| edge.from == node.id) {
            println!("│  └─ {} → {}", edge.relation, short(&edge.to));
        }
    }
}

fn render_dot(graph: &GraphProjection) {
    println!("digraph buzz_context {{");
    for node in &graph.nodes {
        println!(
            "  \"{}\" [label=\"{}\"];",
            node.id,
            escape(&format!("{}\\nkind {}", node.label, node.kind))
        );
    }
    for edge in &graph.edges {
        println!(
            "  \"{}\" -> \"{}\" [label=\"{}\"];",
            edge.from,
            edge.to,
            escape(&edge.relation)
        );
    }
    println!("}}");
}

fn render_mermaid(graph: &GraphProjection) {
    println!("flowchart TD");
    for (index, node) in graph.nodes.iter().enumerate() {
        println!(
            "  n{index}[\"{}\"]",
            escape(&format!("{} · kind {}", node.label, node.kind))
        );
    }
    let indices = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    for edge in &graph.edges {
        if let (Some(from), Some(to)) = (
            indices.get(edge.from.as_str()),
            indices.get(edge.to.as_str()),
        ) {
            println!("  n{from} -->|{}| n{to}", escape(&edge.relation));
        }
    }
}

fn render_pulse(event: &Event) {
    let content: Value = serde_json::from_str(&event.content).unwrap_or(Value::Null);
    let label = content
        .get("label")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
        .unwrap_or("(unlabelled)");
    let role = tag_values(event, "role")
        .into_iter()
        .next()
        .unwrap_or_else(|| "?".into());
    println!(
        "pulse  {}  node {label} [{role}]",
        short(&event.pubkey.to_hex())
    );
    println!("  witnessed   {}", event.created_at.as_secs());
    if let Some(journal) = content.get("journal") {
        println!(
            "  journal     seq {}  head {}",
            journal
                .get("sequence")
                .and_then(Value::as_u64)
                .map_or_else(|| "?".into(), |value| value.to_string()),
            journal
                .get("head")
                .and_then(Value::as_str)
                .map(short)
                .unwrap_or_else(|| "(empty)".into())
        );
    }
    if let Some(checkpoints) = content.get("checkpoints").and_then(Value::as_object) {
        for (name, cursor) in checkpoints {
            println!("  checkpoint  {name}  {cursor}");
        }
    }
    println!();
}

fn event_label(event: &Event) -> String {
    for name in ["d", "t", "h"] {
        if let Some(value) = tag_values(event, name).into_iter().next() {
            return value;
        }
    }
    let content = event.content.trim().replace('\n', " ");
    if content.is_empty() {
        format!("kind {}", event.kind.as_u16())
    } else {
        content.chars().take(48).collect()
    }
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some(name))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect()
}

fn short(value: &str) -> String {
    if value.chars().count() > 12 {
        format!("{}…", value.chars().take(12).collect::<String>())
    } else {
        value.into()
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    #[test]
    fn graph_projection_keeps_causal_event_links() {
        let keys = Keys::generate();
        let root = EventBuilder::new(Kind::TextNote, "root")
            .sign_with_keys(&keys)
            .expect("root");
        let child = EventBuilder::new(Kind::TextNote, "child")
            .tags([Tag::parse(["e", root.id.to_hex().as_str()]).expect("tag")])
            .sign_with_keys(&keys)
            .expect("child");
        let graph = project(&[root.clone(), child.clone()]);
        assert!(graph.edges.iter().any(|edge| {
            edge.from == child.id.to_hex() && edge.to == root.id.to_hex() && edge.relation == "e"
        }));
    }
}
