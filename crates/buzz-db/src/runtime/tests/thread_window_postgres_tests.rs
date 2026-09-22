use super::*;
use crate::thread_window::{AuxBudget, AuxQuery};
use buzz_core::thread_window::Request;
use nostr::{EventBuilder, Kind, Tag, Timestamp};

fn request(channel: Uuid, root: &nostr::Event, upper: u64) -> Request {
    Request::parse(&serde_json::json!({"thread_window":true,"#h":[channel],
        "#e":[root.id.to_hex()],"kinds":[9],"limit":50,
        "until":upper,"before_id":"00".repeat(32)}))
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_upper_fence_terminal_snapshot_and_fallback() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (writer, wname) = create_scratch_db(&admin, "tw_writer").await;
    let (replica, rname) = create_scratch_db(&admin, "tw_replica").await;
    let keys = nostr::Keys::generate();
    let cid = Uuid::new_v4();
    let channel = Uuid::new_v4();
    let base = 1_700_000_000;
    let root = signed_event_at(&keys, "root", base);
    let old = signed_event_at(&keys, "old", base + 10);
    let middle = signed_event_at(&keys, "missing-middle", base + 20);
    for pool in [&writer, &replica] {
        seed_community_channel(pool, cid, channel, &keys).await;
        insert_top_level(pool, cid, channel, &root).await;
        insert_thread_reply(pool, cid, channel, &root, &old).await;
    }
    insert_thread_reply(&writer, cid, channel, &root, &middle).await;
    let db = Db::from_pools(writer.clone(), replica.clone());
    let community = CommunityId::from_uuid(cid);
    let mut req = request(channel, &root, base + 30);
    // An old delivered tail does not justify an uncovered request upper bound.
    db.fence()
        .force_open_for_tests(chrono::DateTime::from_timestamp(base as i64 + 15, 0).unwrap());
    let (page, session) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert!(!session.is_replica());
    assert_eq!(
        page.rows.iter().map(|e| e.event.id).collect::<Vec<_>>(),
        [middle.id, old.id]
    );
    assert!(!page.has_more);
    drop(session);
    // Counterfactual: over-claiming coverage demonstrably loses the middle row.
    db.fence().force_open_for_tests(chrono::Utc::now());
    let (page, session) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert!(session.is_replica());
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].event.id, old.id);
    assert!(!page.has_more && page.next_cursor.is_none());
    drop(session);
    // Default head route remains writer despite an open fence.
    req.cursor = None;
    let (head, session) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert!(!session.is_replica());
    assert_eq!(head.rows[0].event.id, middle.id);
    drop(session);

    // Complete the reply fixture, then lag *recent* edits and channel-less
    // deletions. Coverage of old replies is not a freshness proof for aux.
    insert_thread_reply(&replica, cid, channel, &root, &middle).await;
    req = request(channel, &root, base + 30);
    let (_, mut session) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert!(session.is_replica());
    let edit = EventBuilder::new(Kind::Custom(40003), "new edit")
        .tags([Tag::parse(["e", &old.id.to_hex()]).unwrap()])
        .custom_created_at(Timestamp::from(base + 1000))
        .sign_with_keys(&keys)
        .unwrap();
    let deletion = EventBuilder::new(Kind::Custom(5), "")
        .tags([Tag::parse(["e", &edit.id.to_hex()]).unwrap()])
        .custom_created_at(Timestamp::from(base + 1001))
        .sign_with_keys(&keys)
        .unwrap();
    for pool in [&writer, &replica] {
        event::insert_event(pool, community, &edit, Some(channel))
            .await
            .unwrap();
        event::insert_event(pool, community, &deletion, None)
            .await
            .unwrap();
    }
    let targets = [old.id.to_hex(), edit.id.to_hex()];
    let query = AuxQuery {
        community,
        targets: &targets,
        kinds: &[5, 40003],
        accessible: &[channel],
        cursor: None,
    };
    let mut budget = AuxBudget::default();
    let stale = session
        .thread_window_aux(&query, &mut budget)
        .await
        .unwrap();
    assert!(
        stale.events.is_empty(),
        "held snapshot must not advance after page proof"
    );
    let (_, mut fresh) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert_eq!(
        fresh
            .thread_window_aux(&query, &mut budget)
            .await
            .unwrap()
            .events
            .len(),
        2,
        "control: recent aux is visible on a fresh snapshot, with no reply timestamp bound"
    );
    drop(fresh);
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1 AND pid<>pg_backend_pid()")
        .bind(&rname).execute(&admin).await.unwrap();
    let degraded = session
        .thread_window_aux(&query, &mut budget)
        .await
        .unwrap();
    assert_eq!(degraded.events.len(), 2);
    assert!(
        !session.is_replica(),
        "fallback must permanently release the failed snapshot"
    );
    drop(session);
    // The shared router's reader acquisition failure must also reach the writer,
    // not reinterpret an unavailable reader as an empty terminal window.
    replica.close().await;
    let (page, session) = db
        .get_thread_window_with_session(community, &req)
        .await
        .unwrap();
    assert!(!session.is_replica());
    assert_eq!(page.rows.len(), 2);
    assert!(!page.has_more);
    drop(session);
    drop_scratch_db(&admin, replica, &rname).await;
    drop_scratch_db(&admin, writer, &wname).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_thread_window_prebuild_validation_and_old_ledger() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "tw_prebuild", Some(47)).await;
    for setup in [
        "CREATE INDEX idx_thread_metadata_window ON thread_metadata (community_id,root_event_id,event_created_at ASC,event_id ASC)",
        // Simulate an invalid concurrent-build remnant only in this disposable superuser DB.
        "CREATE INDEX idx_thread_metadata_window ON thread_metadata (community_id,root_event_id,event_created_at DESC,event_id ASC); \
         UPDATE pg_index SET indisvalid=false WHERE indexrelid='idx_thread_metadata_window'::regclass",
    ] {
        sqlx::raw_sql(setup).execute(&pool).await.unwrap();
        let error = migration::run_migrations(&pool).await.unwrap_err();
        assert!(error.to_string().contains("invalid or wrong definition"),
            "must reject catalog shape, not merely time out: {error}");
        let version: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(version, 47);
        sqlx::query("DROP INDEX CONCURRENTLY idx_thread_metadata_window").execute(&pool).await.unwrap();
    }
    sqlx::query("CREATE INDEX CONCURRENTLY idx_thread_metadata_window ON thread_metadata (community_id,root_event_id,event_created_at DESC,event_id ASC)")
        .execute(&pool).await.unwrap();
    let oid: i64 = sqlx::query_scalar("SELECT 'idx_thread_metadata_window'::regclass::oid::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    migration::run_migrations(&pool).await.unwrap();
    assert_eq!(
        oid,
        sqlx::query_scalar::<_, i64>("SELECT 'idx_thread_metadata_window'::regclass::oid::bigint")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    // Model the previous binary's exact embedded ledger, not run_to(47),
    // which would still know version 49 and cannot test VersionMissing.
    let current = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
    )
    .await
    .unwrap();
    let old = sqlx::migrate::Migrator::with_migrations(
        current
            .iter()
            .filter(|m| m.version <= 47)
            .cloned()
            .collect(),
    );
    assert!(matches!(
        old.run(&pool).await,
        Err(sqlx::migrate::MigrateError::VersionMissing(48))
    ));
    drop_scratch_db(&admin, pool, &name).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_thread_window_prebuild_does_not_queue_behind_writer() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "tw_prebuild_writer", Some(47)).await;
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&pool, community, channel, &nostr::Keys::generate()).await;
    sqlx::query("CREATE INDEX CONCURRENTLY idx_thread_metadata_window ON thread_metadata (community_id,root_event_id,event_created_at DESC,event_id ASC)")
        .execute(&pool).await.unwrap();
    let oid: i64 = sqlx::query_scalar("SELECT 'idx_thread_metadata_window'::regclass::oid::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Model an ingestion transaction already holding the table's writer lock.
    // It stays held through every control: releasing it would hide the convoy.
    let mut writer = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE thread_metadata IN ROW EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let insert =
        "INSERT INTO thread_metadata (community_id, channel_id, event_created_at, event_id) \
                  VALUES ($1, $2, now(), $3)";
    let mut witness = pool.acquire().await.unwrap();
    sqlx::query("SET lock_timeout = '200ms'")
        .execute(&mut *witness)
        .await
        .unwrap();
    sqlx::query(insert)
        .bind(community)
        .bind(channel)
        .bind(vec![1_u8; 32])
        .execute(&mut *witness)
        .await
        .unwrap();

    // Keep the exact migration transaction open after its SQL finishes so a
    // second insert observes *all* locks it acquired. This is an explicit
    // overlap barrier, not a race against a fast catalog-only migration.
    let mut migration_tx = pool.begin().await.unwrap();
    let result = sqlx::raw_sql(include_str!(
        "../../../../../migrations/0048_thread_window_index.sql"
    ))
    .execute(&mut *migration_tx)
    .await;
    let concurrent_insert = sqlx::query(insert)
        .bind(community)
        .bind(channel)
        .bind(vec![2_u8; 32])
        .execute(&mut *witness)
        .await;
    migration_tx.rollback().await.unwrap();
    sqlx::query(insert)
        .bind(community)
        .bind(channel)
        .bind(vec![3_u8; 32])
        .execute(&mut *witness)
        .await
        .unwrap();

    // Also bind the public migrator: the advisory schema/destruction lock,
    // ledger write and post-migration catalog checks must work with ingestion.
    let production_result = migration::run_migrations(&pool).await;
    sqlx::query(insert)
        .bind(community)
        .bind(channel)
        .bind(vec![4_u8; 32])
        .execute(&mut *witness)
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&mut *witness)
        .await
        .unwrap();
    let final_oid: i64 =
        sqlx::query_scalar("SELECT 'idx_thread_metadata_window'::regclass::oid::bigint")
            .fetch_one(&mut *witness)
            .await
            .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM thread_metadata WHERE community_id=$1")
            .bind(community)
            .fetch_one(&mut *witness)
            .await
            .unwrap();
    drop(witness);
    writer.rollback().await.unwrap();
    drop_scratch_db(&admin, pool, &name).await;

    assert!(
        result.is_ok(),
        "valid prebuild must bypass write-conflicting CREATE: {result:?}"
    );
    assert!(
        concurrent_insert.is_ok(),
        "migration must allow metadata inserts: {concurrent_insert:?}"
    );
    assert!(
        production_result.is_ok(),
        "production migrator must preserve ingestion progress: {production_result:?}"
    );
    assert_eq!(version, 48);
    assert_eq!(final_oid, oid, "prebuild must not be replaced");
    assert_eq!(count, 4, "all writer witnesses must persist");
}
