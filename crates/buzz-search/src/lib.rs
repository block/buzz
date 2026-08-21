#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Buzz search — community-scoped Postgres full-text search over Buzz events.
//!
//! The index lives in the `events` table: `search_tsv TSVECTOR GENERATED
//! ALWAYS AS (to_tsvector('simple', content)) STORED`, with `GIN
//! (search_tsv)` as the access path. Because the column is `GENERATED ALWAYS`,
//! every row write *is* the index update — there is no separate indexer, no
//! mpsc queue, no reindex job, no consistency window to reason about. A
//! client cannot forge the tsvector out of sync with the content it signed.
//!
//! This crate is the **query** side. Indexing is the SQL row insert — owned
//! by `buzz-db`. The relay refetches canonical events through `buzz-db`'s
//! scoped fetcher and runs access checks per hit; search is never the access
//! boundary (conformance row 50).
//!
//! ## Multi-tenant fence
//!
//! Every [`SearchQuery`] carries a [`CommunityId`]. There is no construction
//! path through this crate that omits it, and every SQL execution binds
//! `community_id = $ctx` as the first predicate. A query bound to community
//! A cannot return events stored under community B, by construction.

/// Search error types.
pub mod error;
/// Search query execution.
pub mod query;

pub use buzz_core::CommunityId;
pub use error::SearchError;
pub use query::{search, ChannelScope, SearchHit, SearchMode, SearchQuery, SearchResult};

use sqlx::PgPool;

/// Thin handle around a `PgPool` for community-scoped FTS.
///
/// Holds nothing the pool itself doesn't already own. The whole purpose of
/// this type is a stable injection point for the relay's `AppState`.
#[derive(Debug, Clone)]
pub struct SearchService {
    pool: PgPool,
}

impl SearchService {
    /// Build a search service over an existing Postgres pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Execute a community-scoped FTS query.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        query::search(&self.pool, query).await
    }
}
