//! Upload a file to the sender's own Google Drive and hand back a shareable
//! link.
//!
//! Why this exists: Buzz's relay upload path is fragile for large media. Video
//! needs ffmpeg on the sender's machine (not preinstalled on macOS), transcodes
//! locally, then lands on a **hosted** relay whose size caps, retention and
//! version this fork does not control. Drive sidesteps all of it — the bytes
//! never touch the relay, and the message carries a link. See
//! `docs/google-drive-integration-spec.md`.
//!
//! There is no new OAuth client and no new connection for the user to manage:
//! this reuses the Google account already connected for Meet, which now
//! requests `drive.file` alongside the Meet scope (see `google_meet.rs`).
//! `drive.file` lets the app create files and manage **only the files it
//! created** — it cannot see anything else in the user's Drive, which is also
//! why browsing an existing folder is not offered.
//!
//! **Sharing is not done here.** The Workspace default for k2alpha.ai is
//! already "anyone in k2alpha.ai with the link can access", so a file created
//! by this code is readable by the team the moment it exists. An explicit
//! `permissions.create` call would be redundant. If that default is ever
//! changed, this is the place that has to grow one.

use std::sync::Mutex;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use tauri::{
    ipc::{InvokeBody, Request},
    Emitter, State,
};

use super::{google_access_token, GoogleAccessToken, GOOGLE_DRIVE_SCOPE};
use crate::app_state::AppState;

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
const DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// Where uploads land, in each person's own Drive rather than a shared team
/// folder. Deliberate: `drive.file` needs no shared-drive setup, and each
/// person's uploads count against their own quota instead of a common pool
/// nobody is watching. The cost is that files scatter across individual
/// Drives — acceptable, because the Buzz message is the index, not the folder.
const UPLOADS_FOLDER_NAME: &str = "Buzz uploads";

/// Drive's resumable endpoint requires every chunk except the last to be a
/// multiple of 256 KiB. 8 MiB keeps the request count low on a large video
/// while still reporting progress often enough to look alive.
const CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// Folder id for this process's lifetime, so a channel's worth of uploads
/// doesn't re-query Drive each time. Not persisted: a stale id across restarts
/// would be worse than one extra request, and the lookup below re-creates the
/// folder if it has been deleted in the meantime.
static UPLOADS_FOLDER_ID: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
struct DriveFile {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "webViewLink", default)]
    web_view_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DriveFileList {
    #[serde(default)]
    files: Vec<DriveFile>,
}

/// What the renderer needs to post the link as a message.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DriveUpload {
    pub file_id: String,
    /// The name Drive stored, which is what the message label should say.
    pub name: String,
    /// `webViewLink` — a Drive viewer page, not a direct byte URL. That is the
    /// point: it renders as an ordinary link rather than an inline player
    /// pointed at something that will not play.
    pub web_view_link: String,
}

/// Drop the cached folder id. Called on disconnect, because the folder belongs
/// to the account that is going away.
pub(crate) fn forget_uploads_folder() {
    if let Ok(mut cached) = UPLOADS_FOLDER_ID.lock() {
        *cached = None;
    }
}

fn decode_header(value: &str) -> Result<String, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| format!("invalid Drive upload header: {error}"))?;
    String::from_utf8(bytes).map_err(|error| format!("invalid Drive upload header text: {error}"))
}

fn optional_header(request: &Request<'_>, name: &str) -> Result<Option<String>, String> {
    request
        .headers()
        .get(name)
        .map(|value| {
            value
                .to_str()
                .map_err(|error| format!("invalid {name} header: {error}"))
                .and_then(decode_header)
        })
        .transpose()
}

/// Escape a value for a Drive `q` search string, where `'` and `\` are the
/// only characters with meaning inside a quoted literal.
fn escape_query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// The id of this user's "Buzz uploads" folder, creating it on first use.
///
/// Under `drive.file` the list below only ever returns files this app created,
/// so it cannot match a same-named folder the user made themselves — which is
/// the correct behaviour, since the app could not write into that one anyway.
async fn ensure_uploads_folder(
    state: &AppState,
    token: &GoogleAccessToken,
) -> Result<String, String> {
    if let Some(cached) = UPLOADS_FOLDER_ID.lock().ok().and_then(|id| id.clone()) {
        return Ok(cached);
    }

    let query = format!(
        "name = '{}' and mimeType = '{DRIVE_FOLDER_MIME}' and trashed = false",
        escape_query_literal(UPLOADS_FOLDER_NAME)
    );
    let response = state
        .http_client
        .get(format!("{DRIVE_API_BASE}/files"))
        .bearer_auth(&token.access_token)
        .query(&[
            ("q", query.as_str()),
            ("spaces", "drive"),
            ("fields", "files(id,name)"),
            ("pageSize", "1"),
        ])
        .send()
        .await
        .map_err(|error| format!("could not reach Google Drive: {error}"))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("could not read your Google Drive: {body}"));
    }
    let listing: DriveFileList = response
        .json()
        .await
        .map_err(|error| format!("invalid Google Drive response: {error}"))?;

    let folder_id = match listing.files.into_iter().next() {
        Some(folder) => folder.id,
        None => {
            let created = state
                .http_client
                .post(format!("{DRIVE_API_BASE}/files"))
                .bearer_auth(&token.access_token)
                .query(&[("fields", "id")])
                .json(&serde_json::json!({
                    "name": UPLOADS_FOLDER_NAME,
                    "mimeType": DRIVE_FOLDER_MIME,
                }))
                .send()
                .await
                .map_err(|error| format!("could not create the Drive folder: {error}"))?;
            if !created.status().is_success() {
                let body = created.text().await.unwrap_or_default();
                return Err(format!("could not create the Drive folder: {body}"));
            }
            let folder: DriveFile = created
                .json()
                .await
                .map_err(|error| format!("invalid Google Drive response: {error}"))?;
            folder.id
        }
    };

    if let Ok(mut cached) = UPLOADS_FOLDER_ID.lock() {
        *cached = Some(folder_id.clone());
    }
    Ok(folder_id)
}

/// Open a resumable upload session and return the session URI Drive wants the
/// bytes PUT to.
async fn begin_resumable_session(
    state: &AppState,
    token: &GoogleAccessToken,
    filename: &str,
    mime: &str,
    total: usize,
    folder_id: &str,
) -> Result<String, String> {
    let response = state
        .http_client
        .post(format!("{DRIVE_UPLOAD_BASE}/files"))
        .bearer_auth(&token.access_token)
        .query(&[
            ("uploadType", "resumable"),
            ("fields", "id,name,webViewLink"),
        ])
        .header("X-Upload-Content-Type", mime)
        .header("X-Upload-Content-Length", total.to_string())
        .json(&serde_json::json!({
            "name": filename,
            "parents": [folder_id],
        }))
        .send()
        .await
        .map_err(|error| format!("could not start the Drive upload: {error}"))?;
    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("could not start the Drive upload: {body}"));
    }
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| "Google Drive did not return an upload session".to_owned())
}

/// PUT the bytes in chunks, emitting progress on the same event the relay
/// upload path uses so the composer's existing progress bar works unchanged.
///
/// Drive answers each intermediate chunk with **308 Resume Incomplete**, which
/// `reqwest` reports as a perfectly ordinary non-success status — so 308 has to
/// be treated as the success case for every chunk but the last.
async fn put_chunks(
    state: &AppState,
    app: &tauri::AppHandle,
    session_uri: &str,
    data: &[u8],
    progress_id: Option<&str>,
) -> Result<DriveFile, String> {
    let total = data.len();
    let mut offset = 0usize;

    loop {
        let end = usize::min(offset + CHUNK_SIZE, total);
        // A zero-byte file still needs one request, or the session never
        // completes and Drive is left holding an empty reservation.
        let range = if total == 0 {
            "bytes */0".to_owned()
        } else {
            format!("bytes {offset}-{}/{total}", end - 1)
        };

        let response = state
            .http_client
            .put(session_uri)
            .header(reqwest::header::CONTENT_LENGTH, (end - offset).to_string())
            .header(reqwest::header::CONTENT_RANGE, range)
            .body(data[offset..end].to_vec())
            .send()
            .await
            .map_err(|error| format!("Drive upload failed: {error}"))?;

        let status = response.status();
        if status.is_success() {
            emit_progress(app, progress_id, total as u64, total as u64);
            return response
                .json()
                .await
                .map_err(|error| format!("invalid Google Drive response: {error}"));
        }
        // 308 = this chunk landed, keep going.
        if status.as_u16() != 308 {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Drive upload failed: {body}"));
        }

        offset = end;
        emit_progress(app, progress_id, offset as u64, total as u64);
        if offset >= total {
            return Err("Google Drive did not finish the upload after the last chunk".to_owned());
        }
    }
}

fn emit_progress(app: &tauri::AppHandle, progress_id: Option<&str>, sent: u64, total: u64) {
    let Some(id) = progress_id else {
        return;
    };
    let _ = app.emit(
        "media-upload-progress",
        serde_json::json!({ "id": id, "sent": sent, "total": total }),
    );
}

/// True when a Google account is connected **and** it granted the Drive scope.
///
/// Separate from `get_google_meet_connection_status` because the two can
/// disagree: anyone who connected before Drive shipped is connected for Meet
/// and not for Drive, and the composer needs to tell them to reconnect rather
/// than letting the upload fail halfway.
#[tauri::command]
pub(crate) async fn get_google_drive_status(
    app_state: State<'_, AppState>,
) -> Result<bool, String> {
    match google_access_token(&app_state).await {
        Ok(token) => Ok(token.has_scope(GOOGLE_DRIVE_SCOPE)),
        // Not connected at all, or the connection expired. Either way the
        // answer to "can this user upload to Drive right now" is no; the
        // composer prompts to connect and the real error surfaces there.
        Err(_) => Ok(false),
    }
}

/// Upload raw IPC bytes to the sender's Drive and return a shareable link.
///
/// Mirrors `upload_media_bytes_raw`'s raw-byte transport so a large file is
/// never expanded into JSON on the way across the IPC boundary.
#[tauri::command]
pub(crate) async fn upload_drive_bytes_raw(
    request: Request<'_>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<DriveUpload, String> {
    let data = match request.body() {
        InvokeBody::Raw(data) => data.clone(),
        InvokeBody::Json(_) => return Err("raw upload requires a byte body".to_string()),
    };
    let filename = optional_header(&request, "x-buzz-filename")?
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| "the Drive upload is missing a filename".to_owned())?;
    let mime = optional_header(&request, "x-buzz-mime")?
        .filter(|mime| !mime.trim().is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let progress_id = optional_header(&request, "x-buzz-progress-id")?;

    let token = google_access_token(&state).await?;
    if !token.has_scope(GOOGLE_DRIVE_SCOPE) {
        return Err(
            "Your Google account is connected for Meet but not for Drive. Disconnect and reconnect it under Settings → Voice to allow Drive uploads."
                .to_owned(),
        );
    }

    let folder_id = ensure_uploads_folder(&state, &token).await?;
    let session_uri =
        begin_resumable_session(&state, &token, &filename, &mime, data.len(), &folder_id).await?;
    let file = put_chunks(&state, &app, &session_uri, &data, progress_id.as_deref()).await?;

    let web_view_link = file
        .web_view_link
        .ok_or_else(|| "Google Drive did not return a link for the uploaded file".to_owned())?;
    Ok(DriveUpload {
        file_id: file.id,
        name: file.name.unwrap_or(filename),
        web_view_link,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_literals_escape_quotes_and_backslashes() {
        assert_eq!(escape_query_literal("Buzz uploads"), "Buzz uploads");
        assert_eq!(escape_query_literal("it's"), "it\\'s");
        assert_eq!(escape_query_literal("a\\b"), "a\\\\b");
    }

    #[test]
    fn headers_round_trip_unicode() {
        let encoded = URL_SAFE_NO_PAD.encode("quarterly review 🎬.mp4");
        assert_eq!(decode_header(&encoded).unwrap(), "quarterly review 🎬.mp4");
    }

    #[test]
    fn chunk_size_is_a_multiple_of_drive_minimum() {
        // Drive rejects any non-final chunk that is not a multiple of 256 KiB.
        assert_eq!(CHUNK_SIZE % (256 * 1024), 0);
    }
}
