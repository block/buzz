#![deny(unsafe_code)]
//! Orbit local-first embedded memory subsystem.
//!
//! Provides SQLite metadata persistence, embedded vector indexing,
//! knowledge graph traversal, and high-level memory facade.

/// Embedded knowledge graph storage and multi-hop traversal.
pub mod graph;
/// SQLite metadata and event store managing `orbit_*` authoritative tables.
pub mod sqlite;
/// Unified memory engine facade orchestrating all stores.
pub mod store;
/// Storage traits for MetadataStore, VectorStore, and GraphStore.
pub mod traits;
/// Embedded dense vector store with cosine similarity retrieval.
pub mod vector;

pub use graph::EmbeddedGraphStore;
pub use sqlite::SqliteMetadataStore;
pub use store::{EmbeddedMemoryStore, MemoryStats};
pub use traits::{GraphStore, MetadataStore, VectorStore};
pub use vector::EmbeddedVectorStore;
