use super::OwnerAuthorizationScope;
use crate::app_state::build_app_state;
use crate::user_operation::UserOperationScope;
use nostr::Keys;
use std::sync::atomic::Ordering;

#[test]
fn same_owner_reinstall_retires_operation_and_proof_scopes() {
    let state = build_app_state();
    let keys = state.local_identity_keys().unwrap();
    let operation = UserOperationScope::capture(&state).unwrap();
    let proof = OwnerAuthorizationScope::capture(&state).unwrap();
    state.replace_local_identity_keys(keys).unwrap();
    assert!(operation.admit(&state).is_err());
    assert!(proof.admit(&state).is_err());
    assert!(UserOperationScope::capture(&state)
        .unwrap()
        .admit(&state)
        .is_ok());
}

#[test]
fn identical_workspace_reapply_preserves_local_captures_but_away_back_retires() {
    let state = build_app_state();
    state
        .install_local_workspace("ws://one.example".into(), None)
        .unwrap();
    let operation = UserOperationScope::capture(&state).unwrap();
    let proof = OwnerAuthorizationScope::capture(&state).unwrap();
    let keys = state.local_identity_keys().unwrap();
    state
        .install_local_workspace("ws://one.example".into(), Some(keys))
        .unwrap();
    assert!(operation.admit(&state).is_ok());
    assert!(proof.admit(&state).is_ok());
    state
        .install_local_workspace("ws://two.example".into(), None)
        .unwrap();
    state
        .install_local_workspace("ws://one.example".into(), None)
        .unwrap();
    assert!(operation.admit(&state).is_err());
    assert!(proof.admit(&state).is_err());
}

#[test]
fn admitted_commit_excludes_identity_replacement_until_guard_is_dropped() {
    let state = build_app_state();
    let operation = UserOperationScope::capture(&state).unwrap();
    let guard = operation.admit(&state).unwrap();
    std::thread::scope(|threads| {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker_state = &state;
        threads.spawn(move || {
            entered_tx.send(()).unwrap();
            worker_state
                .replace_local_identity_keys(Keys::generate())
                .unwrap();
            done_tx.send(()).unwrap();
        });
        entered_rx.recv().unwrap();
        assert!(done_rx.try_recv().is_err());
        // The same lock the production replacement takes is held by admission.
        assert!(state.operation_generation.try_lock().is_err());
        drop(guard);
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    });
    assert!(operation.admit(&state).is_err());
}

#[test]
fn recovery_allows_housekeeping_but_not_owner_proofs() {
    let state = build_app_state();
    state.identity_lost.store(true, Ordering::Release);
    assert!(UserOperationScope::capture(&state)
        .unwrap()
        .admit(&state)
        .is_ok());
    assert!(OwnerAuthorizationScope::capture(&state).is_err());
    assert!(OwnerAuthorizationScope::capture_legacy_repair(&state)
        .unwrap()
        .admit(&state)
        .is_ok());
}
