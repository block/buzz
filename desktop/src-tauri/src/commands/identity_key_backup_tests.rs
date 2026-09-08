use super::{create_backup_with_log_n, verify_ncryptsec_backup_inner};
use crate::app_state::build_app_state;
use nostr::{Keys, ToBech32};

/// Fast scrypt tier for tests; production uses BACKUP_LOG_N (18), covered
/// once in key_backup_tests::round_trip_at_production_cost.
const FAST_LOG_N: u8 = 16;
const PASSWORD: &str = "correct horse battery";

#[test]
fn verification_returns_only_public_identity_and_match_status() {
    let state = build_app_state();
    let backup = create_backup_with_log_n(&state, PASSWORD, FAST_LOG_N).unwrap();
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
fn verification_accepts_maximum_supported_kdf_cost() {
    let state = build_app_state();
    let backup = crate::key_backup::create_backup_blob(
        &Keys::generate(),
        PASSWORD,
        crate::key_backup::MAX_VERIFY_LOG_N,
    )
    .unwrap();
    verify_ncryptsec_backup_inner(&state, &backup, PASSWORD).unwrap();
}

#[test]
fn verification_rejects_unsupported_kdf_cost_before_decryption() {
    let state = build_app_state();
    let supported =
        crate::key_backup::create_backup_blob(&Keys::generate(), PASSWORD, FAST_LOG_N).unwrap();
    let encrypted = crate::key_backup::parse_ncryptsec(&supported).unwrap();
    let mut payload = encrypted.as_vec();
    payload[1] = crate::key_backup::MAX_VERIFY_LOG_N + 1;
    let unsupported = nostr::nips::nip49::EncryptedSecretKey::from_slice(&payload)
        .unwrap()
        .to_bech32()
        .unwrap();

    let err = verify_ncryptsec_backup_inner(&state, &unsupported, PASSWORD).unwrap_err();
    assert_eq!(
        err,
        format!(
            "unsupported backup KDF cost: log_n {} exceeds maximum {}",
            crate::key_backup::MAX_VERIFY_LOG_N + 1,
            crate::key_backup::MAX_VERIFY_LOG_N
        )
    );
}

#[test]
fn rejects_short_passphrase() {
    let state = build_app_state();
    let err = create_backup_with_log_n(&state, "short", FAST_LOG_N).unwrap_err();
    assert!(err.contains("at least"), "{err}");
}

#[test]
fn recovery_mode_blocks_backup_creation() {
    let state = build_app_state();

    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_backup_with_log_n(&state, PASSWORD, FAST_LOG_N).is_err(),
        "lost identity must not be backed up"
    );
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Release);

    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_backup_with_log_n(&state, PASSWORD, FAST_LOG_N).is_err(),
        "locked keyring must not be backed up"
    );
}

/// Concurrent identity changes serialize with backup creation.
#[test]
fn concurrent_identity_swap_vs_backup_is_serialized() {
    let state = std::sync::Arc::new(build_app_state());
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

    let backup = create_backup_with_log_n(&state, PASSWORD, FAST_LOG_N).unwrap();
    swapper.join().unwrap();

    let recovered = crate::key_backup::decrypt_ncryptsec(&backup, PASSWORD)
        .unwrap()
        .public_key();
    assert!(
        recovered == key_a.public_key() || recovered == key_b.public_key(),
        "backup must match one coherent identity"
    );
}
