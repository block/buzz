//! The supersession marker adds metadata, not inferred historical authority.
use sqlx::{postgres::PgPoolOptions, Executor};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn supersession_fast_default_preserves_partition_storage_and_denies_unknown_history() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("isolated test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    let mut tx = pool.begin().await.expect("transaction");
    let schema = format!("supersession_{}", uuid::Uuid::new_v4().simple());
    tx.execute(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path = {schema}"
    )))
    .await
    .expect("isolated schema");
    tx.execute(
        "CREATE TABLE events (id int, deleted_at timestamptz) PARTITION BY RANGE (id);
         CREATE TABLE events_old PARTITION OF events FOR VALUES FROM (0) TO (10);
         CREATE INDEX events_id ON events (id);
         INSERT INTO events VALUES (1, NULL), (2, now());
         CREATE TABLE original_storage AS
         SELECT oid, relfilenode FROM pg_class WHERE relnamespace = current_schema()::regnamespace
         AND relkind IN ('r', 'i');",
    )
    .await
    .expect("populated partition and index");
    tx.execute(include_str!(
        "../../../migrations/0044_workflow_superseded_authority.sql"
    ))
    .await
    .expect("supersession migration");
    let unchanged: bool = sqlx::query_scalar(
        "SELECT bool_and(c.relfilenode = o.relfilenode) FROM original_storage o JOIN pg_class c USING (oid)"
    ).fetch_one(&mut *tx).await.expect("physical storage");
    assert!(unchanged);
    let unknown_denied: bool =
        sqlx::query_scalar("SELECT bool_and(workflow_revision_superseded = false) FROM events")
            .fetch_one(&mut *tx)
            .await
            .expect("no historical inference");
    assert!(unknown_denied);
    let fast_default: bool = sqlx::query_scalar(
        "SELECT atthasmissing AND attmissingval::text = '{f}' FROM pg_attribute
         WHERE attrelid = 'events_old'::regclass AND attname = 'workflow_revision_superseded'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("fast default catalog");
    assert!(
        fast_default,
        "existing partition rows must use a metadata-only default"
    );
    tx.execute(
        "CREATE TABLE events_future PARTITION OF events FOR VALUES FROM (10) TO (20);
         INSERT INTO events VALUES (11, NULL);",
    )
    .await
    .expect("future partition inherits default");
    let new_default: bool =
        sqlx::query_scalar("SELECT NOT workflow_revision_superseded FROM events WHERE id=11")
            .fetch_one(&mut *tx)
            .await
            .expect("new row default");
    assert!(new_default);
    tx.rollback().await.expect("cleanup");
}
