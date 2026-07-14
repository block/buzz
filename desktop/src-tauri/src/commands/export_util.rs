use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Show a save-file dialog with a custom filter and write `data` to the chosen
/// path. Returns `Ok(true)` when the file was written, `Ok(false)` when the
/// user cancelled the dialog.
pub async fn save_bytes_with_dialog(
    app: &AppHandle,
    suggested_filename: &str,
    filter_name: &str,
    extensions: &[&str],
    data: &[u8],
) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(filter_name, extensions)
        .set_file_name(suggested_filename)
        .save_file(move |path| {
            let _ = tx.send(path);
        });

    let selected = rx.await.map_err(|_| "dialog cancelled".to_string())?;
    let file_path = match selected {
        Some(p) => p,
        None => return Ok(false),
    };

    let dest = file_path
        .as_path()
        .ok_or_else(|| "Save dialog returned an invalid path".to_string())?;
    std::fs::write(dest, data).map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(true)
}
