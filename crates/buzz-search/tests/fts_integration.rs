//! Integration tests for community-scoped Postgres FTS.
//!
//! Run with a local PG: `BUZZ_TEST_DATABASE_URL=postgres://buzz:buzz_dev@localhost:5432/buzz cargo test -p buzz-search --tests -- --include-ignored`
//!
//! Each test creates a uniquely-named schema, applies the consolidated `0001`
//! migration into it, exercises a scenario, and drops it. Tests are
//! parallel-safe.

use buzz_core::CommunityId;
use buzz_search::{ChannelScope, SearchQuery, SearchService};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
const MIGRATION_SQL: &str = include_str!("../../../migrations/0001_initial_schema.sql");

async fn setup() -> (PgPool, String) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    let schema = format!("fts_test_{}", Uuid::new_v4().simple());
    // Connect to the default schema first to create the test schema.
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    let create_sql = format!("CREATE SCHEMA \"{schema}\"");
    sqlx::query(sqlx::AssertSqlSafe(create_sql))
        .execute(&admin_pool)
        .await
        .expect("create schema");
    admin_pool.close().await;

    // Connect with search_path set so the migration's CREATE TABLE lands here.
    let url_with_search_path = format!("{url}?options=-c%20search_path%3D{schema}");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_with_search_path)
        .await
        .expect("connect with search_path");
    pool.execute(MIGRATION_SQL)
        .await
        .expect("apply 0001 migration");
    (pool, schema)
}

async fn teardown(pool: PgPool, schema: &str) {
    pool.close().await;
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string()),
        )
        .await
        .expect("reconnect for drop");
    let drop_sql = format!("DROP SCHEMA \"{schema}\" CASCADE");
    sqlx::query(sqlx::AssertSqlSafe(drop_sql))
        .execute(&admin_pool)
        .await
        .expect("drop schema");
    admin_pool.close().await;
}

/// Insert a community row, return its id.
async fn mk_community(pool: &PgPool, host: &str) -> CommunityId {
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

/// Insert a minimal event. `created_at_secs` is unix seconds.
#[allow(clippy::too_many_arguments)]
async fn insert_event(
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
}

fn rand_bytes32() -> [u8; 32] {
    let mut out = [0u8; 32];
    let u = Uuid::new_v4();
    let bytes = u.as_bytes();
    out[..16].copy_from_slice(bytes);
    out[16..].copy_from_slice(bytes);
    out
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn search_finds_event_in_same_community() {
    let (pool, schema) = setup().await;

    let c_a = mk_community(&pool, "a.example").await;
    let evt_id = rand_bytes32();
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c_a,
        evt_id,
        pk,
        1,
        "hello wonderland — buzz everyone",
        None,
        1700000000,
    )
    .await;

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, evt_id);
    assert_eq!(result.hits[0].kind, 1);
    assert_eq!(result.hits[0].created_at, 1700000000);
    assert!(result.hits[0].rank > 0.0);

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn search_does_not_return_other_community_events() {
    // The load-bearing test: event indexed under community A, query bound to
    // community B → zero hits.
    let (pool, schema) = setup().await;

    let c_a = mk_community(&pool, "a.example").await;
    let c_b = mk_community(&pool, "b.example").await;
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c_a,
        rand_bytes32(),
        pk,
        1,
        "only-in-a unique-token-xyz",
        None,
        1700000000,
    )
    .await;

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .unwrap();
    assert_eq!(result_b.hits.len(), 0, "B must not see A's event");

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn kind0_search_by_display_name_works_without_flattening() {
    // The case I worried might regress without the kind:0 content-flattening
    // hack. Postgres FTS over raw JSON content tokenizes through the
    // punctuation and finds display_name/nip05 values.
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
            })
            .await
            .unwrap();
        assert_eq!(r.hits.len(), 1, "kind:0 query {q:?} should find Alice");
    }

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_scope_restricts_results() {
    let (pool, schema) = setup().await;

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
        1,
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
        1,
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
        1,
        "shared-token global",
        None,
        1700000002,
    )
    .await;

    let svc = SearchService::new(pool.clone());

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
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 0);

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deleted_events_are_excluded() {
    let (pool, schema) = setup().await;

    let c = mk_community(&pool, "a.example").await;
    let evt_id = rand_bytes32();
    let pk = rand_bytes32();
    insert_event(&pool, c, evt_id, pk, 1, "deleted-token-q", None, 1700000000).await;

    // Soft-delete
    sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(c.as_uuid())
        .bind(&evt_id[..])
        .execute(&pool)
        .await
        .expect("delete");

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .unwrap();
    assert!(
        r.hits.is_empty(),
        "soft-deleted events must not appear in FTS"
    );

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn empty_query_returns_empty_result_no_roundtrip() {
    let (pool, schema) = setup().await;
    let c = mk_community(&pool, "a.example").await;
    let svc = SearchService::new(pool.clone());

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
            })
            .await
            .unwrap();
        assert!(
            r.hits.is_empty(),
            "empty/whitespace query must return no hits"
        );
    }

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn since_until_filters() {
    let (pool, schema) = setup().await;

    let c = mk_community(&pool, "a.example").await;
    let pk = rand_bytes32();
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        pk,
        1,
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
        1,
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
        1,
        "time-token-zz at C",
        None,
        1_700_020_000,
    )
    .await;

    let svc = SearchService::new(pool.clone());

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
        })
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 1);
    assert_eq!(r.hits[0].created_at, 1_700_010_000);

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn pagination_works() {
    let (pool, schema) = setup().await;

    let c = mk_community(&pool, "a.example").await;
    let pk = rand_bytes32();
    for i in 0..7 {
        insert_event(
            &pool,
            c,
            rand_bytes32(),
            pk,
            1,
            "page-token-qq",
            None,
            1_700_000_000 + i,
        )
        .await;
    }

    let svc = SearchService::new(pool.clone());

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
        })
        .await
        .unwrap();
    assert_eq!(
        r2.hits.len(),
        1,
        "page 3 of 7 with per_page=3 should have 1 hit"
    );

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_less_only_excludes_per_channel_events() {
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
    let (pool, schema) = setup().await;

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
        1,
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
        1,
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
        1,
        "fence-token channel-less",
        None,
        1_700_000_002,
    )
    .await;

    let svc = SearchService::new(pool.clone());

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
        })
        .await
        .unwrap();
    assert_eq!(
        r.hits.len(),
        1,
        "ChannelLessOnly must return only the channel_id IS NULL row"
    );
    assert_eq!(r.hits[0].channel_id, None);

    teardown(pool, &schema).await;
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
///
/// All four events are inserted with the same unique token in their content
/// so a single search query exercises every kind in one round-trip. Only
/// the kind:9 control must surface — the three excluded kinds must not.
///
/// Mutate-bite: drop the `CASE WHEN kind IN (…)` from the generated column
/// (revert to `to_tsvector('simple', content)`) → all four events surface →
/// restore.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn excluded_kinds_are_storage_level_unsearchable() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
    for forbidden in [1059, 30300, 30622] {
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

    teardown(pool, &schema).await;
}
