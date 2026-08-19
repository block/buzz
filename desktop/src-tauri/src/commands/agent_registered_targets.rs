use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        reject_registered_reference_target, ManagedAgentSummary, UpdateManagedAgentRequest,
        UpdateManagedAgentResponse,
    },
};

/// Validate ownership before dispatching commands whose implementation lives in
/// oversized legacy modules. Registered references and unknown pubkeys fail
/// before any lifecycle, config, or delete side effect.
#[tauri::command]
pub async fn update_managed_agent(
    input: UpdateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateManagedAgentResponse, String> {
    reject_registered_reference_target(&app, &input.pubkey)?;
    super::agent_models::update_managed_agent_unchecked(input, app, state).await
}

#[tauri::command]
pub async fn start_managed_agent(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    reject_registered_reference_target(&app, &pubkey)?;
    super::agents::start_managed_agent_unchecked(pubkey, app, state).await
}

#[tauri::command]
pub async fn stop_managed_agent(
    pubkey: String,
    app: AppHandle,
) -> Result<ManagedAgentSummary, String> {
    reject_registered_reference_target(&app, &pubkey)?;
    super::agents::stop_managed_agent_unchecked(pubkey, app).await
}

#[tauri::command]
pub async fn delete_managed_agent(
    pubkey: String,
    force_remote_delete: Option<bool>,
    app: AppHandle,
) -> Result<(), String> {
    reject_registered_reference_target(&app, &pubkey)?;
    super::agents::delete_managed_agent_unchecked(pubkey, force_remote_delete, app).await
}
