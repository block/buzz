//! Bounded-cardinality database pressure instrumentation primitives.
//!
//! Label values come only from the closed enums in this module. Callers must
//! never derive labels from tenant data, events, SQL text, or query identifiers.

use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One valid pool/operation acquisition family.
///
/// Keeping role and operation in one enum makes invalid combinations
/// unrepresentable at call sites and gives the series budget one exhaustive
/// source of truth.
#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PoolOperation {
    WriterBootstrap,
    ReaderBootstrap,
    WriterReadiness,
    WriterTenantResolution,
    WriterAuthentication,
    WriterAuthorization,
    ReaderAuthorization,
    WriterSubscriptionHistory,
    ReaderSubscriptionHistory,
    WriterEventWrite,
    WriterMaintenance,
}

/// Writer-pool operations. Reader-only combinations cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WriterOperation {
    Bootstrap,
    Readiness,
    TenantResolution,
    Authentication,
    Authorization,
    SubscriptionHistory,
    EventWrite,
    Maintenance,
}

impl WriterOperation {
    #[cfg(test)]
    const ALL: [Self; 8] = [
        Self::Bootstrap,
        Self::Readiness,
        Self::TenantResolution,
        Self::Authentication,
        Self::Authorization,
        Self::SubscriptionHistory,
        Self::EventWrite,
        Self::Maintenance,
    ];

    const fn pair(self) -> PoolOperation {
        match self {
            Self::Bootstrap => PoolOperation::WriterBootstrap,
            Self::Readiness => PoolOperation::WriterReadiness,
            Self::TenantResolution => PoolOperation::WriterTenantResolution,
            Self::Authentication => PoolOperation::WriterAuthentication,
            Self::Authorization => PoolOperation::WriterAuthorization,
            Self::SubscriptionHistory => PoolOperation::WriterSubscriptionHistory,
            Self::EventWrite => PoolOperation::WriterEventWrite,
            Self::Maintenance => PoolOperation::WriterMaintenance,
        }
    }
}

/// Reader-pool operations. Writer-only combinations cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReaderOperation {
    Bootstrap,
    Authorization,
    SubscriptionHistory,
}

impl ReaderOperation {
    #[cfg(test)]
    const ALL: [Self; 3] = [
        Self::Bootstrap,
        Self::Authorization,
        Self::SubscriptionHistory,
    ];

    const fn pair(self) -> PoolOperation {
        match self {
            Self::Bootstrap => PoolOperation::ReaderBootstrap,
            Self::Authorization => PoolOperation::ReaderAuthorization,
            Self::SubscriptionHistory => PoolOperation::ReaderSubscriptionHistory,
        }
    }
}

impl PoolOperation {
    pub(crate) const ALL: [Self; 11] = [
        Self::WriterBootstrap,
        Self::ReaderBootstrap,
        Self::WriterReadiness,
        Self::WriterTenantResolution,
        Self::WriterAuthentication,
        Self::WriterAuthorization,
        Self::ReaderAuthorization,
        Self::WriterSubscriptionHistory,
        Self::ReaderSubscriptionHistory,
        Self::WriterEventWrite,
        Self::WriterMaintenance,
    ];

    pub(crate) const fn pool_role(self) -> &'static str {
        match self {
            Self::ReaderBootstrap | Self::ReaderAuthorization | Self::ReaderSubscriptionHistory => {
                "reader"
            }
            _ => "writer",
        }
    }

    pub(crate) const fn operation(self) -> &'static str {
        match self {
            Self::WriterBootstrap | Self::ReaderBootstrap => "bootstrap",
            Self::WriterReadiness => "readiness",
            Self::WriterTenantResolution => "tenant_resolution",
            Self::WriterAuthentication => "authentication",
            Self::WriterAuthorization | Self::ReaderAuthorization => "authorization",
            Self::WriterSubscriptionHistory | Self::ReaderSubscriptionHistory => {
                "subscription_history"
            }
            Self::WriterEventWrite => "event_write",
            Self::WriterMaintenance => "maintenance",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

pub(crate) const POOL_ACQUIRE_VALID_PAIRS: [(&str, &str); 11] = [
    ("writer", "bootstrap"),
    ("reader", "bootstrap"),
    ("writer", "readiness"),
    ("writer", "tenant_resolution"),
    ("writer", "authentication"),
    ("writer", "authorization"),
    ("reader", "authorization"),
    ("writer", "subscription_history"),
    ("reader", "subscription_history"),
    ("writer", "event_write"),
    ("writer", "maintenance"),
];

/// Eleven valid pairs × (12 histogram series + 4 outcome counters + 1 gauge).
pub(crate) const POOL_ACQUIRE_RAW_SERIES_PER_POD: usize = POOL_ACQUIRE_VALID_PAIRS.len() * 17;

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
    Cancelled,
}

impl Outcome {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [Self::Success, Self::Error, Self::Timeout, Self::Cancelled];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
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

    const fn writer_operation(self) -> WriterOperation {
        match self {
            Self::ReplaceParameterizedEvent
            | Self::ReplaceAddressableEvent
            | Self::PublishNip43MembershipLocked
            | Self::AcceptPushLeaseEvent => WriterOperation::EventWrite,
            Self::BeginCommunityDeletionQuiescing | Self::FenceCommunityDeletion => {
                WriterOperation::Maintenance
            }
        }
    }
}

fn record_pool_acquire(
    pair: PoolOperation,
    outcome: Outcome,
    elapsed: Duration,
    emit_legacy: bool,
) {
    // Preserve the original observed population for existing dashboards.
    // Newly instrumented raw-pool seams must not create a deployment-time
    // discontinuity in these compatibility families.
    if emit_legacy && outcome != Outcome::Cancelled {
        metrics::histogram!(
            "buzz_db_pool_acquire_wait_seconds",
            "pool_role" => pair.pool_role(),
            "outcome" => outcome.as_str(),
        )
        .record(elapsed.as_secs_f64());
        metrics::counter!(
            "buzz_db_pool_acquisitions_total",
            "pool_role" => pair.pool_role(),
            "outcome" => outcome.as_str(),
        )
        .increment(1);
    }

    metrics::histogram!(
        "buzz_db_pool_acquire_duration_seconds",
        "pool_role" => pair.pool_role(),
        "operation" => pair.operation(),
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        "buzz_db_pool_acquire_attempts_total",
        "pool_role" => pair.pool_role(),
        "operation" => pair.operation(),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

static POOL_WAITERS: [Mutex<u64>; PoolOperation::ALL.len()] =
    [const { Mutex::new(0) }; PoolOperation::ALL.len()];

#[cfg(test)]
static POOL_METRICS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
#[derive(Clone)]
struct WaiterPublishTestHook {
    pair: PoolOperation,
    value: u64,
    entered: std::sync::Arc<std::sync::Barrier>,
    release: std::sync::Arc<std::sync::Barrier>,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
static WAITER_PUBLISH_TEST_HOOK: Mutex<Option<WaiterPublishTestHook>> = Mutex::new(None);

#[cfg(test)]
static WAITER_LAST_PUBLISHED: [Mutex<u64>; PoolOperation::ALL.len()] =
    [const { Mutex::new(u64::MAX) }; PoolOperation::ALL.len()];

fn publish_waiters(pair: PoolOperation, value: u64) {
    #[cfg(test)]
    {
        let hook = WAITER_PUBLISH_TEST_HOOK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            if hook.pair == pair
                && hook.value == value
                && hook.armed.swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                hook.entered.wait();
                hook.release.wait();
            }
        }
        *WAITER_LAST_PUBLISHED[pair.index()]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
    }
    metrics::gauge!(
        "buzz_db_pool_waiters",
        "pool_role" => pair.pool_role(),
        "operation" => pair.operation(),
    )
    .set(value as f64);
}

/// Re-publish every valid waiter pair, including healthy zero, so exporter
/// idle eviction cannot turn an expected zero into ambiguous missing data.
pub(crate) fn refresh_pool_waiters(include_reader: bool) {
    for pair in PoolOperation::ALL {
        if pair.pool_role() == "reader" && !include_reader {
            continue;
        }
        let waiters = POOL_WAITERS[pair.index()]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        publish_waiters(pair, *waiters);
    }
}

/// Owns one polled connection acquisition until exactly one terminal.
///
/// Because async function bodies do not run until first poll, a future that is
/// constructed and immediately dropped emits nothing. Once armed, dropping it
/// while awaiting SQLx records `cancelled`, duration, and the balanced waiter
/// decrement.
struct PoolAcquireAttempt {
    pair: PoolOperation,
    started: Instant,
    emit_legacy: bool,
    terminal: bool,
}

impl PoolAcquireAttempt {
    fn start(pair: PoolOperation, emit_legacy: bool) -> Self {
        {
            let mut waiters = POOL_WAITERS[pair.index()]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *waiters += 1;
            publish_waiters(pair, *waiters);
        }
        Self {
            pair,
            started: Instant::now(),
            emit_legacy,
            terminal: false,
        }
    }

    fn finish(mut self, outcome: Outcome) {
        self.terminal = true;
        record_pool_acquire(self.pair, outcome, self.started.elapsed(), self.emit_legacy);
        self.release_waiter();
    }

    fn release_waiter(&self) {
        let mut waiters = POOL_WAITERS[self.pair.index()]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(*waiters > 0, "pool waiter balance underflow");
        *waiters = waiters.saturating_sub(1);
        publish_waiters(self.pair, *waiters);
    }
}

impl Drop for PoolAcquireAttempt {
    fn drop(&mut self) {
        if !self.terminal {
            record_pool_acquire(
                self.pair,
                Outcome::Cancelled,
                self.started.elapsed(),
                self.emit_legacy,
            );
            self.release_waiter();
            self.terminal = true;
        }
    }
}

async fn acquire(
    pool: &sqlx::PgPool,
    pair: PoolOperation,
    emit_legacy: bool,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let attempt = PoolAcquireAttempt::start(pair, emit_legacy);
    let result = pool.acquire().await;
    let outcome = result
        .as_ref()
        .map(|_| Outcome::Success)
        .unwrap_or_else(Outcome::from_sqlx_error);
    attempt.finish(outcome);
    result
}

/// Acquire from an authoritative writer pool for one valid writer operation.
pub(crate) async fn acquire_writer(
    pool: &sqlx::PgPool,
    operation: WriterOperation,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    acquire(pool, operation.pair(), false).await
}

/// Acquire from a writer seam already covered by the pre-operation metric.
pub(crate) async fn acquire_writer_with_legacy_metrics(
    pool: &sqlx::PgPool,
    operation: WriterOperation,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    acquire(pool, operation.pair(), true).await
}

/// Acquire from a reader seam already covered by the pre-operation metric.
pub(super) async fn acquire_reader_with_legacy_metrics(
    pool: &sqlx::PgPool,
    operation: ReaderOperation,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    acquire(pool, operation.pair(), true).await
}

/// Acquire within an operation-owned absolute deadline.
///
/// A deadline expiry is a timeout terminal. Dropping the enclosing future
/// before that deadline remains a cancellation terminal.
pub(crate) async fn acquire_writer_until(
    pool: &sqlx::PgPool,
    operation: WriterOperation,
    deadline: tokio::time::Instant,
) -> sqlx::Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let pair = operation.pair();
    let attempt = PoolAcquireAttempt::start(pair, false);
    match tokio::time::timeout_at(deadline, pool.acquire()).await {
        Err(_) => {
            attempt.finish(Outcome::Timeout);
            Err(sqlx::Error::PoolTimedOut)
        }
        Ok(result) => {
            let outcome = result
                .as_ref()
                .map(|_| Outcome::Success)
                .unwrap_or_else(Outcome::from_sqlx_error);
            attempt.finish(outcome);
            result
        }
    }
}

pub(crate) async fn begin_transaction(
    pool: &sqlx::PgPool,
    operation: TransactionOperation,
) -> sqlx::Result<(sqlx::Transaction<'static, sqlx::Postgres>, TransactionTimer)> {
    let connection = acquire_writer_with_legacy_metrics(pool, operation.writer_operation()).await?;
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
        acquire_reader_with_legacy_metrics, acquire_writer, acquire_writer_with_legacy_metrics,
        observe_advisory_lock, record_pool_acquire, refresh_pool_waiters, LockType, Outcome,
        PoolAcquireAttempt, PoolOperation, ReaderOperation, TransactionOperation, TransactionTimer,
        WriterOperation,
    };
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    #[test]
    fn label_vocabularies_are_closed_and_documented() {
        assert_eq!(
            PoolOperation::ALL.map(|pair| (pair.pool_role(), pair.operation())),
            super::POOL_ACQUIRE_VALID_PAIRS
        );
        assert_eq!(
            WriterOperation::ALL.map(WriterOperation::pair),
            [
                PoolOperation::WriterBootstrap,
                PoolOperation::WriterReadiness,
                PoolOperation::WriterTenantResolution,
                PoolOperation::WriterAuthentication,
                PoolOperation::WriterAuthorization,
                PoolOperation::WriterSubscriptionHistory,
                PoolOperation::WriterEventWrite,
                PoolOperation::WriterMaintenance,
            ]
        );
        assert_eq!(
            ReaderOperation::ALL.map(ReaderOperation::pair),
            [
                PoolOperation::ReaderBootstrap,
                PoolOperation::ReaderAuthorization,
                PoolOperation::ReaderSubscriptionHistory,
            ]
        );
        assert_eq!(super::POOL_ACQUIRE_RAW_SERIES_PER_POD, 187);
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
            ["success", "error", "timeout", "cancelled"]
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
            PoolOperation::WriterReadiness,
            Outcome::Success,
            Duration::from_millis(12),
            false,
        );
        record_pool_acquire(
            PoolOperation::ReaderSubscriptionHistory,
            Outcome::Timeout,
            Duration::from_millis(34),
            true,
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
                [("outcome", "timeout"), ("pool_role", "reader")],
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
        for name in [
            "buzz_db_pool_acquire_wait_seconds",
            "buzz_db_pool_acquisitions_total",
        ] {
            assert!(
                !keys.contains(&(
                    name.to_owned(),
                    [
                        ("outcome".to_owned(), "success".to_owned()),
                        ("pool_role".to_owned(), "writer".to_owned()),
                    ]
                    .into_iter()
                    .collect(),
                )),
                "newly instrumented seams must not expand legacy metric population"
            );
        }

        for (name, labels) in [
            (
                "buzz_db_pool_acquire_duration_seconds",
                [("operation", "readiness"), ("pool_role", "writer")],
            ),
            (
                "buzz_db_pool_acquire_duration_seconds",
                [
                    ("operation", "subscription_history"),
                    ("pool_role", "reader"),
                ],
            ),
        ] {
            let labels = labels
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();
            assert!(
                keys.contains(&(name.to_owned(), labels)),
                "missing operation-aware pool duration for {name}"
            );
        }
        for (pool_role, operation, outcome) in [
            ("writer", "readiness", "success"),
            ("reader", "subscription_history", "timeout"),
        ] {
            assert!(keys.contains(&(
                "buzz_db_pool_acquire_attempts_total".to_owned(),
                [
                    ("operation".to_owned(), operation.to_owned()),
                    ("outcome".to_owned(), outcome.to_owned()),
                    ("pool_role".to_owned(), pool_role.to_owned()),
                ]
                .into_iter()
                .collect(),
            )));
        }
        assert!(keys.iter().all(|(name, labels)| {
            name != "buzz_db_pool_acquire_duration_seconds"
                || (!labels.contains_key("outcome") && !labels.contains_key("result"))
        }));

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

    #[test]
    fn cancelled_attempt_records_terminal_and_refreshes_zero() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.blocking_lock();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let attempt = PoolAcquireAttempt::start(PoolOperation::WriterTenantResolution, false);
        drop(attempt);
        refresh_pool_waiters(true);

        let mut saw_cancelled = false;
        let mut saw_zero = false;
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<BTreeMap<_, _>>();
            if labels.get("operation") != Some(&"tenant_resolution") {
                continue;
            }
            match key.key().name() {
                "buzz_db_pool_acquire_attempts_total" => {
                    let DebugValue::Counter(value) = value else {
                        panic!("attempt terminals must be a counter");
                    };
                    saw_cancelled = labels.get("outcome") == Some(&"cancelled") && value == 1;
                }
                "buzz_db_pool_waiters" => {
                    let DebugValue::Gauge(value) = value else {
                        panic!("pool waiters must be a gauge");
                    };
                    saw_zero = value.into_inner() == 0.0;
                }
                _ => {}
            }
        }
        assert!(
            saw_cancelled,
            "dropped armed attempt must terminalize cancellation"
        );
        assert!(saw_zero, "periodic refresh must publish a healthy zero");
    }

    #[test]
    fn waiter_refresh_omits_reader_pairs_when_no_reader_pool_is_configured() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.blocking_lock();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        refresh_pool_waiters(false);

        let published = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_db_pool_waiters" {
                    return None;
                }
                let DebugValue::Gauge(value) = value else {
                    panic!("pool waiters must be gauges");
                };
                assert_eq!(value.into_inner(), 0.0);
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                Some((labels["pool_role"].clone(), labels["operation"].clone()))
            })
            .collect::<BTreeSet<_>>();
        let expected = WriterOperation::ALL
            .into_iter()
            .map(|operation| {
                let pair = operation.pair();
                (pair.pool_role().to_owned(), pair.operation().to_owned())
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(published, expected);
        assert!(published.iter().all(|(pool_role, _)| pool_role == "writer"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn compatibility_metrics_only_cover_preexisting_acquisition_seams() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.lock().await;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(&crate::test_support::database_url())
            .expect("construct lazy compatibility test pool");
        pool.close().await;

        let error = acquire_writer(&pool, WriterOperation::EventWrite)
            .await
            .expect_err("closed newly instrumented seam errors");
        assert!(matches!(error, sqlx::Error::PoolClosed));
        assert_eq!(
            legacy_acquisition_count(&snapshotter.snapshot().into_vec()),
            0
        );

        let error = acquire_writer_with_legacy_metrics(&pool, WriterOperation::EventWrite)
            .await
            .expect_err("closed legacy seam errors");
        assert!(matches!(error, sqlx::Error::PoolClosed));
        assert_eq!(
            legacy_acquisition_count(&snapshotter.snapshot().into_vec()),
            1
        );
    }

    fn legacy_acquisition_count(
        snapshot: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
    ) -> u64 {
        snapshot
            .iter()
            .filter_map(|(key, _, _, value)| {
                (key.key().name() == "buzz_db_pool_acquisitions_total")
                    .then_some(value)
                    .map(|value| match value {
                        DebugValue::Counter(value) => *value,
                        _ => panic!("legacy acquisitions must be a counter"),
                    })
            })
            .sum()
    }

    #[test]
    fn concurrent_attempts_publish_an_exact_balanced_waiter_count() {
        const ATTEMPTS: usize = 8;

        let _test_guard = super::POOL_METRICS_TEST_LOCK.blocking_lock();
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let armed = Arc::new(Barrier::new(ATTEMPTS + 1));
        let release = Arc::new(Barrier::new(ATTEMPTS + 1));
        let threads = (0..ATTEMPTS)
            .map(|_| {
                let armed = Arc::clone(&armed);
                let release = Arc::clone(&release);
                std::thread::spawn(move || {
                    let attempt =
                        PoolAcquireAttempt::start(PoolOperation::WriterTenantResolution, false);
                    armed.wait();
                    release.wait();
                    drop(attempt);
                })
            })
            .collect::<Vec<_>>();

        armed.wait();
        refresh_pool_waiters(true);
        let live = waiter_value(
            &snapshotter.snapshot().into_vec(),
            "writer",
            "tenant_resolution",
        );
        assert_eq!(live, Some(ATTEMPTS as f64));

        release.wait();
        for thread in threads {
            thread.join().expect("waiter thread completes");
        }
        refresh_pool_waiters(true);
        let balanced = waiter_value(
            &snapshotter.snapshot().into_vec(),
            "writer",
            "tenant_resolution",
        );
        assert_eq!(balanced, Some(0.0));
    }

    #[test]
    fn waiter_publication_is_serialized_with_state_mutation() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.blocking_lock();
        let pair = PoolOperation::WriterTenantResolution;
        let first = PoolAcquireAttempt::start(pair, false);
        let second = PoolAcquireAttempt::start(pair, false);
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        *super::WAITER_PUBLISH_TEST_HOOK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(super::WaiterPublishTestHook {
                pair,
                value: 1,
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                armed: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            });

        let first_drop = std::thread::spawn(move || drop(first));
        entered.wait();
        let mutation_lock_held = super::POOL_WAITERS[pair.index()].try_lock().is_err();
        release.wait();
        first_drop.join().expect("first drop completes");
        drop(second);
        *super::WAITER_PUBLISH_TEST_HOOK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

        assert!(
            mutation_lock_held,
            "waiter state mutation must remain locked until its publication completes"
        );
        assert_eq!(
            *super::WAITER_LAST_PUBLISHED[pair.index()]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            0,
            "the final directly published waiter value must be balanced without a refresh"
        );
    }

    fn waiter_value(
        snapshot: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
        pool_role: &str,
        operation: &str,
    ) -> Option<f64> {
        snapshot.iter().find_map(|(key, _, _, value)| {
            let labels = key.key().labels().collect::<Vec<_>>();
            if key.key().name() != "buzz_db_pool_waiters"
                || !labels
                    .iter()
                    .any(|label| label.key() == "pool_role" && label.value() == pool_role)
                || !labels
                    .iter()
                    .any(|label| label.key() == "operation" && label.value() == operation)
            {
                return None;
            }
            let DebugValue::Gauge(value) = value else {
                panic!("pool waiters must be gauges");
            };
            Some(value.into_inner())
        })
    }

    async fn pool_acquire_records_success_timeout_and_error_with_wait_time() {
        // This timeout also bounds the pool's initial connection. Leave enough
        // headroom for a cold PostgreSQL start under the lane's eight workers;
        // the assertion below cares about classification, not a sub-second
        // synthetic timeout budget.
        let database_url = crate::test_support::database_url();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&database_url)
            .await
            .expect("connect size-one test pool");
        let reader_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&database_url)
            .await
            .expect("connect size-one reader test pool");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let held = acquire_writer_with_legacy_metrics(&pool, WriterOperation::EventWrite)
            .await
            .expect("writer acquire succeeds");
        let mut cancelled = Box::pin(acquire_writer_with_legacy_metrics(
            &pool,
            WriterOperation::Authentication,
        ));
        tokio::select! {
            result = &mut cancelled => panic!("blocked acquisition unexpectedly completed: {result:?}"),
            () = tokio::time::sleep(Duration::from_millis(40)) => {}
        }
        let before_cancel = snapshotter.snapshot().into_vec();
        let live_waiter = before_cancel.iter().find_map(|(key, _, _, value)| {
            let labels = key.key().labels().collect::<Vec<_>>();
            if key.key().name() != "buzz_db_pool_waiters"
                || !labels
                    .iter()
                    .any(|label| label.key() == "pool_role" && label.value() == "writer")
                || !labels
                    .iter()
                    .any(|label| label.key() == "operation" && label.value() == "authentication")
            {
                return None;
            }
            let DebugValue::Gauge(value) = value else {
                panic!("pool waiters must be a gauge");
            };
            Some(value.into_inner())
        });
        assert_eq!(live_waiter, Some(1.0));
        let legacy_before_cancel = legacy_acquisition_count(&before_cancel);
        assert_eq!(
            legacy_before_cancel, 1,
            "the completed legacy acquisition must be counted exactly once"
        );
        let writer_success = before_cancel.iter().any(|(key, _, _, value)| {
            if key.key().name() != "buzz_db_pool_acquire_wait_seconds" {
                return false;
            }
            let labels = key.key().labels().collect::<Vec<_>>();
            let DebugValue::Histogram(samples) = value else {
                panic!("pool wait must be a histogram");
            };
            labels
                .iter()
                .any(|label| label.key() == "pool_role" && label.value() == "writer")
                && labels
                    .iter()
                    .any(|label| label.key() == "outcome" && label.value() == "success")
                && !samples.is_empty()
        });
        drop(cancelled);
        let after_cancel = snapshotter.snapshot().into_vec();
        let legacy_after_cancel = legacy_acquisition_count(&after_cancel);
        let mut cancelled_terminal = None;
        let mut balanced_waiter = None;
        for (key, _, _, value) in after_cancel {
            let labels = key.key().labels().collect::<Vec<_>>();
            let label = |name: &str| {
                labels
                    .iter()
                    .find(|label| label.key() == name)
                    .map(|label| label.value())
            };
            if label("pool_role") != Some("writer") || label("operation") != Some("authentication")
            {
                continue;
            }
            match key.key().name() {
                "buzz_db_pool_waiters" => {
                    let DebugValue::Gauge(value) = value else {
                        panic!("pool waiters must be a gauge");
                    };
                    balanced_waiter = Some(value.into_inner());
                }
                "buzz_db_pool_acquire_attempts_total" if label("outcome") == Some("cancelled") => {
                    let DebugValue::Counter(value) = value else {
                        panic!("cancelled acquisition terminal must be a counter");
                    };
                    cancelled_terminal = Some(value);
                }
                _ => {}
            }
        }
        assert_eq!(balanced_waiter, Some(0.0));
        assert_eq!(cancelled_terminal, Some(1));
        assert_eq!(
            legacy_after_cancel, 0,
            "cancelling a legacy seam must not expand its historical population"
        );
        let held_reader = reader_pool
            .acquire()
            .await
            .expect("hold the reader test connection");
        let timeout =
            acquire_reader_with_legacy_metrics(&reader_pool, ReaderOperation::SubscriptionHistory)
                .await
                .expect_err("reader-labeled checkout times out while pool is saturated");
        assert!(matches!(timeout, sqlx::Error::PoolTimedOut));
        drop(held_reader);
        drop(held);
        pool.close().await;
        let closed = acquire_writer_with_legacy_metrics(&pool, WriterOperation::Readiness)
            .await
            .expect_err("closed pool acquire errors");
        assert!(matches!(closed, sqlx::Error::PoolClosed));

        let mut outcomes = BTreeMap::<(String, String), Vec<f64>>::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            let labels = key.key().labels().collect::<Vec<_>>();
            let label = |name: &str| {
                labels
                    .iter()
                    .find(|label| label.key() == name)
                    .map(|label| label.value().to_owned())
                    .unwrap_or_default()
            };
            if key.key().name() == "buzz_db_pool_acquire_wait_seconds" {
                let DebugValue::Histogram(samples) = value else {
                    panic!("pool wait must be a histogram");
                };
                if samples.is_empty() {
                    continue;
                }
                outcomes.insert(
                    (label("pool_role"), label("outcome")),
                    samples
                        .into_iter()
                        .map(|sample| sample.into_inner())
                        .collect(),
                );
            }
        }
        assert!(writer_success);
        assert!(
            !outcomes.contains_key(&("writer".to_owned(), "cancelled".to_owned())),
            "legacy compatibility families must not add a cancellation population"
        );
        assert!(outcomes.contains_key(&("writer".to_owned(), "error".to_owned())));
        let timeout_samples = outcomes
            .get(&("reader".to_owned(), "timeout".to_owned()))
            .expect("reader timeout series");
        assert!(
            timeout_samples.iter().any(|sample| *sample >= 0.05),
            "timeout wait must include the saturated checkout delay: {timeout_samples:?}"
        );
    }

    async fn deletion_catalog_readiness_records_timeout_and_recovers() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.lock().await;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&crate::test_support::database_url())
            .await
            .expect("connect size-one deletion readiness pool");
        let db = crate::Db::from_pool(pool.clone());
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let held = pool.acquire().await.expect("hold the only pool connection");
        let timeout = db
            .validate_deletion_serving_catalog_for_readiness(
                tokio::time::Instant::now() + Duration::from_millis(40),
            )
            .await
            .expect_err("saturated deletion catalog checkout must time out");
        assert!(matches!(
            timeout,
            crate::DbError::Sqlx(sqlx::Error::PoolTimedOut)
        ));
        drop(held);
        db.validate_deletion_serving_catalog_for_readiness(
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .expect("deletion catalog readiness must recover after pool release");

        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(
            waiter_value(&snapshot, "writer", "readiness"),
            Some(0.0),
            "deadline terminal must directly balance the readiness waiter"
        );
        for outcome in ["timeout", "success"] {
            assert!(
                snapshot.iter().any(|(key, _, _, value)| {
                    if key.key().name() != "buzz_db_pool_acquire_attempts_total" {
                        return false;
                    }
                    let labels = key.key().labels().collect::<Vec<_>>();
                    let has = |name: &str, expected: &str| {
                        labels
                            .iter()
                            .any(|label| label.key() == name && label.value() == expected)
                    };
                    has("pool_role", "writer")
                        && has("operation", "readiness")
                        && has("outcome", outcome)
                        && matches!(value, DebugValue::Counter(1))
                }),
                "missing writer/readiness/{outcome} acquisition terminal"
            );
        }
    }

    async fn production_db_methods_emit_exact_pool_operation_labels() {
        use buzz_core::CommunityId;
        use chrono::Utc;
        use uuid::Uuid;

        let database_url = crate::test_support::database_url();
        let writer_pool = crate::Db::connect_writer_pool(&crate::DbConfig {
            database_url: database_url.clone(),
            max_connections: 4,
            min_connections: 0,
            acquire_timeout_secs: 5,
            ..crate::DbConfig::default()
        })
        .await
        .expect("connect production-method writer pool");
        let reader_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&database_url)
            .await
            .expect("connect production-method reader pool");
        let writer_db = crate::Db::from_pool(writer_pool.clone());
        let mut routed_db = crate::Db::from_pools(writer_pool.clone(), reader_pool);
        routed_db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(5)));

        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let test_scope = CommunityId::from_uuid(Uuid::new_v4());
        let query = crate::EventQuery::for_community(test_scope);

        routed_db.read_pool_boot_ping_for_tests().await;
        routed_db
            .verify_replica_fence_at_boot()
            .await
            .expect("real startup fence verification succeeds");
        let _ = crate::replica_fence::probe_once(&writer_pool, routed_db.fence()).await;
        routed_db.fence().force_open_for_tests(Utc::now());
        assert_eq!(
            writer_db
                .readiness_check(tokio::time::Instant::now() + Duration::from_secs(1))
                .await,
            crate::DbReadinessOutcome::Success
        );
        let _ = writer_db
            .lookup_community_by_host("pool-operation-matrix.invalid")
            .await;
        let _ = writer_db
            .lookup_community_by_host_for_management("pool-operation-matrix.invalid")
            .await;
        let _ = writer_db.list_communities_owned_by(&"a".repeat(64)).await;
        let _ = writer_db.lookup_community_host(test_scope).await;
        let _ = writer_db
            .set_community_icon(test_scope, Some("pool-operation-matrix"))
            .await;
        let _ = writer_db
            .create_community_with_owner(
                &format!("pool-operation-matrix-{}.invalid", Uuid::new_v4().simple()),
                &"b".repeat(64),
            )
            .await;
        let _ = writer_db
            .archive_community_owned_by(
                "pool-operation-matrix.invalid",
                &"c".repeat(64),
                "protected.invalid",
            )
            .await;
        let _ = writer_db
            .unarchive_community_owned_by("pool-operation-matrix.invalid", &"c".repeat(64))
            .await;
        let _ = writer_db.community_of_channel(Uuid::new_v4()).await;
        let _ = writer_db.communities_of_channels(&[Uuid::new_v4()]).await;
        let _ = writer_db
            .ensure_user_for_authorization(test_scope, &[17; 32])
            .await;
        let _ = writer_db
            .set_agent_owner_for_authorization(test_scope, &[18; 32], &[19; 32])
            .await;
        let _ = writer_db.is_pubkey_allowed(test_scope, &[7; 32]).await;
        let _ = writer_db
            .is_agent_owner(test_scope, &[8; 32], &[9; 32])
            .await;
        let _ = writer_db
            .moderation_restriction_state(test_scope, &[14; 32])
            .await;
        let _ = writer_db
            .get_agent_channel_policy(test_scope, &[15; 32])
            .await;
        let _ = writer_db
            .get_thread_metadata_by_event(test_scope, &[10; 32])
            .await;
        let _ = writer_db.get_thread_summary(test_scope, &[16; 32]).await;
        let _ = writer_db
            .get_channel_for_event_write(test_scope, Uuid::new_v4())
            .await;
        let _ = writer_db
            .get_members_for_event_write(test_scope, Uuid::new_v4())
            .await;
        let _ = writer_db
            .get_users_bulk_for_event_write(test_scope, &[vec![11; 32]])
            .await;
        let _ = writer_db
            .huddle_started_link_exists_for_event_write(
                test_scope,
                Uuid::new_v4(),
                Uuid::new_v4(),
                &[12; 32],
            )
            .await;
        let _ = writer_db
            .huddle_started_link_exists(test_scope, Uuid::new_v4(), Uuid::new_v4(), &[13; 32])
            .await;
        let _ = writer_db.list_archived(test_scope).await;
        let _ = writer_db
            .query_events_routed("pool_operation_matrix_writer", &query)
            .await;
        let write_tx = writer_db
            .begin_event_write_transaction()
            .await
            .expect("event-write semantic entry point begins a real transaction");
        write_tx
            .rollback()
            .await
            .expect("rollback operation-label fixture");
        let _ = writer_db
            .is_community_active_for_maintenance(test_scope)
            .await;
        let _ = writer_db.usage_community_count().await;
        let _ = writer_db.reap_expired_ephemeral_channels().await;
        let deletion_store = writer_db.deletion_store();
        let _ = deletion_store.reap_expired_serving_write_leases(1).await;
        let _ = deletion_store.serving_lease_stats().await;
        let _ = routed_db.is_relay_member(test_scope, &"a".repeat(64)).await;
        let _ = routed_db
            .query_events_routed("pool_operation_matrix_reader", &query)
            .await;
        routed_db.refresh_pool_waiter_metrics();

        let snapshot = snapshotter.snapshot().into_vec();
        let attempt_labels = snapshot
            .iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_db_pool_acquire_attempts_total" {
                    return None;
                }
                let DebugValue::Counter(value) = value else {
                    panic!("acquisition attempts must be counters");
                };
                if *value == 0 {
                    return None;
                }
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(labels.get("outcome").map(String::as_str), Some("success"));
                Some((labels["pool_role"].clone(), labels["operation"].clone()))
            })
            .collect::<BTreeSet<_>>();
        let expected = super::POOL_ACQUIRE_VALID_PAIRS
            .into_iter()
            .map(|(pool_role, operation)| (pool_role.to_owned(), operation.to_owned()))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            attempt_labels, expected,
            "real production Db/store methods must emit every exact valid operation pair"
        );

        let duration_labels = snapshot
            .iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_db_pool_acquire_duration_seconds" {
                    return None;
                }
                let DebugValue::Histogram(samples) = value else {
                    panic!("acquisition duration must be a histogram");
                };
                assert!(!samples.is_empty());
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key(), label.value()))
                    .collect::<BTreeMap<_, _>>();
                assert!(!labels.contains_key("outcome"));
                Some((
                    labels["pool_role"].to_owned(),
                    labels["operation"].to_owned(),
                ))
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(duration_labels, expected);

        let waiter_labels = snapshot
            .iter()
            .filter_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_db_pool_waiters" {
                    return None;
                }
                let DebugValue::Gauge(value) = value else {
                    panic!("pool waiters must be gauges");
                };
                assert_eq!(value.into_inner(), 0.0);
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key(), label.value()))
                    .collect::<BTreeMap<_, _>>();
                Some((
                    labels["pool_role"].to_owned(),
                    labels["operation"].to_owned(),
                ))
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(waiter_labels, expected);
    }

    async fn serving_write_gate_records_cancel_timeout_success_and_recovery() {
        let _test_guard = super::POOL_METRICS_TEST_LOCK.lock().await;
        let database_url = crate::test_support::database_url();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            // The same budget also covers the pool's initial physical
            // connection. Keep enough headroom for a cold CI database; the
            // held size-one connection below still deterministically drives
            // the checkout timeout terminal.
            .acquire_timeout(Duration::from_secs(1))
            .connect(&database_url)
            .await
            .expect("connect size-one serving-write test pool");
        let db = crate::Db::from_pool(pool.clone());
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            db.migrate().await.expect("migrate serving-write test DB");
        }
        let test_scope = db
            .ensure_configured_community(&format!(
                "pool-observability-{}.example",
                uuid::Uuid::new_v4().simple()
            ))
            .await
            .expect("create serving-write test community")
            .id;
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let held = pool.acquire().await.expect("hold sole writer connection");

        let store = db.deletion_store();
        let mut cancelled = Box::pin(store.is_serving_active(test_scope));
        tokio::select! {
            result = &mut cancelled => panic!("blocked serving-write gate unexpectedly completed: {result:?}"),
            () = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
        assert_eq!(
            waiter_value(&snapshotter.snapshot().into_vec(), "writer", "event_write"),
            Some(1.0)
        );
        drop(cancelled);

        let timeout = store
            .is_serving_active(test_scope)
            .await
            .expect_err("saturated serving-write gate times out");
        assert!(matches!(
            timeout,
            crate::DbError::Sqlx(sqlx::Error::PoolTimedOut)
        ));
        drop(held);

        assert!(store
            .is_serving_active(test_scope)
            .await
            .expect("serving-write gate recovers after release"));
        let lease = store
            .acquire_serving_write_lease(
                test_scope,
                "pool_observability",
                "pool-observability-test",
                Duration::from_secs(5),
            )
            .await
            .expect("serving-write lease acquires through event-write seam");
        assert!(store
            .release_serving_write_lease(&lease)
            .await
            .expect("serving-write lease release"));
        refresh_pool_waiters(false);

        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(waiter_value(&snapshot, "writer", "event_write"), Some(0.0));
        assert_eq!(attempt_count(&snapshot, "event_write", "cancelled"), 1);
        assert_eq!(attempt_count(&snapshot, "event_write", "timeout"), 1);
        assert!(
            attempt_count(&snapshot, "event_write", "success") >= 3,
            "gate recovery plus lease acquire/release must emit successes"
        );
    }

    fn attempt_count(
        snapshot: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            DebugValue,
        )],
        operation: &str,
        outcome: &str,
    ) -> u64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                let labels = key.key().labels().collect::<Vec<_>>();
                (key.key().name() == "buzz_db_pool_acquire_attempts_total"
                    && labels
                        .iter()
                        .any(|label| label.key() == "operation" && label.value() == operation)
                    && labels
                        .iter()
                        .any(|label| label.key() == "outcome" && label.value() == outcome))
                .then(|| match value {
                    DebugValue::Counter(value) => *value,
                    _ => panic!("pool attempts must be a counter"),
                })
            })
            .unwrap_or(0)
    }

    async fn advisory_lock_records_success_contention_timeout_and_error() {
        let database_url = crate::test_support::database_url();
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

    mod postgres_tests {
        #[tokio::test(flavor = "current_thread")]
        #[ignore = "requires Postgres"]
        async fn pool_acquire_records_success_timeout_and_error_with_wait_time() {
            super::pool_acquire_records_success_timeout_and_error_with_wait_time().await;
        }

        #[tokio::test(flavor = "current_thread")]
        #[ignore = "requires Postgres"]
        async fn production_db_methods_emit_exact_pool_operation_labels() {
            super::production_db_methods_emit_exact_pool_operation_labels().await;
        }

        #[tokio::test(flavor = "current_thread")]
        #[ignore = "requires Postgres"]
        async fn deletion_catalog_readiness_records_timeout_and_recovers() {
            super::deletion_catalog_readiness_records_timeout_and_recovers().await;
        }

        #[tokio::test(flavor = "current_thread")]
        #[ignore = "requires Postgres"]
        async fn serving_write_gate_records_cancel_timeout_success_and_recovery() {
            super::serving_write_gate_records_cancel_timeout_success_and_recovery().await;
        }

        #[tokio::test(flavor = "current_thread")]
        #[ignore = "requires Postgres"]
        async fn advisory_lock_records_success_contention_timeout_and_error() {
            super::advisory_lock_records_success_contention_timeout_and_error().await;
        }
    }
}
