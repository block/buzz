//! Integration tests for community-scoped Postgres FTS.
//!
//! Run with a local PG: `BUZZ_TEST_DATABASE_URL=postgres://buzz:buzz_dev@localhost:5432/buzz cargo test -p buzz-search --tests -- --include-ignored`
//!
//! Each test creates a uniquely-named schema, applies the full migration chain
//! (0001 through 0012) into it, exercises a scenario, and drops it. Tests are
//! parallel-safe.

use buzz_core::{
    kind::{
        AUTHOR_ONLY_KINDS, KIND_AGENT_TURN_METRIC, KIND_DRAFT, KIND_MEMBER_ADDED_NOTIFICATION,
        KIND_MEMBER_REMOVED_NOTIFICATION, P_GATED_KINDS,
    },
    CommunityId,
};
use buzz_search::{ChannelScope, SearchQuery, SearchService};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
const MIGRATION_0001_SQL: &str = include_str!("../../../migrations/0001_initial_schema.sql");
const MIGRATION_0002_SQL: &str = include_str!("../../../migrations/0002_git_repo_names.sql");
const MIGRATION_0003_SQL: &str = include_str!("../../../migrations/0003_community_icon.sql");
const MIGRATION_0004_SQL: &str = include_str!("../../../migrations/0004_events_tags_gin.sql");
const MIGRATION_0005_SQL: &str = include_str!("../../../migrations/0005_agent_turn_metric_fts.sql");
const MIGRATION_0006_SQL: &str = include_str!("../../../migrations/0006_moderation.sql");
const MIGRATION_0007_SQL: &str = include_str!("../../../migrations/0007_nip_rs_retention.sql");
const MIGRATION_0008_SQL: &str =
    include_str!("../../../migrations/0008_fresh_install_search_allowlist.sql");
const MIGRATION_0009_SQL: &str =
    include_str!("../../../migrations/0009_nip_rs_database_guards.sql");
const MIGRATION_0010_SQL: &str =
    include_str!("../../../migrations/0010_nip_rs_exact_replay_guard.sql");
const MIGRATION_0011_SQL: &str =
    include_str!("../../../migrations/0011_nip_rs_exact_tag_cardinality.sql");
const MIGRATION_0012_SQL: &str = include_str!("../../../migrations/0012_draft_wrap_fts.sql");

async fn setup() -> (PgPool, String) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    let schema = format!("fts_test_{}", Uuid::new_v4().simple());
    // Connect to the default schema first to create the test schema.
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");

    // Serialize the pgcrypto extension install across all parallel test workers.
    //
    // `CREATE EXTENSION IF NOT EXISTS pgcrypto` targets `pg_extension`, which is
    // database-global with a unique index on `extname`.  PostgreSQL's IF NOT EXISTS
    // is a check-then-insert, not an atomic upsert: when many test workers run
    // concurrently they all see the extension as absent, all attempt the insert,
    // and all but one hit SQLSTATE 23505 (unique violation).  A session-level
    // advisory lock (arbitrary but stable key) serializes the install so that
    // exactly one worker does the real CREATE; the rest treat it as a no-op when
    // they enter the lock.  The lock is released automatically when the session
    // ends, so there is no risk of wedging subsequent runs.
    sqlx::query("SELECT pg_advisory_lock(3723742987654321)")
        .execute(&admin_pool)
        .await
        .expect("acquire pgcrypto advisory lock");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&admin_pool)
        .await
        .expect("install pgcrypto extension");
    sqlx::query("SELECT pg_advisory_unlock(3723742987654321)")
        .execute(&admin_pool)
        .await
        .expect("release pgcrypto advisory lock");

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
    // Apply the full migration chain in order so the test schema exactly matches
    // production. Future FTS-affecting migrations must be added here.
    pool.execute(MIGRATION_0001_SQL)
        .await
        .expect("apply 0001 migration");
    pool.execute(MIGRATION_0002_SQL)
        .await
        .expect("apply 0002 migration");
    pool.execute(MIGRATION_0003_SQL)
        .await
        .expect("apply 0003 migration");
    pool.execute(MIGRATION_0004_SQL)
        .await
        .expect("apply 0004 migration");
    pool.execute(MIGRATION_0005_SQL)
        .await
        .expect("apply 0005 migration");
    pool.execute(MIGRATION_0006_SQL)
        .await
        .expect("apply 0006 migration");
    pool.execute(MIGRATION_0007_SQL)
        .await
        .expect("apply 0007 migration");
    pool.execute(MIGRATION_0008_SQL)
        .await
        .expect("apply 0008 migration");
    pool.execute(MIGRATION_0009_SQL)
        .await
        .expect("apply 0009 migration");
    pool.execute(MIGRATION_0010_SQL)
        .await
        .expect("apply 0010 migration");
    pool.execute(MIGRATION_0011_SQL)
        .await
        .expect("apply 0011 migration");
    pool.execute(MIGRATION_0012_SQL)
        .await
        .expect("apply 0012 migration");
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
        9,
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
            mode: buzz_search::SearchMode::FullText,
        })
        .await
        .expect("search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, evt_id);
    assert_eq!(result.hits[0].kind, 9);
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
        9,
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
            mode: buzz_search::SearchMode::FullText,
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
                mode: buzz_search::SearchMode::FullText,
            })
            .await
            .unwrap();
        assert_eq!(r.hits.len(), 1, "kind:0 query {q:?} should find Alice");
    }

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn prefix_mode_matches_final_token_prefix_without_changing_full_text() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("prefix search ok");
    let prefix_ids: Vec<[u8; 32]> = prefix.hits.iter().map(|h| h.event_id).collect();
    assert!(prefix_ids.contains(&project_plan));
    assert!(prefix_ids.contains(&project_archive));

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
        })
        .await
        .expect("multi-token prefix search ok");
    assert_eq!(multi_token_prefix.hits.len(), 1);
    assert_eq!(multi_token_prefix.hits[0].event_id, project_plan);

    let completed_token_is_exact = svc
        .search(&SearchQuery {
            community: c,
            q: "project pl".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
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

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn prefix_mode_handles_tsquery_boundary_punctuation() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
    let result = svc
        .search(&SearchQuery {
            community: c,
            q: "operators ' : & | ( ) ! alpha be".into(),
            channel_scope: ChannelScope::Any,
            kinds: Some(vec![9]),
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::Prefix,
        })
        .await
        .expect("prefix punctuation search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, hit_id);

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn prefix_mode_preserves_storage_level_privacy_exclusions() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("prefix privacy search ok");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, control_id);

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
            mode: buzz_search::SearchMode::FullText,
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
    insert_event(&pool, c, evt_id, pk, 9, "deleted-token-q", None, 1700000000).await;

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
            mode: buzz_search::SearchMode::FullText,
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
                mode: buzz_search::SearchMode::FullText,
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
            mode: buzz_search::SearchMode::FullText,
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
            9,
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
            mode: buzz_search::SearchMode::FullText,
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
            mode: buzz_search::SearchMode::FullText,
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

/// Search-input hardening: NUL bytes are not valid Postgres text-search input.
/// Sanitize before calling `websearch_to_tsquery` so the bridge does not turn
/// adversarial search text into HTTP 500s.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn nul_bytes_in_query_are_sanitized() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("NUL-containing search query should not bubble a Postgres error");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].event_id, evt_id);

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn enormous_page_number_is_clamped() {
    let (pool, schema) = setup().await;

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

    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("huge page number should be bounded, not error");

    assert_eq!(result.page, 1000);
    assert!(result.hits.is_empty());

    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn very_long_query_is_bounded_before_pg_parse() {
    let (pool, schema) = setup().await;

    let c = mk_community(&pool, "long-query.example").await;
    let svc = SearchService::new(pool.clone());
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
        })
        .await
        .expect("long search query should be capped before Postgres parses it");

    assert!(result.hits.is_empty());

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
///   - 31234 = `KIND_DRAFT`          (NIP-37: channel-bound author-private draft)
///   - 44100 = `KIND_MEMBER_ADDED_NOTIFICATION`  (p-gated membership notice)
///   - 44101 = `KIND_MEMBER_REMOVED_NOTIFICATION` (p-gated membership notice)
///   - 44200 = `KIND_AGENT_TURN_METRIC` (NIP-AM: p-gated encrypted turn metrics)
///
/// All eight events are inserted with the same unique token in their content
/// so a single search query exercises every kind in one round-trip. Only
/// the kind:9 control must surface — the excluded kinds must not.
///
/// Mutate-bite: drop the `CASE WHEN kind IN (…)` from the generated column
/// (revert to `to_tsvector('simple', content)`) → excluded events surface →
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

    // kind:31234 draft wrap (NIP-37, AUTHOR_ONLY_KINDS) — MUST NOT be searchable.
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        KIND_DRAFT as i32,
        &format!("draft wrap — {token}"),
        None,
        1_700_000_004,
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
        1_700_000_005,
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
        1_700_000_006,
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
        1_700_000_007,
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
            mode: buzz_search::SearchMode::FullText,
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
        KIND_DRAFT as i32,
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

    teardown(pool, &schema).await;
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
#[tokio::test]
#[ignore = "requires Postgres"]
async fn author_only_kinds_are_storage_level_unsearchable() {
    let (pool, schema) = setup().await;

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
            per_page: 100,
            mode: buzz_search::SearchMode::FullText,
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

    teardown(pool, &schema).await;
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
#[tokio::test]
#[ignore = "requires Postgres"]
async fn p_gated_persistent_kinds_have_storage_null_tsvector() {
    let (pool, schema) = setup().await;

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
            per_page: 100,
            mode: buzz_search::SearchMode::FullText,
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

    teardown(pool, &schema).await;
}

/// Set up a schema with the LEGACY blocklist expression (pre-#1771 state).
///
/// Applies 0001–0007 (establishing the legacy negative-blocklist `search_tsv`),
/// inserts a dummy row so the `events` table is non-empty, then applies 0008
/// (which skips on non-empty tables) and 0009–0011 (NIP-RS guards, no FTS
/// changes). The resulting schema has the legacy blocklist expression where
/// kind 31234 is NOT excluded — exactly the brownfield state that 0012's
/// rewrite branch must handle.
///
/// Does NOT apply 0012 — the caller controls that step.
async fn setup_legacy_blocklist() -> (PgPool, String) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    let schema = format!("fts_legacy_{}", Uuid::new_v4().simple());
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");

    sqlx::query("SELECT pg_advisory_lock(3723742987654321)")
        .execute(&admin_pool)
        .await
        .expect("acquire pgcrypto advisory lock");
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&admin_pool)
        .await
        .expect("install pgcrypto extension");
    sqlx::query("SELECT pg_advisory_unlock(3723742987654321)")
        .execute(&admin_pool)
        .await
        .expect("release pgcrypto advisory lock");

    let create_sql = format!("CREATE SCHEMA \"{schema}\"");
    sqlx::query(sqlx::AssertSqlSafe(create_sql))
        .execute(&admin_pool)
        .await
        .expect("create schema");
    admin_pool.close().await;

    let url_with_search_path = format!("{url}?options=-c%20search_path%3D{schema}");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url_with_search_path)
        .await
        .expect("connect with search_path");

    // Apply 0001–0007: establishes the legacy negative-blocklist search_tsv.
    pool.execute(MIGRATION_0001_SQL)
        .await
        .expect("apply 0001 migration");
    pool.execute(MIGRATION_0002_SQL)
        .await
        .expect("apply 0002 migration");
    pool.execute(MIGRATION_0003_SQL)
        .await
        .expect("apply 0003 migration");
    pool.execute(MIGRATION_0004_SQL)
        .await
        .expect("apply 0004 migration");
    pool.execute(MIGRATION_0005_SQL)
        .await
        .expect("apply 0005 migration");
    pool.execute(MIGRATION_0006_SQL)
        .await
        .expect("apply 0006 migration");
    pool.execute(MIGRATION_0007_SQL)
        .await
        .expect("apply 0007 migration");

    // Insert a dummy community + event so the table is non-empty.
    // 0008 checks `IF NOT EXISTS (SELECT 1 FROM events LIMIT 1)` and SKIPS
    // on non-empty tables, preserving the legacy blocklist expression.
    let dummy_community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host, signing_key) VALUES ($1, $2, $3)")
        .bind(dummy_community_id)
        .bind("legacy-seed.example")
        .bind(b"signingkey".as_slice())
        .execute(&pool)
        .await
        .expect("insert seed community");
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
         VALUES ($1, $2, $3, to_timestamp($4), $5, '[]'::jsonb, $6, $7)",
    )
    .bind(dummy_community_id)
    .bind(&rand_bytes32()[..])
    .bind(&rand_bytes32()[..])
    .bind(1_600_000_000_i64)
    .bind(9_i32)
    .bind("seed event for non-empty table")
    .bind(b"signature".as_slice())
    .execute(&pool)
    .await
    .expect("insert seed event");

    // Apply 0008: skips because table is non-empty → legacy blocklist preserved.
    pool.execute(MIGRATION_0008_SQL)
        .await
        .expect("apply 0008 migration");
    // Apply 0009–0011: NIP-RS guards, no FTS changes.
    pool.execute(MIGRATION_0009_SQL)
        .await
        .expect("apply 0009 migration");
    pool.execute(MIGRATION_0010_SQL)
        .await
        .expect("apply 0010 migration");
    pool.execute(MIGRATION_0011_SQL)
        .await
        .expect("apply 0011 migration");

    (pool, schema)
}

/// Behavioral test for migration 0012's legacy-blocklist rewrite branch.
///
/// This is the operationally risky path: populated production databases still
/// on the pre-#1771 negative blocklist need 0012 to DROP + re-ADD `search_tsv`
/// with kind 31234 added to the exclusion list. The fresh-install (allowlist)
/// path is covered by the existing tests via `setup()`.
///
/// The test constructs the legacy state, seeds control and draft rows, applies
/// 0012, and asserts:
///   1. 0012 took the REWRITE branch (the live expression now contains 31234).
///   2. The GIN index was rebuilt.
///   3. The kind:9 control row remains searchable via `search_tsv`.
///   4. The kind:31234 draft row's `search_tsv` IS NULL (storage-level unsearchable).
///   5. The discriminator correctly classifies both forms: the post-rewrite
///      expression remains blocklist-shaped (not ELSE NULL).
///
/// Bite-proof: temporarily commenting out the DROP+re-ADD in 0012's legacy
/// branch causes assertions 1 and 4 to fail (31234 not in exclusion list,
/// draft row's `search_tsv` is non-NULL).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_0012_rewrites_legacy_blocklist_to_exclude_drafts() {
    let (pool, schema) = setup_legacy_blocklist().await;

    // Verify we have the legacy blocklist expression BEFORE applying 0012.
    let pre_expr: String = sqlx::query_scalar(
        "SELECT pg_get_expr(adbin, adrelid) FROM pg_attrdef \
         WHERE adrelid = 'events'::regclass \
           AND adnum = (SELECT attnum FROM pg_attribute \
                        WHERE attrelid = 'events'::regclass AND attname = 'search_tsv')",
    )
    .fetch_one(&pool)
    .await
    .expect("read pre-0012 search_tsv expression");

    // Legacy blocklist: THEN NULL (not ELSE NULL) — 0012 must NOT see this as allowlist.
    assert!(
        !pre_expr.contains("ELSE NULL"),
        "pre-0012 expression should be legacy blocklist (THEN NULL, not ELSE NULL), got: {pre_expr}"
    );
    assert!(
        !pre_expr.contains("31234"),
        "pre-0012 expression should not already exclude 31234, got: {pre_expr}"
    );

    // Seed test rows: a kind:9 control and a kind:31234 draft.
    let community_id: Uuid =
        sqlx::query_scalar("SELECT id FROM communities WHERE host = 'legacy-seed.example'")
            .fetch_one(&pool)
            .await
            .expect("fetch seed community");
    let community = CommunityId::from_uuid(community_id);

    let control_id = rand_bytes32();
    let draft_id = rand_bytes32();
    let token = "legacy_rewrite_marker_unique";

    insert_event(
        &pool,
        community,
        control_id,
        rand_bytes32(),
        9,
        &format!("public chat — {token}"),
        None,
        1_700_000_000,
    )
    .await;
    insert_event(
        &pool,
        community,
        draft_id,
        rand_bytes32(),
        KIND_DRAFT as i32,
        &format!("draft wrap — {token}"),
        None,
        1_700_000_001,
    )
    .await;

    // Before 0012: the draft row IS searchable under the legacy blocklist
    // (31234 is not in the exclusion list).
    let pre_tsv: Option<String> =
        sqlx::query_scalar("SELECT search_tsv::text FROM events WHERE id = $1")
            .bind(&draft_id[..])
            .fetch_one(&pool)
            .await
            .expect("read pre-0012 draft search_tsv");
    assert!(
        pre_tsv.is_some(),
        "before 0012, kind:31234 should have non-NULL search_tsv under legacy blocklist"
    );

    // Apply 0012: should take the rewrite branch.
    pool.execute(MIGRATION_0012_SQL)
        .await
        .expect("apply 0012 migration");

    // 1. Verify 0012 took the REWRITE branch: expression now contains 31234.
    let post_expr: String = sqlx::query_scalar(
        "SELECT pg_get_expr(adbin, adrelid) FROM pg_attrdef \
         WHERE adrelid = 'events'::regclass \
           AND adnum = (SELECT attnum FROM pg_attribute \
                        WHERE attrelid = 'events'::regclass AND attname = 'search_tsv')",
    )
    .fetch_one(&pool)
    .await
    .expect("read post-0012 search_tsv expression");
    assert!(
        post_expr.contains("31234"),
        "after 0012, legacy-rewrite expression must include 31234, got: {post_expr}"
    );

    // 2. Verify GIN index was rebuilt.
    let index_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = 'events' \
         AND indexname = 'idx_events_search_tsv')",
    )
    .fetch_one(&pool)
    .await
    .expect("check GIN index existence");
    assert!(
        index_exists,
        "GIN index idx_events_search_tsv must exist after rewrite"
    );

    // 3. Control row (kind:9) remains searchable.
    let svc = SearchService::new(pool.clone());
    let result = svc
        .search(&SearchQuery {
            community,
            q: token.into(),
            channel_scope: ChannelScope::Any,
            kinds: None,
            authors: None,
            since: None,
            until: None,
            page: 1,
            per_page: 10,
            mode: buzz_search::SearchMode::FullText,
        })
        .await
        .expect("search ok");
    let kinds: Vec<i32> = result.hits.iter().map(|h| h.kind).collect();
    assert!(
        kinds.contains(&9),
        "kind:9 control row MUST remain searchable after legacy rewrite, got kinds={kinds:?}"
    );

    // 4. Draft row's search_tsv is now NULL (storage-level unsearchable).
    let post_tsv: Option<String> =
        sqlx::query_scalar("SELECT search_tsv::text FROM events WHERE id = $1")
            .bind(&draft_id[..])
            .fetch_one(&pool)
            .await
            .expect("read post-0012 draft search_tsv");
    assert!(
        post_tsv.is_none(),
        "after 0012, kind:31234 draft row's search_tsv MUST be NULL, got: {post_tsv:?}"
    );
    assert!(
        !kinds.contains(&(KIND_DRAFT as i32)),
        "kind:31234 MUST NOT appear in search results after legacy rewrite, got kinds={kinds:?}"
    );

    // 5. Verify discriminator correctness: the post-rewrite expression is still
    //    blocklist-shaped (THEN NULL, not ELSE NULL) — 0012 extended it, didn't
    //    flip to allowlist. And the allowlist form (from a fresh setup) has ELSE NULL.
    assert!(
        !post_expr.contains("ELSE NULL"),
        "post-rewrite expression should remain blocklist-shaped (not allowlist ELSE NULL), \
         got: {post_expr}"
    );

    teardown(pool, &schema).await;
}

/// Companion to `migration_0012_rewrites_legacy_blocklist_to_exclude_drafts`:
/// verifies that 0012 correctly no-ops on the fresh-install allowlist form
/// and that the discriminator classifies it as allowlist (ELSE NULL shape).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_0012_noops_on_fresh_install_allowlist() {
    let (pool, schema) = setup().await;

    let expr: String = sqlx::query_scalar(
        "SELECT pg_get_expr(adbin, adrelid) FROM pg_attrdef \
         WHERE adrelid = 'events'::regclass \
           AND adnum = (SELECT attnum FROM pg_attribute \
                        WHERE attrelid = 'events'::regclass AND attname = 'search_tsv')",
    )
    .fetch_one(&pool)
    .await
    .expect("read search_tsv expression");

    // The allowlist expression (from 0008) must have the ELSE NULL shape
    // that 0012's discriminator detects.
    assert!(
        expr.contains("ELSE NULL"),
        "fresh-install expression must contain ELSE NULL (allowlist shape), got: {expr}"
    );
    // 31234 should NOT appear in the expression — it's excluded by omission.
    assert!(
        !expr.contains("31234"),
        "fresh-install allowlist should not mention 31234, got: {expr}"
    );

    teardown(pool, &schema).await;
}
