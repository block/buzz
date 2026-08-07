//! Global preference: smart thread participation for managed agents.
//!
//! When enabled (default), buzz-acp continues conversations in active threads
//! without a fresh @mention. Stored under `<app-data>/agents/thread-participation.json`
//! and applied at every spawn via `BUZZ_ACP_THREAD_PARTICIPATION`.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::storage::{atomic_write_json_restricted, managed_agents_base_dir};

const FILE_NAME: &str = "thread-participation.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadParticipationPref {
    /// When true (default), agents auto-continue active threads without @.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for ThreadParticipationPref {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn pref_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(FILE_NAME))
}

/// Load the preference; missing file → enabled (matches buzz-acp default).
pub fn load_thread_participation(app: &AppHandle) -> ThreadParticipationPref {
    let Ok(path) = pref_path(app) else {
        return ThreadParticipationPref::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => ThreadParticipationPref::default(),
    }
}

/// Persist the preference (atomic, restricted perms).
pub fn save_thread_participation(
    app: &AppHandle,
    pref: &ThreadParticipationPref,
) -> Result<(), String> {
    let path = pref_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create agents dir: {e}"))?;
    }
    let payload = serde_json::to_vec_pretty(pref)
        .map_err(|e| format!("serialize thread-participation preference: {e}"))?;
    atomic_write_json_restricted(&path, &payload)
}

/// Apply the preference to a child process environment.
pub fn apply_thread_participation_env(
    command: &mut std::process::Command,
    enabled: bool,
) {
    // Single canonical env var — avoid setting both participation and
    // no-participation flags (clap marks them conflicts_with).
    command.env(
        "BUZZ_ACP_THREAD_PARTICIPATION",
        if enabled { "true" } else { "false" },
    );
    command.env_remove("BUZZ_ACP_NO_THREAD_PARTICIPATION");
}
