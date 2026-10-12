//! Push managed-agent exits to the UI (and so to the web bridge snapshot) without a full
//! `list_managed_agents` poll. Every few seconds this only calls `try_wait` on tracked
//! harness children (no store, keyring or persona reads). `std::process::Child` caches the
//! exit status, so the normal `sync_managed_agent_processes` still sees and records it.
//! On a new exit it emits `agents-data-changed`; the frontend's coalesced refresh re-lists,
//! which records the stop and republishes an urgent agents snapshot to remote clients.

use std::collections::HashSet;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::app_state::AppState;

const POLL: Duration = Duration::from_secs(2);

/// Generations (start nonces) that just exited and weren't reported yet. Updates `seen`
/// and forgets generations that are no longer tracked.
pub(crate) fn newly_exited(
    exited_now: &[String],
    tracked: &[String],
    seen: &mut HashSet<String>,
) -> bool {
    seen.retain(|n| tracked.contains(n));
    let mut fresh = false;
    for n in exited_now {
        if seen.insert(n.clone()) {
            fresh = true;
        }
    }
    fresh
}

pub fn spawn_agent_exit_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            tokio::time::sleep(POLL).await;
            let (exited, tracked) = {
                let state = app.state::<AppState>();
                let Ok(mut runtimes) = state.managed_agent_processes.lock() else {
                    continue;
                };
                let mut exited = Vec::new();
                let mut tracked = Vec::new();
                for rt in runtimes.values_mut() {
                    tracked.push(rt.start_nonce.clone());
                    if !matches!(rt.child.try_wait(), Ok(None)) {
                        exited.push(rt.start_nonce.clone());
                    }
                }
                (exited, tracked)
            };
            if newly_exited(&exited, &tracked, &mut seen) {
                let _ = app.emit("agents-data-changed", ());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_each_exit_once_and_forgets_untracked() {
        let mut seen = HashSet::new();
        let t = vec!["a".to_string(), "b".to_string()];
        assert!(!newly_exited(&[], &t, &mut seen));
        assert!(newly_exited(&["a".into()], &t, &mut seen));
        assert!(!newly_exited(&["a".into()], &t, &mut seen));
        assert!(newly_exited(&["a".into(), "b".into()], &t, &mut seen));
        assert!(!newly_exited(&[], &[], &mut seen));
        assert!(seen.is_empty());
    }
}
