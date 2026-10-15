//! Upgrade contract for push-only trigger retirement.
use super::postgres_tests::{connect_test_pool, reset_public_schema};
use super::{run_migrations, run_migrations_through};
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

    run_migrations(&pool).await.expect("retire push trigger");
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
    let community = uuid::Uuid::new_v4();
    let channel = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("retirement-{community}.example"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by, ttl_seconds, ttl_deadline) \
        VALUES ($1, $2, 'ttl', $3, 60, clock_timestamp() - interval '1 day')",
    )
    .bind(channel)
    .bind(community)
    .bind([11_u8; 32])
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id) \
        VALUES ($1, $2, $3, now(), 9, '[]', 'TTL survives push retirement', $4, $5)")
        .bind(community).bind([12_u8; 32]).bind([11_u8; 32]).bind([13_u8; 64]).bind(channel)
        .execute(&pool).await.unwrap();
    let refreshed: bool = sqlx::query_scalar(
        "SELECT ttl_deadline > clock_timestamp() FROM channels WHERE id=$1 AND community_id=$2",
    )
    .bind(channel)
    .bind(community)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        refreshed,
        "raw inserts still refresh TTL after push retirement"
    );
    pool.close().await;
}
