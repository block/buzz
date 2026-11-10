//! Exercise the shipped psql procedure with real old-snapshot waits.
use sqlx::Executor;
use std::{path::Path, process::Stdio, time::Duration};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_media_prebuild_waits_for_old_transactions_during_creation_and_cleanup() {
    let url = crate::test_support::database_url();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    // This migration-mode fixture owns an empty per-test database. Keep only
    // the columns the operator procedure uses, including its final 0086 guard.
    pool.execute("CREATE TABLE events (community_id bytea, channel_id bytea, tags jsonb, created_at bigint, id bytea, kind int, deleted_at bigint) PARTITION BY RANGE(created_at); CREATE TABLE events_p PARTITION OF events FOR VALUES FROM (0) TO (100); INSERT INTO events(tags,created_at,kind) VALUES ('[]',1,40003)").await.unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expression = crate::store::media::MEDIA_EDIT_TARGET_SQL;
    for cleanup in [false, true] {
        if cleanup {
            pool.execute("DROP INDEX idx_events_media_edit_target")
                .await
                .unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE INDEX idx_events_media_edit_target ON ONLY events(community_id,channel_id,({expression}),created_at DESC,id DESC) WHERE kind=40003 AND deleted_at IS NULL"))).execute(&pool).await.unwrap();
        }
        let mut old = pool.begin().await.unwrap();
        old.execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .await
            .unwrap();
        old.execute("SELECT * FROM events").await.unwrap();
        if cleanup {
            let mut conn = pool.acquire().await.unwrap();
            conn.execute("SET lock_timeout = '50ms'").await.unwrap();
            let error = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE INDEX CONCURRENTLY ON events_p(community_id,channel_id,({expression}),created_at DESC,id DESC) WHERE kind=40003 AND deleted_at IS NULL"))).execute(&mut *conn).await.unwrap_err();
            assert!(error.to_string().contains("lock timeout"), "{error}");
            conn.execute("RESET lock_timeout").await.unwrap();
            let invalid: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_index WHERE indrelid='events_p'::regclass AND NOT indisvalid").fetch_one(&pool).await.unwrap();
            assert_eq!(
                invalid, 1,
                "cleanup starts with a real failed concurrent build"
            );
        }
        let name = format!("media_prebuild_{}", uuid::Uuid::new_v4().simple());
        let mut script = tokio::process::Command::new("psql")
            .args(["-X", "-v", "ON_ERROR_STOP=1", "-f"])
            .arg(root.join("scripts/maintenance/prebuild-media-edit-index.sql"))
            .arg("--dbname")
            .arg(&url)
            .env("PGAPPNAME", &name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let waiting = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND application_name=$1 AND wait_event_type='Lock' AND wait_event='virtualxid')")
                    .bind(&name).fetch_one(&pool).await.unwrap();
                if waiting { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await;
        if waiting.is_err() {
            old.rollback().await.unwrap();
            let result = script.wait_with_output().await.unwrap();
            panic!(
                "cleanup={cleanup}: script did not reach old-transaction wait: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        // Keep the old snapshot until the original one-second deadline has
        // passed. An early exit here catches both CREATE and DROP regressions.
        assert!(
            tokio::time::timeout(Duration::from_millis(1300), script.wait())
                .await
                .is_err(),
            "cleanup={cleanup}: concurrent operation must outlive the short metadata lock budget"
        );
        old.commit().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), script.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.status.success(),
            "cleanup={cleanup}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let valid: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_index WHERE indrelid IN ('events'::regclass,'events_p'::regclass) AND indisvalid AND indisready").fetch_one(&pool).await.unwrap();
        assert_eq!(valid, 2);
        let invalid: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_index WHERE indrelid='events_p'::regclass AND NOT indisvalid",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            invalid, 0,
            "retry must remove the unusable child, not leave it beside a replacement"
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }
}
