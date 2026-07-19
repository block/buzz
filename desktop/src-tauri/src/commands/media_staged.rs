use tauri::State;
use tokio::io::AsyncWriteExt;

use crate::app_state::AppState;

use super::media::{process_picked_path, sanitize_filename, BlobDescriptor};

#[tauri::command]
pub async fn begin_staged_media_upload() -> Result<String, String> {
    let upload_id = uuid::Uuid::new_v4().to_string();
    let path = staged_upload_path(&upload_id)?;
    tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .map_err(|e| format!("failed to create staged upload: {e}"))?;
    Ok(upload_id)
}

fn staged_upload_path(upload_id: &str) -> Result<std::path::PathBuf, String> {
    let parsed = uuid::Uuid::parse_str(upload_id).map_err(|_| "invalid upload id".to_string())?;
    Ok(std::env::temp_dir().join(format!("buzz-staged-upload-{parsed}")))
}

#[tauri::command]
pub async fn append_staged_media_chunk(upload_id: String, data: Vec<u8>) -> Result<(), String> {
    if data.is_empty() || data.len() > 1024 * 1024 {
        return Err("upload chunk must contain 1 byte to 1 MiB".to_string());
    }
    let path = staged_upload_path(&upload_id)?;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("failed to open staged upload: {e}"))?;
    file.write_all(&data)
        .await
        .map_err(|e| format!("failed to write staged upload: {e}"))
}

#[tauri::command]
pub async fn cancel_staged_media_upload(upload_id: String) -> Result<(), String> {
    let path = staged_upload_path(&upload_id)?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove staged upload: {error}")),
    }
}

#[tauri::command]
pub async fn finish_staged_media_upload(
    upload_id: String,
    filename: Option<String>,
    progress_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<BlobDescriptor, String> {
    let path = staged_upload_path(&upload_id)?;
    let progress = progress_id.map(|id| (app, id));
    let result = process_picked_path(path.clone(), &state, false, progress).await;
    let _ = tokio::fs::remove_file(path).await;
    let mut descriptor = result?;
    descriptor.filename = filename.as_deref().map(sanitize_filename);
    Ok(descriptor)
}
