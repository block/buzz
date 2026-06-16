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
        Tag::parse(["u", url]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(["method", method]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        // Nonce prevents replay rejection for rapid-fire requests with identical bodies.
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];
    if let Some(b) = body {
        let hash = hex::encode(Sha256::digest(b));
        tags.push(
            Tag::parse(["payload", &hash])
                .map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        );
    }
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("NIP-98 signing failed: {e}")))?;
    let json = event.as_json();
    Ok(format!("Nostr {}", B64.encode(json.as_bytes())))
}

// ---------------------------------------------------------------------------
// BuzzClient
// ---------------------------------------------------------------------------

pub struct BuzzClient {
    http: reqwest::Client,
    relay_url: String, // base URL, no trailing slash, e.g. "https://relay.buzz.place"
    /// WebSocket URLs (ws/wss). Used only in serverless mode. In serverless we
    /// fan out across all relays — publish succeeds if ANY relay accepts, and
    /// queries merge+dedup results — matching the desktop relay pool and the
    /// standard Nostr client model (relays don't gossip; clients talk to many).
    ws_urls: Vec<String>,
    /// Serverless mode: talk to a generic relay over plain WebSocket instead
    /// of the Buzz HTTP bridge.
    serverless: bool,
    keys: Keys,
    /// Optional NIP-OA auth tag injected into every signed event.
    auth_tag: Option<Tag>,
    /// Raw JSON of the auth tag for the `x-auth-tag` HTTP header.
    auth_tag_json: Option<String>,
}

impl BuzzClient {
    pub fn new(
        relay_url: String,
        ws_url: String,
        serverless: bool,
        keys: Keys,
        auth_tag: Option<Tag>,
        auth_tag_json: Option<String>,
    ) -> Result<Self, CliError> {
        // `ws_url` may be a comma-separated list of relays in serverless mode.
        let ws_urls: Vec<String> = ws_url
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let ws_urls = if ws_urls.is_empty() {
            vec![ws_url]
        } else {
            ws_urls
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| CliError::Other(e.to_string()))?;
        Ok(Self {
            http,
            relay_url,
            ws_urls,
            serverless,
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

    /// Whether this client is in serverless mode (generic relay, plain WS).
    pub fn is_serverless(&self) -> bool {
        self.serverless
    }

    /// Return the owner pubkey carried by the NIP-OA auth tag, if any.
    ///
    /// The auth tag is `["auth", owner_pubkey, conditions, sig]`; the
    /// owner pubkey lives at index 1.
    pub fn auth_tag_owner_hex(&self) -> Option<String> {
        self.auth_tag
            .as_ref()
            .map(|t| t.as_slice())
            .and_then(|slice| slice.get(1).cloned())
    }

    /// Sign an event builder, injecting the NIP-OA auth tag if configured.
    ///
    /// All event creation should go through this method to ensure consistent
    /// auth tag injection. Callers MUST NOT add `auth` tags to the builder
    /// before calling this method.
    pub fn sign_event(&self, builder: EventBuilder) -> Result<nostr::Event, CliError> {
        let builder = if let Some(ref tag) = self.auth_tag {
            builder.tags([tag.clone()])
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
        self.query_multi(std::slice::from_ref(filter)).await
    }

    /// Execute a one-shot query with multiple filters via the HTTP bridge.
    /// Each filter is ORed by the relay (standard Nostr REQ behavior).
    pub async fn query_multi(&self, filters: &[serde_json::Value]) -> Result<String, CliError> {
        if self.serverless {
            return self.query_ws(filters).await;
        }
        let url = format!("{}/query", self.relay_url);
        let body_bytes = serde_json::to_vec(filters)
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
        if self.serverless {
            return self.submit_event_ws(&event).await;
        }
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
    // Serverless WS transport (generic relays, no HTTP bridge)
    // -----------------------------------------------------------------------

    /// Query events over a plain WebSocket: REQ → collect EVENTs until EOSE →
    /// CLOSE. Answers a NIP-42 AUTH challenge if the relay sends one. Returns a
    /// JSON-array string of event objects (same shape as the HTTP `/query`
    /// bridge response, so downstream parsing is unchanged).
    async fn query_ws(&self, filters: &[serde_json::Value]) -> Result<String, CliError> {
        // Fan out across all relays, merge results, dedup by event id. Relays
        // don't gossip — a given event may live on only one relay, so a
        // complete read must union every relay (the standard Nostr client
        // model; see damus RelayPool / nostr-tools SimplePool).
        let mut by_id: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut last_err: Option<CliError> = None;
        let mut any_ok = false;
        for relay in &self.ws_urls {
            match self.query_ws_one(relay, filters).await {
                Ok(events) => {
                    any_ok = true;
                    for ev in events {
                        if let Some(id) = ev.get("id").and_then(|v| v.as_str()) {
                            by_id.entry(id.to_string()).or_insert(ev);
                        }
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
        if !any_ok {
            if let Some(e) = last_err {
                return Err(e);
            }
        }
        let merged: Vec<serde_json::Value> = by_id.into_values().collect();
        Ok(serde_json::Value::Array(merged).to_string())
    }

    /// Query a single relay over plain WS: REQ → collect EVENTs until EOSE →
    /// CLOSE. Answers a NIP-42 AUTH challenge if the relay sends one.
    async fn query_ws_one(
        &self,
        relay: &str,
        filters: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, CliError> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (ws, _) = connect_async(relay)
            .await
            .map_err(|e| CliError::NetworkMsg(format!("relay connect failed: {e}")))?;
        let (mut write, mut read) = ws.split();

        let sub_id = format!("cli-q-{}", uuid::Uuid::new_v4());
        let mut req = vec![
            serde_json::Value::String("REQ".into()),
            serde_json::Value::String(sub_id.clone()),
        ];
        req.extend(filters.iter().cloned());
        write
            .send(Message::Text(
                serde_json::Value::Array(req).to_string().into(),
            ))
            .await
            .map_err(|e| CliError::NetworkMsg(format!("send REQ failed: {e}")))?;

        let mut events: Vec<serde_json::Value> = Vec::new();
        let collect = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let msg = match read.next().await {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return Err(format!("WS read error: {e}")),
                    None => return Err("relay closed during query".to_string()),
                };
                let Message::Text(text) = msg else { continue };
                let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                let Some(arr) = arr.as_array() else { continue };
                let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
                    continue;
                };
                let sub_matches = arr.get(1).and_then(|v| v.as_str()) == Some(sub_id.as_str());
                match tag {
                    "EVENT" if sub_matches => {
                        if let Some(ev) = arr.get(2) {
                            events.push(ev.clone());
                        }
                    }
                    "EOSE" if sub_matches => return Ok(()),
                    "CLOSED" if sub_matches => return Ok(()),
                    "AUTH" => {
                        if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                            if let Ok(json) = self.ws_auth_message(relay, challenge) {
                                let _ = write.send(Message::Text(json.into())).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .await;

        let _ = write
            .send(Message::Text(
                serde_json::json!(["CLOSE", sub_id]).to_string().into(),
            ))
            .await;
        let _ = write.close().await;

        match collect {
            Ok(Ok(())) | Err(_) => Ok(events),
            Ok(Err(e)) => {
                if events.is_empty() {
                    Err(CliError::NetworkMsg(e))
                } else {
                    Ok(events)
                }
            }
        }
    }

    /// Publish a signed event over a plain WebSocket and wait for `OK`. Answers
    /// a NIP-42 AUTH challenge if sent. Returns the relay response as a JSON
    /// string (`{event_id, accepted, message}`) matching the HTTP bridge shape.
    async fn submit_event_ws(&self, event: &nostr::Event) -> Result<String, CliError> {
        // Fan out to every relay; succeed if ANY accepts. This is what makes
        // serverless writes resilient to a single relay rate-limiting
        // ("noting too much") or being down — the desktop relay pool does the
        // same. We try relays in order and return on the first acceptance.
        let event_id = event.id.to_hex();
        let mut last_err: Option<CliError> = None;
        for relay in &self.ws_urls {
            match self.submit_event_ws_one(relay, event).await {
                Ok(msg) => return Ok(msg),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err
            .unwrap_or_else(|| CliError::Other(format!("no relay accepted event {event_id}"))))
    }

    /// Publish a signed event to a single relay over plain WS and wait for OK.
    async fn submit_event_ws_one(
        &self,
        relay: &str,
        event: &nostr::Event,
    ) -> Result<String, CliError> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (ws, _) = connect_async(relay)
            .await
            .map_err(|e| CliError::NetworkMsg(format!("relay connect failed: {e}")))?;
        let (mut write, mut read) = ws.split();

        let event_id = event.id.to_hex();
        let event_msg = serde_json::json!(["EVENT", event]).to_string();
        write
            .send(Message::Text(event_msg.clone().into()))
            .await
            .map_err(|e| CliError::NetworkMsg(format!("send EVENT failed: {e}")))?;

        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let msg = match read.next().await {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return Err(format!("WS read error: {e}")),
                    None => return Err("relay closed during publish".to_string()),
                };
                let Message::Text(text) = msg else { continue };
                let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                let Some(arr) = arr.as_array() else { continue };
                let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
                    continue;
                };
                match tag {
                    "OK" if arr.get(1).and_then(|v| v.as_str()) == Some(event_id.as_str()) => {
                        let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                        let message = arr
                            .get(3)
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        return Ok((accepted, message));
                    }
                    "AUTH" => {
                        if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                            if let Ok(json) = self.ws_auth_message(relay, challenge) {
                                let _ = write.send(Message::Text(json.into())).await;
                                let _ = write.send(Message::Text(event_msg.clone().into())).await;
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .await;

        let _ = write.close().await;

        let (accepted, message) = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(CliError::NetworkMsg(e)),
            // Best-effort: many relays accept silently or are slow to OK.
            Err(_) => (true, "published (no OK before timeout)".to_string()),
        };

        if !accepted {
            return Err(CliError::Other(format!("relay rejected event: {message}")));
        }

        Ok(serde_json::json!({
            "event_id": event_id,
            "accepted": accepted,
            "message": message,
        })
        .to_string())
    }

    /// Build a NIP-42 `["AUTH", <event>]` message string for serverless writes.
    fn ws_auth_message(&self, relay: &str, challenge: &str) -> Result<String, CliError> {
        let url = nostr::RelayUrl::parse(relay)
            .map_err(|e| CliError::Other(format!("invalid relay URL: {e}")))?;
        let event = EventBuilder::auth(challenge.to_string(), url)
            .sign_with_keys(&self.keys)
            .map_err(|e| CliError::Other(format!("auth sign failed: {e}")))?;
        Ok(serde_json::json!(["AUTH", event]).to_string())
    }

    // -----------------------------------------------------------------------
    // WebSocket publish (ephemeral events)
    // -----------------------------------------------------------------------

    /// Publish an ephemeral event via WebSocket with NIP-42 authentication.
    ///
    /// The relay rejects ephemeral kinds (20000–29999) over HTTP. Delegates to
    /// `buzz_ws_client::publish_event` which handles connect, NIP-42 auth,
    /// EVENT send, OK wait, and graceful close.
    pub async fn publish_ephemeral_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let ws_url = to_ws_url(&self.relay_url);
        let ok =
            buzz_ws_client::publish_event(&ws_url, event, &self.keys, self.auth_tag.as_ref(), 10)
                .await
                .map_err(|e| CliError::Other(e.to_string()))?;

        if !ok.accepted {
            return Err(CliError::Relay {
                status: 400,
                body: ok.message,
            });
        }
        Ok(serde_json::json!({
            "event_id": ok.event_id,
            "accepted": true,
            "message": ok.message,
        })
        .to_string())
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
        let now = Timestamp::now().as_secs();
        let expiry = if mime.starts_with("video/") {
            3600
        } else {
            600
        };
        let exp_str = (now + expiry).to_string();

        let mut blossom_tags = vec![
            Tag::parse(["t", "upload"]).map_err(|e| CliError::Other(e.to_string()))?,
            Tag::parse(["x", &sha256]).map_err(|e| CliError::Other(e.to_string()))?,
            Tag::parse(["expiration", &exp_str]).map_err(|e| CliError::Other(e.to_string()))?,
        ];
        // Extract server domain from relay URL for BUD-11 server tag
        if let Ok(parsed) = url::Url::parse(&self.relay_url) {
            if let Some(host) = parsed.host_str() {
                let domain = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                blossom_tags.push(
                    Tag::parse(["server", &domain]).map_err(|e| CliError::Other(e.to_string()))?,
                );
            }
        }

        let auth_event = EventBuilder::new(Kind::from(24242), "Upload file")
            .tags(blossom_tags)
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
            if status == 403 && std::env::var("BUZZ_AUTH_TAG").is_ok() {
                let message = format!(
                    "{message} (BUZZ_AUTH_TAG is set — it may be stale or revoked; try unsetting it)"
                );
                return Err(CliError::Relay {
                    status,
                    body: message,
                });
            }
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
/// BUZZ_RELAY_URL may be ws/wss (copied from MCP config).
pub fn normalize_relay_url(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Normalize a relay URL to its WebSocket form: http:// → ws://,
/// https:// → wss://, strip trailing slash. Used in serverless mode where the
/// transport is a plain WebSocket rather than the HTTP bridge, and by
/// `publish_ephemeral_event` for the NIP-01 WebSocket publish path.
pub fn to_ws_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// Output normalization helpers
// ---------------------------------------------------------------------------

/// Normalize raw event JSON array into consistent shape.
/// Each event becomes: {id, pubkey, kind, content, created_at, tags}
pub fn normalize_events(events: &[serde_json::Value]) -> String {
    let normalized: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
                "kind": e.get("kind").and_then(|v| v.as_u64()).unwrap_or(0),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "tags": e.get("tags").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect();
    serde_json::to_string(&normalized).unwrap_or_default()
}

/// Extract the d-tag value from a Nostr event JSON object.
pub fn extract_d_tag(event: &serde_json::Value) -> String {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .and_then(|tags| {
            tags.iter().find(|t| {
                t.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    == Some("d")
            })
        })
        .and_then(|t| t.as_array())
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract a named tag's value from a Nostr event JSON object.
/// Finds the first tag whose first element matches `key` and returns the second element.
pub fn extract_tag_value(event: &serde_json::Value, key: &str) -> String {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .and_then(|tags| {
            tags.iter().find(|t| {
                t.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    == Some(key)
            })
        })
        .and_then(|t| t.as_array())
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract all p-tags into [{pubkey, role}] from a Nostr event JSON object.
pub fn extract_p_tags(event: &serde_json::Value) -> Vec<serde_json::Value> {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|tags| {
            tags.iter()
                .filter(|t| {
                    t.as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        == Some("p")
                })
                .map(|t| {
                    let a = t.as_array().unwrap();
                    serde_json::json!({
                        "pubkey": a.get(1).and_then(|v| v.as_str()).unwrap_or(""),
                        "role": a.get(3).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("member"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Print a create-command response, injecting the generated entity ID.
pub fn print_create_response(resp: &str, id_key: &str, id_val: &str) {
    let mut v: serde_json::Value = serde_json::from_str(resp).unwrap_or(serde_json::json!({}));
    v[id_key] = serde_json::json!(id_val);
    if v.get("accepted").is_none() {
        v["accepted"] = serde_json::json!(true);
    }
    println!("{v}");
}

/// Normalize a relay write-response into a consistent JSON object.
/// Relay returns: {"event_id": "...", "accepted": true, "message": "..."}
/// Falls back to raw text if parsing fails.
pub fn normalize_write_response(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        if v.get("event_id").is_some() || v.get("accepted").is_some() {
            return serde_json::json!({
                "event_id": v.get("event_id").and_then(|v| v.as_str()).unwrap_or(""),
                "accepted": v.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false),
                "message": v.get("message").and_then(|v| v.as_str()).unwrap_or(""),
            })
            .to_string();
        }
    }
    raw.to_string()
}
