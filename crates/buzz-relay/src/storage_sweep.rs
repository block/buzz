//! Publish storage metrics from the latest complete worker snapshot.
//!
//! The leader checks PostgreSQL on every usage tick. An environment without a
//! snapshot emits no storage gauges; the first worker publication is picked up
//! without restarting or reconfiguring the relay. S3 listing belongs only to
//! the worker. Failed reads retain the last good totals with load health and
//! the original snapshot age, never a fabricated fresh measurement.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use uuid::Uuid;

use buzz_db::Db;
use buzz_media::BucketSnapshot;

/// Storage metrics are read from PostgreSQL unless explicitly disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMetricsMode {
    /// Read the newest completed snapshot persisted by `buzz-admin`.
    Snapshot,
    /// Neither read snapshots nor emit storage-family metrics.
    Disabled,
}

impl StorageMetricsMode {
    /// Read `BUZZ_STORAGE_METRICS`. Unset enables the database reader.
    /// Legacy `inline`/`on` values also select the reader so existing charts
    /// need no coordinated mode change. `off` and unknown values disable it.
    pub fn from_env() -> Self {
        let value = std::env::var("BUZZ_STORAGE_METRICS").ok();
        parse_storage_metrics_mode(value.as_deref())
    }
}

fn parse_storage_metrics_mode(value: Option<&str>) -> StorageMetricsMode {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("off") => StorageMetricsMode::Disabled,
        Some("inline") => {
            tracing::warn!("BUZZ_STORAGE_METRICS=inline now reads worker snapshots; relay S3 sweeps are retired");
            StorageMetricsMode::Snapshot
        }
        Some("snapshot" | "external" | "on") | None => StorageMetricsMode::Snapshot,
        Some(other) => {
            tracing::error!(
                value = other,
                "invalid BUZZ_STORAGE_METRICS value; storage metrics disabled"
            );
            StorageMetricsMode::Disabled
        }
    }
}

// Bound the whole read, including pool acquisition and the database query.
const SNAPSHOT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The most recent successfully decoded worker snapshot.
#[derive(Debug, Clone)]
struct CachedSnapshot {
    data: BucketSnapshot,
    completed_at_wall: DateTime<Utc>,
    duration: Duration,
    max_objects: u64,
}

/// Key tracking which per-community series were emitted in the previous tick,
/// so series for communities that disappear from the snapshot (unmapped, host
/// renamed, or scope-excluded) are zeroed rather than left at their last
/// nonzero value until the recorder's idle-eviction kicks in.
///
/// Carries the resolved host label (not the UUID) so a rename can still zero
/// the old series, and distinguishes bytes vs. objects because they are
/// separate Prometheus series.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum StorageEmittedKey {
    Bytes(String),
    Objects(String),
}

impl StorageEmittedKey {
    fn set(&self, value: f64) {
        match self {
            Self::Bytes(host) => {
                metrics::gauge!("buzz_community_storage_bytes", "community" => host.clone())
                    .set(value);
            }
            Self::Objects(host) => {
                metrics::gauge!("buzz_community_storage_objects", "community" => host.clone())
                    .set(value);
            }
        }
    }
}

/// Cached worker data and emission bookkeeping, shared across usage ticks.
#[derive(Default)]
pub struct StorageSweepState {
    cached: Option<CachedSnapshot>,
    // None means no worker snapshot exists. A failed read is distinct from
    // absence and remains observable even before the first successful read.
    persisted_load_ok: Option<bool>,
    previously_emitted: HashSet<StorageEmittedKey>,
}

/// Load and emit one snapshot on the existing leader-only usage cadence.
///
/// No row is a normal inactive state. Database/decoding failures preserve the
/// last good cache, emit failed-load health, and propagate for caller logging;
/// the next usage tick retries. This path never lists S3 or delays startup.
pub async fn run_storage_metrics_tick(
    db: &Db,
    state: &Mutex<StorageSweepState>,
    mode: StorageMetricsMode,
    host_map: &HashMap<Uuid, String>,
    allows: impl Fn(&Uuid) -> bool,
) -> anyhow::Result<()> {
    if mode == StorageMetricsMode::Disabled {
        return Ok(());
    }
    let result = refresh_persisted_snapshot(db, state).await;
    if result.is_err() {
        record_persisted_snapshot_load_failure(state).await;
    }
    emit_storage_metrics(state, mode, host_map, allows).await;
    result
}

async fn refresh_persisted_snapshot(
    db: &Db,
    state: &Mutex<StorageSweepState>,
) -> anyhow::Result<()> {
    let stored = tokio::time::timeout(SNAPSHOT_READ_TIMEOUT, db.load_storage_accounting_snapshot())
        .await??;
    let Some(stored) = stored else {
        // A removed row also stops refreshing old metric series. They expire
        // through the exporter's idle timeout; absence never means zero bytes.
        let mut state = state.lock().await;
        state.cached = None;
        state.persisted_load_ok = None;
        // Keep the emitted keys until a valid snapshot returns, so its first
        // emission can clear old community labels even before idle eviction.
        return Ok(());
    };
    let snapshot = serde_json::from_value(stored.snapshot)?;
    let duration = Duration::from_millis(u64::try_from(stored.duration_ms)?);
    let max_objects = u64::try_from(stored.max_objects)?;
    anyhow::ensure!(
        max_objects > 0,
        "stored storage snapshot has a zero object cap"
    );
    cache_persisted_snapshot(state, snapshot, stored.completed_at, duration, max_objects).await;
    Ok(())
}

/// Replace the cache with a complete snapshot loaded from durable storage.
async fn cache_persisted_snapshot(
    state: &Mutex<StorageSweepState>,
    snapshot: BucketSnapshot,
    completed_at: DateTime<Utc>,
    duration: Duration,
    max_objects: u64,
) {
    let mut state = state.lock().await;
    state.cached = Some(CachedSnapshot {
        data: snapshot,
        completed_at_wall: completed_at,
        duration,
        max_objects,
    });
    state.persisted_load_ok = Some(true);
}

async fn record_persisted_snapshot_load_failure(state: &Mutex<StorageSweepState>) {
    state.lock().await.persisted_load_ok = Some(false);
}

async fn emit_storage_metrics(
    state: &Mutex<StorageSweepState>,
    mode: StorageMetricsMode,
    host_map: &HashMap<Uuid, String>,
    allows: impl Fn(&Uuid) -> bool,
) {
    if mode == StorageMetricsMode::Disabled {
        return;
    }
    let mut state = state.lock().await;
    let Some(load_ok) = state.persisted_load_ok else {
        return;
    };
    metrics::gauge!("buzz_storage_snapshot_load_ok").set(if load_ok { 1.0 } else { 0.0 });
    emit_cached_storage_metrics(&mut state, host_map, allows);
}

fn emit_cached_storage_metrics(
    state: &mut StorageSweepState,
    host_map: &HashMap<Uuid, String>,
    allows: impl Fn(&Uuid) -> bool,
) {
    let Some(cached) = &state.cached else {
        return;
    };
    let age = Utc::now()
        .signed_duration_since(cached.completed_at_wall)
        .to_std()
        .unwrap_or_default();
    metrics::gauge!("buzz_storage_snapshot_age_seconds").set(age.as_secs_f64());
    metrics::gauge!("buzz_storage_snapshot_duration_seconds").set(cached.duration.as_secs_f64());

    let snapshot = &cached.data;
    metrics::gauge!("buzz_total_storage_bytes", "kind" => "physical")
        .set(snapshot.physical_bytes as f64);
    metrics::gauge!("buzz_total_storage_objects", "kind" => "physical")
        .set(snapshot.physical_objects as f64);
    metrics::gauge!("buzz_total_storage_bytes", "kind" => "logical")
        .set(snapshot.logical_bytes as f64);
    metrics::gauge!("buzz_total_storage_objects", "kind" => "logical")
        .set(snapshot.logical_objects as f64);

    metrics::gauge!("buzz_storage_orphan_blob_bytes").set(snapshot.orphan_blob_bytes as f64);
    metrics::gauge!("buzz_storage_orphan_blobs").set(snapshot.orphan_blob_count as f64);
    metrics::gauge!("buzz_storage_orphan_sidecars").set(snapshot.orphan_sidecar_count as f64);
    metrics::gauge!("buzz_storage_multi_variant_shas").set(snapshot.multi_variant_shas as f64);
    metrics::gauge!("buzz_storage_multi_variant_bytes").set(snapshot.multi_variant_bytes as f64);
    metrics::gauge!("buzz_storage_unknown_key_bytes").set(snapshot.unknown_key_bytes as f64);
    metrics::gauge!("buzz_storage_unknown_key_objects").set(snapshot.unknown_key_objects as f64);
    metrics::gauge!("buzz_storage_snapshot_max_objects").set(cached.max_objects as f64);
    metrics::gauge!("buzz_storage_snapshot_cap_utilization")
        .set(snapshot.physical_objects as f64 / cached.max_objects as f64);

    let mut current = HashSet::new();
    let mut unmapped_bytes = 0u64;
    for (community_id, storage) in &snapshot.per_community {
        let Some(host) = host_map.get(community_id) else {
            unmapped_bytes += storage.bytes;
            continue;
        };
        if !allows(community_id) {
            continue;
        }
        metrics::gauge!("buzz_community_storage_bytes", "community" => host.clone())
            .set(storage.bytes as f64);
        metrics::gauge!("buzz_community_storage_objects", "community" => host.clone())
            .set(storage.objects as f64);
        current.insert(StorageEmittedKey::Bytes(host.clone()));
        current.insert(StorageEmittedKey::Objects(host.clone()));
    }
    metrics::gauge!("buzz_storage_unmapped_community_bytes").set(unmapped_bytes as f64);

    // Zero series for communities that were emitted last tick but are no longer
    // present in the current snapshot (community removed, host renamed, or
    // scope exclusion added).
    for key in state
        .previously_emitted
        .difference(&current)
        .cloned()
        .collect::<Vec<_>>()
    {
        key.set(0.0);
    }
    state.previously_emitted = current;
}

#[cfg(test)]
mod postgres_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use buzz_media::CommunityStorage;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    fn snapshot_with(community: Uuid, bytes: u64, objects: u64) -> BucketSnapshot {
        let mut per_community = HashMap::new();
        per_community.insert(community, CommunityStorage { bytes, objects });
        BucketSnapshot {
            physical_bytes: bytes,
            physical_objects: objects,
            logical_bytes: bytes,
            logical_objects: objects,
            per_community,
            ..Default::default()
        }
    }

    #[test]
    fn default_and_legacy_chart_values_use_worker_snapshots() {
        for value in [
            None,
            Some("inline"),
            Some("on"),
            Some("snapshot"),
            Some("external"),
            Some(" INLINE "),
        ] {
            assert_eq!(
                parse_storage_metrics_mode(value),
                StorageMetricsMode::Snapshot,
                "{value:?}"
            );
        }
        for value in [Some("off"), Some(" OFF "), Some("unknown"), Some("")] {
            assert_eq!(
                parse_storage_metrics_mode(value),
                StorageMetricsMode::Disabled,
                "{value:?}"
            );
        }
    }

    fn gauge_snapshot(recorder: &DebuggingRecorder) -> std::collections::HashMap<String, f64> {
        recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, _, _, _)| key.key().name().contains("_storage_"))
            .filter_map(|(key, _, _, value)| match value {
                DebugValue::Gauge(v) => Some((key.key().name().to_owned(), v.into_inner())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn missing_snapshot_emits_nothing() {
        let state = Mutex::new(StorageSweepState::default());
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &HashMap::new(),
                |_| true,
            ));
        });
        assert!(gauge_snapshot(&recorder).is_empty());
    }

    #[tokio::test]
    async fn disabled_reader_never_queries_a_closed_pool() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        pool.close().await;
        let db = Db::from_pool(pool);
        let state = Mutex::new(StorageSweepState::default());
        let recorder = DebuggingRecorder::new();
        let result = metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(run_storage_metrics_tick(
                &db,
                &state,
                StorageMetricsMode::Disabled,
                &HashMap::new(),
                |_| true,
            ))
        });
        assert!(result.is_ok());
        assert!(gauge_snapshot(&recorder).is_empty());
    }

    #[tokio::test]
    async fn failed_first_read_is_an_error_instead_of_worker_absence() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        pool.close().await;
        let db = Db::from_pool(pool);
        let state = Mutex::new(StorageSweepState::default());
        let recorder = DebuggingRecorder::new();
        let result = metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(run_storage_metrics_tick(
                &db,
                &state,
                StorageMetricsMode::Snapshot,
                &HashMap::new(),
                |_| true,
            ))
        });
        assert!(result.is_err());
        assert_eq!(
            gauge_snapshot(&recorder),
            HashMap::from([("buzz_storage_snapshot_load_ok".into(), 0.0)])
        );
    }

    #[tokio::test]
    async fn warm_cache_emits_community_and_unmapped_totals_with_scope_gating() {
        let mapped = Uuid::new_v4();
        let unmapped = Uuid::new_v4();
        let excluded = Uuid::new_v4();
        let mut per_community = HashMap::new();
        per_community.insert(
            mapped,
            CommunityStorage {
                bytes: 100,
                objects: 2,
            },
        );
        per_community.insert(
            unmapped,
            CommunityStorage {
                bytes: 30,
                objects: 1,
            },
        );
        per_community.insert(
            excluded,
            CommunityStorage {
                bytes: 7,
                objects: 1,
            },
        );
        let snapshot = BucketSnapshot {
            physical_bytes: 137,
            physical_objects: 4,
            logical_bytes: 137,
            logical_objects: 4,
            per_community,
            ..Default::default()
        };

        let state = Mutex::new(StorageSweepState {
            cached: Some(CachedSnapshot {
                data: snapshot,
                completed_at_wall: Utc::now(),
                duration: Duration::from_millis(500),
                max_objects: 1_000,
            }),
            persisted_load_ok: Some(true),
            ..Default::default()
        });

        let mut host_map = HashMap::new();
        host_map.insert(mapped, "mapped.example".to_string());
        host_map.insert(excluded, "excluded.example".to_string());

        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map,
                |id| *id != excluded,
            ));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_snapshot_load_ok"), Some(&1.0));
        assert_eq!(values.get("buzz_total_storage_bytes"), Some(&137.0));
        assert_eq!(
            values.get("buzz_storage_unmapped_community_bytes"),
            Some(&30.0)
        );
        assert_eq!(values.get("buzz_community_storage_bytes"), Some(&100.0));
    }

    // --- F-EXT1 regression: stale per-community series are zeroed ---

    /// Returns a map of `(metric_name, community_label_value) -> gauge_value`
    /// for gauges that carry a "community" label. Used to verify per-community
    /// series are zeroed rather than left stale between emissions.
    fn labeled_community_gauges(
        recorder: &DebuggingRecorder,
    ) -> std::collections::HashMap<(String, String), f64> {
        recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(composite_key, _, _, value)| {
                let DebugValue::Gauge(v) = value else {
                    return None;
                };
                let key = composite_key.key();
                let community = key
                    .labels()
                    .find(|l| l.key() == "community")
                    .map(|l| l.value().to_owned())?;
                Some(((key.name().to_owned(), community), v.into_inner()))
            })
            .collect()
    }

    #[tokio::test]
    async fn stale_per_community_series_are_zeroed_on_disappearance() {
        // Three scenarios in one state machine using a single StorageSweepState:
        // (a) community disappears from snapshot (mapped → no entry),
        // (b) host label rename (same UUID, different host string),
        // (c) scope removal (community excluded by the `allows` predicate).
        //
        // After the second emission, the old series from (a), (b), and (c)
        // must read 0.0, not their last nonzero value.

        let community_a = Uuid::new_v4(); // (a) will disappear from snapshot
        let community_b = Uuid::new_v4(); // (b) will be renamed host.old → host.new
        let community_c = Uuid::new_v4(); // (c) will be scope-excluded

        let make_snapshot = |include_a: bool, b_bytes: u64| {
            let mut per_community = HashMap::new();
            if include_a {
                per_community.insert(
                    community_a,
                    CommunityStorage {
                        bytes: 10,
                        objects: 1,
                    },
                );
            }
            per_community.insert(
                community_b,
                CommunityStorage {
                    bytes: b_bytes,
                    objects: 2,
                },
            );
            per_community.insert(
                community_c,
                CommunityStorage {
                    bytes: 7,
                    objects: 1,
                },
            );
            BucketSnapshot {
                physical_bytes: 10 + b_bytes + 7,
                physical_objects: 4,
                logical_bytes: 10 + b_bytes + 7,
                logical_objects: 4,
                per_community,
                ..Default::default()
            }
        };

        let state = Mutex::new(StorageSweepState {
            cached: Some(CachedSnapshot {
                data: make_snapshot(true, 20),
                completed_at_wall: Utc::now(),
                duration: Duration::from_millis(100),
                max_objects: 1_000,
            }),
            persisted_load_ok: Some(true),
            ..Default::default()
        });

        let recorder = DebuggingRecorder::new();

        // --- Emission 1: all three communities visible ---
        let mut host_map_1 = HashMap::new();
        host_map_1.insert(community_a, "host.a".to_string());
        host_map_1.insert(community_b, "host.old".to_string());
        host_map_1.insert(community_c, "host.c".to_string());
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map_1,
                |_| true,
            ));
        });
        {
            let labeled = labeled_community_gauges(&recorder);
            assert_eq!(
                labeled.get(&(
                    "buzz_community_storage_bytes".to_string(),
                    "host.a".to_string()
                )),
                Some(&10.0),
                "emission 1: host.a bytes should be 10"
            );
            assert_eq!(
                labeled.get(&(
                    "buzz_community_storage_bytes".to_string(),
                    "host.old".to_string()
                )),
                Some(&20.0),
                "emission 1: host.old bytes should be 20"
            );
        }

        // --- Emission 2: community_a gone, community_b renamed, community_c excluded ---
        {
            let mut guard = state.lock().await;
            guard.cached = Some(CachedSnapshot {
                data: make_snapshot(false, 20),
                completed_at_wall: Utc::now(),
                duration: Duration::from_millis(100),
                max_objects: 1_000,
            });
        }
        let mut host_map_2 = HashMap::new();
        // community_a absent from host_map → unmapped (gone from per-community series)
        host_map_2.insert(community_b, "host.new".to_string()); // renamed
        host_map_2.insert(community_c, "host.c".to_string());
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map_2,
                |id| *id != community_c, // (c) scope-excluded
            ));
        });

        let labeled = labeled_community_gauges(&recorder);

        // (a) community_a disappeared — old host.a series must be zeroed
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_bytes".to_string(),
                "host.a".to_string()
            )),
            Some(&0.0),
            "(a) disappeared community: host.a bytes must be zeroed"
        );
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_objects".to_string(),
                "host.a".to_string()
            )),
            Some(&0.0),
            "(a) disappeared community: host.a objects must be zeroed"
        );

        // (b) community_b renamed host.old → host.new — old series must be zeroed
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_bytes".to_string(),
                "host.old".to_string()
            )),
            Some(&0.0),
            "(b) host rename: host.old bytes must be zeroed"
        );
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_bytes".to_string(),
                "host.new".to_string()
            )),
            Some(&20.0),
            "(b) host rename: host.new bytes must be 20"
        );

        // (c) community_c scope-excluded — host.c series must be zeroed
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_bytes".to_string(),
                "host.c".to_string()
            )),
            Some(&0.0),
            "(c) scope removal: host.c bytes must be zeroed"
        );
        assert_eq!(
            labeled.get(&(
                "buzz_community_storage_objects".to_string(),
                "host.c".to_string()
            )),
            Some(&0.0),
            "(c) scope removal: host.c objects must be zeroed"
        );
    }

    #[tokio::test]
    async fn persisted_snapshot_emits_snapshot_health_without_attempt_health() {
        let community = Uuid::from_u128(77);
        let state = Mutex::new(StorageSweepState::default());
        cache_persisted_snapshot(
            &state,
            snapshot_with(community, 100, 25),
            Utc::now() - chrono::Duration::seconds(30),
            Duration::from_secs(12),
            100,
        )
        .await;
        let host_map = HashMap::from([(community, "example.test".to_string())]);
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map,
                |_| true,
            ));
        });
        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_snapshot_load_ok"), Some(&1.0));
        assert_eq!(
            values.get("buzz_storage_snapshot_duration_seconds"),
            Some(&12.0)
        );
        assert_eq!(
            values.get("buzz_storage_snapshot_max_objects"),
            Some(&100.0)
        );
        assert_eq!(
            values.get("buzz_storage_snapshot_cap_utilization"),
            Some(&0.25)
        );
        assert!(values["buzz_storage_snapshot_age_seconds"] >= 30.0);
        assert!(!values.contains_key("buzz_storage_sweep_ok"));
        assert!(!values.contains_key("buzz_storage_sweep_failures"));
        assert!(!values.contains_key("buzz_storage_sweep_duration_seconds"));
        assert!(!values.contains_key("buzz_storage_sweep_age_seconds"));
    }

    #[tokio::test]
    async fn stale_persisted_snapshot_remains_visible_without_claiming_attempt_success() {
        let community = Uuid::from_u128(78);
        let state = Mutex::new(StorageSweepState::default());
        cache_persisted_snapshot(
            &state,
            snapshot_with(community, 200, 50),
            Utc::now() - chrono::Duration::hours(48),
            Duration::from_secs(20),
            1_000,
        )
        .await;

        let host_map = HashMap::from([(community, "stale.example.test".to_string())]);
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map,
                |_| true,
            ));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_snapshot_load_ok"), Some(&1.0));
        assert_eq!(values.get("buzz_total_storage_bytes"), Some(&200.0));
        assert!(values["buzz_storage_snapshot_age_seconds"] >= 48.0 * 60.0 * 60.0);
        assert!(!values.contains_key("buzz_storage_sweep_ok"));
        assert!(!values.contains_key("buzz_storage_sweep_failures"));
    }

    #[tokio::test]
    async fn failed_persisted_load_keeps_last_good_totals_and_reports_unhealthy_handoff() {
        let community = Uuid::from_u128(79);
        let state = Mutex::new(StorageSweepState::default());
        cache_persisted_snapshot(
            &state,
            snapshot_with(community, 300, 75),
            Utc::now() - chrono::Duration::minutes(5),
            Duration::from_secs(30),
            1_000,
        )
        .await;
        record_persisted_snapshot_load_failure(&state).await;

        let host_map = HashMap::from([(community, "cached.example.test".to_string())]);
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
                StorageMetricsMode::Snapshot,
                &host_map,
                |_| true,
            ));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_snapshot_load_ok"), Some(&0.0));
        assert_eq!(values.get("buzz_total_storage_bytes"), Some(&300.0));
        assert!(values["buzz_storage_snapshot_age_seconds"] >= 5.0 * 60.0);
        assert!(!values.contains_key("buzz_storage_sweep_ok"));
        assert!(!values.contains_key("buzz_storage_sweep_failures"));
    }
}
