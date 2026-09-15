//! Behavior tests for the [`ArchiveDb`] init barrier and maintenance-lock
//! guard lifetime — the two contracts Phase 1 introduced and Thufir's pass-1
//! review required to be pinned directly (not via raw SQLite contention).
//!
//! These race PRODUCTION-shaped `with_conn` callers through the real
//! `OnceCell`/`RwLock` orchestration, using the `#[cfg(test)]` path/hook seam
//! on `ArchiveDb` to make timing deterministic instead of relying on a large
//! on-disk fixture or wall-clock sleeps to approach the 5s busy timeout.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// A one-shot latch that blocks a blocking-pool thread until the test releases
/// it. Deterministic stand-in for "the M4 winner holds init open past the busy
/// timeout": the init/closure thread parks here, the test observes the frozen
/// state, then releases. `Mutex` + `Condvar` are `Sync`, so an `Arc<Latch>`
/// captured by the `Send + Sync` init hook type-checks.
struct Latch {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Latch {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(false),
            cv: Condvar::new(),
        })
    }

    /// Block until [`release`](Self::release) is called (returns immediately if
    /// already released).
    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.cv.wait(open).unwrap();
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

/// Poll `cond` on the async runtime until it holds or `timeout` elapses.
/// Panics on timeout so a broken barrier surfaces as a failure, never a hang.
async fn await_until(what: &str, timeout: Duration, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// An archive DB path inside a fresh temp dir. The dir is returned so the
/// caller keeps it alive for the whole test (dropping it deletes the file).
fn temp_db() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive").join("archive.db");
    (dir, path)
}

/// The barrier serializes first-open: exactly one initialization runs while
/// every concurrent `with_conn` caller awaits it, and no ordinary connection
/// opens until that initialization has completed.
///
/// The init hook holds the single init task open on a latch. While it is held
/// we prove no `with_conn` caller has opened its connection (`open_count == 0`)
/// — impossible if callers bypassed the `OnceCell` and opened independently.
/// Releasing the latch lets init finish; all callers then complete, exactly
/// one initialization ran, and every open happened after init.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_first_open_barrier_serializes_init_and_defers_opens() {
    let (_dir, path) = temp_db();
    let init_count = Arc::new(AtomicUsize::new(0));
    let open_count = Arc::new(AtomicUsize::new(0));
    let release = Latch::new();

    let hook = {
        let init_count = Arc::clone(&init_count);
        let release = Arc::clone(&release);
        Arc::new(move || {
            // Runs once, at the head of the single init task. Record the
            // initialization, then park so the test can inspect the frozen
            // pre-open state.
            init_count.fetch_add(1, Ordering::SeqCst);
            release.wait();
        }) as InitHook
    };
    let db = Arc::new(ArchiveDb::with_test_hook(path, hook));

    // Trigger the single init and wait until it is provably in flight (hook
    // ran) and parked on the latch.
    let warm = {
        let db = Arc::clone(&db);
        tokio::spawn(async move { db.warm_init().await })
    };
    await_until("init to start", Duration::from_secs(10), || {
        init_count.load(Ordering::SeqCst) == 1
    })
    .await;

    // Fan out production-shaped callers while init is held. Each records that
    // its task was scheduled (`entered`) and that its closure actually opened a
    // connection (`open_count`).
    let entered = Arc::new(AtomicUsize::new(0));
    let callers: Vec<_> = (0..4)
        .map(|_| {
            let db = Arc::clone(&db);
            let open_count = Arc::clone(&open_count);
            let entered = Arc::clone(&entered);
            tokio::spawn(async move {
                entered.fetch_add(1, Ordering::SeqCst);
                db.with_conn(move |conn| {
                    open_count.fetch_add(1, Ordering::SeqCst);
                    // Touch the migrated schema to prove a usable connection.
                    conn.query_row("SELECT COUNT(*) FROM archive_meta", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(|e| e.to_string())
                })
                .await
            })
        })
        .collect();

    // All four caller tasks are scheduled and running before we judge the
    // barrier: they have entered `with_conn` and can only be parked on the
    // init `OnceCell`. Without the barrier they would instead open independent
    // connections here and bump `open_count` while init is still held.
    await_until("callers to be scheduled", Duration::from_secs(10), || {
        entered.load(Ordering::SeqCst) == 4
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        open_count.load(Ordering::SeqCst),
        0,
        "no ordinary connection may open until initialization completes"
    );

    // Let init finish; every caller now completes against the migrated DB.
    release.release();
    assert!(warm.await.unwrap().is_ok(), "warm init must succeed");
    for caller in callers {
        assert!(
            caller.await.unwrap().is_ok(),
            "every with_conn must succeed"
        );
    }

    assert_eq!(
        init_count.load(Ordering::SeqCst),
        1,
        "exactly one initialization ran behind the barrier"
    );
    assert_eq!(
        open_count.load(Ordering::SeqCst),
        4,
        "all callers opened, and only after init"
    );
}

/// The maintenance read guard lives for the FULL lifetime of a `with_conn`
/// connection: a write-lock contender cannot enter until the closure returns
/// and its connection has dropped. This is the invariant the Phase-4
/// sole-connection VACUUM depends on.
///
/// A `with_conn` closure parks on a latch while holding its connection; a
/// separate task contends for the maintenance write guard. While the closure
/// is parked the writer must be blocked. Releasing the closure — which returns
/// and drops the connection — lets the writer finally acquire the guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_with_conn_read_guard_blocks_writer_until_connection_drops() {
    let (_dir, path) = temp_db();
    let db = Arc::new(ArchiveDb::with_test_path(path));
    db.warm_init().await.expect("init must succeed");

    let in_closure = Arc::new(AtomicUsize::new(0));
    let hold = Latch::new();

    // A live connection: the closure parks holding it (and thus the read
    // guard) until released.
    let work = {
        let db = Arc::clone(&db);
        let in_closure = Arc::clone(&in_closure);
        let hold = Arc::clone(&hold);
        tokio::spawn(async move {
            db.with_conn(move |_conn| {
                in_closure.fetch_add(1, Ordering::SeqCst);
                hold.wait();
                Ok(())
            })
            .await
        })
    };
    await_until(
        "closure to hold the connection",
        Duration::from_secs(10),
        || in_closure.load(Ordering::SeqCst) == 1,
    )
    .await;

    // A genuine write-lock contender.
    let write_entered = Arc::new(AtomicUsize::new(0));
    let writer = {
        let db = Arc::clone(&db);
        let write_entered = Arc::clone(&write_entered);
        tokio::spawn(async move {
            let _w = db.maintenance.write().await;
            write_entered.fetch_add(1, Ordering::SeqCst);
        })
    };

    // While the connection is held, the writer must not have entered, and the
    // write guard must be unavailable.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        write_entered.load(Ordering::SeqCst),
        0,
        "writer must block while a with_conn connection is live"
    );
    assert!(
        !db.maintenance_write_available(),
        "write guard is unavailable while the read guard is held"
    );

    // Release the closure: it returns and its connection drops, releasing the
    // read guard so the writer can proceed.
    hold.release();
    assert!(work.await.unwrap().is_ok(), "held with_conn must succeed");
    writer.await.unwrap();
    assert_eq!(
        write_entered.load(Ordering::SeqCst),
        1,
        "writer enters once the connection has dropped"
    );
    assert!(
        db.maintenance_write_available(),
        "write guard is free again after the connection drops"
    );
}

/// The prune single-flight admits exactly one holder: while the winner's guard
/// is live, every later `try_prune_guard()` returns `None` and — critically —
/// leaves the flag SET, so the winner keeps its claim. The flag clears only
/// when the winner's guard drops, after which a fresh caller wins.
///
/// This pins the eager-vs-lazy guard-construction contract directly. Reverting
/// `try_prune_guard` to `.then_some(PruneGuard(..))` makes each losing caller
/// construct and immediately drop a `PruneGuard`, whose `Drop` clears the flag
/// under the running winner — the third assertion below then wins wrongly.
#[test]
fn test_try_prune_guard_admits_one_and_holds_until_dropped() {
    let (_dir, path) = temp_db();
    let db = ArchiveDb::with_test_path(path);

    let guard = db.try_prune_guard().expect("first caller claims the flag");

    // Two more contenders while the winner holds the guard. The SECOND losing
    // call is the mutation trip-wire: if a prior loser had cleared the flag,
    // this call would wrongly win.
    assert!(
        db.try_prune_guard().is_none(),
        "a contender must lose while the guard is held"
    );
    assert!(
        db.try_prune_guard().is_none(),
        "the flag must still be set — no prior loser may have cleared it"
    );

    // Only the winner's drop releases the flag.
    drop(guard);
    let next = db
        .try_prune_guard()
        .expect("a fresh caller wins once the guard drops");
    assert!(
        db.try_prune_guard().is_none(),
        "single-flight holds again for the new holder"
    );
    drop(next);
}

/// The `maybe_prune`-shaped trigger admits exactly one prune body across a burst
/// of simultaneous triggers: the admitted pass holds its guard across the
/// awaited `with_conn` work, so every concurrent trigger that loses the
/// single-flight returns without entering the body.
///
/// Each task mirrors the shipped control flow verbatim — `let Some(_guard) =
/// try_prune_guard() else { return }` then an awaited `with_conn` pass. The one
/// admitted pass parks on a latch so all triggers are in flight together; the
/// held count must never exceed one. This is Gurney's live-lane regression
/// reproduced deterministically: under the eager-`then_some` bug, losers cleared
/// the flag and multiple passes entered concurrently.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_prune_triggers_admit_exactly_one_pass() {
    let (_dir, path) = temp_db();
    let db = Arc::new(ArchiveDb::with_test_path(path));
    db.warm_init().await.expect("init must succeed");

    let entered = Arc::new(AtomicUsize::new(0));
    let hold = Latch::new();

    let triggers: Vec<_> = (0..8)
        .map(|_| {
            let db = Arc::clone(&db);
            let entered = Arc::clone(&entered);
            let hold = Arc::clone(&hold);
            tokio::spawn(async move {
                let Some(_guard) = db.try_prune_guard() else {
                    return;
                };
                db.with_conn(move |_conn| {
                    entered.fetch_add(1, Ordering::SeqCst);
                    hold.wait();
                    Ok(())
                })
                .await
                .unwrap();
            })
        })
        .collect();

    // Exactly one pass enters and then parks on the latch. Any second admission
    // would race in and push the count past one.
    await_until("one prune pass to enter", Duration::from_secs(10), || {
        entered.load(Ordering::SeqCst) == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "single-flight admits exactly one prune while a pass is held"
    );

    // Release the held pass; the losers already returned, so nothing new enters.
    hold.release();
    for trigger in triggers {
        trigger.await.unwrap();
    }
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "no further pass entered after the held one completed"
    );
}
