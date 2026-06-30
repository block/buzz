//! NIP-50 search query against Postgres FTS, community-scoped.
//!
//! The relay never trusts a hit by itself: this layer returns canonical event
//! ids ordered by relevance, the relay refetches `StoredEvent`s through
//! buzz-db's `(community_id, event_id)` scoped fetcher, and runs the access
//! predicate (`search_hit_accepted` in `bridge.rs`) per hit. Search is never
//! the access boundary — it cannot widen visibility.
//!
//! See conformance row 50.

use buzz_core::CommunityId;
use sqlx::{PgPool, QueryBuilder, Row};
use uuid::Uuid;

use crate::error::SearchError;

/// Channel-scope filter for a community-scoped FTS query.
///
/// Four variants, 1-to-1 with the legacy `(accessible_channels: &[Uuid],
/// include_global: bool)` matrix from the Typesense relay:
///
/// | accessible | include_global | `ChannelScope` |
/// |---|---|---|
/// | non-empty  | true  | `ChannelsOrChannelLess(accessible)` |
/// | non-empty  | false | `Channels(accessible)`              |
/// | empty      | true  | `ChannelLessOnly`                   |
/// | empty      | false | (don't call — caller short-circuits to EOSE) |
///
/// `ChannelLessOnly` is the variant that the old `Option<Vec<Uuid>>` +
/// `bool` 2x2 could not express unambiguously: with empty accessible
/// channels and `include_global=true`, both `Some(vec![]) + true` and
/// `None + true` would broaden to all community channels rather than
/// restrict to channel-less events. The enum closes that hole at the
/// type level.
///
/// Empty-vec edge cases are intentionally not special-cased:
/// `Channels(vec![])` emits `channel_id = ANY('{}')` which Postgres
/// evaluates as false-for-all-rows (zero hits), and
/// `ChannelsOrChannelLess(vec![])` emits `(channel_id = ANY('{}') OR
/// channel_id IS NULL)` which is equivalent to `ChannelLessOnly`.
#[derive(Debug, Clone)]
pub enum ChannelScope {
    /// No channel constraint. Matches every event in the community.
    Any,
    /// Restrict to `channel_id IS NULL` events only — what the legacy
    /// Typesense `channel_id:=__global__` sentinel meant.
    ChannelLessOnly,
    /// Restrict to events whose `channel_id` is in this list.
    Channels(Vec<Uuid>),
    /// Restrict to events whose `channel_id` is in this list, OR are
    /// channel-less (`channel_id IS NULL`).
    ChannelsOrChannelLess(Vec<Uuid>),
}

/// Search matching semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Standard NIP-50-ish word/lexeme search using `websearch_to_tsquery`.
    FullText,
    /// Prefix-match the trailing normalized query token (`pro` matches `project`).
    ///
    /// Intended for bounded typeahead surfaces such as the desktop topbar. The
    /// relay still refetches and re-authorizes every hit; this mode changes only
    /// the candidate tsquery, not the access boundary.
    Prefix,
}

/// A community-scoped FTS query.
///
/// The community is REQUIRED at the type level — there is no construction path
/// that omits it. This is the search-side expression of conformance row zero:
/// every search call carries the server-resolved tenant, never client input.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Server-resolved community. Required.
    pub community: CommunityId,
    /// NIP-50 search text. Empty string is rejected by `search()` early
    /// (no hits, no SQL roundtrip).
    pub q: String,
    /// How to scope hits by channel. See [`ChannelScope`] — the four variants
    /// are 1-to-1 with the legacy `(accessible_channels, include_global)`
    /// matrix, and `ChannelLessOnly` closes the gap where "empty accessible
    /// channels + include global" used to silently broaden to all channels.
    pub channel_scope: ChannelScope,
    /// NIP-01 kinds filter. None = no kind constraint.
    pub kinds: Option<Vec<i32>>,
    /// NIP-01 authors filter (32-byte pubkeys). None = no author constraint.
    pub authors: Option<Vec<Vec<u8>>>,
    /// NIP-01 since (Unix seconds). Inclusive lower bound on created_at.
    pub since: Option<i64>,
    /// NIP-01 until (Unix seconds). Inclusive upper bound on created_at.
    pub until: Option<i64>,
    /// 1-indexed page number.
    pub page: u32,
    /// Page size. Clamped at 500 internally.
    pub per_page: u32,
    /// Matching semantics for the search text.
    pub mode: SearchMode,
}

/// A single FTS hit. The relay refetches the canonical `StoredEvent` and
/// re-authorizes; this struct is just enough to drive that fetch and preserve
/// relevance ordering.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// 32-byte event id.
    pub event_id: [u8; 32],
    /// Nostr kind.
    pub kind: i32,
    /// 32-byte pubkey of author.
    pub pubkey: [u8; 32],
    /// Optional channel UUID. `None` = channel-less event.
    pub channel_id: Option<Uuid>,
    /// Unix seconds.
    pub created_at: i64,
    /// `ts_rank_cd` relevance score (higher = better).
    pub rank: f32,
}

/// Result of a search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Hits on this page, ordered by relevance then created_at desc.
    pub hits: Vec<SearchHit>,
    /// 1-indexed page returned.
    pub page: u32,
}

const PER_PAGE_MAX: u32 = 500;
const PER_PAGE_DEFAULT: u32 = 100;
/// Hard cap on search text handed to Postgres text-search parsers. This keeps a
/// single request from spending unbounded parser CPU/memory while still allowing
/// far longer queries than the desktop UI normally emits.
const SEARCH_TEXT_MAX_CHARS: usize = 4096;
/// Search pages are currently server-generated (WS uses 1..=MAX_SEARCH_PAGES,
/// bridge uses page 1), but clamp here too so a future caller cannot accidentally
/// wire untrusted input into a multi-trillion-row OFFSET.
const PAGE_MAX: u32 = 1000;

fn push_tsquery(qb: &mut QueryBuilder<sqlx::Postgres>, mode: SearchMode, search_text: &str) {
    match mode {
        SearchMode::FullText => {
            qb.push("websearch_to_tsquery('simple', ");
            qb.push_bind(search_text);
            qb.push(")");
        }
        SearchMode::Prefix => {
            // Prefix mode is for typeahead: completed whitespace-delimited tokens
            // are exact, and only the trailing token is suffixed with `:*`.
            // Each raw token still goes through Postgres' `simple` parser before
            // tsquery construction so query-side normalization matches the
            // `search_tsv` generated column. `quote_literal` prevents tsquery
            // syntax injection from punctuation/operators in the raw topbar input.
            qb.push(
                "(SELECT COALESCE(\
                 string_agg(\
                   quote_literal(lexeme) || CASE WHEN token_ord = max_token_ord THEN ':*' ELSE '' END, \
                   ' & ' ORDER BY token_ord, lex_ord\
                 ), \
                 ''\
                 )::tsquery \
                 FROM (\
                   SELECT raw_token.token_ord, normalized.lexeme, normalized.lex_ord, raw_token.max_token_ord \
                   FROM (\
                     SELECT token, token_ord, max(token_ord) OVER () AS max_token_ord \
                     FROM regexp_split_to_table(",
            );
            qb.push_bind(search_text);
            qb.push(
                ", '\\s+') WITH ORDINALITY AS split(token, token_ord)\
                   ) AS raw_token \
                   CROSS JOIN LATERAL unnest(tsvector_to_array(to_tsvector('simple', raw_token.token))) \
                     WITH ORDINALITY AS normalized(lexeme, lex_ord)\
                 ) AS prefix_terms)",
            );
        }
    }
}
fn normalized_search_text(q: &str) -> Option<String> {
    let trimmed = q.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut cleaned = String::with_capacity(trimmed.len().min(SEARCH_TEXT_MAX_CHARS));
    for ch in trimmed.chars().take(SEARCH_TEXT_MAX_CHARS) {
        cleaned.push(if ch == '\0' { ' ' } else { ch });
    }

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// Execute a community-scoped FTS query.
///
/// SQL shape (always):
/// ```sql
/// SELECT id, kind, pubkey, channel_id, EXTRACT(EPOCH FROM created_at)::bigint AS created_at_s,
///        ts_rank_cd(search_tsv, query) AS rank
/// FROM events,
///      <mode-specific tsquery> AS query
/// WHERE community_id = $ctx
///   AND deleted_at IS NULL
///   AND search_tsv @@ query
///   [+ channel scope, kinds, authors, since, until]
/// ORDER BY rank DESC, created_at DESC, id
/// LIMIT $per_page OFFSET (($page - 1) * $per_page)
/// ```
///
/// `community_id = $ctx` is the first predicate and is non-negotiable. There
/// is no code path through this function that omits it.
pub async fn search(pool: &PgPool, query: &SearchQuery) -> Result<SearchResult, SearchError> {
    let Some(search_text) = normalized_search_text(&query.q) else {
        return Ok(SearchResult {
            hits: Vec::new(),
            page: query.page.clamp(1, PAGE_MAX),
        });
    };

    let per_page = query.per_page.clamp(1, PER_PAGE_MAX);
    let per_page_actual = if query.per_page == 0 {
        PER_PAGE_DEFAULT
    } else {
        per_page
    };
    let page = query.page.clamp(1, PAGE_MAX);
    let offset = ((page - 1) as i64) * (per_page_actual as i64);

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, kind, pubkey, channel_id, \
         EXTRACT(EPOCH FROM created_at)::bigint AS created_at_s, \
         ts_rank_cd(search_tsv, search_query.query) AS rank \
         FROM events CROSS JOIN LATERAL (SELECT ",
    );
    push_tsquery(&mut qb, query.mode, &search_text);
    qb.push(" AS query) AS search_query WHERE community_id = ");
    qb.push_bind(*query.community.as_uuid());
    qb.push(" AND deleted_at IS NULL AND search_tsv @@ search_query.query");

    // Channel scope — see `ChannelScope` doc for the four-case mapping. The
    // emitted SQL fragments are identical to the legacy 2x2 tuple for the
    // three carry-over cases; `ChannelLessOnly` is the new fence that the
    // old shape could not express.
    match &query.channel_scope {
        ChannelScope::Any => {
            // No channel constraint.
        }
        ChannelScope::ChannelLessOnly => {
            qb.push(" AND channel_id IS NULL");
        }
        ChannelScope::Channels(ids) => {
            qb.push(" AND channel_id = ANY(");
            qb.push_bind(ids.clone());
            qb.push(")");
        }
        ChannelScope::ChannelsOrChannelLess(ids) => {
            qb.push(" AND (channel_id = ANY(");
            qb.push_bind(ids.clone());
            qb.push(") OR channel_id IS NULL)");
        }
    }

    if let Some(ref kinds) = query.kinds {
        if !kinds.is_empty() {
            qb.push(" AND kind = ANY(");
            qb.push_bind(kinds.clone());
            qb.push(")");
        }
    }

    if let Some(ref authors) = query.authors {
        if !authors.is_empty() {
            qb.push(" AND pubkey = ANY(");
            qb.push_bind(authors.clone());
            qb.push(")");
        }
    }

    if let Some(since) = query.since {
        qb.push(" AND created_at >= to_timestamp(");
        qb.push_bind(since);
        qb.push(")");
    }

    if let Some(until) = query.until {
        qb.push(" AND created_at <= to_timestamp(");
        qb.push_bind(until);
        qb.push(")");
    }

    qb.push(" ORDER BY rank DESC, created_at DESC, id LIMIT ");
    qb.push_bind(per_page_actual as i64);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    let rows = qb.build().fetch_all(pool).await?;

    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let id_bytes: Vec<u8> = row.try_get("id")?;
        let pk_bytes: Vec<u8> = row.try_get("pubkey")?;
        let id: [u8; 32] = id_bytes.try_into().map_err(|v: Vec<u8>| {
            sqlx::Error::Decode(format!("event id column is {} bytes, expected 32", v.len()).into())
        })?;
        let pubkey: [u8; 32] = pk_bytes.try_into().map_err(|v: Vec<u8>| {
            sqlx::Error::Decode(format!("pubkey column is {} bytes, expected 32", v.len()).into())
        })?;
        hits.push(SearchHit {
            event_id: id,
            kind: row.try_get("kind")?,
            pubkey,
            channel_id: row.try_get("channel_id")?,
            created_at: row.try_get("created_at_s")?,
            rank: row.try_get("rank")?,
        });
    }

    Ok(SearchResult { hits, page })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_search_text_trims_and_rejects_empty() {
        assert_eq!(
            normalized_search_text("  hello  ").as_deref(),
            Some("hello")
        );
        assert!(normalized_search_text("   ").is_none());
    }

    #[test]
    fn normalized_search_text_replaces_nul_bytes() {
        assert_eq!(
            normalized_search_text("foo\0bar").as_deref(),
            Some("foo bar")
        );
    }

    #[test]
    fn normalized_search_text_caps_length() {
        let long = "x".repeat(SEARCH_TEXT_MAX_CHARS + 10);
        let cleaned = normalized_search_text(&long).expect("non-empty");
        assert_eq!(cleaned.chars().count(), SEARCH_TEXT_MAX_CHARS);
    }
}
