//! Replica freshness fence for keyset-cursor read routing.
//!
//! A read replica may serve a cursor page only when every row the page could
//! contain is provably present on the replica. The proof rests on two parts:
//!
//! 1. **Commit-time floor** (migration 0021): a deferred constraint trigger
//!    aborts, at COMMIT, any transaction inserting a channel-bearing `events`
//!    row with `created_at` more than `floor` seconds before commit time
//!    (`clock_timestamp()`, evaluated inside commit processing). Enforcement
//!    is armed per session via the `buzz.created_at_floor` GUC, which the
//!    relay's writer pool sets on every connection.
//! 2. **Ordered LSN handshake** (this module): on one pinned writer
//!    connection, three separately-awaited statements sample
//!    `S = clock_timestamp()`, then scan `pg_stat_activity` for the oldest
//!    open transaction, then capture `L = pg_current_wal_lsn()` **last**.
//!    Once the replica reports `pg_last_wal_replay_lsn() >= L`, every
//!    transaction partitions into exactly three buckets:
//!      (a) finished before the activity scan — its commit WAL precedes `L`,
//!          so the replica has replayed it;
//!      (b) open at the activity scan — represented by `xact_start`, so it is
//!          bounded by the `oldest_xact_start` term;
//!      (c) started after the activity scan — its deferred floor guard runs
//!          after `S`, so it cannot commit a row with
//!          `created_at < S - floor`.
//!    There is no fourth bucket. The fence therefore advances to
//!    `min(oldest_xact_start, S) - floor - clock_margin`, and every
//!    channel-window row with `created_at <= fence` is on the replica.
//!
//! Everything fails **closed**: probe errors, masked `pg_stat_activity`
//! visibility, NULL/absent replica LSN (Aurora observability differences),
//! non-advancing replay, or probe staleness all close the fence, which routes
//! all reads back to the writer — degraded capacity, never holes.
//!
//! Operational bypasses (sessions without the GUC, `session_replication_role
//! = replica` restores) are outside the proof by design and require holding
//! the fence closed for their duration; see `migrations/0021`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

/// Seconds of `created_at` history the commit-time floor guard tolerates.
///
/// Must exceed the relay's ingest envelope (±900 s) by enough slack that a
/// legitimately accepted event still commits within the floor even under
/// slow validation/lock waits. The writer pool arms the guard with this value
/// and the fence subtracts it; the two uses must never diverge.
pub const CREATED_AT_FLOOR_SECS: i64 = 960;

/// Safety margin subtracted from the fence on top of the floor.
///
/// All proof timestamps (`clock_timestamp()`, `xact_start`, the guard's
/// clock) come from the writer host, so this only needs to absorb
/// `created_at` second-truncation and scheduling noise, not clock skew
/// between machines.
pub const FENCE_CLOCK_MARGIN_SECS: i64 = 5;

/// How often the probe samples the writer and checks the replica.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(5);

/// A fence older than this is stale: the probe has stopped confirming
/// freshness and the fence closes until a new handshake completes.
pub const FENCE_STALENESS: Duration = Duration::from_secs(30);

/// Sentinel: fence closed (no verified replica coverage).
const CLOSED: i64 = i64::MIN;

/// Shared fence state. `Db` holds an `Arc` of this; the probe task advances
/// it and cursor routing consults it.
#[derive(Debug)]
pub struct ReplicaFence {
    /// Unix micros of the newest verified-complete timestamp, or `CLOSED`.
    fence_micros: AtomicI64,
    /// Unix micros when the fence was last advanced (staleness check).
    updated_micros: AtomicI64,
}

impl ReplicaFence {
    /// A new fence, initially closed.
    pub fn new() -> Self {
        Self {
            fence_micros: AtomicI64::new(CLOSED),
            updated_micros: AtomicI64::new(CLOSED),
        }
    }

    /// Close the fence: all cursor reads route to the writer.
    pub fn close(&self) {
        self.fence_micros.store(CLOSED, Ordering::Relaxed);
    }

    fn advance(&self, fence: DateTime<Utc>) {
        self.fence_micros
            .store(fence.timestamp_micros(), Ordering::Relaxed);
        self.updated_micros
            .store(Utc::now().timestamp_micros(), Ordering::Relaxed);
    }

    /// The current fence, or `None` when closed or stale.
    ///
    /// Rows with `created_at <= fence` are verified present on the replica.
    pub fn verified_through(&self) -> Option<DateTime<Utc>> {
        let raw = self.fence_micros.load(Ordering::Relaxed);
        if raw == CLOSED {
            return None;
        }
        let updated = self.updated_micros.load(Ordering::Relaxed);
        let age_micros = Utc::now().timestamp_micros().saturating_sub(updated);
        if age_micros > FENCE_STALENESS.as_micros() as i64 {
            return None;
        }
        DateTime::from_timestamp_micros(raw)
    }

    /// Whether the replica verifiably holds every channel-window row at or
    /// before `ts`.
    pub fn covers(&self, ts: DateTime<Utc>) -> bool {
        self.verified_through().is_some_and(|fence| ts <= fence)
    }

    /// Test hook: force the fence open through `ts` without a probe.
    /// Used by routing tests that stand up a divergent fake replica.
    pub fn force_open_for_tests(&self, ts: DateTime<Utc>) {
        self.advance(ts);
    }
}

impl Default for ReplicaFence {
    fn default() -> Self {
        Self::new()
    }
}

/// One writer-side sample of the ordered handshake.
#[derive(Debug)]
struct WriterSample {
    /// `S`: writer `clock_timestamp()` captured first.
    sampled_at: DateTime<Utc>,
    /// Oldest open transaction among other client backends at scan time,
    /// or `None` when no transaction was open.
    oldest_xact_start: Option<DateTime<Utc>>,
    /// `L`: writer `pg_current_wal_lsn()` captured last, as text.
    wal_lsn: String,
}

/// Errors that close the fence. All variants are logged and treated
/// identically: fail closed.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// A probe query against writer or replica failed.
    #[error("writer probe query failed: {0}")]
    Writer(#[from] sqlx::Error),
    /// `pg_stat_activity` hid state for another backend that could hold an
    /// open transaction — the oldest-xact term cannot be trusted.
    #[error(
        "pg_stat_activity visibility incomplete: {masked} other client backend(s) with masked or \
         unrecognized state — probe role needs pg_monitor"
    )]
    MaskedActivity {
        /// Number of other client backends with masked/unknown state.
        masked: i64,
    },
    /// The replica returned NULL for the replay-LSN comparison.
    #[error("replica did not report a comparable replay LSN")]
    ReplicaLsnUnavailable,
}

/// Take one ordered writer sample: S, then activity scan, then L **last**.
///
/// The three statements are separately awaited on a single pinned connection;
/// a single SELECT would not guarantee evaluation order across the
/// subexpressions, reopening the race this ordering exists to close.
async fn sample_writer(writer: &PgPool) -> Result<WriterSample, ProbeError> {
    let mut conn = writer.acquire().await?;

    // 1. S first.
    let sampled_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *conn)
        .await?;

    // 2. Activity scan. Classification (fail closed on anything unknown):
    //    - `backend_type IS NULL` → masked. CRITICAL: an unprivileged view
    //      masks `backend_type` itself along with `state`/`xact_start`
    //      (verified on PG 16/17), so filtering on
    //      `backend_type = 'client backend'` would silently EXCLUDE masked
    //      rows and fail open. Masked rows must be detected before any
    //      backend-type filter;
    //    - client backend with `state IS NULL`, or a transactional/unknown
    //      state with NULL `xact_start` → masked → error;
    //    - client backend, `state = 'idle'`, NULL `xact_start`
    //                                 → no transaction → safely ignore;
    //    - any row with non-NULL `xact_start` (any backend type)
    //                                 → include in the minimum. Background
    //      workers cannot insert events rows, but counting them is the
    //      conservative direction (a long autovacuum merely holds the fence
    //      back), and it keeps the classification simple.
    //    Scope is every other backend regardless of role or application
    //    name: an admin psql transaction writes under the same trigger and
    //    must be representable in the oldest-xact term.
    //
    //    Prepared transactions (2PC) are a bucket of their own: while
    //    prepared they have left `pg_stat_activity` but can still commit
    //    after `L`. Their deferred floor guard already ran at PREPARE, so
    //    `pg_prepared_xacts.prepared` bounds their rows exactly like
    //    `xact_start`; fold it into the same minimum.
    let row = sqlx::query(
        r#"
        SELECT
            least(
                (SELECT min(xact_start)
                   FROM pg_stat_activity
                  WHERE pid <> pg_backend_pid()),
                (SELECT min(prepared) FROM pg_prepared_xacts)
            ) AS oldest_xact_start,
            (SELECT count(*)
               FROM pg_stat_activity
              WHERE pid <> pg_backend_pid()
                AND (backend_type IS NULL
                     OR (backend_type = 'client backend'
                         AND (state IS NULL
                              OR (state <> 'idle' AND xact_start IS NULL))))
            ) AS masked
        "#,
    )
    .fetch_one(&mut *conn)
    .await?;
    let masked: i64 = row.get("masked");
    if masked > 0 {
        return Err(ProbeError::MaskedActivity { masked });
    }
    let oldest_xact_start: Option<DateTime<Utc>> = row.get("oldest_xact_start");

    // 3. L last.
    let wal_lsn: String = sqlx::query_scalar("SELECT pg_current_wal_lsn()::text")
        .fetch_one(&mut *conn)
        .await?;

    Ok(WriterSample {
        sampled_at,
        oldest_xact_start,
        wal_lsn,
    })
}

/// Whether the replica has replayed at least through `wal_lsn`.
///
/// The comparison happens on the replica in pg_lsn domain. The
/// `pg_is_in_recovery()` gate is load-bearing: after crash recovery or
/// promotion a *primary* returns a static non-NULL `pg_last_wal_replay_lsn()`
/// rather than NULL, so NULL-checking alone would not reliably detect a
/// misrouted "replica" URL. Not-in-recovery, NULL replay LSN, or Aurora
/// hiding either is an error → fence closes.
async fn replica_covers(replica: &PgPool, wal_lsn: &str) -> Result<bool, ProbeError> {
    let covered: Option<bool> = sqlx::query_scalar(
        r#"
        SELECT CASE
            WHEN pg_is_in_recovery() THEN pg_last_wal_replay_lsn() >= $1::pg_lsn
            ELSE NULL
        END
        "#,
    )
    .bind(wal_lsn)
    .fetch_one(replica)
    .await?;
    covered.ok_or(ProbeError::ReplicaLsnUnavailable)
}

/// Run one full handshake and, on success, advance the fence.
///
/// Returns the new fence value for observability. `Ok(None)` means the
/// replica has not yet replayed past the sample; the fence is left as-is
/// (staleness will close it if this persists).
pub async fn probe_once(
    writer: &PgPool,
    replica: &PgPool,
    fence: &ReplicaFence,
) -> Result<Option<DateTime<Utc>>, ProbeError> {
    let sample = sample_writer(writer).await?;
    if !replica_covers(replica, &sample.wal_lsn).await? {
        return Ok(None);
    }
    let lower = match sample.oldest_xact_start {
        Some(oldest) => oldest.min(sample.sampled_at),
        None => sample.sampled_at,
    };
    let new_fence = lower
        - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS)
        - chrono::Duration::seconds(FENCE_CLOCK_MARGIN_SECS);
    fence.advance(new_fence);
    Ok(Some(new_fence))
}

/// Background probe loop: sample every `PROBE_INTERVAL`, close the fence on
/// any error. Runs for the life of the process.
pub async fn run_probe(writer: PgPool, replica: PgPool, fence: Arc<ReplicaFence>) {
    let mut interval = tokio::time::interval(PROBE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match probe_once(&writer, &replica, &fence).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                // Replica behind the sample: leave the fence; staleness
                // closes it if the replica stays behind.
                tracing::debug!("replica fence: replay behind writer sample");
            }
            Err(e) => {
                fence.close();
                tracing::warn!(error = %e, "replica fence probe failed; fence closed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    fn test_db_url() -> String {
        std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into())
    }

    #[test]
    fn fence_starts_closed_and_opens_on_advance() {
        let fence = ReplicaFence::new();
        assert!(fence.verified_through().is_none(), "must start closed");
        assert!(!fence.covers(Utc::now() - chrono::Duration::days(365)));

        let ts = Utc::now();
        fence.advance(ts);
        assert_eq!(fence.verified_through(), Some(ts));
        assert!(fence.covers(ts - chrono::Duration::seconds(1)));
        assert!(fence.covers(ts), "boundary is inclusive");
        assert!(!fence.covers(ts + chrono::Duration::seconds(1)));

        fence.close();
        assert!(fence.verified_through().is_none(), "close() must close");
        assert!(!fence.covers(ts - chrono::Duration::days(365)));
    }

    #[test]
    fn stale_fence_reads_as_closed() {
        let fence = ReplicaFence::new();
        let ts = Utc::now();
        fence
            .fence_micros
            .store(ts.timestamp_micros(), Ordering::Relaxed);
        // Last update older than the staleness budget.
        let stale = (Utc::now()
            - chrono::Duration::from_std(FENCE_STALENESS).expect("duration")
            - chrono::Duration::seconds(1))
        .timestamp_micros();
        fence.updated_micros.store(stale, Ordering::Relaxed);
        assert!(
            fence.verified_through().is_none(),
            "a fence the probe stopped confirming must read as closed"
        );
    }

    /// The activity scan must (a) represent another session's open
    /// transaction in the oldest-xact term and (b) ignore plain-idle
    /// sessions, per the agreed classification.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn sample_writer_sees_open_transactions_and_ignores_idle() {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");

        // A plain idle session: pinned connection, no transaction.
        let idle_pool = PgPool::connect(&test_db_url()).await.expect("connect idle");
        let _idle_conn = idle_pool.acquire().await.expect("idle conn");

        let before = sample_writer(&pool).await.expect("sample without tx");

        // Now hold a transaction open on a second connection.
        let tx_pool = PgPool::connect(&test_db_url()).await.expect("connect tx");
        let mut tx = tx_pool.begin().await.expect("begin");
        sqlx::query("SELECT 1")
            .execute(&mut *tx)
            .await
            .expect("touch tx");

        let during = sample_writer(&pool).await.expect("sample with open tx");
        let oldest = during
            .oldest_xact_start
            .expect("open transaction must appear in the oldest-xact term");
        assert!(
            oldest <= during.sampled_at,
            "xact_start precedes the sample that observed it"
        );
        // S is captured before the activity scan, L after: the sample's
        // ordering invariant.
        assert!(during.sampled_at >= before.sampled_at);

        tx.rollback().await.expect("rollback");
    }

    /// An unprivileged probe role sees NULL `state`/`xact_start` for other
    /// sessions' rows in `pg_stat_activity`. The oldest-xact term is then
    /// untrustworthy and the sample must fail closed (`MaskedActivity`) —
    /// never silently `MIN()` the hidden row away.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn sample_writer_fails_closed_when_activity_is_masked() {
        let admin = PgPool::connect(&test_db_url()).await.expect("connect");

        // Hold a transaction open as the privileged user: this is the row
        // the unprivileged probe must notice it cannot classify.
        let tx_pool = PgPool::connect(&test_db_url()).await.expect("connect tx");
        let mut tx = tx_pool.begin().await.expect("begin");
        sqlx::query("SELECT 1")
            .execute(&mut *tx)
            .await
            .expect("touch tx");

        // An unprivileged login role (no pg_monitor): pg_stat_activity masks
        // other sessions' state columns for it.
        let role = format!("fence_probe_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN PASSWORD 'fence_probe_test'" // sadscan:disable np.postgres.1
        )))
        .execute(&admin)
        .await
        .expect("create unprivileged role");

        let base = test_db_url();
        let unpriv_url = {
            let rest = base.strip_prefix("postgres://").expect("pg url");
            let at = rest.rfind('@').expect("credentials in url");
            format!("postgres://{role}:fence_probe_test@{}", &rest[at + 1..])
        };
        let unpriv = PgPool::connect(&unpriv_url).await.expect("connect unpriv");

        let err = sample_writer(&unpriv)
            .await
            .expect_err("masked pg_stat_activity must fail closed");
        assert!(
            matches!(err, ProbeError::MaskedActivity { masked } if masked >= 1),
            "expected MaskedActivity, got {err:?}"
        );

        tx.rollback().await.expect("rollback");
        unpriv.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {role}")))
            .execute(&admin)
            .await
            .expect("drop role");
    }

    /// A primary (non-replica) database returns NULL from
    /// `pg_last_wal_replay_lsn()`; the probe must fail closed, never
    /// synthesize freshness. This is also the Aurora-observability guard:
    /// if the reader endpoint hides replay LSNs, routing stays writer-only.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn probe_fails_closed_when_replica_lsn_unavailable() {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        let fence = ReplicaFence::new();
        fence.advance(Utc::now()); // pretend a previous handshake succeeded

        // Using the primary as its own "replica": replay LSN is NULL.
        let err = probe_once(&pool, &pool, &fence)
            .await
            .expect_err("NULL replay LSN must be an error");
        assert!(matches!(err, ProbeError::ReplicaLsnUnavailable));
    }
}
