//! Client-side leader election.
//!
//! When multiple Buzz instances share the same agent keypair, the relay fans
//! every matching event out to all of them (NIP-01). Without coordination,
//! each instance would prompt its agent and respond — duplicate replies for a
//! single mention. A per-agent-key *leader lock* designates exactly one
//! instance as the active responder; non-leaders still receive and render
//! events but suppress the prompt path (and the pre-dispatch `👀` side-effect).
//!
//! This module owns both halves: the **read** side decides whether *this*
//! process is the leader for a given agent pubkey; the **write** side
//! ([`FileLeaderCheck::acquire`] / [`FileLeaderCheck::release`]) claims and
//! relinquishes the lock.
//!
//! # Lock contract
//!
//! - Lock dir: `~/.buzz/leader-locks/`, one file per agent pubkey:
//!   `<pubkey-hex>.lock`, JSON `{"instance_id","pid","claimed_at"}`.
//! - **Absent or unreadable** lock file → this process is leader. Single-instance
//!   dev is thereby unaffected: no lock, no suppression. Any IO error reading the
//!   lock (permission, mid-write truncation) fails safe to leader for the same reason.
//! - **Present** → leader iff the lock's `instance_id` equals this process's
//!   own election id ([`ELECTION_ID_ENV`]).
//! - **No instance id** (`instance_id` unset on the check itself, e.g. tests
//!   constructing a read-only checker) → leader. There is no coordinating
//!   instance, so there is nothing to defer to. The harness always carries a
//!   minted id (see [`FileLeaderCheck::from_env_or_mint`]).
//! - **Malformed** lock file → fail safe to leader. A corrupt lock must never
//!   silence the only responder.
//!
//! ## Claim
//!
//! Claim is **auto-on-launch plus explicit re-claim**. The harness self-elects
//! at startup: [`FileLeaderCheck::from_env_or_mint`] gives every process a
//! unique election id, and the lifecycle calls [`acquire`](FileLeaderCheck::acquire)
//! to take an *unowned* lock. An explicit re-claim / hard-steal gesture (sidebar
//! UI) layers on top in a later phase; both paths share the same writer.
//!
//! Acquisition is guarded by an exclusive `flock` held across the
//! read-decide-write window, closing the TOCTOU race between two instances
//! racing to claim the same key. A claim **succeeds** when the lock is free
//! (absent, empty, or malformed), already ours, held by a **dead** pid, or held
//! by a **stale** claim whose pid was recycled by an unrelated process
//! (failover when a leader crashes without releasing); it **fails** — leaving
//! this process an observer — only when a *live* foreign pid holds a *fresh*
//! claim.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Stand-down timeout: how long `acquire()` is suppressed after `stand_down()`.
///
/// 2× the 5s refresh tick guarantees the target gets at least one full tick to
/// acquire before the old leader resumes. If the target is alive, it acquires
/// within 5s. If the target crashed, the old leader recovers after this timeout
/// — zero-leader window is bounded and self-healing.
#[cfg(not(test))]
const STAND_DOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Shortened for tests so we can exercise the auto-expire path without sleeping.
#[cfg(test)]
const STAND_DOWN_TIMEOUT: Duration = Duration::from_millis(5);

/// Environment variable carrying this window's leader-election identity.
///
/// Per-window election identity. The desktop may inject a process-unique value
/// per spawn; when unset, [`FileLeaderCheck::from_env_or_mint`] mints one
/// in-process so the harness can self-elect headlessly. The Tauri bundle
/// identifier is NOT sufficient as a source — it collides across same-class
/// windows (DMG + dev, or worktrees whose icon-gen fell back to the shared dev
/// id). Distinct from `BUZZ_MANAGED_AGENT` (reaper identity): reaper identity
/// partitions by app-class, election identity must be unique per window —
/// opposite uniqueness requirements.
const ELECTION_ID_ENV: &str = "BUZZ_INSTANCE_ELECTION_ID";

/// A claim older than this is treated as abandoned for takeover purposes, even
/// if its pid still maps to a live process (which can happen after the OS
/// recycles a crashed leader's pid).
///
/// Must comfortably exceed the 5s `leader_refresh` cadence in `lib.rs`: a live
/// leader rewrites `claimed_at` on every tick, so this bound only ever elapses
/// for an abandoned claim. The 2x margin tolerates a single delayed tick
/// without falsely evicting an active leader (which would split-brain — worse
/// than the stall this guards against).
#[cfg(unix)]
const STALE_CLAIM: std::time::Duration = std::time::Duration::from_secs(10);

/// Decides whether this process should act on events for a given agent key.
pub trait LeaderCheck: Send + Sync {
    /// Whether this process is the leader for `agent_pubkey_hex`.
    ///
    /// Reads through the cache on first sight of a key so the very first
    /// dispatch is correct without waiting for a refresh tick; subsequent
    /// calls are served from cache and updated by [`LeaderCheck::refresh`].
    fn is_leader(&self, agent_pubkey_hex: &str) -> bool;

    /// Re-read all known lock files and update cached status. Called on a
    /// fixed cadence by the event loop so leadership changes take effect
    /// without a restart.
    fn refresh(&self);
}

/// On-disk lock file shape for the read side. Only `instance_id` is
/// load-bearing here; `pid` and `claimed_at` are written by the acquire path
/// and ignored by the reader. The write side parses with [`OwnerInfo`] instead,
/// which also reads `pid` for dead-pid takeover.
#[derive(Deserialize)]
struct LockFile {
    instance_id: String,
}

/// Filesystem-backed [`LeaderCheck`] over `~/.buzz/leader-locks/`.
pub struct FileLeaderCheck {
    /// This process's election id ([`ELECTION_ID_ENV`]). `None` when unset —
    /// solo CLI use or the pre-Phase-2 regime, where this process is always
    /// leader.
    instance_id: Option<String>,
    lock_dir: PathBuf,
    /// Cached leader status per agent pubkey hex. Seeded read-through on first
    /// `is_leader`, refreshed in place by `refresh`.
    cache: Mutex<HashMap<String, bool>>,
    /// When set, `acquire()` becomes a no-op (returns false without touching
    /// the lock file) until the stand-down is cleared by `resume()` or after
    /// [`STAND_DOWN_TIMEOUT`] expires.
    standing_down: AtomicBool,
    /// When the stand-down was entered. Used by `is_standing_down()` to
    /// auto-expire the suppression after [`STAND_DOWN_TIMEOUT`].
    stood_down_at: Mutex<Option<Instant>>,
}

impl FileLeaderCheck {
    /// Build from the ambient environment, minting a process-unique election
    /// id when [`ELECTION_ID_ENV`] is unset so a process with no externally
    /// supplied identity can still claim a lock and self-elect headlessly.
    /// Lock dir is `$HOME/.buzz/leader-locks/`.
    ///
    /// The minted id is `{pid}-{nanos}`. The pid feeds dead-pid takeover; the
    /// launch-time nanos make the id unique across processes that share a pid
    /// over time, so a recycled pid can't be mistaken for *us* in the
    /// self-ownership check. Recycled-pid takeover (a crashed leader's pid
    /// reused by an unrelated live process) is handled separately by the
    /// `claimed_at` staleness bound in [`lock_is_takeable`](Self::lock_is_takeable),
    /// not by the nanos. An externally set env value is honored verbatim
    /// (desktop per-spawn injection).
    pub fn from_env_or_mint() -> Self {
        let instance_id = std::env::var(ELECTION_ID_ENV).ok().unwrap_or_else(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}-{}", std::process::id(), nanos)
        });
        Self::new(Some(instance_id), Self::default_lock_dir())
    }

    fn default_lock_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".buzz")
            .join("leader-locks")
    }

    fn new(instance_id: Option<String>, lock_dir: PathBuf) -> Self {
        Self {
            instance_id,
            lock_dir,
            cache: Mutex::new(HashMap::new()),
            standing_down: AtomicBool::new(false),
            stood_down_at: Mutex::new(None),
        }
    }

    /// Read the lock for `pubkey_hex` and compute leadership per the contract.
    fn read_status(&self, pubkey_hex: &str) -> bool {
        // No coordinating instance id → nothing to defer to.
        let Some(self_id) = self.instance_id.as_deref() else {
            return true;
        };
        let path = self.lock_dir.join(format!("{pubkey_hex}.lock"));
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            // Absent (or unreadable) lock → leader.
            Err(_) => return true,
        };
        match serde_json::from_str::<LockFile>(&contents) {
            Ok(lock) => lock.instance_id == self_id,
            // Malformed lock → fail safe to leader.
            Err(_) => true,
        }
    }

    /// Returns this instance's election id, or `None` for always-leader mode.
    pub fn instance_id(&self) -> Option<&str> {
        self.instance_id.as_deref()
    }

    /// Enter stand-down: suppress all `acquire()` calls until `resume()` or
    /// [`STAND_DOWN_TIMEOUT`] expires. Called when this instance receives a
    /// `claim_leadership` targeting a different instance and this instance is
    /// the current leader.
    pub fn stand_down(&self) {
        self.standing_down.store(true, Ordering::Release);
        *self.stood_down_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    /// Exit stand-down. Called by the target after successful acquire, or
    /// automatically when [`STAND_DOWN_TIMEOUT`] expires.
    #[allow(dead_code)]
    pub fn resume(&self) {
        self.standing_down.store(false, Ordering::Release);
        *self.stood_down_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether this instance is currently standing down (suppressing acquire).
    /// Auto-expires after [`STAND_DOWN_TIMEOUT`] to prevent permanent
    /// zero-leader on a lost handoff.
    fn is_standing_down(&self) -> bool {
        if !self.standing_down.load(Ordering::Acquire) {
            return false;
        }
        let mut guard = self.stood_down_at.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = *guard {
            if at.elapsed() > STAND_DOWN_TIMEOUT {
                // Auto-expire: clear stand-down inline to avoid re-locking.
                self.standing_down.store(false, Ordering::Release);
                *guard = None;
                return false;
            }
        }
        true
    }

    /// Claim the leader lock for `pubkey_hex`, returning whether this process
    /// now holds it (and may act as leader).
    ///
    /// Holds an exclusive `flock` across the read-decide-write window so two
    /// instances racing to claim the same key cannot both win. Succeeds when
    /// the lock is free (absent/empty/malformed), already ours, or held by a
    /// dead pid; fails (observer) only when a live foreign pid holds it.
    ///
    /// Best-effort and idempotent: an IO error (e.g. unwritable lock dir)
    /// returns the read-side fail-safe (leader) rather than propagating, and
    /// re-claiming a lock we already own simply rewrites it.
    #[cfg(unix)]
    pub fn acquire(&self, pubkey_hex: &str) -> bool {
        use nix::fcntl::{Flock, FlockArg};
        use std::io::{Read, Seek, SeekFrom, Write};

        // Stand-down suppresses re-claim during cooperative handoff.
        if self.is_standing_down() {
            return false;
        }

        // No election id → nothing to claim with; mirror the read-side
        // always-leader contract.
        let Some(self_id) = self.instance_id.as_deref() else {
            return true;
        };

        if std::fs::create_dir_all(&self.lock_dir).is_err() {
            return true; // can't write a lock → fail safe to leader
        }
        let path = self.lock_dir.join(format!("{pubkey_hex}.lock"));
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
        {
            Ok(f) => f,
            Err(_) => return true,
        };
        let mut locked = match Flock::lock(file, FlockArg::LockExclusive) {
            Ok(l) => l,
            Err(_) => return true,
        };

        let mut contents = String::new();
        if locked.read_to_string(&mut contents).is_err() {
            return true;
        }
        if !self.lock_is_takeable(&contents, self_id) {
            return false; // live foreign owner — observe
        }

        let body = format!(
            r#"{{"instance_id":"{self_id}","pid":{},"claimed_at":"{}"}}"#,
            std::process::id(),
            chrono::Utc::now().to_rfc3339(),
        );
        // Truncate-then-write under the held lock so a shorter claim can't
        // leave a trailing tail of the previous owner's JSON.
        let written = locked
            .seek(SeekFrom::Start(0))
            .and_then(|_| locked.set_len(0))
            .and_then(|_| locked.write_all(body.as_bytes()))
            .and_then(|_| locked.flush());
        written.is_ok()
    }

    /// Release the leader lock for `pubkey_hex` if and only if we own it.
    ///
    /// Empties the file rather than unlinking it: removing a file another
    /// process may already hold an `flock` fd on would race a fresh claim onto
    /// a detached inode. An empty file reads back as malformed → fail-safe to
    /// leader, so the next acquirer treats it as free.
    #[cfg(unix)]
    pub fn release(&self, pubkey_hex: &str) {
        use nix::fcntl::{Flock, FlockArg};
        use std::io::Read;

        let Some(self_id) = self.instance_id.as_deref() else {
            return;
        };
        let path = self.lock_dir.join(format!("{pubkey_hex}.lock"));
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        else {
            return; // nothing to release
        };
        let Ok(mut locked) = Flock::lock(file, FlockArg::LockExclusive) else {
            return;
        };
        let mut contents = String::new();
        if locked.read_to_string(&mut contents).is_err() {
            return;
        }
        // Only clear a lock we still own — never stomp a successor's claim.
        if matches!(serde_json::from_str::<LockFile>(&contents), Ok(l) if l.instance_id == self_id)
        {
            let _ = locked.set_len(0);
        }
    }

    /// Whether the current lock `contents` may be claimed by `self_id`:
    /// free (absent text/empty/malformed), already ours, held by a dead pid, or
    /// held by a *stale* claim whose pid has been recycled by an unrelated
    /// process.
    ///
    /// The stale arm closes a failover stall: a leader SIGKILLed without
    /// release leaves a lock whose pid the OS may recycle for an unrelated,
    /// long-lived process. A bare pid-alive probe would then read that recycled
    /// pid as the original leader and never take over, wedging the agent
    /// leaderless indefinitely. A *live* leader rewrites `claimed_at` on every
    /// refresh tick, so its claim is always fresher than [`STALE_CLAIM`]; only
    /// an abandoned claim ages past the bound. Pairing pid-alive with a stale
    /// timestamp distinguishes a recycled pid from a genuinely active leader
    /// without evicting the latter.
    #[cfg(unix)]
    fn lock_is_takeable(&self, contents: &str, self_id: &str) -> bool {
        #[derive(serde::Deserialize)]
        struct OwnerInfo {
            instance_id: String,
            pid: u32,
            claimed_at: String,
        }
        match serde_json::from_str::<OwnerInfo>(contents) {
            Ok(owner) => {
                owner.instance_id == self_id
                    || !pid_is_alive(owner.pid)
                    || claim_is_stale(&owner.claimed_at)
            }
            Err(_) => true, // empty or malformed → free to take
        }
    }

    /// Non-Unix has no `flock`; claims always succeed (always-leader), matching
    /// the read-side absent→leader contract and the `kill_process_group`
    /// `#[cfg(not(unix))]` pattern. The desktop targets macOS/Linux only.
    #[cfg(not(unix))]
    pub fn acquire(&self, _pubkey_hex: &str) -> bool {
        if self.is_standing_down() {
            return false;
        }
        true
    }

    /// Non-Unix release is a no-op (nothing was written).
    #[cfg(not(unix))]
    pub fn release(&self, _pubkey_hex: &str) {}
}

/// Whether a process with `pid` is alive, via signal 0 (existence probe).
/// `ESRCH` means gone; other errors (e.g. `EPERM` for a live process we can't
/// signal) are treated as alive so we never steal from a running leader.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    !matches!(kill(Pid::from_raw(pid as i32), None), Err(Errno::ESRCH))
}

/// Whether an RFC 3339 `claimed_at` is older than [`STALE_CLAIM`].
///
/// An unparseable timestamp is treated as stale (takeable): a claim we can't
/// date can't be proven fresh, so we must not let it wedge a takeover. A claim
/// timestamped in the future (clock skew) is not stale — `signed_duration_since`
/// is negative, which never exceeds the positive bound.
#[cfg(unix)]
fn claim_is_stale(claimed_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(claimed_at) {
        Ok(claimed) => {
            chrono::Utc::now().signed_duration_since(claimed)
                > chrono::Duration::from_std(STALE_CLAIM).unwrap_or(chrono::Duration::MAX)
        }
        Err(_) => true,
    }
}

impl LeaderCheck for FileLeaderCheck {
    fn is_leader(&self, agent_pubkey_hex: &str) -> bool {
        if let Some(&cached) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_pubkey_hex)
        {
            return cached;
        }
        let status = self.read_status(agent_pubkey_hex);
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent_pubkey_hex.to_string(), status);
        status
    }

    fn refresh(&self) {
        // Re-read only keys we've already been asked about — those are the
        // agent pubkeys this process actually dispatches for.
        let keys: Vec<String> = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for key in keys {
            let status = self.read_status(&key);
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const PUBKEY: &str = "abc123";
    // Opaque per-window election ids — deliberately NOT bundle-class strings.
    // The leader-election identity must be unique per window; baking in
    // `bundle-id == election-id` would mask the same-class collision Phase 2
    // must avoid.
    const SELF_ID: &str = "window-a";
    const OTHER_ID: &str = "window-b";

    /// Unique scratch dir per test, removed on drop. Avoids a dev-dep on
    /// `tempfile` for four file reads.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "buzz-acp-leader-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn lock_path(&self) -> PathBuf {
            self.0.join(format!("{PUBKEY}.lock"))
        }

        fn write_lock(&self, contents: &str) {
            std::fs::write(self.lock_path(), contents).unwrap();
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_absent_lock_file_is_leader() {
        let dir = TmpDir::new();
        let lc = FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone());
        assert!(lc.is_leader(PUBKEY));
    }

    #[test]
    fn test_lock_matching_own_instance_is_leader() {
        let dir = TmpDir::new();
        dir.write_lock(&format!(
            r#"{{"instance_id":"{SELF_ID}","pid":123,"claimed_at":"2026-06-15T00:00:00Z"}}"#
        ));
        let lc = FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone());
        assert!(lc.is_leader(PUBKEY));
    }

    #[test]
    fn test_lock_naming_other_instance_is_observer() {
        let dir = TmpDir::new();
        dir.write_lock(&format!(
            r#"{{"instance_id":"{OTHER_ID}","pid":123,"claimed_at":"2026-06-15T00:00:00Z"}}"#
        ));
        let lc = FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone());
        assert!(!lc.is_leader(PUBKEY));
    }

    #[test]
    fn test_malformed_lock_fails_safe_to_leader() {
        let dir = TmpDir::new();
        dir.write_lock("{ this is not json ");
        let lc = FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone());
        assert!(lc.is_leader(PUBKEY));
    }

    #[test]
    fn test_no_instance_id_is_leader_even_with_foreign_lock() {
        let dir = TmpDir::new();
        dir.write_lock(&format!(
            r#"{{"instance_id":"{OTHER_ID}","pid":123,"claimed_at":"2026-06-15T00:00:00Z"}}"#
        ));
        let lc = FileLeaderCheck::new(None, dir.0.clone());
        assert!(lc.is_leader(PUBKEY));
    }

    #[test]
    fn test_refresh_flips_status_when_lock_changes() {
        let dir = TmpDir::new();
        let lc = FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone());

        // No lock yet → leader, and the key is now cached.
        assert!(lc.is_leader(PUBKEY));

        // A foreign instance claims the lock.
        dir.write_lock(&format!(r#"{{"instance_id":"{OTHER_ID}"}}"#));
        // Cache still says leader until refresh.
        assert!(lc.is_leader(PUBKEY));

        lc.refresh();
        assert!(!lc.is_leader(PUBKEY));

        // Lock removed (leader stepped down) → back to leader after refresh.
        std::fs::remove_file(dir.lock_path()).unwrap();
        lc.refresh();
        assert!(lc.is_leader(PUBKEY));
    }

    // ── Writer (acquire / release) — Unix only ───────────────────────────────
    #[cfg(unix)]
    mod writer {
        use super::*;

        /// A checker rooted at `dir` with election id `id`, as the harness
        /// builds it (always `Some(id)`).
        fn checker(dir: &TmpDir, id: &str) -> FileLeaderCheck {
            FileLeaderCheck::new(Some(id.into()), dir.0.clone())
        }

        /// PID guaranteed not to be alive: spawn a trivial child, reap it.
        /// The kernel won't recycle the number while the test runs.
        fn dead_pid() -> u32 {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let pid = child.id();
            child.wait().unwrap();
            pid
        }

        /// An RFC 3339 timestamp `secs` in the past — `now()` minus an offset,
        /// so a fixture can model a fresh (active) or aged (abandoned) claim.
        fn claimed_secs_ago(secs: i64) -> String {
            (chrono::Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339()
        }

        #[test]
        fn test_acquire_unowned_lock_succeeds_and_writes_self() {
            let dir = TmpDir::new();
            let lc = checker(&dir, SELF_ID);
            assert!(lc.acquire(PUBKEY), "free lock must be claimable");
            // Read side now sees our own id → leader.
            assert!(lc.is_leader(PUBKEY));
        }

        #[test]
        fn test_acquire_creates_lock_dir_when_absent() {
            let dir = TmpDir::new();
            let nested = dir.0.join("missing-subdir");
            let lc = FileLeaderCheck::new(Some(SELF_ID.into()), nested.clone());
            assert!(lc.acquire(PUBKEY));
            assert!(nested.join(format!("{PUBKEY}.lock")).exists());
        }

        #[test]
        fn test_two_writers_race_exactly_one_wins() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            let b = checker(&dir, OTHER_ID);
            // First claim wins; the second sees a live foreign owner (this
            // very test process is alive) and must observe.
            assert!(a.acquire(PUBKEY), "first writer claims the free lock");
            assert!(!b.acquire(PUBKEY), "second writer must not double-claim");
            assert!(a.is_leader(PUBKEY));
            assert!(!b.is_leader(PUBKEY));
        }

        #[test]
        fn test_dead_pid_lock_is_taken_over() {
            let dir = TmpDir::new();
            dir.write_lock(&format!(
                r#"{{"instance_id":"{OTHER_ID}","pid":{},"claimed_at":"2026-06-15T00:00:00Z"}}"#,
                dead_pid()
            ));
            let lc = checker(&dir, SELF_ID);
            assert!(lc.acquire(PUBKEY), "dead-pid lock must be reclaimable");
            assert!(lc.is_leader(PUBKEY), "takeover rewrites the lock to us");
        }

        #[test]
        fn test_live_foreign_lock_blocks_acquire() {
            let dir = TmpDir::new();
            // Live pid + fresh claim → an active leader; claim must fail.
            dir.write_lock(&format!(
                r#"{{"instance_id":"{OTHER_ID}","pid":{},"claimed_at":"{}"}}"#,
                std::process::id(),
                claimed_secs_ago(0),
            ));
            let lc = checker(&dir, SELF_ID);
            assert!(
                !lc.acquire(PUBKEY),
                "live foreign owner must block the claim"
            );
            assert!(!lc.is_leader(PUBKEY));
        }

        #[test]
        fn test_recycled_pid_with_stale_claim_is_taken_over() {
            // A crashed leader's pid recycled to an unrelated live process:
            // pid probes alive, but the abandoned claim has aged past the
            // staleness bound. Without the bound this wedges leaderless.
            let dir = TmpDir::new();
            dir.write_lock(&format!(
                r#"{{"instance_id":"{OTHER_ID}","pid":{},"claimed_at":"{}"}}"#,
                std::process::id(), // a guaranteed-live pid stands in for the recycled one
                claimed_secs_ago(STALE_CLAIM.as_secs() as i64 + 5),
            ));
            let lc = checker(&dir, SELF_ID);
            assert!(
                lc.acquire(PUBKEY),
                "stale claim on a live (recycled) pid must be reclaimable"
            );
            assert!(lc.is_leader(PUBKEY), "takeover rewrites the lock to us");
        }

        #[test]
        fn test_reacquire_own_lock_is_idempotent() {
            let dir = TmpDir::new();
            let lc = checker(&dir, SELF_ID);
            assert!(lc.acquire(PUBKEY));
            assert!(lc.acquire(PUBKEY), "re-claiming our own lock stays leader");
            assert!(lc.is_leader(PUBKEY));
        }

        #[test]
        fn test_release_frees_lock_for_another_writer() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            let b = checker(&dir, OTHER_ID);
            assert!(a.acquire(PUBKEY));
            assert!(!b.acquire(PUBKEY), "blocked while A holds it");

            a.release(PUBKEY);
            // After release the file is empty (malformed) → free to take.
            assert!(b.acquire(PUBKEY), "released lock must be claimable");
            assert!(b.is_leader(PUBKEY));
            assert!(!a.is_leader(PUBKEY), "A no longer owns the lock");
        }

        #[test]
        fn test_release_does_not_stomp_a_successor() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            // B (a different, live owner with a fresh claim) holds the lock.
            dir.write_lock(&format!(
                r#"{{"instance_id":"{OTHER_ID}","pid":{},"claimed_at":"{}"}}"#,
                std::process::id(),
                claimed_secs_ago(0),
            ));
            // A releasing must NOT clear B's claim (A doesn't own it).
            a.release(PUBKEY);
            assert!(
                !FileLeaderCheck::new(Some(SELF_ID.into()), dir.0.clone()).acquire(PUBKEY),
                "B's live lock must survive A's release"
            );
        }

        #[test]
        fn test_no_instance_id_acquire_is_noop_always_leader() {
            let dir = TmpDir::new();
            let lc = FileLeaderCheck::new(None, dir.0.clone());
            assert!(lc.acquire(PUBKEY), "no id → always leader, no write");
            assert!(
                !dir.lock_path().exists(),
                "no-id acquire must not write a lock file"
            );
        }

        #[test]
        fn test_concurrent_acquire_yields_exactly_one_winner() {
            // Without the flock guard, two threads could both read the empty
            // lock and both write — two leaders. The exclusive flock forces a
            // serial read-decide-write, so exactly one wins and the rest see a
            // live foreign owner (this same process's pid).
            let dir = std::sync::Arc::new(TmpDir::new());
            let winners = std::sync::Arc::new(AtomicU32::new(0));
            let start = std::sync::Arc::new(std::sync::Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let dir = dir.clone();
                    let winners = winners.clone();
                    let start = start.clone();
                    std::thread::spawn(move || {
                        let lc = FileLeaderCheck::new(Some(format!("id-{i}")), dir.0.clone());
                        start.wait();
                        if lc.acquire(PUBKEY) {
                            winners.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_eq!(
                winners.load(Ordering::Relaxed),
                1,
                "exactly one concurrent writer may win the lock"
            );
        }

        #[test]
        fn test_stand_down_suppresses_acquire() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            let b = checker(&dir, OTHER_ID);
            assert!(a.acquire(PUBKEY), "A claims the free lock");
            // A stands down and releases — simulating cooperative handoff.
            a.stand_down();
            a.release(PUBKEY);
            // A's next tick: acquire returns false (standing down).
            assert!(
                !a.acquire(PUBKEY),
                "standing-down instance must not re-claim"
            );
            // B can now take the free lock.
            assert!(b.acquire(PUBKEY), "B must acquire the released lock");
        }

        #[test]
        fn test_stand_down_auto_expires() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            assert!(a.acquire(PUBKEY));
            a.stand_down();
            a.release(PUBKEY);
            // acquire is suppressed while standing down.
            assert!(!a.acquire(PUBKEY));
            // Wait for the test-configured timeout (5ms) to expire.
            std::thread::sleep(std::time::Duration::from_millis(10));
            // After timeout, stand-down auto-expires and acquire succeeds.
            assert!(
                a.acquire(PUBKEY),
                "acquire must succeed after stand-down timeout expires"
            );
        }

        #[test]
        fn test_cooperative_handoff_transfers_leadership() {
            let dir = TmpDir::new();
            let a = checker(&dir, SELF_ID);
            let b = checker(&dir, OTHER_ID);
            assert!(a.acquire(PUBKEY), "A is leader");
            assert!(!b.acquire(PUBKEY), "B blocked while A holds it");

            // Cooperative handoff: A stands down + releases.
            a.stand_down();
            a.release(PUBKEY);
            // B acquires on its next tick.
            assert!(b.acquire(PUBKEY), "B must win after A stands down");
            // A's next tick: still standing down, can't re-grab.
            assert!(
                !a.acquire(PUBKEY),
                "A must not re-claim while standing down"
            );
        }

        #[test]
        fn test_instance_id_accessor() {
            let dir = TmpDir::new();
            let with_id = FileLeaderCheck::new(Some("test-id-123".into()), dir.0.clone());
            assert_eq!(with_id.instance_id(), Some("test-id-123"));

            let without_id = FileLeaderCheck::new(None, dir.0.clone());
            assert_eq!(without_id.instance_id(), None);
        }
    }
}
