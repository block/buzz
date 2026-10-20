//! Process-wide gated adapter for the local archive SQLite database.
//!
//! Two coupled guarantees, both required by plan v3 (decisions 1–2):
//!
//! 1. **Init barrier.** Exactly one blocking task opens the DB the first time
//!    and completes every schema migration (including M4, whose index build
//!    over Will's 1.3M-row archive is not free). Every production open — every
//!    Tauri archive command and all future startup/prune work — `await`s that
//!    result before touching its own connection. The barrier is independent of
//!    identity/relay resolution: the archive DB is a single per-nest file
//!    (identity is a row column, not part of the path), and the path resolves
//!    from `nest_dir()`, which is fixed early in `setup()` before the async
//!    workspace relay override settles. This satisfies Thufir's binding
//!    condition that the barrier cover workspace/identity timing and the
//!    globally-mounted observer archive producer.
//!
//! 2. **Maintenance lock.** A shared `RwLock` whose read guard is held for the
//!    full lifetime of every ordinary connection (acquired before the blocking
//!    dispatch, released only after the closure returns and its connection is
//!    dropped). The Phase-4 "reclaim space" conversion will take the *write*
//!    guard for its sole-connection `VACUUM` sequence; nothing else may hold a
//!    live connection while that runs. Phase 1 only ever takes the read guard,
//!    but the lock lives here so the write path has a home.
//!
//! M4 additionally keeps its own `BEGIN IMMEDIATE` + in-lock recheck for crash
//! and cross-process safety — the in-process barrier serializes this process's
//! opens, but a second OS process (or a direct `open_archive_db` in tests) can
//! still race the first open. The two mechanisms are complementary, not
//! redundant: the barrier is startup orchestration, the immediate-lock is
//! durability.

use std::path::PathBuf;

use rusqlite::Connection;
use tokio::sync::{OnceCell, RwLock};

use super::store;
use crate::managed_agents::nest_dir;

/// A hook run once on the blocking pool at the start of the single init task.
/// Test-only: lets a test count initializations and hold the winner long
/// enough to prove concurrent callers await it. Never set in production.
#[cfg(test)]
type InitHook = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Test-only overrides so the barrier and guard-lifetime contracts can be
/// exercised without a real nest: a fixed DB path in place of `nest_dir()` and
/// an optional init hook. `Default` leaves this `None`, so production always
/// resolves the path from the nest and runs no hook.
#[cfg(test)]
struct TestSeam {
    path: PathBuf,
    on_init: Option<InitHook>,
}

/// Gated owner of every production archive DB connection. Lives in
/// [`crate::app_state::AppState`]; commands call [`ArchiveDb::with_conn`].
#[derive(Default)]
pub struct ArchiveDb {
    /// Set to `()` once the first open (which runs all migrations incl. M4)
    /// succeeds. A failed init is NOT cached — the next caller retries — so a
    /// transient error (e.g. a briefly unavailable external volume) does not
    /// wedge the archive for the process lifetime.
    init: OnceCell<()>,
    /// Maintenance lock. Ordinary connections hold the read guard for their
    /// whole lifetime; the Phase-4 conversion holds the write guard.
    maintenance: RwLock<()>,
    /// Test-only path/hook overrides; always `None` in production.
    #[cfg(test)]
    test_seam: Option<TestSeam>,
}

impl ArchiveDb {
    /// Resolve the archive DB path. Production resolves from the nest
    /// directory; a test seam (when present) supplies a fixed path so the
    /// barrier can be exercised without a real nest. Errors only when the nest
    /// cannot be resolved (fatal for archive access, same as the former
    /// `open_db`).
    fn db_path(&self) -> Result<PathBuf, String> {
        #[cfg(test)]
        if let Some(seam) = &self.test_seam {
            return Ok(seam.path.clone());
        }
        let nest = nest_dir().ok_or("cannot resolve nest directory for archive")?;
        Ok(nest.join("archive").join("archive.db"))
    }

    /// The init hook, if a test installed one; always `None` in production.
    #[cfg(test)]
    fn init_hook(&self) -> Option<InitHook> {
        self.test_seam.as_ref().and_then(|s| s.on_init.clone())
    }

    /// Complete the one-time init: open the DB once on the blocking pool,
    /// running `SCHEMA` + all migrations (incl. M4), then drop the connection.
    /// Concurrent callers await the same single execution. Idempotent and
    /// cheap after the first success (the cached `()` short-circuits).
    async fn ensure_initialized(&self) -> Result<(), String> {
        let path = self.db_path()?;
        #[cfg(test)]
        let hook = self.init_hook();
        self.init
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || {
                    // Test hook runs at the very start of the single init task,
                    // before the migration opens the DB — this is where a test
                    // holds the winner past the busy timeout to prove ordinary
                    // callers await it. No-op in production.
                    #[cfg(test)]
                    if let Some(hook) = hook {
                        hook();
                    }
                    // Opening runs every migration; the connection exists only
                    // to complete them behind the barrier, so drop it here.
                    let conn = store::open_archive_db(&path)?;
                    drop(conn);
                    Ok::<(), String>(())
                })
                .await
                .map_err(|e| format!("archive init task failed: {e}"))?
            })
            .await
            .map(|_| ())
    }

    /// Warm the init barrier without running a query. Called once from
    /// `setup()` so the first-open migration cost (M4's index build over a
    /// large archive) is paid at startup rather than blocking a user's first
    /// archive command. A failure here is non-fatal — the first real
    /// [`with_conn`](Self::with_conn) caller retries and surfaces the error.
    pub async fn warm_init(&self) -> Result<(), String> {
        self.ensure_initialized().await
    }

    /// Run `task` against a fresh archive connection on the blocking pool.
    ///
    /// Ordering: await the init barrier → acquire the maintenance read guard →
    /// dispatch the blocking closure with its own connection. The read guard is
    /// held across the `.await` on the blocking join, so it is released only
    /// after `task` returns and the connection it borrowed has dropped — the
    /// guard-lifetime contract the Phase-4 write path depends on.
    pub async fn with_conn<T, F>(&self, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
    {
        self.ensure_initialized().await?;
        let path = self.db_path()?;
        let _guard = self.maintenance.read().await;
        tokio::task::spawn_blocking(move || {
            let conn = store::open_archive_db(&path)?;
            task(&conn)
        })
        .await
        .map_err(|e| format!("archive db task failed: {e}"))?
    }
}

#[cfg(test)]
impl ArchiveDb {
    /// Build an adapter bound to a fixed DB path (no nest required), so the
    /// barrier and guard-lifetime contracts can be exercised in isolation.
    fn with_test_path(path: PathBuf) -> Self {
        Self {
            init: OnceCell::new(),
            maintenance: RwLock::new(()),
            test_seam: Some(TestSeam {
                path,
                on_init: None,
            }),
        }
    }

    /// Build an adapter bound to a fixed path whose single initialization runs
    /// `hook` first — used to count initializations and to hold the init task
    /// open across the concurrent-caller window.
    fn with_test_hook(path: PathBuf, hook: InitHook) -> Self {
        Self {
            init: OnceCell::new(),
            maintenance: RwLock::new(()),
            test_seam: Some(TestSeam {
                path,
                on_init: Some(hook),
            }),
        }
    }

    /// Whether the maintenance WRITE guard can be taken right now. A live
    /// `with_conn` connection holds the read guard, so this returns `false`
    /// while any ordinary connection is open and `true` once all have dropped —
    /// exactly the signal the Phase-4 sole-connection VACUUM will gate on.
    fn maintenance_write_available(&self) -> bool {
        self.maintenance.try_write().is_ok()
    }
}

#[cfg(test)]
#[path = "archive_db_tests.rs"]
mod archive_db_tests;
