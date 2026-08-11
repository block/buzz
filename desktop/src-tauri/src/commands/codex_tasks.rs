use crate::managed_agents::{CodexSharedRuntimeStatus, CodexTaskSummary};
use tauri::AppHandle;

#[tauri::command]
pub async fn list_codex_tasks() -> Result<Vec<CodexTaskSummary>, String> {
    tokio::task::spawn_blocking(crate::managed_agents::list_codex_tasks)
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[tauri::command]
pub async fn get_codex_shared_runtime_status(
    app: AppHandle,
) -> Result<CodexSharedRuntimeStatus, String> {
    crate::managed_agents::codex_shared_runtime_status(&app).await
}

#[tauri::command]
pub async fn enable_codex_shared_runtime(
    app: AppHandle,
) -> Result<CodexSharedRuntimeStatus, String> {
    crate::managed_agents::enable_codex_shared_runtime(&app).await
}

#[tauri::command]
pub async fn launch_codex_desktop_shared() -> Result<(), String> {
    tokio::task::spawn_blocking(crate::managed_agents::launch_codex_desktop_shared)
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}
