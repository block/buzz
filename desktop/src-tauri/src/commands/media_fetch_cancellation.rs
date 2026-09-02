use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
};

use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;
use crate::commands::media::detect_and_validate_mime;
use crate::commands::media_download::{
    fetch_blob_bytes_with_cap, validate_download_url, MAX_DOWNLOAD_BYTES,
};
use crate::relay::relay_api_base_url_with_override;

#[derive(Default)]
struct MediaFetchCancellations {
    tokens: HashMap<String, CancellationToken>,
}

impl MediaFetchCancellations {
    fn begin(&mut self, request_id: &str) -> CancellationToken {
        if let Some(cancel) = self.tokens.get(request_id).cloned() {
            return cancel;
        }
        let cancel = CancellationToken::new();
        self.tokens.insert(request_id.to_string(), cancel.clone());
        cancel
    }

    fn cancel(&mut self, request_id: &str) {
        self.tokens
            .entry(request_id.to_string())
            .or_default()
            .cancel();
    }

    fn finish(&mut self, request_id: &str) {
        self.tokens.remove(request_id);
    }
}

static MEDIA_FETCH_CANCELLATIONS: LazyLock<Mutex<MediaFetchCancellations>> =
    LazyLock::new(|| Mutex::new(MediaFetchCancellations::default()));

pub(super) fn begin_media_fetch(request_id: Option<&str>) -> Option<CancellationToken> {
    let request_id = request_id?;
    MEDIA_FETCH_CANCELLATIONS
        .lock()
        .ok()
        .map(|mut fetches| fetches.begin(request_id))
}

pub(super) fn finish_media_fetch(request_id: Option<&str>) {
    let Some(request_id) = request_id else {
        return;
    };
    if let Ok(mut fetches) = MEDIA_FETCH_CANCELLATIONS.lock() {
        fetches.finish(request_id);
    }
}

/// Cancel a renderer-owned relay media fetch, including an in-flight body.
#[tauri::command]
pub fn cancel_media_fetch(request_id: String) {
    if let Ok(mut fetches) = MEDIA_FETCH_CANCELLATIONS.lock() {
        fetches.cancel(&request_id);
    }
}

/// Release renderer ownership after the fetch promise settles.
#[tauri::command]
pub fn release_media_fetch(request_id: String) {
    finish_media_fetch(Some(&request_id));
}

/// Fetch relay media bytes with renderer-owned cancellation.
#[tauri::command]
pub async fn fetch_media_bytes(
    url: String,
    request_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let cancellation = begin_media_fetch(request_id.as_deref());
    let result = async {
        let relay_base = relay_api_base_url_with_override(&state);
        validate_download_url(&url, &relay_base)?;
        let bytes =
            fetch_blob_bytes_with_cap(&url, &state, MAX_DOWNLOAD_BYTES, cancellation.as_ref())
                .await?;
        detect_and_validate_mime(&bytes)?;
        Ok(tauri::ipc::Response::new(bytes))
    }
    .await;
    finish_media_fetch(request_id.as_deref());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_begin_is_retained() {
        let mut fetches = MediaFetchCancellations::default();
        fetches.cancel("cancel-before-begin");

        let cancellation = fetches.begin("cancel-before-begin");

        assert!(cancellation.is_cancelled());
        fetches.finish("cancel-before-begin");
        assert!(fetches.tokens.is_empty());
    }

    #[test]
    fn cancellation_reaches_active_owner() {
        let mut fetches = MediaFetchCancellations::default();
        let cancellation = fetches.begin("active-fetch");

        fetches.cancel("active-fetch");

        assert!(cancellation.is_cancelled());
        fetches.finish("active-fetch");
        assert!(fetches.tokens.is_empty());
    }
}
