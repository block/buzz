use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::{app_state::AppState, relay::classify_request_error};

static MEDIA_UPLOAD_CANCELLATIONS: LazyLock<Mutex<HashMap<String, CancellationToken>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn begin_media_upload(progress_id: Option<&str>) -> Option<CancellationToken> {
    let progress_id = progress_id?;
    let cancel = CancellationToken::new();
    if let Ok(mut uploads) = MEDIA_UPLOAD_CANCELLATIONS.lock() {
        uploads.insert(progress_id.to_string(), cancel.clone());
    }
    Some(cancel)
}

pub(super) fn cancel_media_upload(progress_id: &str) {
    if let Ok(uploads) = MEDIA_UPLOAD_CANCELLATIONS.lock() {
        if let Some(cancel) = uploads.get(progress_id) {
            cancel.cancel();
        }
    }
}

pub(super) fn finish_media_upload(progress_id: Option<&str>) {
    let Some(progress_id) = progress_id else {
        return;
    };
    if let Ok(mut uploads) = MEDIA_UPLOAD_CANCELLATIONS.lock() {
        uploads.remove(progress_id);
    }
}

pub(super) struct UploadAttempt<'a> {
    pub url: String,
    pub auth_header: &'a str,
    pub mime: &'a str,
    pub sha256: &'a str,
    pub body: bytes::Bytes,
    pub progress: Option<&'a (tauri::AppHandle, String)>,
    pub cancellation: Option<&'a CancellationToken>,
}

pub(super) async fn send_upload_attempt(
    state: &AppState,
    attempt: UploadAttempt<'_>,
) -> Result<reqwest::Response, String> {
    let UploadAttempt {
        url,
        auth_header,
        mime,
        sha256,
        body,
        progress,
        cancellation,
    } = attempt;
    let req = state
        .http_client
        .put(url)
        .header("Authorization", auth_header)
        .header("Content-Type", mime)
        .header("X-SHA-256", sha256);

    let response = if let Some((app, progress_id)) = progress {
        let app = app.clone();
        let progress_id = progress_id.clone();
        let total = body.len() as u64;
        let chunk_size = 64 * 1024;
        let chunk_count = body.len().div_ceil(chunk_size);
        let mut sent: u64 = 0;
        let stream = futures_util::stream::iter((0..chunk_count).map(move |i| {
            let start = i * chunk_size;
            let end = usize::min(start + chunk_size, body.len());
            let chunk = body.slice(start..end);
            sent += chunk.len() as u64;
            let _ = app.emit(
                "media-upload-progress",
                serde_json::json!({ "id": progress_id, "sent": sent, "total": total }),
            );
            Ok::<bytes::Bytes, std::io::Error>(chunk)
        }));
        let request = req
            .header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream))
            .send();
        if let Some(cancellation) = cancellation {
            tokio::select! {
                _ = cancellation.cancelled() => return Err("upload cancelled".to_string()),
                response = request => response,
            }
        } else {
            request.await
        }
    } else {
        let request = req.body(body).send();
        if let Some(cancellation) = cancellation {
            tokio::select! {
                _ = cancellation.cancelled() => return Err("upload cancelled".to_string()),
                response = request => response,
            }
        } else {
            request.await
        }
    };
    response.map_err(|error| classify_request_error(&error))
}

pub(super) fn emit_media_upload_phase(
    app: &tauri::AppHandle,
    progress_id: Option<&str>,
    phase: &'static str,
) {
    let Some(id) = progress_id else {
        return;
    };
    let _ = app.emit(
        "media-upload-phase",
        serde_json::json!({ "id": id, "phase": phase }),
    );
}
