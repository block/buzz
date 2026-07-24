use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_managed_agents, managed_agent_log_path, managed_agent_runtime_log_path, read_log_tail,
        workspace_pair_key, BackendKind, ManagedAgentLogResponse,
    },
};

fn select_log_path(pair_log_path: Option<PathBuf>, legacy_log_path: PathBuf) -> PathBuf {
    pair_log_path
        .filter(|path| path.exists())
        .unwrap_or(legacy_log_path)
}

#[tauri::command]
pub async fn get_managed_agent_log(
    pubkey: String,
    line_count: Option<u32>,
    app: AppHandle,
) -> Result<ManagedAgentLogResponse, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        if record.backend != BackendKind::Local {
            return Err("logs are not available for remote agents".to_string());
        }

        let pair_log_path = workspace_pair_key(&app, record)
            .map(|key| managed_agent_runtime_log_path(&app, &key))
            .transpose()?;
        let log_path = select_log_path(pair_log_path, managed_agent_log_path(&app, &pubkey)?);
        Ok(ManagedAgentLogResponse {
            content: read_log_tail(&log_path, line_count.unwrap_or(120) as usize)?,
            log_path: log_path.display().to_string(),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_existing_pair_log_and_falls_back_to_legacy_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        let legacy = dir.path().join("agent.log");
        let pair = dir.path().join("agent-relay.log");

        assert_eq!(select_log_path(Some(pair.clone()), legacy.clone()), legacy);

        std::fs::write(&pair, "pair log").expect("write pair log");
        assert_eq!(select_log_path(Some(pair.clone()), legacy), pair);
    }
}
