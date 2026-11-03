//! NIP-50 search query against Postgres FTS, community-scoped.
//!
//! The relay never trusts a hit by itself: this layer returns canonical event
//! ids ordered by relevance, the relay refetches `StoredEvent`s through
//! buzz-db's `(community_id, event_id)` scoped fetcher, and runs the access
//! predicate (`search_hit_accepted` in `bridge.rs`) per hit. Search is never
//! the access boundary — it cannot widen visibility.
//!
//! See conformance row 50.

use sqlx::{PgPool, QueryBuilder, Row};
use uuid::Uuid;

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;

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
    /// relay still refetches and re-authorizes every hit; this mode changes the
    /// candidate tsquery and, for relevance outside profile search, ranks only
    /// the newest 1000 matches. It does not change the access boundary. A
    /// trailing token that yields a lexeme shorter than three characters
    /// returns no hits unless the query is kind:0-only (see `PREFIX_MIN_CHARS`).
    Prefix,
}

/// Result ordering for a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSort {
    /// `ts_rank_cd` relevance, newest first among equal ranks. In
    /// [`SearchMode::Prefix`] outside profile search, only the newest 1000
    /// matches are ranked, so pages end with them.
    Relevance,
    /// Newest first, ignoring relevance; ties ordered by event id.
    Newest,
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
    /// Result ordering. Paging is over this order.
    pub sort: SearchSort,
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
    /// Hits on this page, in the query's `sort` order.
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
/// How many of the newest matches a prefix-mode relevance search ranks.
/// The GIN index returns matches in no useful order, so ranking every match
/// before `LIMIT` costs time proportional to the match count, and a short
/// typeahead prefix matches most of a community. Reading candidates newest
/// first lets Postgres stop at this many instead. Every page ranks the same
/// window, so pages stay consistent and end with it.
const PREFIX_RANK_WINDOW: i64 = 1000;
/// Shortest lexeme [`SearchMode::Prefix`] gives `:*` for anything but a
/// kind:0-only (profile) query. If the trailing token yields a shorter one,
/// the tsquery is NULL and the search returns no hits, whatever completed
/// words precede it. The check runs on Postgres' own `simple` parser output,
/// so punctuation such as `c++`, `@al` or `don't` cannot slip a short `:*`
/// past it.
/// The GIN index cannot stop early or return rows in rank order, so a one- or
/// two-character `:*` prefix makes Postgres fetch, recheck and rank a large
/// share of every indexed message before it can apply `LIMIT`, and typeahead
/// sends one on every typing pause. Profiles are few enough to stay
/// one-character typeahead.
const PREFIX_MIN_CHARS: i32 = 3;

/// `min_prefix_chars` applies to [`SearchMode::Prefix`] only; 0 disables it.
fn push_tsquery(
    qb: &mut QueryBuilder<sqlx::Postgres>,
    mode: SearchMode,
    search_text: &str,
    min_prefix_chars: i32,
) {
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
            // A trailing lexeme shorter than `min_prefix_chars` makes the whole
            // tsquery NULL, which matches nothing (see `PREFIX_MIN_CHARS`).
            qb.push(
                "(SELECT CASE WHEN bool_or(token_ord = max_token_ord AND char_length(lexeme) < ",
            );
            qb.push_bind(min_prefix_chars);
            qb.push(
                ") THEN NULL::tsquery ELSE COALESCE(\
                 string_agg(\
                   quote_literal(lexeme) || CASE WHEN token_ord = max_token_ord THEN ':*' ELSE '' END, \
                   ' & ' ORDER BY token_ord, lex_ord\
                 ), \
                 ''\
                 )::tsquery END \
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
/// The mode-specific tsquery is resolved first and bound as a value, so the
/// planner estimates its selectivity instead of assuming a rare match. The
/// search itself is a one-shot statement, planned for that value every time.
///
/// SQL shape:
/// ```sql
/// SELECT id, kind, pubkey, channel_id, EXTRACT(EPOCH FROM created_at)::bigint AS created_at_s,
///        ts_rank_cd(search_tsv, $q) AS rank
/// FROM events            -- prefix relevance: the newest PREFIX_RANK_WINDOW matches
/// WHERE community_id = $ctx
///   AND deleted_at IS NULL
///   AND search_tsv @@ $q
///   [+ channel scope, kinds, authors, since, until]
/// ORDER BY rank DESC, created_at DESC, id   -- SearchSort::Newest: created_at DESC, id
/// LIMIT $per_page OFFSET (($page - 1) * $per_page)
/// ```
///
/// `community_id = $ctx` is the first predicate and is non-negotiable. There
/// is no code path through this function that omits it.
#[datastore_span(name = "search", system = "postgresql")]
pub async fn search(pool: &PgPool, query: &SearchQuery) -> Result<SearchResult, SearchError> {
    let profile_only = query.kinds.as_deref() == Some(&[0][..]);
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
    // Profile typeahead uses broad prefix matching. For one- and two-character
    // queries, a busy community can have enough newer prefix matches to push a
    // short exact display name off the bounded first page. Keep the same result
    // set and pagination contract, but put rows containing the whole lexeme
    // first for this one narrow caller shape.
    let prioritize_exact_profile_lexeme = query.sort == SearchSort::Relevance
        && query.mode == SearchMode::Prefix
        && profile_only
        && search_text.chars().count() <= 2;
    // Profiles are few, so profile typeahead keeps ranking every match.
    let rank_window =
        (query.sort == SearchSort::Relevance && query.mode == SearchMode::Prefix && !profile_only)
            .then_some(PREFIX_RANK_WINDOW);

    let min_prefix_chars = if profile_only { 0 } else { PREFIX_MIN_CHARS };
    // One checkout for both statements, so a busy pool queues a search once.
    let mut conn = pool.acquire().await?;
    let mut resolve: QueryBuilder<sqlx::Postgres> = QueryBuilder::new("SELECT (");
    push_tsquery(&mut resolve, query.mode, &search_text, min_prefix_chars);
    resolve.push(")::text");
    let tsquery: Option<String> = resolve.build_query_scalar().fetch_one(&mut *conn).await?;
    let Some(tsquery) = tsquery.filter(|tsquery| !tsquery.is_empty()) else {
        // A too-short trailing prefix (NULL) or nothing searchable after
        // normalization (empty) matches nothing.
        return Ok(SearchResult {
            hits: Vec::new(),
            page,
        });
    };

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, kind, pubkey, channel_id, \
         EXTRACT(EPOCH FROM created_at)::bigint AS created_at_s, \
         ts_rank_cd(search_tsv, ",
    );
    qb.push_bind(&tsquery);
    qb.push("::tsquery) AS rank FROM ");
    if rank_window.is_some() {
        qb.push("(SELECT id, kind, pubkey, channel_id, created_at, search_tsv FROM ");
    }
    qb.push("events WHERE community_id = ");
    qb.push_bind(*query.community.as_uuid());
    qb.push(" AND deleted_at IS NULL AND search_tsv @@ ");
    qb.push_bind(&tsquery);
    qb.push("::tsquery");

    // Search is a current-state surface; old artifact snapshots cannot match.
    qb.push(" AND (events.kind <> 45010 OR EXISTS (SELECT 1 FROM artifact_heads ah WHERE ah.community_id=events.community_id AND ah.event_id=events.id AND NOT ah.deleted))");

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

    if let Some(window) = rank_window {
        // Matches the backward scan of each partition's primary key, so the
        // newest candidates stream in order and the scan stops at `window`.
        qb.push(" ORDER BY created_at DESC, id DESC LIMIT ");
        qb.push_bind(window);
        qb.push(") AS candidates");
    }

    if query.sort == SearchSort::Newest {
        qb.push(" ORDER BY created_at DESC, id LIMIT ");
    } else if prioritize_exact_profile_lexeme {
        qb.push(" ORDER BY search_tsv @@ websearch_to_tsquery('simple', ");
        qb.push_bind(&search_text);
        qb.push(") DESC, rank DESC, created_at DESC, id LIMIT ");
    } else {
        qb.push(" ORDER BY rank DESC, created_at DESC, id LIMIT ");
    }
    qb.push_bind(per_page_actual as i64);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    // Not a cached prepared statement: after five runs Postgres may switch a
    // cached statement to a generic plan, which cannot see the tsquery.
    let rows = crate::cancel::fetch_all(pool, conn, qb.build().persistent(false)).await?;

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

/// The SQL that normalizes a name, and the query, for result tiers, around
/// its argument: lower-cased, whitespace runs collapsed to one space, trimmed.
const TIER_TEXT: [&str; 2] = ["btrim(regexp_replace(lower(", "), '\\s+', ' ', 'g'))"];

fn tier_name(column: &str) -> String {
    format!("{}{column}{}", TIER_TEXT[0], TIER_TEXT[1])
}

/// Largest page size [`search_profile_names`] accepts.
pub const PROFILE_NAMES_PER_PAGE_MAX: u32 = 100;

/// Find current kind:0 profiles whose `name` or `display_name` matches `q`.
///
/// Matching is the NIP-50 [`SearchMode::Prefix`] rule (every completed query
/// word must equal a word, the last may be a prefix, in any order), applied to
/// the two name fields only. Other profile fields, such as `picture` data URLs,
/// `about` and `nip05`, cannot satisfy a word. The result set is therefore the
/// prefix-mode kind:0 result set minus profiles that match only outside their
/// names; the existing `search_tsv` index stays the access path.
///
/// Order: a name equal to the query, then a name that starts with it, then a
/// name with a word that starts with it, then any other match. The query and
/// names are compared lower-cased with whitespace collapsed, both normalized
/// by the same SQL. Ties go by relevance of the query to the two names
/// (`ts_rank_cd` over the names alone, not the whole profile), then newest
/// first, then event id, so pages are stable. Pages are 1-based, like
/// [`SearchQuery::page`].
#[datastore_span(name = "search_profile_names", system = "postgresql")]
pub async fn search_profile_names(
    pool: &PgPool,
    community: CommunityId,
    q: &str,
    page: u32,
    per_page: u32,
) -> Result<SearchResult, SearchError> {
    let page = page.clamp(1, PAGE_MAX);
    let Some(search_text) = normalized_search_text(q) else {
        return Ok(SearchResult {
            hits: Vec::new(),
            page,
        });
    };
    let per_page = per_page.clamp(1, PROFILE_NAMES_PER_PAGE_MAX);
    let offset = ((page - 1) as i64) * (per_page as i64);
    // Tiers compare the query and each name after the same SQL normalization
    // ([`TIER_TEXT`]), so the two sides agree by construction.
    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
        "SELECT id, kind, pubkey, channel_id, created_at_s, rank FROM (\
         SELECT e.id, e.kind, e.pubkey, e.channel_id, \
         EXTRACT(EPOCH FROM e.created_at)::bigint AS created_at_s, e.created_at, \
         ts_rank_cd(names.tsv, search_query.query) AS rank, \
         LEAST(",
    );
    for field in ["names.name", "names.display_name"] {
        qb.push(format!(
            "CASE WHEN {field} = search_query.typed THEN 0 \
             WHEN starts_with({field}, search_query.typed) THEN 1 \
             WHEN strpos({field}, ' ' || search_query.typed) > 0 THEN 2 ELSE 3 END"
        ));
        if field == "names.name" {
            qb.push(", ");
        }
    }
    qb.push(
        ") AS tier \
         FROM events e CROSS JOIN LATERAL (SELECT ",
    );
    push_tsquery(&mut qb, SearchMode::Prefix, &search_text, 0);
    qb.push(" AS query, ");
    qb.push(TIER_TEXT[0]);
    qb.push_bind(search_text.clone());
    qb.push(TIER_TEXT[1]);
    qb.push(format!(
        " AS typed) AS search_query \
         CROSS JOIN LATERAL profile_name_fields(e.content) AS fields \
         CROSS JOIN LATERAL (SELECT \
           to_tsvector('simple', concat_ws(' ', fields.name, fields.display_name)) AS tsv, \
           {} AS name, {} AS display_name) AS names \
         WHERE e.community_id = ",
        tier_name("fields.name"),
        tier_name("fields.display_name"),
    ));
    qb.push_bind(*community.as_uuid());
    qb.push(
        " AND e.kind = 0 AND e.channel_id IS NULL AND e.deleted_at IS NULL \
         AND e.search_tsv @@ search_query.query \
         AND names.tsv @@ search_query.query\
         ) AS matched ORDER BY tier, rank DESC, created_at DESC, id LIMIT ",
    );
    qb.push_bind(per_page as i64);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    let rows = crate::cancel::fetch_all(pool, pool.acquire().await?, qb.build()).await?;
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
