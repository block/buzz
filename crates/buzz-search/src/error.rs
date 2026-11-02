use thiserror::Error;

/// Errors produced by the FTS service.
#[derive(Debug, Error)]
pub enum SearchError {
    /// A database error from sqlx.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl SearchError {
    /// Whether Postgres cancelled the search statement (SQLSTATE 57014): the
    /// search pool's statement timeout ([`crate::pool_options`]) ended it.
    /// Callers answer it with the same terminal deadline response as any
    /// other cancelled read, so clients do not re-run the same expensive
    /// search. A cancel the search sent itself never reaches a caller: it is
    /// sent only after the caller dropped the search.
    pub fn is_statement_cancelled(&self) -> bool {
        matches!(
            self,
            SearchError::Db(sqlx::Error::Database(db)) if db.code().as_deref() == Some("57014")
        )
    }
}
