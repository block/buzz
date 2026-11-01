//! Durable handoff for isolated media-storage accounting.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgConnection;
use uuid::Uuid;

use crate::{observability, Db, Result};

/// Deployment-global advisory lock for the run-once storage worker.
pub const STORAGE_ACCOUNTING_LOCK_KEY: i64 = 0x4255_5a5a_5354_4f52;

/// Owns the detached PostgreSQL session that excludes overlapping workers.
pub struct StorageAccountingLeader {
    connection: PgConnection,
}

/// A community that storage-usage delivery reports on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageUsageCommunity {
    /// The community UUID used by the snapshot's per-community map.
    pub id: Uuid,
    /// The canonical host stored on the community row.
    pub host: String,
    /// Whether the community is serving: not archived and not being deleted.
    pub active: bool,
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

impl StorageAccountingLeader {
    /// Atomically replace the singleton through the lock-owning session.
    #[datastore_span(name = "save_storage_accounting_snapshot", system = "postgresql")]
    pub async fn save_snapshot(
        &mut self,
        snapshot: &serde_json::Value,
        duration_ms: i64,
        max_objects: i64,
        code_sha: &str,
    ) -> Result<()> {
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
        .execute(&mut self.connection)
        .await?;
        Ok(())
    }
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
            connection: connection.detach(),
        }))
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

    /// Load only the newest snapshot's commit time, so a poller can detect a
    /// new snapshot without decoding the full per-community payload.
    #[datastore_span(name = "load_storage_accounting_completed_at", system = "postgresql")]
    pub async fn load_storage_accounting_completed_at(&self) -> Result<Option<DateTime<Utc>>> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        Ok(sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT completed_at FROM storage_accounting_snapshots WHERE singleton = TRUE",
        )
        .fetch_optional(&mut *connection)
        .await?)
    }

    /// List every community with its host and whether it is still serving.
    #[datastore_span(name = "storage_usage_communities", system = "postgresql")]
    pub async fn storage_usage_communities(&self) -> Result<Vec<StorageUsageCommunity>> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        let rows = sqlx::query_as::<_, (Uuid, String, bool)>(
            "SELECT id, host, \
               (archived_at IS NULL AND deleted_at IS NULL AND deletion_state = 'active') \
             FROM communities",
        )
        .fetch_all(&mut *connection)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, host, active)| StorageUsageCommunity { id, host, active })
            .collect())
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::test_support::ScratchDb;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn complete_snapshot_replaces_atomically_and_worker_lock_excludes_overlap() {
        let (pool, _scratch) =
            ScratchDb::migrated(&crate::test_support::database_url(), "storage_accounting").await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());

        let mut leader = first
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

        leader
            .save_snapshot(&serde_json::json!({"version": 1}), 10, 100, "a")
            .await
            .expect("save first snapshot");
        leader
            .save_snapshot(&serde_json::json!({"version": 2}), 20, 200, "b")
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
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lost_lock_session_cannot_overwrite_successor_snapshot() {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect admin");
        let (pool, _scratch) =
            ScratchDb::migrated(&crate::test_support::database_url(), "storage_accounting").await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());

        let mut stale_leader = first
            .try_lock_storage_accounting()
            .await
            .expect("first lock")
            .expect("first worker owns lock");
        let stale_backend_pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut stale_leader.connection)
            .await
            .expect("load first worker backend pid");
        let terminated = sqlx::query_scalar::<_, bool>("SELECT pg_terminate_backend($1)")
            .bind(stale_backend_pid)
            .fetch_one(&admin)
            .await
            .expect("terminate first worker backend");
        assert!(terminated, "first worker backend must terminate");

        let mut fresh_leader = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(leader) = second
                    .try_lock_storage_accounting()
                    .await
                    .expect("successor lock attempt")
                {
                    break leader;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("successor acquires released lock");

        fresh_leader
            .save_snapshot(&serde_json::json!({"worker": "fresh"}), 20, 200, "b")
            .await
            .expect("successor publishes fresh snapshot");
        stale_leader
            .save_snapshot(&serde_json::json!({"worker": "stale"}), 30, 100, "a")
            .await
            .expect_err("worker that lost its lock session cannot publish");

        let stored = second
            .load_storage_accounting_snapshot()
            .await
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(stored.snapshot, serde_json::json!({"worker": "fresh"}));
        assert_eq!(stored.duration_ms, 20);
        assert_eq!(stored.max_objects, 200);
        assert_eq!(stored.code_sha, "b");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn usage_delivery_reads_snapshot_time_and_community_states() {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect admin");
        let (pool, name) = create_scratch_db(&admin).await;
        let db = Db::from_pool(pool.clone());

        assert_eq!(
            db.load_storage_accounting_completed_at()
                .await
                .expect("load absent completed_at"),
            None
        );
        let mut leader = db
            .try_lock_storage_accounting()
            .await
            .expect("lock")
            .expect("worker owns lock");
        leader
            .save_snapshot(&serde_json::json!({"version": 1}), 10, 100, "a")
            .await
            .expect("save snapshot");
        let stored = db
            .load_storage_accounting_snapshot()
            .await
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(
            db.load_storage_accounting_completed_at()
                .await
                .expect("load completed_at"),
            Some(stored.completed_at)
        );

        let mut expected = Vec::new();
        for (host, archived, state, active) in [
            ("active.example", false, "active", true),
            ("archived.example", true, "active", false),
            ("quiescing.example", false, "quiescing", false),
            ("tombstone.example", false, "tombstone", false),
        ] {
            let id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO communities (host, archived_at, deletion_state) \
                 VALUES ($1, CASE WHEN $2 THEN now() END, $3) RETURNING id",
            )
            .bind(host)
            .bind(archived)
            .bind(state)
            .fetch_one(&pool)
            .await
            .expect("insert community");
            expected.push(StorageUsageCommunity {
                id,
                host: host.to_string(),
                active,
            });
        }
        let mut communities = db
            .storage_usage_communities()
            .await
            .expect("list communities");
        communities.retain(|community| expected.iter().any(|e| e.id == community.id));
        communities.sort_by(|a, b| a.host.cmp(&b.host));
        expected.sort_by(|a, b| a.host.cmp(&b.host));
        assert_eq!(communities, expected);

        drop(leader);
        drop(db);
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }
}
