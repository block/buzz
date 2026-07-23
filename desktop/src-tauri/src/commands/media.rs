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

/// Return true when a PNG/WebP payload declares animation.
///
/// Animated payloads use structural sanitizers so frame timing, looping, and
/// disposal semantics are preserved without flattening the image. The relay's
/// validator remains the final authority for the sanitized container.
fn is_animated_image(body: &[u8], mime: &str) -> bool {
    match mime {
        "image/png" if body.starts_with(b"\x89PNG\r\n\x1a\n") => {
            let mut offset = 8usize;
            while offset.checked_add(12).is_some_and(|end| end <= body.len()) {
                let length = u32::from_be_bytes([
                    body[offset],
                    body[offset + 1],
                    body[offset + 2],
                    body[offset + 3],
                ]) as usize;
                let Some(end) = offset.checked_add(12).and_then(|v| v.checked_add(length)) else {
                    return false;
                };
                if end > body.len() {
                    return false;
                }
                if &body[offset + 4..offset + 8] == b"acTL" {
                    return true;
                }
                offset = end;
            }
            false
        }
        "image/webp"
            if body.len() >= 12 && body.starts_with(b"RIFF") && &body[8..12] == b"WEBP" =>
        {
            let mut offset = 12usize;
            while offset.checked_add(8).is_some_and(|end| end <= body.len()) {
                let chunk = &body[offset..offset + 4];
                if chunk == b"ANIM" || chunk == b"ANMF" {
                    return true;
                }
                let length = u32::from_le_bytes([
                    body[offset + 4],
                    body[offset + 5],
                    body[offset + 6],
                    body[offset + 7],
                ]) as usize;
                let padded = length.checked_add(length & 1);
                let Some(end) = padded.and_then(|v| offset.checked_add(8 + v)) else {
                    return false;
                };
                if end > body.len() {
                    return false;
                }
                offset = end;
            }
            false
        }
        _ => false,
    }
}

pub(crate) fn sanitize_image_for_upload(body: Vec<u8>, mime: &str) -> Result<Vec<u8>, String> {
    let format = match mime {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        // GIF is never re-encoded (that would destroy animation timing);
        // metadata extensions are stripped structurally instead. Unparseable
        // payloads pass through — the relay's validator is the authority.
        "image/gif" => {
            let stripped = super::media_gif::strip_gif_metadata(&body);
            return Ok(stripped.unwrap_or(body));
        }
        _ => return Ok(body),
    };

    if is_animated_image(&body, mime) {
        let oriented_format = match mime {
            "image/png" if super::media_animated::animated_png_uses_exif_orientation(&body) => {
                Some("PNG")
            }
            "image/webp" if super::media_animated::animated_webp_uses_exif_orientation(&body) => {
                Some("WebP")
            }
            _ => None,
        };
        if let Some(format) = oriented_format {
            return Err(format!(
                "animated {format} with EXIF orientation cannot be uploaded without changing its appearance"
            ));
        }
        let color_profile_format = match mime {
            "image/png" if super::media_animated::animated_png_uses_icc_profile(&body) => {
                Some("PNG")
            }
            "image/webp" if super::media_animated::animated_webp_uses_icc_profile(&body) => {
                Some("WebP")
            }
            _ => None,
        };
        if let Some(format) = color_profile_format {
            return Err(format!(
                "animated {format} with an ICC profile cannot be uploaded without changing its colors"
            ));
        }
        let stripped = match mime {
            "image/png" => super::media_animated::strip_animated_png_metadata(&body),
            "image/webp" => super::media_animated::strip_animated_webp_metadata(&body),
            _ => None,
        };
        return Ok(stripped.unwrap_or(body));
    }

    use image::ImageDecoder;
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&body), format);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "failed to decode image for metadata removal".to_string())?;
    decoder
        .set_limits(image::Limits::default())
        .map_err(|_| "image exceeds safe decoding limits".to_string())?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "failed to read image orientation".to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "failed to decode image for metadata removal".to_string())?;
    image.apply_orientation(orientation);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, format)
        .map_err(|_| "failed to encode image without metadata".to_string())?;
    Ok(output.into_inner())
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

/// Lifetime of a Blossom `t=get` read token. Ten minutes keeps a token alive
/// across a video's range-request stream while staying well inside the
/// server's `created_at` freshness window (3600s, matching upload).
pub(crate) const MEDIA_GET_AUTH_EXPIRY_SECS: u64 = 600;

/// Sign a Blossom (BUD-01) `t=get` authorization event, server-scoped to the
/// relay's authority, and return the full `Authorization` header value.
///
/// Server-scoped (a `server` tag, no `x` tag): one token authorizes reads of
/// any blob on that host for its lifetime, which keeps avatar-grid bursts and
/// video range requests cheap. This is deliberately broader than per-blob
/// scoping and is safe only because the relay still enforces NIP-43
/// membership on the verified pubkey — and because callers only attach this
/// header to requests bound for the relay origin itself.
pub(crate) fn sign_blossom_get_auth_header(
    keys: &Keys,
    base_url: &str,
    expiry_secs: u64,
) -> Result<String, String> {
    let server = extract_server_authority(base_url)
        .ok_or_else(|| "cannot derive server authority from relay URL".to_string())?;
    let now = Timestamp::now().as_secs();
    let tags = vec![
        Tag::parse(vec!["t", "get"]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["expiration", &(now + expiry_secs).to_string()])
            .map_err(|e| e.to_string())?,
        Tag::parse(vec!["server".to_string(), server]).map_err(|e| e.to_string())?,
    ];
    let event = EventBuilder::new(Kind::from(24242), "Get buzz-media")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
    ))
}

/// Mint a `t=get` Authorization header value for a relay media fetch, or
/// `None` when signing is unavailable (identity in recovery mode).
///
/// Fail-open by design: while the relay's `BUZZ_REQUIRE_MEDIA_GET_AUTH` flag
/// is off, an unauthenticated request still succeeds, so degrading to no
/// header (instead of erroring) keeps media rendering during key recovery.
/// Once the flag is on, these requests will 403 — the correct outcome for an
/// identity that can't prove membership.
///
/// Safety contract: callers must only attach the returned header to URLs
/// constructed from (or validated against) the app's own relay base URL —
/// never to third-party origins, where the bearer token would leak.
pub(crate) fn mint_media_get_auth(state: &AppState, base_url: &str) -> Option<String> {
    let keys = match state.signing_keys() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("buzz-desktop: media get auth unavailable (unsigned request): {e}");
            return None;
        }
    };
    match sign_blossom_get_auth_header(&keys, base_url, MEDIA_GET_AUTH_EXPIRY_SECS) {
        Ok(header) => Some(header),
        Err(e) => {
            eprintln!("buzz-desktop: media get auth signing failed (unsigned request): {e}");
            None
        }
    }
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
fn should_retry_legacy_upload(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
    )
}

async fn send_upload_attempt(
    state: &State<'_, AppState>,
    url: String,
    auth_header: &str,
    mime: &str,
    sha256: &str,
    body: bytes::Bytes,
    progress: Option<&(tauri::AppHandle, String)>,
) -> Result<reqwest::Response, String> {
    let req = state
        .http_client
        .put(url)
        .header("Authorization", auth_header)
        .header("Content-Type", mime)
        .header("X-SHA-256", sha256);

    let response = if let Some((app, progress_id)) = progress {
        use tauri::Emitter;
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
        req.header(reqwest::header::CONTENT_LENGTH, total)
            .body(reqwest::Body::wrap_stream(stream))
            .send()
            .await
    } else {
        req.body(body).send().await
    };
    response.map_err(|error| classify_request_error(&error))
}

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
    let body = bytes::Bytes::from(body);
    let mut resp = send_upload_attempt(
        state,
        format!("{base_url}/upload"),
        &auth_header,
        mime,
        &sha256,
        body.clone(),
        progress.as_ref(),
    )
    .await?;
    if should_retry_legacy_upload(resp.status()) {
        resp = send_upload_attempt(
            state,
            format!("{base_url}/media/upload"),
            &auth_header,
            mime,
            &sha256,
            body,
            progress.as_ref(),
        )
        .await?;
    }

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
    let body = sanitize_image_for_upload(body, &mime)?;
    do_upload(body, &mime, &state, None).await
}

/// Read a picked path through the TOCTOU-safe pipeline (fd pin → sniff →
/// transcode-or-passthrough → MIME validation → upload).
///
/// When `images_only` is set, the file is rejected **before upload** if it is
/// not an image (videos and non-image files error out; HEIC/HEIF still
/// transcode to JPEG, which is an image). This keeps discarded/non-image
/// files from ever leaving the client on image-only surfaces.
async fn process_picked_path(
    path: std::path::PathBuf,
    state: &State<'_, AppState>,
    images_only: bool,
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
                if images_only {
                    return Err("Please choose an image file.".to_string());
                }
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
    let body = sanitize_image_for_upload(body, &mime)?;

    // Image-only surfaces (e.g. "Send feedback"): reject anything that didn't
    // sniff as an image, BEFORE the upload leaves the client.
    if images_only && !mime.starts_with("image/") {
        return Err("Please choose an image file.".to_string());
    }

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
        let descriptor = process_picked_path(path, &state, false).await?;
        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

/// Open a native single-file dialog constrained to images, read the picked
/// file, and upload it — rejecting anything that doesn't sniff as an image
/// **before** the bytes leave the client.
///
/// This is the secure path for image-only surfaces (e.g. the "Send feedback"
/// attachment). Unlike [`pick_and_upload_media`], the dialog is filtered to
/// common image extensions and `process_picked_path` runs with
/// `images_only = true`, so a user who bypasses the extension filter still
/// can't upload a non-image (videos and other files error out during MIME
/// validation, before `do_upload`). Returns `None` when the user cancels.
#[tauri::command]
pub async fn pick_and_upload_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<BlobDescriptor>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Images",
            &["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp"],
        )
        .pick_file(move |path| {
            let _ = tx.send(path);
        });

    let file_path = match rx.await.map_err(|_| "dialog cancelled".to_string())? {
        Some(path) => path,
        None => return Ok(None),
    };

    let path = file_path.as_path().ok_or("invalid path")?.to_path_buf();
    let descriptor = process_picked_path(path, &state, true).await?;
    Ok(Some(descriptor))
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
    let body = sanitize_image_for_upload(body, &mime)?;

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
    fn test_sign_blossom_get_auth_header_shape() {
        let keys = Keys::generate();
        let header = sign_blossom_get_auth_header(&keys, "http://localhost:3000", 600).unwrap();
        let b64 = header.strip_prefix("Nostr ").expect("Nostr scheme prefix");
        let json = URL_SAFE_NO_PAD.decode(b64).unwrap();
        let event = nostr::Event::from_json(std::str::from_utf8(&json).unwrap()).unwrap();

        assert_eq!(event.kind, Kind::from(24242));
        event.verify().expect("valid signature");

        let tag = |name: &str| -> Option<String> {
            event.tags.iter().find_map(|t| {
                let v = t.as_slice();
                (v.first().map(String::as_str) == Some(name)).then(|| v[1].clone())
            })
        };
        assert_eq!(tag("t").as_deref(), Some("get"));
        assert_eq!(tag("server").as_deref(), Some("localhost:3000"));
        // Server-scoped token: no x tag (BUD-01 allows x OR server).
        assert!(tag("x").is_none());
        let expiration: u64 = tag("expiration").unwrap().parse().unwrap();
        let now = Timestamp::now().as_secs();
        assert!(expiration > now && expiration <= now + 600);
    }

    #[test]
    fn test_sign_blossom_get_auth_header_invalid_base_url() {
        let keys = Keys::generate();
        assert!(sign_blossom_get_auth_header(&keys, "not-a-url", 600).is_err());
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
    fn test_image_sanitizer_bakes_exif_orientation() {
        let source = image::RgbImage::from_fn(2, 3, |x, y| {
            image::Rgb([(x * 80) as u8, (y * 60) as u8, 32])
        });
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .encode_image(&source)
            .unwrap();

        // Minimal little-endian Exif IFD with Orientation=6 (rotate 90°).
        let mut exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0".to_vec();
        exif.extend_from_slice(&[
            0x12, 0x01, // Orientation tag
            0x03, 0x00, // SHORT
            0x01, 0x00, 0x00, 0x00, // count=1
            0x06, 0x00, 0x00, 0x00, // value=6
            0x00, 0x00, 0x00, 0x00, // next IFD
        ]);
        let segment_len = (exif.len() + 2) as u16;
        let mut oriented = encoded[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&segment_len.to_be_bytes());
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&encoded[2..]);

        let sanitized = sanitize_image_for_upload(oriented, "image/jpeg").unwrap();
        let decoded =
            image::load_from_memory_with_format(&sanitized, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        assert!(!sanitized.windows(6).any(|bytes| bytes == b"Exif\0\0"));
    }

    #[test]
    fn test_animated_png_and_webp_are_not_flattened() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&8u32.to_be_bytes());
        apng.extend_from_slice(b"acTL");
        apng.extend_from_slice(&[0; 8]);
        apng.extend_from_slice(&[0; 4]);
        assert!(is_animated_image(&apng, "image/png"));
        assert!(sanitize_image_for_upload(apng, "image/png").is_ok());

        let mut webp = b"RIFF\x0c\0\0\0WEBPANIM".to_vec();
        webp.extend_from_slice(&0u32.to_le_bytes());
        assert!(is_animated_image(&webp, "image/webp"));
        assert!(sanitize_image_for_upload(webp, "image/webp").is_ok());
    }

    #[test]
    fn test_legacy_upload_retry_statuses_are_narrow() {
        assert!(should_retry_legacy_upload(reqwest::StatusCode::NOT_FOUND));
        assert!(should_retry_legacy_upload(
            reqwest::StatusCode::METHOD_NOT_ALLOWED
        ));
        assert!(!should_retry_legacy_upload(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY
        ));
        assert!(!should_retry_legacy_upload(
            reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
        ));
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
