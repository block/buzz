use std::future::Future;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use sha2::{Digest, Sha256};

use crate::error::CliError;

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

fn relay_server_tag(relay_url: &str) -> Option<String> {
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    if authority.is_empty() {
        None
    } else {
        Some(authority)
    }
}

/// Maximum number of attempts per request (initial attempt + two retries).
const RETRY_MAX_ATTEMPTS: u32 = 3;

/// Base sleep durations for full-jitter exponential backoff.
/// `RETRY_BASE_SECS[i]` is the ceiling for attempt `i` before attempt `i+1`.
const RETRY_BASE_SECS: [f64; 2] = [0.5, 1.5];

/// Maximum seconds to honour a relay-provided `retry in Ns` hint from a 429.
const RETRY_IN_MAX_SECS: u64 = 8;

/// Read an env var as a `u64` of seconds and return the corresponding `Duration`.
/// Falls back to `default` if the var is unset or unparseable.
fn env_duration_secs(name: &str, default: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default))
}

/// Parse a `retry in Ns` hint from a relay 429 body.
///
/// Searches the `error` or `message` JSON field for the pattern `retry in <N>s`
/// and returns `Some(N)`. Returns `None` when the body is missing, not valid
/// JSON, or does not contain the pattern.
fn parse_retry_in_secs(body: &str) -> Option<u64> {
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str().map(str::to_string))
        })?;
    const PREFIX: &str = "retry in ";
    let after = text.find(PREFIX).map(|i| &text[i + PREFIX.len()..])?;
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    if end == 0 || after.as_bytes().get(end) != Some(&b's') {
        return None;
    }
    after[..end].parse::<u64>().ok()
}

fn should_retry_legacy_upload(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
    )
}

fn is_safe_media_path_segment(sha256_ext: &str) -> bool {
    let segments: Vec<&str> = sha256_ext.split('.').collect();
    match segments.as_slice() {
        [hash] => is_lower_hex_sha256(hash),
        [hash, ext] => is_lower_hex_sha256(hash) && is_safe_media_ext(ext),
        [hash, "thumb", "jpg"] => is_lower_hex_sha256(hash),
        _ => false,
    }
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn is_safe_media_ext(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9'))
}

fn media_url_from_input(relay_url: &str, input: &str) -> Result<String, CliError> {
    let input = input.trim();
    if input.starts_with("http://") || input.starts_with("https://") {
        let parsed = url::Url::parse(input)
            .map_err(|e| CliError::Usage(format!("invalid media URL: {e}")))?;
        if !parsed.path().starts_with("/media/") {
            return Err(CliError::Usage(
                "media URL must point at a /media/ path".to_string(),
            ));
        }
        let Some(sha256_ext) = parsed.path().strip_prefix("/media/") else {
            return Err(CliError::Usage(
                "media URL must point at a /media/ path".to_string(),
            ));
        };
        if !is_safe_media_path_segment(sha256_ext) {
            return Err(CliError::Usage(
                "media path must be sha256, sha256.ext, or sha256.thumb.jpg".to_string(),
            ));
        }
        let relay = url::Url::parse(relay_url)
            .map_err(|e| CliError::Usage(format!("invalid relay URL: {e}")))?;
        if parsed.scheme() != relay.scheme()
            || parsed.host_str() != relay.host_str()
            || parsed.port_or_known_default() != relay.port_or_known_default()
        {
            return Err(CliError::Usage(
                "refusing to sign media GET for a non-relay origin".to_string(),
            ));
        }
        return Ok(input.to_string());
    }
    if input.contains("://") {
        return Err(CliError::Usage(
            "media URL must use http:// or https://".to_string(),
        ));
    }

    let sha256_ext = input.trim_start_matches("/media/");
    if sha256_ext.is_empty() {
        return Err(CliError::Usage(
            "media input must be a URL or sha256[.ext]".to_string(),
        ));
    }
    if !is_safe_media_path_segment(sha256_ext) {
        return Err(CliError::Usage(
            "media input must be sha256, sha256.ext, or sha256.thumb.jpg".to_string(),
        ));
    }
    Ok(format!(
        "{}/media/{sha256_ext}",
        relay_url.trim_end_matches('/')
    ))
}

fn sign_blossom_get(keys: &Keys, media_url: &str) -> Result<String, CliError> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use nostr::Timestamp;

    let now = Timestamp::now().as_secs();
    let exp_str = (now + 600).to_string();
    let domain = relay_server_tag(media_url)
        .ok_or_else(|| CliError::Usage(format!("invalid media URL: {media_url}")))?;
    let tags = vec![
        Tag::parse(["t", "get"]).map_err(|e| CliError::Other(e.to_string()))?,
        Tag::parse(["expiration", &exp_str]).map_err(|e| CliError::Other(e.to_string()))?,
        Tag::parse(["server", &domain]).map_err(|e| CliError::Other(e.to_string()))?,
    ];

    let auth_event = EventBuilder::new(Kind::from(24242), "Get media")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("signing failed: {e}")))?;

    Ok(format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(auth_event.as_json().as_bytes())
    ))
}

#[cfg(test)]
mod media_download_tests {
    use super::*;

    #[test]
    fn media_url_from_sha_uses_relay_media_path() {
        let hash = "a".repeat(64);
        assert_eq!(
            media_url_from_input("https://relay.example", &format!("{hash}.jpg")).unwrap(),
            format!("https://relay.example/media/{hash}.jpg")
        );
        assert_eq!(
            media_url_from_input("https://relay.example/", &format!("/media/{hash}.jpg")).unwrap(),
            format!("https://relay.example/media/{hash}.jpg")
        );
    }

    #[test]
    fn media_url_accepts_only_same_relay_media_urls() {
        let hash = "a".repeat(64);
        assert!(media_url_from_input(
            "https://relay.example:443",
            &format!("https://relay.example/media/{hash}.jpg")
        )
        .is_ok());
        assert!(media_url_from_input(
            "https://relay.example",
            &format!("http://relay.example/media/{hash}.jpg")
        )
        .is_err());
        assert!(media_url_from_input(
            "https://relay.example",
            &format!("https://evil.example/media/{hash}.jpg")
        )
        .is_err());
        assert!(media_url_from_input(
            "https://relay.example",
            &format!("https://relay.example/media-evil/{hash}.jpg")
        )
        .is_err());
        assert!(media_url_from_input(
            "https://relay.example",
            &format!("ftp://relay.example/media/{hash}.jpg")
        )
        .is_err());
    }

    #[test]
    fn media_url_rejects_path_confusion_and_non_hash_inputs() {
        for input in [
            "abc123.jpg",
            "../evil",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/evil.jpg",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.JPG",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.eviltoolong",
            "https://relay.example/media/abc123.jpg",
            "https://relay.example/media/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.JPG",
        ] {
            assert!(
                media_url_from_input("https://relay.example", input).is_err(),
                "input should be rejected: {input}"
            );
        }
    }

    #[test]
    fn media_get_auth_header_is_server_scoped() {
        let keys = Keys::generate();
        let hash = "a".repeat(64);
        let header = sign_blossom_get(
            &keys,
            &format!("https://relay.example:443/media/{hash}.jpg"),
        )
        .unwrap();
        let encoded = header.strip_prefix("Nostr ").unwrap();
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .unwrap();
        let event = nostr::Event::from_json(std::str::from_utf8(&json).unwrap()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.kind, Kind::from(24242));

        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert!(tags.iter().any(|tag| tag.as_slice() == ["t", "get"]));
        assert!(tags
            .iter()
            .any(|tag| tag.as_slice() == ["server", "relay.example"]));
        assert!(!tags
            .iter()
            .any(|tag| tag.first().map(String::as_str) == Some("x")));
    }

    #[test]
    fn legacy_upload_retry_statuses_are_narrow() {
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
}

const QUERY_PAGE_SIZE: u32 = 500;

fn advance_query_cursor(
    filter: &mut serde_json::Value,
    page: &[serde_json::Value],
) -> Result<(), CliError> {
    let last = page
        .last()
        .expect("a full query page always has a last event");
    let created_at = last
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::Other("query event missing created_at".into()))?;
    let id = last
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| CliError::Other("query event missing valid id".into()))?;
    filter["until"] = serde_json::json!(created_at);
    filter["before_id"] = serde_json::json!(id);
    Ok(())
}

pub struct BuzzClient {
    http: reqwest::Client,
    relay_url: String, // base URL, no trailing slash, e.g. "https://relay.buzz.place"
    keys: Keys,
    /// Optional NIP-OA auth tag injected into every signed event.
    auth_tag: Option<Tag>,
    /// Raw JSON of the auth tag for the `x-auth-tag` HTTP header.
    auth_tag_json: Option<String>,
}

impl BuzzClient {
    /// Create a new client pointing at `relay_url`.
    ///
    /// Timeout defaults are tuned for degraded WAN links and can be overridden
    /// via environment variables:
    ///
    /// - `BUZZ_CONNECT_TIMEOUT_SECS` — TCP connect timeout (default 15 s)
    /// - `BUZZ_TIMEOUT_SECS` — per-request total timeout (default 30 s)
    pub fn new(
        relay_url: String,
        keys: Keys,
        auth_tag: Option<Tag>,
        auth_tag_json: Option<String>,
    ) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            .timeout(env_duration_secs("BUZZ_TIMEOUT_SECS", 30))
            .connect_timeout(env_duration_secs("BUZZ_CONNECT_TIMEOUT_SECS", 15))
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

    /// Execute `op` up to `RETRY_MAX_ATTEMPTS` times, backing off on transient failures.
    ///
    /// Retries when `op` returns:
    /// - `Err(CliError::Network(e))` where `e.is_connect() || e.is_timeout() || e.is_request()`
    /// - `Ok(resp)` with status 429, 502, 503, or 504
    ///
    /// On 429 the relay-provided `retry in Ns` hint (from the `error` or `message`
    /// JSON field) is used as the delay, capped at `RETRY_IN_MAX_SECS`. Unparseable
    /// hints and all other retryable cases use full-jitter exponential backoff.
    ///
    /// The final attempt always returns whatever `op` produces (success or error)
    /// so that `handle_response` can map it to the appropriate exit code.
    async fn with_retry<'a, F, Fut>(&'a self, op: F) -> Result<reqwest::Response, CliError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<reqwest::Response, CliError>> + 'a,
    {
        for attempt in 0..RETRY_MAX_ATTEMPTS {
            let is_last = attempt == RETRY_MAX_ATTEMPTS - 1;
            match op().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if !is_last && matches!(status, 429 | 502 | 503 | 504) {
                        let delay = if status == 429 {
                            let body = resp.text().await.unwrap_or_default();
                            match parse_retry_in_secs(&body) {
                                Some(hint) => Duration::from_secs(hint.min(RETRY_IN_MAX_SECS)),
                                None => {
                                    let base = RETRY_BASE_SECS[attempt as usize];
                                    Duration::from_secs_f64(base * rand::random::<f64>())
                                }
                            }
                        } else {
                            let base = RETRY_BASE_SECS[attempt as usize];
                            Duration::from_secs_f64(base * rand::random::<f64>())
                        };
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if let CliError::Network(ref net_err) = e {
                        if !is_last
                            && (net_err.is_connect()
                                || net_err.is_timeout()
                                || net_err.is_request())
                        {
                            let base = RETRY_BASE_SECS[attempt as usize];
                            let delay = Duration::from_secs_f64(base * rand::random::<f64>());
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
        unreachable!("loop exhausts all RETRY_MAX_ATTEMPTS")
    }

    async fn query_pages(
        &self,
        mut filter: serde_json::Value,
        limit: Option<u32>,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        let mut events = Vec::new();

        while limit.is_none_or(|limit| events.len() < limit as usize) {
            let page_limit = limit
                .map(|limit| (limit as usize - events.len()).min(QUERY_PAGE_SIZE as usize))
                .unwrap_or(QUERY_PAGE_SIZE as usize);
            filter["limit"] = serde_json::json!(page_limit);

            let raw = self.query(&filter).await?;
            let page: Vec<serde_json::Value> = serde_json::from_str(&raw)
                .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
            let done = page.len() < page_limit;

            if !done {
                advance_query_cursor(&mut filter, &page)?;
            }
            events.extend(page);
            if done {
                break;
            }
        }

        Ok(events)
    }

    /// Query up to `limit` historical events, following the relay bridge's
    /// composite `(until, before_id)` cursor across bounded result pages.
    pub async fn query_paginated(
        &self,
        filter: serde_json::Value,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        self.query_pages(filter, Some(limit)).await
    }

    /// Query every historical event matching a filter across bounded pages.
    pub async fn query_all(
        &self,
        filter: serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        self.query_pages(filter, None).await
    }

    /// Sign an event builder verbatim: no NIP-OA auth-tag injection, and none
    /// of [`sign_event`]'s "callers must not add auth tags" enforcement.
    ///
    /// Used only for NIP-IA identity archive/unarchive requests (kind
    /// 9035/9036), whose optional `auth` tag is a *content-level*
    /// owner-of-agent attestation about the *target* identity — unrelated to
    /// this client's own NIP-OA membership delegation (`self.auth_tag`,
    /// which [`sign_event`] injects into every other event and which
    /// `submit_event` separately attaches via the `x-auth-tag` HTTP header).
    /// Routing an identity archive request through `sign_event` would either
    /// silently drop the caller's owner attestation or double up an
    /// unrelated tag.
    pub fn sign_event_unchecked(&self, builder: EventBuilder) -> Result<nostr::Event, CliError> {
        builder
            .sign_with_keys(&self.keys)
            .map_err(|e| CliError::Other(format!("signing failed: {e}")))
    }

    /// GET a public, unauthenticated relay endpoint (e.g. the NIP-11 `/info`
    /// document), returning the raw JSON body. No NIP-98 Authorization and no
    /// `x-auth-tag` header — the endpoint is public relay metadata, not a
    /// membership-scoped resource.
    pub async fn get_public(&self, path: &str) -> Result<String, CliError> {
        let url = format!("{}{path}", self.relay_url);
        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/nostr+json")
            .send()
            .await?;
        self.handle_response(resp).await
    }

    /// Execute a one-shot query via the HTTP bridge.
    /// `filter` is a Nostr filter object (will be wrapped in an array).
    /// Returns the raw JSON response (array of events).
    pub async fn query(&self, filter: &serde_json::Value) -> Result<String, CliError> {
        self.query_multi(std::slice::from_ref(filter)).await
    }

    /// Execute a one-shot query with multiple filters via the HTTP bridge.
    /// Each filter is ORed by the relay (standard Nostr REQ behavior).
    pub async fn query_multi(&self, filters: &[serde_json::Value]) -> Result<String, CliError> {
        let url = format!("{}/query", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(filters)
                .map_err(|e| CliError::Other(format!("filter serialization failed: {e}")))?,
        );
        let resp = self
            .with_retry(|| {
                let body = body.clone();
                let url = url.clone();
                async move {
                    let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
                    Ok(self
                        .with_auth_tag(
                            self.http
                                .post(&url)
                                .header("Authorization", auth)
                                .header("Content-Type", "application/json")
                                .body(body),
                        )
                        .send()
                        .await?)
                }
            })
            .await?;
        self.handle_response(resp).await
    }

    /// Execute a one-shot count via the HTTP bridge.
    /// Returns the count as a JSON string.
    #[allow(dead_code)]
    pub async fn count(&self, filter: &serde_json::Value) -> Result<String, CliError> {
        let url = format!("{}/count", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(&[filter])
                .map_err(|e| CliError::Other(format!("filter serialization failed: {e}")))?,
        );
        let resp = self
            .with_retry(|| {
                let body = body.clone();
                let url = url.clone();
                async move {
                    let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
                    Ok(self
                        .with_auth_tag(
                            self.http
                                .post(&url)
                                .header("Authorization", auth)
                                .header("Content-Type", "application/json")
                                .body(body),
                        )
                        .send()
                        .await?)
                }
            })
            .await?;
        self.handle_response(resp).await
    }

    /// GET an authed relay endpoint (NIP-98), returning the raw JSON body.
    ///
    /// `path` is a root-relative path incl. any query string, e.g.
    /// `/moderation/reports?status=open&limit=20`. Used by the moderation
    /// read commands, which read structured queue/audit rows rather than
    /// stored events.
    pub async fn get_authed(&self, path: &str) -> Result<String, CliError> {
        let url = format!("{}{path}", self.relay_url);
        let resp = self
            .with_retry(|| {
                let url = url.clone();
                async move {
                    let auth = sign_nip98(&self.keys, "GET", &url, None)?;
                    Ok(self
                        .with_auth_tag(self.http.get(&url).header("Authorization", auth))
                        .send()
                        .await?)
                }
            })
            .await?;
        self.handle_response(resp).await
    }

    /// Submit a signed Nostr event via POST /events.
    pub async fn submit_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let url = format!("{}/events", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(&event)
                .map_err(|e| CliError::Other(format!("event serialization failed: {e}")))?,
        );
        let resp = self
            .with_retry(|| {
                let body = body.clone();
                let url = url.clone();
                async move {
                    // Re-sign NIP-98 each attempt: the nonce tag generates a fresh
                    // event ID, keeping retries safe against the relay's replay guard.
                    let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
                    Ok(self
                        .with_auth_tag(
                            self.http
                                .post(&url)
                                .header("Authorization", auth)
                                .header("Content-Type", "application/json")
                                .body(body),
                        )
                        .send()
                        .await?)
                }
            })
            .await?;
        self.handle_response(resp).await
    }

    /// Publish an ephemeral event via WebSocket with NIP-42 authentication.
    ///
    /// The relay rejects ephemeral kinds (20000–29999) over HTTP. Delegates to
    /// `buzz_ws_client::publish_event` which handles connect, NIP-42 auth,
    /// EVENT send, OK wait, and graceful close.
    pub async fn publish_ephemeral_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let ws_url = to_ws_url(&self.relay_url);
        // Outer budget: inner wait constants (20 + 20 + 30 = 70 s) plus 5 s slack.
        // See buzz_ws_client::{AUTH_CHALLENGE_TIMEOUT_SECS, AUTH_OK_TIMEOUT_SECS,
        // PUBLISH_OK_TIMEOUT_SECS} for the inner floors.
        let ok =
            buzz_ws_client::publish_event(&ws_url, event, &self.keys, self.auth_tag.as_ref(), 75)
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
        if let Some(domain) = relay_server_tag(&self.relay_url) {
            blossom_tags
                .push(Tag::parse(["server", &domain]).map_err(|e| CliError::Other(e.to_string()))?);
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

        // 7. PUT request to the BUD-02 /upload endpoint with a generous timeout.
        let upload_timeout = if mime.starts_with("video/") {
            Duration::from_secs(600)
        } else {
            Duration::from_secs(120)
        };
        let url = format!("{}/upload", self.relay_url);
        let upload_body = bytes::Bytes::from(bytes);
        let mut resp = self
            .with_retry(|| {
                let upload_body = upload_body.clone();
                let url = url.clone();
                let auth_header = auth_header.clone();
                let mime = mime.clone();
                let sha256 = sha256.clone();
                async move {
                    Ok(self
                        .with_auth_tag(
                            self.http
                                .put(&url)
                                .timeout(upload_timeout)
                                .header("Authorization", &auth_header)
                                .header("Content-Type", &mime)
                                .header("X-SHA-256", &sha256)
                                .body(upload_body),
                        )
                        .send()
                        .await?)
                }
            })
            .await?;
        if should_retry_legacy_upload(resp.status()) {
            let legacy_url = format!("{}/media/upload", self.relay_url);
            resp = self
                .with_auth_tag(
                    self.http
                        .put(&legacy_url)
                        .timeout(upload_timeout)
                        .header("Authorization", &auth_header)
                        .header("Content-Type", &mime)
                        .header("X-SHA-256", &sha256),
                )
                .body(upload_body)
                .send()
                .await?;
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Relay { status, body });
        }

        resp.json::<BlobDescriptor>()
            .await
            .map_err(|e| CliError::Other(format!("invalid upload response: {e}")))
    }

    /// Download a Blossom media blob using BUD-01 `t=get` auth.
    pub async fn download_media(&self, input: &str) -> Result<bytes::Bytes, CliError> {
        let url = media_url_from_input(&self.relay_url, input)?;
        // Use a dedicated client: 120 s timeout, no redirect forwarding.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            // Do not forward Authorization or x-auth-tag to redirect targets.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| CliError::Other(format!("http client init failed: {e}")))?;
        let resp = self
            .with_retry(|| {
                let url = url.clone();
                let client = client.clone();
                async move {
                    let auth_header = sign_blossom_get(&self.keys, &url)?;
                    Ok(self
                        .with_auth_tag(client.get(&url).header("Authorization", auth_header))
                        .send()
                        .await?)
                }
            })
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Relay { status, body });
        }

        resp.bytes().await.map_err(CliError::Network)
    }

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

/// Normalize a relay URL: ws:// → http://, wss:// → https://, strip trailing slash.
/// BUZZ_RELAY_URL may be ws/wss (copied from MCP config).
pub fn normalize_relay_url(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Convert an HTTP(S) relay base URL back to a WebSocket URL for NIP-01 connections.
fn to_ws_url(http_url: &str) -> String {
    http_url
        .replace("https://", "wss://")
        .replace("http://", "ws://")
}

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

/// Return a create-command response with an entity ID injected.
pub fn create_response_with_id(resp: &str, id_key: &str, id_val: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(resp).unwrap_or(serde_json::json!({}));
    v[id_key] = serde_json::json!(id_val);
    if v.get("accepted").is_none() {
        v["accepted"] = serde_json::json!(true);
    }
    v.to_string()
}

/// Print a create-command response, injecting the generated entity ID.
pub fn print_create_response(resp: &str, id_key: &str, id_val: &str) {
    println!("{}", create_response_with_id(resp, id_key, id_val));
}

/// Extract a JSON field from relay write response messages shaped as
/// `response:{...}`.
pub fn extract_relay_response_field(resp: &str, field: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(resp)
        .ok()?
        .get("message")?
        .as_str()?
        .strip_prefix("response:")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|v| v.get(field)?.as_str().map(str::to_string))
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

#[cfg(test)]
mod retry_tests {
    use super::{parse_retry_in_secs, RETRY_BASE_SECS, RETRY_IN_MAX_SECS, RETRY_MAX_ATTEMPTS};

    // ---- parse_retry_in_secs ----

    #[test]
    fn parse_relay_json_with_error_field() {
        let body = r#"{"error":"rate-limited: quota exceeded; retry in 5s"}"#;
        assert_eq!(parse_retry_in_secs(body), Some(5));
    }

    #[test]
    fn parse_relay_json_with_message_field() {
        let body = r#"{"message":"back off; retry in 3s please"}"#;
        assert_eq!(parse_retry_in_secs(body), Some(3));
    }

    #[test]
    fn parse_retry_in_zero_seconds() {
        let body = r#"{"error":"retry in 0s"}"#;
        assert_eq!(parse_retry_in_secs(body), Some(0));
    }

    #[test]
    fn parse_garbled_body_returns_none() {
        assert_eq!(parse_retry_in_secs("not json at all"), None);
    }

    #[test]
    fn parse_missing_retry_pattern_returns_none() {
        let body = r#"{"error":"rate-limited, please slow down"}"#;
        assert_eq!(parse_retry_in_secs(body), None);
    }

    #[test]
    fn parse_empty_body_returns_none() {
        assert_eq!(parse_retry_in_secs(""), None);
    }

    // ---- jitter bounds ----

    #[test]
    fn jitter_stays_within_base() {
        for &base in &RETRY_BASE_SECS {
            for _ in 0..100 {
                let jitter = base * rand::random::<f64>();
                assert!(
                    (0.0..=base).contains(&jitter),
                    "jitter {jitter} out of [0, {base}]"
                );
            }
        }
    }

    // ---- constant sanity ----

    #[test]
    fn retry_constants_are_sensible() {
        assert_eq!(RETRY_MAX_ATTEMPTS, 3);
        assert_eq!(RETRY_BASE_SECS.len(), (RETRY_MAX_ATTEMPTS - 1) as usize);
        const { assert!(RETRY_IN_MAX_SECS > 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        advance_query_cursor, create_response_with_id, extract_relay_response_field, BuzzClient,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn query_cursor_uses_last_events_composite_sort_key() {
        let mut filter = serde_json::json!({"kinds": [39000], "limit": 500});
        let page = vec![
            serde_json::json!({"id": "a".repeat(64), "created_at": 20}),
            serde_json::json!({"id": "b".repeat(64), "created_at": 10}),
        ];

        advance_query_cursor(&mut filter, &page).unwrap();

        assert_eq!(filter["until"], serde_json::json!(10));
        assert_eq!(filter["before_id"], serde_json::json!("b".repeat(64)));
    }

    #[test]
    fn query_cursor_rejects_missing_or_malformed_sort_key() {
        let mut filter = serde_json::json!({});
        assert!(
            advance_query_cursor(&mut filter, &[serde_json::json!({"id": "a".repeat(64)})])
                .is_err()
        );
        assert!(advance_query_cursor(
            &mut filter,
            &[serde_json::json!({"id": "not-an-event-id", "created_at": 10})]
        )
        .is_err());
    }

    #[test]
    fn extract_relay_response_field_reads_response_message_json() {
        let raw = r#"{"event_id":"abc","accepted":true,"message":"response:{\"workflow_id\":\"relay-id\",\"created\":true}"}"#;
        assert_eq!(
            extract_relay_response_field(raw, "workflow_id").as_deref(),
            Some("relay-id")
        );
    }

    #[test]
    fn extract_relay_response_field_returns_none_for_non_response_message() {
        let raw = r#"{"event_id":"abc","accepted":true,"message":""}"#;
        assert!(extract_relay_response_field(raw, "workflow_id").is_none());
    }

    #[test]
    fn create_response_with_id_overrides_local_id_with_relay_id() {
        let raw = r#"{"event_id":"abc","accepted":true,"message":"response:{\"workflow_id\":\"relay-id\"}"}"#;
        let out = create_response_with_id(raw, "workflow_id", "relay-id");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["workflow_id"].as_str(), Some("relay-id"));
        assert_eq!(v["event_id"].as_str(), Some("abc"));
        assert_eq!(v["accepted"].as_bool(), Some(true));
    }

    // --- (a) auth-suppression regression pair ---

    fn make_auth_tag() -> (Tag, String) {
        let owner_hex = "a".repeat(64);
        let sig_hex = "b".repeat(128);
        let tag_vec = vec![
            "auth".to_string(),
            owner_hex,
            "conditions".to_string(),
            sig_hex,
        ];
        let json = serde_json::to_string(&tag_vec).unwrap();
        let tag = Tag::parse(tag_vec).unwrap();
        (tag, json)
    }

    #[test]
    fn sign_event_unchecked_does_not_inject_ambient_auth_tag() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            keys,
            Some(auth_tag),
            Some(auth_json),
        )
        .unwrap();

        let builder =
            EventBuilder::new(Kind::Custom(9035), "archive").tags([Tag::parse(["-"]).unwrap()]);
        let event = client.sign_event_unchecked(builder).unwrap();

        let auth_tags: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .collect();
        assert!(
            auth_tags.is_empty(),
            "sign_event_unchecked must not inject the ambient NIP-OA auth tag \
             into identity archive events; found {auth_tags:?}"
        );
    }

    #[test]
    fn sign_event_unchecked_preserves_callers_content_auth_tag() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            keys,
            Some(auth_tag),
            Some(auth_json),
        )
        .unwrap();

        let content_auth = Tag::parse([
            "auth",
            &"c".repeat(64),
            "owner-attestation",
            &"d".repeat(128),
        ])
        .unwrap();

        let builder = EventBuilder::new(Kind::Custom(9035), "archive")
            .tags([Tag::parse(["-"]).unwrap(), content_auth]);
        let event = client.sign_event_unchecked(builder).unwrap();

        let auth_tags: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .collect();
        assert_eq!(
            auth_tags.len(),
            1,
            "content-level auth tag must survive sign_event_unchecked; found {auth_tags:?}"
        );
        assert_eq!(auth_tags[0].as_slice()[1], "c".repeat(64));
    }

    #[test]
    fn with_auth_tag_sets_header_when_configured() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            keys,
            Some(auth_tag),
            Some(auth_json.clone()),
        )
        .unwrap();

        let req = client.http.post("https://test.relay/events");
        let req = client.with_auth_tag(req);
        let built = req.build().unwrap();
        let header = built
            .headers()
            .get("x-auth-tag")
            .expect("x-auth-tag header must be present");
        assert_eq!(
            header.to_str().unwrap(),
            &auth_json,
            "x-auth-tag header must carry the raw auth tag JSON"
        );
    }

    #[test]
    fn with_auth_tag_omits_header_when_not_configured() {
        let keys = Keys::generate();
        let client = BuzzClient::new("https://test.relay".into(), keys, None, None).unwrap();

        let req = client.http.post("https://test.relay/events");
        let req = client.with_auth_tag(req);
        let built = req.build().unwrap();
        assert!(
            built.headers().get("x-auth-tag").is_none(),
            "x-auth-tag header must not be present when no auth tag is configured"
        );
    }
}
