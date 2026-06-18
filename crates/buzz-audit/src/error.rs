use thiserror::Error;

/// Errors that can occur during audit log operations.
#[derive(Debug, Error)]
pub enum AuditError {
    /// A database operation failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Attempted to log a NIP-42 AUTH event (kind 22242), which is forbidden.
    #[error("auth events (kind 22242) must never appear in the audit log")]
    AuthEventForbidden,

    /// The `prev_hash` of an entry does not match the hash of the preceding entry.
    #[error(
        "hash chain integrity violation at seq {seq}: expected prev_hash {expected}, got {actual}"
    )]
    ChainViolation {
        /// Sequence number of the offending entry.
        seq: i64,
        /// Hash that was expected based on the previous entry.
        expected: String,
        /// Hash that was actually found in the entry.
        actual: String,
    },

    /// The stored hash of an entry does not match the recomputed hash.
    #[error("hash mismatch at seq {seq}: stored {stored}, computed {computed}")]
    HashMismatch {
        /// Sequence number of the offending entry.
        seq: i64,
        /// Hash value stored in the database.
        stored: String,
        /// Hash value recomputed from the entry fields.
        computed: String,
    },

    /// An unrecognised action string was found in the database.
    #[error("unknown audit action in DB: {0:?}")]
    UnknownAction(String),

    /// A JSON serialization error occurred (e.g. while canonicalising metadata).
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
