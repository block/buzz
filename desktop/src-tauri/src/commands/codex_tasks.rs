use crate::managed_agents::CodexTaskSummary;

#[tauri::command]
pub async fn list_codex_tasks() -> Result<Vec<CodexTaskSummary>, String> {
    tokio::task::spawn_blocking(crate::managed_agents::list_codex_tasks)
        .await
        .map_err(|error| format!("spawn_blocking failed: {error}"))?
}
