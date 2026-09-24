//! Exercise the same database-to-metrics tick called by the relay leader.

use super::*;
use buzz_media::CommunityStorage;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};

fn observe_tick(
    db: &Db,
    state: &Mutex<StorageSweepState>,
    mode: StorageMetricsMode,
    hosts: &HashMap<Uuid, String>,
) -> (anyhow::Result<()>, HashMap<String, f64>) {
    let recorder = DebuggingRecorder::new();
    let result = metrics::with_local_recorder(&recorder, || {
        futures::executor::block_on(run_storage_metrics_tick(db, state, mode, hosts, |_| true))
    });
    let values = recorder
        .snapshotter()
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name().contains("_storage_"))
        .filter_map(|(key, _, _, value)| match value {
            DebugValue::Gauge(value) => Some((key.key().name().to_owned(), value.into_inner())),
            _ => None,
        })
        .collect();
    (result, values)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn worker_publication_activates_reader_without_restart_or_mode_change() {
    // The PostgreSQL lane supplies a separate desired-state database per test.
    // Local callers must likewise supply an explicitly isolated database.
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("isolated BUZZ_TEST_DATABASE_URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect isolated database");
    let db = Db::from_pool(pool.clone());
    let community = Uuid::new_v4();
    let hosts = HashMap::from([(community, "storage.example.test".to_owned())]);

    for configured in [None, Some("inline"), Some("external")] {
        sqlx::query("DELETE FROM storage_accounting_snapshots")
            .execute(&pool)
            .await
            .expect("clear isolated snapshot");
        let state = Mutex::new(StorageSweepState::default());
        let mode = parse_storage_metrics_mode(configured);
        let (result, values) = observe_tick(&db, &state, mode, &hosts);
        result.expect("no worker is normal");
        assert!(
            values.is_empty(),
            "no row must not emit zeros or failed-load health"
        );

        let mut worker = db.try_lock_storage_accounting().await.unwrap().unwrap();
        for bytes in [100, 250] {
            let snapshot = BucketSnapshot {
                physical_bytes: bytes,
                physical_objects: 1,
                logical_bytes: bytes,
                logical_objects: 1,
                per_community: HashMap::from([(community, CommunityStorage { bytes, objects: 1 })]),
                ..Default::default()
            };
            worker
                .save_snapshot(
                    &serde_json::to_value(snapshot).unwrap(),
                    12,
                    1_000,
                    "test-worker",
                )
                .await
                .expect("publish through production worker session");
            let (result, values) = observe_tick(&db, &state, mode, &hosts);
            result.expect("same running reader picks up worker publication");
            assert_eq!(values["buzz_community_storage_bytes"], bytes as f64);
            assert_eq!(values["buzz_total_storage_bytes"], bytes as f64);
            assert_eq!(values["buzz_storage_snapshot_load_ok"], 1.0);
            assert!(!values
                .keys()
                .any(|key| key.starts_with("buzz_storage_sweep_")));
        }
        // Corrupt data must not be mistaken for a worker that is not installed.
        worker
            .save_snapshot(
                &serde_json::json!({"invalid": true}),
                12,
                1_000,
                "bad-worker",
            )
            .await
            .unwrap();
        let (result, values) = observe_tick(&db, &state, mode, &hosts);
        assert!(result.is_err());
        assert_eq!(values["buzz_storage_snapshot_load_ok"], 0.0);
        assert_eq!(values["buzz_community_storage_bytes"], 250.0);

        // A blocked table read is bounded, preserves the cache, and can recover.
        let mut blocker = pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE storage_accounting_snapshots IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let (result, values) = observe_tick(&db, &state, mode, &hosts);
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(8));
        assert_eq!(values["buzz_storage_snapshot_load_ok"], 0.0);
        assert_eq!(values["buzz_community_storage_bytes"], 250.0);
        blocker.rollback().await.unwrap();

        sqlx::query("DELETE FROM storage_accounting_snapshots")
            .execute(&pool)
            .await
            .unwrap();
        let (result, values) = observe_tick(&db, &state, mode, &hosts);
        result.expect("removed worker snapshot is inactive");
        assert!(
            values.is_empty(),
            "do not refresh stale totals or report zero storage"
        );

        worker
            .save_snapshot(
                &serde_json::to_value(BucketSnapshot::default()).unwrap(),
                1,
                100,
                "restarted-worker",
            )
            .await
            .unwrap();
        let (result, values) = observe_tick(&db, &state, mode, &hosts);
        result.expect("reader reactivates after removal and transient failure");
        assert_eq!(values["buzz_storage_snapshot_load_ok"], 1.0);
        assert_eq!(
            values["buzz_total_storage_bytes"], 0.0,
            "a real empty snapshot may report zero"
        );
        assert_eq!(
            values["buzz_community_storage_bytes"], 0.0,
            "clear the previous community when publication resumes"
        );
        drop(worker);
    }
    pool.close().await;
}
