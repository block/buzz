use std::sync::{Mutex, OnceLock};

use tauri::State;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;

use super::media::{process_picked_path, sanitize_filename, BlobDescriptor};

fn active_uploads() -> &'static Mutex<std::collections::HashMap<String, CancellationToken>> {
    static ACTIVE: OnceLock<Mutex<std::collections::HashMap<String, CancellationToken>>> =
        OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

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

/// Append one chunk to a staged upload.
///
/// The chunk rides Tauri's **raw binary IPC** (the request body is an
/// `ArrayBuffer`, not a JSON `number[]`). The old JSON path turned every
/// megabyte into a ~1M-element JS array + `JSON.stringify`/parse on the
/// webview's render thread, which visibly janked typing during an upload.
/// The `upload_id` travels as a request header so the body stays pure bytes.
#[tauri::command]
pub async fn append_staged_media_chunk(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let upload_id = request
        .headers()
        .get("upload-id")
        .and_then(|value| value.to_str().ok())
        .ok_or("missing upload-id header")?;

    let data = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.as_slice(),
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("staged chunk must be sent as raw bytes".to_string())
        }
    };
    if data.is_empty() || data.len() > 1024 * 1024 {
        return Err("upload chunk must contain 1 byte to 1 MiB".to_string());
    }

    let path = staged_upload_path(upload_id)?;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(|e| format!("failed to open staged upload: {e}"))?;
    file.write_all(data)
        .await
        .map_err(|e| format!("failed to write staged upload: {e}"))
}

#[tauri::command]
pub async fn cancel_staged_media_upload(upload_id: String) -> Result<(), String> {
    let path = staged_upload_path(&upload_id)?;
    if let Some(cancel) = active_uploads()
        .lock()
        .map_err(|error| error.to_string())?
        .get(&upload_id)
    {
        cancel.cancel();
    }
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
    let cancel = CancellationToken::new();
    active_uploads()
        .lock()
        .map_err(|error| error.to_string())?
        .insert(upload_id.clone(), cancel.clone());
    let result = process_picked_path(path.clone(), &state, false, progress, Some(cancel)).await;
    active_uploads()
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&upload_id);
    let _ = tokio::fs::remove_file(path).await;
    let mut descriptor = result?;
    descriptor.filename = filename.as_deref().map(sanitize_filename);
    Ok(descriptor)
}
