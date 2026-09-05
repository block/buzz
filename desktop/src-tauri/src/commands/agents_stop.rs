use super::*;
use tauri::Manager;

/// Blocking Stop seam; ownership of a tracked local pair survives config changes.
pub(super) fn stop_managed_agent_blocking<R: tauri::Runtime>(
    pubkey: &str,
    app: &AppHandle<R>,
) -> Result<ManagedAgentSummary, String> {
    let state = app.state::<AppState>();
    let _transition_guard = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let (sync_changed, exited_pubkeys) =
        sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(app));
    if sync_changed {
        save_managed_agents(app, &records)?;
    }
    for pubkey in &exited_pubkeys {
        state.clear_agent_session_caches(pubkey);
    }

    let resolved_record = {
        let disk_record = find_managed_agent_mut(&mut records, pubkey)?;
        // Tracked process ownership authorizes Stop even when next-spawn
        // config moved to a provider or authority hydration failed.
        let tracked_here = crate::managed_agents::workspace_pair_key(app, disk_record)
            .is_some_and(|key| runtimes.contains_key(&key));
        let mut resolved = if tracked_here {
            disk_record.clone()
        } else {
            crate::managed_agents::private_config_overlay::resolved_local_record(
                &state,
                disk_record,
            )?
        };
        // Remote agents without a tracked local child use !shutdown.
        if !tracked_here && resolved.backend != BackendKind::Local {
            return Err(
                "remote agents are stopped via !shutdown message, not this command".to_string(),
            );
        }
        // Pair-scoped: stops only the active workspace's pair; delete and
        // the config-restart flows still drain every pair.
        stop_managed_agent_workspace_pair(app, &mut resolved, &mut runtimes)?;
        crate::managed_agents::private_config_overlay::copy_lifecycle_state(disk_record, &resolved);
        resolved
    };
    save_managed_agents(app, &records)?;
    // Summarize the relay-resolved record so the response reflects the
    // config this device follows, not raw disk.
    summarize_from_disk(app, &resolved_record, &runtimes)
}

#[cfg(test)]
#[path = "agents_stop/tests.rs"]
mod tests;
