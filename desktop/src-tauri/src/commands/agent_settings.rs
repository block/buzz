use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, current_instance_id, find_managed_agent_mut,
        load_managed_agents, load_personas, save_managed_agents, sync_managed_agent_processes,
        ManagedAgentSummary,
    },
    util::now_iso,
};

#[tauri::command]
pub async fn set_managed_agent_start_on_app_launch(
    pubkey: String,
    start_on_app_launch: bool,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_session_cache(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            record.start_on_app_launch = start_on_app_launch;
            record.updated_at = now_iso();
        }

        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let personas = load_personas(&app).unwrap_or_default();
        build_managed_agent_summary(&app, record, &runtimes, &personas)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn set_managed_agent_auto_restart(
    pubkey: String,
    auto_restart_on_config_change: bool,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_session_cache(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            record.auto_restart_on_config_change = auto_restart_on_config_change;
            record.updated_at = now_iso();
        }

        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let personas = load_personas(&app).unwrap_or_default();
        build_managed_agent_summary(&app, record, &runtimes, &personas)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
