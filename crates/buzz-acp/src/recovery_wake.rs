//! Quiet-host recovery wake sources, independent of presence/typing/heartbeat.

use std::time::Instant;
use tokio::sync::mpsc;

use crate::RespawnResult;

pub(super) enum RecoveryWake {
    Respawn(Box<RespawnResult>),
    Retry,
    Maintenance,
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

/// Cancellation-safe: selecting another host event never consumes a respawn.
/// A closed receiver disables only that source, not retry/refill timers.
pub(super) async fn wait(
    respawns: &mut mpsc::Receiver<RespawnResult>,
    retry_at: Option<Instant>,
    maintenance_at: Option<Instant>,
) -> RecoveryWake {
    tokio::select! {
        Some(result) = respawns.recv() => RecoveryWake::Respawn(Box::new(result)),
        _ = sleep_until(retry_at) => RecoveryWake::Retry,
        _ = sleep_until(maintenance_at) => RecoveryWake::Maintenance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn respawn_wakes_without_optional_timers() {
        let (tx, mut rx) = mpsc::channel(1);
        tx.send(RespawnResult {
            index: 0,
            result: Err(anyhow::anyhow!("fixture spawn failure")),
        })
        .await
        .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), wait(&mut rx, None, None))
                .await
                .unwrap(),
            RecoveryWake::Respawn(result) if result.index == 0
        ));
        // Consumed exactly once: neither an empty queue nor a completed spawn
        // produces an immediately ready arm on the next loop iteration.
        assert!(
            timeout(Duration::from_millis(20), wait(&mut rx, None, None))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn closed_respawn_channel_does_not_spin_or_disable_refill() {
        let (tx, mut rx) = mpsc::channel(1);
        drop(tx);
        assert!(
            timeout(Duration::from_millis(20), wait(&mut rx, None, None))
                .await
                .is_err()
        );
        assert!(matches!(
            wait(
                &mut rx,
                None,
                Some(Instant::now() + Duration::from_millis(20))
            )
            .await,
            RecoveryWake::Maintenance
        ));
    }

    #[tokio::test]
    async fn host_shutdown_can_cancel_wait_without_losing_later_respawn() {
        let (tx, mut rx) = mpsc::channel(1);
        let (shutdown, mut shutdown_rx) = tokio::sync::watch::channel(());
        shutdown.send(()).unwrap();
        tokio::select! {
            _ = shutdown_rx.changed() => {}
            _ = wait(&mut rx, None, None) => panic!("unexpected wake"),
        }
        tx.send(RespawnResult {
            index: 0,
            result: Err(anyhow::anyhow!("fixture")),
        })
        .await
        .unwrap();
        assert!(matches!(
            wait(&mut rx, None, None).await,
            RecoveryWake::Respawn(_)
        ));
    }
}
