//! Database error types.

use thiserror::Error;

/// Errors produced by database operations.
#[derive(Debug, Error)]
pub enum DbError {
    /// A SQLx driver-level error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A SQLx migration error.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// Attempted to store an AUTH event (kind 22242), which is forbidden.
    #[error("AUTH events (kind 22242) must not be stored")]
    AuthEventRejected,

    /// Attempted to store an ephemeral event (kinds 20000–29999), which is forbidden.
    #[error("ephemeral events (kind {0}) must not be stored")]
    EphemeralEventRejected(u16),

    /// The requested channel does not exist.
    #[error("channel not found: {0}")]
    ChannelNotFound(uuid::Uuid),

    /// The requested member is not in the channel.
    #[error("member not found in channel {0}")]
    MemberNotFound(uuid::Uuid),

    /// A generic not-found error.
    #[error("not found: {0}")]
    NotFound(String),

    /// The caller lacks permission for the requested operation.
    #[error("access denied: {0}")]
    AccessDenied(String),

    /// JSON serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// A value in the database is malformed or unexpected.
    #[error("invalid data: {0}")]
    InvalidData(String),

    /// A serving write admitted before the lifecycle transition is still live.
    /// This is an ordinary retryable drain condition, not a safety violation.
    #[error(
        "community {community_id} still has {active_count} active serving write lease(s): {operations:?}"
    )]
    ServingWritesNotDrained {
        /// Community whose lifecycle transition must retry.
        community_id: uuid::Uuid,
        /// Number of currently unexpired serving-write leases.
        active_count: i64,
        /// Distinct operation categories holding those leases.
        operations: Vec<String>,
    },

    /// A deletion safety invariant is structurally violated and requires
    /// operator/code remediation rather than blind retry.
    #[error("deletion safety error: {0}")]
    DeletionSafety(String),

    /// A stored timestamp value could not be interpreted.
    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(i64),

    /// A roster mutation would remove the last effective relay Operator,
    /// leaving no one able to administer the deployment through the API.
    /// The transaction is rolled back and the caller must add a replacement
    /// Operator before demoting or deleting the current one.
    #[error("operation would remove the last relay operator")]
    LastOperator,
}

/// Convenience alias for `Result<T, DbError>`.
pub type Result<T> = std::result::Result<T, DbError>;
