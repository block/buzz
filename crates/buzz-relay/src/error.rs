//! Error types for the relay crate.

use thiserror::Error;

/// Top-level error type for relay operations.
#[derive(Debug, Error)]
pub enum RelayError {
    /// A WebSocket transport error occurred.
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// A JSON serialization or deserialization error.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// A database operation failed.
    #[error("Database error: {0}")]
    Database(#[from] sprout_db::DbError),

    /// An authentication error from the auth service.
    #[error("Auth error: {0}")]
    Auth(#[from] sprout_auth::AuthError),

    /// A pub/sub error from the pubsub service.
    #[error("PubSub error: {0}")]
    PubSub(#[from] sprout_pubsub::PubSubError),

    /// The relay has reached its maximum number of concurrent connections.
    #[error("Connection limit reached")]
    ConnectionLimitReached,

    /// The client has exceeded the allowed request rate.
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// The client attempted an operation that requires authentication.
    #[error("Not authenticated")]
    NotAuthenticated,

    /// The client sent a message that could not be parsed.
    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    /// An unexpected internal error occurred.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Convenience alias for relay operation results.
pub type Result<T> = std::result::Result<T, RelayError>;
