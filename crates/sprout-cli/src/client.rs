use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use sha2::{Digest, Sha256};

use crate::error::CliError;

// ---------------------------------------------------------------------------
// Blob / Media types
// ---------------------------------------------------------------------------

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

/// Build an `imeta` tag array from a BlobDescriptor (NIP-92 media metadata).
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

/// MIME types accepted for upload.
const ALLOWED_MIMES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "video/mp4",
];

/// Maximum file size for image uploads (50 MB).
const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

/// Maximum file size for video uploads (500 MB).
const MAX_VIDEO_BYTES: u64 = 500 * 1024 * 1024;

// ---------------------------------------------------------------------------
// NIP-98 HTTP Auth
// ---------------------------------------------------------------------------

/// Sign a NIP-98 HTTP auth event (kind:27235) and return the Authorization header value.
///
/// The event includes:
/// - `u` tag: the full request URL
/// - `method` tag: HTTP method (GET, POST, PUT, DELETE)
/// - `payload` tag: SHA-256 hex of the request body (if present)
fn sign_nip98(
    keys: &Keys,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<String, CliError> {
    let mut tags = vec![
        Tag::parse(&["u", url]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(&["method", method]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        // Nonce prevents replay rejection for rapid-fire requests with identical bodies.
        Tag::parse(&["nonce", &uuid::Uuid::new_v4().to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];
    if let Some(b) = body {
        let hash = hex::encode(Sha256::digest(b));
        tags.push(
            Tag::parse(&["payload", &hash])
                .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        );
    }
    let event = EventBuilder::new(Kind::Custom(27235), "", tags)
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("NIP-98 signing failed: {e}")))?;
    let json = event.as_json();
    Ok(format!("Nostr {}", B64.encode(json.as_bytes())))
}

// ---------------------------------------------------------------------------
// SproutClient
// ---------------------------------------------------------------------------

pub struct SproutClient {
    http: reqwest::Client,
    relay_url: String, // base URL, no trailing slash, e.g. "https://relay.sprout.place"
    keys: Keys,
    /// Optional NIP-OA auth tag injected into every signed event.
    auth_tag: Option<Tag>,
    /// Raw JSON of the auth tag for the `x-auth-tag` HTTP header.
    auth_tag_json: Option<String>,
}

impl SproutClient {
    pub fn new(
        relay_url: String,
        keys: Keys,
        auth_tag: Option<Tag>,
        auth_tag_json: Option<String>,
    ) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| CliError::Other(e.to_string()))?;
        Ok(Self {
            http,
            relay_url,
            keys,
            auth_tag,
            auth_tag_json,
        })
    }

    /// Get the keypair.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Get the relay base URL.
    #[allow(dead_code)]
    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    /// Sign an event builder, injecting the NIP-OA auth tag if configured.
    ///
    /// All event creation should go through this method to ensure consistent
    /// auth tag injection. Callers MUST NOT add `auth` tags to the builder
    /// before calling this method.
    pub fn sign_event(&self, builder: EventBuilder) -> Result<nostr::Event, CliError> {
        let builder = if let Some(ref tag) = self.auth_tag {
            builder.add_tags([tag.clone()])
        } else {
            builder
        };
        let event = builder
            .sign_with_keys(&self.keys)
            .map_err(|e| CliError::Other(format!("signing failed: {e}")))?;

        // Enforce: auth tags may only come from self.auth_tag injection.
        let auth_count = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .count();
        let expected = if self.auth_tag.is_some() { 1 } else { 0 };
        if auth_count != expected {
            return Err(CliError::Other(format!(
                "event has {auth_count} auth tags — expected {expected}; \
                 callers must not add auth tags manually"
            )));
        }

        Ok(event)
    }

    /// Attach the `x-auth-tag` header if configured (NIP-OA relay membership delegation).
    fn with_auth_tag(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.auth_tag_json {
            Some(ref json) => req.header("x-auth-tag", json),
            None => req,
        }
    }

    // -----------------------------------------------------------------------
    // HTTP Bridge: POST /query
    // -----------------------------------------------------------------------

    /// Execute a one-shot query via the HTTP bridge.
    /// `filter` is a Nostr filter object (will be wrapped in an array).
    /// Returns the raw JSON response (array of events).
    pub async fn query(&self, filter: &serde_json::Value) -> Result<String, CliError> {
        let url = format!("{}/query", self.relay_url);
        let body_bytes = serde_json::to_vec(&[filter])
            .map_err(|e| CliError::Other(format!("filter serialization failed: {e}")))?;
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&body_bytes))?;

        let req = self
            .http
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .body(body_bytes);
        let resp = self.with_auth_tag(req).send().await?;

        self.handle_response(resp).await
    }

    /// Execute a one-shot count via the HTTP bridge.
    /// Returns the count as a JSON string.
    #[allow(dead_code)]
    pub async fn count(&self, filter: &serde_json::Value) -> Result<String, CliError> {
        let url = format!("{}/count", self.relay_url);
        let body_bytes = serde_json::to_vec(&[filter])
            .map_err(|e| CliError::Other(format!("filter serialization failed: {e}")))?;
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&body_bytes))?;

        let req = self
            .http
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .body(body_bytes);
        let resp = self.with_auth_tag(req).send().await?;

        self.handle_response(resp).await
    }

    // -----------------------------------------------------------------------
    // HTTP Bridge: POST /events
    // -----------------------------------------------------------------------

    /// Submit a signed Nostr event via POST /events.
    pub async fn submit_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let url = format!("{}/events", self.relay_url);
        let body_bytes = serde_json::to_vec(&event)
            .map_err(|e| CliError::Other(format!("event serialization failed: {e}")))?;
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&body_bytes))?;

        let req = self
            .http
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .body(body_bytes);
        let resp = self.with_auth_tag(req).send().await?;

        self.handle_response(resp).await
    }

    // -----------------------------------------------------------------------
    // File upload (Blossom protocol)
    // -----------------------------------------------------------------------

    /// Upload a file to the relay's Blossom endpoint.
    /// Returns a BlobDescriptor on success.
    pub async fn upload_file(&self, file_path: &str) -> Result<BlobDescriptor, CliError> {
        // 1. Read file — validate it exists and is a regular file
        let metadata = std::fs::metadata(file_path)
            .map_err(|e| CliError::Other(format!("cannot access {file_path}: {e}")))?;
        if !metadata.is_file() {
            return Err(CliError::Usage(format!("{file_path} is not a file")));
        }

        let bytes = std::fs::read(file_path)
            .map_err(|e| CliError::Other(format!("failed to read {file_path}: {e}")))?;

        // 2. Detect MIME from magic bytes
        let mime = infer::get(&bytes)
            .map(|t| t.mime_type().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        if !ALLOWED_MIMES.contains(&mime.as_str()) {
            return Err(CliError::Usage(format!("unsupported file type: {mime}")));
        }

        // 3. Size check
        let max = if mime.starts_with("video/") {
            MAX_VIDEO_BYTES
        } else {
            MAX_IMAGE_BYTES
        };
        if bytes.len() as u64 > max {
            return Err(CliError::Usage(format!(
                "file too large: {} bytes (max {})",
                bytes.len(),
                max
            )));
        }

        // 4. SHA-256
        let sha256 = hex::encode(Sha256::digest(&bytes));

        // 5. Sign Blossom auth event (kind:24242)
        use nostr::Timestamp;
        let now = Timestamp::now().as_u64();
        let expiry = if mime.starts_with("video/") {
            3600
        } else {
            600
        };
        let exp_str = (now + expiry).to_string();

        let mut blossom_tags = vec![
            Tag::parse(&["t", "upload"]).map_err(|e| CliError::Other(e.to_string()))?,
            Tag::parse(&["x", &sha256]).map_err(|e| CliError::Other(e.to_string()))?,
            Tag::parse(&["expiration", &exp_str]).map_err(|e| CliError::Other(e.to_string()))?,
        ];
        // Extract server domain from relay URL for BUD-11 server tag
        if let Ok(parsed) = url::Url::parse(&self.relay_url) {
            if let Some(host) = parsed.host_str() {
                let domain = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                blossom_tags.push(
                    Tag::parse(&["server", &domain]).map_err(|e| CliError::Other(e.to_string()))?,
                );
            }
        }

        let auth_event = EventBuilder::new(Kind::from(24242), "Upload file", blossom_tags)
            .sign_with_keys(&self.keys)
            .map_err(|e| CliError::Other(format!("signing failed: {e}")))?;

        // 6. Base64url encode the auth event for the header
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let auth_header = format!(
            "Nostr {}",
            URL_SAFE_NO_PAD.encode(auth_event.as_json().as_bytes())
        );

        // 7. PUT request to /media/upload — with generous per-request timeout.
        let upload_timeout = if mime.starts_with("video/") {
            Duration::from_secs(600)
        } else {
            Duration::from_secs(120)
        };
        let url = format!("{}/media/upload", self.relay_url);
        let req = self
            .http
            .put(&url)
            .timeout(upload_timeout)
            .header("Authorization", &auth_header)
            .header("Content-Type", &mime)
            .header("X-SHA-256", &sha256);

        let resp = self.with_auth_tag(req).body(bytes).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Relay { status, body });
        }

        resp.json::<BlobDescriptor>()
            .await
            .map_err(|e| CliError::Other(format!("invalid upload response: {e}")))
    }

    // -----------------------------------------------------------------------
    // Response handling
    // -----------------------------------------------------------------------

    async fn handle_response(&self, resp: reqwest::Response) -> Result<String, CliError> {
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .or_else(|| v.get("message"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or(body);
            return Err(CliError::Relay {
                status,
                body: message,
            });
        }
        Ok(resp.text().await?)
    }
}

// ---------------------------------------------------------------------------
// URL normalization
// ---------------------------------------------------------------------------

/// Normalize a relay URL: ws:// → http://, wss:// → https://, strip trailing slash.
/// SPROUT_RELAY_URL may be ws/wss (copied from MCP config).
pub fn normalize_relay_url(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}
