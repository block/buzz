//! Tauri commands for the global smart thread-participation preference.

use tauri::AppHandle;

use crate::managed_agents::thread_participation::{
    load_thread_participation, save_thread_participation, ThreadParticipationPref,
};

/// Read whether managed agents continue active threads without a fresh @.
#[tauri::command]
pub fn get_thread_participation(app: AppHandle) -> Result<ThreadParticipationPref, String> {
    Ok(load_thread_participation(&app))
}

/// Persist the thread-participation preference.
///
/// Takes effect on the next agent spawn/restart (env is baked at spawn).
#[tauri::command]
pub fn set_thread_participation(
    enabled: bool,
    app: AppHandle,
) -> Result<ThreadParticipationPref, String> {
    let pref = ThreadParticipationPref { enabled };
    save_thread_participation(&app, &pref)?;
    Ok(pref)
}
