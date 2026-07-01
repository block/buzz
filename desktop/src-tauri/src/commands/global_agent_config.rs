//! Tauri commands for global agent configuration defaults.
//!
//! `get_global_agent_config` / `set_global_agent_config` — simple load/save
//! around the `global_config` module with the standard save-time validation.

use tauri::AppHandle;

use crate::managed_agents::{
    load_global_agent_config, save_global_agent_config, validate_global_config, GlobalAgentConfig,
};

/// Read the current global agent configuration.
///
/// Returns the default (empty) config if `global-agent-config.json` has not
/// been written yet.
#[tauri::command]
pub fn get_global_agent_config(app: AppHandle) -> Result<GlobalAgentConfig, String> {
    load_global_agent_config(&app)
}

/// Validate and persist a new global agent configuration.
///
/// Strips empty env values before writing (empty = "inherit" semantics), then
/// applies standard validation: POSIX key shape, reserved-key reject,
/// derived-provider-model-key reject, NUL/size caps.
#[tauri::command]
pub fn set_global_agent_config(
    config: GlobalAgentConfig,
    app: AppHandle,
) -> Result<GlobalAgentConfig, String> {
    validate_global_config(&config)?;
    save_global_agent_config(&app, &config)?;
    // Re-read from disk so the returned value reflects the strip-on-write pass.
    load_global_agent_config(&app)
}
