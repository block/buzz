use thiserror::Error;

/// [`SearchError::Engine`] text for a request the client gave up on.
pub(crate) const REQUEST_TIMED_OUT: &str = "request timed out";
/// [`SearchError::Engine`] text for a search OpenSearch ended at its own
/// timeout (`timed_out: true`).
pub(crate) const SEARCH_TIMED_OUT: &str = "search timed out";

/// Errors produced by the search service.
#[derive(Debug, Error)]
pub enum SearchError {
    /// A database error from sqlx.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    /// OpenSearch failed, rejected the request, or returned a partial
    /// result. The text is a fixed description, never the search text.
    #[error("search engine error: {0}")]
    Engine(String),
    /// OpenSearch refused this request itself (HTTP 400 or 413), so sending
    /// it again fails the same way. The text is a fixed description.
    #[error("search engine rejected the request: {0}")]
    Rejected(String),
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

    /// Whether the search ran out of time rather than failed: the Postgres
    /// statement timeout, the OpenSearch client timeout, or OpenSearch's own
    /// search timeout.
    pub fn is_timeout(&self) -> bool {
        self.is_statement_cancelled()
            || matches!(
                self,
                SearchError::Engine(description)
                    if description == REQUEST_TIMED_OUT || description == SEARCH_TIMED_OUT
            )
    }

    /// A fixed description of this error for log lines: never the message,
    /// which can repeat the search text.
    pub fn kind(&self) -> String {
        match self {
            SearchError::Db(e) => buzz_db::error::sqlx_error_kind(e),
            SearchError::Engine(description) => format!("opensearch: {description}"),
            SearchError::Rejected(description) => format!("opensearch rejected: {description}"),
        }
    }
}
