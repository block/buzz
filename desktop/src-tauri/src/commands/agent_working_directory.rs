use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Open the native folder picker for a local managed agent's process working
/// directory. Returns the selected absolute path, or `None` when cancelled.
#[tauri::command]
pub async fn pick_agent_working_directory(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose working folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });

    let selected = rx
        .await
        .map_err(|_| "working-folder dialog closed unexpectedly".to_string())?;
    let Some(path) = selected else {
        return Ok(None);
    };
    let path = path
        .as_path()
        .ok_or_else(|| "Working-folder dialog returned an invalid path".to_string())?;
    crate::managed_agents::validate_working_directory(Some(&path.to_string_lossy()))
}
