//! Search behavior every [`SearchService`] backend must share.
//!
//! Each case is a function taking a [`Backend`]; a test binary turns the
//! cases it runs into tests with [`search_suite_tests!`]. The cases write
//! fixture rows straight into Postgres (queuing each event in the search
//! outbox), then search through [`SearchService::search`], so they check
//! search behavior, not the write path. On OpenSearch every search first
//! drains the outbox and refreshes the index.
//!
//! Run through `scripts/postgres-test-run.sh -p buzz-search --tests` for a
//! fresh desired-state database per test. Outside that lane, point
//! BUZZ_TEST_DATABASE_URL at a disposable dedicated database; fixtures use
//! the production migrator and unique community IDs.

// Each test binary uses its own subset of the cases and helpers.
#![allow(dead_code)]

use buzz_core::{
    kind::{
        AUTHOR_ONLY_KINDS, KIND_AGENT_TURN_METRIC, KIND_MEMBER_ADDED_NOTIFICATION,
        KIND_MEMBER_REMOVED_NOTIFICATION, P_GATED_KINDS,
    },
    CommunityId,
};
use buzz_search::indexer::Indexer;
use buzz_search::opensearch::{OpenSearchConfig, SearchIndex};
use buzz_search::{ChannelScope, SearchError, SearchQuery, SearchResult, SearchService};
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

/// Turns each `fn case` into a test against `$backend`. The attribute after
/// the backend goes on every test, so each binary states what its tests need.
macro_rules! search_suite_tests {
    ($backend:expr; #[$meta:meta] $(fn $case:ident),+ $(,)?) => {
        $(
            #[tokio::test]
            #[$meta]
            async fn $case() {
                search_suite::$case($backend).await;
            }
        )+
    };
}
pub(crate) use search_suite_tests;

/// The engine a case searches with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Postgres full-text search.
    Postgres,
    /// The OpenSearch index at `BUZZ_TEST_OPENSEARCH_URL`, kept in sync by
    /// the indexer.
    OpenSearch,
}

/// A test's search service, for the backend under test.
pub struct Fixture {
    service: SearchService,
    opensearch: Option<(Indexer, SearchIndex)>,
}

impl Fixture {
    /// Connect to the test database and build the backend's search service.
    /// For OpenSearch, that includes a new index of the test's own.
    pub async fn setup(backend: Backend) -> (PgPool, Fixture) {
        let (pool, _schema) = setup().await;
        let fixture = match backend {
            Backend::Postgres => Fixture {
                service: SearchService::new(pool.clone()),
                opensearch: None,
            },
            Backend::OpenSearch => {
                let index = test_index().await;
                Fixture {
                    service: SearchService::with_opensearch(pool.clone(), index.clone()),
                    opensearch: Some((
                        Indexer::new(buzz_db::Db::from_pool(pool.clone()), index.clone()),
                        index,
                    )),
                }
            }
        };
        (pool, fixture)
    }

    /// The search service under test.
    pub fn service(&self) -> Searcher<'_> {
        Searcher { fixture: self }
    }

    /// Release the fixture's resources.
    pub async fn teardown(self, pool: PgPool) {
        if let Some((_, index)) = self.opensearch {
            index.delete().await.expect("delete test index");
        }
        teardown(pool, "").await;
    }
}

/// [`SearchService`] for a case: each search first lets the backend catch up
/// with the rows the case wrote.
#[derive(Clone, Copy)]
pub struct Searcher<'a> {
    fixture: &'a Fixture,
}

impl Searcher<'_> {
    /// [`SearchService::search`], once the backend has caught up.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        if let Some((indexer, index)) = &self.fixture.opensearch {
            indexer.drain().await.expect("drain the search outbox");
            index.refresh().await.expect("refresh the test index");
        }
        self.fixture.service.search(query).await
    }
}

/// A new, empty message index of the calling test's own on the test cluster.
pub async fn test_index() -> SearchIndex {
    let index = SearchIndex::new(&OpenSearchConfig {
        url: std::env::var("BUZZ_TEST_OPENSEARCH_URL")
            .unwrap_or_else(|_| "http://localhost:9200".to_string()),
        basic_auth: None,
        aws_sigv4: None,
        index_prefix: format!("test-{}-", Uuid::new_v4().simple()),
    })
    .expect("OpenSearch client");
    index.ensure().await.expect("create test index");
    index
}

pub const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials
pub async fn setup() -> (PgPool, String) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    // The PostgreSQL lane supplies an isolated desired-state database. Outside
    // that lane use a dedicated database with the production migrations.
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect test database");
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("apply production migrations");
    }
    (pool, String::new())
}

pub async fn teardown(pool: PgPool, _schema: &str) {
    pool.close().await;
    // The test runner owns database cleanup.
}

/// Insert a community row, return its id.
pub async fn mk_community(pool: &PgPool, host: &str) -> CommunityId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host, signing_key) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(host)
        .bind(b"signingkey".as_slice())
        .execute(pool)
        .await
        .expect("insert community");
    CommunityId::from_uuid(id)
}

/// Insert a minimal event, and queue it in the search outbox as admission
/// would. `created_at_secs` is unix seconds.
#[allow(clippy::too_many_arguments)]
pub async fn insert_event(
    pool: &PgPool,
    community: CommunityId,
    id: [u8; 32],
    pubkey: [u8; 32],
    kind: i32,
    content: &str,
    channel_id: Option<Uuid>,
    created_at_secs: i64,
) {
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
         VALUES ($1, $2, $3, to_timestamp($4), $5, '[]'::jsonb, $6, $7, $8)",
    )
    .bind(community.as_uuid())
    .bind(&id[..])
    .bind(&pubkey[..])
    .bind(created_at_secs)
    .bind(kind)
    .bind(content)
    .bind(b"signature".as_slice())
    .bind(channel_id)
    .execute(pool)
    .await
    .expect("insert event");
    // Postgres search ignores the outbox; the indexer skips kinds that are
    // not searchable.
    sqlx::query(
        "INSERT INTO search_outbox (community_id, event_id, event_created_at) \
         VALUES ($1, $2, to_timestamp($3))",
    )
    .bind(community.as_uuid())
    .bind(&id[..])
    .bind(created_at_secs)
    .execute(pool)
    .await
    .expect("queue event for indexing");
}

pub fn rand_bytes32() -> [u8; 32] {
    let mut out = [0u8; 32];
    let u = Uuid::new_v4();
    let bytes = u.as_bytes();
    out[..16].copy_from_slice(bytes);
    out[16..].copy_from_slice(bytes);
    out
}

pub async fn search_finds_event_in_same_community(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c_a = mk_community(&pool, "a.example").await;
    let evt_id = rand_bytes32();
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c_a,
        evt_id,
        pk,
        9,
        "hello wonderland — buzz everyone",
        None,
        1700000000,
    )
    .await;

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c_a,
            q: "wonderland".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, evt_id);
    assert_eq!(result.hits[0].kind, 9);
    assert_eq!(result.hits[0].created_at, 1700000000);
    assert!(result.hits[0].rank > 0.0);

    fx.teardown(pool).await;
}

pub async fn search_does_not_return_other_community_events(backend: Backend) {
    // The load-bearing test: event indexed under community A, query bound to
    // community B → zero hits.
    let (pool, fx) = Fixture::setup(backend).await;

    let c_a = mk_community(&pool, "a.example").await;
    let c_b = mk_community(&pool, "b.example").await;
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c_a,
        rand_bytes32(),
        pk,
        9,
        "only-in-a unique-token-xyz",
        None,
        1700000000,
    )
    .await;

    let svc = fx.service();
    let result_a = svc
        .search(&SearchQuery {
            community: c_a,
            q: "unique-token-xyz".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(result_a.hits.len(), 1, "A should see its own event");

    let result_b = svc
        .search(&SearchQuery {
            community: c_b,
            q: "unique-token-xyz".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(result_b.hits.len(), 0, "B must not see A's event");

    fx.teardown(pool).await;
}

pub async fn kind0_search_by_display_name_works_without_flattening(backend: Backend) {
    // The case I worried might regress without the kind:0 content-flattening
    // hack. Postgres FTS over raw JSON content tokenizes through the
    // punctuation and finds display_name/nip05 values.
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        0,
        r#"{"display_name":"Alice Wonderland","name":"alice","nip05":"alice@buzz.app","about":"hello"}"#,
        None,
        1700000000,
    )
    .await;

    let svc = fx.service();
    for q in ["wonderland", "alice", "alice@buzz.app"] {
        let r = svc
            .search(&SearchQuery {
                community: c,
                q: q.to_string(),
                channel_scope: ChannelScope::Any,
                kinds: Some(vec![0]),
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: buzz_search::SearchMode::FullText,
                sort: buzz_search::SearchSort::Relevance,
            })
            .await
            .unwrap();
        assert_eq!(r.hits.len(), 1, "kind:0 query {q:?} should find Alice");
    }

    fx.teardown(pool).await;
}

pub async fn short_kind0_prefix_prioritizes_exact_lexeme_on_a_noisy_page(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "short-profile-prefix.example").await;
    let exact_id = rand_bytes32();
    insert_event(
        &pool,
        c,
        exact_id,
        rand_bytes32(),
        0,
        r#"{"display_name":"jm"}"#,
        None,
        1_700_000_000,
    )
    .await;

    // These profiles all match jm:* and are newer than the exact name. Without
    // exact-lexeme priority they consume the entire bounded first page.
    for (i, display_name) in ["jma", "jmbravo", "jmcharlie", "jmdelta"]
        .iter()
        .enumerate()
    {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            rand_bytes32(),
            0,
            &format!(r#"{{"display_name":"{display_name}"}}"#),
            None,
            1_700_000_100 + i as i64,
        )
        .await;
    }

    let svc = fx.service();
    let first_page = svc
        .search(&SearchQuery {
            community: c,
            q: "jm".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![0]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 3,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("short profile prefix search ok");

    assert_eq!(first_page.hits.len(), 3);
    assert_eq!(first_page.hits[0].event_id, exact_id);

    fx.teardown(pool).await;
}

pub async fn prefix_mode_matches_final_token_prefix_without_changing_full_text(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "prefix.example").await;
    let project_plan = rand_bytes32();
    let project_archive = rand_bytes32();
    let projectile_plan = rand_bytes32();
    insert_event(
        &pool,
        c,
        project_plan,
        rand_bytes32(),
        9,
        "project planning milestone",
        None,
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        project_archive,
        rand_bytes32(),
        9,
        "project archive",
        None,
        1_700_000_001,
    )
    .await;
    insert_event(
        &pool,
        c,
        projectile_plan,
        rand_bytes32(),
        9,
        "projectile planning distractor",
        None,
        1_700_000_002,
    )
    .await;
    // Rows whose own parser output contains a short lexeme, so a short
    // trailing `:*` from punctuation in the query would match them.
    let contraction = rand_bytes32();
    let hyphenated = rand_bytes32();
    let profile = rand_bytes32();
    insert_event(
        &pool,
        c,
        contraction,
        rand_bytes32(),
        9,
        "don't panic",
        None,
        1_700_000_003,
    )
    .await;
    insert_event(
        &pool,
        c,
        hyphenated,
        rand_bytes32(),
        9,
        "pre-processing done",
        None,
        1_700_000_004,
    )
    .await;
    insert_event(
        &pool,
        c,
        profile,
        rand_bytes32(),
        0,
        r#"{"display_name":"prof"}"#,
        None,
        1_700_000_005,
    )
    .await;

    let svc = fx.service();
    let full_text = svc
        .search(&SearchQuery {
            community: c,
            q: "pro".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("full text search ok");
    assert!(
        full_text.hits.is_empty(),
        "plain full-text search must stay word/lexeme-based, got {:?}",
        full_text
            .hits
            .iter()
            .map(|h| h.event_id)
            .collect::<Vec<_>>()
    );

    let prefix = svc
        .search(&SearchQuery {
            community: c,
            q: "pro".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("prefix search ok");
    let prefix_ids: Vec<[u8; 32]> = prefix.hits.iter().map(|h| h.event_id).collect();
    assert!(prefix_ids.contains(&project_plan));
    assert!(prefix_ids.contains(&project_archive));

    // Non-profile prefix search returns no hits, in either sort, whatever the
    // kinds and whatever completed words precede it, when the trailing token's
    // `simple` parser output has a lexeme below three characters. Punctuation
    // must not pad the count: `pr.`, `@pr` and `#pr` still yield `pr`, `don't`
    // yields `t` and `pre-p` yields `p`. Each row would match without the
    // minimum. kind:0-only stays one-character typeahead.
    for (q, kinds, sort, expected) in [
        (
            "pr",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            " p ",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "project pr",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "project  p ",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        ("pr", None, buzz_search::SearchSort::Relevance, vec![]),
        (
            "pr",
            Some(vec![0, 9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        ("pr", Some(vec![9]), buzz_search::SearchSort::Newest, vec![]),
        (
            "pr.",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "@pr",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "#pr",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "...p",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "don't",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        (
            "pre-p",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![],
        ),
        // Controls: padded but long enough, punctuation-only trailing token
        // (no `:*` lexeme, so the completed word matches exactly), long
        // hyphenated parts, a short completed word (exact, so not limited),
        // and a one-character profile query.
        (
            "pro.",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![project_plan, project_archive, projectile_plan, hyphenated],
        ),
        (
            "project ?",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![project_plan, project_archive],
        ),
        (
            "pre-pro",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![hyphenated],
        ),
        (
            "t pan",
            Some(vec![9]),
            buzz_search::SearchSort::Relevance,
            vec![contraction],
        ),
        (
            "p",
            Some(vec![0]),
            buzz_search::SearchSort::Relevance,
            vec![profile],
        ),
    ] {
        let result = svc
            .search(&SearchQuery {
                community: c,
                q: q.into(),
                channel_scope: ChannelScope::Any,
                kinds: kinds.clone(),
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: buzz_search::SearchMode::Prefix,
                sort,
            })
            .await
            .expect("prefix search ok");
        let mut got: Vec<[u8; 32]> = result.hits.iter().map(|h| h.event_id).collect();
        let mut want = expected.clone();
        got.sort();
        want.sort();
        assert_eq!(got, want, "{q:?} {kinds:?} {sort:?}");
    }
    // A short trailing lexeme is a prefix-mode rule only; full text still
    // matches the contraction exactly.
    let full_text_contraction = svc
        .search(&SearchQuery {
            community: c,
            q: "don't".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("full text search ok");
    assert_eq!(
        full_text_contraction
            .hits
            .iter()
            .map(|h| h.event_id)
            .collect::<Vec<_>>(),
        vec![contraction]
    );

    let multi_token_prefix = svc
        .search(&SearchQuery {
            community: c,
            q: "project pla".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("multi-token prefix search ok");
    assert_eq!(multi_token_prefix.hits.len(), 1);
    assert_eq!(multi_token_prefix.hits[0].event_id, project_plan);

    let completed_token_is_exact = svc
        .search(&SearchQuery {
            community: c,
            q: "project plan".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("completed-token exact + trailing-prefix search ok");
    let completed_ids: Vec<[u8; 32]> = completed_token_is_exact
        .hits
        .iter()
        .map(|h| h.event_id)
        .collect();
    assert!(completed_ids.contains(&project_plan));
    assert!(
        !completed_ids.contains(&projectile_plan),
        "completed tokens must stay exact; only the trailing token gets :*"
    );

    fx.teardown(pool).await;
}

pub async fn prefix_mode_handles_tsquery_boundary_punctuation(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "prefix-punctuation.example").await;
    let hit_id = rand_bytes32();
    insert_event(
        &pool,
        c,
        hit_id,
        rand_bytes32(),
        9,
        "operators ' : & | ( ) ! alpha beta marker",
        None,
        1_700_000_000,
    )
    .await;

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "operators ' : & | ( ) ! alpha bet".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("prefix punctuation search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, hit_id);

    fx.teardown(pool).await;
}

pub async fn prefix_mode_preserves_storage_level_privacy_exclusions(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "prefix-privacy.example").await;
    let token = "prefixprivacy_unique_marker";
    let control_id = rand_bytes32();

    insert_event(
        &pool,
        c,
        control_id,
        rand_bytes32(),
        9,
        &format!("public control {token}"),
        None,
        1_700_000_000,
    )
    .await;

    for (i, &kind) in [1059_u32, 30300, 30622, 44100, 44101].iter().enumerate() {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            rand_bytes32(),
            kind as i32,
            &format!("private kind {kind} {token}"),
            None,
            1_700_000_100 + i as i64,
        )
        .await;
    }

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "prefixpriv".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("prefix privacy search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, control_id);

    fx.teardown(pool).await;
}

/// Prefix relevance (typeahead) ranks only the newest 1000 matches, so an
/// older, better-ranked message drops out, and its pages end with that
/// window; full-text and profile relevance still rank every match.
/// Mutation: rank every prefix match → the old message tops the first page;
/// window profiles too → the old profile drops off the first page.
pub async fn prefix_relevance_ranks_only_the_newest_matches(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;
    let c = mk_community(&pool, "rank-window.example").await;
    let (old, old_profile) = (rand_bytes32(), rand_bytes32());
    for (id, kind) in [(old, 9), (old_profile, 0)] {
        insert_event(
            &pool,
            c,
            id,
            rand_bytes32(),
            kind,
            "planning planning planning planning",
            None,
            1_700_000_000,
        )
        .await;
        // 1000 newer, weaker matches fill the window.
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
             SELECT $1, sha256((g || '/' || $2)::bytea), sha256((g || '/' || $2)::bytea), \
                    to_timestamp(1700000001 + g), $2, '[]'::jsonb, 'planning notes ' || g, 'signature' \
             FROM generate_series(1, 1000) g",
        )
        .bind(c.as_uuid())
        .bind(kind)
        .execute(&pool)
        .await
        .expect("insert newer matches");
    }

    let svc = fx.service();
    let query = |q: &str, mode, page| SearchQuery {
        community: c,
        q: q.into(),
        channel_scope: ChannelScope::Any,
        kinds: Some(vec![9]),
        authors: None,
        since: None,
        until: None,
        page,
        per_page: 20,
        mode,
        sort: buzz_search::SearchSort::Relevance,
    };
    let ids = |r: buzz_search::SearchResult| r.hits.iter().map(|h| h.event_id).collect::<Vec<_>>();

    let typeahead = ids(svc
        .search(&query("plan", buzz_search::SearchMode::Prefix, 1))
        .await
        .expect("prefix"));
    assert_eq!(typeahead.len(), 20);
    assert!(!typeahead.contains(&old), "outside the newest-1000 window");

    let full_text = ids(svc
        .search(&query("planning", buzz_search::SearchMode::FullText, 1))
        .await
        .expect("full text"));
    assert_eq!(full_text.first(), Some(&old), "full text ranks every match");

    // Every page ranks the same window, so prefix pages end with it.
    let last = ids(svc
        .search(&query("plan", buzz_search::SearchMode::Prefix, 50))
        .await
        .expect("page 50"));
    assert_eq!(last.len(), 20);
    assert!(!last.contains(&old));
    let past = svc
        .search(&query("plan", buzz_search::SearchMode::Prefix, 51))
        .await
        .expect("page 51");
    assert!(past.hits.is_empty(), "no page past the window");

    let profiles = ids(svc
        .search(&SearchQuery {
            kinds: Some(vec![0]),
            ..query("plan", buzz_search::SearchMode::Prefix, 1)
        })
        .await
        .expect("profile prefix"));
    assert_eq!(
        profiles.first(),
        Some(&old_profile),
        "profile typeahead ranks every match"
    );

    fx.teardown(pool).await;
}

pub async fn authors_filter_restricts_results(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;
    let c = mk_community(&pool, "a.example").await;
    let alice = rand_bytes32();
    let bob = rand_bytes32();
    let carol = rand_bytes32();
    let alice_id = rand_bytes32();
    let bob_id = rand_bytes32();
    insert_event(
        &pool,
        c,
        alice_id,
        alice,
        9,
        "authorword from alice",
        None,
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        bob_id,
        bob,
        9,
        "authorword from bob",
        None,
        1_700_000_001,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        carol,
        9,
        "authorword from carol",
        None,
        1_700_000_002,
    )
    .await;

    let svc = fx.service();
    let search = |authors: Option<Vec<Vec<u8>>>| {
        let query = SearchQuery {
            community: c,
            q: "authorword".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Newest,
        };
        async move {
            svc.search(&query)
                .await
                .expect("search")
                .hits
                .into_iter()
                .map(|hit| hit.event_id)
                .collect::<Vec<_>>()
        }
    };

    assert_eq!(
        search(Some(vec![alice.to_vec(), bob.to_vec()])).await,
        vec![bob_id, alice_id]
    );
    assert_eq!(search(None).await.len(), 3);

    fx.teardown(pool).await;
}

pub async fn channel_scope_restricts_results(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    // Channels need community_id since channels PK is (community_id, id).
    let ch_a = Uuid::new_v4();
    let ch_b = Uuid::new_v4();
    sqlx::query("INSERT INTO channels (community_id, id, name, channel_type, created_by) VALUES ($1, $2, $3, 'stream'::channel_type, $4), ($1, $5, $6, 'stream'::channel_type, $4)")
        .bind(c.as_uuid())
        .bind(ch_a)
        .bind("ch-a")
        .bind(b"\x01".repeat(32))
        .bind(ch_b)
        .bind("ch-b")
        .execute(&pool)
        .await
        .expect("insert channels");

    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "shared-token in ch-a",
        Some(ch_a),
        1700000000,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "shared-token in ch-b",
        Some(ch_b),
        1700000001,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "shared-token global",
        None,
        1700000002,
    )
    .await;

    let svc = fx.service();

    // restrict to ch_a, exclude global
    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "shared-token".into(),
            channel_scope: ChannelScope::Channels(vec![ch_a]),
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].channel_id, Some(ch_a));

    // restrict to ch_a + include global
    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "shared-token".into(),
            channel_scope: ChannelScope::ChannelsOrChannelLess(vec![ch_a]),
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 2);

    // no channel constraint
    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "shared-token".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 3);

    // empty accessible channels + exclude global = zero
    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "shared-token".into(),
            channel_scope: ChannelScope::Channels(vec![]),
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 0);

    fx.teardown(pool).await;
}

pub async fn deleted_events_are_excluded(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let evt_id = rand_bytes32();
    let pk = rand_bytes32();
    insert_event(&pool, c, evt_id, pk, 9, "deleted-token-q", None, 1700000000).await;

    // Soft-delete
    sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(c.as_uuid())
        .bind(&evt_id[..])
        .execute(&pool)
        .await
        .expect("delete");

    let svc = fx.service();
    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "deleted-token-q".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert!(
        r.hits.is_empty(),
        "soft-deleted events must not appear in FTS"
    );

    fx.teardown(pool).await;
}

pub async fn empty_query_returns_empty_result_no_roundtrip(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;
    let c = mk_community(&pool, "a.example").await;
    let svc = fx.service();

    for q in ["", "   "] {
        let r = svc
            .search(&SearchQuery {
                community: c,
                q: q.into(),
                channel_scope: ChannelScope::Any,
                kinds: None,
                authors: None,
                since: None,
                until: None,
                page: 1,
                per_page: 10,
                mode: buzz_search::SearchMode::FullText,
                sort: buzz_search::SearchSort::Relevance,
            })
            .await
            .unwrap();
        assert!(
            r.hits.is_empty(),
            "empty/whitespace query must return no hits"
        );
    }

    fx.teardown(pool).await;
}

pub async fn since_until_filters(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "time-token-zz at A",
        None,
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "time-token-zz at B",
        None,
        1_700_010_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "time-token-zz at C",
        None,
        1_700_020_000,
    )
    .await;

    let svc = fx.service();

    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "time-token-zz".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: Some(1_700_005_000),
            until: Some(1_700_015_000),
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].created_at, 1_700_010_000);

    fx.teardown(pool).await;
}

pub async fn pagination_works(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let pk = rand_bytes32();
    for i in 0..7 {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            pk,
            9,
            "page-token-qq",
            None,
            1_700_000_000 + i,
        )
        .await;
    }

    let svc = fx.service();

    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "page-token-qq".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 3,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 3);

    let r2 = svc
        .search(&SearchQuery {
            community: c,
            q: "page-token-qq".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 3,
            per_page: 3,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(
        r2.hits.len(),
        1,
        "page 3 of 7 with per_page=3 should have 1 hit"
    );

    fx.teardown(pool).await;
}

pub async fn newest_sort_pages_by_time_then_id_within_scope(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let channel = Uuid::new_v4();
    let pk = rand_bytes32();
    let most_relevant = [9u8; 32];
    let tie_low = [1u8; 32];
    let tie_high = [2u8; 32];
    let newest = [3u8; 32];
    let rows = [
        (
            most_relevant,
            "sortword sortword sortword sortword",
            1_700_000_000,
        ),
        (tie_high, "sortword once", 1_700_000_100),
        (tie_low, "sortword once", 1_700_000_100),
        (newest, "sortword once", 1_700_000_200),
    ];
    for (id, content, created_at) in rows {
        insert_event(&pool, c, id, pk, 9, content, Some(channel), created_at).await;
    }
    // Newer than every scoped row, but outside the requested channel scope.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "sortword once",
        Some(Uuid::new_v4()),
        1_700_000_300,
    )
    .await;

    let svc = fx.service();
    let page = |page, sort| async move {
        svc.search(&SearchQuery {
            community: c,
            q: "sortword".into(),
            channel_scope: ChannelScope::Channels(vec![channel]),
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page,
            per_page: 2,
            mode: buzz_search::SearchMode::FullText,
            sort,
        })
        .await
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.event_id)
        .collect::<Vec<_>>()
    };

    let newest_order = [
        page(1, buzz_search::SearchSort::Newest).await,
        page(2, buzz_search::SearchSort::Newest).await,
        page(3, buzz_search::SearchSort::Newest).await,
    ]
    .concat();
    assert_eq!(newest_order, vec![newest, tie_low, tie_high, most_relevant]);
    assert_eq!(
        page(1, buzz_search::SearchSort::Relevance).await[0],
        most_relevant
    );

    fx.teardown(pool).await;
}

/// A heavy DM user's scope holds thousands of channel ids; the whole scope
/// must reach the backend.
pub async fn channel_scope_of_thousands_of_channels(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;
    let c = mk_community(&pool, "a.example").await;
    let inside = Uuid::new_v4();
    let outside = Uuid::new_v4();
    let hit = rand_bytes32();
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        hit,
        pk,
        9,
        "scopeword inside",
        Some(inside),
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "scopeword outside",
        Some(outside),
        1_700_000_001,
    )
    .await;
    let mut scope: Vec<Uuid> = (0..4_999).map(|_| Uuid::new_v4()).collect();
    scope.push(inside);

    let result = fx
        .service()
        .search(&SearchQuery {
            community: c,
            q: "scopeword".into(),
            channel_scope: ChannelScope::Channels(scope),
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("search");
    let ids: Vec<_> = result.hits.iter().map(|hit| hit.event_id).collect();
    assert_eq!(ids, vec![hit]);

    fx.teardown(pool).await;
}

pub async fn channel_less_only_excludes_per_channel_events(backend: Backend) {
    // Closes the row-3 fence hole: in the legacy 2x2 shape, both
    // `Some(vec![]) + true` and `None + true` silently broadened to all
    // community channels rather than restricting to channel-less events.
    // `ChannelScope::ChannelLessOnly` is the variant that the old type
    // could not express.
    //
    // Adversarial check: mutate this test's expectation to `>= 2` and the
    // assertion goes red against the new SQL `AND channel_id IS NULL`,
    // proving the predicate bites. Mutate `query.rs` `ChannelLessOnly` arm
    // to a no-op (the `Any` semantic the old code emitted) and this test
    // also goes red — three hits instead of one — proving the fix is the
    // emitted predicate, not the variant name.
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "a.example").await;
    let ch_a = Uuid::new_v4();
    let ch_b = Uuid::new_v4();
    sqlx::query("INSERT INTO channels (community_id, id, name, channel_type, created_by) VALUES ($1, $2, $3, 'stream'::channel_type, $4), ($1, $5, $6, 'stream'::channel_type, $4)")
        .bind(c.as_uuid())
        .bind(ch_a)
        .bind("ch-a")
        .bind(b"\x01".repeat(32))
        .bind(ch_b)
        .bind("ch-b")
        .execute(&pool)
        .await
        .expect("insert channels");

    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "fence-token in ch-a",
        Some(ch_a),
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "fence-token in ch-b",
        Some(ch_b),
        1_700_000_001,
    )
    .await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        9,
        "fence-token channel-less",
        None,
        1_700_000_002,
    )
    .await;

    let svc = fx.service();

    let r = svc
        .search(&SearchQuery {
            community: c,
            q: "fence-token".into(),
            channel_scope: ChannelScope::ChannelLessOnly,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .unwrap();
    assert_eq!(
        r.hits.len(),
        1,
        "ChannelLessOnly must return only the channel_id IS NULL row"
    );
    assert_eq!(r.hits[0].channel_id, None);

    fx.teardown(pool).await;
}

/// Search-input hardening: NUL bytes are not valid Postgres text-search input.
/// Sanitize before calling `websearch_to_tsquery` so the bridge does not turn
/// adversarial search text into HTTP 500s.
pub async fn nul_bytes_in_query_are_sanitized(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "nul.example").await;
    let evt_id = rand_bytes32();
    insert_event(
        &pool,
        c,
        evt_id,
        rand_bytes32(),
        9,
        "foo bar search text",
        None,
        1_700_000_000,
    )
    .await;

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "foo\0bar".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("NUL-containing search query should not bubble a Postgres error");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, evt_id);

    fx.teardown(pool).await;
}

pub async fn enormous_page_number_is_clamped(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "page-clamp.example").await;
    for i in 0..5 {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            rand_bytes32(),
            9,
            "clamp-token",
            None,
            1_700_000_000 + i,
        )
        .await;
    }

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "clamp-token".into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: u32::MAX,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("huge page number should be bounded, not error");

    assert_eq!(result.page, 1000);
    assert!(result.hits.is_empty());

    fx.teardown(pool).await;
}

pub async fn very_long_query_is_bounded_before_pg_parse(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "long-query.example").await;
    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "x".repeat(10_000),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("long search query should be capped before Postgres parses it");

    assert!(result.hits.is_empty());

    fx.teardown(pool).await;
}

/// Privacy regression gate: the storage layer MUST NOT make these kinds
/// searchable. The migration's `search_tsv` generated column emits NULL
/// for excluded kinds, so a `search_tsv @@ query` probe never matches.
///
/// Set kept in sync with the pre-rewrite skip in `handlers/event.rs:287-290`
/// on `main`:
///   - 1059  = `KIND_GIFT_WRAP`      (NIP-17 ciphertext)
///   - 30300 = `KIND_EVENT_REMINDER` (in `AUTHOR_ONLY_KINDS`)
///   - 30622 = `KIND_DM_VISIBILITY`  (per-viewer private hide state)
///   - 44100 = `KIND_MEMBER_ADDED_NOTIFICATION`  (p-gated membership notice)
///   - 44101 = `KIND_MEMBER_REMOVED_NOTIFICATION` (p-gated membership notice)
///   - 44200 = `KIND_AGENT_TURN_METRIC` (NIP-AM: p-gated encrypted turn metrics)
///
/// All seven events are inserted with the same unique token in their content
/// so a single search query exercises every kind in one round-trip. Only
/// the kind:9 control must surface — the excluded kinds must not.
///
/// Mutate-bite: drop the `CASE WHEN kind IN (…)` from the generated column
/// (revert to `to_tsvector('simple', content)`) → excluded events surface →
/// restore.
pub async fn excluded_kinds_are_storage_level_unsearchable(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "privacy.example").await;
    let token = "privacykinds_unique_marker_xyzzy";

    // kind:9 control — MUST be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        9,
        &format!("public chat — {token}"),
        None,
        1_700_000_000,
    )
    .await;

    // kind:1059 gift wrap (NIP-17 ciphertext) — MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        1059,
        &format!("gift wrap — {token}"),
        None,
        1_700_000_001,
    )
    .await;

    // kind:30300 event reminder (AUTHOR_ONLY_KINDS) — MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        30300,
        &format!("reminder — {token}"),
        None,
        1_700_000_002,
    )
    .await;

    // kind:30622 DM visibility snapshot — MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        30622,
        &format!("dm visibility — {token}"),
        None,
        1_700_000_003,
    )
    .await;

    // kind:44100 member-added notification — p-gated and MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        KIND_MEMBER_ADDED_NOTIFICATION as i32,
        &format!("member added — {token}"),
        None,
        1_700_000_004,
    )
    .await;

    // kind:44101 member-removed notification — p-gated and MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        KIND_MEMBER_REMOVED_NOTIFICATION as i32,
        &format!("member removed — {token}"),
        None,
        1_700_000_005,
    )
    .await;

    // kind:44200 agent turn metric — p-gated NIP-44 ciphertext and MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        KIND_AGENT_TURN_METRIC as i32,
        &format!("agent turn metric — {token}"),
        None,
        1_700_000_006,
    )
    .await;

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: token.into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("search ok");

    let kinds: Vec<i32> = result.hits.iter().map(|h| h.kind).collect();

    // Positive: kind:9 surfaces (control — proves the search index works at all).
    assert!(
        kinds.contains(&9),
        "kind:9 control row MUST be searchable, got kinds={kinds:?}",
    );

    // Negative (load-bearing): each excluded kind MUST NOT surface.
    for forbidden in [
        1059,
        30300,
        30622,
        KIND_MEMBER_ADDED_NOTIFICATION as i32,
        KIND_MEMBER_REMOVED_NOTIFICATION as i32,
        KIND_AGENT_TURN_METRIC as i32,
    ] {
        assert!(
            !kinds.contains(&forbidden),
            "kind:{forbidden} MUST NOT be searchable — \
             privacy regression in `search_tsv` generated column. kinds={kinds:?}",
        );
    }

    // Tight bound: exactly one hit (the control). Catches any future
    // weakening where some-but-not-all excluded kinds surface.
    assert_eq!(
        result.hits.len(),
        1,
        "expected exactly 1 hit (the kind:9 control), got {} (kinds={kinds:?})",
        result.hits.len(),
    );

    fx.teardown(pool).await;
}

/// Tripwire: every Rust-side author-only kind MUST be excluded from
/// `search_tsv` at the storage layer.
///
/// The schema generated column hard-codes the privacy skip-set, while
/// `AUTHOR_ONLY_KINDS` is a Rust const. If a future author-only kind is added
/// without the matching schema migration, search would still spend FTS budget on
/// those private hits before the relay post-filter rejects them. Catch that
/// drift here by inserting one row per author-only kind and proving only the
/// public kind:9 control is searchable.
pub async fn author_only_kinds_are_storage_level_unsearchable(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "author-only-tripwire.example").await;
    let token = "authoronly_tripwire_marker_qwerty";

    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        9,
        &format!("public control — {token}"),
        None,
        1_700_000_000,
    )
    .await;

    for (i, &kind) in AUTHOR_ONLY_KINDS.iter().enumerate() {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            rand_bytes32(),
            kind as i32,
            &format!("author-only kind:{kind} — {token}"),
            None,
            1_700_000_100 + i as i64,
        )
        .await;
    }

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: token.into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 100,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("search ok");

    let kinds: Vec<i32> = result.hits.iter().map(|h| h.kind).collect();
    assert!(
        kinds.contains(&9),
        "kind:9 control row MUST be searchable, got kinds={kinds:?}",
    );

    for &kind in AUTHOR_ONLY_KINDS {
        assert!(
            !kinds.contains(&(kind as i32)),
            "AUTHOR_ONLY kind:{kind} MUST NOT be searchable — \
             schema skip-set is missing this kind. AUTHOR_ONLY_KINDS={AUTHOR_ONLY_KINDS:?}, \
             hits={kinds:?}",
        );
    }

    assert_eq!(
        result.hits.len(),
        1,
        "expected exactly 1 hit (the kind:9 control), got {} (kinds={kinds:?})",
        result.hits.len(),
    );

    fx.teardown(pool).await;
}

/// Tripwire: every Rust-side `P_GATED_KINDS` entry that is *persistent* (not
/// in the ephemeral 20000–29999 range) MUST be excluded from `search_tsv` at
/// the storage layer.
///
/// L2 (the filter-level `#p` gate in `p_gated_filters_authorized`) prevents
/// reachable leaks today, but it is Rust logic — a future bug or new exempt
/// search entry point could surface tokenized content from these kinds. The
/// L1 NULL tsvector is the unbreakable backstop: `@@` mathematically cannot
/// match NULL. This test catches the drift where someone adds a persistent
/// kind to `P_GATED_KINDS` without the matching `schema/schema.sql` +
/// `migrations/0001_initial_schema.sql` skip-set update.
///
/// Ephemeral kinds (20000–29999) are skipped: they are never stored, so the
/// storage-layer defense does not apply to them regardless of the schema
/// CASE. `p_gated_filters_authorized` remains their sole defense by design.
///
/// Companion to `author_only_kinds_are_storage_level_unsearchable`: that test
/// covers `AUTHOR_ONLY_KINDS` drift; this one covers `P_GATED_KINDS`
/// persistent-subset drift. Together they tripwire both Rust-side privacy
/// constants against the schema literal.
pub async fn p_gated_persistent_kinds_have_storage_null_tsvector(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;

    let c = mk_community(&pool, "p-gated-tripwire.example").await;
    let token = "pgated_tripwire_marker_qwerty";

    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        9,
        &format!("public control — {token}"),
        None,
        1_700_000_000,
    )
    .await;

    let persistent: Vec<u32> = P_GATED_KINDS
        .iter()
        .copied()
        .filter(|&k| !buzz_core::kind::is_ephemeral(k))
        .collect();
    assert!(
        !persistent.is_empty(),
        "P_GATED_KINDS must include at least one persistent kind for this \
         test to be meaningful; got {P_GATED_KINDS:?}",
    );

    for (i, &kind) in persistent.iter().enumerate() {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            rand_bytes32(),
            kind as i32,
            &format!("p-gated kind:{kind} — {token}"),
            None,
            1_700_000_100 + i as i64,
        )
        .await;
    }

    let svc = fx.service();
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: token.into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 100,
            mode: buzz_search::SearchMode::FullText,
            sort: buzz_search::SearchSort::Relevance,
        })
        .await
        .expect("search ok");

    let kinds: Vec<i32> = result.hits.iter().map(|h| h.kind).collect();
    assert!(
        kinds.contains(&9),
        "kind:9 control row MUST be searchable, got kinds={kinds:?}",
    );

    for &kind in &persistent {
        assert!(
            !kinds.contains(&(kind as i32)),
            "P_GATED persistent kind:{kind} MUST NOT be searchable — \
             schema NULL tsvector skip-set is missing this kind. Defense \
             reduces to L2 (filter-level `#p` gate) alone. \
             P_GATED_KINDS={P_GATED_KINDS:?}, hits={kinds:?}",
        );
    }

    assert_eq!(
        result.hits.len(),
        1,
        "expected exactly 1 hit (the kind:9 control), got {} (kinds={kinds:?})",
        result.hits.len(),
    );

    fx.teardown(pool).await;
}

pub async fn artifact_search_uses_only_live_current_head(backend: Backend) {
    let (pool, fx) = Fixture::setup(backend).await;
    let community = mk_community(&pool, "artifact-search.example").await;
    let pk = rand_bytes32();
    let old = rand_bytes32();
    let head = rand_bytes32();
    let artifact = Uuid::new_v4();
    insert_event(
        &pool,
        community,
        old,
        pk,
        45010,
        "artifactneedle old",
        None,
        1700000000,
    )
    .await;
    insert_event(
        &pool,
        community,
        head,
        pk,
        45010,
        "artifactneedle current",
        None,
        1700000001,
    )
    .await;
    sqlx::query("INSERT INTO artifact_heads(community_id,artifact_id,event_id,channel_id,artifact_type) VALUES($1,$2,$3,$4,'buzz.task')")
        .bind(community.as_uuid()).bind(artifact).bind(head.as_slice()).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let svc = fx.service();
    let query = SearchQuery {
        community,
        q: "artifactneedle".into(),
        channel_scope: ChannelScope::Any,
        kinds: Some(vec![45010]),
        authors: None,
        since: None,
        until: None,
        page: 1,
        per_page: 10,
        mode: buzz_search::SearchMode::FullText,
        sort: buzz_search::SearchSort::Relevance,
    };
    let result = svc.search(&query).await.unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, head);
    sqlx::query("UPDATE artifact_heads SET deleted=true WHERE community_id=$1 AND artifact_id=$2")
        .bind(community.as_uuid())
        .bind(artifact)
        .execute(&pool)
        .await
        .unwrap();
    let result = svc.search(&query).await.unwrap();
    assert!(result.hits.is_empty());
    fx.teardown(pool).await;
}
