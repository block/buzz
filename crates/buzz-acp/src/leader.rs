//! Client-side leader election (read side).
//!
//! When multiple Buzz instances share the same agent keypair, the relay fans
//! every matching event out to all of them (NIP-01). Without coordination,
//! each instance would prompt its agent and respond — duplicate replies for a
//! single mention. A per-agent-key *leader lock* designates exactly one
//! instance as the active responder; non-leaders still receive and render
//! events but suppress the prompt path (and the pre-dispatch `👀` side-effect).
//!
//! This module owns only the **read** half: given a lock file written by some
//! instance, decide whether *this* process is the leader for a given agent
//! pubkey. Lock acquisition, stealing, and failover live elsewhere (Phase 2).
//!
//! # Lock contract
//!
//! - Lock dir: `~/.buzz/leader-locks/`, one file per agent pubkey:
//!   `<pubkey-hex>.lock`, JSON `{"instance_id","pid","claimed_at"}`.
//! - **Absent** lock file → this process is leader. Single-instance dev is
//!   thereby unaffected: no lock, no suppression.
//! - **Present** → leader iff the lock's `instance_id` equals this process's
//!   own election id ([`ELECTION_ID_ENV`]).
//! - **No instance id** (env unset, e.g. solo CLI use or pre-Phase-2 where
//!   nothing writes the election id) → leader. There is no coordinating
//!   desktop, so there is nothing to defer to.
//! - **Malformed** lock file → fail safe to leader. A corrupt lock must never
//!   silence the only responder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Deserialize;

/// Environment variable carrying this window's leader-election identity.
///
/// Per-window election identity; Phase 2 supplies a process-unique value —
/// the Tauri bundle identifier is NOT sufficient (it collides across
/// same-class windows like DMG + dev, or worktrees whose icon-gen fell back
/// to the shared dev id). Kept behind this constant so Phase 2 can swap the
/// source without touching the gate. Distinct from `BUZZ_MANAGED_AGENT`
/// (reaper identity): reaper identity partitions by app-class, election
/// identity must be unique per window — opposite uniqueness requirements.
const ELECTION_ID_ENV: &str = "BUZZ_INSTANCE_ELECTION_ID";

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

/// On-disk lock file shape. Only `instance_id` is load-bearing for the read
/// side; `pid` and `claimed_at` are written by the (Phase 2) acquire path and
/// ignored here.
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
}

impl FileLeaderCheck {
    /// Build from the ambient environment: election id from
    /// [`ELECTION_ID_ENV`], lock dir under `$HOME/.buzz/leader-locks/`.
    pub fn from_env() -> Self {
        let lock_dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".buzz")
            .join("leader-locks");
        Self::new(std::env::var(ELECTION_ID_ENV).ok(), lock_dir)
    }

    fn new(instance_id: Option<String>, lock_dir: PathBuf) -> Self {
        Self {
            instance_id,
            lock_dir,
            cache: Mutex::new(HashMap::new()),
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
}

impl LeaderCheck for FileLeaderCheck {
    fn is_leader(&self, agent_pubkey_hex: &str) -> bool {
        if let Some(&cached) = self.cache.lock().unwrap().get(agent_pubkey_hex) {
            return cached;
        }
        let status = self.read_status(agent_pubkey_hex);
        self.cache
            .lock()
            .unwrap()
            .insert(agent_pubkey_hex.to_string(), status);
        status
    }

    fn refresh(&self) {
        // Re-read only keys we've already been asked about — those are the
        // agent pubkeys this process actually dispatches for.
        let keys: Vec<String> = self.cache.lock().unwrap().keys().cloned().collect();
        for key in keys {
            let status = self.read_status(&key);
            self.cache.lock().unwrap().insert(key, status);
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
}
