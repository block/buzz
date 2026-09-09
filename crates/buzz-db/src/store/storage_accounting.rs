//! Durable handoff for isolated media-storage accounting.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgConnection;

use crate::{observability, Db, Result};

/// Deployment-global advisory lock for the run-once storage worker.
pub const STORAGE_ACCOUNTING_LOCK_KEY: i64 = 0x4255_5a5a_5354_4f52;

/// Owns the detached PostgreSQL session that excludes overlapping workers.
pub struct StorageAccountingLeader {
    _connection: PgConnection,
}

/// The newest complete snapshot stored by the worker.
#[derive(Debug, Clone)]
pub struct StoredStorageSnapshot {
    /// Serialized `buzz_media::BucketSnapshot`.
    pub snapshot: serde_json::Value,
    /// Database commit time for freshness reporting.
    pub completed_at: DateTime<Utc>,
    /// Wall-clock duration of the successful S3 fold.
    pub duration_ms: i64,
    /// Object ceiling configured for that run.
    pub max_objects: i64,
    /// Image or source revision supplied by the worker.
    pub code_sha: String,
}

impl Db {
    /// Try to acquire the deployment-global storage-worker lease.
    #[datastore_span(name = "try_lock_storage_accounting", system = "postgresql")]
    pub async fn try_lock_storage_accounting(&self) -> Result<Option<StorageAccountingLeader>> {
        let mut connection = observability::acquire_writer_with_legacy_metrics(
            &self.pool,
            observability::WriterOperation::Maintenance,
        )
        .await?;
        let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(STORAGE_ACCOUNTING_LOCK_KEY)
            .fetch_one(&mut *connection)
            .await?;
        Ok(acquired.then(|| StorageAccountingLeader {
            _connection: connection.detach(),
        }))
    }

    /// Atomically replace the singleton with one fully computed snapshot.
    #[datastore_span(name = "save_storage_accounting_snapshot", system = "postgresql")]
    pub async fn save_storage_accounting_snapshot(
        &self,
        snapshot: &serde_json::Value,
        duration_ms: i64,
        max_objects: i64,
        code_sha: &str,
    ) -> Result<()> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        sqlx::query(
            "INSERT INTO storage_accounting_snapshots \
             (singleton, snapshot, completed_at, duration_ms, max_objects, code_sha) \
             VALUES (TRUE, $1, transaction_timestamp(), $2, $3, $4) \
             ON CONFLICT (singleton) DO UPDATE SET \
               snapshot = EXCLUDED.snapshot, \
               completed_at = EXCLUDED.completed_at, \
               duration_ms = EXCLUDED.duration_ms, \
               max_objects = EXCLUDED.max_objects, \
               code_sha = EXCLUDED.code_sha",
        )
        .bind(snapshot)
        .bind(duration_ms)
        .bind(max_objects)
        .bind(code_sha)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Load the newest complete worker snapshot from the writer.
    #[datastore_span(name = "load_storage_accounting_snapshot", system = "postgresql")]
    pub async fn load_storage_accounting_snapshot(&self) -> Result<Option<StoredStorageSnapshot>> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        let row = sqlx::query_as::<_, (serde_json::Value, DateTime<Utc>, i64, i64, String)>(
            "SELECT snapshot, completed_at, duration_ms, max_objects, code_sha \
             FROM storage_accounting_snapshots WHERE singleton = TRUE",
        )
        .fetch_optional(&mut *connection)
        .await?;
        Ok(row.map(
            |(snapshot, completed_at, duration_ms, max_objects, code_sha)| StoredStorageSnapshot {
                snapshot,
                completed_at,
                duration_ms,
                max_objects,
                code_sha,
            },
        ))
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use uuid::Uuid;

    async fn create_scratch_db(admin: &PgPool) -> (PgPool, String) {
        let name = format!("storage_accounting_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch db");
        let base = crate::test_support::database_url();
        let idx = base.rfind('/').expect("db url has path");
        let pool = PgPool::connect(&format!("{}/{name}", &base[..idx]))
            .await
            .expect("connect scratch db");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch db");
        (pool, name)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn complete_snapshot_replaces_atomically_and_worker_lock_excludes_overlap() {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect admin");
        let (pool, name) = create_scratch_db(&admin).await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());

        let leader = first
            .try_lock_storage_accounting()
            .await
            .expect("first lock")
            .expect("first worker owns lock");
        assert!(
            second
                .try_lock_storage_accounting()
                .await
                .expect("second lock")
                .is_none(),
            "overlapping worker must not start"
        );

        first
            .save_storage_accounting_snapshot(&serde_json::json!({"version": 1}), 10, 100, "a")
            .await
            .expect("save first snapshot");
        first
            .save_storage_accounting_snapshot(&serde_json::json!({"version": 2}), 20, 200, "b")
            .await
            .expect("replace snapshot");
        let stored = second
            .load_storage_accounting_snapshot()
            .await
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(stored.snapshot, serde_json::json!({"version": 2}));
        assert_eq!(stored.duration_ms, 20);
        assert_eq!(stored.max_objects, 200);
        assert_eq!(stored.code_sha, "b");

        drop(leader);
        drop(first);
        drop(second);
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }
}
