//! Readiness-probe telemetry and the dependency diagnostics behind `/_status`.
//!
//! Readiness is deliberately *not* a dependency question. A shared Postgres or
//! Redis failure is shared by every replica, so evaluating it in the probe took
//! the whole deployment out of the load balancer at once and left a reconnect
//! burst with nowhere to land. The Kubernetes probe therefore answers from this
//! process's own lifecycle (see [`crate::router`]), and the same dependency
//! evaluation is reported on the diagnostic `/_status` endpoint, which is never
//! wired to a probe.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use buzz_db::{Db, DbError, DbReadinessOutcome};
use tokio::time::Instant;

const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);

/// Closed label set exported by `buzz_readiness_checks_total{reason}`.
///
/// Readiness answers a local lifecycle question, so this set cannot grow with
/// the number of shared dependencies the relay talks to.
#[cfg(test)]
pub(crate) const READINESS_REASON_LABELS: [&str; 2] = ["ready", "shutting_down"];

/// Maximum raw Prometheus series emitted by readiness and its dependency
/// diagnostics for one pod.
///
/// - 2 probe reasons
/// - 11 valid dependency/outcome pairs (Postgres 5, Redis 3, catalog 3)
/// - 4 histograms x (15 configured buckets + `+Inf` + count + sum) = 72
/// - 1 overall readiness gauge
#[cfg(test)]
pub(crate) const READINESS_RAW_SERIES_PER_POD: usize = 2 + 11 + (4 * 18) + 1;

/// Terminal outcome of one readiness probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadinessReason {
    Ready,
    ShuttingDown,
}

impl ReadinessReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ShuttingDown => "shutting_down",
        }
    }

    pub(crate) fn is_ready(self) -> bool {
        self == Self::Ready
    }
}

/// Records one readiness probe served by the private health listener.
///
/// `still_ready` re-reads process lifecycle *after* the gauge is written. That
/// ordering is the whole fence: `begin_shutdown` is one-way, so a probe that
/// sampled `Ready` immediately before it must not leave a stale ready gauge
/// behind for the rest of the drain. Re-reading before the write would reopen
/// the same window. Nothing else about a probe is shared, so this replaces the
/// generation-fenced coordinator the dependency probe used to require.
pub(crate) fn record_readiness_probe(reason: ReadinessReason, still_ready: impl FnOnce() -> bool) {
    metrics::counter!(
        "buzz_readiness_checks_total",
        "reason" => reason.label(),
    )
    .increment(1);
    record_overall_state(reason.is_ready());
    if !still_ready() {
        record_overall_state(false);
    }
}

/// Publishes the overall readiness gauge. Called by the probe and by terminal
/// shutdown, so a draining pod reports not-ready before its next scrape.
pub(crate) fn record_overall_state(ready: bool) {
    metrics::gauge!("buzz_readiness_state", "check" => "overall").set(if ready {
        1.0
    } else {
        0.0
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostgresOutcome {
    Success,
    PoolTimeout,
    PoolError,
    QueryTimeout,
    QueryError,
}

impl PostgresOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PoolTimeout => "pool_timeout",
            Self::PoolError => "pool_error",
            Self::QueryTimeout => "operation_timeout",
            Self::QueryError => "operation_error",
        }
    }

    fn is_success(self) -> bool {
        self == Self::Success
    }

    fn is_timeout(self) -> bool {
        matches!(self, Self::PoolTimeout | Self::QueryTimeout)
    }
}

impl From<DbReadinessOutcome> for PostgresOutcome {
    fn from(outcome: DbReadinessOutcome) -> Self {
        match outcome {
            DbReadinessOutcome::Success => Self::Success,
            DbReadinessOutcome::PoolTimeout => Self::PoolTimeout,
            DbReadinessOutcome::PoolError => Self::PoolError,
            DbReadinessOutcome::QueryTimeout => Self::QueryTimeout,
            DbReadinessOutcome::QueryError => Self::QueryError,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedisOutcome {
    Success,
    PoolTimeout,
    PoolError,
}

impl RedisOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PoolTimeout => "pool_timeout",
            Self::PoolError => "pool_error",
        }
    }

    fn is_success(self) -> bool {
        self == Self::Success
    }

    fn is_timeout(self) -> bool {
        self == Self::PoolTimeout
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletionCatalogOutcome {
    Success,
    OperationTimeout,
    OperationError,
}

impl DeletionCatalogOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::OperationTimeout => "operation_timeout",
            Self::OperationError => "operation_error",
        }
    }

    fn is_success(self) -> bool {
        self == Self::Success
    }

    fn is_timeout(self) -> bool {
        self == Self::OperationTimeout
    }
}

/// Aggregate dependency verdict reported in the `/_status` diagnostics body.
///
/// This is a diagnostic field, never a metric label: it exists so an operator
/// reading `/_status` gets the same one-line summary the readiness body used to
/// carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DependencyReason {
    Ready,
    PostgresPoolTimeout,
    PostgresPoolError,
    PostgresQueryTimeout,
    PostgresQueryError,
    RedisPoolTimeout,
    RedisPoolError,
    DeletionCatalogTimeout,
    DeletionCatalogError,
    OverallTimeout,
    MultipleDependenciesFailed,
}

impl DependencyReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::PostgresPoolTimeout => "postgres_pool_timeout",
            Self::PostgresPoolError => "postgres_pool_error",
            Self::PostgresQueryTimeout => "postgres_query_timeout",
            Self::PostgresQueryError => "postgres_query_error",
            Self::RedisPoolTimeout => "redis_pool_timeout",
            Self::RedisPoolError => "redis_pool_error",
            Self::DeletionCatalogTimeout => "deletion_catalog_timeout",
            Self::DeletionCatalogError => "deletion_catalog_error",
            Self::OverallTimeout => "overall_timeout",
            Self::MultipleDependenciesFailed => "multiple_dependencies_failed",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TimedOutcome<O> {
    outcome: O,
    duration: Duration,
}

impl<O> TimedOutcome<O> {
    #[cfg(test)]
    pub(crate) fn new(outcome: O, duration: Duration) -> Self {
        Self { outcome, duration }
    }
}

/// One completed dependency evaluation. Every dependency always runs, so the
/// report carries three outcomes and never a partial shape.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DependencyReport {
    postgres: TimedOutcome<PostgresOutcome>,
    redis: TimedOutcome<RedisOutcome>,
    deletion_catalog: TimedOutcome<DeletionCatalogOutcome>,
    pub(crate) reason: DependencyReason,
    total_duration: Duration,
}

impl DependencyReport {
    #[cfg(test)]
    pub(crate) fn from_results(
        postgres: TimedOutcome<PostgresOutcome>,
        redis: TimedOutcome<RedisOutcome>,
        deletion_catalog: TimedOutcome<DeletionCatalogOutcome>,
        total_duration: Duration,
    ) -> Self {
        Self::for_dependencies(postgres, redis, deletion_catalog, total_duration)
    }

    fn for_dependencies(
        postgres: TimedOutcome<PostgresOutcome>,
        redis: TimedOutcome<RedisOutcome>,
        deletion_catalog: TimedOutcome<DeletionCatalogOutcome>,
        total_duration: Duration,
    ) -> Self {
        let reason = final_reason(postgres.outcome, redis.outcome, deletion_catalog.outcome);
        Self {
            postgres,
            redis,
            deletion_catalog,
            reason,
            total_duration,
        }
    }

    pub(crate) fn postgres_ready(self) -> bool {
        self.postgres.outcome.is_success()
    }

    pub(crate) fn redis_ready(self) -> bool {
        self.redis.outcome.is_success()
    }

    pub(crate) fn deletion_catalog_ready(self) -> bool {
        self.deletion_catalog.outcome.is_success()
    }
}

fn final_reason(
    postgres: PostgresOutcome,
    redis: RedisOutcome,
    deletion_catalog: DeletionCatalogOutcome,
) -> DependencyReason {
    let failure_count = usize::from(!postgres.is_success())
        + usize::from(!redis.is_success())
        + usize::from(!deletion_catalog.is_success());

    if failure_count == 0 {
        return DependencyReason::Ready;
    }
    if failure_count > 1 {
        let all_failures_are_timeouts = (postgres.is_success() || postgres.is_timeout())
            && (redis.is_success() || redis.is_timeout())
            && (deletion_catalog.is_success() || deletion_catalog.is_timeout());
        return if all_failures_are_timeouts {
            DependencyReason::OverallTimeout
        } else {
            DependencyReason::MultipleDependenciesFailed
        };
    }

    match postgres {
        PostgresOutcome::PoolTimeout => DependencyReason::PostgresPoolTimeout,
        PostgresOutcome::PoolError => DependencyReason::PostgresPoolError,
        PostgresOutcome::QueryTimeout => DependencyReason::PostgresQueryTimeout,
        PostgresOutcome::QueryError => DependencyReason::PostgresQueryError,
        PostgresOutcome::Success => match redis {
            RedisOutcome::PoolTimeout => DependencyReason::RedisPoolTimeout,
            RedisOutcome::PoolError => DependencyReason::RedisPoolError,
            RedisOutcome::Success => match deletion_catalog {
                DeletionCatalogOutcome::OperationTimeout => {
                    DependencyReason::DeletionCatalogTimeout
                }
                DeletionCatalogOutcome::OperationError => DependencyReason::DeletionCatalogError,
                DeletionCatalogOutcome::Success => DependencyReason::Ready,
            },
        },
    }
}

async fn timed<F, O>(future: F) -> TimedOutcome<O>
where
    F: Future<Output = O>,
{
    let started_at = Instant::now();
    let outcome = future.await;
    TimedOutcome {
        outcome,
        duration: started_at.elapsed(),
    }
}

async fn evaluate_dependencies<P, R, D>(
    postgres: P,
    redis: R,
    deletion_catalog: D,
) -> DependencyReport
where
    P: Future<Output = PostgresOutcome>,
    R: Future<Output = RedisOutcome>,
    D: Future<Output = DeletionCatalogOutcome>,
{
    let started_at = Instant::now();
    let (postgres, redis, deletion_catalog) =
        tokio::join!(timed(postgres), timed(redis), timed(deletion_catalog),);
    DependencyReport::for_dependencies(postgres, redis, deletion_catalog, started_at.elapsed())
}

async fn redis_check(pool: &deadpool_redis::Pool, deadline: Instant) -> RedisOutcome {
    match tokio::time::timeout_at(deadline, pool.get()).await {
        Err(_) => RedisOutcome::PoolTimeout,
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "Redis readiness pool acquisition failed");
            RedisOutcome::PoolError
        }
        Ok(Ok(_connection)) => RedisOutcome::Success,
    }
}

async fn deletion_catalog_check(db: &Db, deadline: Instant) -> DeletionCatalogOutcome {
    classify_deletion_catalog_result(
        db.validate_deletion_serving_catalog_for_readiness(deadline)
            .await,
    )
}

fn classify_deletion_catalog_result(result: buzz_db::Result<()>) -> DeletionCatalogOutcome {
    match result {
        Err(DbError::Sqlx(sqlx::Error::PoolTimedOut)) => DeletionCatalogOutcome::OperationTimeout,
        Err(error) => {
            tracing::debug!(error = %error, "Deletion catalog readiness validation failed");
            DeletionCatalogOutcome::OperationError
        }
        Ok(()) => DeletionCatalogOutcome::Success,
    }
}

#[async_trait::async_trait]
pub(crate) trait DependencyEvaluator: Send + Sync {
    async fn evaluate(&self, db: &Db, redis_pool: &deadpool_redis::Pool) -> DependencyReport;
}

struct ProductionDependencyEvaluator;

#[async_trait::async_trait]
impl DependencyEvaluator for ProductionDependencyEvaluator {
    async fn evaluate(&self, db: &Db, redis_pool: &deadpool_redis::Pool) -> DependencyReport {
        let deadline = Instant::now() + DEPENDENCY_TIMEOUT;
        evaluate_dependencies(
            async { db.readiness_check(deadline).await.into() },
            redis_check(redis_pool, deadline),
            deletion_catalog_check(db, deadline),
        )
        .await
    }
}

/// Evaluates shared-dependency health for the diagnostic `/_status` endpoint.
///
/// This deliberately owns no publication fence. It publishes no gauge, so two
/// concurrent `/_status` requests cannot reorder any shared state — the fence
/// the readiness coordinator used to need went away with the dependency probe.
pub(crate) struct DependencyDiagnostics {
    evaluator: Arc<dyn DependencyEvaluator>,
}

impl Default for DependencyDiagnostics {
    fn default() -> Self {
        Self {
            evaluator: Arc::new(ProductionDependencyEvaluator),
        }
    }
}

impl DependencyDiagnostics {
    #[cfg(test)]
    pub(crate) fn with_evaluator(evaluator: Arc<dyn DependencyEvaluator>) -> Self {
        Self { evaluator }
    }

    /// Runs one bounded dependency evaluation and records its telemetry.
    pub(crate) async fn evaluate(
        &self,
        db: &Db,
        redis_pool: &deadpool_redis::Pool,
    ) -> DependencyReport {
        let report = self.evaluator.evaluate(db, redis_pool).await;
        record_dependency_report(&report);
        report
    }
}

/// Records one dependency evaluation. Counters and durations only — dependency
/// health has no publishable "current state" now that no probe consumes it, and
/// a gauge driven by ad-hoc `/_status` requests would read as authoritative
/// while going stale between operator visits.
fn record_dependency_report(report: &DependencyReport) {
    metrics::histogram!(
        "buzz_readiness_check_duration_seconds",
        "check" => "overall",
    )
    .record(report.total_duration.as_secs_f64());

    record_dependency_attempt(
        "postgres",
        report.postgres.outcome.label(),
        report.postgres.duration,
    );
    record_dependency_attempt("redis", report.redis.outcome.label(), report.redis.duration);
    record_dependency_attempt(
        "deletion_catalog",
        report.deletion_catalog.outcome.label(),
        report.deletion_catalog.duration,
    );
}

fn record_dependency_attempt(dependency: &'static str, outcome: &'static str, duration: Duration) {
    metrics::counter!(
        "buzz_readiness_dependency_checks_total",
        "dependency" => dependency,
        "outcome" => outcome,
    )
    .increment(1);
    metrics::histogram!(
        "buzz_readiness_check_duration_seconds",
        "check" => dependency,
    )
    .record(duration.as_secs_f64());
}

#[cfg(test)]
mod tests {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use metrics_util::CompositeKey;

    use super::*;

    type Snapshot = Vec<(
        CompositeKey,
        Option<metrics::Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    )>;

    fn redis_failure_report() -> DependencyReport {
        DependencyReport::from_results(
            TimedOutcome::new(PostgresOutcome::Success, Duration::from_millis(35)),
            TimedOutcome::new(RedisOutcome::PoolTimeout, Duration::from_secs(2)),
            TimedOutcome::new(DeletionCatalogOutcome::Success, Duration::from_millis(20)),
            Duration::from_secs(2),
        )
    }

    fn exact_metric<'a>(
        snapshot: &'a Snapshot,
        name: &str,
        labels: &[(&str, &str)],
    ) -> Option<&'a DebugValue> {
        snapshot.iter().find_map(|(key, _, _, value)| {
            let actual = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<Vec<_>>();
            (key.key().name() == name
                && actual.len() == labels.len()
                && labels.iter().all(|expected| actual.contains(expected)))
            .then_some(value)
        })
    }

    fn gauge_value(snapshot: &Snapshot, check: &str) -> f64 {
        let value = exact_metric(snapshot, "buzz_readiness_state", &[("check", check)])
            .expect("readiness gauge");
        let DebugValue::Gauge(value) = value else {
            panic!("readiness state must be a gauge");
        };
        value.into_inner()
    }

    #[tokio::test(start_paused = true)]
    async fn evaluation_preserves_a_completed_check_when_another_times_out() {
        let report = evaluate_dependencies(
            async {
                tokio::time::sleep(Duration::from_millis(35)).await;
                PostgresOutcome::Success
            },
            async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                RedisOutcome::PoolTimeout
            },
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                DeletionCatalogOutcome::Success
            },
        )
        .await;

        assert_eq!(report.reason, DependencyReason::RedisPoolTimeout);
        assert_eq!(report.postgres.duration, Duration::from_millis(35));
        assert_eq!(report.redis.duration, Duration::from_secs(2));
    }

    #[test]
    fn simultaneous_dependency_timeouts_are_an_overall_timeout() {
        assert_eq!(
            final_reason(
                PostgresOutcome::PoolTimeout,
                RedisOutcome::PoolTimeout,
                DeletionCatalogOutcome::Success,
            ),
            DependencyReason::OverallTimeout
        );
    }

    #[test]
    fn dependency_types_expose_only_valid_outcome_pairs() {
        assert_eq!(
            [
                PostgresOutcome::Success,
                PostgresOutcome::PoolTimeout,
                PostgresOutcome::PoolError,
                PostgresOutcome::QueryTimeout,
                PostgresOutcome::QueryError,
            ]
            .map(PostgresOutcome::label),
            [
                "success",
                "pool_timeout",
                "pool_error",
                "operation_timeout",
                "operation_error",
            ]
        );
        assert_eq!(
            [
                RedisOutcome::Success,
                RedisOutcome::PoolTimeout,
                RedisOutcome::PoolError,
            ]
            .map(RedisOutcome::label),
            ["success", "pool_timeout", "pool_error"]
        );
        assert_eq!(
            [
                DeletionCatalogOutcome::Success,
                DeletionCatalogOutcome::OperationTimeout,
                DeletionCatalogOutcome::OperationError,
            ]
            .map(DeletionCatalogOutcome::label),
            ["success", "operation_timeout", "operation_error"]
        );
        assert_eq!(READINESS_RAW_SERIES_PER_POD, 86);
    }

    #[test]
    fn deletion_catalog_deadline_is_a_timeout_not_an_operation_error() {
        assert_eq!(
            classify_deletion_catalog_result(Err(DbError::Sqlx(sqlx::Error::PoolTimedOut))),
            DeletionCatalogOutcome::OperationTimeout
        );
        assert_eq!(
            classify_deletion_catalog_result(Err(DbError::InvalidData("catalog".into()))),
            DeletionCatalogOutcome::OperationError
        );
    }

    /// The readiness gauge and counter follow lifecycle only. A dependency
    /// evaluation — however bad — must never move them, which is what let a
    /// shared outage deroute every replica at once.
    #[test]
    fn readiness_telemetry_tracks_lifecycle_and_dependency_failure_never_moves_it() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            record_readiness_probe(ReadinessReason::Ready, || true);
            record_dependency_report(&redis_failure_report());
        });
        let after_failure = snapshotter.snapshot().into_vec();

        assert_eq!(gauge_value(&after_failure, "overall"), 1.0);
        assert!(matches!(
            exact_metric(
                &after_failure,
                "buzz_readiness_checks_total",
                &[("reason", "ready")]
            ),
            Some(DebugValue::Counter(1))
        ));
        assert!(
            matches!(
                exact_metric(
                    &after_failure,
                    "buzz_readiness_dependency_checks_total",
                    &[("dependency", "redis"), ("outcome", "pool_timeout")]
                ),
                Some(DebugValue::Counter(1))
            ),
            "dependency diagnostics must still be counted"
        );
        for dependency in ["postgres", "redis", "deletion_catalog"] {
            assert!(
                exact_metric(
                    &after_failure,
                    "buzz_readiness_state",
                    &[("check", dependency)]
                )
                .is_none(),
                "{dependency} must not publish a readiness gauge"
            );
        }

        metrics::with_local_recorder(&recorder, || {
            record_readiness_probe(ReadinessReason::ShuttingDown, || false);
        });
        let after_shutdown = snapshotter.snapshot().into_vec();

        assert_eq!(gauge_value(&after_shutdown, "overall"), 0.0);
        assert!(matches!(
            exact_metric(
                &after_shutdown,
                "buzz_readiness_checks_total",
                &[("reason", "shutting_down")]
            ),
            Some(DebugValue::Counter(1))
        ));
    }

    /// The publication fence. A probe that sampled `Ready` a moment before
    /// `begin_shutdown` landed must not leave the gauge advertising ready for
    /// the rest of the drain. Deleting the post-write re-read fails the first
    /// case below.
    #[test]
    fn a_probe_that_raced_shutdown_cannot_leave_a_ready_gauge() {
        for (sampled, still_ready, expected, case) in [
            (
                ReadinessReason::Ready,
                false,
                0.0,
                "shutdown landed mid-probe",
            ),
            (ReadinessReason::Ready, true, 1.0, "no shutdown"),
            (
                ReadinessReason::ShuttingDown,
                false,
                0.0,
                "already draining",
            ),
            (
                ReadinessReason::ShuttingDown,
                true,
                0.0,
                "sampled shutdown never publishes ready",
            ),
        ] {
            let recorder = DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            metrics::with_local_recorder(&recorder, || {
                record_readiness_probe(sampled, || still_ready);
            });

            assert_eq!(
                gauge_value(&snapshotter.snapshot().into_vec(), "overall"),
                expected,
                "{case}"
            );
        }
    }

    /// A shutdown probe records no dependency attempt or latency sample: it did
    /// not evaluate anything, and fabricating a sample would misreport the
    /// dependency's real health during a rollout.
    #[test]
    fn a_readiness_probe_never_records_dependency_attempts() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            record_readiness_probe(ReadinessReason::Ready, || true);
            record_readiness_probe(ReadinessReason::ShuttingDown, || false);
        });
        let snapshot = snapshotter.snapshot().into_vec();

        assert!(snapshot.iter().all(|(key, _, _, _)| {
            key.key().name() != "buzz_readiness_dependency_checks_total"
                && key.key().name() != "buzz_readiness_check_duration_seconds"
        }));
    }

    #[test]
    fn readiness_reason_labels_are_the_closed_lifecycle_set() {
        assert_eq!(
            [ReadinessReason::Ready, ReadinessReason::ShuttingDown].map(ReadinessReason::label),
            READINESS_REASON_LABELS
        );
    }
}
