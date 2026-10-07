use super::*;

const SCOPE_INDEX: &str = "CREATE INDEX CONCURRENTLY idx_event_mentions_scope_received \
     ON event_mentions (community_id,pubkey_hex,channel_id,root_id,received_at)";
const SCOPE_COLUMNS: &str = "ALTER TABLE event_mentions \
     ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ, ADD COLUMN IF NOT EXISTS root_id BYTEA";

async fn regclass_oid(pool: &PgPool, name: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT to_regclass($1)::oid::bigint")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn migrated_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The documented brownfield path for 0057: prebuild the event_mentions
/// index concurrently and pre-drop the root index. The migration must keep
/// the prebuilt index, reject a wrong or invalid one without recording the
/// version, and bound its budgets like 0049.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_personal_read_prebuild_validation() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "pr_prebuild", Some(56)).await;
    sqlx::raw_sql(SCOPE_COLUMNS).execute(&pool).await.unwrap();
    for setup in [
        "CREATE INDEX idx_event_mentions_scope_received \
         ON event_mentions (community_id,pubkey_hex,channel_id,received_at)",
        // Simulate an invalid concurrent-build remnant only in this disposable superuser DB.
        "CREATE INDEX idx_event_mentions_scope_received \
         ON event_mentions (community_id,pubkey_hex,channel_id,root_id,received_at); \
         UPDATE pg_index SET indisvalid=false \
         WHERE indexrelid='idx_event_mentions_scope_received'::regclass",
    ] {
        sqlx::raw_sql(setup).execute(&pool).await.unwrap();
        let error = migration::run_migrations(&pool).await.unwrap_err();
        assert!(
            error.to_string().contains("invalid or wrong definition"),
            "must reject catalog shape, not merely time out: {error}"
        );
        assert_eq!(migrated_version(&pool).await, 56);
        sqlx::query("DROP INDEX CONCURRENTLY idx_event_mentions_scope_received")
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::query(SCOPE_INDEX).execute(&pool).await.unwrap();
    sqlx::query("DROP INDEX CONCURRENTLY IF EXISTS public.idx_thread_metadata_root")
        .execute(&pool)
        .await
        .unwrap();
    let oid = regclass_oid(&pool, "idx_event_mentions_scope_received").await;
    assert!(oid.is_some());
    migration::run_migrations(&pool).await.unwrap();
    assert_eq!(
        oid,
        regclass_oid(&pool, "idx_event_mentions_scope_received").await
    );
    assert_eq!(regclass_oid(&pool, "idx_thread_metadata_root").await, None);
    assert!(migrated_version(&pool).await >= 57);
    drop_scratch_db(&admin, pool, &name).await;
}

/// Without a prebuild, 0057 builds the index and drops the root index itself.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_personal_read_fresh_build_and_budgets() {
    let migration = include_str!("../../../../../migrations/0057_personal_read_scopes.sql");
    assert!(migration.contains("SET LOCAL lock_timeout = '1s'"));
    assert!(migration.contains("SET LOCAL statement_timeout = '5s'"));

    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "pr_fresh", Some(56)).await;
    assert!(regclass_oid(&pool, "idx_thread_metadata_root")
        .await
        .is_some());
    migration::run_migrations(&pool).await.unwrap();
    assert!(regclass_oid(&pool, "idx_event_mentions_scope_received")
        .await
        .is_some());
    assert_eq!(regclass_oid(&pool, "idx_thread_metadata_root").await, None);
    drop_scratch_db(&admin, pool, &name).await;
}
