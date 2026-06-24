//! Postgres full-text-search backend.
//!
//! Mirrors the read shape of the Typesense backend: returns event IDs (plus
//! the trivial metadata callers need to fetch the canonical event from
//! Postgres) so the relay's existing
//! `db.get_events_by_ids` → `filters_match` → auth post-filter chain in
//! `crates/buzz-relay/src/handlers/req.rs` is unchanged.
//!
//! Matching uses `plainto_tsquery('simple', $q)` against the `content_tsv`
//! generated column added in migration `0004_search_fts.sql`. Pushdowns:
//!
//! - `kinds`             → `kind = ANY($kinds)`
//! - `authors`           → `pubkey = ANY($authors)`         (hex-decoded)
//! - `channel_ids`       → `channel_id = ANY($chans)`       with optional
//!                          `OR channel_id IS NULL` when the global sentinel
//!                          is present
//! - `since` / `until`   → `created_at >= … AND created_at <= …`
//!
//! Results are ordered by `ts_rank_cd` (cover-density relevance) when the
//! query has searchable text, with `created_at DESC` as a tiebreaker. When
//! the query is empty/`"*"` (no tsquery), ordering falls back to
//! `created_at DESC` only.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::debug;

use crate::error::SearchError;
use crate::query::{SearchHit, SearchQuery, SearchResult, GLOBAL_CHANNEL_SENTINEL};

/// Maximum `per_page` value the Postgres backend will honor in a single
/// query. The relay paginates internally; this is a guardrail against
/// pathological client input rather than a hard NIP-50 limit.
const MAX_PER_PAGE: u32 = 250;

/// Executes a search query against Postgres FTS and returns parsed results.
///
/// Returns `found` as the total number of matching rows (`COUNT(*) OVER ()`
/// window) so the relay's pagination logic can compute exhaustion the same
/// way it does against Typesense.
pub async fn search(pool: &PgPool, query: &SearchQuery) -> Result<SearchResult, SearchError> {
    debug!(
        q = %query.q,
        page = query.page,
        per_page = query.per_page,
        "Executing Postgres FTS search"
    );

    let per_page = query.per_page.min(MAX_PER_PAGE).max(1) as i64;
    let page = query.page.max(1) as i64;
    let offset = (page - 1) * per_page;

    // Decode author hex once; reject any malformed hex by silently dropping it
    // (matches Typesense behavior where an invalid pubkey simply fails to
    // match anything).
    let author_bytes: Vec<Vec<u8>> = query
        .authors
        .iter()
        .filter_map(|hex_str| hex::decode(hex_str.trim()).ok())
        .filter(|b| b.len() == 32)
        .collect();

    // Channel filter splits the sentinel out so we can render
    //   (channel_id = ANY($chans) OR channel_id IS NULL)
    let (channel_uuids, include_global) = split_channel_filter(&query.channel_ids);

    // Build the SQL with literal `ANY` arrays. Using QueryBuilder would be
    // cleaner, but the dynamic shape (optional clauses) is small enough that
    // hand-rolling a query with a stable parameter ordering is more readable
    // and easier to audit.
    //
    // `simple` matches the tokenizer used by the `content_tsv` generated
    // column in migration 0004. plainto_tsquery treats the input as a plain
    // string (handles spaces, ignores punctuation) — closest analogue to
    // Typesense's default `query_by=content` behavior.
    let mut binds = Binds::new();
    let q_trim = query.q.trim();
    let has_text = !q_trim.is_empty() && q_trim != "*";
    // Bind the query text first so the rank expression in the SELECT list and
    // the @@ predicate in WHERE can both reference `$q_idx`. Postgres allows
    // the same parameter slot to appear multiple times in a single statement.
    let q_idx = if has_text {
        Some(binds.push_text(q_trim))
    } else {
        None
    };

    let mut sql = String::from("SELECT id, pubkey, kind, channel_id, created_at, content");
    if let Some(idx) = q_idx {
        // `ts_rank_cd` (cover-density rank) rewards documents where the query
        // terms cluster together — closer match to Typesense's text relevance
        // than plain `ts_rank`. Returned as `rank REAL`.
        sql.push_str(&format!(
            ", ts_rank_cd(content_tsv, plainto_tsquery('simple', ${idx})) AS rank"
        ));
    }
    sql.push_str(", COUNT(*) OVER () AS total FROM events WHERE deleted_at IS NULL");
    if let Some(idx) = q_idx {
        sql.push_str(&format!(
            " AND content_tsv @@ plainto_tsquery('simple', ${idx})"
        ));
    }

    if !query.kinds.is_empty() {
        let kinds_i32: Vec<i32> = query.kinds.iter().map(|k| *k as i32).collect();
        let idx = binds.push_kinds(kinds_i32);
        sql.push_str(&format!(" AND kind = ANY(${idx})"));
    }

    if !author_bytes.is_empty() {
        let idx = binds.push_authors(author_bytes);
        sql.push_str(&format!(" AND pubkey = ANY(${idx})"));
    }

    if !channel_uuids.is_empty() && include_global {
        let idx = binds.push_uuids(channel_uuids);
        sql.push_str(&format!(
            " AND (channel_id = ANY(${idx}) OR channel_id IS NULL)"
        ));
    } else if !channel_uuids.is_empty() {
        let idx = binds.push_uuids(channel_uuids);
        sql.push_str(&format!(" AND channel_id = ANY(${idx})"));
    } else if include_global {
        sql.push_str(" AND channel_id IS NULL");
    }

    if let Some(since) = query.since {
        let idx = binds.push_timestamp(unix_to_timestamptz(since));
        sql.push_str(&format!(" AND created_at >= ${idx}"));
    }
    if let Some(until) = query.until {
        let idx = binds.push_timestamp(unix_to_timestamptz(until));
        sql.push_str(&format!(" AND created_at <= ${idx}"));
    }

    if has_text {
        // ts_rank_cd-then-recency: relevance dominates, recency breaks ties.
        sql.push_str(" ORDER BY rank DESC, created_at DESC");
    } else {
        // No tsquery → no rank column. Fall back to chronological ordering,
        // matching the historical Buzz client expectation.
        sql.push_str(" ORDER BY created_at DESC");
    }
    let limit_idx = binds.push_i64(per_page);
    sql.push_str(&format!(" LIMIT ${limit_idx}"));
    let offset_idx = binds.push_i64(offset);
    sql.push_str(&format!(" OFFSET ${offset_idx}"));

    // The SQL string is built only from static fragments and `$N` parameter
    // placeholders — every dynamic value flows through the bind list. Wrap
    // with `AssertSqlSafe` to satisfy sqlx 0.9's static-SQL lint.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    q = binds.apply(q);
    let rows = q.fetch_all(pool).await?;

    let mut found: u64 = 0;
    let mut hits: Vec<SearchHit> = Vec::with_capacity(rows.len());
    for row in rows {
        let total: i64 = row.try_get("total").unwrap_or(0);
        if found == 0 && total > 0 {
            found = total as u64;
        }

        let id: Vec<u8> = row.try_get("id")?;
        let pubkey: Vec<u8> = row.try_get("pubkey")?;
        let kind_i32: i32 = row.try_get("kind")?;
        let channel_uuid: Option<uuid::Uuid> = row.try_get("channel_id")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let content: String = row.try_get("content")?;

        hits.push(SearchHit {
            event_id: hex::encode(&id),
            content,
            kind: u16::try_from(kind_i32).unwrap_or(0),
            pubkey: hex::encode(&pubkey),
            channel_id: channel_uuid.map(|u| u.to_string()),
            created_at: created_at.timestamp(),
            // `rank` is only in the result set when the query had searchable
            // text; otherwise the SELECT omits the column. ts_rank_cd is
            // typically <1.0 for short docs and grows with match density;
            // we surface it raw, matching Typesense's `text_match` shape.
            // ts_rank_cd returns REAL (f32); widen to the SearchHit::score f64.
            score: if has_text {
                row.try_get::<f32, _>("rank").unwrap_or(0.0) as f64
            } else {
                0.0
            },
        });
    }

    Ok(SearchResult {
        hits,
        found,
        page: query.page,
    })
}

fn split_channel_filter(channel_ids: &[String]) -> (Vec<uuid::Uuid>, bool) {
    let mut include_global = false;
    let mut uuids: Vec<uuid::Uuid> = Vec::with_capacity(channel_ids.len());
    for id in channel_ids {
        if id == GLOBAL_CHANNEL_SENTINEL {
            include_global = true;
        } else if let Ok(u) = uuid::Uuid::parse_str(id) {
            uuids.push(u);
        }
        // silently drop malformed UUID strings — matches Typesense's behavior
        // of treating an unknown channel as a non-match
    }
    (uuids, include_global)
}

fn unix_to_timestamptz(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).unwrap_or_else(|| {
        // Either MIN or MAX, whichever side overflow lands on. Use epoch as
        // a defensive fallback — Buzz events have second-precision created_at
        // bound to NIP-01's u32 range, so this branch is effectively unreachable.
        DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid")
    })
}

/// Bookkeeping for SQL parameter ordering. Each `push_*` records the binding
/// and returns the 1-indexed parameter slot. `apply` re-binds them in order
/// on the final `sqlx::query` so the index labels in the SQL string line up.
struct Binds {
    items: Vec<BoundValue>,
}

enum BoundValue {
    Text(String),
    KindList(Vec<i32>),
    AuthorList(Vec<Vec<u8>>),
    UuidList(Vec<uuid::Uuid>),
    Timestamp(DateTime<Utc>),
    Int8(i64),
}

impl Binds {
    fn new() -> Self {
        Self { items: Vec::new() }
    }
    fn next_idx(&self) -> usize {
        self.items.len() + 1
    }
    fn push_text(&mut self, s: &str) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::Text(s.to_string()));
        i
    }
    fn push_kinds(&mut self, v: Vec<i32>) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::KindList(v));
        i
    }
    fn push_authors(&mut self, v: Vec<Vec<u8>>) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::AuthorList(v));
        i
    }
    fn push_uuids(&mut self, v: Vec<uuid::Uuid>) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::UuidList(v));
        i
    }
    fn push_timestamp(&mut self, t: DateTime<Utc>) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::Timestamp(t));
        i
    }
    fn push_i64(&mut self, n: i64) -> usize {
        let i = self.next_idx();
        self.items.push(BoundValue::Int8(n));
        i
    }
    fn apply<'q>(
        self,
        mut q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
        for item in self.items {
            q = match item {
                BoundValue::Text(s) => q.bind(s),
                BoundValue::KindList(v) => q.bind(v),
                BoundValue::AuthorList(v) => q.bind(v),
                BoundValue::UuidList(v) => q.bind(v),
                BoundValue::Timestamp(t) => q.bind(t),
                BoundValue::Int8(n) => q.bind(n),
            };
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_channel_filter_extracts_global_sentinel() {
        let (uuids, global) = split_channel_filter(&[
            "11111111-1111-1111-1111-111111111111".into(),
            GLOBAL_CHANNEL_SENTINEL.to_string(),
            "not-a-uuid".into(),
            "22222222-2222-2222-2222-222222222222".into(),
        ]);
        assert!(global, "sentinel should be detected");
        assert_eq!(uuids.len(), 2, "malformed UUID should be dropped silently");
    }

    #[test]
    fn split_channel_filter_no_sentinel_means_no_global() {
        let (uuids, global) =
            split_channel_filter(&["11111111-1111-1111-1111-111111111111".into()]);
        assert!(!global);
        assert_eq!(uuids.len(), 1);
    }

    #[test]
    fn unix_to_timestamptz_round_trips() {
        let ts = unix_to_timestamptz(1_700_000_000);
        assert_eq!(ts.timestamp(), 1_700_000_000);
    }
}
