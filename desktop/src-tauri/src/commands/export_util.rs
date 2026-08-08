use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Show a save-file dialog with a custom filter and return the chosen path,
/// or `None` when the user cancelled. Selection only — no write.
pub async fn pick_save_path(
    app: &AppHandle,
    suggested_filename: &str,
    filter_name: &str,
    extensions: &[&str],
) -> Result<Option<std::path::PathBuf>, String> {
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
        None => return Ok(None),
    };

    let dest = file_path
        .as_path()
        .ok_or_else(|| "Save dialog returned an invalid path".to_string())?;
    Ok(Some(dest.to_path_buf()))
}

/// Show a save-file dialog with a custom filter and write `data` to the chosen
/// path. Returns `Ok(true)` when the file was written, `Ok(false)` when the
/// user cancelled the dialog.
///
/// NOT for secrets: the write is plain `std::fs::write` (no atomic commit, no
/// 0o600). Secret exports go through `pick_save_path` and a dedicated
/// secret-file writer such as `key_backup::write_portable_backup_file`.
pub async fn save_bytes_with_dialog(
    app: &AppHandle,
    suggested_filename: &str,
    filter_name: &str,
    extensions: &[&str],
    data: &[u8],
) -> Result<bool, String> {
    let dest = match pick_save_path(app, suggested_filename, filter_name, extensions).await? {
        Some(p) => p,
        None => return Ok(false),
    };

    std::fs::write(dest, data).map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(true)
}
