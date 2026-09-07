use crate::managed_agents::{
    discover_provider_candidates, probe_provider_info, BackendProviderInfo,
};

#[tauri::command]
pub async fn discover_backend_providers() -> Result<Vec<BackendProviderInfo>, String> {
    tokio::task::spawn_blocking(|| {
        discover_provider_candidates()
            .into_iter()
            .map(|(id, path)| BackendProviderInfo {
                id,
                binary_path: path.display().to_string(),
            })
            .collect()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn probe_backend_provider(binary_path: String) -> Result<serde_json::Value, String> {
    // Validate that the requested path is actually a discovered buzz-backend-* binary.
    // This prevents arbitrary binary execution via a compromised frontend or IPC.
    let candidates = discover_provider_candidates();
    let path = std::path::PathBuf::from(&binary_path);
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("binary not found: {binary_path}: {e}"))?;
    let is_known = candidates
        .iter()
        .any(|(_, p)| p.canonicalize().ok().as_ref() == Some(&canonical));
    if !is_known {
        return Err(format!(
            "binary '{binary_path}' is not a discovered buzz-backend-* provider"
        ));
    }
    tokio::task::spawn_blocking(move || probe_provider_info(&canonical))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
