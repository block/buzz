//! Timing for the relay's startup work between metrics bind and serving.
//!
//! [`crate::lifecycle`] covers the earliest phases, which run before the
//! metrics exporter exists, as log-only records. Everything after
//! `metrics_bind` runs with a working recorder and structured logging, so
//! each step logs `Startup phase finished` (`phase`, `elapsed_ms`, `status`)
//! and sets two gauges:
//!
//! - `buzz_startup_phase_current{phase}` is `1` while the step runs and `0`
//!   once it ends, so a pod that is still starting shows which step it is on.
//! - `buzz_startup_phase_seconds{phase}` is the step's wall-clock duration,
//!   set once when the step ends.
//!
//! The `phase` label values are a closed vocabulary ([`StartupStep::ALL`]).
//! They never overlap the early lifecycle phase names.

use std::time::Instant;

use tracing::{info, warn};

/// Gauge that is `1` while a startup step is running.
pub const CURRENT_METRIC: &str = "buzz_startup_phase_current";
/// Gauge holding a finished startup step's duration in seconds.
pub const SECONDS_METRIC: &str = "buzz_startup_phase_seconds";

/// Register the startup-step gauge descriptions.
pub(crate) fn describe_metrics() {
    metrics::describe_gauge!(
        CURRENT_METRIC,
        "1 while the labelled post-metrics-bind startup phase is running, 0 once it ends"
    );
    metrics::describe_gauge!(
        SECONDS_METRIC,
        metrics::Unit::Seconds,
        "Wall-clock duration of the labelled post-metrics-bind startup phase"
    );
}

/// One timed unit of startup work after the metrics listener is up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupStep {
    /// Connect the Postgres writer (and lazy reader) pools.
    DbConnect,
    /// Apply database migrations (only when `BUZZ_AUTO_MIGRATE` is on).
    DbMigrate,
    /// Ensure or audit future event partitions.
    PartitionEnsure,
    /// Verify the community-deletion serving fences.
    DeletionFenceVerify,
    /// Verify the replica floor guard and start the fence probe.
    ReplicaFenceProbe,
    /// Ensure the deployment community, backfill the allowlist, and bootstrap the owner.
    MembershipBootstrap,
    /// Backfill NIP-33 `d_tag` values.
    DTagBackfill,
    /// Connect the audit database pool (only when audit is enabled).
    AuditConnect,
    /// Connect Redis commands and pub/sub.
    RedisConnect,
    /// Connect the search database pool.
    SearchConnect,
    /// Warm NIP-FI JWKS snapshots (only when NIP-FI is configured).
    NipFiJwksWarm,
    /// Start the inter-relay mesh (no-op when the mesh is off).
    MeshBoot,
    /// Run the git object-store conformance probe (when enabled).
    GitConformanceProbe,
    /// Verify the channel roster fence.
    ChannelRosterFence,
    /// Repair large NIP-29 channel roster snapshots.
    LargeRosterReconcile,
    /// Reconcile every community's NIP-43 membership snapshot.
    Nip43Reconcile,
    /// Bind the private health listener.
    HealthBind,
    /// Bind the public relay listener.
    ListenerBind,
}

impl StartupStep {
    /// The complete `phase` label vocabulary.
    pub const ALL: [Self; 18] = [
        Self::DbConnect,
        Self::DbMigrate,
        Self::PartitionEnsure,
        Self::DeletionFenceVerify,
        Self::ReplicaFenceProbe,
        Self::MembershipBootstrap,
        Self::DTagBackfill,
        Self::AuditConnect,
        Self::RedisConnect,
        Self::SearchConnect,
        Self::NipFiJwksWarm,
        Self::MeshBoot,
        Self::GitConformanceProbe,
        Self::ChannelRosterFence,
        Self::LargeRosterReconcile,
        Self::Nip43Reconcile,
        Self::HealthBind,
        Self::ListenerBind,
    ];

    /// Stable `phase` label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DbConnect => "db_connect",
            Self::DbMigrate => "db_migrate",
            Self::PartitionEnsure => "partition_ensure",
            Self::DeletionFenceVerify => "deletion_fence_verify",
            Self::ReplicaFenceProbe => "replica_fence_probe",
            Self::MembershipBootstrap => "membership_bootstrap",
            Self::DTagBackfill => "d_tag_backfill",
            Self::AuditConnect => "audit_connect",
            Self::RedisConnect => "redis_connect",
            Self::SearchConnect => "search_connect",
            Self::NipFiJwksWarm => "nip_fi_jwks_warm",
            Self::MeshBoot => "mesh_boot",
            Self::GitConformanceProbe => "git_conformance_probe",
            Self::ChannelRosterFence => "channel_roster_fence",
            Self::LargeRosterReconcile => "large_roster_reconcile",
            Self::Nip43Reconcile => "nip43_reconcile",
            Self::HealthBind => "health_bind",
            Self::ListenerBind => "listener_bind",
        }
    }
}

/// How a startup step ended (same values as `buzz_process_lifecycle` `status`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepStatus {
    Succeeded,
    /// Non-fatal error; startup continued.
    Degraded,
    /// Dropped without [`StepTimer::finish`] — an early return or a panic.
    Failed,
}

impl StepStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

/// A running startup step. Ends exactly once: [`StepTimer::finish`] reports
/// `succeeded` (or `degraded` after [`StepTimer::degrade`]); dropping it
/// without finishing reports `failed`, which covers `?` returns and panics.
#[must_use = "a dropped step timer reports the step as failed"]
pub struct StepTimer {
    step: StartupStep,
    started: Instant,
    degraded: bool,
    ended: bool,
}

impl StepTimer {
    /// Start timing `step` and mark it as the pod's current step.
    pub fn start(step: StartupStep) -> Self {
        metrics::gauge!(CURRENT_METRIC, "phase" => step.as_str()).set(1.0);
        info!(phase = step.as_str(), "Startup phase started");
        Self {
            step,
            started: Instant::now(),
            degraded: false,
            ended: false,
        }
    }

    /// Record that the step hit a non-fatal error; it will end as `degraded`.
    pub fn degrade(&mut self) {
        self.degraded = true;
    }

    /// End the step as `succeeded`, or `degraded` if [`Self::degrade`] was called.
    pub fn finish(mut self) {
        let status = if self.degraded {
            StepStatus::Degraded
        } else {
            StepStatus::Succeeded
        };
        self.end(status);
    }

    fn end(&mut self, status: StepStatus) {
        if self.ended {
            return;
        }
        self.ended = true;
        let elapsed = self.started.elapsed();
        let phase = self.step.as_str();
        metrics::gauge!(SECONDS_METRIC, "phase" => phase).set(elapsed.as_secs_f64());
        metrics::gauge!(CURRENT_METRIC, "phase" => phase).set(0.0);
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        match status {
            StepStatus::Succeeded => {
                info!(
                    phase,
                    elapsed_ms,
                    status = status.as_str(),
                    "Startup phase finished"
                )
            }
            StepStatus::Degraded | StepStatus::Failed => {
                warn!(
                    phase,
                    elapsed_ms,
                    status = status.as_str(),
                    "Startup phase finished"
                )
            }
        }
    }
}

impl Drop for StepTimer {
    fn drop(&mut self) {
        self.end(StepStatus::Failed);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;
    use crate::lifecycle::StartupPhase;

    /// Gauge values keyed by `(metric, phase)` after running `body`.
    ///
    /// Holds the tracing dispatch lock: `body` fires the step log callsites
    /// with no dispatcher, which must not race a `finished_records` capture.
    fn gauges(body: impl FnOnce()) -> HashMap<(String, String), f64> {
        let _tracing = crate::test_support::tracing_dispatch_lock();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, body);
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                let key = key.key();
                let phase = key
                    .labels()
                    .find(|l| l.key() == "phase")?
                    .value()
                    .to_owned();
                match value {
                    DebugValue::Gauge(v) => Some(((key.name().to_owned(), phase), v.into_inner())),
                    _ => None,
                }
            })
            .collect()
    }

    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// `(phase, status)` of every `Startup phase finished` record `body` emits.
    fn finished_records(body: impl FnOnce()) -> Vec<(String, String)> {
        let _tracing = crate::test_support::tracing_dispatch_lock();
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(logs.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = logs
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|record| record["fields"]["message"] == "Startup phase finished")
            .map(|record| {
                let field = |name: &str| record["fields"][name].as_str().unwrap_or("").to_owned();
                (field("phase"), field("status"))
            })
            .collect()
    }

    fn get(gauges: &HashMap<(String, String), f64>, metric: &str, phase: &str) -> Option<f64> {
        gauges.get(&(metric.to_owned(), phase.to_owned())).copied()
    }

    #[test]
    fn running_step_is_current_and_has_no_duration() {
        let mut held = None;
        let gauges = gauges(|| held = Some(StepTimer::start(StartupStep::Nip43Reconcile)));
        assert_eq!(get(&gauges, CURRENT_METRIC, "nip43_reconcile"), Some(1.0));
        assert_eq!(get(&gauges, SECONDS_METRIC, "nip43_reconcile"), None);
        std::mem::forget(held);
    }

    #[test]
    fn finished_step_clears_current_and_records_duration() {
        let gauges = gauges(|| {
            let step = StepTimer::start(StartupStep::DbConnect);
            std::thread::sleep(std::time::Duration::from_millis(5));
            step.finish();
        });
        assert_eq!(get(&gauges, CURRENT_METRIC, "db_connect"), Some(0.0));
        let seconds = get(&gauges, SECONDS_METRIC, "db_connect").expect("duration gauge");
        assert!(seconds >= 0.005, "duration {seconds} shorter than the step");
    }

    #[test]
    fn dropped_and_degraded_steps_clear_current_and_record_duration() {
        let gauges = gauges(|| {
            drop(StepTimer::start(StartupStep::RedisConnect));
            let mut step = StepTimer::start(StartupStep::PartitionEnsure);
            step.degrade();
            step.finish();
        });
        for phase in ["redis_connect", "partition_ensure"] {
            assert_eq!(get(&gauges, CURRENT_METRIC, phase), Some(0.0));
            assert!(get(&gauges, SECONDS_METRIC, phase).is_some());
        }
    }

    #[test]
    fn each_terminal_path_logs_exactly_one_finished_record_with_its_status() {
        let records = finished_records(|| {
            StepTimer::start(StartupStep::DbConnect).finish();
            let mut degraded = StepTimer::start(StartupStep::PartitionEnsure);
            degraded.degrade();
            degraded.finish();
            drop(StepTimer::start(StartupStep::RedisConnect));
        });
        let expected = [
            ("db_connect", "succeeded"),
            ("partition_ensure", "degraded"),
            ("redis_connect", "failed"),
        ]
        .map(|(phase, status)| (phase.to_owned(), status.to_owned()));
        assert_eq!(records, expected);
    }

    #[test]
    fn a_panic_inside_a_step_logs_it_failed() {
        let records = finished_records(|| {
            let result = std::panic::catch_unwind(|| {
                let _step = StepTimer::start(StartupStep::MeshBoot);
                panic!("step body panicked");
            });
            assert!(result.is_err());
        });
        assert_eq!(records, [("mesh_boot".to_owned(), "failed".to_owned())]);
    }

    #[test]
    fn vocabulary_is_unique_and_disjoint_from_early_lifecycle_phases() {
        let names: HashSet<_> = StartupStep::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names.len(), StartupStep::ALL.len());
        for phase in StartupPhase::ALL {
            assert!(
                !names.contains(phase.as_str()),
                "{} overlaps",
                phase.as_str()
            );
        }
    }
}
