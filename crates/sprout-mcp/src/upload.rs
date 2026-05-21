//! File upload pipeline for Sprout media (Blossom protocol).
//!
//! Reads a local file, validates it against size/type constraints, computes a SHA-256
//! hash, signs a kind:24242 Blossom auth event, PUTs the file to the relay's
//! `/media/upload` endpoint, and returns a [`BlobDescriptor`].

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hex;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use sha2::{Digest, Sha256};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum file size for image uploads (50 MB).
pub const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

/// Maximum file size for video uploads (500 MB).
pub const MAX_VIDEO_BYTES: u64 = 500 * 1024 * 1024;

/// MIME types we accept for upload.
const ALLOWED_MIMES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "video/mp4",
];

// ── Types ─────────────────────────────────────────────────────────────────────

/// Descriptor returned by the relay after a successful upload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlobDescriptor {
    /// Public URL of the uploaded blob.
    pub url: String,
    /// Hex-encoded SHA-256 of the file content.
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
    /// MIME type (e.g. `image/jpeg`).
    #[serde(rename = "type")]
    pub mime_type: String,
    /// Unix timestamp when the file was uploaded.
    pub uploaded: i64,
    /// Image dimensions as `<width>x<height>` (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<String>,
    /// Blurhash placeholder string (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    /// Thumbnail URL (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// Duration in seconds for video/audio (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// Errors that can occur during the upload pipeline.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The path does not exist on disk.
    #[error("file not found: {0}")]
    FileNotFound(String),
    /// The path exists but is not a regular file.
    #[error("not a file: {0}")]
    NotAFile(String),
    /// File exceeds the size limit for its type.
    #[error("file too large: {size} bytes (max {max})")]
    FileTooLarge {
        /// Actual file size.
        size: u64,
        /// Maximum allowed size.
        max: u64,
    },
    /// MIME type is not in the allowlist.
    #[error("unsupported file type: {0}")]
    UnsupportedFileType(String),
    /// Read bytes don't match metadata length.
    #[error("size mismatch: metadata says {expected}, read {actual}")]
    SizeMismatch {
        /// Size reported by filesystem metadata.
        expected: u64,
        /// Actual number of bytes read.
        actual: u64,
    },
    /// Filesystem I/O error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Nostr event signing failed.
    #[error("signing failed: {0}")]
    SigningFailed(String),
    /// Server returned a non-success status.
    #[error("upload rejected ({status}): {body}")]
    ServerRejected {
        /// HTTP status code.
        status: u16,
        /// Response body text.
        body: String,
    },
    /// HTTP transport error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// Could not parse the server's response as a BlobDescriptor.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

// ── Upload pipeline ───────────────────────────────────────────────────────────

/// Upload a local file to the Sprout relay's media endpoint.
///
/// Performs validation, SHA-256 hashing, Blossom auth signing, and the HTTP PUT.
/// Returns the relay's [`BlobDescriptor`] on success.
///
/// `auth_tag_json` is an optional NIP-OA auth tag (JSON-array string) sent as
/// the `x-auth-tag` header for relay membership delegation.
pub async fn upload_file(
    http: &reqwest::Client,
    keys: &Keys,
    relay_http_url: &str,
    server_domain: Option<&str>,
    file_path: &str,
    auth_tag_json: Option<&str>,
) -> Result<BlobDescriptor, UploadError> {
    // 1. Validate path exists
    let metadata = std::fs::metadata(file_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            UploadError::FileNotFound(file_path.to_string())
        } else {
            UploadError::Io(e)
        }
    })?;

    if !metadata.is_file() {
        return Err(UploadError::NotAFile(file_path.to_string()));
    }

    let expected_size = metadata.len();

    // Early rejection: no supported file type exceeds MAX_VIDEO_BYTES.
    // This prevents buffering hundreds of MB into RAM before MIME detection.
    if expected_size > MAX_VIDEO_BYTES {
        return Err(UploadError::FileTooLarge {
            size: expected_size,
            max: MAX_VIDEO_BYTES,
        });
    }

    // 2. Read file into memory
    let bytes = std::fs::read(file_path)?;
    let actual_size = bytes.len() as u64;

    // Post-read size check
    if actual_size != expected_size {
        return Err(UploadError::SizeMismatch {
            expected: expected_size,
            actual: actual_size,
        });
    }

    // 3. Detect MIME via magic bytes
    let mime = infer::get(&bytes)
        .map(|t| t.mime_type())
        .unwrap_or("application/octet-stream");

    if !ALLOWED_MIMES.contains(&mime) {
        return Err(UploadError::UnsupportedFileType(mime.to_string()));
    }

    // 4. Pre-check file size against type-specific limits
    let max_size = if mime.starts_with("video/") {
        MAX_VIDEO_BYTES
    } else {
        MAX_IMAGE_BYTES
    };

    if actual_size > max_size {
        return Err(UploadError::FileTooLarge {
            size: actual_size,
            max: max_size,
        });
    }

    // 5. Compute SHA-256
    let sha256 = hex::encode(Sha256::digest(&bytes));

    // 6. Sign Blossom auth event (kind:24242)
    let now = Timestamp::now().as_u64();
    let expiry = if mime.starts_with("video/") {
        3600
    } else {
        600
    };
    let exp_str = (now + expiry).to_string();

    let mut tags = vec![
        Tag::parse(["t", "upload"]).map_err(|e| UploadError::SigningFailed(e.to_string()))?,
        Tag::parse(["x", &sha256]).map_err(|e| UploadError::SigningFailed(e.to_string()))?,
        Tag::parse(["expiration", &exp_str])
            .map_err(|e| UploadError::SigningFailed(e.to_string()))?,
    ];
    if let Some(domain) = server_domain {
        tags.push(
            Tag::parse(["server", domain])
                .map_err(|e| UploadError::SigningFailed(e.to_string()))?,
        );
    }

    let auth_event = EventBuilder::new(Kind::from(24242), "Upload file")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| UploadError::SigningFailed(e.to_string()))?;

    // 7. Base64url encode the auth event
    let auth_header = format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(auth_event.as_json().as_bytes())
    );

    // 8. HTTP PUT — with a generous per-request timeout.
    // The shared reqwest client has a 10s timeout suitable for REST API calls,
    // but uploads can take minutes for large files. Override per-request.
    let upload_timeout = if mime.starts_with("video/") {
        std::time::Duration::from_secs(600) // 10 min for video (up to 500 MB)
    } else {
        std::time::Duration::from_secs(120) // 2 min for images (up to 50 MB)
    };

    let url = format!("{}/media/upload", relay_http_url.trim_end_matches('/'));
    let mut req = http
        .put(&url)
        .timeout(upload_timeout)
        .header("Authorization", &auth_header)
        .header("Content-Type", mime)
        .header("X-SHA-256", &sha256);
    if let Some(tag) = auth_tag_json {
        req = req.header("x-auth-tag", tag);
    }
    let resp = req.body(bytes).send().await?;

    // 9. Handle response
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(UploadError::ServerRejected {
            status: status.as_u16(),
            body,
        });
    }

    let body = resp.text().await?;
    serde_json::from_str::<BlobDescriptor>(&body)
        .map_err(|e| UploadError::InvalidResponse(format!("{e}: {body}")))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a NIP-92 `imeta` tag from a [`BlobDescriptor`].
///
/// The returned `Vec<String>` is suitable for passing to `Tag::parse`.
pub fn build_imeta_tag(d: &BlobDescriptor) -> Vec<String> {
    let mut tag = vec![
        "imeta".to_string(),
        format!("url {}", d.url),
        format!("m {}", d.mime_type),
        format!("x {}", d.sha256),
        format!("size {}", d.size),
    ];
    if let Some(ref dim) = d.dim {
        tag.push(format!("dim {dim}"));
    }
    if let Some(ref bh) = d.blurhash {
        tag.push(format!("blurhash {bh}"));
    }
    if let Some(ref th) = d.thumb {
        tag.push(format!("thumb {th}"));
    }
    if let Some(dur) = d.duration {
        tag.push(format!("duration {dur}"));
    }
    tag
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn image_descriptor() -> BlobDescriptor {
        BlobDescriptor {
            url: "https://relay.example.com/media/abc123.jpg".to_string(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            size: 12345,
            mime_type: "image/jpeg".to_string(),
            uploaded: 1700000000,
            dim: Some("1920x1080".to_string()),
            blurhash: Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string()),
            thumb: Some("https://relay.example.com/media/abc123_thumb.jpg".to_string()),
            duration: None,
        }
    }

    fn video_descriptor() -> BlobDescriptor {
        BlobDescriptor {
            url: "https://relay.example.com/media/vid456.mp4".to_string(),
            sha256: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
            size: 5_000_000,
            mime_type: "video/mp4".to_string(),
            uploaded: 1700000000,
            dim: Some("1280x720".to_string()),
            blurhash: None,
            thumb: None,
            duration: Some(42.5),
        }
    }

    fn minimal_descriptor() -> BlobDescriptor {
        BlobDescriptor {
            url: "https://relay.example.com/media/min.png".to_string(),
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            size: 100,
            mime_type: "image/png".to_string(),
            uploaded: 1700000000,
            dim: None,
            blurhash: None,
            thumb: None,
            duration: None,
        }
    }

    #[test]
    fn test_build_imeta_tag_image() {
        let d = image_descriptor();
        let tag = build_imeta_tag(&d);

        assert_eq!(tag[0], "imeta");
        assert_eq!(tag[1], "url https://relay.example.com/media/abc123.jpg");
        assert_eq!(tag[2], "m image/jpeg");
        assert_eq!(
            tag[3],
            "x e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(tag[4], "size 12345");
        assert_eq!(tag[5], "dim 1920x1080");
        assert_eq!(tag[6], "blurhash LEHV6nWB2yk8pyo0adR*.7kCMdnj");
        assert_eq!(
            tag[7],
            "thumb https://relay.example.com/media/abc123_thumb.jpg"
        );
        // No duration for images
        assert_eq!(tag.len(), 8);
    }

    #[test]
    fn test_build_imeta_tag_video() {
        let d = video_descriptor();
        let tag = build_imeta_tag(&d);

        assert_eq!(tag[0], "imeta");
        assert_eq!(tag[1], "url https://relay.example.com/media/vid456.mp4");
        assert_eq!(tag[2], "m video/mp4");
        assert_eq!(
            tag[3],
            "x abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        );
        assert_eq!(tag[4], "size 5000000");
        assert_eq!(tag[5], "dim 1280x720");
        // No blurhash or thumb
        assert_eq!(tag[6], "duration 42.5");
        assert_eq!(tag.len(), 7);
    }

    #[test]
    fn test_build_imeta_tag_minimal() {
        let d = minimal_descriptor();
        let tag = build_imeta_tag(&d);

        assert_eq!(tag.len(), 5);
        assert_eq!(tag[0], "imeta");
        assert_eq!(tag[1], "url https://relay.example.com/media/min.png");
        assert_eq!(tag[2], "m image/png");
        assert_eq!(
            tag[3],
            "x 0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(tag[4], "size 100");
    }

    #[test]
    fn test_mime_allowlist() {
        // Allowed types
        assert!(ALLOWED_MIMES.contains(&"image/jpeg"));
        assert!(ALLOWED_MIMES.contains(&"image/png"));
        assert!(ALLOWED_MIMES.contains(&"image/gif"));
        assert!(ALLOWED_MIMES.contains(&"image/webp"));
        assert!(ALLOWED_MIMES.contains(&"video/mp4"));

        // Rejected types
        assert!(!ALLOWED_MIMES.contains(&"application/pdf"));
        assert!(!ALLOWED_MIMES.contains(&"text/plain"));
        assert!(!ALLOWED_MIMES.contains(&"image/svg+xml"));
        assert!(!ALLOWED_MIMES.contains(&"video/webm"));
        assert!(!ALLOWED_MIMES.contains(&"application/octet-stream"));
    }

    #[test]
    fn test_file_size_limits() {
        // Image limit: 50 MB
        assert_eq!(MAX_IMAGE_BYTES, 50 * 1024 * 1024);
        assert_eq!(MAX_IMAGE_BYTES, 52_428_800);

        // Video limit: 500 MB
        assert_eq!(MAX_VIDEO_BYTES, 500 * 1024 * 1024);
        assert_eq!(MAX_VIDEO_BYTES, 524_288_000);

        // Video limit is 10x image limit
        assert_eq!(MAX_VIDEO_BYTES, MAX_IMAGE_BYTES * 10);
    }
}
