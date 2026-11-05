//! Migration 0084 drops the unused `idx_events_community_deleted` from the
//! partitioned `events` table. PostgreSQL cannot drop a partitioned index
//! concurrently, so the migration takes ACCESS EXCLUSIVE on `events` and every
//! partition. Its lock budget must make it give up behind a long read, rather
//! than queue every writer behind it, and leave the database on 0083 for the
//! next boot to retry.

use super::*;

/// The last migration before the drop.
const BEFORE_DROP: i64 = 83;

async fn ledger_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .expect("read migration ledger")
}

/// The parent index, if present, and the partition indexes attached to it.
async fn index_presence(pool: &PgPool) -> (bool, i64) {
    sqlx::query_as(
        "SELECT to_regclass('public.idx_events_community_deleted') IS NOT NULL, \
                (SELECT count(*) FROM pg_inherits i \
                 JOIN pg_class p ON p.oid = i.inhparent \
                 WHERE p.relname = 'idx_events_community_deleted')",
    )
    .fetch_one(pool)
    .await
    .expect("look up index")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_0084_drops_the_index_within_its_lock_budget() {
    let (pool, _scratch) = ScratchDb::migrated_through(
        &admin_url().await,
        "events_deleted_drop",
        BEFORE_DROP,
    )
    .await;
    let keys = nostr::Keys::generate();
    let community = Uuid::new_v4();
    let channel = Uuid::new_v4();
    seed_community_channel(&pool, community, channel, &keys).await;
    let now = nostr::Timestamp::now().as_secs();
    insert_top_level(&pool, community, channel, &signed_event_at(&keys, "before", now)).await;
    let (parent, partitions) = index_presence(&pool).await;
    assert!(parent && partitions > 0, "0083 carries the partitioned index");

    // A long read holds ACCESS SHARE on `events` and its partitions.
    let mut reader = pool.begin().await.expect("begin reader");
    let _: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&mut *reader)
        .await
        .expect("long read");
    let started = Instant::now();
    let error = migration::run_migrations(&pool)
        .await
        .expect_err("the drop must not wait behind the read");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "gave up at the lock budget, not the statement budget: {:?}",
        started.elapsed()
    );
    assert!(error.to_string().contains("lock timeout"), "{error}");
    assert_eq!(ledger_version(&pool).await, BEFORE_DROP, "left for a retry");
    assert_eq!(index_presence(&pool).await, (parent, partitions));
    reader.rollback().await.expect("end reader");

    migration::run_migrations(&pool)
        .await
        .expect("the retry drops the index");
    assert_eq!(ledger_version(&pool).await, BEFORE_DROP + 1);
    assert_eq!(
        index_presence(&pool).await,
        (false, 0),
        "the parent and every partition index are gone"
    );
    let leftover: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes \
         WHERE schemaname = 'public' AND tablename LIKE 'events%' \
           AND indexdef LIKE '%(community_id, deleted_at)'",
    )
    .fetch_one(&pool)
    .await
    .expect("look for partition copies");
    assert_eq!(leftover, 0, "no partition keeps a detached copy");
    insert_top_level(&pool, community, channel, &signed_event_at(&keys, "after", now + 1)).await;
}
