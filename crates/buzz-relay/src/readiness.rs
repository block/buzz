//! Readiness dependency evaluation and ordered metrics publication.
//!
//! [`ReadinessCoordinator`] is process-owned. Its mutex is the linearization
//! point shared by health-probe commits and terminal shutdown, so an older
//! evaluation can never overwrite newer gauges or publish ready after shutdown.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use buzz_db::{Db, DbError, DbReadinessOutcome};
use tokio::time::Instant;

const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// Closed label set exported by `buzz_readiness_checks_total{reason}`.
#[cfg(test)]
pub(crate) const READINESS_REASON_LABELS: [&str; 12] = [
    "ready",
    "shutting_down",
    "postgres_pool_timeout",
    "postgres_pool_error",
    "postgres_query_timeout",
    "postgres_query_error",
    "redis_pool_timeout",
    "redis_pool_error",
    "deletion_catalog_timeout",
    "deletion_catalog_error",
    "overall_timeout",
    "multiple_dependencies_failed",
];

/// Maximum raw Prometheus series emitted by readiness for one pod.
///
/// - 12 overall reasons
/// - 11 valid dependency/outcome pairs (Postgres 5, Redis 3, catalog 3)
/// - 4 histograms x (15 configured buckets + `+Inf` + count + sum) = 72
/// - 4 current-state gauges
#[cfg(test)]
pub(crate) const READINESS_RAW_SERIES_PER_POD: usize = 12 + 11 + (4 * 18) + 4;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadinessReason {
    Ready,
    ShuttingDown,
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

impl ReadinessReason {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ShuttingDown => "shutting_down",
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReadinessEvaluation {
    postgres: Option<TimedOutcome<PostgresOutcome>>,
    redis: Option<TimedOutcome<RedisOutcome>>,
    deletion_catalog: Option<TimedOutcome<DeletionCatalogOutcome>>,
    pub(crate) reason: ReadinessReason,
    total_duration: Duration,
}

impl ReadinessEvaluation {
    pub(crate) fn shutting_down() -> Self {
        Self {
            postgres: None,
            redis: None,
            deletion_catalog: None,
            reason: ReadinessReason::ShuttingDown,
            total_duration: Duration::ZERO,
        }
    }

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
            postgres: Some(postgres),
            redis: Some(redis),
            deletion_catalog: Some(deletion_catalog),
            reason,
            total_duration,
        }
    }

    pub(crate) fn is_ready(self) -> bool {
        self.reason == ReadinessReason::Ready
    }

    pub(crate) fn postgres_ready(self) -> bool {
        self.postgres
            .is_some_and(|result| result.outcome.is_success())
    }

    pub(crate) fn redis_ready(self) -> bool {
        self.redis.is_some_and(|result| result.outcome.is_success())
    }

    pub(crate) fn deletion_catalog_ready(self) -> bool {
        self.deletion_catalog
            .is_some_and(|result| result.outcome.is_success())
    }

    fn dependencies_ran(self) -> bool {
        self.postgres.is_some() || self.redis.is_some() || self.deletion_catalog.is_some()
    }
}

fn final_reason(
    postgres: PostgresOutcome,
    redis: RedisOutcome,
    deletion_catalog: DeletionCatalogOutcome,
) -> ReadinessReason {
    let failure_count = usize::from(!postgres.is_success())
        + usize::from(!redis.is_success())
        + usize::from(!deletion_catalog.is_success());

    if failure_count == 0 {
        return ReadinessReason::Ready;
    }
    if failure_count > 1 {
        let all_failures_are_timeouts = (postgres.is_success() || postgres.is_timeout())
            && (redis.is_success() || redis.is_timeout())
            && (deletion_catalog.is_success() || deletion_catalog.is_timeout());
        return if all_failures_are_timeouts {
            ReadinessReason::OverallTimeout
        } else {
            ReadinessReason::MultipleDependenciesFailed
        };
    }

    match postgres {
        PostgresOutcome::PoolTimeout => ReadinessReason::PostgresPoolTimeout,
        PostgresOutcome::PoolError => ReadinessReason::PostgresPoolError,
        PostgresOutcome::QueryTimeout => ReadinessReason::PostgresQueryTimeout,
        PostgresOutcome::QueryError => ReadinessReason::PostgresQueryError,
        PostgresOutcome::Success => match redis {
            RedisOutcome::PoolTimeout => ReadinessReason::RedisPoolTimeout,
            RedisOutcome::PoolError => ReadinessReason::RedisPoolError,
            RedisOutcome::Success => match deletion_catalog {
                DeletionCatalogOutcome::OperationTimeout => ReadinessReason::DeletionCatalogTimeout,
                DeletionCatalogOutcome::OperationError => ReadinessReason::DeletionCatalogError,
                DeletionCatalogOutcome::Success => ReadinessReason::Ready,
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
) -> ReadinessEvaluation
where
    P: Future<Output = PostgresOutcome>,
    R: Future<Output = RedisOutcome>,
    D: Future<Output = DeletionCatalogOutcome>,
{
    let started_at = Instant::now();
    let (postgres, redis, deletion_catalog) =
        tokio::join!(timed(postgres), timed(redis), timed(deletion_catalog),);
    ReadinessEvaluation::for_dependencies(postgres, redis, deletion_catalog, started_at.elapsed())
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
pub(crate) trait ReadinessEvaluator: Send + Sync {
    async fn evaluate(&self, db: &Db, redis_pool: &deadpool_redis::Pool) -> ReadinessEvaluation;
}

struct ProductionReadinessEvaluator;

#[async_trait::async_trait]
impl ReadinessEvaluator for ProductionReadinessEvaluator {
    async fn evaluate(&self, db: &Db, redis_pool: &deadpool_redis::Pool) -> ReadinessEvaluation {
        let deadline = Instant::now() + READINESS_TIMEOUT;
        evaluate_dependencies(
            async { db.readiness_check(deadline).await.into() },
            redis_check(redis_pool, deadline),
            deletion_catalog_check(db, deadline),
        )
        .await
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProbeTicket {
    generation: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProbeStart {
    Evaluate(ProbeTicket),
    ShuttingDown,
}

#[derive(Debug, Default)]
struct PublicationState {
    next_generation: u64,
    latest_published_generation: u64,
    shutdown_generation: Option<u64>,
}

/// Serializes readiness result publication with terminal process shutdown.
pub(crate) struct ReadinessCoordinator {
    state: Mutex<PublicationState>,
    evaluator: Arc<dyn ReadinessEvaluator>,
}

impl Default for ReadinessCoordinator {
    fn default() -> Self {
        Self {
            state: Mutex::new(PublicationState::default()),
            evaluator: Arc::new(ProductionReadinessEvaluator),
        }
    }
}

impl ReadinessCoordinator {
    #[cfg(test)]
    pub(crate) fn with_evaluator(evaluator: Arc<dyn ReadinessEvaluator>) -> Self {
        Self {
            state: Mutex::new(PublicationState::default()),
            evaluator,
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, PublicationState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) async fn evaluate(
        &self,
        db: &Db,
        redis_pool: &deadpool_redis::Pool,
    ) -> ReadinessEvaluation {
        self.evaluator.evaluate(db, redis_pool).await
    }

    /// Allocates a health-probe generation or records a truthful shutdown fast path.
    pub(crate) fn begin_probe(&self) -> ProbeStart {
        let mut state = self.lock_state();
        if state.shutdown_generation.is_some() {
            let evaluation = ReadinessEvaluation::shutting_down();
            record_attempt_metrics(&evaluation, ReadinessReason::ShuttingDown);
            record_overall_state(false);
            return ProbeStart::ShuttingDown;
        }

        state.next_generation = state.next_generation.saturating_add(1);
        ProbeStart::Evaluate(ProbeTicket {
            generation: state.next_generation,
        })
    }

    /// Commits one completed health probe through the shared publication fence.
    pub(crate) fn finish_probe(
        &self,
        ticket: ProbeTicket,
        evaluation: ReadinessEvaluation,
    ) -> ReadinessEvaluation {
        let mut state = self.lock_state();
        if state.shutdown_generation.is_some() {
            record_attempt_metrics(&evaluation, ReadinessReason::ShuttingDown);
            return ReadinessEvaluation::shutting_down();
        }

        record_attempt_metrics(&evaluation, evaluation.reason);
        if ticket.generation > state.latest_published_generation {
            record_current_state(&evaluation);
            state.latest_published_generation = ticket.generation;
        }
        evaluation
    }

    /// Returns whether a compatibility/public readiness evaluation may start.
    pub(crate) fn public_evaluation_allowed(&self) -> bool {
        self.lock_state().shutdown_generation.is_none()
    }

    /// Makes shutdown dominate a public request that was already in flight.
    pub(crate) fn finish_public_evaluation(
        &self,
        evaluation: ReadinessEvaluation,
    ) -> ReadinessEvaluation {
        if self.lock_state().shutdown_generation.is_some() {
            ReadinessEvaluation::shutting_down()
        } else {
            evaluation
        }
    }

    /// Commits terminal shutdown and immediately publishes overall not-ready.
    pub(crate) fn begin_shutdown(&self) {
        let mut state = self.lock_state();
        if state.shutdown_generation.is_none() {
            let generation = state.next_generation.saturating_add(1);
            state.shutdown_generation = Some(generation);
            record_overall_state(false);
        }
    }
}

fn record_attempt_metrics(evaluation: &ReadinessEvaluation, reason: ReadinessReason) {
    metrics::counter!(
        "buzz_readiness_checks_total",
        "reason" => reason.label(),
    )
    .increment(1);

    if !evaluation.dependencies_ran() {
        return;
    }

    metrics::histogram!(
        "buzz_readiness_check_duration_seconds",
        "check" => "overall",
    )
    .record(evaluation.total_duration.as_secs_f64());

    if let Some(result) = evaluation.postgres {
        record_dependency_attempt("postgres", result.outcome.label(), result.duration);
    }
    if let Some(result) = evaluation.redis {
        record_dependency_attempt("redis", result.outcome.label(), result.duration);
    }
    if let Some(result) = evaluation.deletion_catalog {
        record_dependency_attempt("deletion_catalog", result.outcome.label(), result.duration);
    }
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

fn record_current_state(evaluation: &ReadinessEvaluation) {
    record_overall_state(evaluation.is_ready());
    if let Some(result) = evaluation.postgres {
        record_dependency_state("postgres", result.outcome.is_success());
    }
    if let Some(result) = evaluation.redis {
        record_dependency_state("redis", result.outcome.is_success());
    }
    if let Some(result) = evaluation.deletion_catalog {
        record_dependency_state("deletion_catalog", result.outcome.is_success());
    }
}

fn record_overall_state(ready: bool) {
    metrics::gauge!("buzz_readiness_state", "check" => "overall").set(if ready {
        1.0
    } else {
        0.0
    });
}

fn record_dependency_state(dependency: &'static str, ready: bool) {
    metrics::gauge!("buzz_readiness_state", "check" => dependency).set(if ready {
        1.0
    } else {
        0.0
    });
}

#[cfg(test)]
mod tests {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use metrics_util::CompositeKey;

    use super::*;

    fn ready_evaluation() -> ReadinessEvaluation {
        ReadinessEvaluation::from_results(
            TimedOutcome::new(PostgresOutcome::Success, Duration::from_millis(35)),
            TimedOutcome::new(RedisOutcome::Success, Duration::from_millis(10)),
            TimedOutcome::new(DeletionCatalogOutcome::Success, Duration::from_millis(20)),
            Duration::from_millis(35),
        )
    }

    fn redis_failure_evaluation() -> ReadinessEvaluation {
        ReadinessEvaluation::from_results(
            TimedOutcome::new(PostgresOutcome::Success, Duration::from_millis(35)),
            TimedOutcome::new(RedisOutcome::PoolTimeout, Duration::from_secs(2)),
            TimedOutcome::new(DeletionCatalogOutcome::Success, Duration::from_millis(20)),
            Duration::from_secs(2),
        )
    }

    fn exact_metric<'a>(
        snapshot: &'a [(
            CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
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

    fn gauge_value(
        snapshot: &[(
            CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
        check: &str,
    ) -> f64 {
        let value = exact_metric(snapshot, "buzz_readiness_state", &[("check", check)])
            .expect("readiness gauge");
        let DebugValue::Gauge(value) = value else {
            panic!("readiness state must be a gauge");
        };
        value.into_inner()
    }

    #[tokio::test(start_paused = true)]
    async fn evaluation_preserves_a_completed_check_when_another_times_out() {
        let evaluation = evaluate_dependencies(
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

        assert_eq!(evaluation.reason, ReadinessReason::RedisPoolTimeout);
        assert_eq!(
            evaluation.postgres.map(|result| result.duration),
            Some(Duration::from_millis(35))
        );
        assert_eq!(
            evaluation.redis.map(|result| result.duration),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn simultaneous_dependency_timeouts_are_an_overall_timeout() {
        assert_eq!(
            final_reason(
                PostgresOutcome::PoolTimeout,
                RedisOutcome::PoolTimeout,
                DeletionCatalogOutcome::Success,
            ),
            ReadinessReason::OverallTimeout
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
        assert_eq!(READINESS_RAW_SERIES_PER_POD, 99);
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

    #[test]
    fn slow_older_failure_cannot_overwrite_newer_success_gauges() {
        let coordinator = ReadinessCoordinator::default();
        let ProbeStart::Evaluate(slow_a) = coordinator.begin_probe() else {
            panic!("serving probe A");
        };
        let ProbeStart::Evaluate(fast_b) = coordinator.begin_probe() else {
            panic!("serving probe B");
        };
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            coordinator.finish_probe(fast_b, ready_evaluation());
            coordinator.finish_probe(slow_a, redis_failure_evaluation());
        });
        let snapshot = snapshotter.snapshot().into_vec();

        assert_eq!(gauge_value(&snapshot, "overall"), 1.0);
        assert_eq!(gauge_value(&snapshot, "redis"), 1.0);
        assert!(matches!(
            exact_metric(
                &snapshot,
                "buzz_readiness_checks_total",
                &[("reason", "ready")]
            ),
            Some(DebugValue::Counter(1))
        ));
        assert!(matches!(
            exact_metric(
                &snapshot,
                "buzz_readiness_checks_total",
                &[("reason", "redis_pool_timeout")]
            ),
            Some(DebugValue::Counter(1))
        ));
    }

    #[test]
    fn slow_older_success_cannot_overwrite_newer_failure_gauges() {
        let coordinator = ReadinessCoordinator::default();
        let ProbeStart::Evaluate(slow_a) = coordinator.begin_probe() else {
            panic!("serving probe A");
        };
        let ProbeStart::Evaluate(fast_b) = coordinator.begin_probe() else {
            panic!("serving probe B");
        };
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            coordinator.finish_probe(fast_b, redis_failure_evaluation());
            coordinator.finish_probe(slow_a, ready_evaluation());
        });
        let snapshot = snapshotter.snapshot().into_vec();

        assert_eq!(gauge_value(&snapshot, "overall"), 0.0);
        assert_eq!(gauge_value(&snapshot, "postgres"), 1.0);
        assert_eq!(gauge_value(&snapshot, "redis"), 0.0);
        assert_eq!(gauge_value(&snapshot, "deletion_catalog"), 1.0);
    }

    #[test]
    fn shutdown_fast_path_preserves_dependency_state_and_histograms() {
        let coordinator = ReadinessCoordinator::default();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            let ProbeStart::Evaluate(ticket) = coordinator.begin_probe() else {
                panic!("initial serving probe");
            };
            coordinator.finish_probe(ticket, ready_evaluation());
            coordinator.begin_shutdown();
            assert!(matches!(
                coordinator.begin_probe(),
                ProbeStart::ShuttingDown
            ));
        });
        let after = snapshotter.snapshot().into_vec();

        for dependency in ["postgres", "redis", "deletion_catalog"] {
            assert_eq!(
                gauge_value(&after, dependency),
                1.0,
                "shutdown must not fabricate {dependency} state"
            );
        }
        for check in ["overall", "postgres", "redis", "deletion_catalog"] {
            assert!(
                matches!(
                    exact_metric(
                        &after,
                        "buzz_readiness_check_duration_seconds",
                        &[("check", check)]
                    ),
                    Some(DebugValue::Histogram(values)) if values.len() == 1
                ),
                "shutdown fast path must not add a {check} duration"
            );
        }
        assert_eq!(gauge_value(&after, "overall"), 0.0);
        assert!(matches!(
            exact_metric(
                &after,
                "buzz_readiness_checks_total",
                &[("reason", "shutting_down")]
            ),
            Some(DebugValue::Counter(1))
        ));
    }

    #[test]
    fn shutdown_dominates_an_in_flight_success_without_resurrecting_gauges() {
        let coordinator = ReadinessCoordinator::default();
        let ProbeStart::Evaluate(ticket) = coordinator.begin_probe() else {
            panic!("serving probe");
        };
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let response = metrics::with_local_recorder(&recorder, || {
            coordinator.begin_shutdown();
            coordinator.finish_probe(ticket, ready_evaluation())
        });
        let snapshot = snapshotter.snapshot().into_vec();

        assert_eq!(response.reason, ReadinessReason::ShuttingDown);
        assert_eq!(gauge_value(&snapshot, "overall"), 0.0);
        assert!(
            exact_metric(&snapshot, "buzz_readiness_state", &[("check", "postgres")]).is_none()
        );
        assert!(matches!(
            exact_metric(
                &snapshot,
                "buzz_readiness_dependency_checks_total",
                &[("dependency", "postgres"), ("outcome", "success")]
            ),
            Some(DebugValue::Counter(1))
        ));
    }
}
