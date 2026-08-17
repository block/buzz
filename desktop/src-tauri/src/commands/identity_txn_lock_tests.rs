//! W6-A interleave coverage — driven through the PRODUCTION lock-owning seam.
//!
//! The identity-persist entry points (`import_identity`,
//! `persist_current_identity`, and the pairing recover path) all route through
//! `crate::commands::identity::persist_identity_locked`, which acquires the same
//! cross-process secret transaction lock a concurrent agent save/GC holds
//! (`transaction_lock_at(store_txn_lock_dir(store_file))`) and then persists the
//! identity under it. So an identity keyring write can no longer interleave with
//! a projection transaction on the shared `SecretStore` blob.
//!
//! This drives that seam directly instead of hand-building a lock span: the
//! injected store pauses the persist mid-keyring-write, and while it is paused a
//! concurrent agent-save projection span (an independent open file description,
//! exactly what a second process or a parallel save has) must be excluded.
//! Delete the acquisition from `persist_identity_locked` and the probe acquires
//! the lock instead of reporting exclusion — this test goes red.

use crate::app_state::{IdentityKeyStore, IdentityStorage};
use crate::secret_store::KeyringProbe;
use std::collections::HashMap;
use std::sync::mpsc;

/// An `IdentityKeyStore` that pauses the persist exactly once, inside `store`,
/// so an observer can prove `persist_identity_locked` holds the transaction lock
/// across the keyring write. `store` runs after the seam acquires the lock and
/// before it releases, so the pause is squarely inside the protected span.
struct BlockingIdentityStore {
    slot: std::cell::RefCell<HashMap<String, String>>,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    tripped: std::cell::Cell<bool>,
}

impl BlockingIdentityStore {
    fn new(entered: mpsc::Sender<()>, release: mpsc::Receiver<()>) -> Self {
        Self {
            slot: std::cell::RefCell::new(HashMap::new()),
            entered,
            release,
            tripped: std::cell::Cell::new(false),
        }
    }
}

impl IdentityKeyStore for BlockingIdentityStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        KeyringProbe::ReachableButEmpty
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.slot.borrow().get(name).cloned())
    }
    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        // Pause once, under the seam's held lock, so the observer's non-blocking
        // probe reports EWOULDBLOCK. Store afterward so the read-back verify and
        // the rest of the persist succeed (→ SystemKeyring).
        if !self.tripped.replace(true) {
            self.entered.send(()).expect("signal persist entered store");
            self.release.recv().expect("observer released the persist");
        }
        self.slot
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.slot.borrow_mut().remove(name);
        Ok(())
    }
    fn verify_stored(&self, name: &str, expected: &str) -> Result<bool, String> {
        Ok(self.slot.borrow().get(name).map(String::as_str) == Some(expected))
    }
}

/// While `persist_identity_locked` holds the transaction lock across its keyring
/// write, a concurrent agent-save projection span (an independent open file
/// description) must be excluded, and may acquire only after the persist span
/// releases.
///
/// Deterministic by handshake: the persist span holds the lock until the
/// injected store's paused `store` call is explicitly released — no timing
/// sleeps. The probe runs while the persist is paused, so exclusion is the
/// seam's own acquisition.
#[test]
fn test_persist_identity_locked_excludes_concurrent_agent_save_on_shared_txn_lock() {
    use crate::secret_store::store_txn_lock_dir;
    use std::os::unix::io::AsRawFd;

    // A store directory shared by identity and agent secrets — the same file
    // `managed_agents_store_path` yields and the seam resolves its lock from.
    let dir = std::env::temp_dir().join(format!("buzz-w6a-interleave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create store dir");
    let store_file = dir.join("managed-agents.json");
    std::fs::write(&store_file, b"[]").expect("seed store file");
    let data_dir = dir.clone();
    let key_path = dir.join("identity.key");

    // The crux of W6-A: the persist seam derives its lock target from the store
    // FILE via `store_txn_lock_dir`, so identity persist and agent save contend
    // on one directory inode instead of racing on the shared keyring blob.
    let lock_dir = store_txn_lock_dir(&store_file);

    let keys = nostr::Keys::generate();
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();

    // Identity-persist span: run the PRODUCTION seam on a worker; its injected
    // store pauses inside the keyring write, holding the lock until released.
    let persist_lock_dir = lock_dir.clone();
    let persist = std::thread::spawn(move || {
        let store = BlockingIdentityStore::new(entered_tx, release_rx);
        let storage = crate::commands::identity::persist_identity_locked(
            &store,
            &keys,
            &key_path,
            &data_dir,
            &persist_lock_dir,
        )
        .expect("identity persist under the txn lock");
        assert_eq!(
            storage,
            IdentityStorage::SystemKeyring,
            "the blocking store persists successfully to the keyring"
        );
    });

    entered_rx
        .recv()
        .expect("persist reached its keyring write under the lock");

    // Agent-save projection span: a non-blocking acquire on an independent OFD
    // must fail while the persist seam holds the lock.
    let probe = std::fs::File::open(&lock_dir).expect("agent save opens store dir");
    let rc = unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        // Acquired: the seam is NOT holding the lock (regression). Release the
        // persist so the thread can exit, then fail with a clear message.
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) };
        release_tx.send(()).expect("release persist span");
        persist.join().expect("identity persist thread");
        let _ = std::fs::remove_dir_all(&dir);
        panic!("an agent save acquired the txn lock while identity persist should have held it");
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK),
        "exclusion must report EWOULDBLOCK"
    );
    drop(probe);

    // Hand back: persist releases, and the save span acquires through the real
    // blocking API only after release.
    release_tx.send(()).expect("release identity persist span");
    persist.join().expect("identity persist thread");

    let save_guard = crate::secret_store::transaction_lock_at(&lock_dir)
        .expect("agent save acquires txn lock after release");
    drop(save_guard);

    let _ = std::fs::remove_dir_all(&dir);
}
