//! Fleet-wide session slots shared by every harness process on this host.
//!
//! One slot is one live ACP session — the adapter's per-channel worker (for
//! `claude-agent-acp`, a `claude` process plus its MCP servers), which is what
//! actually costs memory. Slots are `flock`ed lock files in a shared
//! directory: a harness holds a slot for as long as it keeps the lock file
//! open, and the kernel releases the lock the moment that process exits for
//! any reason, so a crashed harness can never strand a slot.
//!
//! This is the `maximumPoolSize` half of a connection pool. The per-harness
//! `--agents` bound and the `--session-idle-close` return path live in
//! `pool.rs` and the main loop.
//!
//! Unix-only (`flock(2)`); elsewhere `FleetPool::new` refuses to start.

use std::path::{Path, PathBuf};

use tokio::time::Instant;

/// One acquired fleet slot. Dropping it releases the slot.
pub struct FleetSlot {
    index: u32,
    #[cfg(unix)]
    _lock: nix::fcntl::Flock<std::fs::File>,
}

impl FleetSlot {
    pub fn index(&self) -> u32 {
        self.index
    }
}

/// Handle on the shared slot directory for one harness.
pub struct FleetPool {
    dir: PathBuf,
    slots: u32,
    /// Set on the first failed acquire, cleared on the next success, so the
    /// exhausted/acquired transitions are logged once each instead of on
    /// every dispatch pass.
    waiting_since: Option<Instant>,
}

impl FleetPool {
    pub fn new(dir: impl Into<PathBuf>, slots: u32) -> std::io::Result<Self> {
        let dir = dir.into();
        if !cfg!(unix) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "fleet slots need flock(2); unavailable on this platform",
            ));
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            slots,
            waiting_since: None,
        })
    }

    pub fn slots(&self) -> u32 {
        self.slots
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// True while the last acquire attempt found every slot held.
    pub fn is_waiting(&self) -> bool {
        self.waiting_since.is_some()
    }

    /// Claim the lowest free slot without blocking. `None` when every slot is
    /// held (by this or another harness); the caller parks its work and tries
    /// again on its next dispatch pass.
    ///
    /// A slot released while another thread is between `fork` and `exec` can
    /// look held for a few microseconds (the child briefly holds a duplicate
    /// of the descriptor). Callers already retry on their next pass, so this
    /// is never worse than a short wait.
    pub fn try_acquire(&mut self) -> Option<FleetSlot> {
        for index in 0..self.slots {
            let path = self.dir.join(format!("slot-{index:02}.lock"));
            match try_lock_file(&path) {
                Ok(Some(lock)) => {
                    if let Some(since) = self.waiting_since.take() {
                        tracing::info!(
                            target: "fleet",
                            slot = index,
                            waited_secs = since.elapsed().as_secs(),
                            "fleet slot acquired after wait"
                        );
                    }
                    return Some(FleetSlot {
                        index,
                        #[cfg(unix)]
                        _lock: lock,
                    });
                }
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(
                        target: "fleet",
                        path = %path.display(),
                        "fleet slot unusable: {error}"
                    );
                    continue;
                }
            }
        }
        if self.waiting_since.is_none() {
            self.waiting_since = Some(Instant::now());
            tracing::info!(
                target: "fleet",
                slots = self.slots,
                dir = %self.dir.display(),
                "fleet_exhausted — parking work until a slot is released"
            );
        }
        None
    }
}

/// `Ok(Some)` = locked, `Ok(None)` = held by someone else, `Err` = cannot
/// open or lock the file at all.
#[cfg(unix)]
fn try_lock_file(path: &Path) -> std::io::Result<Option<nix::fcntl::Flock<std::fs::File>>> {
    use nix::errno::Errno;
    use nix::fcntl::{Flock, FlockArg};

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => Ok(Some(lock)),
        Err((_, Errno::EWOULDBLOCK)) => Ok(None),
        Err((_, errno)) => Err(std::io::Error::from(errno)),
    }
}

#[cfg(not(unix))]
fn try_lock_file(_path: &Path) -> std::io::Result<Option<()>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "flock(2) unavailable",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "buzz-acp-fleet-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    /// Retry briefly: a sibling test mid-`spawn` can hold a duplicate of a
    /// just-dropped lock descriptor until its `exec` (see `try_acquire`).
    async fn acquire_soon(pool: &mut FleetPool) -> Option<FleetSlot> {
        for _ in 0..50 {
            if let Some(slot) = pool.try_acquire() {
                return Some(slot);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        None
    }

    #[tokio::test]
    async fn acquires_up_to_the_cap_and_reuses_released_slots() {
        let dir = scratch_dir("cap");
        let mut pool = FleetPool::new(&dir, 2).expect("create slot dir");
        assert!(dir.is_dir());

        let first = pool.try_acquire().expect("slot 0");
        let second = pool.try_acquire().expect("slot 1");
        assert_eq!((first.index(), second.index()), (0, 1));
        assert!(pool.try_acquire().is_none());
        assert!(pool.is_waiting());

        drop(first);
        let reused = acquire_soon(&mut pool).await.expect("slot 0 again");
        assert_eq!(reused.index(), 0);
        assert!(!pool.is_waiting());
        drop(second);
        drop(reused);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn slots_are_shared_between_pools_on_the_same_dir() {
        // Two pools on one directory stand in for two harness processes: the
        // lock lives on the file, not in this pool's memory.
        let dir = scratch_dir("shared");
        let mut one = FleetPool::new(&dir, 1).expect("create slot dir");
        let mut two = FleetPool::new(&dir, 1).expect("reuse slot dir");

        let held = one.try_acquire().expect("first pool takes the slot");
        assert!(two.try_acquire().is_none());
        drop(held);
        assert!(acquire_soon(&mut two).await.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_dead_holder_releases_its_slot_to_the_kernel() {
        // A child process takes the lock and is killed; the slot must come
        // back without any bookkeeping on our side.
        let dir = scratch_dir("dead");
        let mut pool = FleetPool::new(&dir, 1).expect("create slot dir");
        let lock_path = dir.join("slot-00.lock");
        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(format!(
                "exec 9>'{}'; flock 9; echo locked; sleep 30 9>&-",
                lock_path.display()
            ))
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn locking child");
        let mut stdout = child.stdout.take().expect("child stdout");
        let mut buf = [0u8; 8];
        let _ = tokio::io::AsyncReadExt::read(&mut stdout, &mut buf).await;

        assert!(pool.try_acquire().is_none(), "child holds the only slot");
        child.kill().await.expect("kill child");
        let _ = child.wait().await;
        assert!(
            acquire_soon(&mut pool).await.is_some(),
            "slot released on child exit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
