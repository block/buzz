use crate::managed_agents::{
    CodexSharedRuntimeStatus, CodexSshConnectRequest, CodexSshRuntimeStatus,
    CodexSshTaskQueryRequest, CodexTaskHistory, CodexTaskSummary,
};
use tauri::AppHandle;

#[tauri::command]
pub async fn list_codex_tasks() -> Result<Vec<CodexTaskSummary>, String> {
    tokio::task::spawn_blocking(crate::managed_agents::list_codex_tasks)
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[tauri::command]
pub async fn get_codex_task_history(
    app: AppHandle,
    agent_pubkey: String,
) -> Result<CodexTaskHistory, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::get_codex_task_history(&app, &agent_pubkey)
    })
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

#[tauri::command]
pub async fn take_over_codex_desktop_shared(
    app: AppHandle,
    confirmed: bool,
) -> Result<CodexSharedRuntimeStatus, String> {
    crate::managed_agents::take_over_codex_desktop_shared(&app, confirmed).await
}

#[tauri::command]
pub async fn connect_codex_ssh(
    request: CodexSshConnectRequest,
) -> Result<CodexSshRuntimeStatus, String> {
    tokio::task::spawn_blocking(move || crate::managed_agents::connect(request))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[tauri::command]
pub async fn stop_codex_ssh(host: String, username: String, port: u16) -> Result<(), String> {
    let key = format!("{}@{}:{}", username.trim(), host.trim(), port);
    tokio::task::spawn_blocking(move || crate::managed_agents::stop(&key))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[tauri::command]
pub async fn list_codex_ssh_tasks(
    request: CodexSshTaskQueryRequest,
) -> Result<Vec<CodexTaskSummary>, String> {
    tokio::task::spawn_blocking(move || crate::managed_agents::list_codex_ssh_tasks(request))
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}
