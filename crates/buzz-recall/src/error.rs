#![deny(unsafe_code)]
//! Error types for the recall agent and IDE transcript parsers.

use thiserror::Error;

/// Result type alias for recall operations.
pub type Result<T> = std::result::Result<T, RecallError>;

/// Errors encountered during transcript parsing, redaction, or session ingestion.
#[derive(Debug, Error)]
pub enum RecallError {
    /// Filesystem I/O error.
    #[error("Recall I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("Recall JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML serialization/deserialization error.
    #[error("Recall YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// Database or storage error.
    #[error("Recall database error: {0}")]
    Database(#[from] buzz_db::error::DbError),

    /// AI or embedding error.
    #[error("Recall AI embedder error: {0}")]
    Ai(#[from] buzz_ai::error::AiError),

    /// Parser failure for a specific agent harness.
    #[error("Parser '{agent}' failed on file '{path}': {message}")]
    Parser {
        /// Name of the agent harness.
        agent: &'static str,
        /// File path that caused the error.
        path: String,
        /// Detail of the error.
        message: String,
    },

    /// General custom error.
    #[error("Recall error: {0}")]
    Custom(String),
}
