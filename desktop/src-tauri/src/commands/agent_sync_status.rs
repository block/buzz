use tauri::Manager;

/// Read the current community's bootstrap warning without hiding cached agents
/// (in particular, a tracked child must remain visible and stoppable).
#[tauri::command]
pub fn get_managed_agent_sync_error(app: tauri::AppHandle) -> Result<Option<String>, String> {
    managed_agent_sync_error(&app)
}

pub(crate) fn managed_agent_sync_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<String>, String> {
    let state = app.state::<crate::app_state::AppState>();
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let scope = crate::managed_agents::retention::active_retention_scope(app, &state)?;
    let error = state
        .managed_agent_bootstrap_error
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(error
        .as_ref()
        .filter(|(path, _)| *path == scope.db_path)
        .map(|(_, error)| error.clone()))
}
