use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, BackendKind},
};

/// Apply an edited provider-backed record to its remote provider. Local records
/// return without action because their next spawn consumes the saved change.
pub(super) async fn apply_update(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
) -> Result<(), String> {
    let provider_deploy = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let BackendKind::Provider { id, config } = &record.backend else {
            return Ok(());
        };
        (
            id.clone(),
            config.clone(),
            record.provider_binary_path.clone(),
            super::agents::build_deploy_payload(app, state, record)?,
        )
    };

    let (provider_id, provider_config, cached_binary_path, agent_json) = provider_deploy;
    super::agents::deploy_to_provider(
        app,
        state,
        pubkey,
        &provider_id,
        &provider_config,
        agent_json,
        cached_binary_path.as_deref(),
    )
    .await
}
