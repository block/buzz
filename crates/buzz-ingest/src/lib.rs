#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-ingest` — File watcher, AST chunker, and ingestion pipeline for Orbit.
//!
//! Converts raw workspace files into content-hash deduplicated, secret-redacted,
//! AST-aware chunks and dense vectors persisted into Orbit's embedded storage.

/// AST-aware semantic chunker for multiple languages.
pub mod chunker;
/// Error types for the ingestion pipeline.
pub mod error;
/// Git repository context and metadata extraction.
pub mod git;
/// Ingestion orchestration pipeline connecting chunker, embedder, and storage.
pub mod pipeline;
/// Filesystem watcher and change detection.
pub mod watcher;

pub use chunker::{AstChunker, SupportedLanguage};
pub use error::{IngestError, Result};
pub use git::{GitMetadata, GitMetadataExtractor};
pub use pipeline::{IngestSummary, IngestionPipeline};
pub use watcher::{FileChangeEvent, WorkspaceWatcher};
