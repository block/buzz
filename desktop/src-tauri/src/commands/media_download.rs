use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_state::AppState;
use crate::commands::export_util::save_bytes_with_dialog;
use crate::commands::media::{detect_and_validate_mime, sanitize_filename};
use crate::commands::personas::{
    decode_snapshot_from_bytes, MAX_SNAPSHOT_JSON_BYTES, MAX_SNAPSHOT_PNG_BYTES, PNG_MAGIC,
};
use crate::relay::{classify_request_error, relay_api_base_url_with_override, relay_error_message};

/// Maximum download size: 50 MiB. Prevents OOM from oversized responses.
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// Download request timeout.
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Validate that a URL is a legitimate relay media URL.
///
/// Ensures:
/// - URL scheme is `https` (or `http` for localhost dev)
/// - URL origin matches the relay base URL
/// - URL path matches `/media/{hash}.{ext}`
fn validate_download_url(url: &str, relay_base: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "invalid URL".to_string())?;
    let base = url::Url::parse(relay_base).map_err(|_| "invalid relay base URL".to_string())?;

    // Scheme must be https (allow http for localhost dev servers).
    match parsed.scheme() {
        "https" => {}
        "http" => {
            let host = parsed.host_str().unwrap_or("");
            if host != "localhost" && host != "127.0.0.1" && host != "[::1]" {
                return Err("download URL must use HTTPS".to_string());
            }
        }
        _ => return Err("download URL must use HTTPS".to_string()),
    }

    // Origin must match relay.
    if parsed.origin() != base.origin() {
        return Err("download URL must match the relay origin".to_string());
    }

    // Path must be /media/{filename}.
    let path = parsed.path();
    if !path.starts_with("/media/") {
        return Err("download URL must be a /media/ path".to_string());
    }

    Ok(())
}

/// Download an image from a URL and save it via a native save-file dialog.
#[tauri::command]
pub async fn download_image(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // SSRF protection: only allow downloads from the relay's /media/ path.
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    // Infer filename from the URL path (e.g. "abcdef123.jpg" from a Blossom URL).
    let filename = url::Url::parse(&url)
        .ok()
        .and_then(|u| {
            u.path_segments()?
                .next_back()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "image.png".to_string());

    // Derive extension for the save dialog filter.
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_string();

    let bytes = fetch_blob_bytes(&url, &state).await?;

    // Validate the downloaded content is actually a supported media type.
    detect_and_validate_mime(&bytes)?;

    save_bytes_with_dialog(&app, &filename, "Images", &[&ext], &bytes).await
}

/// Download an arbitrary file attachment from a relay `/media/` URL and save it
/// via a native save-file dialog.
///
/// The frontend supplies `filename` from the message's imeta `filename` field
/// (the URL path is only the content hash, so it carries no human-readable
/// name). We sanitize it defensively before using it as the suggested name.
///
/// Mirrors `download_image`'s SSRF and size protections, but uses a generic
/// "All Files" dialog filter and derives the extension from the supplied
/// filename rather than assuming an image.
#[tauri::command]
pub async fn download_file(
    url: String,
    filename: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // SSRF protection: only allow downloads from the relay's /media/ path.
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    // The imeta filename is the only human-readable name we have; sanitize it
    // so directory traversal / control characters can never reach the dialog.
    let filename = sanitize_filename(&filename);

    // Derive extension for the save dialog filter from the supplied filename.
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_string());

    let bytes = fetch_blob_bytes(&url, &state).await?;

    // Reuse the upload-side allow/deny policy: rejects executables, HTML, and
    // other types the relay would never have accepted, while permitting the
    // arbitrary `application/octet-stream` / text payloads that uploads allow.
    detect_and_validate_mime(&bytes)?;

    // Generic filter: an arbitrary attachment is not necessarily an image.
    let extensions: Vec<&str> = ext.as_deref().into_iter().collect();
    save_bytes_with_dialog(&app, &filename, "All Files", &extensions, &bytes).await
}

/// Fetch relay media bytes for the composer image editor.
///
/// The editor composites the image onto a canvas and needs pixel access.
/// Handing the webview raw bytes over IPC (which it wraps in a same-origin
/// `blob:` URL) keeps the canvas un-tainted without involving CORS — and
/// therefore without any media-proxy header or origin-gate changes.
///
/// Same SSRF validation, size cap, and content policy as the download
/// commands above.
///
/// Returns `tauri::ipc::Response` so the bytes cross IPC as a raw buffer
/// instead of a JSON number array (which would be ~3x the size to
/// serialize and deserialize at the 50 MiB cap).
#[tauri::command]
pub async fn fetch_media_bytes(
    url: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    let bytes = fetch_blob_bytes(&url, &state).await?;
    detect_and_validate_mime(&bytes)?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Copy an image from a relay media URL directly to the system clipboard.
///
/// Fetches the image, decodes it to RGBA8, and writes it to the clipboard via
/// `arboard`. Same SSRF validation, size cap, and content policy as the download
/// commands above.
///
/// `arboard` requires clipboard access on the main thread on macOS, so the
/// write is dispatched via `run_on_main_thread` and the result is relayed back
/// through a one-shot channel.
#[tauri::command]
pub async fn copy_image_to_clipboard(
    url: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    let bytes = fetch_blob_bytes(&url, &state).await?;
    detect_and_validate_mime(&bytes)?;

    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("failed to decode image: {e}"))?;

    // Guard against decompression bombs: a small compressed file can decode to
    // a huge RGBA buffer. Cap at 50 MiB (matching the download size cap).
    let pixels = img.width() as u64 * img.height() as u64;
    if pixels * 4 > MAX_DOWNLOAD_BYTES {
        return Err("image too large to copy to clipboard".to_string());
    }

    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let raw = rgba.into_raw();

    // arboard requires main-thread access on macOS. Use a sync channel so the
    // async command can await the result.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    app.run_on_main_thread(move || {
        let result = arboard::Clipboard::new()
            .map_err(|e| format!("clipboard error: {e}"))
            .and_then(|mut clipboard| {
                clipboard
                    .set_image(arboard::ImageData {
                        width,
                        height,
                        bytes: std::borrow::Cow::Owned(raw),
                    })
                    .map_err(|e| format!("clipboard error: {e}"))
            });
        // Ignore send errors — the receiver dropped only if the command was
        // cancelled, in which case nobody is waiting for the result.
        let _ = tx.send(result);
    })
    .map_err(|e| format!("main thread dispatch failed: {e}"))?;

    rx.recv()
        .map_err(|_| "clipboard result channel closed unexpectedly".to_string())?
}

/// Fetch blob bytes from a (pre-validated) relay media URL through the app's
/// HTTP client, enforcing the download size cap. The caller is responsible for
/// validating the URL origin and for any content-type checks on the result.
async fn fetch_blob_bytes(url: &str, state: &State<'_, AppState>) -> Result<Vec<u8>, String> {
    fetch_blob_bytes_with_cap(url, state, MAX_DOWNLOAD_BYTES).await
}

/// Core streaming fetcher with a caller-supplied byte cap.
async fn fetch_blob_bytes_with_cap(
    url: &str,
    state: &State<'_, AppState>,
    cap: u64,
) -> Result<Vec<u8>, String> {
    // Fetch bytes via the app's HTTP client (goes through WARP tunnel).
    let resp = state
        .http_client
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !resp.status().is_success() {
        return Err(relay_error_message(resp).await);
    }

    // Check Content-Length header upfront if present.
    if let Some(content_length) = resp.content_length() {
        if content_length > cap {
            return Err(format!(
                "file too large ({} MiB, max {} MiB)",
                content_length / (1024 * 1024),
                cap / (1024 * 1024)
            ));
        }
    }

    // Stream the response with a running byte count to enforce the size cap
    // even when Content-Length is missing or dishonest.
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify_request_error(&e))?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(format!("file too large (max {} MiB)", cap / (1024 * 1024)));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}

/// The snapshot file format inferred from the sanitized filename suffix.
/// Carries the format-specific byte cap used during bounded fetch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SnapshotFileKind {
    /// `.agent.json` — plaintext JSON; accepts memory; 5 MiB cap.
    Json,
    /// `.agent.png` — PNG with embedded metadata; no memory; 10 MiB cap.
    Png,
}

impl SnapshotFileKind {
    fn cap(self) -> u64 {
        match self {
            SnapshotFileKind::Json => MAX_SNAPSHOT_JSON_BYTES as u64,
            SnapshotFileKind::Png => MAX_SNAPSHOT_PNG_BYTES as u64,
        }
    }
}

/// Verify that the leading bytes of `bytes` are consistent with `kind`.
///
/// PNG magic (`\x89PNG`) is required for `Png` and must be absent for `Json`.
/// Returns an error with a clear message on mismatch so that `fetch_snapshot_bytes`
/// fails closed before any bytes reach the frontend.
fn ensure_bytes_match_kind(bytes: &[u8], kind: SnapshotFileKind) -> Result<(), String> {
    let has_png_magic = bytes.len() >= 4 && bytes[..4] == PNG_MAGIC;
    match kind {
        SnapshotFileKind::Png if !has_png_magic => {
            Err("format mismatch: filename is .agent.png but bytes are not a PNG".to_string())
        }
        SnapshotFileKind::Json if has_png_magic => {
            Err("format mismatch: filename is .agent.json but bytes are a PNG".to_string())
        }
        _ => Ok(()),
    }
}

/// Determine whether a sanitized filename is a valid agent snapshot candidate.
/// Returns the `SnapshotFileKind` for the extension, or an error.
fn snapshot_kind_for_filename(filename: &str) -> Result<SnapshotFileKind, String> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".agent.json") {
        Ok(SnapshotFileKind::Json)
    } else if lower.ends_with(".agent.png") {
        Ok(SnapshotFileKind::Png)
    } else {
        Err(format!(
            "\"{}\" is not a snapshot filename — expected .agent.json or .agent.png",
            filename
        ))
    }
}

/// Fetch and validate an agent snapshot attachment in memory.
///
/// Input validation (before HTTP):
/// - URL must be a valid same-relay `/media/` URL.
/// - Filename must end with `.agent.json` or `.agent.png`.
/// - `expected_sha256` and `expected_size` must be non-empty strings.
///
/// During fetch:
/// - Enforces a format-specific cap (5 MiB JSON, 10 MiB PNG) via
///   Content-Length header and streamed byte count.
///
/// Post-fetch validation (all must pass; returns an error on first failure):
/// 1. Byte length equals `expected_size`.
/// 2. SHA-256 hex of bytes equals `expected_sha256` (lowercase).
/// 3. `decode_snapshot_from_bytes` succeeds — bytes are a well-formed snapshot.
///
/// Returns `tauri::ipc::Response` so bytes cross IPC as a raw buffer rather
/// than a JSON number array (which would be ~3× the size at the 5–10 MiB cap).
#[tauri::command]
pub async fn fetch_snapshot_bytes(
    url: String,
    filename: String,
    expected_sha256: String,
    expected_size: usize,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    // ── Pre-fetch validation ──────────────────────────────────────────────
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    // Sanitize the filename and verify it is a recognised snapshot extension.
    let filename = sanitize_filename(&filename);
    let kind = snapshot_kind_for_filename(&filename)?;
    let cap = kind.cap();

    if expected_sha256.is_empty() {
        return Err("missing expected sha256 (imeta x field)".to_string());
    }
    if expected_sha256.len() != 64 || !expected_sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "invalid expected sha256 — must be a 64-hex-digit lowercase string".to_string(),
        );
    }
    if expected_size == 0 {
        return Err("missing or zero expected size (imeta size field)".to_string());
    }
    if expected_size as u64 > cap {
        return Err(format!(
            "declared size {} exceeds the {} MiB cap for this format",
            expected_size,
            cap / (1024 * 1024)
        ));
    }

    // ── Bounded fetch ─────────────────────────────────────────────────────
    let bytes = fetch_blob_bytes_with_cap(&url, &state, cap).await?;

    // ── Post-fetch validation ─────────────────────────────────────────────
    // 1. Byte length must equal the declared imeta size.
    if bytes.len() != expected_size {
        return Err(format!(
            "size mismatch: fetched {} bytes but imeta declared {}",
            bytes.len(),
            expected_size
        ));
    }

    // 2. SHA-256 must match the declared imeta x value.
    let actual_sha256 = hex::encode(Sha256::digest(&bytes));
    if actual_sha256 != expected_sha256.to_ascii_lowercase() {
        return Err("hash mismatch: fetched bytes do not match the declared SHA-256".to_string());
    }

    // 3. Byte magic must match the expected kind (filename → format).
    //    This prevents .agent.png delivering JSON bytes (including memory-bearing
    //    JSON) and .agent.json delivering PNG bytes.
    ensure_bytes_match_kind(&bytes, kind)?;

    // 4. Bytes must parse as a valid agent snapshot.  This rejects malformed
    //    payloads, memory-bearing PNGs, JSON with inconsistent memory fields,
    //    and any format/extension mismatch not caught by magic-byte check.
    decode_snapshot_from_bytes(&bytes).map_err(|e| format!("invalid snapshot: {e}"))?;

    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_kind_json_returns_json_kind_and_correct_cap() {
        let kind = snapshot_kind_for_filename("analyst.agent.json").unwrap();
        assert_eq!(kind, SnapshotFileKind::Json);
        assert_eq!(kind.cap(), MAX_SNAPSHOT_JSON_BYTES as u64);
    }

    #[test]
    fn snapshot_kind_png_returns_png_kind_and_correct_cap() {
        let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
        assert_eq!(kind, SnapshotFileKind::Png);
        assert_eq!(kind.cap(), MAX_SNAPSHOT_PNG_BYTES as u64);
    }

    #[test]
    fn snapshot_kind_plain_json_rejected() {
        assert!(snapshot_kind_for_filename("data.json").is_err());
    }

    #[test]
    fn snapshot_kind_deceptive_name_rejected() {
        // foo.agent.json.exe must not match .agent.json
        assert!(snapshot_kind_for_filename("foo.agent.json.exe").is_err());
    }

    #[test]
    fn snapshot_kind_plain_png_rejected() {
        assert!(snapshot_kind_for_filename("photo.png").is_err());
    }

    #[test]
    fn snapshot_kind_agent_json_only_rejected() {
        // "agent.json" without the leading dot — plain filename, not the suffix
        assert!(snapshot_kind_for_filename("agentjson").is_err());
    }

    // ── Focused boundary tests: format mismatch and consistency ──────────────
    //
    // These tests exercise the guard logic that fetch_snapshot_bytes applies
    // after the bounded fetch + hash check.  The validation has two layers:
    //
    //   1. Magic-byte kind check: filename kind (from snapshot_kind_for_filename)
    //      must match the actual byte format (PNG magic or absence of it).
    //   2. decode_snapshot_from_bytes: rejects malformed manifests including
    //      JSON with level:none + non-empty entries.
    //
    // We verify each rejection path directly — no live HTTP required.

    #[test]
    fn fetch_boundary_png_filename_with_json_bytes_rejected() {
        use crate::managed_agents::agent_snapshot::{
            encode_snapshot_json, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
            AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
        };
        let snapshot = AgentSnapshot {
            format: FORMAT_DISCRIMINATOR.to_string(),
            version: FORMAT_VERSION,
            definition: AgentSnapshotDefinition {
                name: "test".to_string(),
                system_prompt: None,
                runtime: None,
                model: None,
                provider: None,
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                name_pool: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
            },
            profile: AgentSnapshotProfile {
                display_name: "Test".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
                entries: vec![],
            },
        };
        let json_bytes = encode_snapshot_json(&snapshot).unwrap();
        // .agent.png filename → Png kind; JSON bytes must be rejected.
        let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
        let result = ensure_bytes_match_kind(&json_bytes, kind);
        assert!(
            result.is_err(),
            ".agent.png filename with JSON bytes must be rejected by the magic-byte guard"
        );
        assert!(
            result.unwrap_err().contains("not a PNG"),
            "error must describe the mismatch"
        );
    }

    #[test]
    fn fetch_boundary_png_filename_with_memory_bearing_json_bytes_rejected() {
        use crate::managed_agents::agent_snapshot::{
            encode_snapshot_json, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
            AgentSnapshotMemoryEntry, AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
        };
        // This is the trust-hole case: memory-bearing JSON delivered under a
        // .agent.png label to bypass the PNG no-memory policy.
        let snapshot = AgentSnapshot {
            format: FORMAT_DISCRIMINATOR.to_string(),
            version: FORMAT_VERSION,
            definition: AgentSnapshotDefinition {
                name: "test".to_string(),
                system_prompt: None,
                runtime: None,
                model: None,
                provider: None,
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                name_pool: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
            },
            profile: AgentSnapshotProfile {
                display_name: "Test".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: crate::managed_agents::agent_snapshot::MemoryLevel::Everything,
                entries: vec![AgentSnapshotMemoryEntry {
                    slug: "core".to_string(),
                    body: "Secret memory.".to_string(),
                }],
            },
        };
        let json_bytes = encode_snapshot_json(&snapshot).unwrap();
        let kind = snapshot_kind_for_filename("analyst.agent.png").unwrap();
        let result = ensure_bytes_match_kind(&json_bytes, kind);
        assert!(
            result.is_err(),
            ".agent.png filename with memory-bearing JSON bytes must be rejected"
        );
    }

    #[test]
    fn fetch_boundary_json_filename_with_png_bytes_rejected() {
        use crate::managed_agents::agent_snapshot::{
            encode_snapshot_png, AgentSnapshot, AgentSnapshotDefinition, AgentSnapshotMemory,
            AgentSnapshotProfile, FORMAT_DISCRIMINATOR, FORMAT_VERSION,
        };
        let snapshot = AgentSnapshot {
            format: FORMAT_DISCRIMINATOR.to_string(),
            version: FORMAT_VERSION,
            definition: AgentSnapshotDefinition {
                name: "test".to_string(),
                system_prompt: None,
                runtime: None,
                model: None,
                provider: None,
                parallelism: None,
                respond_to: None,
                respond_to_allowlist: vec![],
                name_pool: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
            },
            profile: AgentSnapshotProfile {
                display_name: "Test".to_string(),
                about: None,
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: crate::managed_agents::agent_snapshot::MemoryLevel::None,
                entries: vec![],
            },
        };
        let png_bytes = encode_snapshot_png(&snapshot, None).unwrap();
        // .agent.json filename → Json kind; PNG bytes must be rejected.
        let kind = snapshot_kind_for_filename("analyst.agent.json").unwrap();
        let result = ensure_bytes_match_kind(&png_bytes, kind);
        assert!(
            result.is_err(),
            ".agent.json filename with PNG bytes must be rejected by the magic-byte guard"
        );
        assert!(
            result.unwrap_err().contains("bytes are a PNG"),
            "error must describe the mismatch"
        );
    }

    #[test]
    fn decode_boundary_json_none_level_with_entries_rejected() {
        use crate::commands::personas::decode_snapshot_from_bytes;
        // Construct JSON bytes directly: level=none but entries non-empty.
        // encode_snapshot_json does not guard against this, so we can produce it.
        let raw = serde_json::json!({
            "format": "buzz-agent-snapshot",
            "version": 1,
            "definition": { "name": "test" },
            "profile": { "displayName": "Test" },
            "memory": {
                "level": "none",
                "entries": [{"slug": "core", "body": "leaked"}]
            }
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let result = decode_snapshot_from_bytes(&bytes);
        assert!(
            result.is_err(),
            "JSON with level:none + non-empty entries must be rejected by decode_snapshot_from_bytes"
        );
        assert!(
            result
                .unwrap_err()
                .contains("'none' but entries are present"),
            "error must describe the consistency violation"
        );
    }

    const RELAY_BASE: &str = "https://relay.example.com";

    #[test]
    fn test_validate_download_url_valid_relay_url() {
        assert!(validate_download_url(
            "https://relay.example.com/media/abcdef1234567890.jpg",
            RELAY_BASE,
        )
        .is_ok());
    }

    #[test]
    fn test_validate_download_url_valid_relay_url_png() {
        assert!(
            validate_download_url("https://relay.example.com/media/abc123.png", RELAY_BASE,)
                .is_ok()
        );
    }

    #[test]
    fn test_validate_download_url_non_relay_origin_rejected() {
        let result = validate_download_url("https://evil.example.com/media/abc123.jpg", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("relay origin"));
    }

    #[test]
    fn test_validate_download_url_private_ip_rejected() {
        let result = validate_download_url("http://169.254.169.254/latest/meta-data/", RELAY_BASE);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_download_url_loopback_rejected() {
        let result = validate_download_url("http://127.0.0.1/media/abc.jpg", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("relay origin"));
    }

    #[test]
    fn test_validate_download_url_localhost_allowed_for_localhost_relay() {
        assert!(validate_download_url(
            "http://localhost:3000/media/abc.jpg",
            "http://localhost:3000",
        )
        .is_ok());
    }

    #[test]
    fn test_validate_download_url_missing_media_path_rejected() {
        let result = validate_download_url("https://relay.example.com/other/abc.jpg", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("/media/"));
    }

    #[test]
    fn test_validate_download_url_non_https_scheme_rejected() {
        let result = validate_download_url("ftp://relay.example.com/media/abc.jpg", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[test]
    fn test_validate_download_url_http_non_localhost_rejected() {
        let result = validate_download_url("http://relay.example.com/media/abc.jpg", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[test]
    fn test_validate_download_url_root_path_rejected() {
        let result = validate_download_url("https://relay.example.com/", RELAY_BASE);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("/media/"));
    }
}
