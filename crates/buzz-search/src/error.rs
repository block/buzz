use thiserror::Error;

/// Errors produced by the FTS service.
#[derive(Debug, Error)]
pub enum SearchError {
    /// A database error from sqlx.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl SearchError {
    /// Whether Postgres cancelled the search (SQLSTATE 57014), as the search
    /// pool's statement deadline does.
    pub fn is_statement_cancelled(&self) -> bool {
        matches!(self, SearchError::Db(sqlx::Error::Database(db)) if db.code().as_deref() == Some("57014"))
    }
}
