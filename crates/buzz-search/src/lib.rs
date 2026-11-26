#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Buzz search — community-scoped message search over Buzz events.
//!
//! [`SearchService`] answers from one of two backends: Postgres full-text
//! search, described below and still the one the relay uses, or the
//! [`opensearch`] index, which [`indexer::Indexer`] keeps in sync from the
//! `search_outbox` queue.
//!
//! ## Postgres full-text search
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
//! `community_id = $ctx` as the first predicate; every OpenSearch query
//! filters on the community first. A query bound to community
//! A cannot return events stored under community B, by construction.

mod aws_credentials;
#[doc(hidden)]
pub mod cancel;
/// Search error types.
pub mod error;
/// The OpenSearch indexer worker.
pub mod indexer;
/// The OpenSearch message index.
pub mod opensearch;
/// Search query execution.
pub mod query;

pub use buzz_core::CommunityId;
pub use error::SearchError;
pub use query::{
    search, search_profile_names, ChannelScope, SearchHit, SearchMode, SearchQuery, SearchResult,
    SearchSort, PROFILE_NAMES_PER_PAGE_MAX,
};

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Options for the pool behind [`SearchService`]: every new connection's
/// session `statement_timeout` is lowered to buzz-db's event-read deadline
/// ([`buzz_db::event::READ_STATEMENT_TIMEOUT`]) unless it is already
/// tighter. Searches cancel their statement when abandoned (see
/// [`cancel`]); this bounds the ones a cancel cannot reach.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new().after_connect(|conn, _meta| {
        Box::pin(buzz_db::event::tighten_session_read_statement_timeout(conn))
    })
}

/// The single entry point for community-scoped message search: a
/// [`SearchQuery`] in, a [`SearchResult`] of hit keys out.
///
/// Which engine answers [`SearchService::search`] is internal to this type,
/// so callers and their tests don't depend on it. Profile name search always
/// runs on Postgres.
#[derive(Debug, Clone)]
pub struct SearchService {
    pool: PgPool,
    backend: Backend,
}

/// The engine behind [`SearchService::search`].
#[derive(Debug, Clone)]
enum Backend {
    /// Postgres full-text search over `events.search_tsv`.
    Postgres,
    /// The OpenSearch message index, kept in sync by [`indexer::Indexer`].
    OpenSearch(Box<opensearch::SearchIndex>),
}

impl SearchService {
    /// Build a search service over an existing Postgres pool, searching with
    /// Postgres full-text search.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            backend: Backend::Postgres,
        }
    }

    /// Build a search service that answers message searches from the
    /// OpenSearch `index`. Profile name search stays on `pool`.
    ///
    /// Only tests select this backend for now; the relay still searches
    /// Postgres.
    pub fn with_opensearch(pool: PgPool, index: opensearch::SearchIndex) -> Self {
        Self {
            pool,
            backend: Backend::OpenSearch(Box::new(index)),
        }
    }

    /// Execute a community-scoped message search.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        match &self.backend {
            Backend::Postgres => query::search(&self.pool, query).await,
            Backend::OpenSearch(index) => index.search(query).await,
        }
    }

    /// Find people by profile name, and by verified profile names signed by
    /// `verified_by`; see [`search_profile_names`].
    pub async fn search_profile_names(
        &self,
        community: CommunityId,
        q: &str,
        verified_by: Option<[u8; 32]>,
        page: u32,
        per_page: u32,
    ) -> Result<SearchResult, SearchError> {
        query::search_profile_names(&self.pool, community, q, verified_by, page, per_page).await
    }
}
