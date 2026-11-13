use sqlx::{Executor, PgPool};
use std::{path::Path, process::Stdio, time::Duration};

const MIGRATION: &str = include_str!("../../../../../../migrations/0089_event_reference_index.sql");

async fn catalog(pool: &PgPool) -> (String, bool, bool, bool) {
    sqlx::query_as("SELECT pg_get_indexdef(indexrelid),indisvalid,indisready,indislive FROM pg_index WHERE indexrelid='public.idx_events_reference_ids'::regclass")
        .fetch_one(pool).await.unwrap()
}

async fn seed(pool: &PgPool) {
    pool.execute("INSERT INTO communities(id,host) VALUES ('f0000000-0000-0000-0000-000000000001','reference-index.example');
        INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig)
        VALUES ('f0000000-0000-0000-0000-000000000001',decode(repeat('aa',32),'hex'),decode(repeat('bb',32),'hex'),now(),1,'[]','existing content',decode(repeat('cc',64),'hex'))")
        .await.unwrap();
}

async fn apply(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(MIGRATION).execute(&mut *tx).await?;
    tx.commit().await
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_index_populated_upgrade_matches_desired_state_and_new_partitions() {
    let desired = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let expected = catalog(&desired).await;
    assert!(expected.1 && expected.2 && expected.3);
    let admin = std::env::var("BUZZ_POSTGRES_ADMIN_URL").unwrap();
    let (pool, _scratch) =
        crate::test_support::ScratchDb::migrated_through(&admin, "reference_upgrade", 86).await;
    seed(&pool).await;
    let before: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(e) FROM events e")
        .fetch_one(&pool)
        .await
        .unwrap();
    crate::migration::run_migrations(&pool).await.unwrap();
    assert_eq!(catalog(&pool).await, expected);
    let oid: i64 = sqlx::query_scalar("SELECT 'idx_events_reference_ids'::regclass::oid::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    apply(&pool).await.unwrap();
    let after_oid: i64 =
        sqlx::query_scalar("SELECT 'idx_events_reference_ids'::regclass::oid::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after_oid, oid, "prebuilt index is reused");
    let after: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(e) FROM events e")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after, before);

    // The desired-state fixture's empty historical partition can be replaced
    // without touching application data. New partitions must inherit the index.
    desired.execute("DROP TABLE events_p_past; CREATE TABLE events_reference_past PARTITION OF events FOR VALUES FROM (MINVALUE) TO ('2026-01-01')").await.unwrap();
    let inherited: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_inherits h JOIN pg_index i ON i.indexrelid=h.inhrelid WHERE h.inhparent='idx_events_reference_ids'::regclass AND i.indrelid='events_reference_past'::regclass AND i.indisvalid AND i.indisready AND i.indislive")
        .fetch_one(&desired).await.unwrap();
    assert_eq!(inherited, 1);
    assert_eq!(catalog(&desired).await, expected);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_index_migration_rejects_wrong_definition() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    pool.execute("DROP INDEX idx_events_reference_ids; CREATE INDEX idx_events_reference_ids ON events(community_id)").await.unwrap();
    seed(&pool).await;
    let before: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(e) FROM events e")
        .fetch_one(&pool)
        .await
        .unwrap();
    let error = apply(&pool).await.unwrap_err();
    assert!(error.to_string().contains("wrong definition"), "{error}");
    let after: serde_json::Value = sqlx::query_scalar("SELECT to_jsonb(e) FROM events e")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_index_concurrent_prebuild_is_repeatable() {
    let url = crate::test_support::database_url();
    let pool = PgPool::connect(&url).await.unwrap();
    let expected = catalog(&pool).await;
    pool.execute("DROP INDEX idx_events_reference_ids")
        .await
        .unwrap();
    seed(&pool).await;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for _ in 0..2 {
        let child = tokio::process::Command::new("psql")
            .args(["-X", "-v", "ON_ERROR_STOP=1", "-f"])
            .arg(root.join("scripts/maintenance/prebuild-event-reference-index.sql"))
            .arg("--dbname")
            .arg(&url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(catalog(&pool).await, expected);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE content='existing content'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_recheck_uses_index_with_unrelated_history() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    seed(&pool).await;
    pool.execute(r#"INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig)
        SELECT 'f0000000-0000-0000-0000-000000000001',decode(md5(n::text)||md5(n::text),'hex'),
        decode(repeat('bb',32),'hex'),now(),1,
        jsonb_build_array(jsonb_build_array('e',upper(md5((n+300000)::text)||md5((n+300000)::text)))),
        'unrelated',decode(repeat('cc',64),'hex') FROM generate_series(1,20000) n;
        ANALYZE events"#).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    tx.execute("CREATE TEMP TABLE content_batch_work(table_name text,row_key jsonb,batch bigint,action text);
        INSERT INTO content_batch_work SELECT 'events',jsonb_build_object('id',md5(n::text)||md5(n::text),'created_at',now()),1,'would_delete' FROM generate_series(1,100) n;
        ANALYZE content_batch_work").await.unwrap();
    let query = include_str!("recheck.sql");
    let community = uuid::Uuid::parse_str("f0000000-0000-0000-0000-000000000001").unwrap();
    let referenced: bool = sqlx::query_scalar(query)
        .bind(community)
        .bind(1_i64)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(!referenced);
    let plan: serde_json::Value =
        sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {query}"))
            .bind(community)
            .bind(1_i64)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let indexes: Vec<String> = sqlx::query_scalar("SELECT c.relname FROM pg_inherits h JOIN pg_class c ON c.oid=h.inhrelid WHERE h.inhparent='idx_events_reference_ids'::regclass")
        .fetch_all(&mut *tx).await.unwrap();
    fn check(node: &serde_json::Value, indexes: &[String], used: &mut bool) {
        if let Some(index) = node["Index Name"].as_str() {
            if indexes.iter().any(|name| name == index) {
                *used = true;
                assert_eq!(node["Actual Rows"].as_u64(), Some(0));
            }
        }
        if node["Node Type"] == "Seq Scan"
            && node["Relation Name"]
                .as_str()
                .is_some_and(|n| n.starts_with("events"))
        {
            assert!(
                node["Rows Removed by Filter"].as_u64().unwrap_or(0) < 1000,
                "history scan: {node}"
            );
        }
        if let Some(children) = node["Plans"].as_array() {
            for child in children {
                check(child, indexes, used);
            }
        }
    }
    let mut used = false;
    check(&plan[0]["Plan"], &indexes, &mut used);
    assert!(
        used,
        "production recheck did not use normalized reference index: {plan}"
    );
}
