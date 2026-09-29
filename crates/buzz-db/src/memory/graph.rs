#![deny(unsafe_code)]
//! Embedded graph store implementation for Orbit memory.
//!
//! Stores entities and bi-temporal relations in `~/.orbit/brain/graph/`
//! with multi-hop neighborhood traversal and crash-safe file persistence.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use buzz_core::memory::{Entity, GraphHit, Relation};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::{DbError, Result};
use crate::memory::traits::GraphStore;

#[derive(Debug, Default, Serialize, Deserialize)]
struct GraphData {
    entities: HashMap<Uuid, Entity>,
    relations: Vec<Relation>,
}

/// Embedded persistent knowledge graph with bi-temporal relation tracking and BFS traversal.
#[derive(Debug, Clone)]
pub struct EmbeddedGraphStore {
    data_dir: PathBuf,
    // ponytail: adjacency list with atomic JSON journal, upgrade to Ladybug/Kùzu embedded engine when graph nodes > 500k
    state: Arc<RwLock<GraphData>>,
}

impl EmbeddedGraphStore {
    /// Opens or creates the graph store at the specified directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let data_dir = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| DbError::Internal(format!("Failed to create graph store dir {}: {e}", data_dir.display())))?;

        let store = Self {
            data_dir,
            state: Arc::new(RwLock::new(GraphData::default())),
        };

        store.load_from_disk()?;
        Ok(store)
    }

    /// Opens the default graph store at `~/.orbit/brain/graph/`.
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let default_path = PathBuf::from(home)
            .join(".orbit")
            .join("brain")
            .join("graph");
        Self::open(default_path)
    }

    fn journal_file(&self) -> PathBuf {
        self.data_dir.join("graph.bin")
    }

    fn load_from_disk(&self) -> Result<()> {
        let path = self.journal_file();
        if !path.exists() {
            return Ok(());
        }

        let file = File::open(&path)
            .map_err(|e| DbError::Internal(format!("Failed to open graph journal {}: {e}", path.display())))?;
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|e| DbError::Internal(format!("Failed to read graph journal: {e}")))?;

        if bytes.is_empty() {
            return Ok(());
        }

        let data: GraphData = serde_json::from_slice(&bytes)
            .map_err(|e| DbError::Internal(format!("Failed to deserialize graph journal: {e}")))?;

        let mut lock = self.state.write().unwrap();
        *lock = data;
        Ok(())
    }

    fn save_to_disk(&self) -> Result<()> {
        let path = self.journal_file();
        let temp_path = self.data_dir.join("graph.bin.tmp");

        let bytes = {
            let lock = self.state.read().unwrap();
            serde_json::to_vec(&*lock)
                .map_err(|e| DbError::Internal(format!("Failed to serialize graph journal: {e}")))?
        };

        {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_path)
                .map_err(|e| DbError::Internal(format!("Failed to create temp graph journal: {e}")))?;
            let mut writer = BufWriter::new(file);
            writer
                .write_all(&bytes)
                .map_err(|e| DbError::Internal(format!("Failed to write graph journal: {e}")))?;
            writer
                .flush()
                .map_err(|e| DbError::Internal(format!("Failed to flush graph journal: {e}")))?;
        }

        std::fs::rename(&temp_path, &path)
            .map_err(|e| DbError::Internal(format!("Failed to atomically replace graph journal: {e}")))?;

        Ok(())
    }

    /// Finds entities by name matching a query string within a workspace.
    pub fn find_entities_by_name(&self, workspace_path: &str, query: &str) -> Vec<Entity> {
        let lock = self.state.read().unwrap();
        let q = query.to_ascii_lowercase();
        lock.entities
            .values()
            .filter(|e| {
                (workspace_path.is_empty() || e.workspace_path == workspace_path)
                    && (e.name.to_ascii_lowercase().contains(&q) || q.contains(&e.name.to_ascii_lowercase()))
            })
            .cloned()
            .collect()
    }
}

impl GraphStore for EmbeddedGraphStore {
    async fn upsert_entities(&self, entities: &[Entity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }

        {
            let mut lock = self.state.write().unwrap();
            for entity in entities {
                // Duplicate entity merging: check by ID first, then by (workspace, type, name)
                let existing_id = if lock.entities.contains_key(&entity.id) {
                    Some(entity.id)
                } else {
                    lock.entities.values().find(|e| {
                        e.workspace_path == entity.workspace_path
                            && e.entity_type.eq_ignore_ascii_case(&entity.entity_type)
                            && e.name.eq_ignore_ascii_case(&entity.name)
                    }).map(|e| e.id)
                };

                if let Some(target_id) = existing_id {
                    if let Some(existing) = lock.entities.get_mut(&target_id) {
                        existing.merge_with(entity);
                    }
                } else {
                    lock.entities.insert(entity.id, entity.clone());
                }
            }
        }

        self.save_to_disk()?;
        Ok(())
    }

    async fn upsert_relations(&self, relations: &[Relation]) -> Result<()> {
        if relations.is_empty() {
            return Ok(());
        }

        {
            let mut lock = self.state.write().unwrap();
            for relation in relations {
                if let Some(pos) = lock.relations.iter().position(|r| r.id == relation.id) {
                    lock.relations[pos] = relation.clone();
                } else {
                    lock.relations.push(relation.clone());
                }
            }
        }

        self.save_to_disk()?;
        Ok(())
    }

    async fn neighborhood(&self, seed_ids: &[Uuid], hops: u8) -> Result<Vec<GraphHit>> {
        self.neighborhood_as_of(seed_ids, hops, None, None).await
    }

    async fn neighborhood_as_of(
        &self,
        seed_ids: &[Uuid],
        hops: u8,
        as_of: Option<DateTime<Utc>>,
        max_nodes: Option<usize>,
    ) -> Result<Vec<GraphHit>> {
        let lock = self.state.read().unwrap();
        let max_nodes = max_nodes.unwrap_or(100);
        let mut results = Vec::new();
        let mut visited = HashSet::new();
        let mut queue: VecDeque<(Uuid, u8, Option<Relation>)> = VecDeque::new();

        for seed in seed_ids {
            if lock.entities.contains_key(seed) && visited.insert(*seed) {
                queue.push_back((*seed, 0, None));
            }
        }

        while let Some((curr_id, depth, incoming_rel)) = queue.pop_front() {
            if let Some(entity) = lock.entities.get(&curr_id) {
                results.push(GraphHit {
                    entity: entity.clone(),
                    relation: incoming_rel,
                    depth,
                });
                if results.len() >= max_nodes {
                    break;
                }
            }

            if depth < hops {
                // Find all outgoing and incoming relations that are valid at `as_of`
                for rel in &lock.relations {
                    let is_valid = match as_of {
                        Some(t) => rel.is_valid_at(t),
                        None => rel.is_active(),
                    };
                    if !is_valid {
                        continue;
                    }

                    if rel.source_entity_id == curr_id && !visited.contains(&rel.target_entity_id) {
                        visited.insert(rel.target_entity_id);
                        queue.push_back((rel.target_entity_id, depth + 1, Some(rel.clone())));
                    } else if rel.target_entity_id == curr_id && !visited.contains(&rel.source_entity_id) {
                        visited.insert(rel.source_entity_id);
                        queue.push_back((rel.source_entity_id, depth + 1, Some(rel.clone())));
                    }
                }
            }
        }

        Ok(results)
    }

    async fn invalidate_relation(&self, id: &Uuid, invalid_at: Option<DateTime<Utc>>) -> Result<bool> {
        let mut found = false;
        {
            let mut lock = self.state.write().unwrap();
            if let Some(rel) = lock.relations.iter_mut().find(|r| r.id == *id) {
                rel.invalidate(invalid_at);
                found = true;
            }
        }

        if found {
            self.save_to_disk()?;
        }
        Ok(found)
    }

    async fn resolve_contradiction(
        &self,
        workspace_path: &str,
        source_id: &Uuid,
        target_id: &Uuid,
        relation_type: &str,
        replacement: &Relation,
    ) -> Result<usize> {
        let mut count = 0;
        {
            let mut lock = self.state.write().unwrap();
            for rel in lock.relations.iter_mut() {
                if (workspace_path.is_empty() || rel.workspace_path == workspace_path)
                    && rel.source_entity_id == *source_id
                    && rel.target_entity_id == *target_id
                    && rel.relation_type.eq_ignore_ascii_case(relation_type)
                    && rel.is_active()
                {
                    rel.invalidate(Some(replacement.valid_at));
                    count += 1;
                }
            }
            // Add replacement relation
            if let Some(pos) = lock.relations.iter().position(|r| r.id == replacement.id) {
                lock.relations[pos] = replacement.clone();
            } else {
                lock.relations.push(replacement.clone());
            }
        }

        self.save_to_disk()?;
        Ok(count)
    }

    async fn get_entity(&self, id: &Uuid) -> Result<Option<Entity>> {
        let lock = self.state.read().unwrap();
        Ok(lock.entities.get(id).cloned())
    }

    async fn get_relation(&self, id: &Uuid) -> Result<Option<Relation>> {
        let lock = self.state.read().unwrap();
        Ok(lock.relations.iter().find(|r| r.id == *id).cloned())
    }

    async fn list_entities(&self, workspace_path: &str) -> Result<Vec<Entity>> {
        let lock = self.state.read().unwrap();
        let entities = lock.entities.values()
            .filter(|e| workspace_path.is_empty() || e.workspace_path == workspace_path)
            .cloned()
            .collect();
        Ok(entities)
    }

    async fn list_relations(&self, workspace_path: &str, active_only: bool) -> Result<Vec<Relation>> {
        let lock = self.state.read().unwrap();
        let relations = lock.relations.iter()
            .filter(|r| {
                (workspace_path.is_empty() || r.workspace_path == workspace_path)
                    && (!active_only || r.is_active())
            })
            .cloned()
            .collect();
        Ok(relations)
    }

    async fn delete_entity(&self, id: &Uuid) -> Result<bool> {
        let mut modified = false;
        {
            let mut lock = self.state.write().unwrap();
            if lock.entities.remove(id).is_some() {
                modified = true;
            }
            // Cascade delete relations connected to this entity
            let rel_before = lock.relations.len();
            lock.relations.retain(|r| r.source_entity_id != *id && r.target_entity_id != *id);
            if lock.relations.len() < rel_before {
                modified = true;
            }
        }

        if modified {
            self.save_to_disk()?;
        }
        Ok(modified)
    }

    async fn delete_by_provenance_chunk(&self, chunk_id: &Uuid) -> Result<usize> {
        let count;
        {
            let mut lock = self.state.write().unwrap();
            let before = lock.relations.len();
            lock.relations.retain(|r| r.provenance_chunk_id != Some(*chunk_id));
            count = before - lock.relations.len();
        }

        if count > 0 {
            self.save_to_disk()?;
        }
        Ok(count)
    }

    async fn delete_by_document_id(&self, _document_id: &Uuid) -> Result<usize> {
        // ponytail: relations link to chunks; when chunks delete, provenance links clear
        Ok(0)
    }

    async fn count_entities(&self) -> Result<usize> {
        let lock = self.state.read().unwrap();
        Ok(lock.entities.len())
    }

    async fn count_relations(&self) -> Result<usize> {
        let lock = self.state.read().unwrap();
        Ok(lock.relations.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_graph_store_traversal_and_reopen() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_graph_test_{}", Uuid::new_v4()));

        let ent1 = Entity::new("/workspace", "Postgres", "Technology", "Relational database");
        let ent2 = Entity::new("/workspace", "Sqlite", "Technology", "Embedded database");
        let ent3 = Entity::new("/workspace", "LanceDB", "Technology", "Vector database");

        let rel1 = Relation::new("/workspace", ent1.id, ent2.id, "alternatives", 0.9);
        let rel2 = Relation::new("/workspace", ent2.id, ent3.id, "complements", 0.85);

        // 1. Insert and traverse
        {
            let store = EmbeddedGraphStore::open(&temp_dir).expect("open graph store");
            store.upsert_entities(&[ent1.clone(), ent2.clone(), ent3.clone()]).await.expect("upsert entities");
            store.upsert_relations(&[rel1.clone(), rel2.clone()]).await.expect("upsert relations");

            assert_eq!(store.count_entities().await.unwrap(), 3);
            assert_eq!(store.count_relations().await.unwrap(), 2);

            // 2-hop neighborhood starting from ent1
            let neighborhood = store.neighborhood(&[ent1.id], 2).await.expect("neighborhood");
            assert_eq!(neighborhood.len(), 3);
            assert_eq!(neighborhood[0].entity.name, "Postgres");
            assert_eq!(neighborhood[0].depth, 0);
            assert_eq!(neighborhood[1].entity.name, "Sqlite");
            assert_eq!(neighborhood[1].depth, 1);
            assert_eq!(neighborhood[2].entity.name, "LanceDB");
            assert_eq!(neighborhood[2].depth, 2);
        }

        // 2. Reopen and verify persistence
        {
            let store = EmbeddedGraphStore::open(&temp_dir).expect("reopen graph store");
            assert_eq!(store.count_entities().await.unwrap(), 3);
            assert_eq!(store.count_relations().await.unwrap(), 2);

            let deleted = store.delete_entity(&ent2.id).await.expect("delete entity");
            assert!(deleted);
            assert_eq!(store.count_entities().await.unwrap(), 2);
            // Relations referencing ent2 should be cascade-deleted
            assert_eq!(store.count_relations().await.unwrap(), 0);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_duplicate_entity_merge_and_contradiction_resolution() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_graph_merge_test_{}", Uuid::new_v4()));
        let store = EmbeddedGraphStore::open(&temp_dir).expect("open graph store");

        // 1. Entity upsert with duplicate merge
        let id = Entity::deterministic_id("/workspace", "Technology", "Postgres");
        let mut ent1 = Entity::new_deterministic("/workspace", "Postgres", "Technology", "Relational database");
        ent1.metadata = serde_json::json!({"version": "15"});

        store.upsert_entities(&[ent1]).await.expect("upsert ent1");
        assert_eq!(store.count_entities().await.unwrap(), 1);

        // Upsert duplicate with additional description and metadata
        let mut ent2 = Entity::new_deterministic("/workspace", "Postgres", "Technology", "High reliability ACID engine");
        ent2.metadata = serde_json::json!({"port": 5432});

        store.upsert_entities(&[ent2]).await.expect("upsert ent2 duplicate");
        // Count should still be 1 (merged)
        assert_eq!(store.count_entities().await.unwrap(), 1);

        let fetched = store.get_entity(&id).await.unwrap().expect("entity exists");
        assert!(fetched.description.contains("Relational database"));
        assert!(fetched.description.contains("High reliability ACID engine"));
        assert_eq!(fetched.metadata["version"], "15");
        assert_eq!(fetched.metadata["port"], 5432);

        // 2. Contradiction resolution and bi-temporal query
        let arch_node = Entity::new_deterministic("/workspace", "Architecture", "Concept", "Primary data store");
        store.upsert_entities(&[arch_node.clone()]).await.unwrap();

        let t0 = Utc::now() - chrono::Duration::hours(2);
        let mut old_rel = Relation::new("/workspace", arch_node.id, id, "USES_DATABASE", 1.0);
        old_rel.valid_at = t0;
        store.upsert_relations(&[old_rel.clone()]).await.unwrap();

        // Active relations at t0
        let hits_t0 = store.neighborhood_as_of(&[arch_node.id], 1, Some(t0 + chrono::Duration::minutes(10)), None).await.unwrap();
        assert_eq!(hits_t0.len(), 2); // arch_node + postgres

        // New decision replaces old database with Sqlite
        let sqlite_node = Entity::new_deterministic("/workspace", "Sqlite", "Technology", "Embedded SQL database");
        store.upsert_entities(&[sqlite_node.clone()]).await.unwrap();

        let t1 = Utc::now();
        let mut replacement = Relation::new("/workspace", arch_node.id, sqlite_node.id, "USES_DATABASE", 1.0);
        replacement.valid_at = t1;

        let invalidated_count = store
            .resolve_contradiction("/workspace", &arch_node.id, &id, "USES_DATABASE", &replacement)
            .await
            .unwrap();
        assert_eq!(invalidated_count, 1);

        // Current active relations should have Sqlite, not Postgres
        let active_rels = store.list_relations("/workspace", true).await.unwrap();
        assert_eq!(active_rels.len(), 1);
        assert_eq!(active_rels[0].target_entity_id, sqlite_node.id);

        // Historical query as of t0 + 10min should still see Postgres!
        let hist_hits = store.neighborhood_as_of(&[arch_node.id], 1, Some(t0 + chrono::Duration::minutes(10)), None).await.unwrap();
        assert!(hist_hits.iter().any(|h| h.entity.id == id));
        assert!(!hist_hits.iter().any(|h| h.entity.id == sqlite_node.id));

        // Present query (as_of None) sees Sqlite!
        let now_hits = store.neighborhood_as_of(&[arch_node.id], 1, None, None).await.unwrap();
        assert!(now_hits.iter().any(|h| h.entity.id == sqlite_node.id));
        assert!(!now_hits.iter().any(|h| h.entity.id == id));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
