//! Concurrent save-path interleave test for `managed_agents/storage.rs`.
//!
//! A child of the `tests` module (`#[path]`-included from `storage_tests.rs`)
//! so it reuses that module's `FakeCombinedStore` and `record_with_pubkey_and_key`
//! helpers without duplication, while keeping `storage_tests.rs` under the
//! desktop file-size gate.
//!
//! These tests drive the PRODUCTION lock-owning entry points
//! (`save_managed_agents_locked_at`, `save_agent_definitions_locked_at`) — the
//! seams that own `lock -> raw read -> project -> atomic commit` — rather than
//! hand-building a lock span around the lock-free `*_at` cores. That is the
//! point: the exclusion the test observes is the production seam's own
//! acquisition, so deleting `transaction_lock_at` from either wrapper turns the
//! matching assertion red.

use super::super::{save_agent_definitions_locked_at, save_managed_agents_locked_at};
use super::{
    record_with_pubkey_and_key, FakeCombinedStore, KeyStore, KeyringProbe, ManagedAgentRecord,
};
use crate::managed_agents::secret_projection::ProjectionStore;
use std::collections::HashMap;

/// A `ProjectionStore` + `KeyStore` that pauses the production save exactly
/// once, mid-mutation, so an observer can prove the save holds the transaction
/// lock at that instant.
///
/// The pause fires inside `store_batch_verified` — the single blob write every
/// save routes its secret projection through — which the production seam
/// reaches only AFTER acquiring the lock and BEFORE the JSON commit. On the
/// first call it signals `entered` and blocks on `release`; every other
/// operation delegates to an inner `FakeCombinedStore` so the save still reads
/// back and commits normally once released.
struct BarrierStore {
    inner: FakeCombinedStore,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
    tripped: std::cell::Cell<bool>,
}

impl BarrierStore {
    fn new(entered: std::sync::mpsc::Sender<()>, release: std::sync::mpsc::Receiver<()>) -> Self {
        Self {
            inner: FakeCombinedStore::new(),
            entered,
            release,
            tripped: std::cell::Cell::new(false),
        }
    }

    /// Signal + block on the first secret write, once. Runs under the txn lock
    /// the production seam acquired, before the JSON commit.
    fn trip_once(&self) {
        if !self.tripped.replace(true) {
            self.entered.send(()).expect("signal save entered mutation");
            self.release.recv().expect("observer released the save");
        }
    }
}

impl KeyStore for BarrierStore {
    fn probe(&self, name: &str) -> KeyringProbe {
        self.inner.probe(name)
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        self.inner.load(name)
    }
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        self.inner.load_all_readonly()
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        KeyStore::write_and_verify(&self.inner, name, value)
    }
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.inner.store_all(entries)
    }
}

impl ProjectionStore for BarrierStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
        ProjectionStore::write_and_verify(&self.inner, key, value)
    }
    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        self.inner.load_key(key)
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        self.inner.load_all()
    }
    fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.inner.store_batch(entries)
    }
    fn remove_batch(&self, keys: &[&str]) -> Result<(), String> {
        self.inner.remove_batch(keys)
    }
    fn store_batch_verified(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        // Pause here, under the production seam's held lock, so the observer's
        // non-blocking probe reports EWOULDBLOCK. Delegate afterward so the save
        // reads its projection back and commits.
        self.trip_once();
        self.inner.store_batch_verified(entries)
    }
}

/// A non-blocking acquire of the store-dir txn lock on an independent open file
/// description must fail while another holder has it. `true` = excluded
/// (`EWOULDBLOCK`), `false` = acquired (lock NOT held).
fn probe_excluded(lock_dir: &std::path::Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let probe = std::fs::File::open(lock_dir).expect("open store dir for probe");
    let rc = unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        // Acquired: release immediately so we leave no lock behind.
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) };
        return false;
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK),
        "exclusion must report EWOULDBLOCK"
    );
    true
}

/// Concurrent instance-save and definition-save against ONE store dir must lose
/// neither half and re-inline neither half's secrets, and each save must hold
/// the cross-process transaction lock across its own read-modify-write cycle.
///
/// Driven through the production lock-owning seams. Each save is paused
/// mid-mutation by an injected `BarrierStore`; while paused, a non-blocking
/// probe on an independent open file description proves the seam holds the lock
/// at that instant (delete the acquisition and the probe acquires instead —
/// red). After the instance save releases and commits, the definition save runs
/// the same way and its raw instance re-read observes the committed instance
/// rather than the empty seed, so neither half is clobbered or re-inlined.
#[test]
fn production_saves_hold_txn_lock_and_preserve_both_halves_without_reinlining() {
    use crate::secret_store::store_txn_lock_dir;

    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("managed-agents.json");
    std::fs::write(&store_path, b"[]").expect("seed empty store");
    let lock_dir = store_txn_lock_dir(&store_path);

    // ── Instance save: holds the lock across its mutation ──────────────────
    // Instance half: a keyed record with an inline key AND inline env, both of
    // which the save must project into the keyring (never leave in JSON).
    let mut instance = record_with_pubkey_and_key("instance-pub", "nsec1instkey");
    instance.env_vars = [("INSTANCE_SECRET".to_string(), "inst-plaintext".to_string())]
        .into_iter()
        .collect();

    let (inst_entered_tx, inst_entered_rx) = std::sync::mpsc::channel::<()>();
    let (inst_release_tx, inst_release_rx) = std::sync::mpsc::channel::<()>();
    let inst_store_path = store_path.clone();
    let instance_save = std::thread::spawn(move || {
        let store = BarrierStore::new(inst_entered_tx, inst_release_rx);
        save_managed_agents_locked_at(
            &inst_store_path,
            Some(&store),
            std::slice::from_ref(&instance),
        )
        .expect("instance save commits");
    });

    inst_entered_rx
        .recv()
        .expect("instance save reached its mutation under the lock");
    assert!(
        probe_excluded(&lock_dir),
        "the instance save must hold the txn lock across its mutation"
    );
    inst_release_tx.send(()).expect("release instance save");
    instance_save.join().expect("instance save thread");

    // ── Definition save: holds the lock, and preserves the committed instance ─
    let mut definition: ManagedAgentRecord = serde_json::from_str(
        r#"{
            "pubkey": "",
            "name": "def-def-slug",
            "slug": "def-slug",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#,
    )
    .expect("definition record");
    definition.env_vars = [("DEF_SECRET".to_string(), "def-plaintext".to_string())]
        .into_iter()
        .collect();

    let (def_entered_tx, def_entered_rx) = std::sync::mpsc::channel::<()>();
    let (def_release_tx, def_release_rx) = std::sync::mpsc::channel::<()>();
    let def_store_path = store_path.clone();
    let definition_save = std::thread::spawn(move || {
        let store = BarrierStore::new(def_entered_tx, def_release_rx);
        save_agent_definitions_locked_at(
            &def_store_path,
            Some(&store),
            std::slice::from_ref(&definition),
        )
        .expect("definition save commits");
    });

    def_entered_rx
        .recv()
        .expect("definition save reached its mutation under the lock");
    assert!(
        probe_excluded(&lock_dir),
        "the definition save must hold the txn lock across its mutation"
    );
    def_release_tx.send(()).expect("release definition save");
    definition_save.join().expect("definition save thread");

    // ── Neither half lost, neither half re-inlined ─────────────────────────
    let committed = std::fs::read_to_string(&store_path).expect("read committed");
    assert!(
        !committed.contains("inst-plaintext"),
        "the instance env must stay projected, never re-inlined by the definition save"
    );
    assert!(
        !committed.contains("nsec1instkey"),
        "the instance key must stay in the keyring, never re-inlined by the definition save"
    );
    assert!(
        !committed.contains("def-plaintext"),
        "the definition env must be projected, never written inline"
    );

    let records: Vec<ManagedAgentRecord> =
        serde_json::from_str(&committed).expect("parse committed");
    let instance = records
        .iter()
        .find(|r| r.pubkey == "instance-pub")
        .expect("the instance must survive the concurrent definition save");
    assert!(
        instance.env_vars_ref.is_some() && instance.env_vars.is_empty(),
        "the instance env must remain a projected ref with no inline residue"
    );
    assert!(
        instance.private_key_nsec.is_empty(),
        "the instance key must remain projected out of the JSON"
    );
    let definition = records
        .iter()
        .find(|r| r.pubkey.is_empty() && r.slug.as_deref() == Some("def-slug"))
        .expect("the definition must be committed alongside the instance");
    assert!(
        definition.env_vars_ref.is_some() && definition.env_vars.is_empty(),
        "the definition env must be a projected ref with no inline residue"
    );
}
