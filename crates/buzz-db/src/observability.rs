//! Bounded-cardinality database pressure instrumentation primitives.
//!
//! Label values come only from the closed enums in this module. Callers must
//! never derive labels from tenant data, events, SQL text, or query identifiers.

use std::future::Future;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PoolRole {
    Writer,
    Reader,
}

impl PoolRole {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 2] = [Self::Writer, Self::Reader];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Reader => "reader",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LockType {
    Replacement,
    Membership,
    PushGate,
    Deletion,
    MigrationSchemaSafety,
}

impl LockType {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 5] = [
        Self::Replacement,
        Self::Membership,
        Self::PushGate,
        Self::Deletion,
        Self::MigrationSchemaSafety,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Replacement => "replacement",
            Self::Membership => "membership",
            Self::PushGate => "push_gate",
            Self::Deletion => "deletion",
            Self::MigrationSchemaSafety => "migration_schema_safety",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Success,
    Error,
    Timeout,
}

impl Outcome {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 3] = [Self::Success, Self::Error, Self::Timeout];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }

    fn from_sqlx_error(error: &sqlx::Error) -> Self {
        match error {
            sqlx::Error::PoolTimedOut => Self::Timeout,
            sqlx::Error::Database(database) if database.code().as_deref() == Some("55P03") => {
                Self::Timeout
            }
            _ => Self::Error,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransactionOperation {
    ReplaceParameterizedEvent,
    ReplaceAddressableEvent,
    PublishNip43MembershipLocked,
    AcceptPushLeaseEvent,
    BeginCommunityDeletionQuiescing,
    FenceCommunityDeletion,
}

impl TransactionOperation {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 6] = [
        Self::ReplaceParameterizedEvent,
        Self::ReplaceAddressableEvent,
        Self::PublishNip43MembershipLocked,
        Self::AcceptPushLeaseEvent,
        Self::BeginCommunityDeletionQuiescing,
        Self::FenceCommunityDeletion,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReplaceParameterizedEvent => "replace_parameterized_event",
            Self::ReplaceAddressableEvent => "replace_addressable_event",
            Self::PublishNip43MembershipLocked => "publish_nip43_membership_locked",
            Self::AcceptPushLeaseEvent => "accept_push_lease_event",
            Self::BeginCommunityDeletionQuiescing => "begin_community_deletion_quiescing",
            Self::FenceCommunityDeletion => "fence_community_deletion",
        }
    }
}

pub(crate) fn record_pool_acquire(role: PoolRole, outcome: Outcome, elapsed: Duration) {
    metrics::histogram!(
        "buzz_db_pool_acquire_wait_seconds",
        "pool_role" => role.as_str(),
        "outcome" => outcome.as_str(),
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        "buzz_db_pool_acquisitions_total",
        "pool_role" => role.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub(crate) async fn acquire(
    pool: &sqlx::PgPool,
    role: PoolRole,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let started = Instant::now();
    let result = pool.acquire().await;
    let outcome = result
        .as_ref()
        .map(|_| Outcome::Success)
        .unwrap_or_else(Outcome::from_sqlx_error);
    record_pool_acquire(role, outcome, started.elapsed());
    result
}

pub(crate) async fn begin_transaction(
    pool: &sqlx::PgPool,
    operation: TransactionOperation,
) -> sqlx::Result<(sqlx::Transaction<'static, sqlx::Postgres>, TransactionTimer)> {
    let connection = acquire(pool, PoolRole::Writer).await?;
    let transaction = sqlx::Transaction::begin(connection, None).await?;
    Ok((transaction, TransactionTimer::start(operation)))
}

pub(crate) async fn observe_advisory_lock<T, F>(lock_type: LockType, future: F) -> sqlx::Result<T>
where
    F: Future<Output = sqlx::Result<T>>,
{
    let started = Instant::now();
    let result = future.await;
    let outcome = result
        .as_ref()
        .map(|_| Outcome::Success)
        .unwrap_or_else(Outcome::from_sqlx_error);
    metrics::histogram!(
        "buzz_db_advisory_lock_wait_seconds",
        "lock_type" => lock_type.as_str(),
        "outcome" => outcome.as_str(),
    )
    .record(started.elapsed().as_secs_f64());
    metrics::counter!(
        "buzz_db_advisory_lock_acquisitions_total",
        "lock_type" => lock_type.as_str(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
    result
}

pub(crate) struct TransactionTimer {
    operation: TransactionOperation,
    started: Instant,
    outcome: Outcome,
}

impl TransactionTimer {
    pub(crate) fn start(operation: TransactionOperation) -> Self {
        Self {
            operation,
            started: Instant::now(),
            outcome: Outcome::Error,
        }
    }

    pub(crate) async fn observe<T, E, F>(mut self, future: F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
    {
        let result = future.await;
        if result.is_ok() {
            self.outcome = Outcome::Success;
        }
        result
    }
}

impl Drop for TransactionTimer {
    fn drop(&mut self) {
        metrics::histogram!(
            "buzz_db_transaction_duration_seconds",
            "operation" => self.operation.as_str(),
            "outcome" => self.outcome.as_str(),
        )
        .record(self.started.elapsed().as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        acquire, observe_advisory_lock, record_pool_acquire, LockType, Outcome, PoolRole,
        TransactionOperation, TransactionTimer,
    };
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    #[test]
    fn label_vocabularies_are_closed_and_documented() {
        assert_eq!(PoolRole::ALL.map(PoolRole::as_str), ["writer", "reader"]);
        assert_eq!(
            LockType::ALL.map(LockType::as_str),
            [
                "replacement",
                "membership",
                "push_gate",
                "deletion",
                "migration_schema_safety",
            ]
        );
        assert_eq!(
            Outcome::ALL.map(Outcome::as_str),
            ["success", "error", "timeout"]
        );
        assert_eq!(
            TransactionOperation::ALL.map(TransactionOperation::as_str),
            [
                "replace_parameterized_event",
                "replace_addressable_event",
                "publish_nip43_membership_locked",
                "accept_push_lease_event",
                "begin_community_deletion_quiescing",
                "fence_community_deletion",
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn transaction_timer_observe_classifies_result_outcomes() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let success = TransactionTimer::start(TransactionOperation::ReplaceParameterizedEvent)
            .observe(async { Ok::<_, &str>("committed") })
            .await;
        assert_eq!(success, Ok("committed"));

        let error = TransactionTimer::start(TransactionOperation::AcceptPushLeaseEvent)
            .observe(async { Err::<(), _>("rollback") })
            .await;
        assert_eq!(error, Err("rollback"));

        let keys = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, ..)| {
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                (key.key().name().to_owned(), labels)
            })
            .collect::<BTreeSet<_>>();

        for (operation, outcome) in [
            ("replace_parameterized_event", "success"),
            ("accept_push_lease_event", "error"),
        ] {
            assert!(keys.contains(&(
                "buzz_db_transaction_duration_seconds".to_owned(),
                [
                    ("operation".to_owned(), operation.to_owned()),
                    ("outcome".to_owned(), outcome.to_owned()),
                ]
                .into_iter()
                .collect(),
            )));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn primitives_record_fixed_success_error_and_timeout_labels() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        record_pool_acquire(
            PoolRole::Writer,
            Outcome::Success,
            Duration::from_millis(12),
        );
        record_pool_acquire(
            PoolRole::Reader,
            Outcome::Timeout,
            Duration::from_millis(34),
        );
        let lock_ok: sqlx::Result<()> =
            observe_advisory_lock(LockType::Replacement, async { Ok(()) }).await;
        assert!(lock_ok.is_ok());
        let lock_error: sqlx::Result<()> =
            observe_advisory_lock(LockType::Membership, async { Err(sqlx::Error::PoolClosed) })
                .await;
        assert!(lock_error.is_err());

        let committed: Result<(), ()> =
            TransactionTimer::start(TransactionOperation::ReplaceParameterizedEvent)
                .observe(async { Ok(()) })
                .await;
        assert!(committed.is_ok());
        let rolled_back: Result<(), ()> =
            TransactionTimer::start(TransactionOperation::AcceptPushLeaseEvent)
                .observe(async { Err(()) })
                .await;
        assert!(rolled_back.is_err());

        let snapshot = snapshotter.snapshot().into_vec();
        let keys = snapshot
            .iter()
            .map(|(key, ..)| {
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                (key.key().name().to_owned(), labels)
            })
            .collect::<BTreeSet<_>>();

        for expected in [
            (
                "buzz_db_pool_acquire_wait_seconds",
                [("outcome", "success"), ("pool_role", "writer")],
            ),
            (
                "buzz_db_pool_acquire_wait_seconds",
                [("outcome", "timeout"), ("pool_role", "reader")],
            ),
            (
                "buzz_db_pool_acquisitions_total",
                [("outcome", "success"), ("pool_role", "writer")],
            ),
            (
                "buzz_db_pool_acquisitions_total",
                [("outcome", "timeout"), ("pool_role", "reader")],
            ),
            (
                "buzz_db_advisory_lock_wait_seconds",
                [("lock_type", "replacement"), ("outcome", "success")],
            ),
            (
                "buzz_db_advisory_lock_wait_seconds",
                [("lock_type", "membership"), ("outcome", "error")],
            ),
            (
                "buzz_db_advisory_lock_acquisitions_total",
                [("lock_type", "replacement"), ("outcome", "success")],
            ),
            (
                "buzz_db_advisory_lock_acquisitions_total",
                [("lock_type", "membership"), ("outcome", "error")],
            ),
            (
                "buzz_db_transaction_duration_seconds",
                [
                    ("operation", "replace_parameterized_event"),
                    ("outcome", "success"),
                ],
            ),
            (
                "buzz_db_transaction_duration_seconds",
                [
                    ("operation", "accept_push_lease_event"),
                    ("outcome", "error"),
                ],
            ),
        ] {
            let expected_labels = expected
                .1
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect::<BTreeMap<_, _>>();
            assert!(
                keys.contains(&(expected.0.to_owned(), expected_labels)),
                "missing metric series {expected:?}; got {keys:?}"
            );
        }

        for (key, _, _, value) in snapshot {
            if key.key().name().ends_with("_seconds") {
                let DebugValue::Histogram(samples) = value else {
                    panic!("seconds metrics must be histograms");
                };
                assert!(samples.iter().all(|sample| sample.into_inner() >= 0.0));
            } else if key.key().name().ends_with("_total") {
                let DebugValue::Counter(value) = value else {
                    panic!("total metrics must be counters");
                };
                assert_eq!(value, 1);
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires Postgres"]
    async fn pool_acquire_records_success_timeout_and_error_with_wait_time() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned()); // sadscan:disable np.postgres.1 -- local test-only credentials
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(75))
            .connect(&database_url)
            .await
            .expect("connect size-one test pool");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let held = acquire(&pool, PoolRole::Writer)
            .await
            .expect("writer acquire succeeds");
        let timeout = acquire(&pool, PoolRole::Reader)
            .await
            .expect_err("reader-labeled checkout times out while pool is saturated");
        assert!(matches!(timeout, sqlx::Error::PoolTimedOut));
        drop(held);
        pool.close().await;
        let closed = acquire(&pool, PoolRole::Writer)
            .await
            .expect_err("closed pool acquire errors");
        assert!(matches!(closed, sqlx::Error::PoolClosed));

        let mut outcomes = BTreeMap::<(String, String), Vec<f64>>::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            if key.key().name() != "buzz_db_pool_acquire_wait_seconds" {
                continue;
            }
            let DebugValue::Histogram(samples) = value else {
                panic!("pool wait must be a histogram");
            };
            let labels = key.key().labels().collect::<Vec<_>>();
            let label = |name: &str| {
                labels
                    .iter()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
                    .unwrap_or_default()
            };
            outcomes.insert(
                (label("pool_role"), label("outcome")),
                samples
                    .into_iter()
                    .map(|sample| sample.into_inner())
                    .collect(),
            );
        }
        assert!(outcomes.contains_key(&("writer".to_owned(), "success".to_owned())));
        assert!(outcomes.contains_key(&("writer".to_owned(), "error".to_owned())));
        let timeout_samples = outcomes
            .get(&("reader".to_owned(), "timeout".to_owned()))
            .expect("reader timeout series");
        assert!(
            timeout_samples.iter().any(|sample| *sample >= 0.05),
            "timeout wait must include the saturated checkout delay: {timeout_samples:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires Postgres"]
    async fn advisory_lock_records_success_contention_timeout_and_error() {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned()); // sadscan:disable np.postgres.1 -- local test-only credentials
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect advisory-lock test pool");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let mut success_tx = pool.begin().await.expect("begin success transaction");
        observe_advisory_lock(
            LockType::Replacement,
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(0x62757a7a6f627331_i64)
                .execute(&mut *success_tx),
        )
        .await
        .expect("uncontended lock succeeds");
        success_tx
            .rollback()
            .await
            .expect("rollback success transaction");

        let contention_key = 0x62757a7a6f627332_i64;
        let mut holder = pool.begin().await.expect("begin lock holder");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(contention_key)
            .execute(&mut *holder)
            .await
            .expect("holder acquires contention key");
        let mut waiter = pool.begin().await.expect("begin lock waiter");
        let waiter_task = tokio::spawn(async move {
            let result = observe_advisory_lock(
                LockType::Deletion,
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(contention_key)
                    .execute(&mut *waiter),
            )
            .await;
            (waiter, result)
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            !waiter_task.is_finished(),
            "waiter must be blocked by holder"
        );
        holder.commit().await.expect("release contention key");
        let (waiter, waited) = waiter_task.await.expect("join lock waiter");
        waited.expect("contended lock succeeds after release");
        waiter.rollback().await.expect("rollback waiter");

        let timeout_key = 0x62757a7a6f627333_i64;
        let mut timeout_holder = pool.begin().await.expect("begin timeout holder");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(timeout_key)
            .execute(&mut *timeout_holder)
            .await
            .expect("holder acquires timeout key");
        let mut timeout_waiter = pool.begin().await.expect("begin timeout waiter");
        sqlx::query("SET LOCAL lock_timeout = '30ms'")
            .execute(&mut *timeout_waiter)
            .await
            .expect("set test-only lock timeout");
        let timed_out = observe_advisory_lock(
            LockType::MigrationSchemaSafety,
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(timeout_key)
                .execute(&mut *timeout_waiter),
        )
        .await
        .expect_err("lock wait times out");
        assert_eq!(
            timed_out
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("55P03")
        );
        timeout_holder
            .rollback()
            .await
            .expect("release timeout key");

        let mut aborted = pool.begin().await.expect("begin error transaction");
        sqlx::query("SELECT 1 / 0")
            .execute(&mut *aborted)
            .await
            .expect_err("abort transaction before lock");
        observe_advisory_lock(
            LockType::Membership,
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(0x62757a7a6f627334_i64)
                .execute(&mut *aborted),
        )
        .await
        .expect_err("lock statement fails in aborted transaction");
        aborted
            .rollback()
            .await
            .expect("rollback aborted transaction");

        let mut outcomes = BTreeMap::<(String, String), Vec<f64>>::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            if key.key().name() != "buzz_db_advisory_lock_wait_seconds" {
                continue;
            }
            let DebugValue::Histogram(samples) = value else {
                panic!("lock wait must be a histogram");
            };
            let labels = key.key().labels().collect::<Vec<_>>();
            let label = |name: &str| {
                labels
                    .iter()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
                    .unwrap_or_default()
            };
            outcomes.insert(
                (label("lock_type"), label("outcome")),
                samples
                    .into_iter()
                    .map(|sample| sample.into_inner())
                    .collect(),
            );
        }
        assert!(outcomes.contains_key(&("replacement".to_owned(), "success".to_owned())));
        assert!(outcomes.contains_key(&("membership".to_owned(), "error".to_owned())));
        assert!(
            outcomes.contains_key(&("migration_schema_safety".to_owned(), "timeout".to_owned()))
        );
        let contention = outcomes
            .get(&("deletion".to_owned(), "success".to_owned()))
            .expect("deletion contention series");
        assert!(
            contention.iter().any(|sample| *sample >= 0.04),
            "lock timer must include the holder wait: {contention:?}"
        );
    }
}
