//! Tauri commands for the nono agent sandbox settings.
//!
//! The on/off setting is written by `set_agent_sandbox_enabled` (in
//! `global_agent_config`, which owns the restart path). These commands report
//! whether sandboxed launches can work and manage the user-owned profile.

use serde::Serialize;
use tauri::AppHandle;

use crate::managed_agents::{load_global_agent_config, sandbox};

/// What the settings UI shows about the agent sandbox.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSandboxStatus {
    /// The platform supports the sandbox (macOS and Linux).
    pub supported: bool,
    /// The user setting: agents launch sandboxed.
    pub enabled: bool,
    /// Why sandboxed agents cannot start right now (missing sidecar, invalid
    /// profile, …). `None` when enabled launches will work, or when disabled.
    pub problem: Option<String>,
    /// The user-editable profile file.
    pub profile_path: Option<String>,
}

/// Report the sandbox setting and run the same checks a sandboxed spawn runs.
#[tauri::command]
pub async fn get_agent_sandbox_status(app: AppHandle) -> Result<AgentSandboxStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let global = load_global_agent_config(&app)?;
        let enabled = sandbox::sandbox_requested(&global);
        let problem = sandbox::prepare_agent_sandbox(&global).err();
        Ok(AgentSandboxStatus {
            supported: cfg!(unix),
            enabled,
            problem,
            profile_path: local_profile_path()
                .ok()
                .map(|path| path.display().to_string()),
        })
    })
    .await
    .map_err(|error| format!("agent sandbox status task failed: {error}"))?
}

/// Open the user-editable sandbox profile in the default app for JSON files.
#[tauri::command]
pub async fn open_agent_sandbox_profile(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = tauri::async_runtime::spawn_blocking(local_profile_path)
        .await
        .map_err(|error| format!("agent sandbox profile task failed: {error}"))??;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| format!("open agent sandbox profile: {error}"))
}

/// Restore the user-editable sandbox profile to its default. Running agents
/// keep their sandbox until restarted.
#[tauri::command]
pub async fn reset_agent_sandbox_profile() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(reset_local_profile)
        .await
        .map_err(|error| format!("agent sandbox profile task failed: {error}"))?
        .map(|_| ())
}

#[cfg(unix)]
fn local_profile_path() -> Result<std::path::PathBuf, String> {
    crate::managed_agents::sandbox_profile::ensure_local_profile_path()
}

#[cfg(unix)]
fn reset_local_profile() -> Result<std::path::PathBuf, String> {
    crate::managed_agents::sandbox_profile::reset_local_profile()
}

#[cfg(not(unix))]
fn local_profile_path() -> Result<std::path::PathBuf, String> {
    Err("the agent sandbox is not supported on this platform".to_string())
}

#[cfg(not(unix))]
fn reset_local_profile() -> Result<std::path::PathBuf, String> {
    Err("the agent sandbox is not supported on this platform".to_string())
}
