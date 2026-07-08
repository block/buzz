use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_state::AppState;
use crate::relay::{
    classify_request_error, parse_json_response, relay_api_base_url_with_override,
    relay_error_message,
};

use super::media_transcode::{
    has_heic_extension, is_heic_file, is_video_file, transcode_and_extract_poster,
    transcode_heic_path_to_jpeg_bytes,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub mime_type: String,
    pub uploaded: i64,
    pub dim: Option<String>,
    pub blurhash: Option<String>,
    pub thumb: Option<String>,
    /// Video duration in seconds. `None` for non-video blobs.
    pub duration: Option<f64>,
    /// NIP-71 poster frame URL. `None` for non-video blobs or if extraction failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Original filename captured client-side (the relay is content-addressed
    /// and never learns it). Generic files use it for file-card labels; custom
    /// emoji upload uses it to suggest a shortcode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the server authority from a URL for BUD-11 server tag scoping.
///
/// Returns `host` for default ports (80/443), `host:port` for non-default ports.
fn extract_server_authority(url_str: &str) -> Option<String> {
    let parsed = url::Url::parse(url_str).ok()?;
    let host = parsed.host_str()?;
    match parsed.port() {
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host.to_string()),
    }
}

/// Resolve the real filesystem path of an already-opened file descriptor.
///
/// Returns the path the kernel associates with the inode, not the pathname
/// used to open it. Immune to post-open renames/symlink swaps.
#[cfg(target_os = "macos")]
fn fd_real_path(file: &std::fs::File) -> Result<std::path::PathBuf, String> {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    let ret = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
    if ret == -1 {
        return Err(format!(
            "fcntl F_GETPATH failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = std::str::from_utf8(&buf[..nul]).map_err(|e| e.to_string())?;
    Ok(std::path::PathBuf::from(s))
}

#[cfg(target_os = "linux")]
fn fd_real_path(file: &std::fs::File) -> Result<std::path::PathBuf, String> {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn fd_real_path(file: &std::fs::File) -> Result<std::path::PathBuf, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED,
    };
    let handle = file.as_raw_handle() as *mut core::ffi::c_void;
    let mut buf = vec![0u16; 1024];
    let len = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            buf.as_mut_ptr(),
            buf.len() as u32,
            FILE_NAME_NORMALIZED,
        )
    };
    if len == 0 {
        return Err(format!(
            "GetFinalPathNameByHandleW failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let path_str = String::from_utf16_lossy(&buf[..len as usize]);
    // Strip \\?\ prefix that Windows adds
    let cleaned = path_str.strip_prefix(r"\\?\").unwrap_or(&path_str);
    Ok(std::path::PathBuf::from(cleaned))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn fd_real_path(_file: &std::fs::File) -> Result<std::path::PathBuf, String> {
    Err("fd_real_path not supported on this platform".to_string())
}

/// MIME types blocked from upload — mirrors the server's generic-file deny-list.
///
/// Active-content XSS carriers and native executables. Everything else (images,
/// video, documents, archives, audio, text, data) is accepted; un-sniffable
/// files fall back to `application/octet-stream` and are served as downloads.
const BLOCKED_MIME: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "image/svg+xml",
    "application/javascript",
    "text/javascript",
    "application/x-msdownload",
    "application/x-executable",
    "application/vnd.microsoft.portable-executable",
    "application/x-mach-binary",
    "application/x-sharedlib",
    "application/x-elf",
    "application/x-msi",
    "application/vnd.android.package-archive",
    "application/x-apple-diskimage",
];

/// Sanitize a filename for use as a display label in the imeta `filename` field.
///
/// Strips any directory components (keeps only the final path segment), removes
/// control characters, and bounds length to 255. Mirrors the relay's filename
/// validation so a sanitized name always passes ingest. Returns a fallback when
/// the result would be empty.
pub(crate) fn sanitize_filename(name: &str) -> String {
    // Keep only the final path segment — defend against `../` and absolute paths
    // regardless of separator style.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(255).collect();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

pub(crate) fn detect_and_validate_mime(body: &[u8]) -> Result<String, String> {
    let mime = infer::get(body)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if BLOCKED_MIME.contains(&mime.as_str()) {
        return Err(format!("unsupported file type: {mime}"));
    }
    Ok(mime)
}

fn sign_blossom_upload_auth(
    keys: &Keys,
    sha256: &str,
    expiry_secs: u64,
    base_url: &str,
) -> Result<nostr::Event, String> {
    let now = Timestamp::now().as_secs();
    let mut tags = vec![
        Tag::parse(vec!["t", "upload"]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["x", sha256]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["expiration", &(now + expiry_secs).to_string()])
            .map_err(|e| e.to_string())?,
    ];
    if let Some(domain) = extract_server_authority(base_url) {
        tags.push(Tag::parse(vec!["server".to_string(), domain]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::from(24242), "Upload buzz-media")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())
}

/// Execute the upload HTTP request. Shared by all upload entry points.
// TODO(v2): Stream large video files to the relay instead of buffering in RAM.
// Current approach works for videos up to ~100MB but will OOM on 500MB files.
// Fix: use reqwest's Body::wrap_stream() to stream from the temp file directly.
// The server already supports streaming upload via process_video_upload.
async fn do_upload(
    body: Vec<u8>,
    mime: &str,
    state: &State<'_, AppState>,
    progress: Option<(tauri::AppHandle, String)>,
) -> Result<BlobDescriptor, String> {
    let sha256 = hex::encode(Sha256::digest(&body));

    // Video uploads get a 1-hour auth window to survive slow connections;
    // images use 5 minutes. Must match the server-side max_age_secs values
    // in process_upload (600s) and process_video_upload (3600s).
    let expiry_secs = if mime.starts_with("video/") {
        3600
    } else {
        300
    };
    let base_url = relay_api_base_url_with_override(state);
    let auth_event = {
        let keys = state.signing_keys()?;
        sign_blossom_upload_auth(&keys, &sha256, expiry_secs, &base_url)?
    };

    let auth_header = format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(auth_event.as_json().as_bytes())
    );
    let req = state
        .http_client
        .put(format!("{base_url}/media/upload"))
        .header("Authorization", &auth_header)
        .header("Content-Type", mime)
        .header("X-SHA-256", &sha256);

    // With a progress channel, stream the body in chunks and emit a
    // `media-upload-progress` event as each chunk is handed to the socket,
    // so the renderer can draw a determinate progress bar.
    let resp = if let Some((app, progress_id)) = progress {
        use tauri::Emitter;
        let total = body.len() as u64;
        // Ref-counted slices of one buffer — no second copy of the payload.
        let body = bytes::Bytes::from(body);
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
        req.header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
    } else {
        req.body(body).send().await
    }
    .map_err(|e| classify_request_error(&e))?;

    if !resp.status().is_success() {
        return Err(relay_error_message(resp).await);
    }

    parse_json_response::<BlobDescriptor>(resp).await
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Upload a file that is already in the OS temp directory.
///
/// Trust boundary: only reads files inside `temp_dir()`. Opens the fd first,
/// then resolves the fd's real path to verify containment (TOCTOU-safe).
#[tauri::command]
pub async fn upload_media(
    file_path: String,
    is_temp: bool,
    state: State<'_, AppState>,
) -> Result<BlobDescriptor, String> {
    let path = std::path::Path::new(&file_path);
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;

    let fd_path = fd_real_path(&file)?;
    let canonical_temp = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());
    if !fd_path.starts_with(&canonical_temp) {
        return Err("upload source must be in system temp directory".to_string());
    }

    use std::io::Read;
    let mut body = Vec::new();
    file.read_to_end(&mut body)
        .map_err(|e| format!("failed to read file: {e}"))?;
    drop(file);

    if is_temp {
        let _ = std::fs::remove_file(&fd_path);
    }

    let mime = detect_and_validate_mime(&body)?;
    do_upload(body, &mime, &state, None).await
}

/// Read a picked path through the TOCTOU-safe pipeline (fd pin → sniff →
/// transcode-or-passthrough → MIME validation → upload).
async fn process_picked_path(
    path: std::path::PathBuf,
    state: &State<'_, AppState>,
) -> Result<BlobDescriptor, String> {
    // Pin the inode by opening the fd BEFORE spawn_blocking. This prevents a
    // local attacker from swapping the file between dialog return and read.
    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;

    // Extension hint for HEIC detection — some HEIC files from non-Apple
    // tooling carry brands outside HEIC_BRANDS, but the `.heic`/`.heif`
    // extension still tells us the webview can't render them. Computed before
    // the closure since `path` isn't moved in.
    let heic_by_ext = has_heic_extension(&path);

    // All sync I/O (sniff, transcode, read) runs off the async runtime to
    // avoid blocking Tokio worker threads during long ffmpeg transcodes.
    let (body, poster_bytes) =
        tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
            use std::io::Read;

            // Sniff magic bytes from the pinned fd — no re-open, no TOCTOU.
            let mut header = [0u8; 4096];
            let n = file.read(&mut header).map_err(|e| e.to_string())?;

            if is_video_file(&header[..n]) {
                // ffmpeg needs a path, not an fd. Resolve the fd's real path
                // so we pass the actual inode's location, not the original
                // (potentially swapped) pathname. Same pattern as upload_media.
                // IMPORTANT: keep `file` alive (fd open) until after transcode
                // completes — this prevents the inode from being unlinked or
                // the resolved path from becoming stale during the ffmpeg run.
                let fd_path = fd_real_path(&file)?;
                let result = transcode_and_extract_poster(&fd_path);
                drop(file); // release fd only after ffmpeg is done
                result
            } else if heic_by_ext || is_heic_file(&header[..n]) {
                // HEIC/HEIF still: Chromium/the webview can't decode it, so
                // transcode to JPEG before upload (mirrors mobile). Resolve the
                // fd's real path so ffmpeg reads the pinned inode, and keep
                // `file` alive until the transcode finishes.
                let fd_path = fd_real_path(&file)?;
                let result = transcode_heic_path_to_jpeg_bytes(&fd_path).map(|jpeg| (jpeg, None));
                drop(file); // release fd only after ffmpeg is done
                result
            } else {
                // Image: read the rest from the already-open fd (TOCTOU-safe).
                let mut bytes = header[..n].to_vec();
                file.read_to_end(&mut bytes)
                    .map_err(|e| format!("failed to read file: {e}"))?;
                Ok((bytes, None))
            }
        })
        .await
        .map_err(|e| format!("transcode task failed: {e}"))??;

    let mime = detect_and_validate_mime(&body)?;

    // Upload video first, then poster (best-effort). If poster upload fails,
    // the video descriptor is returned without an image field.
    let mut descriptor = do_upload(body, &mime, state, None).await?;

    if let Some(poster) = poster_bytes {
        match do_upload(poster, "image/jpeg", state, None).await {
            Ok(poster_desc) => descriptor.image = Some(poster_desc.url),
            Err(e) => eprintln!("buzz-desktop: poster upload failed (non-fatal): {e}"),
        }
    }

    descriptor.filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(sanitize_filename);

    Ok(descriptor)
}

/// Open a native file dialog (multi-select), read each selected file, and
/// upload it. Returns the resulting `BlobDescriptor` list — empty when the
/// user cancels.
///
/// All file I/O happens in trusted Rust — the renderer never touches the
/// filesystem. This is the secure path for the 📎 paperclip button.
///
/// **Residual TOCTOU note:** The Tauri dialog plugin returns pathnames, not
/// file handles, so there is a small race window between dialog return and
/// `File::open()` — an inherent limit of the OS file-picker API. The risk is
/// bounded (local attacker winning a race against an immediate open) and
/// server-side content validation (MIME, image decode, size caps) is the
/// defense in depth.
///
/// Uploads run sequentially; on first failure, prior uploads are not
/// rolled back (they're already content-addressed on the relay).
#[tauri::command]
pub async fn pick_and_upload_media(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<BlobDescriptor>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    // No filter — accept any file. The deny-list (active content + executables)
    // and size caps are enforced by `detect_and_validate_mime` and the relay.
    app.dialog().file().pick_files(move |paths| {
        let _ = tx.send(paths);
    });

    let file_paths = match rx.await.map_err(|_| "dialog cancelled".to_string())? {
        Some(paths) => paths,
        None => return Ok(Vec::new()),
    };

    let mut descriptors = Vec::with_capacity(file_paths.len());
    for file_path in file_paths {
        let path = file_path.as_path().ok_or("invalid path")?.to_path_buf();
        let descriptor = process_picked_path(path, &state).await?;
        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

/// Upload raw bytes directly (for paste and drag-drop).
///
/// The renderer already has the bytes in memory from the clipboard/drag event.
/// If the bytes are a video, they're written to a temp file, transcoded via
/// ffmpeg, and the transcoded output is uploaded instead.
#[tauri::command]
pub async fn upload_media_bytes(
    data: Vec<u8>,
    filename: Option<String>,
    progress_id: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<BlobDescriptor, String> {
    if data.is_empty() {
        return Err("empty upload".to_string());
    }

    let (body, poster_bytes) = if is_video_file(&data) {
        // Video: write to temp → transcode + extract poster → read results.
        // All blocking I/O runs off the async runtime via spawn_blocking.
        tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
            let tmp_input =
                std::env::temp_dir().join(format!("buzz-drop-{}", uuid::Uuid::new_v4()));
            // Cleanup guard: remove temp file on ALL exit paths (including write failure).
            let result = (|| {
                std::fs::write(&tmp_input, &data)
                    .map_err(|e| format!("failed to write temp file: {e}"))?;
                transcode_and_extract_poster(&tmp_input)
            })();
            let _ = std::fs::remove_file(&tmp_input);
            result
        })
        .await
        .map_err(|e| format!("transcode task failed: {e}"))??
    } else if is_heic_file(&data) {
        // HEIC/HEIF still pasted/dropped: no filename here, so detection is
        // magic-bytes only. ffmpeg needs a path, so write to temp, transcode
        // to JPEG, and clean up. (Mirrors mobile's pre-upload transcode.)
        tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
            let tmp_input =
                std::env::temp_dir().join(format!("buzz-drop-{}", uuid::Uuid::new_v4()));
            // Cleanup guard: remove temp file on ALL exit paths (including write failure).
            let result = (|| {
                std::fs::write(&tmp_input, &data)
                    .map_err(|e| format!("failed to write temp file: {e}"))?;
                transcode_heic_path_to_jpeg_bytes(&tmp_input).map(|jpeg| (jpeg, None))
            })();
            let _ = std::fs::remove_file(&tmp_input);
            result
        })
        .await
        .map_err(|e| format!("transcode task failed: {e}"))??
    } else {
        (data, None)
    };

    let mime = detect_and_validate_mime(&body)?;

    // Upload video first, then poster (best-effort).
    let progress = progress_id.map(|id| (app, id));
    let mut descriptor = do_upload(body, &mime, &state, progress).await?;

    if let Some(poster) = poster_bytes {
        match do_upload(poster, "image/jpeg", &state, None).await {
            Ok(poster_desc) => descriptor.image = Some(poster_desc.url),
            Err(e) => eprintln!("buzz-desktop: poster upload failed (non-fatal): {e}"),
        }
    }

    descriptor.filename = filename.as_deref().map(sanitize_filename);

    Ok(descriptor)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_server_authority_default_ports() {
        assert_eq!(
            extract_server_authority("https://relay.example.com"),
            Some("relay.example.com".to_string())
        );
        assert_eq!(
            extract_server_authority("https://relay.example.com:443"),
            Some("relay.example.com".to_string())
        );
        assert_eq!(
            extract_server_authority("http://relay.example.com:80"),
            Some("relay.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_server_authority_non_default_ports() {
        assert_eq!(
            extract_server_authority("http://localhost:3000"),
            Some("localhost:3000".to_string())
        );
        assert_eq!(
            extract_server_authority("https://relay.example.com:8443"),
            Some("relay.example.com:8443".to_string())
        );
    }

    #[test]
    fn test_extract_server_authority_ipv6() {
        assert_eq!(
            extract_server_authority("http://[::1]:3000"),
            Some("[::1]:3000".to_string())
        );
    }

    #[test]
    fn test_extract_server_authority_invalid() {
        assert_eq!(extract_server_authority("not-a-url"), None);
        assert_eq!(extract_server_authority(""), None);
    }

    #[test]
    fn test_detect_and_validate_mime_jpeg() {
        // Minimal JPEG: SOI + EOI
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_and_validate_mime(&jpeg).unwrap(), "image/jpeg");
    }

    #[test]
    fn test_detect_and_validate_mime_accepts_text_as_octet_stream() {
        // Plain text has no magic bytes — infer returns None, so it's accepted
        // as opaque binary (served as a download). This is the common Slack case.
        let text = b"hello world";
        assert_eq!(
            detect_and_validate_mime(text).unwrap(),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_detect_and_validate_mime_rejects_html() {
        let html = b"<!DOCTYPE html><html><body><script>alert(1)</script></body></html>";
        assert!(detect_and_validate_mime(html).is_err());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        // Strips directory components and traversal.
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/abs/path/notes.txt"), "notes.txt");
        assert_eq!(sanitize_filename(r"C:\Users\me\doc.docx"), "doc.docx");
        // Empty / separator-only falls back.
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("/"), "file");
        // Control chars removed.
        assert_eq!(sanitize_filename("a\nb\tc.txt"), "abc.txt");
    }
}
