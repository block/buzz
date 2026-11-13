use tauri::State;

use crate::app_state::AppState;

type CmdResult<T> = Result<T, String>;

const MESH_FEATURE_DISABLED: &str = "Share Compute is not included in this build (mesh-llm feature off — typical for Linux/Windows release packages; macOS releases enable it).";

/// Build-time probe: Share Compute / mesh-llm is compiled into this binary.
#[tauri::command]
pub fn mesh_feature_enabled() -> bool {
    false
}

#[tauri::command]
pub async fn mesh_start_node(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
    _request: serde_json::Value,
) -> CmdResult<serde_json::Value> {
    Err(MESH_FEATURE_DISABLED.to_string())
}

#[tauri::command]
pub async fn mesh_stop_node(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> CmdResult<serde_json::Value> {
    Err(MESH_FEATURE_DISABLED.to_string())
}

#[tauri::command]
pub async fn mesh_node_status(_state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    Err(MESH_FEATURE_DISABLED.to_string())
}

#[tauri::command]
pub async fn mesh_serving_usage(_state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    Err(MESH_FEATURE_DISABLED.to_string())
}

#[tauri::command]
pub async fn mesh_installed_models(
    _state: State<'_, AppState>,
) -> CmdResult<Vec<serde_json::Value>> {
    Err(MESH_FEATURE_DISABLED.to_string())
}

#[tauri::command]
pub async fn mesh_model_catalog() -> CmdResult<serde_json::Value> {
    Err(MESH_FEATURE_DISABLED.to_string())
}
