use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::{app_state::AppState, relay::classify_request_error};

#[derive(Default)]
struct MediaUploadCancellations {
    tokens: HashMap<String, CancellationToken>,
}

impl MediaUploadCancellations {
    fn begin(&mut self, progress_id: &str) -> CancellationToken {
        if let Some(cancel) = self.tokens.get(progress_id).cloned() {
            return cancel;
        }
        let cancel = CancellationToken::new();
        self.tokens.insert(progress_id.to_string(), cancel.clone());
        cancel
    }

    fn cancel(&mut self, progress_id: &str) {
        let cancel = self.tokens.entry(progress_id.to_string()).or_default();
        cancel.cancel();
    }

    fn finish(&mut self, progress_id: &str) {
        self.tokens.remove(progress_id);
    }
}

static MEDIA_UPLOAD_CANCELLATIONS: LazyLock<Mutex<MediaUploadCancellations>> =
    LazyLock::new(|| Mutex::new(MediaUploadCancellations::default()));

pub(super) fn begin_media_upload(progress_id: Option<&str>) -> Option<CancellationToken> {
    let progress_id = progress_id?;
    MEDIA_UPLOAD_CANCELLATIONS
        .lock()
        .ok()
        .map(|mut uploads| uploads.begin(progress_id))
}

pub(super) fn cancel_media_upload(progress_id: &str) {
    if let Ok(mut uploads) = MEDIA_UPLOAD_CANCELLATIONS.lock() {
        uploads.cancel(progress_id);
    }
}

pub(super) fn finish_media_upload(progress_id: Option<&str>) {
    let Some(progress_id) = progress_id else {
        return;
    };
    if let Ok(mut uploads) = MEDIA_UPLOAD_CANCELLATIONS.lock() {
        uploads.finish(progress_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_begin_is_retained() {
        let progress_id = format!("cancel-before-begin-{}", uuid::Uuid::new_v4());

        cancel_media_upload(&progress_id);
        let cancellation = begin_media_upload(Some(&progress_id)).expect("cancellation token");

        assert!(cancellation.is_cancelled());
        finish_media_upload(Some(&progress_id));
    }

    #[test]
    fn cancellation_after_begin_reaches_registered_token() {
        let progress_id = format!("cancel-after-begin-{}", uuid::Uuid::new_v4());
        let cancellation = begin_media_upload(Some(&progress_id)).expect("cancellation token");

        cancel_media_upload(&progress_id);

        assert!(cancellation.is_cancelled());
        finish_media_upload(Some(&progress_id));
    }

    #[test]
    fn late_cancellation_after_native_finish_is_removed_on_release() {
        let mut uploads = MediaUploadCancellations::default();
        let id = "late-cancel";

        uploads.begin(id);
        uploads.finish(id);
        uploads.cancel(id);
        assert!(uploads.tokens.contains_key(id));

        uploads.finish(id);
        assert!(!uploads.tokens.contains_key(id));
    }

    #[test]
    fn repeated_concurrent_cycles_leave_no_registry_entries() {
        let mut uploads = MediaUploadCancellations::default();
        let ids = (0..256)
            .map(|index| format!("cycle-{index}"))
            .collect::<Vec<_>>();

        for id in &ids {
            uploads.begin(id);
        }
        for id in &ids {
            uploads.cancel(id);
        }
        for id in &ids {
            uploads.finish(id);
        }

        assert!(uploads.tokens.is_empty());
    }

    #[test]
    fn dispatched_cancellations_are_not_evicted_before_begin() {
        let mut uploads = MediaUploadCancellations::default();
        let ids = (0..129)
            .map(|index| format!("dispatched-{index}"))
            .collect::<Vec<_>>();

        for id in &ids {
            uploads.cancel(id);
        }

        let oldest = uploads.begin(&ids[0]);
        assert!(oldest.is_cancelled());
        assert_eq!(uploads.tokens.len(), ids.len());

        for id in &ids {
            uploads.finish(id);
        }
        assert!(uploads.tokens.is_empty());
    }
}
