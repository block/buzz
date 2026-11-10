//! Upgrade contracts for the event follow-up trigger retirements: push
//! (0063), then channel TTL (0065).
use super::postgres_tests::{connect_test_pool, reset_public_schema};
use super::run_migrations_through;
use sqlx::PgPool;

async fn ttl_definitions(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT tgrelid::regclass::text, pg_get_triggerdef(oid) FROM pg_trigger \
         WHERE tgname = 'events_refresh_channel_ttl' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("read TTL triggers")
}

async fn push_trigger_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_trigger WHERE tgname = 'events_enqueue_push_match'")
        .fetch_one(pool)
        .await
        .expect("count push triggers")
}

/// Insert a channel with an expired TTL deadline, then a raw `INSERT INTO
/// events` on it that bypasses the application follow-up. Returns whether the
/// database itself pushed the deadline forward.
async fn raw_insert_refreshes_expired_ttl(pool: &PgPool, content: &str) -> bool {
    let community = uuid::Uuid::new_v4();
    let channel = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("retirement-{community}.example"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by, ttl_seconds, ttl_deadline) \
        VALUES ($1, $2, 'ttl', $3, 60, clock_timestamp() - interval '1 day')",
    )
    .bind(channel)
    .bind(community)
    .bind([11_u8; 32])
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
        VALUES ($1, $2, $3, now(), 9, '[]', $4, $5, $6)",
    )
    .bind(community)
    .bind([12_u8; 32])
    .bind([11_u8; 32])
    .bind(content)
    .bind([13_u8; 64])
    .bind(channel)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "SELECT ttl_deadline > clock_timestamp() FROM channels WHERE id=$1 AND community_id=$2",
    )
    .bind(channel)
    .bind(community)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_push_retirement_preserves_ttl_and_published_checksums() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations_through(&pool, 62)
        .await
        .expect("apply published migrations");
    assert!(
        push_trigger_count(&pool).await > 1,
        "parent and partition triggers exist before upgrade"
    );
    let ttl_before = ttl_definitions(&pool).await;
    assert!(ttl_before.len() > 1);
    let function_before: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('refresh_channel_ttl_after_event_insert()'::regprocedure)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let checksums_before: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 62 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    run_migrations_through(&pool, 63)
        .await
        .expect("retire push trigger");
    assert_eq!(
        push_trigger_count(&pool).await,
        0,
        "retired on parent and every partition"
    );
    let function_absent: bool =
        sqlx::query_scalar("SELECT to_regprocedure('enqueue_push_match_job()') IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(function_absent);
    assert_eq!(ttl_definitions(&pool).await, ttl_before);
    let function_after: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('refresh_channel_ttl_after_event_insert()'::regprocedure)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(function_after, function_before);
    let checksums_after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 62 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums_after, checksums_before);

    // A raw writer has no application TTL follow-up. The preserved deferred
    // trigger must still refresh its channel when the event commits.
    assert!(
        raw_insert_refreshes_expired_ttl(&pool, "TTL survives push retirement").await,
        "raw inserts still refresh TTL after push retirement"
    );
    pool.close().await;
}

async fn ttl_function_present(pool: &PgPool) -> bool {
    sqlx::query_scalar(
        "SELECT to_regprocedure('refresh_channel_ttl_after_event_insert()') IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .expect("look up TTL function")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_ttl_retirement_drops_every_partition_trigger_and_keeps_checksums() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations_through(&pool, 64)
        .await
        .expect("apply published migrations");
    assert!(
        ttl_definitions(&pool).await.len() > 1,
        "parent and partition TTL triggers exist before upgrade"
    );
    assert!(ttl_function_present(&pool).await);
    let checksums_before: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 64 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    run_migrations_through(&pool, 65)
        .await
        .expect("retire TTL trigger");
    assert!(
        ttl_definitions(&pool).await.is_empty(),
        "retired on parent and every partition"
    );
    assert!(!ttl_function_present(&pool).await);
    assert_eq!(push_trigger_count(&pool).await, 0);
    let checksums_after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 64 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums_after, checksums_before);

    // With both triggers gone, a raw writer that skips the application
    // follow-up no longer extends an ephemeral channel's deadline.
    assert!(
        !raw_insert_refreshes_expired_ttl(&pool, "TTL no longer refreshed by the database").await,
        "raw inserts must not refresh TTL after 0065"
    );
    pool.close().await;
}
