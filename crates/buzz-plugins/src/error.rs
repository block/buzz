#![deny(unsafe_code)]
//! Error types for Orbit plugins.

use thiserror::Error;

/// Result type alias for plugin operations.
pub type Result<T> = std::result::Result<T, PluginError>;

/// Plugin error enumeration.
#[derive(Debug, Error)]
pub enum PluginError {
    /// Filesystem I/O error during discovery or parsing.
    #[error("Plugin I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization or deserialization error.
    #[error("Plugin JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Document parsing error with specific context.
    #[error("Transcript parsing error for plugin '{plugin}': {message}")]
    ParseError {
        /// Name of the plugin that failed.
        plugin: &'static str,
        /// Description of the parsing failure.
        message: String,
    },

    /// Custom error message.
    #[error("Plugin error: {0}")]
    Custom(String),
}
