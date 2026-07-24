use tauri::State;

use crate::app_state::AppState;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn mesh_start_node(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
    _request: serde_json::Value,
) -> CmdResult<serde_json::Value> {
    Err("mesh-llm feature not enabled".to_string())
}

#[tauri::command]
pub async fn mesh_stop_node(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
) -> CmdResult<serde_json::Value> {
    Err("mesh-llm feature not enabled".to_string())
}

#[tauri::command]
pub async fn mesh_node_status(_state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    Err("mesh-llm feature not enabled".to_string())
}

#[tauri::command]
pub async fn mesh_installed_models(
    _state: State<'_, AppState>,
) -> CmdResult<Vec<serde_json::Value>> {
    Err("mesh-llm feature not enabled".to_string())
}

#[tauri::command]
pub async fn mesh_model_catalog() -> CmdResult<serde_json::Value> {
    Err("mesh-llm feature not enabled".to_string())
}

#[tauri::command]
pub async fn mesh_availability(_state: State<'_, AppState>) -> CmdResult<serde_json::Value> {
    Err("mesh-llm feature not enabled".to_string())
}
