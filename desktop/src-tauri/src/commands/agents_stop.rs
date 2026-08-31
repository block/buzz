//! Ordinary selected-generation Stop, shared with pair controls.
use super::*;

#[tauri::command]
pub async fn stop_managed_agent(
    pubkey: String,
    selected_run_id: Option<String>,
    expected_relay_url: Option<String>,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let relay = expected_relay_url
            .ok_or("Exact Stop unsupported without selected community; refresh runtime status")?;
        crate::managed_agents::stop_managed_agent_runtime(
            pubkey.clone(),
            relay,
            selected_run_id,
            app.clone(),
        )?;
        let state = app.state::<AppState>();
        let records = load_managed_agents(&app)?;
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or("agent not found")?;
        summarize_from_disk(&app, record, &runtimes)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
