use futures_util::StreamExt;
use tauri::State;

use crate::app_state::AppState;
use crate::commands::export_util::save_bytes_with_dialog;
use crate::relay::{classify_request_error, relay_api_base_url_with_override, relay_error_message};

use super::media::{detect_and_validate_mime, sanitize_filename};

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
#[tauri::command]
pub async fn fetch_media_bytes(url: String, state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    let relay_base = relay_api_base_url_with_override(&state);
    validate_download_url(&url, &relay_base)?;

    let bytes = fetch_blob_bytes(&url, &state).await?;
    detect_and_validate_mime(&bytes)?;
    Ok(bytes)
}

/// Fetch blob bytes from a (pre-validated) relay media URL through the app's
/// HTTP client, enforcing the download size cap. The caller is responsible for
/// validating the URL origin and for any content-type checks on the result.
async fn fetch_blob_bytes(url: &str, state: &State<'_, AppState>) -> Result<Vec<u8>, String> {
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
        if content_length > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "file too large ({} MiB, max {} MiB)",
                content_length / (1024 * 1024),
                MAX_DOWNLOAD_BYTES / (1024 * 1024)
            ));
        }
    }

    // Stream the response with a running byte count to enforce the size cap
    // even when Content-Length is missing or dishonest.
    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify_request_error(&e))?;
        if bytes.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "file too large (max {} MiB)",
                MAX_DOWNLOAD_BYTES / (1024 * 1024)
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
