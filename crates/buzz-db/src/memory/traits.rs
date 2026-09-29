#![deny(unsafe_code)]
//! Storage trait abstractions for Orbit memory engine.

use std::fmt::Debug;
use buzz_core::memory::{
    Chunk, Document, EmbeddingRecord, Entity, GraphHit, MemoryFeedback, Relation, VectorFilter,
    VectorHit,
};
use uuid::Uuid;
use crate::error::Result;

/// Relational and event metadata storage trait.
pub trait MetadataStore: Send + Sync + Debug {
    /// Inserts or updates a source document record.
    fn upsert_document(&self, doc: &Document) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Fetches a document by its primary key ID.
    fn get_document(&self, id: &Uuid) -> impl std::future::Future<Output = Result<Option<Document>>> + Send;
    /// Fetches a document by workspace and content hash (for deduplication).
    fn get_document_by_hash(&self, workspace_path: &str, content_hash: &str) -> impl std::future::Future<Output = Result<Option<Document>>> + Send;
    /// Deletes a document by ID.
    fn delete_document(&self, id: &Uuid) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Inserts or updates a processed context chunk.
    fn upsert_chunk(&self, chunk: &Chunk) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Fetches a chunk by its primary key ID.
    fn get_chunk(&self, id: &Uuid) -> impl std::future::Future<Output = Result<Option<Chunk>>> + Send;
    /// Fetches all chunks belonging to a document.
    fn get_chunks_by_document(&self, document_id: &Uuid) -> impl std::future::Future<Output = Result<Vec<Chunk>>> + Send;
    /// Deletes a chunk by ID.
    fn delete_chunk(&self, id: &Uuid) -> impl std::future::Future<Output = Result<bool>> + Send;
    /// Deletes all chunks belonging to a document.
    fn delete_chunks_by_document(&self, document_id: &Uuid) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Records user or agent feedback on a retrieved memory item.
    fn record_feedback(&self, feedback: &MemoryFeedback) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Returns total document count.
    fn count_documents(&self) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Returns total chunk count.
    fn count_chunks(&self) -> impl std::future::Future<Output = Result<usize>> + Send;
}

/// Dense vector storage and nearest-neighbor search trait.
pub trait VectorStore: Send + Sync + Debug {
    /// Inserts or updates dense embedding records.
    fn upsert_embeddings(&self, items: &[EmbeddingRecord]) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Searches for top-k nearest neighbors matching query vector and filter.
    fn search(&self, query: &[f32], filter: &VectorFilter, k: usize) -> impl std::future::Future<Output = Result<Vec<VectorHit>>> + Send;
    /// Deletes embedding by its authoritative chunk identifier.
    fn delete_by_chunk_id(&self, chunk_id: &Uuid) -> impl std::future::Future<Output = Result<bool>> + Send;
    /// Deletes all embeddings belonging to a document.
    fn delete_by_document_id(&self, document_id: &Uuid) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Returns total vector records stored.
    fn count_vectors(&self) -> impl std::future::Future<Output = Result<usize>> + Send;
}

/// Knowledge graph entity and relation traversal trait.
pub trait GraphStore: Send + Sync + Debug {
    /// Inserts or updates semantic entities.
    fn upsert_entities(&self, entities: &[Entity]) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Inserts or updates typed bi-temporal relations.
    fn upsert_relations(&self, relations: &[Relation]) -> impl std::future::Future<Output = Result<()>> + Send;
    /// Traverses the graph up to `hops` distance from seed entity IDs.
    fn neighborhood(&self, seed_ids: &[Uuid], hops: u8) -> impl std::future::Future<Output = Result<Vec<GraphHit>>> + Send;
    /// Deletes an entity by ID.
    fn delete_entity(&self, id: &Uuid) -> impl std::future::Future<Output = Result<bool>> + Send;
    /// Deletes relations associated with a provenance chunk.
    fn delete_by_provenance_chunk(&self, chunk_id: &Uuid) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Deletes all graph records associated with a document ID.
    fn delete_by_document_id(&self, document_id: &Uuid) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Returns total entity count.
    fn count_entities(&self) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Returns total relation count.
    fn count_relations(&self) -> impl std::future::Future<Output = Result<usize>> + Send;
}
