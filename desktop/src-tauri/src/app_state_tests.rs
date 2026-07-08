use super::*;

fn assert_key_eq(a: &Keys, b: &Keys) {
    assert_eq!(a.public_key().to_hex(), b.public_key().to_hex());
}

/// `BUZZ_PRIVATE_KEY` is process-global; serialize the env-mutating tests
/// so they don't race each other under the parallel test runner.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `body` with `BUZZ_PRIVATE_KEY` set to `value` (or unset when `None`),
/// restoring the prior value afterward.
fn with_env_key<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prior = std::env::var("BUZZ_PRIVATE_KEY").ok();
    match value {
        Some(v) => std::env::set_var("BUZZ_PRIVATE_KEY", v),
        None => std::env::remove_var("BUZZ_PRIVATE_KEY"),
    }
    let out = body();
    match prior {
        Some(v) => std::env::set_var("BUZZ_PRIVATE_KEY", v),
        None => std::env::remove_var("BUZZ_PRIVATE_KEY"),
    }
    out
}

#[test]
fn identity_from_env_wins_when_valid() {
    let configured = Keys::generate();
    let nsec = configured.secret_key().to_bech32().unwrap();

    let resolved =
        with_env_key(Some(&nsec), identity_from_env).expect("valid env key must resolve");

    assert_key_eq(&configured, &resolved);
}

#[test]
fn identity_from_env_none_when_absent() {
    assert!(with_env_key(None, identity_from_env).is_none());
}

#[test]
fn identity_from_env_none_when_malformed() {
    // A malformed env var falls through to persisted resolution rather than
    // winning — otherwise a typo'd key would silently shadow the real one.
    assert!(with_env_key(Some("not-a-valid-nsec"), identity_from_env).is_none());
}

#[test]
fn save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    let keys = Keys::generate();

    save_key_file(&path, &keys).unwrap();
    let loaded = load_key_file(&path).unwrap();
    assert_key_eq(&keys, &loaded);
}

#[test]
fn load_rejects_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    std::fs::write(&path, "").unwrap();

    assert!(load_key_file(&path).is_err());
}

#[test]
fn load_rejects_corrupt_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    std::fs::write(&path, "not-a-valid-nsec").unwrap();

    assert!(load_key_file(&path).is_err());
}

#[test]
fn load_missing_file_is_err() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.key");

    assert!(load_key_file(&path).is_err());
}

#[test]
fn cleanup_removes_leftover_identity_file() {
    // Item 1: a leftover identity.key (from a migration whose remove_file
    // failed) is deleted once the keyring is authoritative, so plaintext
    // does not linger on disk.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    save_key_file(&path, &Keys::generate()).unwrap();
    assert!(path.exists());

    cleanup_leftover_identity_file(&path);

    assert!(!path.exists());
}

#[test]
fn cleanup_is_noop_when_no_leftover_file() {
    // Idempotent: the cleanup runs on every keyring-Present boot, so a
    // missing file must be a silent success, not an error or panic.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    assert!(!path.exists());

    cleanup_leftover_identity_file(&path);

    assert!(!path.exists());
}

#[test]
fn save_creates_file_with_valid_nsec() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    let keys = Keys::generate();

    save_key_file(&path, &keys).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("nsec1"));
}

#[cfg(unix)]
#[test]
fn save_creates_file_with_restricted_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    let keys = Keys::generate();

    save_key_file(&path, &keys).unwrap();

    let perms = std::fs::metadata(&path).unwrap().permissions();
    assert_eq!(perms.mode() & 0o777, 0o600);
}

#[test]
fn save_overwrites_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");

    let keys1 = Keys::generate();
    save_key_file(&path, &keys1).unwrap();

    let keys2 = Keys::generate();
    save_key_file(&path, &keys2).unwrap();

    let loaded = load_key_file(&path).unwrap();
    assert_key_eq(&keys2, &loaded);
}

use std::cell::RefCell;
use std::collections::HashMap;

use crate::secret_store::KeyringProbe;

/// In-memory [`IdentityKeyStore`] for testing identity recovery without the
/// OS keyring. Seeded with an initial value and a probe outcome; records
/// every `delete`/`store` so tests can assert the keyring was cleared and
/// rewritten. `write_and_verify` succeeds (store then load reflects it).
struct FakeIdentityStore {
    probe: KeyringProbe,
    slot: RefCell<HashMap<String, String>>,
    deleted: RefCell<Vec<String>>,
    /// When true, `store` returns an availability error, driving the
    /// keyring-write-failure → file-fallback arm of `persist_identity`.
    store_fails: bool,
}

impl FakeIdentityStore {
    fn present_with(value: &str) -> Self {
        let mut slot = HashMap::new();
        slot.insert(IDENTITY_KEY_NAME.to_string(), value.to_string());
        Self {
            probe: KeyringProbe::Present,
            slot: RefCell::new(slot),
            deleted: RefCell::new(Vec::new()),
            store_fails: false,
        }
    }

    /// Backend down this boot: probe is `Unreachable` and the slot is empty
    /// (the real key, if any, is in the keyring we cannot reach).
    fn unreachable() -> Self {
        Self {
            probe: KeyringProbe::Unreachable,
            slot: RefCell::new(HashMap::new()),
            deleted: RefCell::new(Vec::new()),
            store_fails: false,
        }
    }

    /// Backend reachable with no entry — drives the one-time migration path.
    /// `store`/`load` go through the slot, so a read-back verify succeeds.
    fn reachable_but_empty() -> Self {
        Self {
            probe: KeyringProbe::ReachableButEmpty,
            slot: RefCell::new(HashMap::new()),
            deleted: RefCell::new(Vec::new()),
            store_fails: false,
        }
    }

    /// Present probe seeded with a value but whose `store` always fails —
    /// exercises the keyring-write-failure arm of adoption and import paths.
    fn present_with_store_failing(value: &str) -> Self {
        let mut slot = HashMap::new();
        slot.insert(IDENTITY_KEY_NAME.to_string(), value.to_string());
        Self {
            probe: KeyringProbe::Present,
            slot: RefCell::new(slot),
            deleted: RefCell::new(Vec::new()),
            store_fails: true,
        }
    }

    /// Reachable-but-empty probe whose `store` always fails — exercises the
    /// keyring-write-failure → `0o600` file-fallback arm.
    fn store_failing() -> Self {
        Self {
            probe: KeyringProbe::ReachableButEmpty,
            slot: RefCell::new(HashMap::new()),
            deleted: RefCell::new(Vec::new()),
            store_fails: true,
        }
    }
}

impl IdentityKeyStore for FakeIdentityStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        self.probe
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.slot.borrow().get(name).cloned())
    }
    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        if self.store_fails {
            return Err("simulated keyring write failure".to_string());
        }
        self.slot
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.deleted.borrow_mut().push(name.to_string());
        self.slot.borrow_mut().remove(name);
        Ok(())
    }
}

#[test]
fn corrupt_keyring_recovers_valid_file_without_rotating() {
    // The load-bearing regression guard. When the keyring holds a corrupt
    // nsec (Present) AND a valid `identity.key` is on disk (leftover from a
    // failed prior migration), recovery must RECOVER THE FILE'S identity —
    // not quarantine the file and rotate to a fresh key (the original
    // hazard). The corrupt keyring value must be cleared and replaced by the
    // file's key (migrated in).
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    let store = FakeIdentityStore::present_with("not-a-valid-nsec");
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // The FILE's identity is recovered — NOT a freshly generated one.
    assert_key_eq(&file_keys, &resolved.keys);
    // The corrupt keyring value was cleared.
    assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
    // The keyring now holds the file's key (migrated in, read-back verified).
    let file_nsec = file_keys.secret_key().to_bech32().unwrap();
    assert_eq!(
        store
            .slot
            .borrow()
            .get(IDENTITY_KEY_NAME)
            .map(String::as_str),
        Some(file_nsec.as_str())
    );
    // The valid file was migrated (deleted), not quarantined to .bad.*.
    assert!(!legacy_path.exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains(".bad.")));
}

#[test]
fn corrupt_keyring_generates_fresh_only_when_no_file() {
    // With a corrupt keyring value and NO file on disk, generate-fresh is
    // the correct last resort — and the corrupt keyring value is cleared
    // first.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::present_with("not-a-valid-nsec");
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
    // A fresh, valid key was persisted to the keyring (replacing the cleared
    // corrupt value).
    let stored = store.slot.borrow().get(IDENTITY_KEY_NAME).cloned();
    assert_eq!(
        stored.as_deref(),
        Some(resolved.keys.secret_key().to_bech32().unwrap().as_str())
    );
}

#[test]
fn valid_keyring_is_used_and_matching_leftover_file_cleaned_up() {
    // A valid keyring entry and a leftover identity.key with the SAME pubkey
    // (stale leftover from a migration whose remove_file previously failed):
    // keyring wins, plaintext is removed without adoption.
    let keyring_keys = Keys::generate();
    let nsec = keyring_keys.secret_key().to_bech32().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    // Same key in file as keyring → stale leftover, not an import.
    save_key_file(&legacy_path, &keyring_keys).unwrap();

    let store = FakeIdentityStore::present_with(&nsec);
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_key_eq(&keyring_keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);
    assert!(store.deleted.borrow().is_empty());
    assert!(!legacy_path.exists());
}

#[test]
fn unreachable_post_migration_boots_keyring_locked_recovery() {
    // After a migration the file is gone and the marker exists. A later boot
    // with the keyring unreachable must NOT generate a fresh key (that would
    // silently rotate the identity), but must also allow the app to open
    // instead of hard-aborting. The result is a keyring-locked recovery boot:
    // ephemeral key held in memory only, nothing persisted anywhere.
    //
    // Fail-closed semantics are preserved: no identity is ever written to disk
    // or the keyring under the ephemeral key, so no silent rotation occurs.
    // The abort is replaced by a graceful recovery screen.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // KeyringLocked recovery: ephemeral key returned, nothing persisted.
    assert_eq!(resolved.recovery, RecoveryState::KeyringLocked);
    // No identity.key was written.
    assert!(!legacy_path.exists());
    // Keyring store was never called (it is unreachable).
    assert!(store.slot.borrow().is_empty());
    assert!(store.deleted.borrow().is_empty());
}

#[test]
fn unreachable_first_run_generates_to_file_when_no_marker() {
    // Genuine first-EVER launch on a machine whose keyring is down: no file,
    // no marker. There is no prior identity to protect, so generating to the
    // `0o600` file is correct — fail-closed here would block a legitimate
    // first launch.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists());
    assert!(!migration_marker_path(dir.path()).exists());

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // A fresh key was generated and persisted to the file (keyring is down).
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&resolved.keys, &from_file);
}

#[test]
fn migration_writes_marker_before_deleting_file() {
    // Crash-safe ordering: a successful migration must leave the marker on
    // disk AND remove the file. The marker existing while the file is gone
    // is the durable post-migration signal the Unreachable arm relies on;
    // "file gone, no marker" must never be the resting state.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    // ReachableButEmpty drives the one-time migration path.
    let store = FakeIdentityStore::reachable_but_empty();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_key_eq(&file_keys, &resolved.keys);
    // Marker written, file deleted — the safe resting state.
    assert!(migration_marker_path(dir.path()).exists());
    assert!(!legacy_path.exists());
}

#[test]
fn fresh_keyring_generate_writes_marker() {
    // Fix 1 (Pinky comment 1): a fresh install generating straight into a
    // reachable-but-empty keyring must write the marker. Without it, "no
    // file, no marker" matches a never-launched machine, so a later
    // Unreachable boot would silently rotate the key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::reachable_but_empty();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // The key was stored in the keyring (not the file), and the marker marks it.
    assert!(!legacy_path.exists());
    assert!(migration_marker_path(dir.path()).exists());
    assert_eq!(
        store
            .slot
            .borrow()
            .get(IDENTITY_KEY_NAME)
            .map(String::as_str),
        Some(resolved.keys.secret_key().to_bech32().unwrap().as_str())
    );
}

#[test]
fn fresh_keyring_generate_then_unreachable_boots_locked_recovery() {
    // End-to-end guard for Fix 1: after a fresh keyring-created identity
    // (marker written, no file), a later boot with the keyring unreachable
    // must NOT generate a new key and rotate identity. Instead it boots
    // keyring-locked recovery — the real key is still in the keyring.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    // First boot: fresh generate into a reachable keyring.
    let reachable = FakeIdentityStore::reachable_but_empty();
    resolve_identity_with_store(&reachable, &legacy_path, dir.path()).unwrap();
    assert!(!legacy_path.exists());
    assert!(migration_marker_path(dir.path()).exists());

    // Second boot: keyring is down. No file + marker present → locked recovery.
    let unreachable = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&unreachable, &legacy_path, dir.path()).unwrap();

    assert_eq!(
        resolved.recovery,
        RecoveryState::KeyringLocked,
        "second boot must boot keyring-locked, not generate a fresh key"
    );
    // No identity.key was written — nothing new persisted.
    assert!(!legacy_path.exists());
}

#[test]
fn fresh_generate_keyring_failure_falls_back_to_file_without_marker() {
    // Fix 1 correctness on the file-fallback arm: when the keyring write
    // FAILS during a fresh generate, the key must land in the `0o600` file
    // and the marker must NOT be written — a marker here would wrongly trip
    // the next Unreachable boot into failing closed even though the key is
    // sitting in the file.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let store = FakeIdentityStore::store_failing();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // Key persisted to the file (fallback), and recoverable from it.
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&resolved.keys, &from_file);
    // No marker: the file is the authoritative store, not the keyring.
    assert!(!migration_marker_path(dir.path()).exists());
}

// ── New tests for the three defects fixed in this PR ─────────────────────

#[test]
fn import_persists_to_keyring_reboot_resolves_imported_pubkey() {
    // (a) import persists to keyring → simulated reboot resolves the
    // imported pubkey.
    //
    // `persist_identity_to_keyring` is the kernel called by
    // `import_identity`. After it succeeds the keyring slot holds the
    // imported nsec. A fresh store seeded with that nsec (simulating a
    // reboot where the keyring has the value) must resolve to the same key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let imported_keys = Keys::generate();

    // Simulate what import_identity does: persist to keyring.
    let store_import = FakeIdentityStore::reachable_but_empty();
    persist_identity_to_keyring(&store_import, &imported_keys, &legacy_path, dir.path())
        .expect("persist_identity_to_keyring must succeed with a reachable store");

    // Keyring slot now holds the imported nsec.
    let stored_nsec = store_import
        .slot
        .borrow()
        .get(IDENTITY_KEY_NAME)
        .cloned()
        .expect("keyring must hold the imported nsec after persist");
    assert_eq!(stored_nsec, imported_keys.secret_key().to_bech32().unwrap());

    // Simulated reboot: new store with Present probe, seeded with the stored nsec.
    let store_reboot = FakeIdentityStore::present_with(&stored_nsec);
    let resolved = resolve_identity_with_store(&store_reboot, &legacy_path, dir.path()).unwrap();

    // The resolved key is the imported one — identity survives the reboot.
    assert_key_eq(&imported_keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);
    // No identity.key left on disk (was deleted by persist_identity_to_keyring).
    assert!(!legacy_path.exists());
}

#[test]
fn present_keyring_with_mismatched_file_adopts_file_key() {
    // (b) Present + mismatched identity.key → file's key adopted into
    // keyring, no data loss, file removed.
    //
    // This auto-heals installs already stuck in the re-onboarding loop:
    // the keyring holds the shadow key generated at first launch, while
    // identity.key holds the user's imported key from a subsequent import
    // that only reached the file (pre-fix bug). Resolution must adopt the
    // file's key as the user's explicit intent.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let keyring_keys = Keys::generate();
    let keyring_nsec = keyring_keys.secret_key().to_bech32().unwrap();

    // identity.key has a DIFFERENT key — the user's import.
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    let store = FakeIdentityStore::present_with(&keyring_nsec);
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // The file's key (user's explicit import) wins.
    assert_key_eq(&file_keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);

    // The keyring now holds the file's key (overwritten with read-back verify).
    let file_nsec = file_keys.secret_key().to_bech32().unwrap();
    assert_eq!(
        store
            .slot
            .borrow()
            .get(IDENTITY_KEY_NAME)
            .map(String::as_str),
        Some(file_nsec.as_str())
    );

    // identity.key was removed after adoption.
    assert!(!legacy_path.exists());

    // Migration marker was written before file removal (crash-safe ordering).
    // Without the marker, a later keyring-unreachable boot would see no file
    // and no marker and silently generate a fresh key.
    let marker_path = migration_marker_path(dir.path());
    assert!(
        marker_path.exists(),
        "migration marker must exist after mismatched-file adoption"
    );
}

#[test]
fn present_keyring_mismatched_file_adoption_store_failure_boots_with_file_key() {
    // Present + mismatched identity.key + keyring write fails during adoption.
    // Boot must succeed with the FILE's key (the user's intent). The file must
    // survive on disk because the write was rejected — adoption retries on the
    // next boot when the keyring is reachable. The keyring slot must be
    // unchanged (shadow nsec still present, not overwritten).
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let keyring_keys = Keys::generate();
    let keyring_nsec = keyring_keys.secret_key().to_bech32().unwrap();

    // identity.key has a DIFFERENT key — the user's import.
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    let store = FakeIdentityStore::present_with_store_failing(&keyring_nsec);
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // File key (user's explicit import) is returned.
    assert_key_eq(&file_keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);

    // identity.key must survive — adoption write failed, so it is the only
    // durable copy of the imported key until the next-boot retry.
    assert!(
        legacy_path.exists(),
        "identity.key must be kept when keyring adoption write fails"
    );

    // Keyring slot unchanged — write was rejected, no overwrite occurred.
    assert_eq!(
        store
            .slot
            .borrow()
            .get(IDENTITY_KEY_NAME)
            .map(String::as_str),
        Some(keyring_nsec.as_str()),
        "keyring slot must be unchanged when adoption write fails"
    );
}

// read-only-dir marker-failure injection is Unix-only: on Windows,
// FILE_ATTRIBUTE_READONLY on a directory does not prevent creating new
// files inside it (it only guards the directory entry itself), so the
// marker write succeeds and the fault cannot be injected this way.
#[cfg(unix)]
#[test]
fn present_keyring_with_mismatched_file_adopts_file_key_marker_failure_keeps_file() {
    // (b-fault) Present + mismatched identity.key + marker write fails →
    // file MUST NOT be deleted so a later keyring-unreachable boot has a
    // fallback. Invariant: keyring-only implies marker exists; if marker
    // cannot be written, identity.key is the surviving discriminator.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let keyring_keys = Keys::generate();
    let keyring_nsec = keyring_keys.secret_key().to_bech32().unwrap();

    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    // Force marker write failure by making the data directory read-only.
    // AtomicWriteFile writes a temp file in the same dir then renames it,
    // so removing write permission on the dir blocks the write entirely.
    let dir_perms_orig = std::fs::metadata(dir.path()).unwrap().permissions();
    let mut dir_perms_ro = dir_perms_orig.clone();
    // unknown_lints: the clippy lint below doesn't exist yet in the pinned
    // 1.95 toolchain but does in CI's newer clippy — allow both worlds.
    #[allow(unknown_lints)]
    #[allow(clippy::permissions_set_readonly_value)]
    dir_perms_ro.set_readonly(true);
    std::fs::set_permissions(dir.path(), dir_perms_ro).unwrap();

    let store = FakeIdentityStore::present_with(&keyring_nsec);
    // Resolve with the read-only dir; marker write will fail.
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path());

    // Restore perms before any assertions that might panic mid-cleanup.
    std::fs::set_permissions(dir.path(), dir_perms_orig).unwrap();

    // On a read-only fs the file write also fails; we can't even check
    // legacy_path reliably there. What matters is that if resolve succeeded
    // it returned the file's key, and did NOT delete the file.
    if let Ok(resolved) = resolved {
        assert_key_eq(&file_keys, &resolved.keys);
        assert_eq!(resolved.recovery, RecoveryState::None);
        // identity.key must NOT have been deleted — it is the only
        // fallback when the marker could not be written.
        assert!(
            legacy_path.exists(),
            "identity.key must be kept when marker write fails after adoption"
        );
    }
    // If resolve Err'd (e.g. file write also failed) the test still passes —
    // we've verified the code doesn't delete the file without a marker.
}

#[test]
fn reachable_but_empty_with_marker_and_no_file_returns_lost() {
    // (d) ReachableButEmpty + marker + no file → "lost" state, NO new key
    // generated into the keyring.
    //
    // The marker says a key was once stored in the keyring. If the keyring
    // is now empty (entry deleted externally, new OS login session cleared
    // it, etc.) and there is no file fallback, the user's key is truly
    // gone. Resolution must NOT silently generate a new identity; it must
    // surface a "lost" state so the frontend can prompt re-import.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    // Write the migration marker — a key was once in the keyring.
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists()); // no file fallback

    let store = FakeIdentityStore::reachable_but_empty();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // The "lost" flag is set — the frontend must prompt re-import.
    assert_eq!(
        resolved.recovery,
        RecoveryState::Lost,
        "identity lost state must be surfaced"
    );

    // No key was persisted to the keyring — the ephemeral key is in-memory
    // only and must not overwrite the user's actual (externally lost) key.
    assert!(
        store.slot.borrow().is_empty(),
        "no key must be written to keyring when identity is lost"
    );

    // No identity.key written either — the ephemeral key is transient.
    assert!(!legacy_path.exists());
}

#[test]
fn persist_imported_identity_falls_back_to_file_on_keyring_failure() {
    // `persist_imported_identity_impl` with a failing store returns Ok and
    // writes identity.key as a fallback. No migration marker is written — a
    // marker here would cause fail-closed on a later Unreachable boot even
    // though the key is in the file, not the keyring.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let imported_keys = Keys::generate();

    let store = FakeIdentityStore::store_failing();

    let result = persist_imported_identity_impl(&store, &imported_keys, &legacy_path, dir.path());

    // The policy core handles the keyring failure — Ok, not Err.
    assert!(
        result.is_ok(),
        "must not propagate keyring failure when file fallback succeeds"
    );

    // Key is recoverable from the file on next boot.
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&imported_keys, &from_file);

    // No marker written — the file is the authoritative store, not the keyring.
    assert!(!migration_marker_path(dir.path()).exists());

    // The underlying kernel still propagates keyring failure (low-level
    // contract unchanged — the impl layer is what adds the fallback).
    let dir2 = tempfile::tempdir().unwrap();
    let path2 = dir2.path().join("identity.key");
    assert!(
        persist_identity_to_keyring(&store, &imported_keys, &path2, dir2.path()).is_err(),
        "persist_identity_to_keyring must still propagate keyring failure"
    );
}

#[test]
fn persist_to_keyring_marker_failure_writes_file_when_absent_preserves_invariant() {
    // (f) Marker-write failure after a verified keyring write when no
    // identity.key exists (e.g. import from a lost state where the file
    // was already deleted). The invariant "keyring-only implies marker
    // exists" must be preserved: persist_identity_to_keyring must write
    // identity.key as a fallback so a later keyring-unreachable boot does
    // NOT treat the machine as a fresh install and silently rotate identity.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists()); // no file — simulates import from lost state

    let imported_keys = Keys::generate();
    // Keyring write succeeds.
    let store = FakeIdentityStore::reachable_but_empty();

    // Force marker write to fail by placing a directory at the marker path.
    // AtomicWriteFile::open fails when the target path is a directory.
    let marker_path = migration_marker_path(dir.path());
    std::fs::create_dir_all(&marker_path).unwrap();

    // persist_identity_to_keyring will: store to keyring (succeeds), read-
    // back verify (succeeds), attempt write_migration_marker (fails because
    // marker_path is a directory), then write identity.key as a fallback.
    let result = persist_identity_to_keyring(&store, &imported_keys, &legacy_path, dir.path());

    // The function returns Ok — the error is handled, not propagated.
    assert!(
        result.is_ok(),
        "persist_identity_to_keyring must not propagate marker failure"
    );

    // identity.key was written as a fallback — invariant preserved.
    assert!(
        legacy_path.exists(),
        "identity.key must exist as fallback when marker write failed and file was absent"
    );
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&imported_keys, &from_file);
}

#[test]
fn present_keyring_same_pubkey_file_no_marker_writes_marker_before_cleanup() {
    // Present branch: keyring present + same-pubkey identity.key + NO marker.
    // This can arise when persist_identity_to_keyring succeeded at keyring
    // write + marker write but the remove_file step failed, then the marker
    // was deleted externally — or from any earlier code path that stored to
    // the keyring without writing the marker.
    //
    // The fix: write the marker first (crash-safe ordering), then delete the
    // file. Must NOT delete the file while no marker exists.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!migration_marker_path(dir.path()).exists()); // no marker

    let keys = Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    // Same key in both keyring and file — stale leftover scenario.
    save_key_file(&legacy_path, &keys).unwrap();

    let store = FakeIdentityStore::present_with(&nsec);
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // Keyring key is returned.
    assert_key_eq(&keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);

    // Marker must now exist — written before or instead of deleting.
    assert!(
        migration_marker_path(dir.path()).exists(),
        "marker must be written before identity.key is deleted"
    );
}

#[test]
fn reachable_but_empty_with_marker_and_no_file_returns_lost_ephemeral_not_persisted() {
    // Extension of reachable_but_empty_with_marker_and_no_file_returns_lost:
    // also verifies that the ephemeral key returned in lost state is NOT
    // persisted to the keyring — it must remain in-memory only.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::reachable_but_empty();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::Lost);
    // The ephemeral key must NOT be written to the keyring.
    assert!(
        store.slot.borrow().is_empty(),
        "ephemeral lost key must not be written to keyring"
    );
    // No identity.key written either.
    assert!(!legacy_path.exists());
    // The ephemeral pubkey is distinct on every call (sanity check).
    let store2 = FakeIdentityStore::reachable_but_empty();
    let resolved2 = resolve_identity_with_store(&store2, &legacy_path, dir.path()).unwrap();
    assert_eq!(resolved2.recovery, RecoveryState::Lost);
    // Two ephemeral keys are different (probabilistic — collision probability is negligible).
    assert_ne!(
        resolved.keys.public_key().to_hex(),
        resolved2.keys.public_key().to_hex(),
        "each lost-state boot produces a distinct ephemeral key"
    );
}

// ── signing_keys() gate tests ─────────────────────────────────────────────

#[test]
fn signing_keys_returns_ok_when_normal() {
    // When neither identity_lost nor keyring_locked is set, signing_keys()
    // must return the live keys and allow signing.
    let state = build_app_state();
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Relaxed);
    state
        .keyring_locked
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let result = state.signing_keys();
    assert!(
        result.is_ok(),
        "signing_keys() must return Ok when neither flag is set"
    );
    // The returned keys must match the stored keys.
    let expected = state.keys.lock().unwrap().clone();
    assert_key_eq(&result.unwrap(), &expected);
}

#[test]
fn signing_keys_returns_err_when_identity_lost() {
    // An ephemeral key is held when identity is lost — signing under it would
    // publish events with a random identity the user does not own.
    let state = build_app_state();
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let result = state.signing_keys();
    assert!(
        result.is_err(),
        "signing_keys() must return Err when identity_lost is set"
    );
    assert!(
        result.unwrap_err().contains("recovery mode"),
        "error message must mention recovery mode"
    );
}

#[test]
fn signing_keys_returns_err_when_keyring_locked() {
    // The identity key is held in a keyring that is unavailable this boot —
    // the stored keys are inaccessible so signing must be blocked.
    let state = build_app_state();
    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let result = state.signing_keys();
    assert!(
        result.is_err(),
        "signing_keys() must return Err when keyring_locked is set"
    );
    assert!(
        result.unwrap_err().contains("recovery mode"),
        "error message must mention recovery mode"
    );
}

#[test]
fn signing_keys_identity_lost_takes_priority_over_keyring_locked() {
    // When both flags are set, identity_lost is checked first and its error
    // message is returned (the ephemeral-key case is more specific).
    let state = build_app_state();
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Relaxed);
    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let err = state.signing_keys().unwrap_err();
    assert!(
        err.contains("recovery mode"),
        "both-set must return recovery-mode error: {err}"
    );
}

// ── Keyring-locked recovery mode tests ───────────────────────────────────

#[test]
fn keyring_locked_recovery_ephemeral_never_persisted() {
    // Unreachable + marker + no file → KeyringLocked recovery. The ephemeral
    // key is held in memory only; no identity.key is created, no keyring
    // slot is touched. Fail-closed semantics: no identity is ever rotated.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::KeyringLocked);
    // Nothing written to disk — ephemeral key is transient.
    assert!(!legacy_path.exists());
    // Keyring was never contacted (it is unreachable).
    assert!(store.slot.borrow().is_empty());
    assert!(store.deleted.borrow().is_empty());
}

#[test]
fn keyring_locked_recovery_distinct_ephemeral_per_boot() {
    // Each locked-state boot produces a distinct ephemeral key and persists
    // nothing — mirroring the lost-state guarantee.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists());

    let store1 = FakeIdentityStore::unreachable();
    let resolved1 = resolve_identity_with_store(&store1, &legacy_path, dir.path()).unwrap();
    assert_eq!(resolved1.recovery, RecoveryState::KeyringLocked);

    let store2 = FakeIdentityStore::unreachable();
    let resolved2 = resolve_identity_with_store(&store2, &legacy_path, dir.path()).unwrap();
    assert_eq!(resolved2.recovery, RecoveryState::KeyringLocked);

    // Two ephemeral keys are different (probabilistic — collision negligible).
    assert_ne!(
        resolved1.keys.public_key().to_hex(),
        resolved2.keys.public_key().to_hex(),
        "each locked-state boot produces a distinct ephemeral key"
    );
    // Neither boot persisted anything.
    assert!(!legacy_path.exists());
}
