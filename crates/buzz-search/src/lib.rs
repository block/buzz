#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Buzz search — community-scoped message search over Buzz events.
//!
//! [`SearchService`] answers from one of two backends: Postgres full-text
//! search, described below, or the [`opensearch`] index, which
//! [`indexer::Indexer`] keeps in sync from the `search_outbox` queue. With
//! OpenSearch configured, the `search.backend` flag picks for the whole relay
//! between them, or runs OpenSearch as a shadow of Postgres ([`backend`]).
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
/// Choosing the relay's search backend, and shadow comparisons.
pub mod backend;
#[doc(hidden)]
pub mod cancel;
/// Search error types.
pub mod error;
/// The OpenSearch indexer worker.
pub mod indexer;
/// The OpenSearch message index.
pub mod opensearch;
/// Gauges over the indexer's queue.
pub mod outbox_metrics;
/// Search query execution.
pub mod query;

pub use backend::{BackendSetting, SEARCH_BACKEND_FLAG};
pub use buzz_core::CommunityId;
pub use error::SearchError;
pub use query::{
    search, search_profile_names, ChannelScope, ScoreParts, ScoredHit, ScoredResult, SearchHit,
    SearchMode, SearchQuery, SearchResult, SearchSort, PROFILE_NAMES_PER_PAGE_MAX,
};

use std::sync::Arc;

use buzz_feature_flags::FlagEvaluator;
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
/// so callers and their tests don't depend on it: Postgres, unless the
/// service has an OpenSearch index, in which case [`backend`] chooses per
/// search. Profile name search always runs on Postgres.
#[derive(Debug, Clone)]
pub struct SearchService {
    pool: PgPool,
    opensearch: Option<backend::OpenSearchBackend>,
}

impl SearchService {
    /// Build a search service over an existing Postgres pool, searching with
    /// Postgres full-text search only.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            opensearch: None,
        }
    }

    /// Build a search service that answers word-mode message searches from
    /// the OpenSearch `index`, whatever any flag says. For tests and tools.
    pub fn with_opensearch(pool: PgPool, index: opensearch::SearchIndex) -> Self {
        Self::with_selection(
            pool,
            index,
            backend::Selection::Fixed(BackendSetting::OpenSearch),
        )
    }

    /// Build a search service that chooses Postgres, shadow or OpenSearch
    /// for the whole relay from [`SEARCH_BACKEND_FLAG`] in `flags`, read on every
    /// search; see [`backend`].
    pub fn with_opensearch_switch(
        pool: PgPool,
        index: opensearch::SearchIndex,
        flags: Arc<dyn FlagEvaluator>,
    ) -> Self {
        Self::with_selection(pool, index, backend::Selection::Flag(flags))
    }

    fn with_selection(
        pool: PgPool,
        index: opensearch::SearchIndex,
        selection: backend::Selection,
    ) -> Self {
        Self {
            pool,
            opensearch: Some(backend::OpenSearchBackend::new(index, selection)),
        }
    }

    /// Give shadow OpenSearch searches `deadline` instead of
    /// [`backend::SHADOW_DEADLINE`]. No effect without OpenSearch.
    pub fn with_shadow_deadline(mut self, deadline: std::time::Duration) -> Self {
        if let Some(opensearch) = &mut self.opensearch {
            opensearch.set_shadow_deadline(deadline);
        }
        self
    }

    /// Execute a community-scoped message search.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        use backend::{primary, Engine, Route};
        let route = match &self.opensearch {
            None => Route::Postgres,
            Some(opensearch) => opensearch.route(query),
        };
        let postgres = || primary(Engine::Postgres, query, query::search(&self.pool, query));
        match route {
            Route::Postgres => postgres().await,
            Route::OpenSearch(index) => {
                primary(Engine::OpenSearch, query, index.search(query)).await
            }
            Route::Shadow(compare) => {
                let result = postgres().await;
                // Fails only if the shadow task panicked; the result stands.
                let _ = compare.send(result.as_ref().ok().map(backend::top_ids));
                result
            }
        }
    }

    /// [`SearchService::search`] with each hit's score parts, for tests and
    /// ranking evaluation. It asks the backend that would serve the search,
    /// Postgres while the relay shadows, and records no metrics.
    pub async fn search_scored(&self, query: &SearchQuery) -> Result<ScoredResult, SearchError> {
        match self
            .opensearch
            .as_ref()
            .and_then(|opensearch| opensearch.serving(query))
        {
            Some(index) => index.search_scored(query).await,
            None => query::search(&self.pool, query).await.map(Into::into),
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
