#![deny(unsafe_code)]
//! Error types for the Orbit AI engine.

use thiserror::Error;

/// Result alias for AI operations.
pub type Result<T> = std::result::Result<T, AiError>;

/// Errors encountered during embedding or reranking execution.
#[derive(Debug, Error)]
pub enum AiError {
    /// ONNX Runtime or tensor inference failure.
    #[error("ONNX error: {0}")]
    OnnxError(String),

    /// HTTP request or API error when connecting to cloud providers.
    #[error("Cloud API error: {0}")]
    HttpError(String),

    /// OS Keyring error when retrieving or storing secrets.
    #[error("Keyring error: {0}")]
    KeyringError(String),

    /// Invalid configuration or missing credentials.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Serialization or JSON parsing error.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Generic internal error.
    #[error("Internal AI error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for AiError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}

impl From<reqwest::Error> for AiError {
    fn from(err: reqwest::Error) -> Self {
        Self::HttpError(err.to_string())
    }
}
