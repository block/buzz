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

#[doc(hidden)]
pub mod cancel;
/// Search error types.
pub mod error;
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

/// Lowest PostgreSQL `server_version_num` the search SQL is written for.
///
/// The kind-0 (profile) path guards its JSON field extraction with
/// `pg_input_is_valid`, which PostgreSQL added in 16. Ingest has validated
/// kind-0 content as JSON since the relay's first release, but the column
/// carries no constraint and no migration ever re-validated historical rows,
/// so the guard stays and the floor is enforced by a startup warning rather
/// than assumed.
pub const MIN_SERVER_VERSION_NUM: i32 = 160_000;

/// Explain why `server_version_num` is below [`MIN_SERVER_VERSION_NUM`], or
/// `None` when the server satisfies the floor.
///
/// The relay logs the returned message at startup so an operator on an older
/// server learns that profile search will fail with `undefined_function`
/// before the first user types a name.
pub fn server_version_shortfall(server_version_num: i32) -> Option<String> {
    (server_version_num < MIN_SERVER_VERSION_NUM).then(|| {
        format!(
            "PostgreSQL server_version_num {server_version_num} is below the search floor \
             {MIN_SERVER_VERSION_NUM} (PostgreSQL 16): kind-0 profile search uses \
             pg_input_is_valid and will fail on this server; upgrade PostgreSQL to 16 or newer"
        )
    })
}

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

    /// Read the connected server's `server_version_num` (for example
    /// `160004` for PostgreSQL 16.4) so the relay can compare it against
    /// [`MIN_SERVER_VERSION_NUM`] at startup.
    pub async fn server_version_num(&self) -> Result<i32, SearchError> {
        let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
            .fetch_one(&self.pool)
            .await?;
        Ok(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortfall_names_the_floor_only_below_postgres_16() {
        let message = server_version_shortfall(150_008).expect("PostgreSQL 15 is below the floor");
        assert!(message.contains("150008"), "{message}");
        assert!(message.contains("pg_input_is_valid"), "{message}");
        assert!(message.contains("PostgreSQL 16"), "{message}");
        assert_eq!(server_version_shortfall(160_000), None);
        assert_eq!(server_version_shortfall(170_002), None);
    }
}
