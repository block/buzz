//! Durable handoff for isolated media-storage accounting.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgConnection, Connection};

use crate::{observability, Db, Result};

/// Deployment-global advisory lock for the run-once storage worker.
pub const STORAGE_ACCOUNTING_LOCK_KEY: i64 = 0x4255_5a5a_5354_4f52;

/// Owns the detached PostgreSQL session that excludes overlapping workers.
pub struct StorageAccountingLeader {
    connection: PgConnection,
}

/// Days of per-community usage history kept. Each run prunes older rows in
/// the same transaction that appends its own, so the table stays bounded at
/// roughly one row per community per run inside this window.
pub const STORAGE_USAGE_HISTORY_RETENTION_DAYS: i32 = 400;

/// One per-community measurement from a completed accounting run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageUsageRecord {
    /// Monotonic feed position; pass the last one back as `after`.
    pub cursor: i64,
    /// Commit time of the accounting run that measured it.
    pub measured_at: DateTime<Utc>,
    /// The community's current canonical host.
    pub community_host: String,
    /// Logical bytes bound to the community.
    pub storage_bytes: i64,
    /// Logical objects bound to the community.
    pub storage_objects: i64,
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
    /// Atomically replace the singleton through the lock-owning session and
    /// append the run's per-community usage history in the same transaction.
    ///
    /// History gets one row per community in the snapshot's `per_community`
    /// map, plus an explicit zero for each serving community the map omits,
    /// so a consumer keeping the latest value per community never keeps a
    /// stale nonzero one. Rows past the retention window are pruned in the
    /// same transaction. A worker whose lock session died can write neither.
    ///
    /// Only communities that are not yet fenced for deletion get rows, and
    /// their community rows are held `FOR KEY SHARE` until commit. Fencing
    /// takes `FOR UPDATE` on the community row before purge can run, so a
    /// fence either waits for this commit (and purge later severs the rows)
    /// or commits first (and the re-checked row is skipped). A scan that
    /// captured a community before it was deleted therefore cannot attribute
    /// usage to it after purge severed its history.
    /// Runs serialize on the worker lock, so history ids grow in commit order
    /// and are safe to page with an `id > cursor` feed.
    #[datastore_span(name = "save_storage_accounting_snapshot", system = "postgresql")]
    pub async fn save_snapshot(
        &mut self,
        snapshot: &serde_json::Value,
        duration_ms: i64,
        max_objects: i64,
        code_sha: &str,
    ) -> Result<()> {
        let mut tx = self.connection.begin().await?;
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
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "WITH measured AS ( \
               SELECT (entry.key)::uuid AS community_id, \
                      (entry.value->>'bytes')::bigint AS bytes, \
                      (entry.value->>'objects')::bigint AS objects \
               FROM jsonb_each(COALESCE($1->'per_community', '{}'::jsonb)) AS entry \
             ), \
             eligible AS ( \
               SELECT c.id FROM communities c \
               WHERE c.deletion_state IN ('active', 'quiescing') \
                 AND (c.id IN (SELECT community_id FROM measured) \
                      OR (c.archived_at IS NULL AND c.deleted_at IS NULL \
                          AND c.deletion_state = 'active')) \
               FOR KEY SHARE OF c \
             ) \
             INSERT INTO storage_usage_history \
               (measured_at, usage_community_id, storage_bytes, storage_objects) \
             SELECT transaction_timestamp(), e.id, \
                    COALESCE(m.bytes, 0), COALESCE(m.objects, 0) \
             FROM eligible e LEFT JOIN measured m ON m.community_id = e.id",
        )
        .bind(snapshot)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM storage_usage_history \
             WHERE measured_at < transaction_timestamp() - make_interval(days => $1)",
        )
        .bind(STORAGE_USAGE_HISTORY_RETENTION_DAYS)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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

    /// Page the usage history after `after`, oldest first. Rows whose
    /// community was deleted (provenance severed, or a tombstone) are skipped
    /// before `limit` applies, so a page shorter than `limit` means the feed
    /// is caught up.
    #[datastore_span(name = "list_storage_usage", system = "postgresql")]
    pub async fn list_storage_usage(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<StorageUsageRecord>> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        let rows = sqlx::query_as::<_, (i64, DateTime<Utc>, String, i64, i64)>(
            "SELECT h.id, h.measured_at, c.host, h.storage_bytes, h.storage_objects \
             FROM storage_usage_history h \
             JOIN communities c ON c.id = h.usage_community_id \
             WHERE h.id > $1 AND c.deletion_state <> 'tombstone' \
             ORDER BY h.id LIMIT $2",
        )
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(cursor, measured_at, community_host, storage_bytes, storage_objects)| {
                    StorageUsageRecord {
                        cursor,
                        measured_at,
                        community_host,
                        storage_bytes,
                        storage_objects,
                    }
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::test_support::ScratchDb;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

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
    async fn usage_history_feeds_each_run_with_zero_fill_paging_and_retention() {
        let (pool, _scratch) =
            ScratchDb::migrated(&crate::test_support::database_url(), "storage_accounting").await;
        let db = Db::from_pool(pool.clone());
        let mut ids = std::collections::HashMap::new();
        for (host, archived, state) in [
            ("measured.example", false, "active"),
            ("idle.example", false, "active"),
            ("archived.example", true, "active"),
            ("quiescing.example", false, "quiescing"),
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
            ids.insert(host, id);
        }
        // A row older than the retention window, pruned by the next run.
        sqlx::query(
            "INSERT INTO storage_usage_history \
             (measured_at, usage_community_id, storage_bytes, storage_objects) \
             VALUES (now() - make_interval(days => $1 + 1), $2, 1, 1)",
        )
        .bind(STORAGE_USAGE_HISTORY_RETENTION_DAYS)
        .bind(ids["measured.example"])
        .execute(&pool)
        .await
        .expect("insert expired history");

        let mut leader = db
            .try_lock_storage_accounting()
            .await
            .expect("lock")
            .expect("worker owns lock");
        let run = |measured: u64, archived: u64| {
            serde_json::json!({
                "per_community": {
                    ids["measured.example"].to_string(): {"bytes": measured, "objects": 2},
                    ids["archived.example"].to_string(): {"bytes": archived, "objects": 1},
                    Uuid::new_v4().to_string(): {"bytes": 9, "objects": 9},
                },
            })
        };
        leader
            .save_snapshot(&run(100, 50), 10, 100, "a")
            .await
            .expect("save first run");
        let ours = |records: Vec<StorageUsageRecord>| -> Vec<(String, i64, i64)> {
            let mut rows: Vec<_> = records
                .into_iter()
                .filter(|r| ids.contains_key(r.community_host.as_str()))
                .map(|r| (r.community_host, r.storage_bytes, r.storage_objects))
                .collect();
            rows.sort();
            rows
        };
        let first = db.list_storage_usage(0, 1000).await.expect("first page");
        let first_cursor = first.last().expect("first run rows").cursor;
        let first_measured_at = first[0].measured_at;
        assert!(first.iter().all(|r| r.measured_at == first_measured_at));
        // Measured and archived-with-bytes rows are reported, the serving
        // community the run omitted is zero-filled, the quiescing one is not,
        // the orphan id with no community row is skipped, and the expired row
        // is gone.
        assert_eq!(
            ours(first),
            vec![
                ("archived.example".to_owned(), 50, 1),
                ("idle.example".to_owned(), 0, 0),
                ("measured.example".to_owned(), 100, 2),
            ]
        );

        leader
            .save_snapshot(&run(150, 50), 10, 100, "b")
            .await
            .expect("save second run");
        let second = db
            .list_storage_usage(first_cursor, 1000)
            .await
            .expect("second page");
        assert!(second.iter().all(|r| r.cursor > first_cursor));
        assert!(second.iter().all(|r| r.measured_at > first_measured_at));
        assert_eq!(
            ours(second),
            vec![
                ("archived.example".to_owned(), 50, 1),
                ("idle.example".to_owned(), 0, 0),
                ("measured.example".to_owned(), 150, 2),
            ]
        );

        // Single-row pages walk the same feed in cursor order with no gaps.
        let mut cursor = 0;
        let mut walked = 0;
        loop {
            let page = db.list_storage_usage(cursor, 1).await.expect("page");
            let Some(row) = page.into_iter().next() else {
                break;
            };
            assert!(row.cursor > cursor);
            cursor = row.cursor;
            walked += 1;
        }
        let total = db.list_storage_usage(0, 1000).await.expect("all").len();
        assert_eq!(walked, total);
    }

    /// A fence that holds its community row while a scan saves makes the save
    /// wait, and the save then re-checks the row and skips the now-fenced
    /// community, so purge can never miss a row the save was still writing.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn usage_history_waits_for_a_concurrent_fence_and_skips_its_community() {
        let (pool, _scratch) =
            ScratchDb::migrated(&crate::test_support::database_url(), "storage_accounting").await;
        let db = Db::from_pool(pool.clone());
        let mut ids = Vec::new();
        for host in ["fencing.example", "bystander.example"] {
            let id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO communities (host, deletion_state) \
                 VALUES ($1, 'quiescing') RETURNING id",
            )
            .bind(host)
            .fetch_one(&pool)
            .await
            .expect("insert community");
            ids.push(id);
        }
        let (fencing, bystander) = (ids[0], ids[1]);

        // The same row lock and transition the deletion fence takes.
        let mut fence = pool.begin().await.expect("begin fence");
        sqlx::query("SELECT deletion_fence_generation FROM communities WHERE id = $1 FOR UPDATE")
            .bind(fencing)
            .execute(&mut *fence)
            .await
            .expect("lock community for fence");
        sqlx::query(
            "UPDATE communities SET deletion_state = 'fenced', \
                    deletion_fence_generation = deletion_fence_generation + 1 \
             WHERE id = $1",
        )
        .bind(fencing)
        .execute(&mut *fence)
        .await
        .expect("fence community");

        let mut leader = db
            .try_lock_storage_accounting()
            .await
            .expect("lock")
            .expect("worker owns lock");
        let scan = serde_json::json!({
            "per_community": {
                fencing.to_string(): {"bytes": 7, "objects": 1},
                bystander.to_string(): {"bytes": 3, "objects": 1},
            },
        });
        let save = tokio::spawn(async move {
            leader
                .save_snapshot(&scan, 1, 1, "racing-fence")
                .await
                .expect("save snapshot")
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                     WHERE datname = current_database() AND wait_event_type = 'Lock' \
                       AND query LIKE '%INSERT INTO storage_usage_history%')",
                )
                .fetch_one(&pool)
                .await
                .expect("read lock waits");
                if waiting {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("save waits on the fenced community row");
        assert!(!save.is_finished(), "save must not commit past the fence");

        fence.commit().await.expect("commit fence");
        save.await.expect("save task");

        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT usage_community_id, storage_bytes FROM storage_usage_history \
             WHERE usage_community_id = ANY($1) ORDER BY storage_bytes",
        )
        .bind(vec![fencing, bystander])
        .fetch_all(&pool)
        .await
        .expect("read history");
        assert_eq!(rows, vec![(bystander, 3)]);
    }

    /// A tombstone keeps its host, so the feed must not serve rows still
    /// attributed to it.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn usage_feed_hides_rows_attributed_to_a_tombstone() {
        let (pool, _scratch) =
            ScratchDb::migrated(&crate::test_support::database_url(), "storage_accounting").await;
        let db = Db::from_pool(pool.clone());
        let tombstone = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO communities (host, deleted_at, archived_at, deletion_state) \
             VALUES ('tombstone.example', now(), now(), 'tombstone') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("insert tombstone");
        sqlx::query(
            "INSERT INTO storage_usage_history \
             (measured_at, usage_community_id, storage_bytes, storage_objects) \
             VALUES (now(), $1, 7, 1)",
        )
        .bind(tombstone)
        .execute(&pool)
        .await
        .expect("insert attributed row");

        let feed = db.list_storage_usage(0, 1000).await.expect("read feed");
        assert!(feed
            .iter()
            .all(|row| row.community_host != "tombstone.example"));
    }
}
