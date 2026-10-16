//! Contract-migration coverage; application insert coverage belongs to phase 1.

use super::*;

async fn floor_objects(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM pg_trigger WHERE tgname = 'events_created_at_floor'), \
                (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = 'public' AND p.proname = 'events_created_at_floor_guard')",
    )
    .fetch_one(pool)
    .await
    .expect("read floor catalog")
}

async fn retained_guards(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT c.relname, pg_get_triggerdef(t.oid) FROM pg_trigger t \
         JOIN pg_class c ON c.oid = t.tgrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND t.tgname <> 'events_created_at_floor' \
         UNION ALL \
         SELECT c.relname, pg_get_constraintdef(k.oid) FROM pg_constraint k \
         JOIN pg_class c ON c.oid = k.conrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND k.contype = 'f' \
         ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .expect("read retained triggers and foreign keys")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_floor_retirement_drops_parent_and_partition_clones() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect isolated migration database");
    crate::migration::run_migrations_through(&pool, 60)
        .await
        .expect("apply pre-contract schema");
    let partitions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_partition_tree('events'::regclass)")
            .fetch_one(&pool)
            .await
            .expect("count parent and partitions");
    assert_eq!(floor_objects(&pool).await, (partitions, 1));
    let before = retained_guards(&pool).await;
    crate::migration::run_migrations(&pool)
        .await
        .expect("apply contract migration and retained startup checks");
    assert_eq!(floor_objects(&pool).await, (0, 0));
    assert_eq!(retained_guards(&pool).await, before);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_floor_retirement_cleans_child_copies_idempotently() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect isolated migration database");
    crate::migration::run_migrations_through(&pool, 60)
        .await
        .expect("apply pre-contract schema");
    // Model independently attached copies after the parent guard was removed.
    sqlx::raw_sql(
        "DROP TRIGGER events_created_at_floor ON events; \
         CREATE CONSTRAINT TRIGGER events_created_at_floor \
         AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p_past \
         DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
         EXECUTE FUNCTION events_created_at_floor_guard();",
    )
    .execute(&pool)
    .await
    .expect("install independent child copy");
    assert_eq!(floor_objects(&pool).await, (1, 1));
    let before = retained_guards(&pool).await;
    crate::migration::run_migrations(&pool)
        .await
        .expect("retire independent copy");
    assert_eq!(floor_objects(&pool).await, (0, 0));
    let mut tx = pool.begin().await.expect("begin idempotence check");
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/0062_retire_created_at_floor.sql"
    ))
    .execute(&mut *tx)
    .await
    .expect("repeat contract SQL after retirement");
    tx.commit().await.expect("commit repeated retirement");
    assert_eq!(floor_objects(&pool).await, (0, 0));
    assert_eq!(retained_guards(&pool).await, before);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_retirement_desired_state_has_no_floor_objects() {
    // The nextest wrapper bootstraps this database through real pgschema apply
    // and reconciliation; desired-state/migration parity is checked by
    // `admin_schema_parity_between_desired_state_and_migrations`.
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect desired-state database");
    assert_eq!(floor_objects(&pool).await, (0, 0));
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_retirement_writer_and_probe_run_on_retired_schema() {
    let url = crate::test_support::database_url();
    let db = Db::new(&DbConfig {
        database_url: url.clone(),
        read_database_url: Some(url),
        max_connections: 2,
        min_connections: 1,
        ..DbConfig::default()
    })
    .await
    .expect("connect production writer with retired schema");
    let armed: Option<String> =
        sqlx::query_scalar("SELECT nullif(current_setting('buzz.created_at_floor', true), '')")
            .fetch_one(&db.pool)
            .await
            .expect("inspect writer session setting");
    assert_eq!(armed, None, "writer must no longer arm the retired guard");
    assert_eq!(floor_objects(&db.pool).await, (0, 0));
    assert!(db.spawn_fence_probe());
    crate::replica_fence::probe_once(&db.pool, db.fence())
        .await
        .expect("retired schema still supports the ordered heartbeat");

    db.pool.close().await;
    db.read_pool
        .as_ref()
        .expect("reader configured")
        .close()
        .await;
}
