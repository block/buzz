//! Readiness-probe telemetry and the dependency diagnostics behind `/_status`.
//!
//! Readiness is deliberately *not* a dependency question. A shared Postgres or
//! Redis failure is shared by every replica, so evaluating it in the probe took
//! the whole deployment out of the load balancer at once and left a reconnect
//! burst with nowhere to land. The Kubernetes probe therefore answers from this
//! process's own lifecycle (see [`crate::router`]), and the same dependency
//! evaluation is reported on the diagnostic `/_status` endpoint, which is never
//! wired to a probe.
//!
//! Dependency evaluation is also decoupled from requests. One per-pod loop
//! ([`run_dependency_sampler`]) evaluates on a fixed cadence, publishes the
//! dependency metrics, and caches the report; `/_status` only reads that cache.
//! Evaluating per request made the load a pressured dependency sees depend on
//! how often someone looked at the endpoint, with nothing bounding how many
//! evaluations could be in flight at once.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use buzz_db::{Db, DbError, DbReadinessOutcome};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(2);

/// Fixed cadence of the per-pod dependency sampling loop.
///
/// Slow enough that a pod adds negligible load to a shared dependency, fast
/// enough that an operator opening `/_status` during an incident reads
/// something current. Documented in `deploy/charts/buzz/README.md`.
pub const DEPENDENCY_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// The completion-epoch publisher always refreshes at least this frequently.
///
/// Main computes a cadence from the configured gauge idle timeout and caps it
/// at this value so the completion timestamp series survives idle eviction
/// without adding high-frequency noise.
const DEPENDENCY_SAMPLE_COMPLETION_REPUBLISH_MAX_INTERVAL: Duration = DEPENDENCY_SAMPLE_INTERVAL;

/// Age past which a cached report is reported stale rather than current.
///
/// Two cadences: one full cycle can be missed by an evaluation that consumed
/// its whole [`DEPENDENCY_TIMEOUT`] budget, so anything older than that means
/// the sampler itself is not keeping up.
const DEPENDENCY_SAMPLE_STALE_AFTER: Duration = DEPENDENCY_SAMPLE_INTERVAL.saturating_mul(2);

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
/// - 2 readiness gauges (overall lifecycle + completion epoch)
#[cfg(test)]
pub(crate) const READINESS_RAW_SERIES_PER_POD: usize = 2 + 11 + (4 * 18) + 1 + 1;

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
/// The counter and gauge describe the same immutable lifecycle observation.
/// The gauge is therefore the latest private readiness-probe observation, not
/// a transition-owned lifecycle mirror.
pub(crate) fn record_readiness_probe(reason: ReadinessReason) {
    metrics::counter!(
        "buzz_readiness_checks_total",
        "reason" => reason.label(),
    )
    .increment(1);
    metrics::gauge!("buzz_readiness_state", "check" => "overall").set(if reason.is_ready() {
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

/// One completed evaluation and when it was observed.
#[derive(Debug, Clone, Copy)]
struct DependencySample {
    report: DependencyReport,
    observed_at: Instant,
}

/// What the cache can tell `/_status`.
///
/// "No report yet" is a distinct state, not a fabricated healthy one, and a
/// report is always accompanied by its age: a cached verdict presented without
/// one would read as authoritative however long ago it was taken.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DependencySnapshot {
    /// The sampler has not completed its first evaluation yet.
    NotYetSampled,
    Sampled {
        report: DependencyReport,
        age: Duration,
        stale: bool,
    },
}

/// Republish cadence for the completion-epoch gauge.
///
/// The configured gauge idle timeout comes from `main.rs`; this returns a
/// bounded cadence that is strictly below that timeout and never slower than
/// the dependency sampler itself.
pub fn dependency_sample_completion_republish_interval(gauge_idle_timeout_secs: u64) -> Duration {
    let idle_timeout_secs = gauge_idle_timeout_secs.max(1);
    let refresh_secs = idle_timeout_secs.saturating_div(3).max(1);
    let strict_upper_bound_secs = idle_timeout_secs.saturating_sub(1).max(1);
    Duration::from_secs(
        refresh_secs
            .min(strict_upper_bound_secs)
            .min(DEPENDENCY_SAMPLE_COMPLETION_REPUBLISH_MAX_INTERVAL.as_secs()),
    )
}

/// The per-pod owner of shared-dependency evaluation.
///
/// [`run_dependency_sampler`] is the only caller of [`Self::sample`], so at
/// most one evaluation exists at a time and no request path can start another.
/// `/_status` reads [`Self::snapshot`], which touches no dependency.
pub(crate) struct DependencyDiagnostics {
    evaluator: Arc<dyn DependencyEvaluator>,
    latest: Mutex<Option<DependencySample>>,
    sample_completion_epoch_seconds: AtomicU64,
    sample_completion_written: AtomicBool,
}

impl Default for DependencyDiagnostics {
    fn default() -> Self {
        Self::with_evaluator(Arc::new(ProductionDependencyEvaluator))
    }
}

impl DependencyDiagnostics {
    pub(crate) fn with_evaluator(evaluator: Arc<dyn DependencyEvaluator>) -> Self {
        Self {
            evaluator,
            latest: Mutex::new(None),
            sample_completion_epoch_seconds: AtomicU64::new(0),
            sample_completion_written: AtomicBool::new(false),
        }
    }

    /// Runs one bounded evaluation, publishes its telemetry, and replaces the
    /// cached report.
    ///
    /// The completion timestamp is written last, once the cache already serves
    /// this report, so the gauge can never describe a sample `/_status` is not
    /// yet answering with.
    pub(crate) async fn sample(&self, db: &Db, redis_pool: &deadpool_redis::Pool) {
        let report = self.evaluator.evaluate(db, redis_pool).await;
        record_dependency_report(&report);
        let sample = DependencySample {
            report,
            observed_at: Instant::now(),
        };
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(sample);
        self.record_dependency_sample_completion(SystemTime::now());
    }

    /// The latest completed evaluation with its age. Starts no dependency work.
    pub(crate) fn snapshot(&self) -> DependencySnapshot {
        let latest = *self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        match latest {
            None => DependencySnapshot::NotYetSampled,
            Some(sample) => {
                let age = sample.observed_at.elapsed();
                DependencySnapshot::Sampled {
                    report: sample.report,
                    age,
                    stale: age > DEPENDENCY_SAMPLE_STALE_AFTER,
                }
            }
        }
    }

    fn record_dependency_sample_completion(&self, completed_at: SystemTime) {
        let epoch_seconds = dependency_sample_completion_epoch_seconds(completed_at);
        self.sample_completion_epoch_seconds
            .store(epoch_seconds, Ordering::Release);
        self.sample_completion_written
            .store(true, Ordering::Release);
        publish_dependency_sample_completion_metric(epoch_seconds);
    }

    fn latest_sample_completion_epoch_seconds(&self) -> Option<u64> {
        self.sample_completion_written
            .load(Ordering::Acquire)
            .then(|| self.sample_completion_epoch_seconds.load(Ordering::Acquire))
    }

    fn republish_dependency_sample_completion(&self) {
        if let Some(epoch_seconds) = self.latest_sample_completion_epoch_seconds() {
            publish_dependency_sample_completion_metric(epoch_seconds);
        }
    }
}

/// Runs the per-pod dependency sampling loop until `cancel` fires.
///
/// One loop, one fixed cadence, each evaluation awaited before the next tick is
/// taken, so this pod never has two evaluations in flight. `Skip` matches the
/// community revalidator: an evaluation that overruns its slot delays the next
/// cycle instead of queueing a catch-up burst into the dependency that was
/// already slow. The first tick fires immediately, so the not-yet-sampled
/// window is one evaluation long.
pub async fn run_dependency_sampler(state: Arc<AppState>, cancel: CancellationToken) {
    run_dependency_sampler_with_interval(state, cancel, DEPENDENCY_SAMPLE_INTERVAL).await;
}

async fn run_dependency_sampler_with_interval(
    state: Arc<AppState>,
    cancel: CancellationToken,
    sample_interval: Duration,
) {
    let mut interval = tokio::time::interval(sample_interval.max(Duration::from_millis(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {
                state
                    .dependency_diagnostics
                    .sample(&state.db, &state.redis_pool)
                    .await;
            }
        }
    }
}

/// Re-emits the latest completion epoch so the gauge survives recorder idle
/// eviction even when no newer dependency sample completes.
pub async fn run_dependency_sample_completion_publisher(
    state: Arc<AppState>,
    republish_interval: Duration,
    cancel: CancellationToken,
) {
    run_dependency_sample_completion_publisher_for_diagnostics(
        Arc::clone(&state.dependency_diagnostics),
        republish_interval,
        cancel,
    )
    .await;
}

async fn run_dependency_sample_completion_publisher_for_diagnostics(
    diagnostics: Arc<DependencyDiagnostics>,
    republish_interval: Duration,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(republish_interval.max(Duration::from_millis(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = interval.tick() => diagnostics.republish_dependency_sample_completion(),
        }
    }
}

/// Records one dependency evaluation. Counters and durations only — dependency
/// health has no publishable "current state" now that no probe consumes it; a
/// per-dependency gauge would read as an authoritative verdict on infrastructure
/// this pod only samples every [`DEPENDENCY_SAMPLE_INTERVAL`].
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

/// Publishes the Unix time the cached report completed.
///
/// A completion timestamp rather than an age, because age then belongs to the
/// query — `time() - buzz_readiness_dependency_sample_completed_timestamp_seconds`
/// — and grows on its own while this pod is wedged. A gauge carrying the age
/// needs a writer to advance it, so the one failure it most needs to expose, a
/// sampler that stopped running, is the one that would freeze it at its last
/// value and read as permanently fresh. A completed sample is the only thing
/// that may advance this epoch; the independent publisher only re-emits the
/// stored value so the series survives gauge idle-eviction. This keeps the
/// `buzz_storage_sweep_age_seconds` convention that absence means
/// "not yet sampled" rather than "fresh".
///
fn dependency_sample_completion_epoch_seconds(completed_at: SystemTime) -> u64 {
    completed_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn publish_dependency_sample_completion_metric(epoch_seconds: u64) {
    metrics::gauge!("buzz_readiness_dependency_sample_completed_timestamp_seconds")
        .set(epoch_seconds as f64);
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
        assert_eq!(READINESS_RAW_SERIES_PER_POD, 87);
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
    fn completion_timestamp_republish_interval_stays_below_idle_timeout() {
        assert_eq!(
            dependency_sample_completion_republish_interval(900),
            DEPENDENCY_SAMPLE_INTERVAL,
            "default idle timeout should republish on the sampler cadence"
        );
        assert_eq!(
            dependency_sample_completion_republish_interval(15),
            Duration::from_secs(5),
            "the minimum configured idle timeout must still get multiple republishes"
        );
        assert!(dependency_sample_completion_republish_interval(15) < Duration::from_secs(15));
    }

    /// The readiness gauge and counter use the same immutable reason sampled by
    /// the private probe. A dependency evaluation — however bad — must never
    /// move them, which is what let a shared outage deroute every replica at
    /// once.
    #[test]
    fn readiness_telemetry_tracks_lifecycle_and_dependency_failure_never_moves_it() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            record_readiness_probe(ReadinessReason::Ready);
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
            record_readiness_probe(ReadinessReason::ShuttingDown);
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

    /// A shutdown probe records no dependency attempt or latency sample: it did
    /// not evaluate anything, and fabricating a sample would misreport the
    /// dependency's real health during a rollout.
    #[test]
    fn a_readiness_probe_never_records_dependency_attempts() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            record_readiness_probe(ReadinessReason::Ready);
            record_readiness_probe(ReadinessReason::ShuttingDown);
        });
        let snapshot = snapshotter.snapshot().into_vec();

        assert!(snapshot.iter().all(|(key, _, _, _)| {
            key.key().name() != "buzz_readiness_dependency_checks_total"
                && key.key().name() != "buzz_readiness_check_duration_seconds"
        }));
    }

    /// A `Db` and a Redis pool on a closed port. The scripted evaluators below
    /// never touch either, so no connection is ever attempted; they exist only
    /// to satisfy the production `sample` signature.
    fn unreachable_dependencies() -> (Db, deadpool_redis::Pool) {
        let pool = sqlx::PgPool::connect_lazy("postgres://127.0.0.1:1/buzz").expect("lazy pg pool");
        let redis_pool = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        (Db::from_pool(pool), redis_pool)
    }

    struct FixedEvaluator(DependencyReport);

    #[async_trait::async_trait]
    impl DependencyEvaluator for FixedEvaluator {
        async fn evaluate(&self, _db: &Db, _redis_pool: &deadpool_redis::Pool) -> DependencyReport {
            self.0
        }
    }

    fn ready_diagnostics() -> DependencyDiagnostics {
        DependencyDiagnostics::with_evaluator(Arc::new(FixedEvaluator(
            DependencyReport::from_results(
                TimedOutcome::new(PostgresOutcome::Success, Duration::from_millis(3)),
                TimedOutcome::new(RedisOutcome::Success, Duration::from_millis(2)),
                TimedOutcome::new(DeletionCatalogOutcome::Success, Duration::from_millis(1)),
                Duration::from_millis(3),
            ),
        )))
    }

    fn sampled(snapshot: DependencySnapshot) -> (Duration, bool) {
        let DependencySnapshot::Sampled { age, stale, .. } = snapshot else {
            panic!("a completed sample must be reported as sampled");
        };
        (age, stale)
    }

    /// An operator reading `/_status` must be able to tell "nothing has been
    /// sampled yet" from "this is current" from "this outlived the sampler".
    /// A cached report presented without its age would read as authoritative
    /// however old it is.
    #[tokio::test(start_paused = true)]
    async fn freshness_separates_not_yet_sampled_from_a_fresh_and_a_stale_report() {
        let (db, redis_pool) = unreachable_dependencies();
        let diagnostics = ready_diagnostics();

        assert!(
            matches!(diagnostics.snapshot(), DependencySnapshot::NotYetSampled),
            "no evaluation has completed, so there is nothing to report"
        );

        diagnostics.sample(&db, &redis_pool).await;
        assert_eq!(sampled(diagnostics.snapshot()), (Duration::ZERO, false));

        tokio::time::advance(DEPENDENCY_SAMPLE_INTERVAL).await;
        assert_eq!(
            sampled(diagnostics.snapshot()),
            (DEPENDENCY_SAMPLE_INTERVAL, false),
            "one cadence of age is the steady state, not staleness"
        );

        tokio::time::advance(DEPENDENCY_SAMPLE_INTERVAL + Duration::from_secs(1)).await;
        let (age, stale) = sampled(diagnostics.snapshot());
        assert_eq!(age, DEPENDENCY_SAMPLE_INTERVAL * 2 + Duration::from_secs(1));
        assert!(stale, "a report that outlived two cadences missed a cycle");
    }

    /// The completion timestamp written since the previous snapshot, if any.
    ///
    /// `Snapshotter::snapshot` drains, so a window in which nothing wrote this
    /// metric either omits the key entirely or, once registered, replays as
    /// `0`. Zero is not a time any sample could have completed at, so folding
    /// it into `None` keeps "nobody wrote this in that window" expressible —
    /// the property a completion timestamp must have and an age cannot.
    fn sample_completion_metric_write(snapshot: &Snapshot) -> Option<u64> {
        exact_metric(
            snapshot,
            "buzz_readiness_dependency_sample_completed_timestamp_seconds",
            &[],
        )
        .map(|value| {
            let DebugValue::Gauge(value) = value else {
                panic!("the sample completion timestamp must be a gauge");
            };
            value.into_inner() as u64
        })
        .filter(|written| *written != 0)
    }

    fn epoch_seconds_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_secs()
    }

    fn sample_completion_metric_value_from_scrape(scrape: &str) -> Option<u64> {
        scrape.lines().find_map(|line| {
            line.strip_prefix("buzz_readiness_dependency_sample_completed_timestamp_seconds ")
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| value as u64)
        })
    }

    fn sample_completion_metric_type_from_scrape(scrape: &str) -> Option<&str> {
        scrape.lines().find_map(|line| {
            line.strip_prefix(
                "# TYPE buzz_readiness_dependency_sample_completed_timestamp_seconds ",
            )
        })
    }

    /// Minimal model of Datadog OpenMetrics v2 with `send_monotonic_counter:true`.
    ///
    /// Gauge values are forwarded as-is. Counter values become per-scrape deltas
    /// (`monotonic_count` / `.count`), so the exported sample itself is no
    /// longer available to monitor queries.
    fn datadog_openmetrics_v2_completion_epoch(
        scrape: &str,
        previous_counter_raw: &mut Option<u64>,
    ) -> Option<u64> {
        let raw = sample_completion_metric_value_from_scrape(scrape)?;
        match sample_completion_metric_type_from_scrape(scrape) {
            Some("gauge") => Some(raw),
            Some("counter") => {
                let transformed = previous_counter_raw
                    .map(|previous| raw.saturating_sub(previous))
                    .unwrap_or(raw);
                *previous_counter_raw = Some(raw);
                Some(transformed)
            }
            Some(other) => panic!("unexpected metric type: {other}"),
            None => panic!("missing sample completion metric type"),
        }
    }

    /// The scrape-side half of the same question. Freshness is published as the
    /// Unix time the cached report completed, written only by a completed
    /// sample, so the age belongs to the query
    /// (`time() - buzz_readiness_dependency_sample_completed_timestamp_seconds`)
    /// and grows on its own while this pod is wedged. A gauge carrying the age
    /// instead would need a writer to advance it, so the one failure it most
    /// needs to expose — a sampler that stopped — is the one that would freeze
    /// it at its last value and read as permanently fresh.
    #[test]
    fn the_completion_timestamp_gauge_tracks_completed_samples() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("paused current-thread runtime");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let (db, redis_pool) = unreachable_dependencies();
                let diagnostics = ready_diagnostics();

                assert_eq!(
                    sample_completion_metric_write(&snapshotter.snapshot().into_vec()),
                    None,
                    "absence is the not-yet-sampled signal, not a fresh zero"
                );

                let before_first = epoch_seconds_now();
                diagnostics.sample(&db, &redis_pool).await;
                let after_first = epoch_seconds_now();
                let first = sample_completion_metric_write(&snapshotter.snapshot().into_vec())
                    .expect("a completed sample must publish when it completed");
                assert!(
                    (before_first..=after_first).contains(&first),
                    "{first} must be the wall time the sample completed, \
                     not an age, and not a stale reading"
                );

                // Paused time: no task, timer, or aging loop can run here.
                tokio::time::advance(DEPENDENCY_SAMPLE_STALE_AFTER + Duration::from_secs(1)).await;
                assert_eq!(
                    sample_completion_metric_write(&snapshotter.snapshot().into_vec()),
                    None,
                    "only a completed sample may write the gauge, so the age a \
                     query derives from it grows with no server-side writer"
                );
                assert!(
                    sampled(diagnostics.snapshot()).1,
                    "the cached `/_status` age reports the same report as stale"
                );

                let before_second = epoch_seconds_now();
                diagnostics.sample(&db, &redis_pool).await;
                let after_second = epoch_seconds_now();
                let second = sample_completion_metric_write(&snapshotter.snapshot().into_vec())
                    .expect("the next completed sample republishes the timestamp");
                assert!(
                    (before_second..=after_second).contains(&second) && second >= first,
                    "{second} must re-anchor to the second completion, after {first}"
                );
                assert!(
                    !sampled(diagnostics.snapshot()).1,
                    "a fresh completion clears staleness"
                );
            });
        });
    }

    #[test]
    fn openmetrics_v2_preserves_epoch_only_when_raw_prometheus_type_is_gauge() {
        let (recorder, handle) = crate::metrics::readiness_test_recorder();
        let mut previous_counter_raw = None;

        assert_eq!(
            datadog_openmetrics_v2_completion_epoch(&handle.render(), &mut previous_counter_raw),
            None,
            "absence before the first completion remains absence after transform"
        );

        let first = 4_750_000_001_u64;
        metrics::with_local_recorder(&recorder, || {
            publish_dependency_sample_completion_metric(first);
        });
        let after_first_completion = handle.render();
        assert_eq!(
            datadog_openmetrics_v2_completion_epoch(
                &after_first_completion,
                &mut previous_counter_raw,
            ),
            Some(first),
            "after first completion the transformed value must be the completion epoch"
        );

        // Same raw scrape value after idle timeout: sampler produced no new completion.
        assert_eq!(
            datadog_openmetrics_v2_completion_epoch(
                &after_first_completion,
                &mut previous_counter_raw,
            ),
            Some(first),
            "idle-timeout survival still must preserve the full epoch"
        );

        let second = 4_750_000_005_u64;
        metrics::with_local_recorder(&recorder, || {
            publish_dependency_sample_completion_metric(second);
        });
        let after_later_completion = handle.render();
        assert_eq!(
            datadog_openmetrics_v2_completion_epoch(
                &after_later_completion,
                &mut previous_counter_raw,
            ),
            Some(second),
            "a later completion must transform to the new full epoch"
        );

        // Same raw value again while only the sampler is stopped.
        assert_eq!(
            datadog_openmetrics_v2_completion_epoch(
                &after_later_completion,
                &mut previous_counter_raw,
            ),
            Some(second),
            "sampler stoppage must not collapse the epoch to a delta"
        );
    }

    async fn wait_for_sample_completion_scrape_value(
        handle: &metrics_exporter_prometheus::PrometheusHandle,
        predicate: impl Fn(u64) -> bool,
    ) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = sample_completion_metric_value_from_scrape(&handle.render()) {
                if predicate(value) {
                    return value;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for completion timestamp scrape value"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[test]
    fn scrape_retains_the_completion_timestamp_across_gauge_idle_timeout() {
        let timeout = Duration::from_secs(1);
        let republish_interval = Duration::from_millis(100);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let (recorder, handle) =
            crate::metrics::readiness_test_recorder_with_idle_timeout(timeout.as_secs());

        metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let (db, redis_pool) = unreachable_dependencies();
                let diagnostics = Arc::new(ready_diagnostics());
                let publisher_cancel = CancellationToken::new();
                let publisher = tokio::spawn(
                    run_dependency_sample_completion_publisher_for_diagnostics(
                    diagnostics.clone(),
                    republish_interval,
                    publisher_cancel.clone(),
                ));

                assert_eq!(
                    sample_completion_metric_value_from_scrape(&handle.render()),
                    None,
                    "absence before the first completion is the not-yet-sampled signal"
                );

                diagnostics.sample(&db, &redis_pool).await;
                let first = wait_for_sample_completion_scrape_value(&handle, |value| value > 0).await;

                tokio::time::sleep(timeout + Duration::from_millis(200)).await;
                assert_eq!(
                    sample_completion_metric_value_from_scrape(&handle.render()),
                    Some(first),
                    "publisher must keep the first completion epoch exported after idle timeout"
                );

                while epoch_seconds_now() <= first {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                diagnostics.sample(&db, &redis_pool).await;
                let second = wait_for_sample_completion_scrape_value(&handle, |value| value > first).await;

                tokio::time::sleep(timeout + Duration::from_millis(200)).await;
                assert_eq!(
                    sample_completion_metric_value_from_scrape(&handle.render()),
                    Some(second),
                    "with no further samples, publisher must keep the latest epoch while the sampler is stopped"
                );

                publisher_cancel.cancel();
                publisher.await.expect("completion publisher task");
            });
        });
    }

    #[test]
    fn readiness_reason_labels_are_the_closed_lifecycle_set() {
        assert_eq!(
            [ReadinessReason::Ready, ReadinessReason::ShuttingDown].map(ReadinessReason::label),
            READINESS_REASON_LABELS
        );
    }
}
