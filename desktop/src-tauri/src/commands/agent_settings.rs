use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, find_managed_agent_mut, load_managed_agents, load_personas,
        save_managed_agents, sync_managed_agent_processes, ManagedAgentSummary,
    },
    util::now_iso,
};

#[tauri::command]
pub fn set_managed_agent_start_on_app_launch(
    pubkey: String,
    start_on_app_launch: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    if sync_managed_agent_processes(&mut records, &mut runtimes) {
        save_managed_agents(&app, &records)?;
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
}
