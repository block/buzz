//! Recovery of committed artifact work that has no visibility proof.

use std::{sync::Arc, time::Duration};

use crate::state::AppState;

/// Retry pending certifications without a client retry or an enabled read gate.
pub async fn run(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut after = None;
    loop {
        interval.tick().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut progress = false;
        for _ in 0..100 {
            if state
                .shutting_down
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            match state.db.recover_artifact_certifications_after(after).await {
                Ok((completed, next)) => {
                    after = next;
                    progress |= completed > 0;
                    if next.is_none() {
                        if !progress {
                            break;
                        }
                        progress = false;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "artifact certification recovery failed");
                    break;
                }
            }
        }
    }
}
