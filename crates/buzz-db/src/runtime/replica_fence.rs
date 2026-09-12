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
//! 2. **Ordered heartbeat handshake** (this module): on one pinned writer
//!    connection, separately-awaited statements sample
//!    `S = clock_timestamp()`, then scan `pg_stat_activity` for the oldest
//!    open transaction, then — **last** — commit heartbeat token `M` via a
//!    single-row `UPDATE replica_heartbeat ... RETURNING token, epoch`
//!    (migration 0026). Because the single-row UPDATE serializes all pods'
//!    probes, tokens are globally commit-ordered. A reader **session** that
//!    observes `token >= M` on its own connection has, by WAL/storage replay
//!    order, also replayed every commit that preceded M's commit; every
//!    transaction then partitions into exactly three buckets:
//!    (a) finished before the activity scan — its commit precedes `M`'s
//!    commit, so the replica session has replayed it;
//!    (b) open at the activity scan — represented by `xact_start`, so it is
//!    bounded by the `oldest_xact_start` term;
//!    (c) started after the activity scan — its deferred floor guard runs
//!    after `S`, so it cannot commit a row with
//!    `created_at < S - floor`.
//!    There is no fourth bucket. Each committed token `M` therefore proves a
//!    **fence wall** of `min(oldest_xact_start, S) - floor - clock_margin`:
//!    every channel-window row with `created_at <= fence_wall(M)` is present
//!    on any reader session observing `token >= M`.
//!
//! Unlike the previous WAL-LSN observation (`pg_last_wal_replay_lsn()`, which
//! Aurora reader endpoints hide), the token observation is portable and —
//! critically — **snapshot-local**: routing opens a `REPEATABLE READ, READ
//! ONLY` transaction on the reader session that will serve the page and
//! observes the heartbeat as its first statement, so the proof binds to the
//! exact snapshot every follow-up statement in the request (page,
//! participants, aux closure) reads from — never to a different pooled
//! session (readers behind one endpoint may sit at different replay
//! positions), and never to a later autocommit snapshot on the same wire.
//! An observed token lower than the newest retained `M` is ordinary
//! replication lag, not a fault; the resolver simply proves from an older
//! retained `M`. Regression detection is writer-side only: a non-monotonic
//! `RETURNING token` or an epoch change (restore/re-seed) clears the retained
//! ring, so no stale entry can masquerade as fresh coverage.
//!
//! Everything fails **closed**: probe errors, masked `pg_stat_activity`
//! visibility, an unreadable heartbeat row on the reader session, an epoch
//! mismatch, or an observed token below every retained entry all route the
//! request back to the writer — degraded capacity, never holes.
//!
//! Operational bypasses (sessions without the GUC, `session_replication_role
//! = replica` restores) are outside the proof by design and require holding
//! the fence closed for their duration; see `migrations/0021`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use buzz_datastore_tracing::datastore_span;

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

/// How often the probe samples the writer and commits a heartbeat token.
///
/// 500ms keeps the cadence at least 2x under the smallest sensible bounded
/// budget (`BUZZ_REPLICA_READ_MAX_AGE_MS`, deploy plan 1000ms) so
/// eligibility doesn't flap between beats. Cost is one single-row UPDATE
/// tuple of WAL per beat per pod — ~20 beats/s fleet-wide, <0.1% of the
/// writer.
pub const PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// A fence whose newest entry is older than this is stale: the probe has
/// stopped committing tokens and routing eligibility closes until a new
/// handshake completes.
///
/// Note this is an availability hygiene gate, not a soundness requirement:
/// a retained entry's proof (`token >= M` on a session implies every row
/// `<= fence_wall(M)` is present there) never decays. Closing on staleness
/// just stops spending reader checkouts once the probe is evidently dead.
pub const FENCE_STALENESS: Duration = Duration::from_secs(30);

/// How many `(token, fence_wall)` entries the fence retains. At one probe
/// per [`PROBE_INTERVAL`] (500ms) this is ~60 seconds of history — a reader
/// session lagging further than that behind the newest token fails closed
/// (routes to the writer) rather than proving from thin air. Aurora reader
/// lag is typically tens of milliseconds; a reader minutes behind is a
/// fault, not a routing candidate.
const RING_CAPACITY: usize = 120;

// The retained window must outlast the staleness gate: if the ring held
// less than FENCE_STALENESS of history, a non-stale newest entry could
// coexist with proved-but-evicted older entries, failing sessions closed
// for capacity rather than lag. Compile-checked so a future cadence or
// capacity tweak can't silently shrink the window below the gate.
const _: () = assert!(
    RING_CAPACITY as u64 * PROBE_INTERVAL.as_millis() as u64 > FENCE_STALENESS.as_millis() as u64,
    "fence ring must retain more history than the staleness gate"
);

/// One retained heartbeat observation: proof material for reader sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEntry {
    /// The committed heartbeat token `M`.
    pub token: i64,
    /// Monotonic instant captured just before `M` was committed. `elapsed()`
    /// bounds (from above) how old a session observing `token >= M` can be —
    /// the freshness term of the head-routing predicate.
    pub committed_at: Instant,
    /// `min(oldest_xact_start, S) - floor - clock_margin` for `M`'s
    /// handshake: every channel-window row with `created_at <= fence_wall`
    /// is present on any session observing `token >= M`.
    pub fence_wall: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct FenceInner {
    /// Epoch the retained ring belongs to. `None` until the first probe —
    /// or after the test hook, whose injected entry deliberately bypasses
    /// the epoch comparison in [`ReplicaFence::resolve`].
    epoch: Option<Uuid>,
    /// Retained entries in strictly increasing token order.
    ring: VecDeque<TokenEntry>,
}

/// Outcome of recording one probe sample.
#[derive(Debug, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Entry retained; proofs may cite it.
    Recorded,
    /// The token went backwards within the same epoch — a restore that kept
    /// the old epoch. The ring was cleared and the entry discarded; the
    /// probe must rotate the epoch before recording again (a reader still on
    /// the pre-rewind timeline could otherwise observe a *higher* token that
    /// proves nothing about the new timeline).
    TokenRegression,
}

/// Outcome of resolving one reader-session observation against the ring.
/// Everything but [`ResolveOutcome::Proved`] fails closed (routes to the
/// writer); the variants exist so route metrics can name the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The observation proves this retained entry.
    Proved(TokenEntry),
    /// The observed epoch is not the ring's epoch — the session is on a
    /// different timeline (restore) or the ring was rotated under it.
    EpochMismatch,
    /// The observed token is below every retained entry: the reader lags
    /// further than the ring's history (or the ring is empty).
    TokenBehind,
}

impl ResolveOutcome {
    /// The proved entry, if any.
    pub fn proved(self) -> Option<TokenEntry> {
        match self {
            ResolveOutcome::Proved(entry) => Some(entry),
            _ => None,
        }
    }
}

/// Shared fence state. `Db` holds an `Arc` of this; the probe task records
/// entries and per-request routing resolves proofs against it.
#[derive(Debug, Default)]
pub struct ReplicaFence {
    inner: Mutex<FenceInner>,
}

impl ReplicaFence {
    /// A new fence, initially closed (empty ring).
    pub fn new() -> Self {
        Self::default()
    }

    /// Close the fence: drop all retained proofs; reads route to the writer.
    pub fn close(&self) {
        let mut inner = self.inner.lock().expect("fence lock poisoned");
        inner.ring.clear();
    }

    /// Record one probe sample. Epoch changes (re-seed) clear the ring and
    /// start a new one under the observed epoch — sound, because an entry
    /// only proves commits on its own timeline and readers must match the
    /// epoch to cite it. A same-epoch token regression is the unsafe case:
    /// see [`RecordOutcome::TokenRegression`].
    pub fn record(
        &self,
        token: i64,
        epoch: Uuid,
        committed_at: Instant,
        fence_wall: DateTime<Utc>,
    ) -> RecordOutcome {
        let mut inner = self.inner.lock().expect("fence lock poisoned");
        if inner.epoch != Some(epoch) {
            inner.ring.clear();
            inner.epoch = Some(epoch);
        } else if inner.ring.back().is_some_and(|last| token <= last.token) {
            inner.ring.clear();
            return RecordOutcome::TokenRegression;
        }
        if inner.ring.len() == RING_CAPACITY {
            inner.ring.pop_front();
        }
        inner.ring.push_back(TokenEntry {
            token,
            committed_at,
            fence_wall,
        });
        RecordOutcome::Recorded
    }

    /// Resolve the strongest proof a reader session's observation supports:
    /// the greatest retained entry with `entry.token <= observed_token`,
    /// provided the observed epoch matches the ring's. Non-`Proved` outcomes
    /// fail closed; they are distinguished only for route metrics.
    pub fn resolve(&self, observed_token: i64, observed_epoch: Uuid) -> ResolveOutcome {
        let inner = self.inner.lock().expect("fence lock poisoned");
        match inner.epoch {
            Some(e) if e != observed_epoch => return ResolveOutcome::EpochMismatch,
            // `None` with a non-empty ring only happens via the test hook;
            // the epoch comparison is deliberately skipped there.
            _ => {}
        }
        inner
            .ring
            .iter()
            .rev()
            .find(|entry| entry.token <= observed_token)
            .copied()
            .map_or(ResolveOutcome::TokenBehind, ResolveOutcome::Proved)
    }

    /// The newest retained entry, staleness-gated. Used as the cheap
    /// pre-check before spending a reader checkout, and for observability.
    pub fn newest(&self) -> Option<TokenEntry> {
        let inner = self.inner.lock().expect("fence lock poisoned");
        inner
            .ring
            .back()
            .filter(|entry| entry.committed_at.elapsed() <= FENCE_STALENESS)
            .copied()
    }

    /// Age of the newest retained entry, ungated (observability: how long
    /// since the probe last committed a token).
    pub fn heartbeat_age(&self) -> Option<Duration> {
        let inner = self.inner.lock().expect("fence lock poisoned");
        inner.ring.back().map(|entry| entry.committed_at.elapsed())
    }

    /// The newest fence wall, or `None` when closed or stale.
    ///
    /// Rows with `created_at <= fence` are verified present on a reader
    /// session that proves the newest entry; whether a *given* session does
    /// is decided per request via [`ReplicaFence::resolve`].
    pub fn verified_through(&self) -> Option<DateTime<Utc>> {
        self.newest().map(|entry| entry.fence_wall)
    }

    /// Whether some retained entry's wall covers `ts` — the cheap routing
    /// pre-check (the connection-local observation still has to prove it).
    pub fn covers(&self, ts: DateTime<Utc>) -> bool {
        self.verified_through().is_some_and(|fence| ts <= fence)
    }

    /// Test hook: force the fence open through `ts` without a probe.
    /// Injects an entry any observed token satisfies (`i64::MIN`) with no
    /// epoch recorded, so the epoch comparison is bypassed — routing tests
    /// stand up a divergent fake replica whose heartbeat epoch differs from
    /// the writer's.
    pub fn force_open_for_tests(&self, ts: DateTime<Utc>) {
        self.force_open_for_tests_at(ts, Instant::now());
    }

    /// [`ReplicaFence::force_open_for_tests`] with an explicit commit
    /// instant, for pinning age-gated behavior (head-budget and staleness
    /// tests inject entries "committed" in the past).
    pub fn force_open_for_tests_at(&self, ts: DateTime<Utc>, committed_at: Instant) {
        let mut inner = self.inner.lock().expect("fence lock poisoned");
        inner.epoch = None;
        inner.ring.clear();
        inner.ring.push_back(TokenEntry {
            token: i64::MIN,
            committed_at,
            fence_wall: ts,
        });
    }
}

/// Catalog-level verification that the commit-time floor guard (migration
/// 0021) is present and correctly shaped on the `events` parent AND every
/// partition: right function, `DEFERRABLE INITIALLY DEFERRED`, row-level,
/// AFTER, firing on both INSERT and UPDATE (an UPDATE can move an exempt
/// channel-NULL row into the guarded set, or rewrite `created_at` downward).
///
/// This is a name-and-shape check only; it cannot detect a sabotaged
/// function body. [`verify_floor_guard_behavior`] proves the semantics.
///
/// Generic over the executor so the migration path can run it on the
/// lock-holding connection while the startup probe keeps using the pool.
#[datastore_span(name = "replica_fence_verify_catalog", system = "postgresql")]
pub async fn verify_floor_guard_catalog<'e>(
    executor: impl sqlx::PgExecutor<'e>,
) -> crate::Result<()> {
    // tgtype bits: 1 = ROW, 2 = BEFORE, 4 = INSERT, 16 = UPDATE, 64 = INSTEAD.
    // Required: ROW + INSERT + UPDATE set, BEFORE + INSTEAD clear.
    let missing: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname::text
        FROM (
            SELECT 'events'::regclass AS oid
            UNION ALL
            SELECT inhrelid FROM pg_inherits WHERE inhparent = 'events'::regclass
        ) rels
        JOIN pg_class c ON c.oid = rels.oid
        WHERE NOT EXISTS (
            SELECT 1 FROM pg_trigger t
            WHERE t.tgrelid = rels.oid
              AND t.tgname = 'events_created_at_floor'
              AND t.tgfoid = 'events_created_at_floor_guard'::regproc
              AND t.tgdeferrable
              AND t.tginitdeferred
              AND t.tgtype & 1 = 1      -- row-level
              AND t.tgtype & 2 = 0      -- AFTER, not BEFORE
              AND t.tgtype & 64 = 0     -- not INSTEAD OF
              AND t.tgtype & 4 = 4      -- fires on INSERT
              AND t.tgtype & 16 = 16    -- fires on UPDATE
        )
        "#,
    )
    .fetch_all(executor)
    .await?;
    if !missing.is_empty() {
        return Err(crate::error::DbError::InvalidData(format!(
            "created_at floor guard trigger missing or mis-shaped on: {} \
             (replica fence must stay closed)",
            missing.join(", ")
        )));
    }
    Ok(())
}

/// Behavioral verification of the floor guard, end-to-end through the armed
/// pool. A catalog check cannot detect a no-op function body or an unarmed
/// pool; this proves the semantics the fence proof cites, inside one
/// rolled-back transaction:
///
/// 1. the pool's session GUC equals [`CREATED_AT_FLOOR_SECS`] (arming);
/// 2. an old channel-bearing INSERT raises `check_violation` (23514);
/// 3. a fresh channel-bearing INSERT commits;
/// 4. rewriting a fresh row's `created_at` below the floor raises;
/// 5. an old channel-NULL INSERT is exempt, but flipping its `channel_id`
///    on raises (the `UPDATE OF` arm).
///
/// `SET CONSTRAINTS ALL IMMEDIATE` makes the deferred trigger fire per
/// statement so each adversary is observable under a savepoint; deferral to
/// COMMIT is separately pinned by the held-transaction fixture.
#[datastore_span(name = "replica_fence_verify_behavior", system = "postgresql")]
pub async fn verify_floor_guard_behavior(pool: &PgPool) -> crate::Result<()> {
    use crate::error::DbError;

    let expect_violation = |res: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
                            what: &str|
     -> crate::Result<()> {
        match res {
            Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23514") => Ok(()),
            Ok(_) => Err(DbError::InvalidData(format!(
                "floor guard is inert: {what} was accepted (replica fence must stay closed)"
            ))),
            Err(e) => Err(DbError::InvalidData(format!(
                "floor guard verification failed unexpectedly on {what}: {e}"
            ))),
        }
    };

    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Bootstrap,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // 1. Pool arming (Perci: assert the effective value, not the intent).
    let armed: String = sqlx::query_scalar("SHOW buzz.created_at_floor")
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            DbError::InvalidData(format!(
                "buzz.created_at_floor GUC not set on this pool: {e}"
            ))
        })?;
    if armed != CREATED_AT_FLOOR_SECS.to_string() {
        return Err(DbError::InvalidData(format!(
            "buzz.created_at_floor is '{armed}', expected '{CREATED_AT_FLOOR_SECS}': \
             pool is not armed"
        )));
    }

    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await?;

    // Scratch community satisfying the FK; the whole transaction rolls back.
    let community = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("fence-verify-{}.invalid", community.simple()))
        .execute(&mut *tx)
        .await?;
    let channel = uuid::Uuid::new_v4();

    let insert = |tx_id: [u8; 32], age_secs: i64, ch: Option<uuid::Uuid>| {
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, \
             content, sig, received_at, channel_id) \
             VALUES ($1, $2, $3, clock_timestamp() - make_interval(secs => $4::double precision), \
             9, '[]', 'fence-verify', $5, NOW(), $6)",
        )
        .bind(community)
        .bind(tx_id.to_vec())
        .bind(vec![0u8; 32])
        .bind(age_secs as f64)
        .bind(vec![0u8; 64])
        .bind(ch)
    };
    let old_age = CREATED_AT_FLOOR_SECS + 60;

    // 2. Old channel-bearing insert must raise.
    sqlx::query("SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;
    let res = insert(rand_id(), old_age, Some(channel))
        .execute(&mut *tx)
        .await;
    expect_violation(res, "an old channel-bearing INSERT")?;
    sqlx::query("ROLLBACK TO SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;

    // 3. Fresh channel-bearing insert must pass.
    let fresh_id = rand_id();
    insert(fresh_id, 0, Some(channel)).execute(&mut *tx).await?;

    // 4. Rewriting created_at below the floor must raise (in- or
    //    cross-partition, either arm of the guard catches the NEW row).
    sqlx::query("SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query(
        "UPDATE events SET created_at = clock_timestamp() - make_interval(secs => $1::double precision) \
         WHERE community_id = $2 AND id = $3",
    )
    .bind(old_age as f64)
    .bind(community)
    .bind(fresh_id.to_vec())
    .execute(&mut *tx)
    .await;
    expect_violation(res, "rewriting created_at below the floor")?;
    sqlx::query("ROLLBACK TO SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;

    // 5. Old channel-NULL insert is exempt; flipping channel_id on must raise.
    let null_id = rand_id();
    insert(null_id, old_age, None).execute(&mut *tx).await?;
    sqlx::query("SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query("UPDATE events SET channel_id = $1 WHERE community_id = $2 AND id = $3")
        .bind(channel)
        .bind(community)
        .bind(null_id.to_vec())
        .execute(&mut *tx)
        .await;
    expect_violation(res, "moving an old channel-NULL row into a channel")?;
    sqlx::query("ROLLBACK TO SAVEPOINT floor_probe")
        .execute(&mut *tx)
        .await?;

    tx.rollback().await?;
    Ok(())
}

fn rand_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    for chunk in id.chunks_mut(16) {
        chunk.copy_from_slice(&uuid::Uuid::new_v4().into_bytes()[..chunk.len()]);
    }
    id
}

/// One writer-side sample of the ordered handshake.
#[derive(Debug)]
struct WriterSample {
    /// `S`: writer `clock_timestamp()` captured first.
    sampled_at: DateTime<Utc>,
    /// Oldest open transaction among other client backends at scan time,
    /// or `None` when no transaction was open.
    oldest_xact_start: Option<DateTime<Utc>>,
    /// `M`: the heartbeat token committed **last**, after the scan.
    token: i64,
    /// Heartbeat epoch returned with `M`.
    epoch: Uuid,
    /// Monotonic instant captured immediately before committing `M` — an
    /// upper bound on how old a session observing `token >= M` can be.
    committed_at: Instant,
}

/// Errors that close the fence. All variants are logged and treated
/// identically: fail closed.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// A probe query against the writer failed.
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
    /// The single heartbeat row (migration 0026) is missing on the writer.
    #[error("replica_heartbeat row missing on the writer — migration 0026 not applied?")]
    HeartbeatRowMissing,
}

/// Take one ordered writer sample: S, then activity scan, then commit the
/// heartbeat token **last**.
///
/// The statements are separately awaited on a single pinned connection;
/// a single SELECT would not guarantee evaluation order across the
/// subexpressions, reopening the race this ordering exists to close.
async fn sample_writer(writer: &PgPool) -> Result<WriterSample, ProbeError> {
    let mut conn = crate::observability::acquire_writer(
        writer,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?;

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
    //    after the token. Their deferred floor guard already ran at PREPARE,
    //    so `pg_prepared_xacts.prepared` bounds their rows exactly like
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

    // 3. Token commit LAST, on the same pinned connection. The single-row
    //    UPDATE serializes concurrent pods' probes, so RETURNING token is
    //    globally commit-ordered. `committed_at` is captured before the
    //    round trip so `elapsed()` over-estimates the observation's age —
    //    the conservative direction for the head-freshness bound.
    let committed_at = Instant::now();
    let row = sqlx::query(
        "UPDATE replica_heartbeat SET token = token + 1 WHERE id = 1 RETURNING token, epoch",
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ProbeError::HeartbeatRowMissing)?;

    Ok(WriterSample {
        sampled_at,
        oldest_xact_start,
        token: row.get("token"),
        epoch: row.get("epoch"),
        committed_at,
    })
}

/// The fence wall proved by one handshake:
/// `min(oldest_xact_start, S) - floor - clock_margin`.
fn fence_wall(sample_s: DateTime<Utc>, oldest_xact_start: Option<DateTime<Utc>>) -> DateTime<Utc> {
    let lower = match oldest_xact_start {
        Some(oldest) => oldest.min(sample_s),
        None => sample_s,
    };
    lower
        - chrono::Duration::seconds(CREATED_AT_FLOOR_SECS)
        - chrono::Duration::seconds(FENCE_CLOCK_MARGIN_SECS)
}

/// Run one full handshake and record the resulting `(token, fence_wall)`.
///
/// On a same-epoch token regression (the writer was restored from a backup
/// that kept its epoch), the retained ring has already been cleared by
/// [`ReplicaFence::record`]; this additionally **rotates the epoch** on the
/// writer and records the rotated token, so a reader still serving the
/// pre-rewind timeline (whose old, higher token would otherwise satisfy
/// `token >= M`) fails the epoch check instead of proving stale coverage.
pub async fn probe_once(writer: &PgPool, fence: &ReplicaFence) -> Result<TokenEntry, ProbeError> {
    let sample = sample_writer(writer).await?;
    let wall = fence_wall(sample.sampled_at, sample.oldest_xact_start);
    match fence.record(sample.token, sample.epoch, sample.committed_at, wall) {
        RecordOutcome::Recorded => Ok(TokenEntry {
            token: sample.token,
            committed_at: sample.committed_at,
            fence_wall: wall,
        }),
        RecordOutcome::TokenRegression => {
            tracing::warn!(
                token = sample.token,
                "replica heartbeat token regressed within its epoch (restore?); rotating epoch"
            );
            // The rotation commit happens after this sample's activity scan,
            // so the same three-bucket argument (and the same wall) holds
            // for the rotated token.
            let committed_at = Instant::now();
            let mut connection = crate::observability::acquire_writer(
                writer,
                crate::observability::WriterOperation::Maintenance,
            )
            .await?;
            let row = sqlx::query(
                "UPDATE replica_heartbeat SET epoch = gen_random_uuid(), token = token + 1 \
                 WHERE id = 1 RETURNING token, epoch",
            )
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(ProbeError::HeartbeatRowMissing)?;
            let token: i64 = row.get("token");
            let epoch: Uuid = row.get("epoch");
            // A fresh epoch always clears and records; regression is
            // impossible against an empty ring.
            fence.record(token, epoch, committed_at, wall);
            Ok(TokenEntry {
                token,
                committed_at,
                fence_wall: wall,
            })
        }
    }
}

/// The Aurora **PostgreSQL** instance-identity function. Named once so the
/// capability probe and the observation query can never disagree — and
/// pinned by a unit test, because the MySQL-family spelling
/// (`aurora_server_id`) is a near-miss that would make the capability probe
/// cache a permanent `false` on real Aurora (42883) and silently strip the
/// instance id from canary evidence.
pub const AURORA_IDENTITY_FN: &str = "aurora_db_instance_identifier";

/// Whether this reader endpoint supports [`AURORA_IDENTITY_FN`] — probed
/// ONCE per process on a plain autocommit checkout, never inside a request
/// transaction (an undefined-function error would abort the transaction
/// and fail the proof). `Ok(false)` is the definitive "not Aurora" answer
/// (undefined_function, SQLSTATE 42883); transient errors surface as `Err`
/// so the caller can retry the probe on a later request instead of caching
/// a wrong answer.
#[datastore_span(
    name = "replica_fence_reader_supports_aurora_identity",
    system = "postgresql"
)]
pub async fn reader_supports_aurora_identity(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    match sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {AURORA_IDENTITY_FN}()"
    )))
    .fetch_one(&mut *conn)
    .await
    {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("42883") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Observe the heartbeat on a specific reader session — the
/// connection-local half of the proof. Returns the observed token/epoch
/// plus the backend identity of the session for route-decision evidence:
/// `addr:port pid=N` (`local` on unix sockets), prefixed with the Aurora
/// instance id when `aurora` is set (only pass `true` after
/// [`reader_supports_aurora_identity`] confirmed it — the function
/// reference fails at parse time on plain Postgres). `None` when the row
/// is missing there (migration not yet replayed): fail closed.
pub async fn observe_heartbeat(
    conn: &mut PgConnection,
    aurora: bool,
) -> Result<Option<HeartbeatObservation>, sqlx::Error> {
    const ADDR_PID: &str = "COALESCE(host(inet_server_addr()) || ':' || \
         inet_server_port()::text, 'local') || ' pid=' || pg_backend_pid()::text";
    let sql = if aurora {
        format!(
            "SELECT token, epoch, {AURORA_IDENTITY_FN}() || ' @ ' || {ADDR_PID} AS backend \
             FROM replica_heartbeat WHERE id = 1"
        )
    } else {
        format!(
            "SELECT token, epoch, {ADDR_PID} AS backend \
             FROM replica_heartbeat WHERE id = 1"
        )
    };
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(|r| HeartbeatObservation {
        token: r.get("token"),
        epoch: r.get("epoch"),
        backend: r.get("backend"),
    }))
}

/// One reader-session heartbeat observation (see [`observe_heartbeat`]).
#[derive(Debug, Clone)]
pub struct HeartbeatObservation {
    /// The token the session has replayed through.
    pub token: i64,
    /// The epoch the session observes — must match the ring's.
    pub epoch: Uuid,
    /// Backend identity of the observed session, so live evidence records
    /// which reader served both proof and page.
    pub backend: String,
}

/// Background probe loop: commit a heartbeat token every `PROBE_INTERVAL`;
/// close the fence on any error. Runs for the life of the process.
pub async fn run_probe(writer: PgPool, fence: Arc<ReplicaFence>) {
    let mut interval = tokio::time::interval(PROBE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match probe_once(&writer, &fence).await {
            Ok(_) => {}
            Err(e) => {
                fence.close();
                tracing::warn!(error = %e, "replica fence probe failed; fence closed");
            }
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    /// A private scratch database with migrations applied: the probe tests
    /// mutate the singleton heartbeat row (rewind/rotate), which must never
    /// race the shared dev database or each other.
    async fn scratch_db() -> (PgPool, PgPool, String) {
        let admin = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect admin");
        let name = format!("fence_probe_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch db");
        let base = crate::test_support::database_url();
        let idx = base.rfind('/').expect("db url has a path segment");
        let pool = PgPool::connect(&format!("{}/{}", &base[..idx], name))
            .await
            .expect("connect scratch db");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch db");
        (admin, pool, name)
    }

    async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(admin)
        .await;
    }

    #[test]
    fn fence_starts_closed_and_opens_on_record() {
        let fence = ReplicaFence::new();
        assert!(fence.verified_through().is_none(), "must start closed");
        assert!(!fence.covers(Utc::now() - chrono::Duration::days(365)));

        let ts = Utc::now();
        let epoch = Uuid::new_v4();
        assert_eq!(
            fence.record(1, epoch, Instant::now(), ts),
            RecordOutcome::Recorded
        );
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
        // Newest entry committed longer ago than the staleness budget.
        let stale_instant = Instant::now() - (FENCE_STALENESS + Duration::from_secs(1));
        fence.record(1, Uuid::new_v4(), stale_instant, ts);
        assert!(
            fence.verified_through().is_none(),
            "a fence the probe stopped confirming must read as closed"
        );
        // heartbeat_age is deliberately ungated (observability).
        assert!(fence.heartbeat_age().expect("entry retained") > FENCE_STALENESS);
    }

    /// Resolve picks the greatest retained entry <= the observed token —
    /// a lagged reader proves from an older wall, never from thin air.
    #[test]
    fn resolve_picks_greatest_retained_token_at_or_below_observation() {
        let fence = ReplicaFence::new();
        let epoch = Uuid::new_v4();
        let base = Utc::now();
        for (token, secs) in [(10i64, 0i64), (20, 10), (30, 20)] {
            fence.record(
                token,
                epoch,
                Instant::now(),
                base + chrono::Duration::seconds(secs),
            );
        }

        // Exact hit.
        assert_eq!(fence.resolve(20, epoch).proved().expect("proof").token, 20);
        // Between entries: prove from the older one.
        assert_eq!(fence.resolve(25, epoch).proved().expect("proof").token, 20);
        // Ahead of everything retained: newest.
        assert_eq!(
            fence.resolve(1000, epoch).proved().expect("proof").token,
            30
        );
        // Behind everything retained: no proof.
        assert_eq!(
            fence.resolve(9, epoch),
            ResolveOutcome::TokenBehind,
            "token below ring fails closed"
        );
        // Wrong epoch: no proof, regardless of token.
        assert_eq!(
            fence.resolve(1000, Uuid::new_v4()),
            ResolveOutcome::EpochMismatch,
            "epoch mismatch fails closed"
        );
    }

    /// An epoch change clears the ring and starts a new one; a same-epoch
    /// token regression clears the ring and reports the fault.
    #[test]
    fn record_epoch_change_resets_and_same_epoch_regression_fails() {
        let fence = ReplicaFence::new();
        let epoch_a = Uuid::new_v4();
        let ts = Utc::now();
        fence.record(10, epoch_a, Instant::now(), ts);
        fence.record(11, epoch_a, Instant::now(), ts);

        // New epoch, lower token: fine — new timeline, old proofs dropped.
        let epoch_b = Uuid::new_v4();
        assert_eq!(
            fence.record(3, epoch_b, Instant::now(), ts),
            RecordOutcome::Recorded
        );
        assert_eq!(
            fence.resolve(11, epoch_a),
            ResolveOutcome::EpochMismatch,
            "entries from the old epoch must be gone"
        );
        assert_eq!(fence.resolve(3, epoch_b).proved().expect("proof").token, 3);

        // Same epoch, non-increasing token: regression → cleared + reported.
        assert_eq!(
            fence.record(3, epoch_b, Instant::now(), ts),
            RecordOutcome::TokenRegression
        );
        assert!(
            fence.verified_through().is_none(),
            "ring cleared on regression"
        );
        assert_eq!(
            fence.resolve(i64::MAX, epoch_b),
            ResolveOutcome::TokenBehind
        );
    }

    /// The ring is bounded: old entries fall off and stop proving coverage.
    #[test]
    fn ring_capacity_evicts_oldest_entries() {
        let fence = ReplicaFence::new();
        let epoch = Uuid::new_v4();
        let ts = Utc::now();
        for token in 0..(RING_CAPACITY as i64 + 10) {
            fence.record(token, epoch, Instant::now(), ts);
        }
        assert_eq!(
            fence.resolve(5, epoch),
            ResolveOutcome::TokenBehind,
            "evicted tokens must no longer prove coverage"
        );
        assert_eq!(
            fence
                .resolve(i64::MAX, epoch)
                .proved()
                .expect("proof")
                .token,
            RING_CAPACITY as i64 + 9
        );
    }

    /// The activity scan must (a) represent another session's open
    /// transaction in the oldest-xact term and (b) ignore plain-idle
    /// sessions, per the agreed classification.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_cluster_global_sample_writer_sees_open_transactions_and_ignores_idle()
    {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply migration schema");

        // A plain idle session: pinned connection, no transaction.
        let idle_pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect idle");
        let _idle_conn = idle_pool.acquire().await.expect("idle conn");

        let before = sample_writer(&pool).await.expect("sample without tx");

        // Now hold a transaction open on a second connection.
        let tx_pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect tx");
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
        // S is captured before the activity scan, the token commit after:
        // the sample's ordering invariant.
        assert!(during.sampled_at >= before.sampled_at);
        assert!(
            during.token > before.token,
            "each sample must commit a strictly newer token"
        );
        assert_eq!(during.epoch, before.epoch, "epoch is stable across samples");

        tx.rollback().await.expect("rollback");
    }

    /// An unprivileged probe role sees NULL `state`/`xact_start` for other
    /// sessions' rows in `pg_stat_activity`. The oldest-xact term is then
    /// untrustworthy and the sample must fail closed (`MaskedActivity`) —
    /// never silently `MIN()` the hidden row away.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cluster_global_sample_writer_fails_closed_when_activity_is_masked() {
        let admin = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect");

        // Hold a transaction open as the privileged user: this is the row
        // the unprivileged probe must notice it cannot classify.
        let tx_pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect tx");
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

        let base = crate::test_support::database_url();
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

    /// The Aurora PostgreSQL identity function name is exact — the
    /// MySQL-family near-miss (`aurora_server_id`) would make the
    /// capability probe cache a permanent false on real Aurora and
    /// silently strip the instance id from canary evidence (Wren, delta
    /// review of a472327). AWS reference: aurora_db_instance_identifier()
    /// (Aurora PostgreSQL user guide; also awslabs/pg-collector).
    #[test]
    fn aurora_identity_function_name_is_the_postgres_one() {
        assert_eq!(AURORA_IDENTITY_FN, "aurora_db_instance_identifier");
    }

    /// The Aurora identity capability probe must answer a definitive
    /// `false` on plain Postgres (undefined_function), not error — and the
    /// error path must not poison the connection for later statements.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn aurora_identity_probe_reports_false_on_plain_postgres() {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect");
        let mut conn = pool.acquire().await.expect("conn");
        assert!(
            !reader_supports_aurora_identity(&mut conn)
                .await
                .expect("probe must not error on plain postgres"),
            "plain postgres must report no aurora identity support"
        );
        // The failed function lookup must not have wedged the session.
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *conn)
            .await
            .expect("connection usable after probe");
        assert_eq!(one, 1);
    }

    /// End-to-end probe against a real database: each probe commits a
    /// strictly newer token, records a retained entry, and a session on the
    /// same database observes a token/epoch that resolves that entry.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cluster_global_probe_commits_tokens_and_sessions_prove_coverage() {
        let (admin, pool, name) = scratch_db().await;
        let fence = ReplicaFence::new();

        let first = probe_once(&pool, &fence).await.expect("first probe");
        let second = probe_once(&pool, &fence).await.expect("second probe");
        assert!(second.token > first.token, "tokens strictly increase");
        assert!(
            second.fence_wall >= first.fence_wall
                || second.fence_wall
                    > first.fence_wall - chrono::Duration::seconds(FENCE_CLOCK_MARGIN_SECS),
            "walls advance with the clock (modulo an open transaction)"
        );

        // A "reader" session on the same database observes at least the
        // second token and proves the newest retained entry.
        let mut conn = pool.acquire().await.expect("reader conn");
        let obs = observe_heartbeat(&mut conn, false)
            .await
            .expect("observe")
            .expect("heartbeat row present");
        assert!(obs.token >= second.token);
        assert!(
            obs.backend.contains(" pid="),
            "backend identity must carry the backend pid, got {:?}",
            obs.backend
        );
        // TCP fixtures also carry addr:port; unix-socket fixtures read 'local'.
        assert!(
            obs.backend.starts_with("local pid=") || obs.backend.contains(':'),
            "backend identity must carry addr:port or 'local', got {:?}",
            obs.backend
        );
        let proof = fence
            .resolve(obs.token, obs.epoch)
            .proved()
            .expect("proof resolves");
        assert_eq!(proof.token, second.token, "newest retained entry cited");

        // An epoch nobody committed proves nothing.
        assert_eq!(
            fence.resolve(obs.token, Uuid::new_v4()),
            ResolveOutcome::EpochMismatch
        );

        drop(conn);
        drop_scratch_db(&admin, pool, &name).await;
    }

    /// A same-epoch token rewind on the writer (restore adversary) must not
    /// leave proofs standing: the probe rotates the epoch, so a reader still
    /// on the pre-rewind timeline — observing a *higher* token under the old
    /// epoch — fails the epoch check instead of proving stale coverage.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cluster_global_probe_rotates_epoch_on_same_epoch_token_regression() {
        let (admin, pool, name) = scratch_db().await;
        let fence = ReplicaFence::new();

        let before = probe_once(&pool, &fence).await.expect("probe");
        let mut conn = pool.acquire().await.expect("conn");
        let old_epoch = observe_heartbeat(&mut conn, false)
            .await
            .expect("observe")
            .expect("row")
            .epoch;

        // Rewind the token in place, keeping the epoch: the restore shape.
        sqlx::query("UPDATE replica_heartbeat SET token = 0 WHERE id = 1")
            .execute(&pool)
            .await
            .expect("rewind token");

        let after = probe_once(&pool, &fence).await.expect("recovery probe");
        // The pre-rewind observation must no longer prove anything.
        assert_eq!(
            fence.resolve(before.token, old_epoch),
            ResolveOutcome::EpochMismatch,
            "old-epoch observations must fail closed after rotation"
        );
        // A fresh observation on the new timeline proves the rotated entry.
        let obs = observe_heartbeat(&mut conn, false)
            .await
            .expect("observe")
            .expect("row");
        assert_ne!(obs.epoch, old_epoch, "epoch rotated");
        assert_eq!(
            fence
                .resolve(obs.token, obs.epoch)
                .proved()
                .expect("proof")
                .token,
            after.token
        );

        drop(conn);
        drop_scratch_db(&admin, pool, &name).await;
    }
}
