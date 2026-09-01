use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::app_state::AppState;
use crate::commands::clipboard::with_clipboard;
use crate::commands::export_util::save_bytes_with_dialog;
use crate::commands::media::{detect_and_validate_mime, mint_media_get_auth, sanitize_filename};
use crate::commands::{
    personas::{
        parse_snapshot_payload_from_bytes, MAX_SNAPSHOT_JSON_BYTES, MAX_SNAPSHOT_PNG_BYTES,
        PNG_MAGIC,
    },
    team_snapshot::{
        decode_team_snapshot_from_bytes, MAX_TEAM_SNAPSHOT_JSON_BYTES, MAX_TEAM_SNAPSHOT_PNG_BYTES,
    },
};
use crate::relay::{classify_request_error, relay_api_base_url_with_override, relay_error_message};

/// Maximum download size: 50 MiB. Prevents OOM from oversized responses.
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// Maximum markdown-document size for the in-app viewer: 2 MiB.
///
/// This is the *enforcement* boundary for the viewer — the frontend's
/// matching `MAX_MARKDOWN_DOC_BYTES` pre-gate reads the untrusted imeta
/// `size` field and is UX-only (a forged or absent size must not buy a
/// larger fetch). Keep in sync with
/// `desktop/src/shared/ui/markdown/markdownDocFile.ts`.
const MAX_MARKDOWN_DOC_BYTES: u64 = 2 * 1024 * 1024;

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

/// Fetch a markdown document attachment for the in-app viewer, enforcing the
/// viewer's 2 MiB ceiling natively during the fetch.
///
/// The generic `fetch_media_bytes` command streams up to the 50 MiB download
/// cap; routing the viewer through it would let an attachment with a forged
/// or absent imeta `size` occupy 50 MiB of memory and IPC before the
/// frontend's decoder rejected it. This command makes the 2 MiB limit a
/// native boundary: Content-Length over the cap is refused before the body
/// is read, and the streamed byte count aborts mid-transfer when the header
/// is missing or dishonest.
///
/// Deliberately skips `detect_and_validate_mime`: the bytes are only ever
/// strictly UTF-8-decoded and rendered through the escaping markdown
/// pipeline (never handed to the webview as a document), and a legitimate
/// `.md` file may open with bytes that sniff as a blocked type — e.g. an
/// SVG snippet pasted at the top of the file. Binary payloads fail the
/// frontend's fatal UTF-8 decode and fall back to download.
#[tauri::command]
pub async fn fetch_markdown_doc_bytes(
    url: String,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    let bytes = fetch_blob_bytes_with_cap(&url, &state, MAX_MARKDOWN_DOC_BYTES).await?;
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
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let result = with_clipboard(&clipboard_app, |clipboard| {
            clipboard.set_image(arboard::ImageData {
                width,
                height,
                bytes: std::borrow::Cow::Owned(raw),
            })
        });
        // Ignore send errors — the receiver dropped only if the command was
        // cancelled, in which case nobody is waiting for the result.
        let _ = tx.send(result);
    })
    .map_err(|e| format!("main thread dispatch failed: {e}"))?;

    rx.recv()
        .map_err(|_| "clipboard result channel closed unexpectedly".to_string())?
}

/// Write text to the system clipboard through the native shell.
///
/// WebKit can revoke browser clipboard permission after a user action awaits a
/// long-running operation such as snapshot encoding and upload. Keeping the
/// delayed write in the native layer makes that flow reliable on macOS.
#[tauri::command]
pub async fn copy_text_to_clipboard(
    text: String,
    html: Option<String>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let result = with_clipboard(&clipboard_app, |clipboard| {
            if let Some(html) = html {
                clipboard.set_html(html, Some(text))
            } else {
                clipboard.set_text(text)
            }
        });
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

/// The command-facing error for a media-fetch response status, or `None` if
/// the status is success and the body should be read.
///
/// A no-redirect client surfaces a relay 3xx as a redirection status rather
/// than following it; that is reported explicitly as a redirect (not a bare
/// "relay returned 302") so the failure is actionable and cannot be mistaken
/// for an ordinary relay error — following it would forward the minted media
/// auth header across origins. Pulled out of `fetch_blob_bytes_with_cap` so
/// the redirect-refusal message is unit-testable without a Tauri `State`.
fn redirect_refusal_error(status: reqwest::StatusCode) -> Option<String> {
    status.is_redirection().then(|| {
        format!(
            "media fetch refused: relay returned a {status} redirect, which is \
             not followed for authenticated downloads (redirect-hop SSRF guard)"
        )
    })
}

/// Core streaming fetcher with a caller-supplied byte cap.
async fn fetch_blob_bytes_with_cap(
    url: &str,
    state: &State<'_, AppState>,
    cap: u64,
) -> Result<Vec<u8>, String> {
    // Fetch bytes via the no-redirect media client (goes through the VPN tunnel).
    // A no-redirect client keeps the minted media auth token from being
    // forwarded across origins by a relay-issued 3xx (redirect-hop SSRF); a
    // 3xx is returned verbatim and rejected by the `is_success` check below.
    let mut req = state.media_fetch_client.get(url).timeout(DOWNLOAD_TIMEOUT);

    // Every caller pre-validates `url` against the relay origin via
    // `validate_download_url`, satisfying the mint_media_get_auth safety
    // contract (the token never leaves the relay origin).
    let relay_base = relay_api_base_url_with_override(state);
    if let Some(auth) = mint_media_get_auth(state, &relay_base) {
        req = req.header("authorization", auth);
    }

    let resp = req.send().await.map_err(|e| classify_request_error(&e))?;

    if let Some(err) = redirect_refusal_error(resp.status()) {
        return Err(err);
    }

    if !resp.status().is_success() {
        return Err(relay_error_message(resp).await);
    }

    // Check Content-Length header upfront if present.
    if let Some(err) = declared_length_refusal_error(resp.content_length(), cap) {
        return Err(err);
    }

    // Stream the response with a running byte count to enforce the size cap
    // even when Content-Length is missing or dishonest.
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify_request_error(&e))?;
        append_chunk_within_cap(&mut bytes, &chunk, cap)?;
    }

    Ok(bytes)
}

/// The refusal for a declared Content-Length that exceeds the byte cap, or
/// `None` when the header is absent or within bounds. An absent header is
/// allowed through — `append_chunk_within_cap` still enforces the cap on the
/// streamed bytes, so a missing or dishonest length never buys a larger
/// download. Pulled out of `fetch_blob_bytes_with_cap` so the pre-body
/// refusal is unit-testable without a Tauri `State`.
fn declared_length_refusal_error(content_length: Option<u64>, cap: u64) -> Option<String> {
    let content_length = content_length?;
    (content_length > cap).then(|| {
        format!(
            "file too large ({} MiB, max {} MiB)",
            content_length / (1024 * 1024),
            cap / (1024 * 1024)
        )
    })
}

/// Append a response chunk to the accumulator, refusing once the running
/// total would exceed `cap`. This is the enforcement point that holds even
/// when Content-Length is missing or dishonest: the transfer aborts
/// mid-stream instead of buffering past the cap. Pulled out of
/// `fetch_blob_bytes_with_cap` so the cutoff is unit-testable without a
/// Tauri `State`.
fn append_chunk_within_cap(bytes: &mut Vec<u8>, chunk: &[u8], cap: u64) -> Result<(), String> {
    if bytes.len() as u64 + chunk.len() as u64 > cap {
        return Err(format!("file too large (max {} MiB)", cap / (1024 * 1024)));
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

/// The snapshot file format inferred from the sanitized filename suffix.
/// Carries the format-specific byte cap used during bounded fetch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SnapshotFileKind {
    /// `.agent.json` — plaintext JSON; accepts memory; 5 MiB cap.
    AgentJson,
    /// `.agent.png` — PNG with embedded metadata; 10 MiB cap.
    AgentPng,
    /// `.team.json` — team template; 25 MiB cap.
    TeamJson,
    /// `.team.png` — team template PNG; 50 MiB cap.
    TeamPng,
}

impl SnapshotFileKind {
    fn cap(self) -> u64 {
        match self {
            SnapshotFileKind::AgentJson => MAX_SNAPSHOT_JSON_BYTES as u64,
            SnapshotFileKind::AgentPng => MAX_SNAPSHOT_PNG_BYTES as u64,
            SnapshotFileKind::TeamJson => MAX_TEAM_SNAPSHOT_JSON_BYTES as u64,
            SnapshotFileKind::TeamPng => MAX_TEAM_SNAPSHOT_PNG_BYTES as u64,
        }
    }

    fn is_png(self) -> bool {
        matches!(self, SnapshotFileKind::AgentPng | SnapshotFileKind::TeamPng)
    }

    fn label(self) -> &'static str {
        match self {
            SnapshotFileKind::AgentJson => ".agent.json",
            SnapshotFileKind::AgentPng => ".agent.png",
            SnapshotFileKind::TeamJson => ".team.json",
            SnapshotFileKind::TeamPng => ".team.png",
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
    if kind.is_png() && !has_png_magic {
        Err(format!(
            "format mismatch: filename is {} but bytes are not a PNG",
            kind.label()
        ))
    } else if !kind.is_png() && has_png_magic {
        Err(format!(
            "format mismatch: filename is {} but bytes are a PNG",
            kind.label()
        ))
    } else {
        Ok(())
    }
}

/// Determine whether a sanitized filename is a valid agent snapshot candidate.
/// Returns the `SnapshotFileKind` for the extension, or an error.
fn snapshot_kind_for_filename(filename: &str) -> Result<SnapshotFileKind, String> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".agent.json") {
        Ok(SnapshotFileKind::AgentJson)
    } else if lower.ends_with(".agent.png") {
        Ok(SnapshotFileKind::AgentPng)
    } else if lower.ends_with(".team.json") {
        Ok(SnapshotFileKind::TeamJson)
    } else if lower.ends_with(".team.png") {
        Ok(SnapshotFileKind::TeamPng)
    } else {
        Err(format!(
            "\"{}\" is not a snapshot filename — expected .agent.json, .agent.png, .team.json, or .team.png",
            filename
        ))
    }
}

/// Reject a metadata-declared size before opening the media stream.
///
/// Keeping this as a pure helper makes the per-kind cap a testable production
/// boundary, rather than relying on a duplicated test-side comparison.
fn ensure_declared_size_within_cap(
    expected_size: usize,
    kind: SnapshotFileKind,
) -> Result<(), String> {
    if expected_size as u64 > kind.cap() {
        return Err(format!(
            "declared size {} exceeds the {} MiB cap for this format",
            expected_size,
            kind.cap() / (1024 * 1024)
        ));
    }
    Ok(())
}

/// Fetch and validate an agent or team snapshot attachment in memory.
///
/// Input validation (before HTTP):
/// - URL must be a valid same-relay `/media/` URL.
/// - Filename must end case-insensitively with `.agent.json`, `.agent.png`,
///   `.team.json`, or `.team.png`.
/// - `expected_sha256` and `expected_size` must be non-empty strings.
///
/// During fetch, `SnapshotFileKind::cap()` enforces the kind-specific cap via
/// Content-Length and streamed byte count: 5 MiB JSON / 10 MiB PNG for agents,
/// or 25 MiB JSON / 50 MiB PNG for teams.
///
/// Post-fetch validation (all must pass; returns an error on first failure):
/// 1. Byte length equals `expected_size`.
/// 2. SHA-256 hex of bytes equals `expected_sha256` (lowercase).
/// 3. The byte magic matches the filename-selected kind.
/// 4. Agent kinds pass `decode_snapshot_from_bytes`; team kinds pass
///    `decode_team_snapshot_from_bytes`.
///
/// Returns `tauri::ipc::Response` so bytes cross IPC as a raw buffer rather
/// than a JSON number array (which would be ~3× the size at the applicable cap).
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
    ensure_declared_size_within_cap(expected_size, kind)?;

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

    // 3. Byte magic must match the expected kind (filename → format), so a
    //    filename cannot select a JSON or PNG parser for the other format.
    ensure_bytes_match_kind(&bytes, kind)?;

    // 4. Bytes must parse as the snapshot type selected by the filename.
    //    Team parsing rejects retired flat JSON and persona-pack ZIP inputs
    //    before anything reaches the frontend. Agent kinds accept both plain
    //    manifests and structurally valid locked (encrypted) card envelopes —
    //    transit validation never decrypts; unlock happens at import time.
    match kind {
        SnapshotFileKind::AgentJson | SnapshotFileKind::AgentPng => {
            parse_snapshot_payload_from_bytes(&bytes)
                .map_err(|e| format!("invalid agent snapshot: {e}"))?;
        }
        SnapshotFileKind::TeamJson | SnapshotFileKind::TeamPng => {
            decode_team_snapshot_from_bytes(&bytes)
                .map_err(|e| format!("invalid team snapshot: {e}"))?;
        }
    }

    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
#[path = "media_download_tests.rs"]
mod tests;
