use super::*;

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

// ── B1: read-back corruption ──────────────────────────────────────────────

#[test]
fn persist_identity_to_keyring_readback_corrupt_returns_err() {
    // B1.1: store() succeeds but load() returns a different valid-format value.
    // The read-back verify in persist_identity_to_keyring must detect the
    // mismatch and return Err so the caller knows the key was not durably stored.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let other_keys = Keys::generate();
    let other_nsec = other_keys.secret_key().to_bech32().unwrap();
    let store = FakeIdentityStore::with_readback_corruption(&other_nsec);
    let imported_keys = Keys::generate();

    let result = persist_identity_to_keyring(&store, &imported_keys, &legacy_path, dir.path());

    assert!(
        result.is_err(),
        "must return Err when read-back returns a different value"
    );
    assert!(
        result.unwrap_err().contains("read-back"),
        "error message must mention read-back verify failure"
    );
}

#[test]
fn persist_imported_identity_impl_readback_corrupt_falls_back_to_file() {
    // B1.2: persist_imported_identity_impl with a readback-corrupt store returns
    // Ok and writes identity.key as a fallback, and the file holds the original key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");

    let other_keys = Keys::generate();
    let other_nsec = other_keys.secret_key().to_bech32().unwrap();
    let store = FakeIdentityStore::with_readback_corruption(&other_nsec);
    let imported_keys = Keys::generate();

    let result = persist_imported_identity_impl(&store, &imported_keys, &legacy_path, dir.path());

    assert!(
        result.is_ok(),
        "must return Ok when file fallback succeeds after readback corruption: {:?}",
        result.err()
    );
    assert!(
        legacy_path.exists(),
        "identity.key must be written as fallback"
    );
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&imported_keys, &from_file);
}

// ── B2: corrupt key material recovery ────────────────────────────────────

#[test]
fn reachable_but_empty_corrupt_file_generates_fresh() {
    // B2.1: ReachableButEmpty probe + corrupt identity.key → migrate_identity_file
    // returns Ok(None) for the corrupt file, then generate_and_persist runs and
    // stores a fresh valid key in the keyring. No panic; resolve succeeds.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    std::fs::write(&legacy_path, b"this-is-not-a-valid-nsec").unwrap();

    let store = FakeIdentityStore::reachable_but_empty();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::None);
    // The keyring now holds the fresh key.
    let stored_nsec = store
        .slot
        .borrow()
        .get(IDENTITY_KEY_NAME)
        .cloned()
        .expect("keyring must hold a fresh key after corrupt-file recovery");
    let keyring_keys = Keys::parse(&stored_nsec).expect("keyring value must be a valid nsec");
    assert_key_eq(&resolved.keys, &keyring_keys);
}

#[test]
fn present_corrupt_keyring_and_corrupt_file_generates_fresh() {
    // B2.2: Present probe with a corrupt keyring value AND a corrupt identity.key.
    // recover_from_keyring clears the bad entry, migrate_identity_file returns
    // Ok(None) for the corrupt file, then generate_and_persist stores a fresh key
    // in the keyring. Resolve succeeds; keyring holds the fresh valid key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    std::fs::write(&legacy_path, b"this-is-not-a-valid-nsec").unwrap();

    let store = FakeIdentityStore::present_with("not-a-valid-nsec");
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::None);
    // The corrupt keyring entry was cleared.
    assert!(
        store
            .deleted
            .borrow()
            .contains(&IDENTITY_KEY_NAME.to_string()),
        "corrupt keyring entry must be cleared"
    );
    // Keyring holds the newly generated valid key.
    let stored_nsec = store
        .slot
        .borrow()
        .get(IDENTITY_KEY_NAME)
        .cloned()
        .expect("keyring must hold a fresh key after double-corrupt recovery");
    let keyring_keys = Keys::parse(&stored_nsec).expect("keyring value must be a valid nsec");
    assert_key_eq(&resolved.keys, &keyring_keys);
}

// ── B3: Unreachable probe branches ───────────────────────────────────────

#[test]
fn unreachable_with_valid_file_resolves_to_file_key() {
    // B3.a+b (inputs are indistinguishable at this level): Unreachable + valid
    // identity.key → resolves to the file's key. The keyring is never contacted
    // and the file is kept on disk (no migration when keyring is down).
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_key_eq(&file_keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);
    assert!(
        legacy_path.exists(),
        "identity.key must not be deleted when keyring is unreachable"
    );
    assert!(
        store.slot.borrow().is_empty(),
        "keyring must not be contacted when unreachable"
    );
}

#[test]
fn unreachable_valid_file_with_marker_resolves_to_file_not_locked_recovery() {
    // Unreachable + valid identity.key + marker present → resolves to the file
    // key, NOT KeyringLocked recovery. The locked-recovery branch only fires
    // when the file is ABSENT; a present file is always used as a direct
    // fallback regardless of the marker.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let file_keys = Keys::generate();
    save_key_file(&legacy_path, &file_keys).unwrap();
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_key_eq(&file_keys, &resolved.keys);
    assert_eq!(
        resolved.recovery,
        RecoveryState::None,
        "must not enter locked-recovery when a valid file is present"
    );
}

#[test]
fn unreachable_corrupt_file_generates_fresh() {
    // B3.c: Unreachable + corrupt identity.key → load_file_or_generate quarantines
    // the corrupt file, generates a fresh key, and saves it to identity.key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    std::fs::write(&legacy_path, b"this-is-not-a-valid-nsec").unwrap();
    assert!(!migration_marker_path(dir.path()).exists());

    let store = FakeIdentityStore::unreachable();
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::None);
    // A fresh key was saved to identity.key (quarantine renames the corrupt file).
    assert!(
        legacy_path.exists(),
        "fresh key must be saved to identity.key"
    );
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&resolved.keys, &from_file);
}

// ── B4: marker-write failure variants ────────────────────────────────────

#[test]
fn persist_identity_to_keyring_marker_failure_file_fallback_returns_ok() {
    // B4.1: marker write fails (data_dir is an existing file, so the marker
    // path cannot be created), but the file fallback succeeds — returns Ok and
    // identity.key exists and holds the original key.
    let dir = tempfile::tempdir().unwrap();
    let key_dir = tempfile::tempdir().unwrap();
    let legacy_path = key_dir.path().join("identity.key");
    assert!(!legacy_path.exists());

    // Make data_dir a FILE so marker write fails.
    let data_dir_file = dir.path().join("data_as_file");
    std::fs::write(&data_dir_file, b"not a dir").unwrap();

    let store = FakeIdentityStore::reachable_but_empty();
    let imported_keys = Keys::generate();

    let result = persist_identity_to_keyring(&store, &imported_keys, &legacy_path, &data_dir_file);

    assert!(
        result.is_ok(),
        "must return Ok when file fallback succeeds despite marker failure: {:?}",
        result.err()
    );
    assert!(
        legacy_path.exists(),
        "identity.key must be written as fallback"
    );
    let from_file = load_key_file(&legacy_path).unwrap();
    assert_key_eq(&imported_keys, &from_file);
}

#[test]
fn persist_identity_to_keyring_marker_and_file_failure_returns_err() {
    // B4.2: both marker write and file write fail → must return Err (A2 fix).
    // data_dir is a FILE (marker write fails); legacy_path is in a non-existent
    // subdirectory so AtomicWriteFile::open fails on the file write too.
    let dir = tempfile::tempdir().unwrap();

    let data_dir_file = dir.path().join("data_as_file");
    std::fs::write(&data_dir_file, b"not a dir").unwrap();

    // Parent directory does not exist → file write fails.
    let legacy_path = dir.path().join("nonexistent_subdir").join("identity.key");
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::reachable_but_empty();
    let imported_keys = Keys::generate();

    let result = persist_identity_to_keyring(&store, &imported_keys, &legacy_path, &data_dir_file);

    assert!(
        result.is_err(),
        "must return Err when both marker write and file write fail"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("persisted") || err_msg.contains("marker") || err_msg.contains("file"),
        "error message must describe the dual failure: {err_msg}"
    );
}

#[test]
fn present_keyring_no_file_no_marker_self_heals_marker() {
    // B4.3 / A3 coverage: Present(valid) + no identity.key + no migration marker.
    // After resolve, the marker must exist (self-healed by A3) so a later
    // keyring-Unreachable boot does not treat this as a fresh install.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists());
    assert!(!migration_marker_path(dir.path()).exists());

    let keys = Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let store = FakeIdentityStore::present_with(&nsec);

    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    assert_key_eq(&keys, &resolved.keys);
    assert_eq!(resolved.recovery, RecoveryState::None);
    assert!(
        migration_marker_path(dir.path()).exists(),
        "marker must be self-healed by A3 when Present(valid) + no file + no marker"
    );
}

// ── I1: uncached read-back verify ─────────────────────────────────────────

#[test]
fn verify_fails_store_does_not_write_marker_or_delete_file() {
    // I1: when verify_stored() returns Ok(false) (simulating a backend that
    // stores to a cache but does NOT confirm the OS round-trip),
    // persist_identity_to_keyring must return Err — the durable state is
    // uncertain. The caller must NOT write the migration marker or delete
    // identity.key while the durability of the write is unconfirmed.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    let imported_keys = Keys::generate();
    save_key_file(&legacy_path, &imported_keys).unwrap();

    let store = FakeIdentityStore::with_verify_failing();

    let result = persist_identity_to_keyring(&store, &imported_keys, &legacy_path, dir.path());

    // Must return Err — durability of the write was not confirmed.
    assert!(
        result.is_err(),
        "persist_identity_to_keyring must return Err when verify_stored returns false"
    );
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("read-back"),
        "error must mention read-back verify failure: {err_msg}"
    );

    // No migration marker written — the write was not confirmed durable.
    assert!(
        !migration_marker_path(dir.path()).exists(),
        "migration marker must NOT be written when verify_stored fails"
    );

    // identity.key must still exist — must not be deleted without confirmation.
    assert!(
        legacy_path.exists(),
        "identity.key must NOT be deleted when verify_stored fails"
    );
}

// ── I2: corrupt keyring + marker = Lost recovery ──────────────────────────

#[test]
fn corrupt_keyring_marker_present_no_file_is_lost() {
    // I2: Present(corrupt) + migration marker + no identity.key → the prior
    // identity was migrated into the keyring and is now unrecoverable (corrupt
    // AND no file backup). Must enter Lost recovery, NOT generate a fresh key.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    assert!(!legacy_path.exists());

    let store = FakeIdentityStore::present_with("not-a-valid-nsec");
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // Must enter Lost recovery — a prior identity existed and is now unrecoverable.
    assert_eq!(
        resolved.recovery,
        RecoveryState::Lost,
        "corrupt keyring + marker + no file must return Lost recovery, not a fresh key"
    );

    // No identity.key written — the ephemeral key is in-memory only.
    assert!(!legacy_path.exists());
}

#[test]
fn corrupt_keyring_no_marker_no_file_generates_fresh() {
    // I2 (counter-case): Present(corrupt) + NO marker + no identity.key →
    // genuine first launch with a corrupt keyring, no prior identity to
    // protect. generate_and_persist is still the correct last resort.
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("identity.key");
    assert!(!legacy_path.exists());
    assert!(!migration_marker_path(dir.path()).exists());

    let store = FakeIdentityStore::present_with("not-a-valid-nsec");
    let resolved = resolve_identity_with_store(&store, &legacy_path, dir.path()).unwrap();

    // No lost recovery — this is a fresh machine with no prior identity.
    assert_eq!(
        resolved.recovery,
        RecoveryState::None,
        "corrupt keyring + no marker + no file must generate a fresh key (no prior identity)"
    );

    // A fresh, valid key was stored (keyring or file).
    assert!(
        store.slot.borrow().contains_key(IDENTITY_KEY_NAME) || legacy_path.exists(),
        "a fresh key must be stored in the keyring or the file after generate_and_persist"
    );
}
