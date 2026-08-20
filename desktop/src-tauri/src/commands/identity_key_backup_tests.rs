use super::{
    create_backup_with_log_n, create_ncryptsec_backup_inner, verify_ncryptsec_backup_inner,
};
use crate::app_state::build_app_state;
use nostr::{Keys, ToBech32};

/// Fast scrypt tier for tests; production uses BACKUP_LOG_N (18), covered
/// once in key_backup_tests::round_trip_at_production_cost.
const FAST_LOG_N: u8 = 16;
const PASSWORD: &str = "correct horse battery";

/// Mirrors `create_ncryptsec_backup`'s caller-side lock precondition. The
/// production command holds `identity_mutation` before it admits a lease and
/// invokes `create_backup_with_log_n`; direct helper tests must exercise the
/// same contract.
fn create_backup(state: &crate::app_state::AppState, password: &str) -> Result<String, String> {
    let _mutation_guard = state.identity_mutation.blocking_lock();
    create_backup_with_log_n(state, password, FAST_LOG_N)
}

#[test]
fn verification_returns_only_public_identity_and_match_status() {
    let state = build_app_state();
    let backup = create_backup(&state, PASSWORD).unwrap();
    let result = verify_ncryptsec_backup_inner(&state, &backup, PASSWORD).unwrap();
    assert_eq!(result.pubkey, state.current_pubkey().unwrap().to_hex());
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
    let err = create_backup(&state, "short").unwrap_err();
    assert!(err.contains("at least"), "{err}");
}

#[test]
fn recovery_mode_blocks_backup_creation() {
    let state = build_app_state();

    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_backup(&state, PASSWORD).is_err(),
        "lost identity must not be backed up"
    );
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Release);

    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(
        create_backup(&state, PASSWORD).is_err(),
        "locked keyring must not be backed up"
    );
}

/// Transition owns `identity_mutation`; a backup that arrives before the drain
/// waits without admitting an egress lease. Once the transition releases the
/// mutation lock, the backup admits and completes, proving the former AB-BA
/// cycle cannot form.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn backup_waits_for_transition_mutation_before_egress_admission() {
    use crate::owner_identity_egress::{
        current_identity_persistence_generation, identity_persistence_state,
        in_flight_egress_leases_for_test, reset_registry_for_test, EGRESS_REGISTRY_TEST_LOCK,
    };
    use tauri::Manager;

    let _egress_guard = EGRESS_REGISTRY_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_registry_for_test();

    let app = tauri::test::mock_builder()
        .manage(build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let app_handle = app.handle().clone();
    let state = app.state::<crate::app_state::AppState>();
    let transition_guard = state.identity_mutation.lock().await;
    let generation_before = current_identity_persistence_generation();

    let backup = tokio::spawn(async move {
        create_ncryptsec_backup_inner(PASSWORD.to_string(), app_handle).await
    });
    tokio::task::yield_now().await;

    assert!(
        !backup.is_finished(),
        "backup must wait for identity_mutation before it can admit an egress lease"
    );
    assert_eq!(
        in_flight_egress_leases_for_test(),
        0,
        "backup blocked on identity_mutation must not admit an egress lease"
    );
    assert_eq!(
        identity_persistence_state(),
        crate::owner_identity_egress::IdentityPersistenceState::Live,
        "queued backup must not begin or obstruct the transition egress drain"
    );
    assert_eq!(
        current_identity_persistence_generation(),
        generation_before,
        "queued backup has not admitted or changed the persistence generation"
    );

    // Model the transition reaching its drain barrier while it still owns the
    // mutation lock. The blocked backup contributed no lease, so the barrier
    // completes rather than forming the former AB-BA wait cycle.
    let generation_after_drain = crate::owner_identity_egress::begin_egress_drain()
        .expect("transition begins its egress drain after the backup queues");
    assert!(generation_after_drain > generation_before);
    crate::owner_identity_egress::wait_egress_drain_blocking();
    assert_eq!(
        in_flight_egress_leases_for_test(),
        0,
        "transition drain does not wait on the backup blocked at identity_mutation"
    );
    crate::owner_identity_egress::resume_egress_live();

    // The transition's proven exit reopens B-generation admission before
    // releasing `identity_mutation`; now the queued backup can acquire it,
    // admit, and derive under the settled identity.
    drop(transition_guard);
    let backup = backup
        .await
        .expect("backup task must not panic")
        .expect("backup succeeds after the transition releases identity_mutation");
    assert!(backup.value.starts_with("ncryptsec1"));

    reset_registry_for_test();
}

/// caller holds `identity_mutation` across the KDF (the `create_backup_with_log_n`
/// precondition, mirroring `create_ncryptsec_backup`), so a swap taking the
/// same lock cannot interleave with the derive.
#[test]
fn concurrent_identity_swap_vs_backup_is_serialized() {
    let state = std::sync::Arc::new(build_app_state());
    let key_a = state.identity_lifecycle_keys_guard().unwrap().clone();
    let key_b = Keys::generate();

    let swapper = {
        let state = state.clone();
        let key_b = key_b.clone();
        std::thread::spawn(move || {
            // Mirrors import_identity's locking: mutation guard held
            // across the key swap.
            let _guard = state.identity_mutation.blocking_lock();
            *state.identity_lifecycle_keys_guard().unwrap() = key_b;
        })
    };

    // The backup caller holds identity_mutation across the derive, so the swap
    // above either fully precedes or fully follows it — never mid-KDF.
    let backup = {
        let _guard = state.identity_mutation.blocking_lock();
        create_backup_with_log_n(&state, PASSWORD, FAST_LOG_N).unwrap()
    };
    swapper.join().unwrap();

    let recovered = crate::key_backup::decrypt_ncryptsec(&backup, PASSWORD)
        .unwrap()
        .public_key();
    assert!(
        recovered == key_a.public_key() || recovered == key_b.public_key(),
        "backup must match one coherent identity"
    );
}
