use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_state::AppState;
use crate::managed_agents::resolve_command;
use crate::relay::relay_api_base_url_with_override;

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
    let handle = file.as_raw_handle() as isize;
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

/// MIME allowlist — must match the server's allowed types.
const ALLOWED_MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "video/mp4",
];

pub(crate) fn detect_and_validate_mime(body: &[u8]) -> Result<String, String> {
    let mime = infer::get(body)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if !ALLOWED_MIME.contains(&mime.as_str()) {
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
    let now = Timestamp::now().as_u64();
    let mut tags = vec![
        Tag::parse(vec!["t", "upload"]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["x", sha256]).map_err(|e| e.to_string())?,
        Tag::parse(vec!["expiration", &(now + expiry_secs).to_string()])
            .map_err(|e| e.to_string())?,
    ];
    if let Some(domain) = extract_server_authority(&base_url) {
        tags.push(Tag::parse(vec!["server".to_string(), domain]).map_err(|e| e.to_string())?);
    }
    EventBuilder::new(Kind::from(24242), "Upload sprout-media")
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
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
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

    let resp = req
        .body(body)
        .send()
        .await
        .map_err(|e| format!("upload failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("upload failed ({status}): {text}"));
    }

    resp.json::<BlobDescriptor>()
        .await
        .map_err(|e| format!("parse failed: {e}"))
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
    do_upload(body, &mime, &state).await
}

// ── Video transcode helpers ──────────────────────────────────────────────────

/// Locate ffmpeg using the same discovery logic as managed agents
/// (login shell PATH, /opt/homebrew/bin, /usr/local/bin, etc.).
/// Returns the resolved absolute path on success.
fn find_ffmpeg() -> Result<std::path::PathBuf, String> {
    let ffmpeg_path = resolve_command("ffmpeg", None).ok_or_else(|| {
        "ffmpeg is required for video uploads but was not found.\n\n\
         Install it:\n  \
         macOS:   brew install ffmpeg\n  \
         Linux:   sudo apt install ffmpeg\n  \
         Windows: winget install ffmpeg"
            .to_string()
    })?;

    match std::process::Command::new(&ffmpeg_path)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(ffmpeg_path),
        Ok(_) => Err(
            "ffmpeg was found but returned an error — it may be broken or misconfigured"
                .to_string(),
        ),
        Err(e) => Err(format!("failed to check for ffmpeg: {e}")),
    }
}

/// Detect if a file is a video based on magic bytes.
fn is_video_file(buf: &[u8]) -> bool {
    infer::get(buf).map_or(false, |t| t.mime_type().starts_with("video/"))
}

/// Maximum wall-clock time for an ffmpeg transcode before we kill it.
/// 10 minutes is generous for any reasonable video; pathological inputs
/// (crafted to cause exponential decode time) get killed instead of
/// blocking a Tokio worker thread indefinitely.
const FFMPEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Run an ffmpeg command with a wall-clock timeout.
///
/// Spawns the child process, polls `try_wait()` every 500ms, and kills it
/// if the deadline is exceeded. Returns the same `Output` as `Command::output()`.
///
/// **IMPORTANT**: callers MUST pass `-loglevel error` (or `quiet`) to ffmpeg.
/// This function reads stderr only after the child exits. If ffmpeg writes
/// enough progress/diagnostic output to fill the OS pipe buffer (~64 KiB),
/// the child blocks on write() and never exits — causing a false timeout.
/// `-loglevel error` suppresses progress spam, keeping stderr small.
fn run_ffmpeg_with_timeout(
    cmd: &mut std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn ffmpeg: {e}"))?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process exited — collect output.
                let stdout = child.stdout.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                let stderr = child.stderr.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Still running — check deadline.
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    return Err(format!("ffmpeg timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => return Err(format!("failed to wait on ffmpeg: {e}")),
        }
    }
}

/// Transcode any video file to H.264/AAC/MP4/fast-start via ffmpeg.
///
/// Always re-encodes — handles HEVC, VP9, ProRes, non-faststart MP4, 10-bit,
/// wrong pixel format, MOV containers, etc. Output is guaranteed to pass the
/// relay's `validate_video_file()`.
///
/// Returns the path to a temp file. Caller must clean up.
fn transcode_to_mp4(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    // UUID-based temp path — unique across concurrent uploads.
    let output =
        std::env::temp_dir().join(format!("sprout-transcode-{}.mp4", uuid::Uuid::new_v4()));

    let result = run_ffmpeg_with_timeout(
        std::process::Command::new(ffmpeg)
            .args(["-y", "-loglevel", "error"]) // suppress progress spam — prevents stderr pipe deadlock
            .arg("-i")
            .arg(source) // OsStr — handles non-UTF-8 paths on Unix
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
            ])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        FFMPEG_TIMEOUT,
    )?;

    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.is_empty() && !l.starts_with("  "))
            .unwrap_or("unknown error");
        return Err(format!("Video conversion failed: {detail}"));
    }

    Ok(output)
}

/// Extract a single JPEG poster frame from a transcoded MP4 via ffmpeg.
///
/// Seeks to 1 second (avoids black leader frames), falls back to first frame
/// for videos shorter than 1 second. Output is scaled to 640px wide with even
/// dimensions. Returns the path to a temp JPEG. Caller must clean up.
///
/// Best-effort: returns `Err` on failure — callers should log and continue
/// without a poster rather than failing the entire video upload.
fn extract_poster_frame(
    mp4_path: &std::path::Path,
    ffmpeg: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let output = std::env::temp_dir().join(format!("sprout-poster-{}.jpg", uuid::Uuid::new_v4()));

    // Poster extraction is a single-frame decode — 30s is generous.
    let poster_timeout = std::time::Duration::from_secs(30);

    // Try seeking to 1s first (avoids black first frames from fade-ins).
    let result = run_ffmpeg_with_timeout(
        std::process::Command::new(ffmpeg)
            .args(["-y", "-loglevel", "error"])
            .arg("-ss")
            .arg("1")
            .arg("-i")
            .arg(mp4_path)
            .args(["-vframes", "1", "-vf", "scale=640:-2", "-q:v", "2"])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        poster_timeout,
    )?;

    // If seek to 1s failed (video shorter than 1s), retry from first frame.
    if !result.status.success()
        || !output.exists()
        || std::fs::metadata(&output).map_or(true, |m| m.len() == 0)
    {
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            eprintln!("sprout-desktop: poster seek-to-1s failed, trying first frame: {stderr}");
        }
        let _ = std::fs::remove_file(&output);
        let fallback = run_ffmpeg_with_timeout(
            std::process::Command::new(ffmpeg)
                .args(["-y", "-loglevel", "error"])
                .arg("-i")
                .arg(mp4_path)
                .args(["-vframes", "1", "-vf", "scale=640:-2", "-q:v", "2"])
                .arg(&output)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped()),
            poster_timeout,
        )?;

        if !fallback.status.success() || !output.exists() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            eprintln!("sprout-desktop: poster frame extraction failed: {stderr}");
            let _ = std::fs::remove_file(&output);
            return Err("ffmpeg could not extract a poster frame".to_string());
        }
    }

    Ok(output)
}

/// Transcode video and extract poster frame. Returns (video_bytes, Option<poster_bytes>).
///
/// Poster extraction is best-effort — if it fails, returns `None` for the poster
/// and the video bytes are still valid. All temp files are cleaned up.
fn transcode_and_extract_poster(
    source: &std::path::Path,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let ffmpeg_path = find_ffmpeg()?;
    let transcoded = transcode_to_mp4(source, &ffmpeg_path)?;

    // Extract poster from the transcoded file (not the original — guarantees decodability).
    let poster_bytes = match extract_poster_frame(&transcoded, &ffmpeg_path) {
        Ok(poster_path) => {
            let bytes = std::fs::read(&poster_path).ok();
            let _ = std::fs::remove_file(&poster_path);
            bytes
        }
        Err(e) => {
            eprintln!("sprout-desktop: poster extraction failed (non-fatal): {e}");
            None
        }
    };

    let video_bytes =
        std::fs::read(&transcoded).map_err(|e| format!("failed to read transcoded file: {e}"));
    let _ = std::fs::remove_file(&transcoded);

    Ok((video_bytes?, poster_bytes))
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
    let mut descriptor = do_upload(body, &mime, state).await?;

    if let Some(poster) = poster_bytes {
        match do_upload(poster, "image/jpeg", state).await {
            Ok(poster_desc) => descriptor.image = Some(poster_desc.url),
            Err(e) => eprintln!("sprout-desktop: poster upload failed (non-fatal): {e}"),
        }
    }

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
    app.dialog()
        .file()
        .add_filter(
            "Media",
            &[
                "jpg", "jpeg", "png", "gif", "webp", "mp4", "mov", "mkv", "webm", "avi",
            ],
        )
        .pick_files(move |paths| {
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
                std::env::temp_dir().join(format!("sprout-drop-{}", uuid::Uuid::new_v4()));
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
    } else {
        (data, None)
    };

    let mime = detect_and_validate_mime(&body)?;

    // Upload video first, then poster (best-effort).
    let mut descriptor = do_upload(body, &mime, &state).await?;

    if let Some(poster) = poster_bytes {
        match do_upload(poster, "image/jpeg", &state).await {
            Ok(poster_desc) => descriptor.image = Some(poster_desc.url),
            Err(e) => eprintln!("sprout-desktop: poster upload failed (non-fatal): {e}"),
        }
    }

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
    fn test_detect_and_validate_mime_rejects_text() {
        let text = b"hello world";
        assert!(detect_and_validate_mime(text).is_err());
    }

    #[test]
    fn test_is_video_file_mp4() {
        // Minimal ftyp box (MP4 magic bytes)
        let ftyp: &[u8] = &[
            0x00, 0x00, 0x00, 0x14, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0x00, 0x00,
            0x00, 0x00, b'i', b's', b'o', b'm',
        ];
        assert!(is_video_file(ftyp));
    }

    #[test]
    fn test_is_video_file_jpeg_is_not_video() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        assert!(!is_video_file(&jpeg));
    }

    #[test]
    fn test_is_video_file_empty() {
        assert!(!is_video_file(&[]));
    }

    #[test]
    fn test_find_ffmpeg_runs() {
        // This test verifies the function doesn't panic.
        // It may pass or fail depending on whether ffmpeg is installed.
        let _ = find_ffmpeg();
    }
}
