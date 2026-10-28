//! Fixed-vocabulary metrics for writer-pool connection setup.
//!
//! SQLx exposes the point immediately after a physical connection succeeds,
//! but it does not expose a callback immediately before each physical dial.
//! Consequently, `physical_connect` is a success milestone rather than a
//! duration phase. The aggregate `writer_pool` phase owns failures that occur
//! before `after_connect`, while the session phases own their exact failures.

use std::time::{Duration, Instant};

use super::DbPoolRole;

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
    /// Complete metric-label vocabulary.
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
    /// Stable metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }
}

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
    pool_role: DbPoolRole,
    step: DbConnectionStep,
    started: Instant,
    finished: bool,
}

impl DbConnectionStepAttempt {
    pub(crate) fn start(pool_role: DbPoolRole, step: DbConnectionStep) -> Self {
        metrics::counter!(
            "buzz_db_connection_step_started_total",
            "pool_role" => pool_role.as_str(),
            "step" => step.as_str(),
        )
        .increment(1);
        Self {
            pool_role,
            step,
            started: Instant::now(),
            finished: false,
        }
    }

    pub(crate) fn succeed(self) {
        self.finish(DbConnectionOutcome::Succeeded);
    }

    pub(crate) fn fail(self) {
        self.finish(DbConnectionOutcome::Failed);
    }

    pub(crate) fn time_out(self) {
        self.finish(DbConnectionOutcome::TimedOut);
    }

    fn finish(mut self, outcome: DbConnectionOutcome) {
        record_terminal(
            self.pool_role,
            self.step,
            outcome,
            Some(self.started.elapsed()),
        );
        self.finished = true;
    }
}

impl Drop for DbConnectionStepAttempt {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let outcome = if std::thread::panicking() {
            DbConnectionOutcome::Failed
        } else {
            DbConnectionOutcome::Cancelled
        };
        record_terminal(
            self.pool_role,
            self.step,
            outcome,
            Some(self.started.elapsed()),
        );
        self.finished = true;
    }
}

pub(crate) fn record_milestone(pool_role: DbPoolRole, step: DbConnectionStep) {
    record_terminal(pool_role, step, DbConnectionOutcome::Succeeded, None);
}

fn record_terminal(
    pool_role: DbPoolRole,
    step: DbConnectionStep,
    outcome: DbConnectionOutcome,
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
}

pub(crate) fn classify_pool_outcome(error: &sqlx::Error) -> DbConnectionOutcome {
    if matches!(error, sqlx::Error::PoolTimedOut) {
        DbConnectionOutcome::TimedOut
    } else {
        DbConnectionOutcome::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

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
        let attempt =
            DbConnectionStepAttempt::start(DbPoolRole::Writer, DbConnectionStep::CreatedAtFloor);
        drop(attempt);

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
                DebugValue::Counter(value) => {
                    assert_eq!(value, 1);
                    if key.key().name() == "buzz_db_connection_step_attempts_total" {
                        assert_eq!(labels.get("outcome"), Some(&"cancelled"));
                    }
                }
                DebugValue::Histogram(values) => assert_eq!(values.len(), 1),
                DebugValue::Gauge(_) => panic!("connection setup has no gauges"),
            }
        }
    }

    #[test]
    fn pool_timeout_is_distinguished_from_other_failures() {
        assert_eq!(
            classify_pool_outcome(&sqlx::Error::PoolTimedOut),
            DbConnectionOutcome::TimedOut
        );
        assert_eq!(
            classify_pool_outcome(&sqlx::Error::PoolClosed),
            DbConnectionOutcome::Failed
        );
        let io = sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "postgres://secret-user:secret-password@example.invalid/private",
        ));
        assert_eq!(classify_pool_outcome(&io), DbConnectionOutcome::Failed);
    }
}
