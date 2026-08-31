//! Per-request readiness evaluation and metrics.
//!
//! Evaluations are temporary: no readiness history is retained in the relay.
//! Counters and histograms accumulate in the metrics recorder, while gauges
//! expose only the latest result observed by the next OpenMetrics scrape.

use std::future::Future;
use std::time::Duration;

use buzz_db::{Db, DbReadinessOutcome};
use tokio::time::Instant;

const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckOutcome {
    Success,
    PoolTimeout,
    PoolError,
    OperationTimeout,
    OperationError,
}

impl CheckOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PoolTimeout => "pool_timeout",
            Self::PoolError => "pool_error",
            Self::OperationTimeout => "operation_timeout",
            Self::OperationError => "operation_error",
        }
    }

    pub(crate) fn is_success(self) -> bool {
        self == Self::Success
    }

    fn is_timeout(self) -> bool {
        matches!(self, Self::PoolTimeout | Self::OperationTimeout)
    }
}

impl From<DbReadinessOutcome> for CheckOutcome {
    fn from(outcome: DbReadinessOutcome) -> Self {
        match outcome {
            DbReadinessOutcome::Success => Self::Success,
            DbReadinessOutcome::PoolTimeout => Self::PoolTimeout,
            DbReadinessOutcome::PoolError => Self::PoolError,
            DbReadinessOutcome::QueryTimeout => Self::OperationTimeout,
            DbReadinessOutcome::QueryError => Self::OperationError,
        }
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
pub(crate) struct CheckResult {
    pub(crate) outcome: CheckOutcome,
    duration: Duration,
}

impl CheckResult {
    #[cfg(test)]
    pub(crate) fn new(outcome: CheckOutcome, duration: Duration) -> Self {
        Self { outcome, duration }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReadinessEvaluation {
    pub(crate) postgres: Option<CheckResult>,
    pub(crate) redis: Option<CheckResult>,
    pub(crate) deletion_catalog: Option<CheckResult>,
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
        postgres: CheckResult,
        redis: CheckResult,
        deletion_catalog: CheckResult,
        total_duration: Duration,
    ) -> Self {
        Self::for_dependencies(postgres, redis, deletion_catalog, total_duration)
    }

    fn for_dependencies(
        postgres: CheckResult,
        redis: CheckResult,
        deletion_catalog: CheckResult,
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
}

fn final_reason(
    postgres: CheckOutcome,
    redis: CheckOutcome,
    deletion_catalog: CheckOutcome,
) -> ReadinessReason {
    let failures = [postgres, redis, deletion_catalog]
        .into_iter()
        .filter(|outcome| !outcome.is_success())
        .collect::<Vec<_>>();

    if failures.is_empty() {
        return ReadinessReason::Ready;
    }
    if failures.len() > 1 {
        return if failures.iter().all(|outcome| outcome.is_timeout()) {
            ReadinessReason::OverallTimeout
        } else {
            ReadinessReason::MultipleDependenciesFailed
        };
    }

    if !postgres.is_success() {
        return match postgres {
            CheckOutcome::PoolTimeout => ReadinessReason::PostgresPoolTimeout,
            CheckOutcome::PoolError => ReadinessReason::PostgresPoolError,
            CheckOutcome::OperationTimeout => ReadinessReason::PostgresQueryTimeout,
            CheckOutcome::OperationError => ReadinessReason::PostgresQueryError,
            CheckOutcome::Success => ReadinessReason::Ready,
        };
    }
    if !redis.is_success() {
        return match redis {
            CheckOutcome::PoolTimeout | CheckOutcome::OperationTimeout => {
                ReadinessReason::RedisPoolTimeout
            }
            CheckOutcome::PoolError | CheckOutcome::OperationError => {
                ReadinessReason::RedisPoolError
            }
            CheckOutcome::Success => ReadinessReason::Ready,
        };
    }

    match deletion_catalog {
        CheckOutcome::PoolTimeout | CheckOutcome::OperationTimeout => {
            ReadinessReason::DeletionCatalogTimeout
        }
        CheckOutcome::PoolError | CheckOutcome::OperationError => {
            ReadinessReason::DeletionCatalogError
        }
        CheckOutcome::Success => ReadinessReason::Ready,
    }
}

async fn timed<F>(future: F) -> CheckResult
where
    F: Future<Output = CheckOutcome>,
{
    let started_at = Instant::now();
    let outcome = future.await;
    CheckResult {
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
    P: Future<Output = CheckOutcome>,
    R: Future<Output = CheckOutcome>,
    D: Future<Output = CheckOutcome>,
{
    let started_at = Instant::now();
    let (postgres, redis, deletion_catalog) =
        tokio::join!(timed(postgres), timed(redis), timed(deletion_catalog),);
    ReadinessEvaluation::for_dependencies(postgres, redis, deletion_catalog, started_at.elapsed())
}

async fn redis_check(pool: &deadpool_redis::Pool, deadline: Instant) -> CheckOutcome {
    match tokio::time::timeout_at(deadline, pool.get()).await {
        Err(_) => CheckOutcome::PoolTimeout,
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "Redis readiness pool acquisition failed");
            CheckOutcome::PoolError
        }
        Ok(Ok(_connection)) => CheckOutcome::Success,
    }
}

async fn deletion_catalog_check(db: &Db, deadline: Instant) -> CheckOutcome {
    match tokio::time::timeout_at(deadline, db.validate_deletion_serving_catalog()).await {
        Err(_) => CheckOutcome::OperationTimeout,
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "Deletion catalog readiness validation failed");
            CheckOutcome::OperationError
        }
        Ok(Ok(())) => CheckOutcome::Success,
    }
}

pub(crate) async fn evaluate(db: &Db, redis_pool: &deadpool_redis::Pool) -> ReadinessEvaluation {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    evaluate_dependencies(
        async { db.readiness_check(deadline).await.into() },
        redis_check(redis_pool, deadline),
        deletion_catalog_check(db, deadline),
    )
    .await
}

pub(crate) fn record_metrics(evaluation: &ReadinessEvaluation) {
    let ready = evaluation.is_ready();
    metrics::counter!(
        "buzz_readiness_checks_total",
        "result" => if ready { "success" } else { "failure" },
        "reason" => evaluation.reason.label(),
    )
    .increment(1);
    metrics::histogram!(
        "buzz_readiness_check_duration_seconds",
        "check" => "overall",
        "outcome" => if ready { "success" } else { "failure" },
    )
    .record(evaluation.total_duration.as_secs_f64());
    metrics::gauge!("buzz_readiness_state", "check" => "overall").set(if ready {
        1.0
    } else {
        0.0
    });

    record_dependency("postgres", evaluation.postgres);
    record_dependency("redis", evaluation.redis);
    record_dependency("deletion_catalog", evaluation.deletion_catalog);
}

fn record_dependency(dependency: &'static str, result: Option<CheckResult>) {
    let Some(result) = result else {
        metrics::gauge!("buzz_readiness_state", "check" => dependency).set(0.0);
        return;
    };

    metrics::counter!(
        "buzz_readiness_dependency_checks_total",
        "dependency" => dependency,
        "outcome" => result.outcome.label(),
    )
    .increment(1);
    metrics::histogram!(
        "buzz_readiness_check_duration_seconds",
        "check" => dependency,
        "outcome" => result.outcome.label(),
    )
    .record(result.duration.as_secs_f64());
    metrics::gauge!("buzz_readiness_state", "check" => dependency).set(
        if result.outcome.is_success() {
            1.0
        } else {
            0.0
        },
    );
}

#[cfg(test)]
mod tests {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn evaluation_preserves_a_completed_check_when_the_other_times_out() {
        let evaluation = evaluate_dependencies(
            async {
                tokio::time::sleep(Duration::from_millis(35)).await;
                CheckOutcome::Success
            },
            async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                CheckOutcome::PoolTimeout
            },
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                CheckOutcome::Success
            },
        )
        .await;

        assert_eq!(
            evaluation.postgres.map(|result| result.outcome),
            Some(CheckOutcome::Success)
        );
        assert_eq!(
            evaluation.redis.map(|result| result.outcome),
            Some(CheckOutcome::PoolTimeout)
        );
        assert_eq!(evaluation.reason, ReadinessReason::RedisPoolTimeout);
        assert_eq!(
            evaluation.deletion_catalog.map(|result| result.outcome),
            Some(CheckOutcome::Success)
        );
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
                CheckOutcome::PoolTimeout,
                CheckOutcome::OperationTimeout,
                CheckOutcome::Success,
            ),
            ReadinessReason::OverallTimeout
        );
    }

    #[test]
    fn deletion_catalog_failure_has_a_specific_reason() {
        assert_eq!(
            final_reason(
                CheckOutcome::Success,
                CheckOutcome::Success,
                CheckOutcome::OperationError,
            ),
            ReadinessReason::DeletionCatalogError
        );
    }

    #[test]
    fn metrics_include_overall_and_dependency_results_and_current_states() {
        let evaluation = ReadinessEvaluation::from_results(
            CheckResult::new(CheckOutcome::Success, Duration::from_millis(35)),
            CheckResult::new(CheckOutcome::PoolTimeout, Duration::from_secs(2)),
            CheckResult::new(CheckOutcome::Success, Duration::from_millis(20)),
            Duration::from_secs(2),
        );
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || record_metrics(&evaluation));
        let snapshot = snapshotter.snapshot().into_vec();

        let find = |name: &str, labels: &[(&str, &str)]| {
            snapshot.iter().find(|(key, ..)| {
                key.key().name() == name
                    && labels.iter().all(|(expected_key, expected_value)| {
                        key.key().labels().any(|label| {
                            label.key() == *expected_key && label.value() == *expected_value
                        })
                    })
            })
        };

        let overall = find(
            "buzz_readiness_checks_total",
            &[("result", "failure"), ("reason", "redis_pool_timeout")],
        )
        .expect("overall readiness failure counter");
        assert!(matches!(
            &overall.3,
            DebugValue::Counter(value) if *value == 1
        ));

        let postgres = find(
            "buzz_readiness_dependency_checks_total",
            &[("dependency", "postgres"), ("outcome", "success")],
        )
        .expect("Postgres success counter");
        assert!(matches!(
            &postgres.3,
            DebugValue::Counter(value) if *value == 1
        ));

        let redis = find(
            "buzz_readiness_dependency_checks_total",
            &[("dependency", "redis"), ("outcome", "pool_timeout")],
        )
        .expect("Redis timeout counter");
        assert!(matches!(
            &redis.3,
            DebugValue::Counter(value) if *value == 1
        ));

        let deletion_catalog = find(
            "buzz_readiness_dependency_checks_total",
            &[("dependency", "deletion_catalog"), ("outcome", "success")],
        )
        .expect("deletion catalog success counter");
        assert!(matches!(
            &deletion_catalog.3,
            DebugValue::Counter(value) if *value == 1
        ));

        let postgres_state =
            find("buzz_readiness_state", &[("check", "postgres")]).expect("Postgres state gauge");
        let DebugValue::Gauge(value) = &postgres_state.3 else {
            panic!("Postgres state must be a gauge");
        };
        assert_eq!(value.into_inner(), 1.0);

        let redis_state =
            find("buzz_readiness_state", &[("check", "redis")]).expect("Redis state gauge");
        let DebugValue::Gauge(value) = &redis_state.3 else {
            panic!("Redis state must be a gauge");
        };
        assert_eq!(value.into_inner(), 0.0);

        let deletion_catalog_state = find("buzz_readiness_state", &[("check", "deletion_catalog")])
            .expect("deletion catalog state gauge");
        let DebugValue::Gauge(value) = &deletion_catalog_state.3 else {
            panic!("deletion catalog state must be a gauge");
        };
        assert_eq!(value.into_inner(), 1.0);

        assert!(find(
            "buzz_readiness_check_duration_seconds",
            &[("check", "postgres"), ("outcome", "success")],
        )
        .is_some());
        assert!(find(
            "buzz_readiness_check_duration_seconds",
            &[("check", "redis"), ("outcome", "pool_timeout")],
        )
        .is_some());
    }

    #[test]
    fn shutdown_records_no_dependency_attempts_and_zeroes_all_states() {
        let evaluation = ReadinessEvaluation::shutting_down();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || record_metrics(&evaluation));
        let snapshot = snapshotter.snapshot().into_vec();

        assert!(snapshot.iter().any(|(key, ..)| {
            key.key().name() == "buzz_readiness_checks_total"
                && key
                    .key()
                    .labels()
                    .any(|label| label.key() == "reason" && label.value() == "shutting_down")
        }));
        assert!(!snapshot
            .iter()
            .any(|(key, ..)| { key.key().name() == "buzz_readiness_dependency_checks_total" }));

        let states: Vec<_> = snapshot
            .iter()
            .filter(|(key, ..)| key.key().name() == "buzz_readiness_state")
            .collect();
        assert_eq!(states.len(), 4);
        assert!(states.iter().all(|entry| {
            matches!(&entry.3, DebugValue::Gauge(value) if value.into_inner() == 0.0)
        }));
    }
}
