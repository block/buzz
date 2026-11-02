//! Recovery of committed artifact work that has no visibility proof.

use std::{sync::Arc, time::Duration};

use crate::state::AppState;

/// Retry pending certifications without a client retry or an enabled read gate.
pub async fn run(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if state
            .shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        match tokio::time::timeout(
            Duration::from_secs(5),
            state.db.recover_artifact_certifications(),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(%error, "artifact certification recovery failed"),
            Err(_) => tracing::warn!("artifact certification recovery reached its time limit"),
        }
    }
}
