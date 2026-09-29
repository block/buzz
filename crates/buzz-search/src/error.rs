use thiserror::Error;

/// Errors produced by the FTS service and SuperRAG retrieval engine.
#[derive(Debug, Error)]
pub enum SearchError {
    /// A database error from sqlx.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    /// A memory database error from buzz-db.
    #[error("memory db error: {0}")]
    MemoryDb(#[from] buzz_db::DbError),

    /// An AI or embedding error from buzz-ai.
    #[error("ai error: {0}")]
    Ai(#[from] buzz_ai::AiError),

    /// Internal error during query routing, fusion, or reranking.
    #[error("internal search error: {0}")]
    Internal(String),
}

