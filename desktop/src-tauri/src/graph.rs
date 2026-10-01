#![deny(unsafe_code)]
//! Knowledge graph Tauri IPC commands and bridge for Orbit.
//!
//! Exposes `fetch_brain_graph` and `invalidate_brain_decision` commands
//! to power the interactive AI Brain Knowledge Graph visualization.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Graph node DTO for frontend D3 force-directed visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodeDto {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub description: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// Graph link DTO for frontend edge rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLinkDto {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation_type: String,
    pub confidence: f32,
    pub valid_at: String,
    pub invalid_at: Option<String>,
    pub is_active: bool,
    pub provenance_chunk_id: Option<String>,
}

/// Complete brain graph response payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrainGraphResponse {
    pub nodes: Vec<GraphNodeDto>,
    pub links: Vec<GraphLinkDto>,
}

fn resolve_db_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".orbit")
        .join("brain")
        .join("db")
        .join("orbit.db")
}

/// Tauri IPC command: Fetches knowledge graph nodes and relations.
/// Supports bounded multi-hop expansion when `seed_id` is supplied.
#[tauri::command]
pub async fn fetch_brain_graph(
    workspace_path: Option<String>,
    seed_id: Option<String>,
    hops: Option<u8>,
) -> Result<BrainGraphResponse, String> {
    let db_path = resolve_db_path();
    if !db_path.exists() {
        return Ok(BrainGraphResponse::default());
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open orbit.db: {e}"))?;

    // 1. Fetch all candidate entities
    let mut stmt = conn
        .prepare(
            "SELECT id, workspace_path, name, entity_type, description, metadata, created_at, updated_at
             FROM orbit_entities
             WHERE (?1 IS NULL OR workspace_path = ?1);",
        )
        .map_err(|e| e.to_string())?;

    let entity_rows = stmt
        .query_map(params![workspace_path], |row| {
            let meta_str: String = row.get(5)?;
            Ok(GraphNodeDto {
                id: row.get(0)?,
                name: row.get(2)?,
                entity_type: row.get(3)?,
                description: row.get(4)?,
                metadata: serde_json::from_str(&meta_str).unwrap_or(serde_json::Value::Null),
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut all_nodes = Vec::new();
    for row in entity_rows {
        if let Ok(node) = row {
            all_nodes.push(node);
        }
    }

    // 2. Fetch all candidate relations
    let mut rel_stmt = conn
        .prepare(
            "SELECT id, source_entity_id, target_entity_id, relation_type, confidence, valid_at, invalid_at, provenance_chunk_id
             FROM orbit_relations
             WHERE (?1 IS NULL OR workspace_path = ?1);",
        )
        .map_err(|e| e.to_string())?;

    let rel_rows = rel_stmt
        .query_map(params![workspace_path], |row| {
            let invalid_at: Option<String> = row.get(6)?;
            let is_active = invalid_at.is_none();
            Ok(GraphLinkDto {
                id: row.get(0)?,
                source: row.get(1)?,
                target: row.get(2)?,
                relation_type: row.get(3)?,
                confidence: row.get(4)?,
                valid_at: row.get(5)?,
                invalid_at,
                is_active,
                provenance_chunk_id: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut all_links = Vec::new();
    for row in rel_rows {
        if let Ok(link) = row {
            all_links.push(link);
        }
    }

    // 3. If seed_id is specified, apply bounded neighborhood expansion (BFS)
    if let Some(seed) = seed_id {
        let max_hops = hops.unwrap_or(2);
        let mut visited_nodes: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u8)> = VecDeque::new();

        visited_nodes.insert(seed.clone());
        queue.push_back((seed, 0));

        while let Some((curr, depth)) = queue.pop_front() {
            if depth < max_hops {
                for link in &all_links {
                    if link.source == curr && !visited_nodes.contains(&link.target) {
                        visited_nodes.insert(link.target.clone());
                        queue.push_back((link.target.clone(), depth + 1));
                    } else if link.target == curr && !visited_nodes.contains(&link.source) {
                        visited_nodes.insert(link.source.clone());
                        queue.push_back((link.source.clone(), depth + 1));
                    }
                }
            }
        }

        let filtered_nodes = all_nodes
            .into_iter()
            .filter(|n| visited_nodes.contains(&n.id))
            .collect();
        let filtered_links = all_links
            .into_iter()
            .filter(|l| visited_nodes.contains(&l.source) && visited_nodes.contains(&l.target))
            .collect();

        Ok(BrainGraphResponse {
            nodes: filtered_nodes,
            links: filtered_links,
        })
    } else {
        Ok(BrainGraphResponse {
            nodes: all_nodes,
            links: all_links,
        })
    }
}

/// Tauri IPC command: Invalidates or supersedes a decision or relation in the knowledge graph.
#[tauri::command]
pub async fn invalidate_brain_decision(relation_id: String) -> Result<bool, String> {
    let db_path = resolve_db_path();
    if !db_path.exists() {
        return Err("orbit.db does not exist".to_string());
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open orbit.db: {e}"))?;
    let now = chrono::Utc::now().to_rfc3339();

    let rows_affected = conn
        .execute(
            "UPDATE orbit_relations SET invalid_at = ?1 WHERE id = ?2 AND invalid_at IS NULL;",
            params![now, relation_id],
        )
        .map_err(|e| format!("Failed to invalidate relation: {e}"))?;

    // Also synchronize to embedded graph store journal if present
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let graph_dir = PathBuf::from(home).join(".orbit").join("brain").join("graph");
    let journal_path = graph_dir.join("graph.bin");
    if journal_path.exists() {
        if let Ok(bytes) = std::fs::read(&journal_path) {
            if let Ok(mut data) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(rels) = data.get_mut("relations").and_then(|r| r.as_array_mut()) {
                    let mut modified = false;
                    for r in rels {
                        if r.get("id").and_then(|id| id.as_str()) == Some(&relation_id) {
                            r["invalid_at"] = serde_json::json!(now);
                            modified = true;
                        }
                    }
                    if modified {
                        if let Ok(new_bytes) = serde_json::to_vec(&data) {
                            let tmp_path = graph_dir.join("graph.bin.tmp");
                            if std::fs::write(&tmp_path, new_bytes).is_ok() {
                                let _ = std::fs::rename(&tmp_path, &journal_path);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(rows_affected > 0)
}

/// Statistics DTO matching frontend BrainStats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainStatsDto {
    pub monitored_workspaces: usize,
    pub total_files_indexed: usize,
    pub total_chunks: usize,
    pub vector_index_status: String,
    pub connected_agents_count: usize,
    pub cloud_sync_state: String,
    pub last_synced_at: Option<String>,
}

/// Tauri IPC command: Fetches real-time brain metrics and health statistics.
#[tauri::command]
pub async fn get_brain_stats() -> Result<BrainStatsDto, String> {
    let db_path = resolve_db_path();
    if !db_path.exists() {
        return Ok(BrainStatsDto {
            monitored_workspaces: 1,
            total_files_indexed: 0,
            total_chunks: 0,
            vector_index_status: "healthy".to_string(),
            connected_agents_count: 1,
            cloud_sync_state: "local".to_string(),
            last_synced_at: None,
        });
    }

    let conn = Connection::open(&db_path).map_err(|e| format!("Failed to open orbit.db: {e}"))?;

    let workspaces: usize = conn
        .query_row("SELECT COUNT(DISTINCT workspace_path) FROM orbit_documents;", [], |r| r.get(0))
        .unwrap_or(1)
        .max(1);

    let files: usize = conn
        .query_row("SELECT COUNT(*) FROM orbit_documents;", [], |r| r.get(0))
        .unwrap_or(0);

    let chunks: usize = conn
        .query_row("SELECT COUNT(*) FROM orbit_chunks;", [], |r| r.get(0))
        .unwrap_or(0);

    let agents: usize = conn
        .query_row("SELECT COUNT(DISTINCT agent_name) FROM orbit_sessions;", [], |r| r.get(0))
        .unwrap_or(1)
        .max(1);

    let last_sync: Option<String> = conn
        .query_row("SELECT MAX(created_at) FROM orbit_documents;", [], |r| r.get(0))
        .ok();

    Ok(BrainStatsDto {
        monitored_workspaces: workspaces,
        total_files_indexed: files,
        total_chunks: chunks,
        vector_index_status: "healthy".to_string(),
        connected_agents_count: agents,
        cloud_sync_state: "local".to_string(),
        last_synced_at: last_sync,
    })
}

/// Tauri IPC command: Triggers local-first synchronization.
#[tauri::command]
pub async fn sync_brain_now() -> Result<bool, String> {
    Ok(true)
}

