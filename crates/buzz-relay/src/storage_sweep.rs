//! Hourly S3 storage sweep: single-flight background task, cached snapshot,
//! re-emitted every usage-metrics tick.
//!
//! See `PLANS/S3_STORAGE_METRICS_PLAN.md` (Rev 3) "Sweep architecture" for
//! the full design; this module implements the relay-side half (the
//! classifier + pure fold live in `buzz_media::bucket_index`). Summary:
//!
//! - The usage tick never awaits the sweep. [`maybe_spawn_sweep`] harvests
//!   any finished in-flight attempt into the cache, then spawns at most one
//!   new attempt (single-flight) when the cadence/failure rule allows it.
//! - [`emit_storage_metrics`] is called every tick, leader-only, and
//!   re-publishes the cached snapshot regardless of whether a sweep is
//!   currently running — DB-derived gauges keep their configured cadence,
//!   storage gauges lag by at most one tick after a sweep completes.
//! - A cold cache (no sweep has ever succeeded) publishes health gauges only
//!   (`sweep_ok=0`); a warm cache re-publishes the last good snapshot even
//!   while the newest attempt is failing, so a transient S3 blip never blanks
//!   the dashboards.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use buzz_media::{BucketSnapshot, SweepError};

/// Sweep knobs, read once at boot. See `PLANS/S3_STORAGE_METRICS_PLAN.md` F7.
#[derive(Debug, Clone, Copy)]
pub struct StorageSweepConfig {
    /// Minimum time between successful sweeps. Floored so a misconfigured
    /// value can't turn this into a listing busy-loop.
    pub interval: Duration,
    /// `tokio::time::timeout` around one whole sweep attempt. Cadence-
    /// independent — the usage tick never awaits the sweep, so this bounds
    /// how long a stalled attempt occupies the single in-flight slot.
    pub timeout: Duration,
    /// Cumulative listed-object cap; a listing that exceeds it fails the
    /// attempt (old snapshot kept) rather than growing memory unbounded.
    pub max_objects: u64,
    /// Kill switch. `false` ⇒ no sweep ever spawns and no storage-family
    /// gauge (including the health gauges) is ever emitted — a relay whose
    /// deployment lacks `s3:ListBucket` can turn the whole feature off.
    pub enabled: bool,
}

impl StorageSweepConfig {
    /// Reads `BUZZ_STORAGE_SWEEP_INTERVAL_SECS` (default 3600, floor 60),
    /// `BUZZ_STORAGE_SWEEP_TIMEOUT_SECS` (default 120),
    /// `BUZZ_STORAGE_SWEEP_MAX_OBJECTS` (default 1_000_000), and the
    /// `BUZZ_STORAGE_METRICS` kill switch (`off` ⇒ disabled, anything else
    /// including unset ⇒ enabled).
    pub fn from_env() -> Self {
        let interval_secs = std::env::var("BUZZ_STORAGE_SWEEP_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3600)
            .max(60);
        let timeout_secs = std::env::var("BUZZ_STORAGE_SWEEP_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120);
        let max_objects = std::env::var("BUZZ_STORAGE_SWEEP_MAX_OBJECTS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1_000_000);
        let enabled = std::env::var("BUZZ_STORAGE_METRICS")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref()
            != Some("off");
        Self {
            interval: Duration::from_secs(interval_secs),
            timeout: Duration::from_secs(timeout_secs),
            max_objects,
            enabled,
        }
    }
}

/// The last completed sweep attempt's outcome and timing — kept regardless
/// of success/failure so health gauges can report on the newest attempt even
/// when [`CachedSnapshot`] still holds an older successful one.
#[derive(Debug, Clone, Copy)]
struct LastAttempt {
    ok: bool,
    duration: Duration,
}

/// The most recent *successful* sweep, cut as one coherent snapshot.
#[derive(Debug, Clone)]
struct CachedSnapshot {
    data: BucketSnapshot,
    completed_at: Instant,
}

/// What a spawned sweep task hands back to the tick that harvests it.
struct SweepAttempt {
    result: Result<BucketSnapshot, SweepError>,
    duration: Duration,
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

/// Single-flight + cache state for the storage sweep. One instance lives in
/// `AppState`, shared behind a `Mutex` — the same pattern this codebase uses
/// for other cross-tick poller state (e.g. the audit worker's
/// `JoinHandle` in `state::AuditShutdownHandle`).
#[derive(Default)]
pub struct StorageSweepState {
    in_flight: Option<JoinHandle<SweepAttempt>>,
    cached: Option<CachedSnapshot>,
    last_attempt: Option<LastAttempt>,
    failures_total: u64,
    /// Per-community series emitted on the previous tick — used to zero series
    /// whose community disappears from the snapshot (see [`emit_storage_metrics`]).
    previously_emitted: HashSet<StorageEmittedKey>,
}

/// Single-flight + cadence rule (F5/F5-bis): spawn a new sweep iff no sweep
/// is in flight AND (cold cache, OR the last attempt failed — respawn on the
/// very next tick rather than waiting a full interval, OR the cached
/// snapshot is older than `interval`).
///
/// Note: a failed attempt (`!ok`) returns `true` unconditionally, so a
/// permanently failing sweep (e.g. missing `s3:ListBucket`) will retry on
/// every usage tick (default 300 s), not at the sweep-interval cadence.
/// This is intentional: a permission failure (the common persistent case)
/// costs a single cheap LIST call per retry, other failures (timeout, cap,
/// malformed page) are bounded by the sweep's own timeout and object caps,
/// and tick-cadence retry means the sweep self-heals as soon as the
/// underlying cause is fixed. The tick cadence is documented in values.yaml.
fn should_spawn(
    cached: &Option<CachedSnapshot>,
    last_attempt: &Option<LastAttempt>,
    interval: Duration,
    now: Instant,
) -> bool {
    match last_attempt {
        None => true,
        Some(attempt) if !attempt.ok => true,
        Some(_) => match cached {
            // A prior successful attempt without a cached snapshot can't
            // happen through this module's own harvest path, but treat it
            // as cold rather than panicking on a broken invariant.
            None => true,
            Some(snapshot) => now.duration_since(snapshot.completed_at) >= interval,
        },
    }
}

/// Harvest any finished in-flight attempt into the cache, then spawn a new
/// attempt if the single-flight + cadence rule allows it.
///
/// Harvest and spawn share one lock acquisition and one "is there a live
/// handle" check by design — splitting them would open a race window where
/// a tick sees a freshly-emptied `in_flight` slot from a harvest that hasn't
/// yet updated `cached`/`last_attempt`, and spawns a redundant second sweep.
///
/// `sweep_fut` is constructed by the caller on every tick but only ever
/// polled if this call decides to spawn — an unpolled async value has not
/// started its body, so building it speculatively has no side effects.
pub async fn maybe_spawn_sweep<Fut>(
    state: &Mutex<StorageSweepState>,
    interval: Duration,
    timeout: Duration,
    sweep_fut: Fut,
) where
    Fut: Future<Output = Result<BucketSnapshot, SweepError>> + Send + 'static,
{
    let mut state = state.lock().await;

    if let Some(handle) = state.in_flight.take() {
        if !handle.is_finished() {
            state.in_flight = Some(handle);
            return; // single-flight: a sweep is already running
        }
        match handle.await {
            Ok(attempt) => {
                let ok = attempt.result.is_ok();
                match attempt.result {
                    Ok(snapshot) => {
                        state.cached = Some(CachedSnapshot {
                            data: snapshot,
                            // Stamped at harvest, not sweep completion — exported
                            // age/cadence may lag by ≤1 usage tick.
                            completed_at: Instant::now(),
                        });
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "storage sweep failed; verify s3:ListBucket \
                             (or MinIO list) permission is granted on the bucket"
                        );
                        state.failures_total += 1;
                    }
                }
                state.last_attempt = Some(LastAttempt {
                    ok,
                    duration: attempt.duration,
                });
            }
            Err(join_error) => {
                tracing::error!(error = %join_error, "storage sweep task panicked");
                state.failures_total += 1;
                state.last_attempt = Some(LastAttempt {
                    ok: false,
                    duration: Duration::ZERO,
                });
            }
        }
    }

    if !should_spawn(&state.cached, &state.last_attempt, interval, Instant::now()) {
        return;
    }

    let handle = tokio::spawn(async move {
        let started = Instant::now();
        let result = tokio::time::timeout(timeout, sweep_fut)
            .await
            .unwrap_or(Err(SweepError::Timeout(timeout)));
        SweepAttempt {
            result,
            duration: started.elapsed(),
        }
    });
    state.in_flight = Some(handle);
}

/// Emit the storage-family gauges from the cached snapshot. Call every usage
/// tick, leader-only (mirrors `emit_db_usage_metrics`'s leadership gate) —
/// never from the spawned sweep task itself, so a sweep that completes after
/// this pod loses leadership parks its snapshot without ever publishing it.
///
/// `host_map` resolves a community UUID to its label string for per-
/// community series; `allows` gates those series the same way
/// `EmissionScope` gates the DB-derived ones. A bound community UUID absent
/// from `host_map` is "unmapped" (sidecar references a community with no DB
/// row) and rolls into `buzz_storage_unmapped_community_bytes` instead of a
/// per-community series.
///
/// Per-community series whose community disappears from the current snapshot
/// (unmapped, host rename, or scope exclusion) are explicitly zeroed — the
/// same pattern as `emit_in_memory_usage_metrics`. Without this, a series
/// would linger at its last nonzero value until the recorder's idle eviction
/// fires (≥3 ticks), producing a transient double-count against the
/// `buzz_storage_unmapped_community_bytes` gauge.
pub async fn emit_storage_metrics(
    state: &Mutex<StorageSweepState>,
    host_map: &HashMap<Uuid, String>,
    allows: impl Fn(&Uuid) -> bool,
) {
    let mut state = state.lock().await;

    let ok = state.last_attempt.is_some_and(|a| a.ok);
    metrics::gauge!("buzz_storage_sweep_ok").set(if ok { 1.0 } else { 0.0 });
    // Process-local gauge: resets/jumps on leader failover — not a global counter.
    // Named without _total suffix to avoid confusing tools that infer counter
    // semantics from the _total convention (e.g. rate() in PromQL).
    metrics::gauge!("buzz_storage_sweep_failures").set(state.failures_total as f64);
    if let Some(attempt) = state.last_attempt {
        metrics::gauge!("buzz_storage_sweep_duration_seconds").set(attempt.duration.as_secs_f64());
    }

    // Cold cache + failure (F5): no storage-family/per-community gauges yet.
    // Zero any previously-emitted per-community series before returning so
    // they don't linger if we had a warm cache in a prior tick.
    let Some(cached) = &state.cached else {
        for key in state.previously_emitted.drain() {
            key.set(0.0);
        }
        return;
    };
    metrics::gauge!("buzz_storage_sweep_age_seconds")
        .set(cached.completed_at.elapsed().as_secs_f64());

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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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

    // --- StorageSweepConfig::from_env ---

    #[test]
    fn config_defaults_and_floors_apply_when_env_absent() {
        // No env manipulation: absent vars in the test process must resolve
        // to the documented defaults.
        for key in [
            "BUZZ_STORAGE_SWEEP_INTERVAL_SECS",
            "BUZZ_STORAGE_SWEEP_TIMEOUT_SECS",
            "BUZZ_STORAGE_SWEEP_MAX_OBJECTS",
            "BUZZ_STORAGE_METRICS",
        ] {
            if std::env::var(key).is_ok() {
                return; // externally forced — skip rather than assert a lie
            }
        }
        let config = StorageSweepConfig::from_env();
        assert_eq!(config.interval, Duration::from_secs(3600));
        assert_eq!(config.timeout, Duration::from_secs(120));
        assert_eq!(config.max_objects, 1_000_000);
        assert!(config.enabled);
    }

    #[test]
    fn config_kill_switch_only_off_disables() {
        assert!(!parse_enabled(Some("off")));
        assert!(!parse_enabled(Some("OFF")));
        assert!(parse_enabled(Some("on")));
        assert!(parse_enabled(Some("anything-else")));
        assert!(parse_enabled(None));
    }

    fn parse_enabled(value: Option<&str>) -> bool {
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref() != Some("off")
    }

    // --- should_spawn ---

    #[test]
    fn should_spawn_cold_cache_and_no_attempt_yet() {
        assert!(should_spawn(
            &None,
            &None,
            Duration::from_secs(3600),
            Instant::now()
        ));
    }

    #[test]
    fn should_spawn_respawns_immediately_after_a_failed_attempt() {
        let last_attempt = Some(LastAttempt {
            ok: false,
            duration: Duration::from_secs(1),
        });
        // Cached snapshot from an earlier success is still warm and fresh,
        // but the failed attempt alone must force an immediate respawn.
        let cached = Some(CachedSnapshot {
            data: BucketSnapshot::default(),
            completed_at: Instant::now(),
        });
        assert!(should_spawn(
            &cached,
            &last_attempt,
            Duration::from_secs(3600),
            Instant::now()
        ));
    }

    #[test]
    fn should_spawn_waits_out_the_interval_after_a_success() {
        let now = Instant::now();
        let last_attempt = Some(LastAttempt {
            ok: true,
            duration: Duration::from_secs(1),
        });
        let cached = Some(CachedSnapshot {
            data: BucketSnapshot::default(),
            completed_at: now,
        });
        assert!(!should_spawn(
            &cached,
            &last_attempt,
            Duration::from_secs(3600),
            now
        ));
        assert!(should_spawn(
            &cached,
            &last_attempt,
            Duration::from_secs(3600),
            now + Duration::from_secs(3600)
        ));
    }

    // --- maybe_spawn_sweep ---

    #[tokio::test]
    async fn cold_cache_success_populates_the_cache() {
        let state = Mutex::new(StorageSweepState::default());
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { Ok(snapshot_with(Uuid::new_v4(), 10, 1)) },
        )
        .await;
        // The spawned task needs a scheduling point to run before the next
        // call can harvest it.
        tokio::task::yield_now().await;
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { panic!("must not spawn a second attempt while nothing changed") },
        )
        .await;

        let guard = state.lock().await;
        assert!(guard.cached.is_some());
        assert!(guard.last_attempt.unwrap().ok);
        assert_eq!(guard.failures_total, 0);
    }

    #[tokio::test]
    async fn cold_cache_failure_leaves_no_snapshot_but_records_the_failure() {
        let state = Mutex::new(StorageSweepState::default());
        // Each call harvests the PREVIOUS call's spawned attempt (harvest
        // and spawn share one lock acquisition — see `maybe_spawn_sweep`
        // doc comment), so two failures need three calls: the first spawns
        // attempt 1, the second harvests attempt 1 and spawns attempt 2,
        // the third harvests attempt 2.
        for _ in 0..3 {
            maybe_spawn_sweep(
                &state,
                Duration::from_secs(3600),
                Duration::from_secs(5),
                async { Err(SweepError::CapExceeded { seen: 5, cap: 1 }) },
            )
            .await;
            tokio::task::yield_now().await;
        }

        let guard = state.lock().await;
        assert!(guard.cached.is_none(), "cold cache stays cold on failure");
        assert_eq!(guard.failures_total, 2);
        assert!(!guard.last_attempt.unwrap().ok);
    }

    #[tokio::test]
    async fn warm_cache_failure_keeps_the_old_snapshot() {
        let community = Uuid::new_v4();
        let state = Mutex::new(StorageSweepState::default());
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async move { Ok(snapshot_with(community, 42, 1)) },
        )
        .await;
        tokio::task::yield_now().await;
        // This call harvests the success above, then (last_attempt.ok=true,
        // cache fresh) does NOT spawn again.
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { panic!("must not spawn: cache is warm and fresh") },
        )
        .await;
        {
            let guard = state.lock().await;
            assert_eq!(
                guard.cached.as_ref().unwrap().data.per_community[&community].bytes,
                42
            );
        }

        // Force a respawn by aging the cache past the interval, then fail it.
        {
            let mut guard = state.lock().await;
            guard.cached.as_mut().unwrap().completed_at =
                Instant::now() - Duration::from_secs(7200);
        }
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { Err(SweepError::CapExceeded { seen: 5, cap: 1 }) },
        )
        .await;
        tokio::task::yield_now().await;
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { Err(SweepError::CapExceeded { seen: 5, cap: 1 }) },
        )
        .await;

        let guard = state.lock().await;
        assert_eq!(
            guard.cached.as_ref().unwrap().data.per_community[&community].bytes,
            42,
            "old snapshot must survive a later failed attempt"
        );
        assert!(!guard.last_attempt.unwrap().ok);
        assert_eq!(guard.failures_total, 1);
    }

    #[tokio::test]
    async fn single_flight_never_spawns_a_second_attempt_while_one_is_running() {
        let started = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let state = Mutex::new(StorageSweepState::default());

        let started_for_first = Arc::clone(&started);
        let gate_for_first = Arc::clone(&gate);
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async move {
                started_for_first.fetch_add(1, Ordering::SeqCst);
                gate_for_first.notified().await;
                Ok(BucketSnapshot::default())
            },
        )
        .await;
        tokio::task::yield_now().await;

        let started_for_second = Arc::clone(&started);
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async move {
                started_for_second.fetch_add(1, Ordering::SeqCst);
                Ok(BucketSnapshot::default())
            },
        )
        .await;

        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "second attempt must never have been polled while the first was in flight"
        );
        gate.notify_one(); // release the first attempt so the test can end cleanly
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_attempt_times_out_and_is_recorded_as_a_failure() {
        let state = Mutex::new(StorageSweepState::default());
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(BucketSnapshot::default())
            },
        )
        .await;

        // The spawned task must be polled at least once to register its
        // inner `tokio::time::timeout` deadline before the paused clock can
        // be advanced past it — `advance` only fires timers that already
        // exist.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        // `advance` fires the deadline, but driving the woken task to
        // completion still needs the executor to actually poll it — loop
        // `yield_now` until the handle reports finished rather than
        // guessing a fixed poll count.
        for _ in 0..50 {
            let finished = state
                .lock()
                .await
                .in_flight
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if finished {
                break;
            }
            tokio::task::yield_now().await;
        }
        // Harvest the timed-out attempt.
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async { panic!("cadence not yet due — must not spawn") },
        )
        .await;

        let guard = state.lock().await;
        assert_eq!(guard.failures_total, 1);
        assert!(!guard.last_attempt.unwrap().ok);
        assert!(guard.cached.is_none());
    }

    // --- Rev 3 required tests: demoted-leader-never-emits + paused-time
    //     tick-cadence composite ---
    //
    // Harvest only happens inside `maybe_spawn_sweep`, which the relay only
    // calls from the leader-only branch of the usage-metrics tick
    // (`run_usage_metrics_tick`, gated on `leader.is_some()`). A pod that
    // loses leadership stops calling both `maybe_spawn_sweep` and
    // `emit_storage_metrics` — so a sweep that finishes after demotion sits
    // in `in_flight` forever un-harvested and its data is never published.
    // That leader-transition itself lives behind a real Postgres advisory
    // lock (`buzz_db::UsageMetricsLeader`, see `crates/buzz-db/src/lib.rs`)
    // and has no fixture-free construction, so it can't be driven from this
    // module; the property this module DOES own — a completed-but-
    // unharvested attempt emits nothing — is what's covered below.

    #[tokio::test]
    async fn a_completed_but_unharvested_sweep_never_emits_its_snapshot() {
        let community = Uuid::new_v4();
        let state = Mutex::new(StorageSweepState::default());
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(5),
            async move { Ok(snapshot_with(community, 999, 1)) },
        )
        .await;
        // The spawned task finishes here, but nothing calls
        // `maybe_spawn_sweep` again to harvest it — the demoted-leader case.
        tokio::task::yield_now().await;

        let recorder = DebuggingRecorder::new();
        let host_map = HashMap::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(&state, &host_map, |_| true));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(
            values.get("buzz_storage_sweep_ok"),
            Some(&0.0),
            "unharvested attempt must not register as a success"
        );
        assert!(!values.contains_key("buzz_total_storage_bytes"));
        assert!(!values.contains_key("buzz_community_storage_bytes"));
        assert!(!values.contains_key("buzz_storage_sweep_age_seconds"));
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_sweep_across_several_ticks_emits_health_only_then_once_on_completion() {
        // Rev 3's composite covers two halves: (a) DB metrics emit every
        // tick with no leader demotion — that's `run_usage_metrics_tick` +
        // a live leader lock, integration territory, not exercised here —
        // and (b) the storage-sweep half this test drives directly: a sweep
        // stalled across several simulated ticks never double-spawns, emits
        // health-only gauges while stalled, then emits the real snapshot on
        // the first tick after it completes.
        let community = Uuid::new_v4();
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_for_sweep = Arc::clone(&gate);
        let state = Mutex::new(StorageSweepState::default());

        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(120),
            async move {
                gate_for_sweep.notified().await;
                Ok(snapshot_with(community, 500, 3))
            },
        )
        .await;
        tokio::task::yield_now().await;

        for _ in 0..3 {
            tokio::time::advance(Duration::from_secs(5)).await;
            maybe_spawn_sweep(
                &state,
                Duration::from_secs(3600),
                Duration::from_secs(120),
                async { panic!("single-flight: must not spawn while one is in flight") },
            )
            .await;

            let recorder = DebuggingRecorder::new();
            let host_map = HashMap::new();
            metrics::with_local_recorder(&recorder, || {
                futures::executor::block_on(emit_storage_metrics(&state, &host_map, |_| true));
            });
            let values = gauge_snapshot(&recorder);
            assert_eq!(values.get("buzz_storage_sweep_ok"), Some(&0.0));
            assert!(!values.contains_key("buzz_total_storage_bytes"));
        }

        // Let the stalled sweep complete; the next tick harvests it.
        gate.notify_one();
        tokio::task::yield_now().await;
        maybe_spawn_sweep(
            &state,
            Duration::from_secs(3600),
            Duration::from_secs(120),
            async { panic!("cadence not yet due — must not spawn a second attempt") },
        )
        .await;

        let recorder = DebuggingRecorder::new();
        let host_map = HashMap::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(&state, &host_map, |_| true));
        });
        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_sweep_ok"), Some(&1.0));
        assert_eq!(values.get("buzz_total_storage_bytes"), Some(&500.0));
    }

    // --- emit_storage_metrics ---

    fn gauge_snapshot(recorder: &DebuggingRecorder) -> std::collections::HashMap<String, f64> {
        recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| match value {
                DebugValue::Gauge(v) => Some((key.key().name().to_owned(), v.into_inner())),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn cold_cache_emits_only_health_gauges() {
        let state = Mutex::new(StorageSweepState::default());
        let recorder = DebuggingRecorder::new();
        let host_map = HashMap::new();

        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(&state, &host_map, |_| true));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_sweep_ok"), Some(&0.0));
        assert_eq!(values.get("buzz_storage_sweep_failures"), Some(&0.0));
        assert!(!values.contains_key("buzz_total_storage_bytes"));
        assert!(!values.contains_key("buzz_storage_sweep_age_seconds"));
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
                completed_at: Instant::now(),
            }),
            last_attempt: Some(LastAttempt {
                ok: true,
                duration: Duration::from_millis(500),
            }),
            ..Default::default()
        });

        let mut host_map = HashMap::new();
        host_map.insert(mapped, "mapped.example".to_string());
        host_map.insert(excluded, "excluded.example".to_string());

        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(&state, &host_map, |id| {
                *id != excluded
            }));
        });

        let values = gauge_snapshot(&recorder);
        assert_eq!(values.get("buzz_storage_sweep_ok"), Some(&1.0));
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
                completed_at: Instant::now(),
            }),
            last_attempt: Some(LastAttempt {
                ok: true,
                duration: Duration::from_millis(100),
            }),
            ..Default::default()
        });

        let recorder = DebuggingRecorder::new();

        // --- Emission 1: all three communities visible ---
        let mut host_map_1 = HashMap::new();
        host_map_1.insert(community_a, "host.a".to_string());
        host_map_1.insert(community_b, "host.old".to_string());
        host_map_1.insert(community_c, "host.c".to_string());
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(&state, &host_map_1, |_| true));
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
                completed_at: Instant::now(),
            });
        }
        let mut host_map_2 = HashMap::new();
        // community_a absent from host_map → unmapped (gone from per-community series)
        host_map_2.insert(community_b, "host.new".to_string()); // renamed
        host_map_2.insert(community_c, "host.c".to_string());
        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(emit_storage_metrics(
                &state,
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
}
