//! Which engine answers a message search, chosen for the whole relay by the
//! `search.backend` flag, and the shadow comparison between the two.
//!
//! | Flag value | Setting | Who serves | OpenSearch |
//! |---|---|---|---|
//! | 0 (default, and unknown values) | [`BackendSetting::Postgres`] | Postgres | not queried |
//! | 1 | [`BackendSetting::Shadow`] | Postgres | queried off the response path and compared |
//! | 2 | [`BackendSetting::OpenSearch`] | OpenSearch | its failures fail the search |
//!
//! The flag is evaluated for the relay process, not the searched community,
//! so a relay never serves some communities from one engine and some from
//! the other. It is read on every search, so a provider that changes values
//! while the relay runs switches it without a restart.
//!
//! Searches the index cannot answer stay on Postgres at every setting:
//! prefix mode, and any search that names a kind outside
//! [`buzz_db::search_outbox::SEARCHABLE_KINDS`] or names no kinds. That is
//! a routing rule known before the search runs, not a fallback: an
//! OpenSearch failure at setting 2 still fails the search.
//!
//! Metrics and the shadow log line never carry query text.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use buzz_feature_flags::{EvaluationContext, FlagEvaluator, IntegerFlag};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tracing::info;

use buzz_db::search_outbox::SEARCHABLE_KINDS;

use crate::opensearch::SearchIndex;
use crate::{SearchError, SearchMode, SearchQuery, SearchResult};

/// Integer flag choosing the search backend for the relay; see the module
/// docs. Environment form: `BUZZ_FEATURE_FLAG_SEARCH_BACKEND`.
pub const SEARCH_BACKEND_FLAG: IntegerFlag = IntegerFlag::new("search.backend", 0);

/// Longest a shadow OpenSearch query may run before it is abandoned and
/// counted as a timeout.
pub const SHADOW_DEADLINE: Duration = Duration::from_secs(2);

/// Most shadow queries in flight at once, so a slow cluster cannot pile up
/// requests behind live traffic.
pub const SHADOW_MAX_IN_FLIGHT: usize = 32;

/// How many top hits the shadow overlap compares.
pub const OVERLAP_TOP: usize = 10;

/// What the relay's `search.backend` value asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSetting {
    /// Postgres serves; OpenSearch is not queried (0, and unknown values).
    Postgres,
    /// Postgres serves; OpenSearch runs alongside and is compared (1).
    Shadow,
    /// OpenSearch serves (2).
    OpenSearch,
}

impl BackendSetting {
    /// The setting a flag value selects. Unknown values select Postgres.
    pub fn from_flag_value(value: i64) -> Self {
        match value {
            0 => Self::Postgres,
            1 => Self::Shadow,
            2 => Self::OpenSearch,
            _ => {
                metrics::counter!("buzz_search_backend_flag_invalid_total").increment(1);
                Self::Postgres
            }
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Shadow => "shadow",
            Self::OpenSearch => "opensearch",
        }
    }
}

/// How the setting is chosen.
#[derive(Clone)]
pub(crate) enum Selection {
    /// Always this setting (tests and tools).
    Fixed(BackendSetting),
    /// [`SEARCH_BACKEND_FLAG`], evaluated for the relay process.
    Flag(Arc<dyn FlagEvaluator>),
}

impl std::fmt::Debug for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(setting) => f.debug_tuple("Fixed").field(setting).finish(),
            Self::Flag(_) => f.write_str("Flag"),
        }
    }
}

impl Selection {
    /// The one place the switch is read.
    fn setting(&self) -> BackendSetting {
        match self {
            Self::Fixed(setting) => *setting,
            Self::Flag(flags) => BackendSetting::from_flag_value(
                flags.evaluate_int(SEARCH_BACKEND_FLAG, &EvaluationContext::for_process()),
            ),
        }
    }
}

/// The OpenSearch side of a [`crate::SearchService`].
#[derive(Debug, Clone)]
pub(crate) struct OpenSearchBackend {
    index: SearchIndex,
    selection: Selection,
    shadow_deadline: Duration,
    shadow_slots: Arc<Semaphore>,
}

/// Which engine a search runs on, as reported in metrics.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Engine {
    Postgres,
    OpenSearch,
}

impl Engine {
    fn label(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::OpenSearch => "opensearch",
        }
    }
}

/// What the routed search should do.
pub(crate) enum Route<'a> {
    /// Serve from Postgres.
    Postgres,
    /// Serve from this OpenSearch index.
    OpenSearch(&'a SearchIndex),
    /// Serve from Postgres and hand its result to the shadow comparison.
    Shadow(oneshot::Sender<Option<Vec<[u8; 32]>>>),
}

impl OpenSearchBackend {
    pub(crate) fn new(index: SearchIndex, selection: Selection) -> Self {
        Self {
            index,
            selection,
            shadow_deadline: SHADOW_DEADLINE,
            shadow_slots: Arc::new(Semaphore::new(SHADOW_MAX_IN_FLIGHT)),
        }
    }

    pub(crate) fn set_shadow_deadline(&mut self, deadline: Duration) {
        self.shadow_deadline = deadline;
    }

    /// Decide where `query` runs, starting the shadow search if it gets one.
    pub(crate) fn route(&self, query: &SearchQuery) -> Route<'_> {
        let setting = self.selection.setting();
        if setting == BackendSetting::Postgres {
            return Route::Postgres;
        }
        if !index_holds_kinds(query) {
            skipped(setting, "unindexed_kinds");
            return Route::Postgres;
        }
        if query.mode != SearchMode::FullText {
            skipped(setting, "unsupported_mode");
            return Route::Postgres;
        }
        if setting == BackendSetting::OpenSearch {
            return Route::OpenSearch(&self.index);
        }
        // One comparison per search: later pages of the same query would
        // multiply the shadow's load without saying much more.
        if query.page > 1 {
            return Route::Postgres;
        }
        let Ok(permit) = Arc::clone(&self.shadow_slots).try_acquire_owned() else {
            skipped(setting, "shadow_busy");
            return Route::Postgres;
        };
        let (postgres_tx, postgres_rx) = oneshot::channel();
        tokio::spawn(shadow(
            self.index.clone(),
            query.clone(),
            self.shadow_deadline,
            postgres_rx,
            permit,
        ));
        Route::Shadow(postgres_tx)
    }
}

/// Whether the index holds every kind `query` can return: it names kinds,
/// and all of them are indexed. No kinds means every kind.
fn index_holds_kinds(query: &SearchQuery) -> bool {
    query.kinds.as_ref().is_some_and(|kinds| {
        !kinds.is_empty() && kinds.iter().all(|kind| SEARCHABLE_KINDS.contains(kind))
    })
}

/// Run `search` on `engine` as the primary search, recording its metrics.
pub(crate) async fn primary(
    engine: Engine,
    query: &SearchQuery,
    search: impl std::future::Future<Output = Result<SearchResult, SearchError>>,
) -> Result<SearchResult, SearchError> {
    let started = Instant::now();
    let result = search.await;
    let outcome = match &result {
        Ok(result) => hits_outcome(query, result),
        Err(error) if error.is_timeout() => "timeout",
        Err(_) => "error",
    };
    record(engine, "primary", started.elapsed(), outcome);
    result
}

/// The shadow OpenSearch search: run it under its deadline, wait for
/// Postgres' hits (`None` if Postgres failed or the caller went away), then
/// record and log the comparison.
async fn shadow(
    index: SearchIndex,
    query: SearchQuery,
    deadline: Duration,
    postgres: oneshot::Receiver<Option<Vec<[u8; 32]>>>,
    permit: OwnedSemaphorePermit,
) {
    let started = Instant::now();
    let outcome = tokio::time::timeout(deadline, index.search(&query)).await;
    let elapsed = started.elapsed();
    // The slot bounds OpenSearch queries in flight, not Postgres: free it
    // before waiting for Postgres' hits.
    drop(permit);
    let (opensearch_outcome, opensearch_hits, error_kind) = match outcome {
        Ok(Ok(result)) => (hits_outcome(&query, &result), Some(top_ids(&result)), None),
        Ok(Err(error)) if error.is_timeout() => ("timeout", None, Some(error.kind())),
        Ok(Err(error)) => ("error", None, Some(error.kind())),
        Err(_) => ("timeout", None, None),
    };

    let postgres_hits = postgres.await.ok().flatten();
    record(Engine::OpenSearch, "shadow", elapsed, opensearch_outcome);
    let overlap = match (&postgres_hits, &opensearch_hits) {
        (Some(postgres), Some(opensearch)) => overlap(postgres, opensearch),
        _ => None,
    };
    if let Some(overlap) = overlap {
        metrics::histogram!("buzz_search_shadow_overlap_ratio").record(overlap);
    }
    info!(
        community = %query.community,
        page = query.page,
        postgres_hits = postgres_hits.as_ref().map(Vec::len),
        opensearch_hits = opensearch_hits.as_ref().map(Vec::len),
        overlap,
        opensearch_ms = elapsed.as_millis() as u64,
        opensearch_outcome,
        error_kind,
        "search shadow comparison"
    );
}

/// The ids of a result's top [`OVERLAP_TOP`] hits, in order.
pub(crate) fn top_ids(result: &SearchResult) -> Vec<[u8; 32]> {
    result
        .hits
        .iter()
        .take(OVERLAP_TOP)
        .map(|hit| hit.event_id)
        .collect()
}

/// Shared ids over the larger top list; `None` when both are empty.
fn overlap(a: &[[u8; 32]], b: &[[u8; 32]]) -> Option<f64> {
    let larger = a.len().max(b.len());
    if larger == 0 {
        return None;
    }
    let a: HashSet<_> = a.iter().collect();
    let shared = b.iter().filter(|id| a.contains(id)).count();
    Some(shared as f64 / larger as f64)
}

fn hits_outcome(query: &SearchQuery, result: &SearchResult) -> &'static str {
    match (result.hits.is_empty(), query.page <= 1) {
        (false, _) => "hits",
        (true, true) => "zero_results",
        (true, false) => "empty_page",
    }
}

fn record(engine: Engine, role: &'static str, elapsed: Duration, outcome: &'static str) {
    metrics::histogram!(
        "buzz_search_duration_seconds",
        "backend" => engine.label(),
        "role" => role,
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        "buzz_search_requests_total",
        "backend" => engine.label(),
        "role" => role,
        "outcome" => outcome,
    )
    .increment(1);
}

fn skipped(setting: BackendSetting, reason: &'static str) {
    metrics::counter!(
        "buzz_search_opensearch_skipped_total",
        "setting" => setting.label(),
        "reason" => reason,
    )
    .increment(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_values_select_settings_and_unknown_values_mean_postgres() {
        for (value, setting) in [
            (0, BackendSetting::Postgres),
            (1, BackendSetting::Shadow),
            (2, BackendSetting::OpenSearch),
            (3, BackendSetting::Postgres),
            (-1, BackendSetting::Postgres),
        ] {
            assert_eq!(BackendSetting::from_flag_value(value), setting, "{value}");
        }
    }

    #[test]
    fn only_searches_of_indexed_kinds_reach_the_index() {
        let query = |kinds: Option<Vec<i32>>| SearchQuery {
            community: buzz_core::CommunityId::from_uuid(uuid::Uuid::nil()),
            q: "word".into(),
            channel_scope: crate::ChannelScope::Any,
            kinds,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: crate::SearchMode::FullText,
            sort: crate::SearchSort::Relevance,
        };
        for (kinds, indexed) in [
            (Some(vec![9]), true),
            (Some(vec![9, 40002, 40008, 45001, 45003]), true),
            (Some(vec![0]), false),
            (Some(vec![45010]), false),
            (Some(vec![9, 0]), false),
            (Some(vec![]), false),
            (None, false),
        ] {
            assert_eq!(
                index_holds_kinds(&query(kinds.clone())),
                indexed,
                "{kinds:?}"
            );
        }
    }

    #[test]
    fn overlap_compares_the_larger_top_list() {
        let id = |n: u8| [n; 32];
        assert_eq!(overlap(&[], &[]), None);
        assert_eq!(overlap(&[id(1)], &[]), Some(0.0));
        assert_eq!(overlap(&[id(1), id(2)], &[id(2), id(1)]), Some(1.0));
        assert_eq!(
            overlap(&[id(1), id(2), id(3), id(4)], &[id(4), id(9)]),
            Some(0.25)
        );
    }
}
