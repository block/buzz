//! Blossom BUD-02 response types.

use serde::{Deserialize, Serialize};

/// Blossom BlobDescriptor — returned by PUT /media/upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobDescriptor {
    /// Full URL to the blob.
    pub url: String,
    /// SHA-256 hex hash (64 chars).
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
    /// MIME type.
    #[serde(rename = "type")]
    pub mime_type: String,
    /// Unix timestamp of upload.
    pub uploaded: i64,
    /// Pixel dimensions ("WxH").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<String>,
    /// Blurhash string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    /// Thumbnail URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// Video duration in seconds. `None` for non-video blobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}
