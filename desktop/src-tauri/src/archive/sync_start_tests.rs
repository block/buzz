use super::*;
use crate::builderlab::session::SessionValidity;

/// Model the exact immutable auth lifetime used by remote signers. Canceling
/// generation 1 and creating generation 2 is same-pubkey logout/relogin; no
/// HTTP mock or replacement credential lookup can mask the stale capability.
#[tokio::test]
async fn delayed_ownership_rejects_logged_out_generation_before_acquisition() {
    let sync = Arc::new(ArchiveSyncState::default());
    let gate = sync.latest.lock().await;
    let old = SessionValidity {
        generation: 1,
        cancel: CancellationToken::new(),
        expires: chrono::Utc::now() + chrono::Duration::hours(1),
    };
    let cancel = CancellationToken::new();
    let (waiting_tx, waiting_rx) = tokio::sync::oneshot::channel();
    let task = {
        let sync = sync.clone();
        let old = old.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            old.check().unwrap(); // command entry passes
            waiting_tx.send(()).unwrap();
            let mut ownership = sync
                .begin_generation(
                    (1, 1),
                    ("same-owner".into(), "wss://relay".into()),
                    Some(old.generation),
                    cancel,
                    Arc::new(Notify::new()),
                )
                .await
                .unwrap();
            ownership.cleanup_uninstalled = true;
            let acquired = std::sync::atomic::AtomicBool::new(false);
            let result = ownership.validate_install(|| old.check()).map(|_| {
                acquired.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            assert!(result.is_err());
            assert!(!acquired.load(std::sync::atomic::Ordering::SeqCst));
        })
    };
    waiting_rx.await.unwrap();
    old.cancel.cancel();
    let replacement = SessionValidity {
        generation: 2,
        cancel: CancellationToken::new(),
        expires: old.expires,
    };
    drop(gate);
    task.await.unwrap();
    assert!(cancel.is_cancelled());
    assert!(sync.running.lock().await.is_none());
    replacement.check().unwrap();
    let next_cancel = CancellationToken::new();
    let mut ownership = sync
        .begin_generation(
            (2, 1),
            ("same-owner".into(), "wss://relay".into()),
            Some(replacement.generation),
            next_cancel.clone(),
            Arc::new(Notify::new()),
        )
        .await
        .unwrap();
    ownership.cleanup_uninstalled = true;
    ownership.validate_install(|| replacement.check()).unwrap();
    drop(ownership);
    assert!(next_cancel.is_cancelled());
}

#[tokio::test]
async fn failed_or_abandoned_install_revokes_admission_and_clears_slot() {
    let sync = ArchiveSyncState::default();
    let cancel = CancellationToken::new();
    let mut ownership = sync
        .begin_generation(
            (1, 1),
            ("owner".into(), "relay".into()),
            None,
            cancel.clone(),
            Arc::new(Notify::new()),
        )
        .await
        .unwrap();
    ownership.cleanup_uninstalled = true;
    assert!(ownership
        .validate_install(|| Err::<(), _>("capture failed".into()))
        .is_err());
    drop(ownership);
    assert!(cancel.is_cancelled());
    assert!(sync.running.lock().await.is_none());
}
