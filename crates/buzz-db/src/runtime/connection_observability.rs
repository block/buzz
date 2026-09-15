//! Fixed-vocabulary evidence for writer-pool connection setup.
//!
//! SQLx exposes the point immediately after a physical connection succeeds,
//! but it does not expose a callback immediately before each physical dial.
//! Consequently, `physical_connect` is a success milestone rather than a
//! duration phase. The aggregate `writer_pool` phase owns failures that occur
//! before `after_connect`, while the session phases own their exact failures.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Database pool roles with connection-lifecycle coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbPoolRole {
    /// The authoritative relay writer pool.
    Writer,
}

impl DbPoolRole {
    /// Stable metric/log value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Writer => "writer",
        }
    }
}

/// Fixed writer connection-setup steps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbConnectionStep {
    /// Construct the writer pool and satisfy its initial minimum size.
    WriterPool,
    /// A physical connection has completed DNS/network/TLS/authentication.
    PhysicalConnect,
    /// Install the created-at replica-fence floor.
    CreatedAtFloor,
    /// Install lock, idle-transaction, and statement timeouts.
    SessionTimeouts,
    /// Verify READ COMMITTED transaction isolation.
    Isolation,
    /// The physical connection passed every required session premise.
    Ready,
}

impl DbConnectionStep {
    /// Complete wire vocabulary.
    pub const ALL: [Self; 6] = [
        Self::WriterPool,
        Self::PhysicalConnect,
        Self::CreatedAtFloor,
        Self::SessionTimeouts,
        Self::Isolation,
        Self::Ready,
    ];

    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WriterPool => "writer_pool",
            Self::PhysicalConnect => "physical_connect",
            Self::CreatedAtFloor => "created_at_floor",
            Self::SessionTimeouts => "session_timeouts",
            Self::Isolation => "isolation",
            Self::Ready => "ready",
        }
    }

    /// Stable process-lifecycle phase.
    pub const fn lifecycle_phase(self) -> &'static str {
        match self {
            Self::WriterPool => "db_writer_pool",
            Self::PhysicalConnect => "db_physical_connect",
            Self::CreatedAtFloor => "db_created_at_floor",
            Self::SessionTimeouts => "db_session_timeouts",
            Self::Isolation => "db_isolation",
            Self::Ready => "db_ready",
        }
    }
}

/// Lifecycle edge for one connection-setup step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbConnectionEdge {
    /// Work on a measurable phase began.
    Started,
    /// Work reached a bounded terminal.
    Terminal,
}

impl DbConnectionEdge {
    /// Stable log value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Terminal => "terminal",
        }
    }
}

/// Bounded terminal outcome for a connection-setup step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbConnectionOutcome {
    /// The step completed successfully.
    Succeeded,
    /// The step failed.
    Failed,
    /// The aggregate pool deadline expired.
    TimedOut,
    /// The owning future was dropped before a terminal.
    Cancelled,
}

impl DbConnectionOutcome {
    /// Stable metric/log value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Secret-safe reason for a failed connection-setup step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DbConnectionReason {
    /// A generic network I/O error; the SQLx seam cannot safely narrow it.
    ConnectIo,
    /// TLS negotiation or certificate validation failed.
    Tls,
    /// PostgreSQL rejected authentication.
    Authentication,
    /// PostgreSQL rejected the connection for another bounded reason.
    ServerReject,
    /// A required session-setting query failed.
    SessionSetup,
    /// The effective transaction isolation was not READ COMMITTED.
    IsolationMismatch,
    /// SQLx exhausted the pool/connect deadline.
    Timeout,
    /// The pool was closed.
    PoolClosed,
    /// A phase guard was dropped with no explicit terminal.
    OwnerDropped,
    /// A panic unwound through a phase.
    Panic,
    /// No narrower safe classification exists.
    Unknown,
}

impl DbConnectionReason {
    /// Stable log value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConnectIo => "connect_io",
            Self::Tls => "tls",
            Self::Authentication => "authentication",
            Self::ServerReject => "server_reject",
            Self::SessionSetup => "session_setup",
            Self::IsolationMismatch => "isolation_mismatch",
            Self::Timeout => "timeout",
            Self::PoolClosed => "pool_closed",
            Self::OwnerDropped => "owner_dropped",
            Self::Panic => "panic",
            Self::Unknown => "unknown",
        }
    }
}

/// One fixed-schema connection lifecycle event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DbConnectionLifecycleEvent {
    pool_role: DbPoolRole,
    connection_ordinal: Option<u64>,
    step: DbConnectionStep,
    edge: DbConnectionEdge,
    outcome: Option<DbConnectionOutcome>,
    reason: Option<DbConnectionReason>,
    elapsed: Option<Duration>,
}

impl DbConnectionLifecycleEvent {
    /// Pool role.
    pub const fn pool_role(self) -> DbPoolRole {
        self.pool_role
    }

    /// Process-local connection ordinal, absent for the aggregate pool phase.
    pub const fn connection_ordinal(self) -> Option<u64> {
        self.connection_ordinal
    }

    /// Connection-setup step.
    pub const fn step(self) -> DbConnectionStep {
        self.step
    }

    /// Lifecycle edge.
    pub const fn edge(self) -> DbConnectionEdge {
        self.edge
    }

    /// Terminal outcome, absent from start edges.
    pub const fn outcome(self) -> Option<DbConnectionOutcome> {
        self.outcome
    }

    /// Secret-safe terminal reason.
    pub const fn reason(self) -> Option<DbConnectionReason> {
        self.reason
    }

    /// Phase duration, absent from the physical-connect and ready milestones.
    pub const fn elapsed(self) -> Option<Duration> {
        self.elapsed
    }
}

/// Sink for fixed-schema database connection lifecycle events.
pub trait DbConnectionObserver: Send + Sync {
    /// Record one event.
    fn record(&self, event: DbConnectionLifecycleEvent);
}

#[derive(Default)]
pub(crate) struct NoopDbConnectionObserver;

impl DbConnectionObserver for NoopDbConnectionObserver {
    fn record(&self, _event: DbConnectionLifecycleEvent) {}
}

pub(crate) type SharedDbConnectionObserver = Arc<dyn DbConnectionObserver>;

/// Valid role/step pairs with an explicit start counter.
pub const CONNECTION_STARTED_STEPS: [(DbPoolRole, DbConnectionStep); 4] = [
    (DbPoolRole::Writer, DbConnectionStep::WriterPool),
    (DbPoolRole::Writer, DbConnectionStep::CreatedAtFloor),
    (DbPoolRole::Writer, DbConnectionStep::SessionTimeouts),
    (DbPoolRole::Writer, DbConnectionStep::Isolation),
];

/// Valid role/step pairs with a duration histogram.
pub const CONNECTION_DURATION_STEPS: [(DbPoolRole, DbConnectionStep); 4] = CONNECTION_STARTED_STEPS;

/// Valid role/step/outcome terminal combinations.
pub const CONNECTION_TERMINALS: [(DbPoolRole, DbConnectionStep, DbConnectionOutcome); 15] = [
    (
        DbPoolRole::Writer,
        DbConnectionStep::WriterPool,
        DbConnectionOutcome::Succeeded,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::WriterPool,
        DbConnectionOutcome::Failed,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::WriterPool,
        DbConnectionOutcome::TimedOut,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::WriterPool,
        DbConnectionOutcome::Cancelled,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::PhysicalConnect,
        DbConnectionOutcome::Succeeded,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::CreatedAtFloor,
        DbConnectionOutcome::Succeeded,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::CreatedAtFloor,
        DbConnectionOutcome::Failed,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::CreatedAtFloor,
        DbConnectionOutcome::Cancelled,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::SessionTimeouts,
        DbConnectionOutcome::Succeeded,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::SessionTimeouts,
        DbConnectionOutcome::Failed,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::SessionTimeouts,
        DbConnectionOutcome::Cancelled,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::Isolation,
        DbConnectionOutcome::Succeeded,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::Isolation,
        DbConnectionOutcome::Failed,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::Isolation,
        DbConnectionOutcome::Cancelled,
    ),
    (
        DbPoolRole::Writer,
        DbConnectionStep::Ready,
        DbConnectionOutcome::Succeeded,
    ),
];

/// Four start counters + four 13-series histograms + fifteen terminal counters.
pub const CONNECTION_RAW_SERIES_PER_POD: usize = 4 + (4 * 13) + 15;

pub(crate) struct DbConnectionStepAttempt {
    observer: SharedDbConnectionObserver,
    pool_role: DbPoolRole,
    connection_ordinal: Option<u64>,
    step: DbConnectionStep,
    started: Instant,
    finished: bool,
}

impl DbConnectionStepAttempt {
    pub(crate) fn start(
        observer: SharedDbConnectionObserver,
        pool_role: DbPoolRole,
        connection_ordinal: Option<u64>,
        step: DbConnectionStep,
    ) -> Self {
        metrics::counter!(
            "buzz_db_connection_step_started_total",
            "pool_role" => pool_role.as_str(),
            "step" => step.as_str(),
        )
        .increment(1);
        observer.record(DbConnectionLifecycleEvent {
            pool_role,
            connection_ordinal,
            step,
            edge: DbConnectionEdge::Started,
            outcome: None,
            reason: None,
            elapsed: None,
        });
        Self {
            observer,
            pool_role,
            connection_ordinal,
            step,
            started: Instant::now(),
            finished: false,
        }
    }

    pub(crate) fn succeed(self) {
        self.finish(DbConnectionOutcome::Succeeded, None);
    }

    pub(crate) fn fail(self, reason: DbConnectionReason) {
        self.finish(DbConnectionOutcome::Failed, Some(reason));
    }

    pub(crate) fn time_out(self) {
        self.finish(
            DbConnectionOutcome::TimedOut,
            Some(DbConnectionReason::Timeout),
        );
    }

    fn finish(mut self, outcome: DbConnectionOutcome, reason: Option<DbConnectionReason>) {
        let elapsed = self.started.elapsed();
        record_terminal(
            &self.observer,
            self.pool_role,
            self.connection_ordinal,
            self.step,
            outcome,
            reason,
            Some(elapsed),
        );
        self.finished = true;
    }
}

impl Drop for DbConnectionStepAttempt {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let (outcome, reason) = if std::thread::panicking() {
            (DbConnectionOutcome::Failed, DbConnectionReason::Panic)
        } else {
            (
                DbConnectionOutcome::Cancelled,
                DbConnectionReason::OwnerDropped,
            )
        };
        record_terminal(
            &self.observer,
            self.pool_role,
            self.connection_ordinal,
            self.step,
            outcome,
            Some(reason),
            Some(self.started.elapsed()),
        );
        self.finished = true;
    }
}

pub(crate) fn record_milestone(
    observer: &SharedDbConnectionObserver,
    pool_role: DbPoolRole,
    connection_ordinal: u64,
    step: DbConnectionStep,
) {
    record_terminal(
        observer,
        pool_role,
        Some(connection_ordinal),
        step,
        DbConnectionOutcome::Succeeded,
        None,
        None,
    );
}

fn record_terminal(
    observer: &SharedDbConnectionObserver,
    pool_role: DbPoolRole,
    connection_ordinal: Option<u64>,
    step: DbConnectionStep,
    outcome: DbConnectionOutcome,
    reason: Option<DbConnectionReason>,
    elapsed: Option<Duration>,
) {
    metrics::counter!(
        "buzz_db_connection_step_attempts_total",
        "pool_role" => pool_role.as_str(),
        "step" => step.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
    if let Some(elapsed) = elapsed {
        metrics::histogram!(
            "buzz_db_connection_step_duration_seconds",
            "pool_role" => pool_role.as_str(),
            "step" => step.as_str(),
        )
        .record(elapsed.as_secs_f64());
    }
    observer.record(DbConnectionLifecycleEvent {
        pool_role,
        connection_ordinal,
        step,
        edge: DbConnectionEdge::Terminal,
        outcome: Some(outcome),
        reason,
        elapsed,
    });
}

pub(crate) fn classify_pool_error(
    error: &sqlx::Error,
) -> (DbConnectionOutcome, DbConnectionReason) {
    match error {
        sqlx::Error::PoolTimedOut => (DbConnectionOutcome::TimedOut, DbConnectionReason::Timeout),
        sqlx::Error::PoolClosed => (DbConnectionOutcome::Failed, DbConnectionReason::PoolClosed),
        sqlx::Error::Io(_) => (DbConnectionOutcome::Failed, DbConnectionReason::ConnectIo),
        sqlx::Error::Tls(_) => (DbConnectionOutcome::Failed, DbConnectionReason::Tls),
        sqlx::Error::Database(error)
            if matches!(error.code().as_deref(), Some("28P01" | "28000")) =>
        {
            (
                DbConnectionOutcome::Failed,
                DbConnectionReason::Authentication,
            )
        }
        sqlx::Error::Database(_) => (
            DbConnectionOutcome::Failed,
            DbConnectionReason::ServerReject,
        ),
        _ => (DbConnectionOutcome::Failed, DbConnectionReason::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingObserver(Mutex<Vec<DbConnectionLifecycleEvent>>);

    impl DbConnectionObserver for CapturingObserver {
        fn record(&self, event: DbConnectionLifecycleEvent) {
            self.0.lock().expect("capture DB event").push(event);
        }
    }

    #[test]
    fn vocabulary_and_series_budget_are_frozen() {
        assert_eq!(
            DbConnectionStep::ALL.map(DbConnectionStep::as_str),
            [
                "writer_pool",
                "physical_connect",
                "created_at_floor",
                "session_timeouts",
                "isolation",
                "ready",
            ]
        );
        assert_eq!(CONNECTION_RAW_SERIES_PER_POD, 71);
    }

    #[test]
    fn dropped_step_is_cancelled_exactly_once_without_sensitive_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let observer = Arc::new(CapturingObserver::default());
        let attempt = DbConnectionStepAttempt::start(
            observer.clone(),
            DbPoolRole::Writer,
            Some(7),
            DbConnectionStep::CreatedAtFloor,
        );
        drop(attempt);

        let events = observer.0.lock().expect("read DB events").clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].edge(), DbConnectionEdge::Started);
        assert_eq!(events[1].edge(), DbConnectionEdge::Terminal);
        assert_eq!(events[1].outcome(), Some(DbConnectionOutcome::Cancelled));
        assert_eq!(events[1].reason(), Some(DbConnectionReason::OwnerDropped));

        let metrics = snapshotter.snapshot().into_vec();
        assert_eq!(metrics.len(), 3);
        for (key, _, _, value) in metrics {
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(labels.get("pool_role"), Some(&"writer"));
            assert_eq!(labels.get("step"), Some(&"created_at_floor"));
            assert!(!labels.contains_key("reason"));
            assert!(!labels.contains_key("connection_ordinal"));
            match value {
                DebugValue::Counter(value) => assert_eq!(value, 1),
                DebugValue::Histogram(values) => assert_eq!(values.len(), 1),
                DebugValue::Gauge(_) => panic!("connection lifecycle has no gauges"),
            }
        }
    }

    #[test]
    fn pool_errors_use_only_bounded_classes() {
        assert_eq!(
            classify_pool_error(&sqlx::Error::PoolTimedOut),
            (DbConnectionOutcome::TimedOut, DbConnectionReason::Timeout)
        );
        assert_eq!(
            classify_pool_error(&sqlx::Error::PoolClosed),
            (DbConnectionOutcome::Failed, DbConnectionReason::PoolClosed)
        );
        let io = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "postgres://secret-user:secret-password@example.invalid/private",
        ));
        assert_eq!(
            classify_pool_error(&io),
            (DbConnectionOutcome::Failed, DbConnectionReason::ConnectIo)
        );
    }
}
