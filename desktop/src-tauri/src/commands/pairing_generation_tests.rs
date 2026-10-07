use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::{
    clear_pairing_session_if_current, commit_recovery_if_current, invalidate_pairing_generation,
    recovery_result_after_completion, validate_recovery_payload_type, PairingHandle,
    PairingSession, PayloadType,
};

#[tokio::test]
async fn overlapping_starts_are_serialized() {
    let pairing = Arc::new(PairingHandle::new());
    let first_pairing = Arc::clone(&pairing);
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let first = tokio::spawn(async move {
        let _guard = first_pairing.start_lock.lock().await;
        locked_tx.send(()).expect("signal acquired start lock");
        tokio::time::sleep(Duration::from_millis(50)).await;
    });

    locked_rx.await.expect("first start acquired lock");
    assert!(pairing.start_lock.try_lock().is_err());
    first.await.expect("first start task");
    assert!(pairing.start_lock.try_lock().is_ok());
}

#[test]
fn recovery_rejects_non_nsec_payloads() {
    assert!(validate_recovery_payload_type(PayloadType::Nsec).is_ok());
    assert_eq!(
        validate_recovery_payload_type(PayloadType::Custom).unwrap_err(),
        "Mobile device sent an unsupported recovery payload"
    );
}

#[test]
fn superseded_recovery_cannot_commit_identity() {
    let generation = AtomicU64::new(2);
    let committed = std::sync::atomic::AtomicBool::new(false);

    let generation_fence = std::sync::Mutex::new(());
    let result = commit_recovery_if_current(&generation, &generation_fence, 1, || {
        committed.store(true, Ordering::SeqCst);
        Ok(())
    });

    assert_eq!(
        result.unwrap_err(),
        "Pairing session was superseded or cancelled"
    );
    assert!(!committed.load(Ordering::SeqCst));
}

#[test]
fn invalidation_after_check_waits_for_identity_commit() {
    let generation = Arc::new(AtomicU64::new(7));
    let generation_fence = Arc::new(std::sync::Mutex::new(()));
    let (checked_tx, checked_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let committed = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let recovery_generation = Arc::clone(&generation);
    let recovery_fence = Arc::clone(&generation_fence);
    let recovery_committed = Arc::clone(&committed);
    let recovery = std::thread::spawn(move || {
        commit_recovery_if_current(&recovery_generation, &recovery_fence, 7, || {
            checked_tx.send(()).expect("signal generation checked");
            finish_rx.recv().expect("release identity commit");
            recovery_committed.store(true, Ordering::SeqCst);
            Ok(())
        })
    });

    checked_rx.recv().expect("generation checked");
    let invalidation_generation = Arc::clone(&generation);
    let invalidation_fence = Arc::clone(&generation_fence);
    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let (invalidated_tx, invalidated_rx) = std::sync::mpsc::channel();
    let invalidation = std::thread::spawn(move || {
        attempted_tx.send(()).expect("signal invalidation attempt");
        let next = invalidate_pairing_generation(&invalidation_generation, &invalidation_fence)
            .expect("invalidate generation");
        invalidated_tx.send(next).expect("signal invalidated");
    });

    attempted_rx.recv().expect("invalidation attempted");
    assert!(invalidated_rx
        .recv_timeout(Duration::from_millis(50))
        .is_err());
    assert!(!committed.load(Ordering::SeqCst));

    finish_tx.send(()).expect("finish identity commit");
    recovery.join().expect("recovery task").unwrap();
    assert!(committed.load(Ordering::SeqCst));
    assert_eq!(invalidated_rx.recv().expect("invalidation completed"), 8);
    invalidation.join().expect("invalidation task");
}

#[test]
fn completion_publish_failure_does_not_undo_successful_import() {
    assert!(recovery_result_after_completion(Ok(()), Err("socket closed".into())).is_ok());
}

#[tokio::test]
async fn stale_task_does_not_clear_replacement_session() {
    let (initial, _) = PairingSession::new_source("ws://initial.example".to_string());
    let session = Arc::new(tokio::sync::Mutex::new(Some(initial)));
    let generation = AtomicU64::new(1);

    generation.store(2, Ordering::SeqCst);
    let (replacement, _) = PairingSession::new_source("ws://replacement.example".to_string());
    *session.lock().await = Some(replacement);

    clear_pairing_session_if_current(&session, &generation, 1).await;

    assert!(session.lock().await.is_some());
}

#[tokio::test]
async fn current_task_clears_its_session() {
    let (active, _) = PairingSession::new_source("ws://active.example".to_string());
    let session = Arc::new(tokio::sync::Mutex::new(Some(active)));
    let generation = AtomicU64::new(3);

    clear_pairing_session_if_current(&session, &generation, 3).await;

    assert!(session.lock().await.is_none());
}
