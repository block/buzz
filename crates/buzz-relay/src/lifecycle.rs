//! Fixed-schema evidence for the relay's earliest startup steps.
//!
//! These events are written directly to stderr because crypto, tracing,
//! configuration, and metrics setup can fail before the normal telemetry
//! stack exists. Values are closed enums; raw errors and secrets never enter
//! the lifecycle schema.

use std::{
    io::Write as _,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use uuid::Uuid;

const EVENT_NAME: &str = "buzz_process_lifecycle";
const SCHEMA_VERSION: u8 = 1;

/// A bounded early-startup phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    /// Process entry through a usable metrics listener.
    ProcessTelemetry,
    /// Install the process-wide rustls provider.
    CryptoInit,
    /// Install structured logging and optional OTLP tracing.
    TracingInit,
    /// Parse environment-backed configuration.
    ConfigLoad,
    /// Load and validate relay key material.
    KeyLoad,
    /// Install the Prometheus recorder and bind its listener.
    MetricsBind,
}

impl StartupPhase {
    /// The complete wire vocabulary.
    pub const ALL: [Self; 6] = [
        Self::ProcessTelemetry,
        Self::CryptoInit,
        Self::TracingInit,
        Self::ConfigLoad,
        Self::KeyLoad,
        Self::MetricsBind,
    ];

    /// Stable wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessTelemetry => "process_telemetry",
            Self::CryptoInit => "crypto_init",
            Self::TracingInit => "tracing_init",
            Self::ConfigLoad => "config_load",
            Self::KeyLoad => "key_load",
            Self::MetricsBind => "metrics_bind",
        }
    }
}

/// A bounded terminal status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleStatus {
    /// Required work completed.
    Succeeded,
    /// Optional work failed and startup may continue.
    Degraded,
    /// Required work failed.
    Failed,
    /// Control flow dropped the phase without an explicit terminal.
    Abandoned,
}

impl LifecycleStatus {
    #[cfg(test)]
    const ALL: [Self; 4] = [
        Self::Succeeded,
        Self::Degraded,
        Self::Failed,
        Self::Abandoned,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }
}

/// A secret-safe terminal reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleReason {
    /// Tokio runtime construction failed.
    RuntimeBuild,
    /// Another rustls provider was already installed.
    ProviderConflict,
    /// The optional OTLP exporter could not be built.
    ExporterBuild,
    /// Required configuration was missing, malformed, or unusable.
    ConfigInvalid,
    /// A required value was missing.
    Missing,
    /// A required value was invalid.
    RequiredInvalid,
    /// A required listener could not bind.
    Bind,
    /// A global metrics recorder already existed.
    RecorderConflict,
    /// A phase owner disappeared without a terminal.
    OwnerDropped,
    /// A panic unwound through the phase.
    Panic,
}

impl LifecycleReason {
    #[cfg(test)]
    const ALL: [Self; 10] = [
        Self::RuntimeBuild,
        Self::ProviderConflict,
        Self::ExporterBuild,
        Self::ConfigInvalid,
        Self::Missing,
        Self::RequiredInvalid,
        Self::Bind,
        Self::RecorderConflict,
        Self::OwnerDropped,
        Self::Panic,
    ];

    /// Stable wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBuild => "runtime_build",
            Self::ProviderConflict => "provider_conflict",
            Self::ExporterBuild => "exporter_build",
            Self::ConfigInvalid => "config_invalid",
            Self::Missing => "missing",
            Self::RequiredInvalid => "required_invalid",
            Self::Bind => "bind",
            Self::RecorderConflict => "recorder_conflict",
            Self::OwnerDropped => "owner_dropped",
            Self::Panic => "panic",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct LifecycleEvent {
    event_name: &'static str,
    schema_version: u8,
    process_boot_id: Uuid,
    sequence: u64,
    track: &'static str,
    phase: &'static str,
    edge: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    process_started_at_unix_ms: u64,
    observed_at_unix_ms: u64,
    process_elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    phase_elapsed_ms: Option<u64>,
}

trait EventWriter: Send + Sync {
    fn emit(&self, event: &LifecycleEvent);
}

struct StderrWriter;

impl EventWriter for StderrWriter {
    fn emit(&self, event: &LifecycleEvent) {
        // Best effort: reporting a startup error must never create another
        // panic. This sink intentionally ignores RUST_LOG filters.
        let mut stderr = std::io::stderr().lock();
        if serde_json::to_writer(&mut stderr, event).is_ok() {
            let _ = stderr.write_all(b"\n");
        }
    }
}

struct ProcessLifecycle {
    boot_id: Uuid,
    sequence: AtomicU64,
    wall_origin: SystemTime,
    monotonic_origin: Instant,
    writer: Arc<dyn EventWriter>,
}

impl ProcessLifecycle {
    fn new(writer: Arc<dyn EventWriter>) -> Arc<Self> {
        let wall_origin = SystemTime::now();
        let monotonic_origin = Instant::now();
        Arc::new(Self {
            boot_id: Uuid::new_v4(),
            sequence: AtomicU64::new(1),
            wall_origin,
            monotonic_origin,
            writer,
        })
    }

    fn start(self: &Arc<Self>, phase: StartupPhase) -> PhaseGuard {
        let started_at = if phase == StartupPhase::ProcessTelemetry {
            self.monotonic_origin
        } else {
            Instant::now()
        };
        self.emit(phase, "started", None, None, None);
        PhaseGuard {
            lifecycle: Arc::clone(self),
            phase,
            started_at,
            finished: false,
        }
    }

    fn emit(
        &self,
        phase: StartupPhase,
        edge: &'static str,
        status: Option<LifecycleStatus>,
        reason: Option<LifecycleReason>,
        elapsed: Option<Duration>,
    ) {
        self.writer.emit(&LifecycleEvent {
            event_name: EVENT_NAME,
            schema_version: SCHEMA_VERSION,
            process_boot_id: self.boot_id,
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            track: "startup",
            phase: phase.as_str(),
            edge,
            status: status.map(LifecycleStatus::as_str),
            reason: reason.map(LifecycleReason::as_str),
            process_started_at_unix_ms: millis_since_epoch(self.wall_origin),
            observed_at_unix_ms: millis_since_epoch(SystemTime::now()),
            process_elapsed_ms: saturating_millis(self.monotonic_origin.elapsed()),
            phase_elapsed_ms: elapsed.map(saturating_millis),
        });
    }
}

/// Owns one phase from its start event through exactly one terminal.
pub struct PhaseGuard {
    lifecycle: Arc<ProcessLifecycle>,
    phase: StartupPhase,
    started_at: Instant,
    finished: bool,
}

impl PhaseGuard {
    /// Record successful completion.
    pub fn succeed(self) {
        self.finish(LifecycleStatus::Succeeded, None);
    }

    /// Record an allowed degradation.
    pub fn degrade(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Degraded, Some(reason));
    }

    /// Record a fatal failure.
    pub fn fail(self, reason: LifecycleReason) {
        self.finish(LifecycleStatus::Failed, Some(reason));
    }

    fn finish(mut self, status: LifecycleStatus, reason: Option<LifecycleReason>) {
        let elapsed = self.started_at.elapsed();
        self.lifecycle
            .emit(self.phase, "terminal", Some(status), reason, Some(elapsed));
        self.finished = true;
    }
}

impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let (status, reason) = if std::thread::panicking() {
            (LifecycleStatus::Failed, LifecycleReason::Panic)
        } else {
            (LifecycleStatus::Abandoned, LifecycleReason::OwnerDropped)
        };
        self.lifecycle.emit(
            self.phase,
            "terminal",
            Some(status),
            Some(reason),
            Some(self.started_at.elapsed()),
        );
        self.finished = true;
    }
}

/// Tracks the aggregate early-startup phase and its fixed subphases.
pub struct BootTracker {
    lifecycle: Arc<ProcessLifecycle>,
    headline: PhaseGuard,
    degraded: Option<LifecycleReason>,
}

impl BootTracker {
    /// Start lifecycle accounting before constructing Tokio.
    pub fn start_before_runtime<Runtime, Error>(
        build: impl FnOnce() -> Result<Runtime, Error>,
    ) -> Result<(Runtime, Self), Error> {
        Self::start_before_runtime_with_writer(Arc::new(StderrWriter), build)
    }

    fn start_before_runtime_with_writer<Runtime, Error>(
        writer: Arc<dyn EventWriter>,
        build: impl FnOnce() -> Result<Runtime, Error>,
    ) -> Result<(Runtime, Self), Error> {
        let lifecycle = ProcessLifecycle::new(writer);
        let boot = Self {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        match build() {
            Ok(runtime) => Ok((runtime, boot)),
            Err(error) => {
                boot.fail(LifecycleReason::RuntimeBuild);
                Err(error)
            }
        }
    }

    /// Start a fixed early-startup subphase.
    #[must_use = "dropping a phase guard emits an abandoned terminal"]
    pub fn start(&self, phase: StartupPhase) -> PhaseGuard {
        assert_ne!(phase, StartupPhase::ProcessTelemetry);
        self.lifecycle.start(phase)
    }

    /// Run a required phase and atomically terminalize both it and startup on failure.
    pub fn run_required<T, Error>(
        self,
        phase: StartupPhase,
        work: impl FnOnce() -> Result<T, Error>,
        classify: impl FnOnce(&Error) -> LifecycleReason,
    ) -> Result<(Self, T), Error> {
        let phase_guard = self.start(phase);
        match work() {
            Ok(value) => {
                phase_guard.succeed();
                Ok((self, value))
            }
            Err(error) => {
                let reason = classify(&error);
                phase_guard.fail(reason);
                self.fail(reason);
                Err(error)
            }
        }
    }

    /// Preserve the first optional degradation for the aggregate terminal.
    pub fn mark_degraded(&mut self, reason: LifecycleReason) {
        self.degraded.get_or_insert(reason);
    }

    /// Finish early startup with a structured lifecycle terminal.
    pub fn finish(self) {
        let status = if self.degraded.is_some() {
            LifecycleStatus::Degraded
        } else {
            LifecycleStatus::Succeeded
        };
        self.headline.finish(status, self.degraded);
    }

    fn fail(self, reason: LifecycleReason) {
        self.headline.fail(reason);
    }
}

fn millis_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(saturating_millis)
        .unwrap_or(0)
}

fn saturating_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{panic::AssertUnwindSafe, sync::Mutex};

    #[derive(Default)]
    struct CapturingWriter(Mutex<Vec<LifecycleEvent>>);

    impl EventWriter for CapturingWriter {
        fn emit(&self, event: &LifecycleEvent) {
            self.0.lock().expect("capturing writer").push(event.clone());
        }
    }

    fn recorder() -> (Arc<ProcessLifecycle>, Arc<CapturingWriter>) {
        let writer = Arc::new(CapturingWriter::default());
        (ProcessLifecycle::new(writer.clone()), writer)
    }

    fn events(writer: &CapturingWriter) -> Vec<LifecycleEvent> {
        writer.0.lock().expect("capturing writer").clone()
    }

    #[test]
    fn explicit_and_dropped_terminals_are_exactly_once() {
        let (lifecycle, writer) = recorder();
        lifecycle.start(StartupPhase::ConfigLoad).succeed();
        drop(lifecycle.start(StartupPhase::KeyLoad));

        let events = events(&writer);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].status, Some("succeeded"));
        assert_eq!(events[3].status, Some("abandoned"));
        assert_eq!(events[3].reason, Some("owner_dropped"));
    }

    #[test]
    fn panic_unwind_is_bounded() {
        let (lifecycle, writer) = recorder();
        let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _phase = lifecycle.start(StartupPhase::CryptoInit);
            panic!("controlled test panic");
        }));
        assert!(panic.is_err());
        let events = events(&writer);
        assert_eq!(events[1].status, Some("failed"));
        assert_eq!(events[1].reason, Some("panic"));
    }

    #[test]
    fn runtime_failure_terminalizes_the_headline() {
        let writer = Arc::new(CapturingWriter::default());
        let result = BootTracker::start_before_runtime_with_writer(
            writer.clone(),
            || -> Result<(), &'static str> { Err("controlled") },
        );
        assert!(matches!(result, Err("controlled")));
        let events = events(&writer);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].phase, "process_telemetry");
        assert_eq!(events[1].reason, Some("runtime_build"));
    }

    #[test]
    fn aggregate_preserves_optional_degradation() {
        let (lifecycle, writer) = recorder();
        let mut boot = BootTracker {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        boot.mark_degraded(LifecycleReason::ExporterBuild);
        boot.finish();
        let events = events(&writer);
        assert_eq!(events[1].status, Some("degraded"));
        assert_eq!(events[1].reason, Some("exporter_build"));
    }

    #[test]
    fn required_failure_terminalizes_subphase_and_headline() {
        let (lifecycle, writer) = recorder();
        let boot = BootTracker {
            headline: lifecycle.start(StartupPhase::ProcessTelemetry),
            lifecycle,
            degraded: None,
        };
        let result = boot.run_required(
            StartupPhase::MetricsBind,
            || -> Result<(), &'static str> { Err("controlled") },
            |_error| LifecycleReason::RecorderConflict,
        );
        assert!(matches!(result, Err("controlled")));

        let events = events(&writer);
        assert_eq!(events.len(), 4);
        assert_eq!(events[2].phase, "metrics_bind");
        assert_eq!(events[2].status, Some("failed"));
        assert_eq!(events[2].reason, Some("recorder_conflict"));
        assert_eq!(events[3].phase, "process_telemetry");
        assert_eq!(events[3].status, Some("failed"));
        assert_eq!(events[3].reason, Some("recorder_conflict"));
    }

    #[test]
    fn schema_and_vocabulary_are_frozen() {
        assert_eq!(
            StartupPhase::ALL.map(StartupPhase::as_str),
            [
                "process_telemetry",
                "crypto_init",
                "tracing_init",
                "config_load",
                "key_load",
                "metrics_bind",
            ]
        );
        let (lifecycle, writer) = recorder();
        drop(lifecycle.start(StartupPhase::ConfigLoad));
        let values: Vec<_> = events(&writer)
            .iter()
            .map(|event| serde_json::to_value(event).expect("serialize lifecycle event"))
            .collect();
        assert_eq!(values[0]["schema_version"], SCHEMA_VERSION);
        assert_eq!(values[0]["event_name"], EVENT_NAME);
        assert_eq!(values[1]["status"], "abandoned");
        let mut started_keys: Vec<_> = values[0]
            .as_object()
            .expect("started event object")
            .keys()
            .map(String::as_str)
            .collect();
        started_keys.sort_unstable();
        assert_eq!(
            started_keys,
            [
                "edge",
                "event_name",
                "observed_at_unix_ms",
                "phase",
                "process_boot_id",
                "process_elapsed_ms",
                "process_started_at_unix_ms",
                "schema_version",
                "sequence",
                "track",
            ]
        );
        let mut terminal_keys: Vec<_> = values[1]
            .as_object()
            .expect("terminal event object")
            .keys()
            .map(String::as_str)
            .collect();
        terminal_keys.sort_unstable();
        assert_eq!(
            terminal_keys,
            [
                "edge",
                "event_name",
                "observed_at_unix_ms",
                "phase",
                "phase_elapsed_ms",
                "process_boot_id",
                "process_elapsed_ms",
                "process_started_at_unix_ms",
                "reason",
                "schema_version",
                "sequence",
                "status",
                "track",
            ]
        );
        assert_eq!(
            LifecycleStatus::ALL.map(LifecycleStatus::as_str),
            ["succeeded", "degraded", "failed", "abandoned",]
        );
        assert_eq!(
            LifecycleReason::ALL.map(LifecycleReason::as_str),
            [
                "runtime_build",
                "provider_conflict",
                "exporter_build",
                "config_invalid",
                "missing",
                "required_invalid",
                "bind",
                "recorder_conflict",
                "owner_dropped",
                "panic",
            ]
        );
    }
}
