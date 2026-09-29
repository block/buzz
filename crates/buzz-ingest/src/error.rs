#![deny(unsafe_code)]
//! Error types for the Orbit ingestion pipeline.

use thiserror::Error;

/// Result alias for ingestion pipeline operations.
pub type Result<T> = std::result::Result<T, IngestError>;

/// Errors encountered during file watching, AST chunking, or ingestion orchestration.
#[derive(Debug, Error)]
pub enum IngestError {
    /// Filesystem or I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Database or persistence error.
    #[error("Database error: {0}")]
    Database(#[from] buzz_db::error::DbError),

    /// AI or embedding error.
    #[error("AI error: {0}")]
    Ai(#[from] buzz_ai::error::AiError),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Language AST or chunking error.
    #[error("Chunking error: {0}")]
    Chunking(String),

    /// Git metadata extraction error.
    #[error("Git error: {0}")]
    Git(String),

    /// Watcher error.
    #[error("Watcher error: {0}")]
    Watcher(String),
}
