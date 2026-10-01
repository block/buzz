#![deny(unsafe_code)]
//! High-level unified memory facade for Orbit.
//!
//! Orchestrates SQLite metadata, embedded vector index, and knowledge graph
//! with cascading deletion, content-hash deduplication, and secret redaction.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use buzz_core::memory::{
    Chunk, Document, EmbeddingRecord, Entity, GraphHit, Relation, VectorFilter, VectorHit,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::Result;
use crate::memory::graph::EmbeddedGraphStore;
use crate::memory::sqlite::SqliteMetadataStore;
use crate::memory::traits::{GraphStore, MetadataStore, VectorStore};
use crate::memory::vector::EmbeddedVectorStore;
use crate::redactor::SecretRedactor;

/// High-level memory statistics across all three embedded stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Total ingested documents.
    pub total_documents: usize,
    /// Total processed chunks.
    pub total_chunks: usize,
    /// Total indexed vector embeddings.
    pub total_vectors: usize,
    /// Total semantic entities.
    pub total_entities: usize,
    /// Total bi-temporal relations.
    pub total_relations: usize,
}

/// Unified Orbit memory store coordinating metadata, vector index, and graph traversal.
#[derive(Debug, Clone)]
pub struct EmbeddedMemoryStore {
    base_dir: PathBuf,
    metadata: Arc<SqliteMetadataStore>,
    vectors: Arc<EmbeddedVectorStore>,
    graph: Arc<EmbeddedGraphStore>,
}

impl EmbeddedMemoryStore {
    /// Opens the unified memory engine rooted at `base_path`.
    ///
    /// Subdirectories:
    /// - `base_path/db/orbit.db` (SQLite metadata)
    /// - `base_path/vectors/` (Vector embeddings)
    /// - `base_path/graph/` (Knowledge graph)
    pub fn open(base_path: impl AsRef<Path>) -> Result<Self> {
        let base_dir = base_path.as_ref().to_path_buf();
        let db_path = base_dir.join("db").join("orbit.db");
        let vector_path = base_dir.join("vectors");
        let graph_path = base_dir.join("graph");

        let metadata = Arc::new(SqliteMetadataStore::open(&db_path)?);
        let vectors = Arc::new(EmbeddedVectorStore::open(&vector_path)?);
        let graph = Arc::new(EmbeddedGraphStore::open(&graph_path)?);

        Ok(Self {
            base_dir,
            metadata,
            vectors,
            graph,
        })
    }

    /// Opens the default memory store rooted at `~/.orbit/brain/`.
    pub fn open_default() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let default_base = PathBuf::from(home).join(".orbit").join("brain");
        Self::open(default_base)
    }

    /// Access the underlying metadata store.
    pub fn metadata(&self) -> &Arc<SqliteMetadataStore> {
        &self.metadata
    }

    /// Access the underlying vector store.
    pub fn vectors(&self) -> &Arc<EmbeddedVectorStore> {
        &self.vectors
    }

    /// Access the underlying graph store.
    pub fn graph(&self) -> &Arc<EmbeddedGraphStore> {
        &self.graph
    }

    /// Base directory of this memory store.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Stores a document with idempotent content-hash deduplication.
    pub async fn remember_document(&self, doc: &Document) -> Result<Document> {
        // 1. Idempotency check: if document with matching workspace and content_hash exists, return it
        if let Some(existing) = self.metadata.get_document_by_hash(&doc.workspace_path, &doc.content_hash).await? {
            return Ok(existing);
        }

        self.metadata.upsert_document(doc).await?;
        Ok(doc.clone())
    }

    /// Stores a chunk with automatic secret redaction and optional vector embedding.
    pub async fn remember_chunk(&self, chunk: &Chunk, embedding: Option<Vec<f32>>) -> Result<Chunk> {
        // 1. Run secret redactor before persistent storage
        let sanitized_content = SecretRedactor::redact(&chunk.content);
        let mut clean_chunk = chunk.clone();
        clean_chunk.content = sanitized_content;

        // 2. Persist chunk to SQLite metadata
        self.metadata.upsert_chunk(&clean_chunk).await?;

        // 3. Persist dense vector if provided
        if let Some(emb) = embedding {
            let record = EmbeddingRecord {
                chunk_id: clean_chunk.id,
                document_id: clean_chunk.document_id,
                workspace_path: clean_chunk.workspace_path.clone(),
                embedding: emb,
                content: clean_chunk.content.clone(),
                created_at: clean_chunk.created_at,
            };
            self.vectors.upsert_embeddings(&[record]).await?;
        }

        Ok(clean_chunk)
    }

    /// Stores knowledge graph entities and bi-temporal relations.
    pub async fn remember_graph(&self, entities: &[Entity], relations: &[Relation]) -> Result<()> {
        if !entities.is_empty() {
            self.graph.upsert_entities(entities).await?;
            for ent in entities {
                let _ = self.metadata.upsert_entity(ent);
            }
        }
        if !relations.is_empty() {
            self.graph.upsert_relations(relations).await?;
            for rel in relations {
                let _ = self.metadata.upsert_relation(rel);
            }
        }
        Ok(())
    }

    /// Forgets a document and propagates cascading deletion to vectors and graph provenance.
    pub async fn forget_document(&self, document_id: &Uuid) -> Result<bool> {
        // 1. Fetch all associated chunks to clean up vectors and graph provenance
        let chunks = self.metadata.get_chunks_by_document(document_id).await?;
        for chunk in &chunks {
            self.vectors.delete_by_chunk_id(&chunk.id).await?;
            self.graph.delete_by_provenance_chunk(&chunk.id).await?;
        }

        // 2. Delete vectors by document_id as fallback
        self.vectors.delete_by_document_id(document_id).await?;

        // 3. Delete chunks from SQLite
        self.metadata.delete_chunks_by_document(document_id).await?;

        // 4. Delete document from SQLite
        self.metadata.delete_document(document_id).await
    }

    /// Forgets a chunk and propagates deletion to vector index and graph relations.
    pub async fn forget_chunk(&self, chunk_id: &Uuid) -> Result<bool> {
        self.vectors.delete_by_chunk_id(chunk_id).await?;
        self.graph.delete_by_provenance_chunk(chunk_id).await?;
        self.metadata.delete_chunk(chunk_id).await
    }

    /// Searches vector index for nearest neighbors matching query and filter.
    pub async fn search_vectors(&self, query: &[f32], filter: &VectorFilter, k: usize) -> Result<Vec<VectorHit>> {
        self.vectors.search(query, filter, k).await
    }

    /// Traverses graph neighborhood starting from seed entities.
    pub async fn graph_neighborhood(&self, seeds: &[Uuid], hops: u8) -> Result<Vec<GraphHit>> {
        self.graph.neighborhood(seeds, hops).await
    }

    /// Traverses graph neighborhood as of a specific timestamp with optional max nodes cap.
    pub async fn graph_neighborhood_as_of(
        &self,
        seeds: &[Uuid],
        hops: u8,
        as_of: Option<DateTime<Utc>>,
        max_nodes: Option<usize>,
    ) -> Result<Vec<GraphHit>> {
        self.graph.neighborhood_as_of(seeds, hops, as_of, max_nodes).await
    }

    /// Invalidates a relation by ID across both embedded graph and SQLite metadata.
    pub async fn invalidate_graph_relation(&self, id: &Uuid, invalid_at: Option<DateTime<Utc>>) -> Result<bool> {
        let found = self.graph.invalidate_relation(id, invalid_at).await?;
        let _ = self.metadata.invalidate_relation(id, invalid_at);
        Ok(found)
    }

    /// Resolves contradictions between entities of relation_type, marking obsolete edges invalid in both stores.
    pub async fn resolve_graph_contradiction(
        &self,
        workspace_path: &str,
        source_id: &Uuid,
        target_id: &Uuid,
        relation_type: &str,
        replacement: &Relation,
    ) -> Result<usize> {
        let count = self.graph.resolve_contradiction(workspace_path, source_id, target_id, relation_type, replacement).await?;
        let _ = self.metadata.invalidate_relations_by_edge(
            workspace_path,
            source_id,
            target_id,
            relation_type,
            Some(replacement.valid_at),
        );
        let _ = self.metadata.upsert_relation(replacement);
        Ok(count)
    }

    /// Fetches an entity by ID from the graph.
    pub async fn get_entity(&self, id: &Uuid) -> Result<Option<Entity>> {
        self.graph.get_entity(id).await
    }

    /// Fetches a relation by ID from the graph.
    pub async fn get_relation(&self, id: &Uuid) -> Result<Option<Relation>> {
        self.graph.get_relation(id).await
    }

    /// Lists entities in the graph for a workspace.
    pub async fn list_entities(&self, workspace_path: &str) -> Result<Vec<Entity>> {
        self.graph.list_entities(workspace_path).await
    }

    /// Lists relations in the graph for a workspace, optionally filtering to active only.
    pub async fn list_relations(&self, workspace_path: &str, active_only: bool) -> Result<Vec<Relation>> {
        self.graph.list_relations(workspace_path, active_only).await
    }

    /// Finds entities matching a query in the graph store.
    pub fn find_entities_by_name(&self, workspace_path: &str, query: &str) -> Vec<Entity> {
        self.graph.find_entities_by_name(workspace_path, query)
    }

    /// Gathers cross-store health and count statistics.
    pub async fn stats(&self) -> Result<MemoryStats> {
        let total_documents = self.metadata.count_documents().await?;
        let total_chunks = self.metadata.count_chunks().await?;
        let total_vectors = self.vectors.count_vectors().await?;
        let total_entities = self.graph.count_entities().await?;
        let total_relations = self.graph.count_relations().await?;

        Ok(MemoryStats {
            total_documents,
            total_chunks,
            total_vectors,
            total_entities,
            total_relations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unified_memory_store_workflow_and_delete_propagation() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_memory_facade_test_{}", Uuid::new_v4()));

        // 1. Initialize store
        let store = EmbeddedMemoryStore::open(&temp_dir).expect("open store");

        // 2. Ingest document
        let doc = Document::new(
            "file",
            "/src/auth.rs",
            "/my-workspace",
            b"pub fn authenticate() {}",
            5000,
            serde_json::json!({"repo": "orbit"}),
        );
        let saved_doc = store.remember_document(&doc).await.expect("remember doc");
        assert_eq!(saved_doc.id, doc.id);

        // Idempotent test
        let duplicate_doc = Document::new(
            "file",
            "/src/auth.rs",
            "/my-workspace",
            b"pub fn authenticate() {}",
            5000,
            serde_json::json!({}),
        );
        let dedup = store.remember_document(&duplicate_doc).await.expect("dedup");
        assert_eq!(dedup.id, saved_doc.id);

        // 3. Ingest chunk with secret to verify SecretRedactor
        let raw_chunk = Chunk::new(
            saved_doc.id,
            &saved_doc.workspace_path,
            0,
            "let api_key = \"sk-proj1234567890abcdef1234567890\";",
            "project",
        );
        let saved_chunk = store
            .remember_chunk(&raw_chunk, Some(vec![1.0, 0.0, 0.5]))
            .await
            .expect("remember chunk");

        assert!(saved_chunk.content.contains("[REDACTED:OPENAI_KEY]"));
        assert!(!saved_chunk.content.contains("sk-proj1234567890"));

        // 4. Ingest entity and relation with provenance
        let ent = Entity::new(&saved_doc.workspace_path, "AuthService", "Architecture", "Auth layer");
        let mut rel = Relation::new(
            &saved_doc.workspace_path,
            ent.id,
            ent.id,
            "self_check",
            1.0,
        );
        rel.provenance_chunk_id = Some(saved_chunk.id);
        store.remember_graph(&[ent.clone()], &[rel.clone()]).await.expect("remember graph");

        // 5. Verify stats
        let stats = store.stats().await.expect("stats");
        assert_eq!(stats.total_documents, 1);
        assert_eq!(stats.total_chunks, 1);
        assert_eq!(stats.total_vectors, 1);
        assert_eq!(stats.total_entities, 1);
        assert_eq!(stats.total_relations, 1);

        // 6. Delete propagation test
        let forgotten = store.forget_document(&saved_doc.id).await.expect("forget doc");
        assert!(forgotten);

        let stats_after = store.stats().await.expect("stats after");
        assert_eq!(stats_after.total_documents, 0);
        assert_eq!(stats_after.total_chunks, 0);
        assert_eq!(stats_after.total_vectors, 0);
        assert_eq!(stats_after.total_relations, 0); // relation cleared by provenance
        assert_eq!(stats_after.total_entities, 1); // entity preserved

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
