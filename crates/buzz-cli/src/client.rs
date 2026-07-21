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
/// Defensive cap against pathological hints; real relay hints observed up to ~24 s.
const RETRY_IN_MAX_SECS: u64 = 30;

/// Returns a full-jitter delay for attempt `i`: a random duration in `[0, RETRY_BASE_SECS[i])`.
fn jitter_delay(attempt: u32) -> Duration {
    Duration::from_secs_f64(RETRY_BASE_SECS[attempt as usize] * rand::random::<f64>())
}

/// Read an env var as a `u64` of seconds and return the corresponding `Duration`.
/// Falls back to `default` if the var is unset, unparseable, or zero (zero is treated
/// as invalid to prevent accidentally disabling all timeouts).
fn env_duration_secs(name: &str, default: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default))
}

/// Scan a plain-text string for a `retry in <N>s` pattern and return `N`.
///
/// Matches the literal substring `retry in ` followed by one or more ASCII digits
/// and the character `s`. Works on both extracted field values (`rate-limited:
/// quota exceeded; retry in 4s`) and substrings of raw relay JSON bodies.
/// Returns `None` when the pattern is absent or the digit sequence is empty.
fn parse_retry_hint_text(text: &str) -> Option<u64> {
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

/// Parse a `retry in Ns` hint from a relay 429 JSON body.
///
/// Extracts the `error` or `message` field and delegates to
/// `parse_retry_hint_text`. Returns `None` when the body is not valid JSON or
/// the extracted field does not contain the pattern.
fn parse_retry_in_secs(body: &str) -> Option<u64> {
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str().map(str::to_string))
        })?;
    parse_retry_hint_text(&text)
}

fn should_retry_legacy_upload(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
    )
}

/// Returns `true` for moderation command kinds (9040–9044).
///
/// These events execute immediately at the relay without dedup, so they must
/// not be blindly retried on ambiguous outcomes.
fn is_moderation_kind(kind: u16) -> bool {
    matches!(kind, 9040..=9044)
}

/// Returns `true` for HTTP status codes that indicate a successful response
/// (equivalent to `reqwest::StatusCode::is_success()` for u16).
fn resp_was_success(status: u16) -> bool {
    (200..300).contains(&status)
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

fn sign_blossom_upload(
    keys: &Keys,
    sha256: &str,
    mime: &str,
    relay_url: &str,
) -> Result<String, CliError> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use nostr::Timestamp;

    let now = Timestamp::now().as_secs();
    let expiry: u64 = if mime.starts_with("video/") {
        3600
    } else {
        600
    };
    let exp_str = (now + expiry).to_string();

    let mut tags = vec![
        Tag::parse(["t", "upload"]).map_err(|e| CliError::Other(e.to_string()))?,
        Tag::parse(["x", sha256]).map_err(|e| CliError::Other(e.to_string()))?,
        Tag::parse(["expiration", &exp_str]).map_err(|e| CliError::Other(e.to_string()))?,
    ];
    if let Some(domain) = relay_server_tag(relay_url) {
        tags.push(Tag::parse(["server", &domain]).map_err(|e| CliError::Other(e.to_string()))?);
    }

    let auth_event = EventBuilder::new(Kind::from(24242), "Upload file")
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
    ///
    /// A value of zero for either variable is treated as invalid and falls back to the default.
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
    ///
    /// Body reads happen OUTSIDE this boundary. Use `with_retry_body` when body
    /// transfer should also be covered (idempotent reads only).
    async fn with_retry_response<'a, F, Fut>(&'a self, op: F) -> Result<reqwest::Response, CliError>
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
                                None => jitter_delay(attempt),
                            }
                        } else {
                            jitter_delay(attempt)
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
                            tokio::time::sleep(jitter_delay(attempt)).await;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
        unreachable!("loop exhausts all RETRY_MAX_ATTEMPTS")
    }

    /// Execute `op` up to `RETRY_MAX_ATTEMPTS` times, including body-transfer failures
    /// and transient relay error statuses.
    ///
    /// Like `with_retry_response`, but the closure is expected to consume the response
    /// body and return the parsed result as `T`. Retries on non-last attempts when `op`
    /// returns:
    ///
    /// - `Err(CliError::Network(e))` where `e.is_connect() || e.is_timeout() ||
    ///   e.is_request() || e.is_body() || e.is_decode()` — covers both connection
    ///   failures and mid-body TCP drops.
    /// - `Err(CliError::Relay { status: 429 | 502 | 503 | 504, .. })` — transient relay
    ///   or proxy errors. For 429 the `retry in Ns` hint from the body is used as the
    ///   delay (capped at `RETRY_IN_MAX_SECS`); all others use exponential jitter.
    ///
    /// Use this variant only for **idempotent** operations (reads, downloads).
    async fn with_retry_body<'a, T, F, Fut>(&'a self, op: F) -> Result<T, CliError>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, CliError>> + 'a,
        T: 'a,
    {
        for attempt in 0..RETRY_MAX_ATTEMPTS {
            let is_last = attempt == RETRY_MAX_ATTEMPTS - 1;
            match op().await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    if !is_last {
                        let delay = match &e {
                            CliError::Network(net_err)
                                if net_err.is_connect()
                                    || net_err.is_timeout()
                                    || net_err.is_request()
                                    || net_err.is_body()
                                    || net_err.is_decode() =>
                            {
                                Some(jitter_delay(attempt))
                            }
                            CliError::Relay { status: 429, body } => {
                                let d = parse_retry_hint_text(body)
                                    .map(|s| Duration::from_secs(s.min(RETRY_IN_MAX_SECS)))
                                    .unwrap_or_else(|| jitter_delay(attempt));
                                Some(d)
                            }
                            CliError::Relay {
                                status: 502..=504, ..
                            } => Some(jitter_delay(attempt)),
                            _ => None,
                        };
                        if let Some(d) = delay {
                            tokio::time::sleep(d).await;
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
        self.with_retry_body(|| {
            let body = body.clone();
            let url = url.clone();
            async move {
                let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
                let resp = self
                    .with_auth_tag(
                        self.http
                            .post(&url)
                            .header("Authorization", auth)
                            .header("Content-Type", "application/json")
                            .body(body),
                    )
                    .send()
                    .await?;
                self.handle_response(resp).await
            }
        })
        .await
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
        self.with_retry_body(|| {
            let body = body.clone();
            let url = url.clone();
            async move {
                let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
                let resp = self
                    .with_auth_tag(
                        self.http
                            .post(&url)
                            .header("Authorization", auth)
                            .header("Content-Type", "application/json")
                            .body(body),
                    )
                    .send()
                    .await?;
                self.handle_response(resp).await
            }
        })
        .await
    }

    /// GET an authed relay endpoint (NIP-98), returning the raw JSON body.
    ///
    /// `path` is a root-relative path incl. any query string, e.g.
    /// `/moderation/reports?status=open&limit=20`. Used by the moderation
    /// read commands, which read structured queue/audit rows rather than
    /// stored events.
    pub async fn get_authed(&self, path: &str) -> Result<String, CliError> {
        let url = format!("{}{path}", self.relay_url);
        self.with_retry_body(|| {
            let url = url.clone();
            async move {
                let auth = sign_nip98(&self.keys, "GET", &url, None)?;
                let resp = self
                    .with_auth_tag(self.http.get(&url).header("Authorization", auth))
                    .send()
                    .await?;
                self.handle_response(resp).await
            }
        })
        .await
    }

    /// Submit a signed Nostr event via POST /events.
    ///
    /// For non-idempotent moderation command kinds (9040–9044), an ambiguous
    /// outcome (mid-request error, body loss, non-ingest 429, or 502/503/504)
    /// surfaces as `CliError::DeliveryUnknown` instead of being retried.  These
    /// events execute at the relay *before* any dedup check, so a blind re-send
    /// can duplicate the mutation.  Only confirmed-unreceived failures (TCP
    /// connect error or a pre-ingest 429 carrying a `rate-limited:` body) are
    /// safe to retry.
    ///
    /// All other event kinds retain the standard retry policy.
    pub async fn submit_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let kind = event.kind.as_u16();
        if is_moderation_kind(kind) {
            self.submit_moderation_event(event).await
        } else {
            self.submit_stored_event(event).await
        }
    }

    /// Submit a moderation command (kinds 9040–9044) with non-idempotent retry policy.
    async fn submit_moderation_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let url = format!("{}/events", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(&event)
                .map_err(|e| CliError::Other(format!("event serialization failed: {e}")))?,
        );

        for attempt in 0..RETRY_MAX_ATTEMPTS {
            let is_last = attempt == RETRY_MAX_ATTEMPTS - 1;

            // Re-sign NIP-98 each attempt: the nonce tag generates a fresh
            // event ID, keeping retries safe against the relay's replay guard.
            let auth = sign_nip98(&self.keys, "POST", &url, Some(&body))?;
            let send_result: Result<reqwest::Response, CliError> = self
                .with_auth_tag(
                    self.http
                        .post(&url)
                        .header("Authorization", auth)
                        .header("Content-Type", "application/json")
                        .body(body.clone()),
                )
                .send()
                .await
                .map_err(CliError::from);

            match send_result {
                Err(e) => {
                    if let CliError::Network(ref net_err) = e {
                        // Only connect-failure is safe to retry: the relay never saw
                        // the request. Timeout and mid-request errors are ambiguous.
                        if !is_last && net_err.is_connect() {
                            tokio::time::sleep(jitter_delay(attempt)).await;
                            continue;
                        }
                        if net_err.is_connect()
                            || net_err.is_timeout()
                            || net_err.is_request()
                            || net_err.is_body()
                            || net_err.is_decode()
                        {
                            // Ambiguous: the relay may have executed this command.
                            return Err(CliError::DeliveryUnknown(format!(
                                "moderation command (kind {}) outcome unknown: {}",
                                event.kind.as_u16(),
                                net_err
                            )));
                        }
                    }
                    return Err(e);
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 429 {
                        // Only retry if the relay's own ingest layer rejected it
                        // (body starts with "rate-limited:"). A proxy-level 429
                        // does not guarantee the relay never saw the request.
                        let body_text = resp.text().await.unwrap_or_default();
                        if !is_last && body_text.trim_start().starts_with("rate-limited:") {
                            match parse_retry_in_secs(&body_text) {
                                Some(hint) => {
                                    tokio::time::sleep(Duration::from_secs(
                                        hint.min(RETRY_IN_MAX_SECS),
                                    ))
                                    .await;
                                }
                                None => {
                                    tokio::time::sleep(jitter_delay(attempt)).await;
                                }
                            }
                            continue;
                        }
                        // Non-ingest 429 or final attempt: outcome unknown.
                        return Err(CliError::DeliveryUnknown(format!(
                            "moderation command (kind {}) outcome unknown: HTTP 429",
                            event.kind.as_u16()
                        )));
                    }
                    if matches!(status, 502..=504) {
                        // Proxy-level error: the relay may have received and executed
                        // the command before the proxy failed.
                        return Err(CliError::DeliveryUnknown(format!(
                            "moderation command (kind {}) outcome unknown: HTTP {status}",
                            event.kind.as_u16()
                        )));
                    }
                    // 2xx or definitive error (4xx other than 429): read body normally.
                    let body_text = resp.text().await.map_err(|e| {
                        // Body loss after relay confirmed receipt is ambiguous for
                        // non-idempotent commands.
                        CliError::DeliveryUnknown(format!(
                            "moderation command (kind {}) outcome unknown: response body lost: {e}",
                            event.kind.as_u16()
                        ))
                    })?;
                    // Map the body through handle_response's error logic inline.
                    if !resp_was_success(status) {
                        let message = serde_json::from_str::<serde_json::Value>(&body_text)
                            .ok()
                            .and_then(|v| {
                                v.get("error")
                                    .or_else(|| v.get("message"))
                                    .and_then(|m| m.as_str())
                                    .map(str::to_string)
                            })
                            .unwrap_or(body_text);
                        let message = if status == 403 && std::env::var("BUZZ_AUTH_TAG").is_ok() {
                            format!(
                                "{message} (BUZZ_AUTH_TAG is set — it may be stale or revoked; try unsetting it)"
                            )
                        } else {
                            message
                        };
                        return Err(CliError::Relay {
                            status,
                            body: message,
                        });
                    }
                    return Ok(body_text);
                }
            }
        }
        unreachable!("loop exhausts all RETRY_MAX_ATTEMPTS")
    }

    /// Submit a stored event (all non-moderation kinds) with the standard retry policy.
    async fn submit_stored_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let url = format!("{}/events", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(&event)
                .map_err(|e| CliError::Other(format!("event serialization failed: {e}")))?,
        );
        let resp = self
            .with_retry_response(|| {
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
        // Hard cap — inner wait ceilings sum to 70 s; connect time and network RTT are
        // additional overhead absorbed by this budget.
        // See buzz_ws_client::{AUTH_CHALLENGE_TIMEOUT_SECS, AUTH_OK_TIMEOUT_SECS,
        // PUBLISH_OK_TIMEOUT_SECS} for the inner ceilings.
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

        // 5. PUT request to the BUD-02 /upload endpoint with a generous timeout.
        // Auth is signed per attempt — matches the per-attempt signing pattern in download_media.
        let upload_timeout = if mime.starts_with("video/") {
            Duration::from_secs(600)
        } else {
            Duration::from_secs(120)
        };
        let url = format!("{}/upload", self.relay_url);
        let upload_body = bytes::Bytes::from(bytes);
        let mut resp = self
            .with_retry_response(|| {
                let upload_body = upload_body.clone();
                let url = url.clone();
                let mime = mime.clone();
                let sha256 = sha256.clone();
                async move {
                    let auth_header =
                        sign_blossom_upload(&self.keys, &sha256, &mime, &self.relay_url)?;
                    Ok(self
                        .with_auth_tag(
                            self.http
                                .put(&url)
                                .timeout(upload_timeout)
                                .header("Authorization", auth_header)
                                .header("Content-Type", &mime)
                                .header("X-SHA-256", &sha256)
                                .body(upload_body),
                        )
                        .send()
                        .await?)
                }
            })
            .await?;
        // The /upload → /media/upload fallback intentionally bypasses with_retry: a 404 or
        // 405 means the primary endpoint doesn't exist on this relay version, not a transient
        // failure.  Retrying would loop without effect.
        if should_retry_legacy_upload(resp.status()) {
            let legacy_url = format!("{}/media/upload", self.relay_url);
            let auth_header = sign_blossom_upload(&self.keys, &sha256, &mime, &self.relay_url)?;
            resp = self
                .with_auth_tag(
                    self.http
                        .put(&legacy_url)
                        .timeout(upload_timeout)
                        .header("Authorization", auth_header)
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
        self.with_retry_body(|| {
            let url = url.clone();
            let client = client.clone();
            async move {
                let auth_header = sign_blossom_get(&self.keys, &url)?;
                let resp = self
                    .with_auth_tag(client.get(&url).header("Authorization", auth_header))
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(CliError::Relay { status, body });
                }
                resp.bytes().await.map_err(CliError::Network)
            }
        })
        .await
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
    use std::time::Duration;

    use super::{
        env_duration_secs, is_moderation_kind, jitter_delay, parse_retry_hint_text,
        parse_retry_in_secs, RETRY_BASE_SECS, RETRY_IN_MAX_SECS, RETRY_MAX_ATTEMPTS,
    };

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

    // ---- parse_retry_hint_text ----

    #[test]
    fn hint_text_plain_extracted_field_returns_secs() {
        // Shape produced by handle_response: JSON extracted, plain text arrives.
        assert_eq!(
            parse_retry_hint_text("rate-limited: quota exceeded; retry in 4s"),
            Some(4)
        );
    }

    #[test]
    fn hint_text_raw_json_body_returns_secs() {
        // Shape from download_media's inline error path: raw JSON body preserved.
        assert_eq!(
            parse_retry_hint_text(r#"{"error":"rate-limited: retry in 7s"}"#),
            Some(7)
        );
    }

    #[test]
    fn hint_text_plain_no_pattern_returns_none() {
        assert_eq!(parse_retry_hint_text("rate-limited: slow down"), None);
    }

    #[test]
    fn hint_text_empty_returns_none() {
        assert_eq!(parse_retry_hint_text(""), None);
    }

    // ---- is_moderation_kind ----

    #[test]
    fn moderation_kind_covers_9040_through_9044() {
        for kind in 9040u16..=9044 {
            assert!(is_moderation_kind(kind), "kind {kind} should be moderation");
        }
    }

    #[test]
    fn non_moderation_kinds_are_not_moderation() {
        for kind in [1u16, 9039, 9045, 39000, 20000, 30023] {
            assert!(
                !is_moderation_kind(kind),
                "kind {kind} should not be moderation"
            );
        }
    }

    // ---- jitter bounds ----

    #[test]
    fn jitter_stays_within_base() {
        for attempt in 0..RETRY_BASE_SECS.len() as u32 {
            let base = RETRY_BASE_SECS[attempt as usize];
            for _ in 0..100 {
                let delay = jitter_delay(attempt).as_secs_f64();
                assert!(
                    (0.0..=base).contains(&delay),
                    "jitter {delay} out of [0, {base}]"
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

    // ---- env_duration_secs ----

    #[test]
    fn env_duration_secs_parsing() {
        // All assertions share one env var key; sequential set/remove prevents races.
        const KEY: &str = "BUZZ_CLI_TEST_DURATION_SECS";

        // Valid numeric value is parsed.
        std::env::set_var(KEY, "42");
        assert_eq!(env_duration_secs(KEY, 30), Duration::from_secs(42));

        // Non-numeric falls back to default.
        std::env::set_var(KEY, "not-a-number");
        assert_eq!(env_duration_secs(KEY, 30), Duration::from_secs(30));

        // Zero is treated as invalid and falls back to default.
        std::env::set_var(KEY, "0");
        assert_eq!(env_duration_secs(KEY, 30), Duration::from_secs(30));

        // Unset uses the default.
        std::env::remove_var(KEY);
        assert_eq!(env_duration_secs(KEY, 30), Duration::from_secs(30));
    }
}

/// Integration tests for the kind-aware retry policy and body-boundary coverage.
///
/// These tests spin up a local HTTP server using axum and issue real HTTP requests
/// through `BuzzClient` to verify behavioural properties — not implementation details.
#[cfg(test)]
mod retry_policy_tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, Response, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use nostr::{EventBuilder, Keys, Kind};
    use tokio::net::TcpListener;

    use super::super::error::CliError;
    use super::BuzzClient;

    /// Spawn a one-shot axum server on a random port.  The handler `f` receives the
    /// attempt counter (incremented before every call) and returns a `(StatusCode,
    /// String)`.  Returns the base URL and a join handle so the caller can assert
    /// attempt counts after the test.
    async fn test_server<F>(f: F) -> (String, Arc<AtomicU32>)
    where
        F: Fn(u32) -> (StatusCode, String) + Send + Sync + 'static,
    {
        let counter = Arc::new(AtomicU32::new(0));
        let handler: Arc<dyn Fn(u32) -> (StatusCode, String) + Send + Sync> = Arc::new(f);
        let state = (handler, counter.clone());

        type S = (
            Arc<dyn Fn(u32) -> (StatusCode, String) + Send + Sync>,
            Arc<AtomicU32>,
        );
        let app = Router::new()
            .route(
                "/events",
                post(
                    |State((handler, ctr)): State<S>, _headers: HeaderMap, _body: Body| async move {
                        let n = ctr.fetch_add(1, Ordering::SeqCst) + 1;
                        let (status, body) = handler(n);
                        Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .unwrap()
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), counter)
    }

    fn test_client(base_url: &str) -> BuzzClient {
        let keys = Keys::generate();
        BuzzClient::new(base_url.to_string(), keys, None, None).unwrap()
    }

    fn make_moderation_event(keys: &Keys, kind: u16) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind), "")
            .sign_with_keys(keys)
            .unwrap()
    }

    fn make_stored_event(keys: &Keys) -> nostr::Event {
        EventBuilder::new(Kind::TextNote, "hi")
            .sign_with_keys(keys)
            .unwrap()
    }

    /// A moderation command (kind 9040) that fails the first attempt with HTTP 429
    /// carrying a plain (non-relay-ingest) body is NOT retried — surfaces as
    /// `DeliveryUnknown`.
    #[tokio::test]
    async fn moderation_kind_non_ingest_429_returns_delivery_unknown() {
        let (url, attempts) = test_server(|_n| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":"slow down"}"#.to_string(),
            )
        })
        .await;
        let client = test_client(&url);
        let event = make_moderation_event(client.keys(), 9040);
        let err = client.submit_event(event).await.unwrap_err();
        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "expected DeliveryUnknown, got {err:?}"
        );
        // Non-ingest 429 must not be retried — exactly 1 attempt.
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "must not retry non-ingest 429"
        );
    }

    /// A moderation command (kind 9041) that gets a relay-ingest 429
    /// (`rate-limited:` prefix) IS retried until it succeeds.
    #[tokio::test]
    async fn moderation_kind_ingest_429_is_retried_until_success() {
        let (url, attempts) = test_server(|n| {
            if n < 2 {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    r#"rate-limited: quota exceeded; retry in 0s"#.to_string(),
                )
            } else {
                (
                    StatusCode::OK,
                    r#"{"event_id":"abc","accepted":true,"message":""}"#.to_string(),
                )
            }
        })
        .await;
        let client = test_client(&url);
        let event = make_moderation_event(client.keys(), 9041);
        let result = client.submit_event(event).await;
        assert!(
            result.is_ok(),
            "expected Ok after ingest-429 retry, got {result:?}"
        );
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "must have retried at least once"
        );
    }

    /// A moderation command (kind 9042) that gets HTTP 502 returns `DeliveryUnknown`
    /// immediately — proxy errors leave relay execution state ambiguous.
    #[tokio::test]
    async fn moderation_kind_502_returns_delivery_unknown() {
        let (url, attempts) =
            test_server(|_n| (StatusCode::BAD_GATEWAY, "bad gateway".to_string())).await;
        let client = test_client(&url);
        let event = make_moderation_event(client.keys(), 9042);
        let err = client.submit_event(event).await.unwrap_err();
        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "expected DeliveryUnknown for 502, got {err:?}"
        );
        // 502 must not be retried — exactly 1 attempt.
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "must not retry 502 for moderation kind"
        );
    }

    /// A stored (non-moderation) event submitted to a server that returns 502 on the
    /// first attempt and then succeeds is retried under the standard policy.
    #[tokio::test]
    async fn stored_event_502_is_retried_under_standard_policy() {
        let (url, attempts) = test_server(|n| {
            if n == 1 {
                (StatusCode::BAD_GATEWAY, "transient".to_string())
            } else {
                (
                    StatusCode::OK,
                    r#"{"event_id":"abc","accepted":true,"message":""}"#.to_string(),
                )
            }
        })
        .await;
        let client = test_client(&url);
        let event = make_stored_event(client.keys());
        let result = client.submit_event(event).await;
        assert!(
            result.is_ok(),
            "expected Ok after 502 retry for stored event, got {result:?}"
        );
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "must have retried at least once"
        );
    }

    /// Spin up a one-shot axum server that handles `GET /info` (and any other GET).
    /// Same contract as `test_server` — returns base URL and attempt counter.
    async fn get_server<F>(f: F) -> (String, Arc<AtomicU32>)
    where
        F: Fn(u32) -> (StatusCode, String) + Send + Sync + 'static,
    {
        let counter = Arc::new(AtomicU32::new(0));
        let handler: Arc<dyn Fn(u32) -> (StatusCode, String) + Send + Sync> = Arc::new(f);
        let state = (handler, counter.clone());

        type S = (
            Arc<dyn Fn(u32) -> (StatusCode, String) + Send + Sync>,
            Arc<AtomicU32>,
        );
        let app = Router::new()
            .route(
                "/{*path}",
                axum::routing::get(
                    |State((handler, ctr)): State<S>, _headers: HeaderMap| async move {
                        let n = ctr.fetch_add(1, Ordering::SeqCst) + 1;
                        let (status, body) = handler(n);
                        Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .unwrap()
                    },
                ),
            )
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), counter)
    }

    /// `with_retry_body` retries transient HTTP 502 on a read path (`get_authed`)
    /// and succeeds on the next attempt.
    #[tokio::test]
    async fn query_502_is_retried_then_succeeds() {
        let (url, attempts) = get_server(|n| {
            if n == 1 {
                (
                    StatusCode::BAD_GATEWAY,
                    "transient gateway error".to_string(),
                )
            } else {
                (StatusCode::OK, r#"{"ok":true}"#.to_string())
            }
        })
        .await;
        let client = test_client(&url);
        let result = client.get_authed("/info").await;
        assert!(
            result.is_ok(),
            "expected Ok after 502 retry, got {result:?}"
        );
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "must have retried at least once"
        );
    }

    /// `with_retry_body` retries a 429 with a `retry in Ns` hint, honours the hint
    /// delay (not the shorter jitter fallback), and ultimately succeeds.
    ///
    /// Uses a 2s hint; jitter max for attempt 0 is 0.5s, so asserting elapsed ≥ 2s
    /// cleanly distinguishes hint-honoured from jitter-fallback.
    #[tokio::test]
    async fn query_429_with_hint_is_retried() {
        let (url, attempts) = get_server(|n| {
            if n < 2 {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    // handle_response extracts the "error" field; the plain text
                    // "rate-limited: retry in 2s" then reaches parse_retry_hint_text.
                    r#"{"error":"rate-limited: retry in 2s"}"#.to_string(),
                )
            } else {
                (StatusCode::OK, r#"{"ok":true}"#.to_string())
            }
        })
        .await;
        let client = test_client(&url);
        let t0 = std::time::Instant::now();
        // Measure from just before attempt 1 fires so we capture the inter-attempt wait.
        let result = client.get_authed("/info").await;
        // Record elapsed after attempt 1 returns (inside the future) is not possible
        // directly, but the total includes the hint sleep; jitter max is 0.5s so ≥ 2s
        // proves the hint was honoured.
        let elapsed = t0.elapsed();
        assert!(
            result.is_ok(),
            "expected Ok after 429 retry, got {result:?}"
        );
        assert!(
            attempts.load(Ordering::SeqCst) >= 2,
            "must have retried at least once"
        );
        assert!(
            elapsed.as_secs_f64() >= 2.0,
            "elapsed {:.2}s < 2s — hint was not honoured (fell back to jitter)",
            elapsed.as_secs_f64()
        );
    }

    /// A definitive 4xx (403 Forbidden) is NOT retried — exactly 1 attempt.
    #[tokio::test]
    async fn query_403_is_not_retried() {
        let (url, attempts) = get_server(|_n| {
            (
                StatusCode::FORBIDDEN,
                r#"{"error":"not allowed"}"#.to_string(),
            )
        })
        .await;
        let client = test_client(&url);
        let result = client.get_authed("/info").await;
        assert!(
            matches!(result, Err(CliError::Relay { status: 403, .. })),
            "expected Relay 403 error, got {result:?}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "403 must not be retried"
        );
    }

    /// `with_retry_body` retries on `is_body()` network errors (F2: body transfer inside
    /// the retry boundary).  Verified by confirming that a call through `get_authed`
    /// (which uses `with_retry_body`) retries when the server drops the connection after
    /// sending headers.  We simulate body loss by returning an intentionally truncated
    /// chunked response that reqwest will surface as an `is_body()` error.
    ///
    /// This test uses a raw TCP server to write partial HTTP responses; axum cannot
    /// easily simulate mid-body connection drops.
    #[tokio::test]
    async fn with_retry_body_retries_on_body_transfer_failure() {
        use tokio::io::AsyncWriteExt;

        let counter = Arc::new(AtomicU32::new(0));
        let counter2 = counter.clone();

        // Bind a raw TCP listener.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let n = counter2.fetch_add(1, Ordering::SeqCst) + 1;

                // Consume the request (required to avoid connection reset by server).
                let mut buf = vec![0u8; 4096];
                use tokio::io::AsyncReadExt;
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    stream.read(&mut buf),
                )
                .await;

                if n < 3 {
                    // Attempts 1 & 2: send valid headers claiming a body, then drop.
                    let partial = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"partial\":";
                    let _ = stream.write_all(partial).await;
                    // Drop the stream without completing the body — causes is_body() on client.
                } else {
                    // Attempt 3: complete response.
                    let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
                    let _ = stream.write_all(ok).await;
                }
            }
        });

        let base = format!("http://{addr}");
        // get_authed internally uses with_retry_body — the body read is inside the retry loop.
        let client = test_client(&base);
        // Stub path: the raw TCP server ignores the URL and always responds based on attempt count.
        let result = client.get_authed("/any-path").await;
        assert!(
            result.is_ok(),
            "expected Ok after body-loss retries, got {result:?}"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "expected 3 attempts (2 body-loss + 1 success)"
        );
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
