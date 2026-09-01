//! Storage/rollout contract for the workflow wake FTS migration on PostgreSQL 17.
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use uuid::Uuid;

const MIGRATION: &str = include_str!("../../../migrations/0044_workflow_mention_wake_fts.sql");
const ALLOWLIST: &str = "CASE WHEN kind IN (0,9,40002,45001,45003) THEN to_tsvector('simple',content) ELSE NULL::tsvector END";
const DESIRED: &str = "CASE WHEN kind IN (1059,30179,30300,30350,30622,44100,44101,44200,44620) THEN NULL::tsvector ELSE to_tsvector('simple',content) END";

async fn fixture(expression: &str) -> (PgPool, String) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("isolated PostgreSQL URL");
    let schema = format!("wake_fts_{}", Uuid::new_v4().simple());
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    pool.execute(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET search_path = {schema};
         CREATE TABLE events(kind int, content text, created_at int,
           search_tsv tsvector GENERATED ALWAYS AS ({expression}) STORED)
           PARTITION BY RANGE(created_at);
         CREATE TABLE events_old PARTITION OF events FOR VALUES FROM (0) TO (100);
         CREATE INDEX custom_fts_index ON events USING gin(search_tsv) WITH (fastupdate=off);
         INSERT INTO events(kind,content,created_at) VALUES
           (9,'public control',1),(44620,'private payload',2),(44620,'',3);
         CREATE TEMP TABLE original_nodes AS SELECT oid, relname, relfilenode FROM pg_class
           WHERE relnamespace = '{schema}'::regnamespace;
         CREATE TEMP TABLE original_indexes AS SELECT c.relname, pg_get_indexdef(c.oid) AS definition, c.reloptions
           FROM pg_class c WHERE c.relnamespace = '{schema}'::regnamespace AND c.relkind IN ('i','I');"
    )))
    .await
    .expect("fixture schema");
    (pool, schema)
}

async fn cleanup(pool: PgPool, schema: String) {
    pool.execute(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .await
        .expect("cleanup");
    pool.close().await;
}

async fn assert_null_and_future_partition(pool: &PgPool) {
    pool.execute(
        "CREATE TABLE events_future PARTITION OF events FOR VALUES FROM (100) TO (200);
        INSERT INTO events_future(kind,content,created_at) VALUES (44620,'future private',101);
        UPDATE events SET content='changed private' WHERE kind=44620;
        INSERT INTO events(kind,content,created_at) VALUES (9,'was public',102);
        UPDATE events SET kind=44620 WHERE created_at=102;",
    )
    .await
    .expect("parent and direct leaf writes");
    let nonnull: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE kind=44620 AND search_tsv IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .expect("raw vectors");
    assert_eq!(nonnull, 0);
    let matches: Vec<i32> = sqlx::query_scalar(
        "SELECT kind FROM events WHERE search_tsv @@ websearch_to_tsquery('simple','-absentword')",
    )
    .fetch_all(pool)
    .await
    .expect("NOT-only query");
    assert_eq!(matches, vec![9]);
    assert!(
        pool.execute("UPDATE events SET search_tsv=to_tsvector('private') WHERE kind=44620")
            .await
            .is_err(),
        "generated column must reject direct vector assignments"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn safe_policies_preserve_heap_and_index_files() {
    let migrated = format!("CASE WHEN kind=30179 THEN NULL::tsvector ELSE (CASE WHEN kind=30350 THEN NULL::tsvector ELSE ({ALLOWLIST}) END) END");
    for expression in [ALLOWLIST, &migrated, DESIRED] {
        let (pool, schema) = fixture(expression).await;
        pool.execute(MIGRATION).await.expect("safe migration");
        let changed: i64 = sqlx::query_scalar("SELECT count(*) FROM original_nodes b JOIN pg_class c USING(oid) WHERE b.relfilenode <> c.relfilenode")
            .fetch_one(&pool).await.expect("physical files");
        assert_eq!(changed, 0, "safe policy must not rewrite heaps or indexes");
        assert_null_and_future_partition(&pool).await;
        cleanup(pool, schema).await;
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_policy_preserves_column_dependencies_and_nonwake_values() {
    let (pool, schema) = fixture("to_tsvector('simple',content)").await;
    pool.execute("CREATE VIEW public_projection AS SELECT search_tsv FROM events WHERE kind=9;
        CREATE TEMP TABLE original_projection AS SELECT created_at,content,search_tsv FROM events WHERE kind<>44620;")
        .await.expect("dependent view");
    pool.execute(MIGRATION).await.expect("legacy migration");
    let differences: i64 = sqlx::query_scalar("SELECT count(*) FROM original_indexes b FULL JOIN (SELECT c.relname, pg_get_indexdef(c.oid) AS definition, c.reloptions FROM pg_class c WHERE c.relnamespace=current_schema()::regnamespace AND c.relkind IN ('i','I')) a USING(relname) WHERE (b.definition,b.reloptions) IS DISTINCT FROM (a.definition,a.reloptions)")
        .fetch_one(&pool).await.expect("custom index definitions");
    assert_eq!(
        differences, 0,
        "custom index definitions and options must survive"
    );
    let changed: i64 = sqlx::query_scalar("SELECT count(*) FROM original_nodes b JOIN pg_class c ON c.relname=b.relname AND c.relnamespace=current_schema()::regnamespace WHERE b.relfilenode <> c.relfilenode")
        .fetch_one(&pool).await.expect("physical files");
    assert!(changed > 0, "legacy correction honestly requires a rewrite");
    let differences: i64 = sqlx::query_scalar("SELECT count(*) FROM original_projection o JOIN events e USING(created_at) WHERE (o.content,o.search_tsv) IS DISTINCT FROM (e.content,e.search_tsv)")
        .fetch_one(&pool).await.expect("nonwake values");
    assert_eq!(differences, 0);
    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public_projection WHERE search_tsv IS NOT NULL")
            .fetch_one(&pool)
            .await
            .expect("dependent view still works");
    assert_eq!(visible, 1);
    assert_null_and_future_partition(&pool).await;
    cleanup(pool, schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn divergent_partition_policy_fails_without_mutation() {
    let (pool, schema) = fixture(ALLOWLIST).await;
    pool.execute("DROP INDEX custom_fts_index; ALTER TABLE events_old ALTER COLUMN search_tsv SET EXPRESSION AS (to_tsvector('simple',content));")
        .await.expect("divergent leaf");
    let before: String = sqlx::query_scalar("SELECT pg_get_expr(adbin,adrelid) FROM pg_attrdef d JOIN pg_attribute a ON a.attrelid=d.adrelid AND a.attnum=d.adnum WHERE a.attrelid='events_old'::regclass AND a.attname='search_tsv'")
        .fetch_one(&pool).await.expect("leaf expression");
    let error = pool
        .execute(MIGRATION)
        .await
        .expect_err("reject divergent policy");
    assert!(error.to_string().contains("divergent search_tsv policy"));
    let after: String = sqlx::query_scalar("SELECT pg_get_expr(adbin,adrelid) FROM pg_attrdef d JOIN pg_attribute a ON a.attrelid=d.adrelid AND a.attnum=d.adnum WHERE a.attrelid='events_old'::regclass AND a.attname='search_tsv'")
        .fetch_one(&pool).await.expect("unchanged leaf expression");
    assert_eq!(before, after);
    cleanup(pool, schema).await;
}
