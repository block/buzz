use super::{create_and_persist_backup_with_log_n, verify_ncryptsec_backup_inner};
use crate::app_state::build_app_state;
use nostr::Keys;

/// Fast scrypt tier for tests; production uses BACKUP_LOG_N (18), covered
/// once in key_backup_tests::round_trip_at_production_cost.
const FAST_LOG_N: u8 = 16;
const PASSWORD: &str = "correct horse battery";

#[test]
fn returned_bytes_equal_on_disk_bytes() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();
    let returned =
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();

    let path = crate::key_backup::backup_file_path(dir.path());
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        returned, on_disk,
        "webview must receive the exact persisted bytes"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // And the persisted blob provably recovers the live identity.
    let keys = state.keys.lock().unwrap().clone();
    let recovered = crate::key_backup::decrypt_ncryptsec(&on_disk, PASSWORD).unwrap();
    assert_eq!(recovered.public_key(), keys.public_key());
}

#[test]
fn verification_returns_only_public_identity_and_match_status() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();
    let backup =
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();
    let result = verify_ncryptsec_backup_inner(&state, &backup, PASSWORD).unwrap();
    assert_eq!(
        result.pubkey,
        state.keys.lock().unwrap().public_key().to_hex()
    );
    assert!(result.npub.starts_with("npub1"));
    assert!(result.matches_current_identity);
}

#[test]
fn verification_reports_valid_backup_for_a_different_identity() {
    let state = build_app_state();
    let other = Keys::generate();
    let backup = crate::key_backup::create_backup_blob(&other, PASSWORD, FAST_LOG_N).unwrap();
    let result = verify_ncryptsec_backup_inner(&state, &backup, PASSWORD).unwrap();
    assert_eq!(result.pubkey, other.public_key().to_hex());
    assert!(!result.matches_current_identity);
}

#[test]
fn verification_rejects_wrong_password() {
    let state = build_app_state();
    let backup =
        crate::key_backup::create_backup_blob(&Keys::generate(), PASSWORD, FAST_LOG_N).unwrap();
    assert_eq!(
        verify_ncryptsec_backup_inner(&state, &backup, "wrong password").unwrap_err(),
        "wrong backup password or damaged key backup"
    );
}

#[test]
fn overwrite_replaces_atomically() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();
    let first =
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();
    let second =
        create_and_persist_backup_with_log_n(&state, dir.path(), "another passphrase", FAST_LOG_N)
            .unwrap();
    assert_ne!(first, second, "fresh salt/nonce per action");

    let path = crate::key_backup::backup_file_path(dir.path());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), second);
}

#[test]
fn rejects_short_passphrase() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();
    let err =
        create_and_persist_backup_with_log_n(&state, dir.path(), "short", FAST_LOG_N).unwrap_err();
    assert!(err.contains("at least"), "{err}");
    assert!(!crate::key_backup::backup_file_path(dir.path()).exists());
}

#[test]
fn recovery_mode_blocks_backup_creation() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();

    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).is_err(),
        "lost identity must not be backed up"
    );
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Release);

    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).is_err(),
        "locked keyring must not be backed up"
    );
    assert!(!crate::key_backup::backup_file_path(dir.path()).exists());
}

/// Blocker-1 regression (Wren, implementation review): a failed
/// different-key import must leave BOTH the old in-memory identity and
/// the old canonical backup intact. Persistence runs before cleanup, so
/// an `Err` from persist means nothing was mutated or deleted.
#[test]
fn failed_import_persistence_preserves_old_identity_and_backup() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();
    let old_pubkey = state.keys.lock().unwrap().public_key();

    // A valid canonical backup for the live (old) identity.
    create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();
    let backup_path = crate::key_backup::backup_file_path(dir.path());
    let backup_before = std::fs::read_to_string(&backup_path).unwrap();

    // Different-key import whose durable persistence fails (both
    // keyring and file fallback down).
    let _guard = state.identity_mutation.lock().unwrap();
    let err = super::commit_imported_identity(&state, dir.path(), Keys::generate(), |_| {
        Err("keyring and file both unavailable".to_string())
    })
    .unwrap_err();
    assert!(err.contains("unavailable"), "{err}");

    // Old identity still live; old backup untouched byte-for-byte.
    assert_eq!(state.keys.lock().unwrap().public_key(), old_pubkey);
    assert_eq!(
        std::fs::read_to_string(&backup_path).unwrap(),
        backup_before
    );
    assert!(
        crate::key_backup::decrypt_ncryptsec(&backup_before, PASSWORD)
            .unwrap()
            .public_key()
            == old_pubkey,
        "surviving backup must still recover the still-live identity"
    );
}

/// Successful different-key import removes the previous identity's
/// backup — cleanup runs after the durable commit, not before.
#[test]
fn successful_import_removes_stale_backup_after_commit() {
    let state = build_app_state();
    let dir = tempfile::tempdir().unwrap();

    create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();
    let backup_path = crate::key_backup::backup_file_path(dir.path());
    assert!(backup_path.exists());

    let new_keys = Keys::generate();
    let backup_present_at_persist = std::cell::Cell::new(false);
    let _guard = state.identity_mutation.lock().unwrap();
    let pubkey = super::commit_imported_identity(&state, dir.path(), new_keys.clone(), |_| {
        // Ordering probe: the old backup must still exist while
        // persistence is running (cleanup has not happened yet).
        backup_present_at_persist.set(backup_path.exists());
        Ok(())
    })
    .unwrap();

    assert!(
        backup_present_at_persist.get(),
        "cleanup must not precede persist"
    );
    assert_eq!(pubkey, new_keys.public_key());
    assert_eq!(
        state.keys.lock().unwrap().public_key(),
        new_keys.public_key()
    );
    assert!(
        !backup_path.exists(),
        "stale backup must be removed post-commit"
    );
}

/// Concurrent identity swap vs backup creation: `identity_mutation`
/// serializes both, so every persisted blob decrypts to the identity that
/// was live for the whole of its create operation — never a torn state.
#[test]
fn concurrent_identity_swap_vs_backup_is_serialized() {
    let state = std::sync::Arc::new(build_app_state());
    let dir = tempfile::tempdir().unwrap();
    let key_a = state.keys.lock().unwrap().clone();
    let key_b = Keys::generate();

    let swapper = {
        let state = state.clone();
        let key_b = key_b.clone();
        std::thread::spawn(move || {
            // Mirrors import_identity's locking: mutation guard held
            // across the key swap.
            let _guard = state.identity_mutation.lock().unwrap();
            *state.keys.lock().unwrap() = key_b;
        })
    };

    let backup =
        create_and_persist_backup_with_log_n(&state, dir.path(), PASSWORD, FAST_LOG_N).unwrap();
    swapper.join().unwrap();

    let recovered = crate::key_backup::decrypt_ncryptsec(&backup, PASSWORD)
        .unwrap()
        .public_key();
    assert!(
        recovered == key_a.public_key() || recovered == key_b.public_key(),
        "backup must match one coherent identity"
    );
    // Whichever won, the persisted file equals the returned blob.
    let on_disk = std::fs::read_to_string(crate::key_backup::backup_file_path(dir.path())).unwrap();
    assert_eq!(on_disk, backup);
}
