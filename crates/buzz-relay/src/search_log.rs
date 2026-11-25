//! One stdout line per message search, so search quality can be compared
//! across backends: how often a search returns nothing, and how often the
//! same searcher searches again moments later.
//!
//! Every line goes to the [`TARGET`] tracing target with `event="search"`
//! and a fresh `search_id`. It records who searched (`searcher`,
//! `community`), on which `backend`, how (`transport`, `mode`, `sort`,
//! `page`, `limit` and the kind, author, time and channel filters), what came
//! back (`result_count` and, in order, the first [`HIT_IDS_MAX`] `hit_ids`
//! the searcher received, after access checks; `backend_hits` counts what the
//! backend returned before them) and the query's shape (`word_count`,
//! `quoted`, `has_url`). Never its text.
//!
//! With `BUZZ_SEARCH_LOG_QUERY_TEXT_RETENTION_DAYS` set, each search also
//! writes its text to [`TEXT_TARGET`] as `event="search_text"`, with the same
//! `search_id` and the setting as `retention_days`, so a deployment's log
//! pipeline can keep it apart from other logs and expire it. The relay does
//! not expire what it writes to stdout.
//!
//! `buzz_relay::search=off` in `RUST_LOG` turns off both lines. Lines are
//! built only when the target is on, and written after the search finishes,
//! so logging never fails or delays a search. See `docs/search-logging.md`.

use buzz_core::CommunityId;
use buzz_search::{ChannelScope, SearchMode, SearchQuery, SearchSort};
use uuid::Uuid;

/// The tracing target for search lines.
pub(crate) const TARGET: &str = "buzz_relay::search";

/// The tracing target for search text, under [`TARGET`] so turning that off
/// turns this off too.
pub(crate) const TEXT_TARGET: &str = "buzz_relay::search::text";

/// Most returned event ids a line lists; `result_count` counts them all.
const HIT_IDS_MAX: usize = 100;

/// Most kinds a line lists; `kind_count` counts them all.
const KINDS_MAX: usize = 10;

/// Most characters of search text a text line holds.
const QUERY_TEXT_MAX_CHARS: usize = 1024;

/// How a search ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Dropped before the searcher got its results: the client went away,
    /// a newer search replaced it (HTTP typeahead superseded, or a WebSocket
    /// subscription replaced or closed), or the request was cancelled.
    Aborted,
    Served,
    Error(&'static str),
}

/// What a search asked for and got, logged when it is dropped. One per
/// search filter the relay sends to the backend, however many backend pages
/// that filter reads.
pub(crate) struct SearchLog(Option<Box<Line>>);

struct Line {
    search_id: Uuid,
    transport: &'static str,
    community: CommunityId,
    searcher: String,
    backend: &'static str,
    mode: &'static str,
    sort: &'static str,
    page: u32,
    limit: u32,
    kinds: Option<String>,
    kind_count: usize,
    author_count: usize,
    since: Option<i64>,
    until: Option<i64>,
    channel_scope: &'static str,
    channel_count: usize,
    shape: QueryShape,
    /// The search text and its retention, when the deployment logs text.
    text: Option<(String, u32)>,
    outcome: Outcome,
    backend_hits: usize,
    result_count: usize,
    hit_ids: Vec<[u8; 32]>,
    started: std::time::Instant,
    /// Set by [`SearchLog::stop_clock`]; otherwise the line times to its drop.
    duration: Option<std::time::Duration>,
}

impl SearchLog {
    /// Start the line for `query`, sent over `transport` (`"ws"` or
    /// `"http"`) by `searcher` (a 32-byte pubkey) asking for `limit` results.
    pub(crate) fn start(
        transport: &'static str,
        state: &crate::state::AppState,
        searcher: &[u8],
        query: &SearchQuery,
        limit: u32,
    ) -> Self {
        if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
            return Self(None);
        }
        let kinds = query.kinds.as_deref().unwrap_or_default();
        let (channel_scope, channel_count) = match &query.channel_scope {
            ChannelScope::Any => ("any", 0),
            ChannelScope::ChannelLessOnly => ("channel_less", 0),
            ChannelScope::Channels(ids) => ("channels", ids.len()),
            ChannelScope::ChannelsOrChannelLess(ids) => ("channels_or_channel_less", ids.len()),
        };
        let text = state
            .config
            .search_log_query_text_retention_days
            .filter(|_| tracing::enabled!(target: TEXT_TARGET, tracing::Level::INFO))
            .map(|days| (query.q.chars().take(QUERY_TEXT_MAX_CHARS).collect(), days));
        Self(Some(Box::new(Line {
            search_id: Uuid::new_v4(),
            transport,
            community: query.community,
            searcher: hex::encode(searcher),
            backend: state.search.backend_name(),
            mode: match query.mode {
                SearchMode::FullText => "full_text",
                SearchMode::Prefix => "prefix",
            },
            sort: match query.sort {
                SearchSort::Relevance => "relevance",
                SearchSort::Newest => "newest",
            },
            page: query.page,
            limit,
            kinds: (!kinds.is_empty()).then(|| {
                kinds
                    .iter()
                    .take(KINDS_MAX)
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            }),
            kind_count: kinds.len(),
            author_count: query.authors.as_ref().map_or(0, Vec::len),
            since: query.since,
            until: query.until,
            channel_scope,
            channel_count,
            shape: QueryShape::of(&query.q),
            text,
            outcome: Outcome::Aborted,
            backend_hits: 0,
            result_count: 0,
            hit_ids: Vec::new(),
            started: std::time::Instant::now(),
            duration: None,
        })))
    }

    /// The backend answered one page with `hits` hits.
    pub(crate) fn backend_hits(&mut self, hits: usize) {
        if let Some(line) = &mut self.0 {
            line.backend_hits += hits;
        }
    }

    /// The searcher was sent the event `id`, the next result in order.
    pub(crate) fn returned(&mut self, id: [u8; 32]) {
        if let Some(line) = &mut self.0 {
            line.result_count += 1;
            if line.hit_ids.len() < HIT_IDS_MAX {
                line.hit_ids.push(id);
            }
        }
    }

    /// This search's own work is done, though its outcome waits on the rest
    /// of the request: `duration_ms` stops here.
    pub(crate) fn stop_clock(&mut self) {
        if let Some(line) = &mut self.0 {
            line.duration.get_or_insert_with(|| line.started.elapsed());
        }
    }

    /// Every result was sent.
    pub(crate) fn served(&mut self) {
        if let Some(line) = &mut self.0 {
            line.outcome = Outcome::Served;
        }
    }

    /// The search ended in a server-side failure, a fixed `reason` code.
    pub(crate) fn error(&mut self, reason: &'static str) {
        if let Some(line) = &mut self.0 {
            line.outcome = Outcome::Error(reason);
        }
    }
}

impl Drop for SearchLog {
    fn drop(&mut self) {
        let Some(line) = self.0.take() else {
            return;
        };
        let (outcome, reason) = match line.outcome {
            Outcome::Aborted => ("aborted", None),
            Outcome::Served => ("served", None),
            Outcome::Error(reason) => ("error", Some(reason)),
        };
        let hit_ids = line
            .hit_ids
            .iter()
            .map(hex::encode)
            .collect::<Vec<_>>()
            .join(",");
        tracing::info!(
            target: TARGET,
            event = "search",
            search_id = %line.search_id,
            transport = line.transport,
            community = %line.community,
            searcher = line.searcher.as_str(),
            backend = line.backend,
            mode = line.mode,
            sort = line.sort,
            page = line.page,
            limit = line.limit,
            kinds = line.kinds.as_deref(),
            kind_count = line.kind_count,
            author_count = line.author_count,
            since = line.since,
            until = line.until,
            channel_scope = line.channel_scope,
            channel_count = line.channel_count,
            word_count = line.shape.word_count,
            quoted = line.shape.quoted,
            has_url = line.shape.has_url,
            outcome,
            reason,
            backend_hits = line.backend_hits,
            result_count = line.result_count,
            hit_ids = hit_ids.as_str(),
            duration_ms = line
                .duration
                .unwrap_or_else(|| line.started.elapsed())
                .as_millis() as u64,
            "search"
        );
        if let Some((query, retention_days)) = &line.text {
            tracing::info!(
                target: TEXT_TARGET,
                event = "search_text",
                search_id = %line.search_id,
                retention_days,
                query = query.as_str(),
                "search text"
            );
        }
    }
}

/// What a search text looks like, without what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueryShape {
    /// Whitespace-separated words.
    word_count: usize,
    /// Whether the text has a `"`, which full-text mode reads as a phrase.
    quoted: bool,
    /// Whether a word looks like a URL: it has `://` or starts with `www.`.
    has_url: bool,
}

impl QueryShape {
    fn of(q: &str) -> Self {
        let mut shape = Self {
            word_count: 0,
            quoted: q.contains('"'),
            has_url: false,
        };
        for word in q.split_whitespace() {
            shape.word_count += 1;
            shape.has_url |= word.contains("://")
                || word
                    .get(..4)
                    .is_some_and(|start| start.eq_ignore_ascii_case("www."));
        }
        shape
    }
}

/// Fixtures for the handler tests that check search lines.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    /// The lines on `target` captured so far.
    pub(crate) fn lines(
        out: &crate::access::tests::Captured,
        target: &str,
    ) -> Vec<serde_json::Value> {
        out.raw()
            .lines()
            .map(|l| serde_json::from_str(l).expect("JSON log line"))
            .filter(|v: &serde_json::Value| v["target"] == target)
            .collect()
    }

    /// A Postgres-backed relay with one community and channel holding two
    /// messages that match `needle` and one that does not.
    pub(crate) struct Seeded {
        pub(crate) state: Arc<crate::state::AppState>,
        pub(crate) tenant: buzz_core::tenant::TenantContext,
        pub(crate) channel: uuid::Uuid,
        pub(crate) reader: Keys,
    }

    impl Seeded {
        pub(crate) async fn new() -> Self {
            let state = crate::state::tests::test_state_with_database_url_and_pool_options(
                &crate::test_support::database_url(),
                sqlx::postgres::PgPoolOptions::new(),
            )
            .await;
            let pool = state.db.pool().clone();
            let community = uuid::Uuid::new_v4();
            let host = format!("search-log-{}.example", community.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community)
                .bind(&host)
                .execute(&pool)
                .await
                .expect("insert community");
            let channel = uuid::Uuid::new_v4();
            sqlx::query(
                "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1, $2, $3, $4)",
            )
            .bind(channel)
            .bind(community)
            .bind(format!("search-log-{}", channel.simple()))
            .bind(vec![7_u8; 32])
            .execute(&pool)
            .await
            .expect("insert channel");
            let community = buzz_core::CommunityId::from_uuid(community);
            let author = Keys::generate();
            let h = Tag::parse(["h", &channel.to_string()]).expect("h tag");
            let now = Timestamp::now().as_secs();
            for (content, age) in [("needle one", 30), ("needle two", 20), ("haystack", 10)] {
                let event = EventBuilder::new(Kind::Custom(9), content)
                    .tags([h.clone()])
                    .custom_created_at(Timestamp::from_secs(now - age))
                    .sign_with_keys(&author)
                    .expect("sign");
                state
                    .db
                    .insert_event(community, &event, Some(channel))
                    .await
                    .expect("insert event");
            }
            Self {
                state,
                tenant: buzz_core::tenant::TenantContext::resolved(community, host),
                channel,
                reader: Keys::generate(),
            }
        }

        /// The same relay, logging search text kept for `days`.
        pub(crate) fn with_text_retention(&self, days: u32) -> Arc<crate::state::AppState> {
            let mut state = (*self.state).clone();
            Arc::make_mut(&mut state.config).search_log_query_text_retention_days = Some(days);
            Arc::new(state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_shape_counts_words_quotes_and_urls() {
        for (q, word_count, quoted, has_url) in [
            ("", 0, false, false),
            ("   ", 0, false, false),
            ("deploy", 1, false, false),
            ("  deploy   failed\tagain ", 3, false, false),
            (r#""release notes" q3"#, 3, true, false),
            ("see https://example.org/a", 2, false, true),
            ("WWW.example.org", 1, false, true),
            ("git+ssh://example.org", 1, false, true),
            ("www", 1, false, false),
            ("example.org", 1, false, false),
            ("señor www.", 2, false, true),
        ] {
            assert_eq!(
                QueryShape::of(q),
                QueryShape {
                    word_count,
                    quoted,
                    has_url
                },
                "{q:?}"
            );
        }
    }
}
