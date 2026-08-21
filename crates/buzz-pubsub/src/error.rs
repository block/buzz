use thiserror::Error;

/// Errors that can occur in pub/sub, presence, and typing operations.
#[derive(Debug, Error)]
pub enum PubSubError {
    /// A Redis command failed.
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    /// Failed to acquire a connection from the Redis pool.
    #[error("Redis pool error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),

    /// JSON serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The broadcast receiver fell behind and dropped messages.
    #[error("Broadcast receiver lagged: {0} messages dropped")]
    BroadcastLagged(u64),

    /// The pub/sub subscriber task has stopped unexpectedly.
    #[error("Pub/sub subscriber task stopped")]
    SubscriberStopped,

    /// A Redis channel key could not be parsed as a valid channel ID.
    #[error("Invalid channel key: {0}")]
    InvalidChannelKey(String),
}

impl From<tokio::sync::broadcast::error::RecvError> for PubSubError {
    fn from(e: tokio::sync::broadcast::error::RecvError) -> Self {
        match e {
            tokio::sync::broadcast::error::RecvError::Lagged(n) => PubSubError::BroadcastLagged(n),
            tokio::sync::broadcast::error::RecvError::Closed => PubSubError::SubscriberStopped,
        }
    }
}
