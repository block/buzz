//! Error types for runtime config adapters.

/// Errors returned when reading or validating a runtime's MCP configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("config file exceeds the {limit} byte limit")]
    TooLarge {
        /// Maximum config size accepted by the adapter.
        limit: u64,
    },

    #[error("invalid configuration: {0}")]
    Validation(String),
}
