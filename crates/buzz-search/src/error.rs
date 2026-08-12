use thiserror::Error;

/// Errors produced by the FTS service.
#[derive(Debug, Error)]
pub enum SearchError {
    /// Search is intentionally unavailable in the selected runtime profile.
    #[error("search unsupported by runtime profile")]
    Unsupported,
    /// A database error from sqlx.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}
