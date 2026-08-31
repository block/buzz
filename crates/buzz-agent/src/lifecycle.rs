//! Connection owns session creation and prompt tasks, and outlives MCP cleanup.
use std::{sync::Arc, time::Duration};

use crate::{cancel_all_sessions, App};

pub(crate) async fn shutdown(app: &Arc<App>) -> bool {
    let mut tasks = app.tasks.lock().await;
    let drain = async {
        // session/new can complete during teardown. Repeat cancellation so a
        // prompt which had not acquired its session at EOF cannot escape it.
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            cancel_all_sessions(app).await;
            if tasks.is_empty() {
                break;
            }
            tokio::select! {
                result = tasks.join_next() => {
                    if let Some(Err(error)) = result {
                        tracing::warn!(%error, "connection task failed during shutdown");
                    }
                }
                _ = tick.tick() => {}
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(10), drain)
        .await
        .is_err()
    {
        tracing::warn!("connection task drain timed out; teardown is unconfirmed");
        tasks.shutdown().await;
    }
    // No task can still borrow a client or create a new session. Await rmcp's
    // transport close. The client requests explicit supported work completion
    // before closing stdin and inspecting its retained MCP child exit.
    let sessions = std::mem::take(&mut *app.sessions.lock().await);
    let mut closing = tokio::task::JoinSet::new();
    for (_, session) in sessions {
        closing.spawn(async move { session.mcp.shutdown().await });
    }
    let mut joined = true;
    while let Some(result) = closing.join_next().await {
        joined &= result.is_ok();
    }
    joined
        && app
            .unfinished_tasks
            .load(std::sync::atomic::Ordering::Acquire)
            == 0
        && crate::owned_mcp::all_confirmed()
}
