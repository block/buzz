use std::future::Future;
use std::time::Duration;

use base64::Engine;
use nostr::{EventBuilder, JsonUtil, Kind, PublicKey, Tag};
use sha2::{Digest, Sha256};

use crate::credentials::{
    Credentials, FEDERATED_IDENTITY_ADAPTER_ENV, FEDERATED_IDENTITY_TOKEN_ENV,
};
use crate::error::CliError;

/// NIP-FI assertion header (`docs/nips/NIP-FI.md` §Client-attached transport).
const FEDERATED_IDENTITY_HEADER: &str = "Nostr-Federated-Identity";
/// `User-Agent` sent on every WebSocket upgrade.
const USER_AGENT: &str = concat!("buzz-cli/", env!("CARGO_PKG_VERSION"));

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

/// MIME types the relay stores as inline media; everything else is a generic
/// file it validates, stores and serves as a download.
const MEDIA_MIMES: &[&str] = &[
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

/// Maximum file size for generic file uploads (100 MB), the relay's default.
const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

fn relay_server_tag(relay_url: &str) -> Option<String> {
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    if authority.is_empty() {
        None
    } else {
        Some(authority)
    }
}

/// Maximum number of attempts per request (initial attempt + two retries).
/// The WebSocket connect and upgrade, before any NIP-42 wait.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Connections a WebSocket publish may open to complete NIP-42 sign-in.
const WS_SIGN_IN_ATTEMPTS: u32 = 3;
/// Times an HTTP proof is signed while its NIP-FI assertion keeps expiring.
const AUTHORIZE_ROUNDS: u32 = 3;
/// How old an HTTP proof may be when sent, well inside the relay's 60 s.
const PROOF_MAX_AGE_SECS: u64 = 30;
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
#[cfg(test)]
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

/// Extract the `error` or `message` field from a relay JSON error body.
///
/// Production relay error bodies are shaped as `{"error":"..."}` (via `api_error()`).
/// Returns the extracted field value, or `None` if the body is not valid JSON or
/// neither field is present.  The raw body should be retained for diagnostics when
/// `None` is returned.
fn extract_relay_message_field(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
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

/// Returns `true` if a stored-event exhaustion error is ambiguous (the relay
/// may have executed the command) and should be converted to `DeliveryUnknown`.
///
/// Connect failures are definitively pre-relay (never executed) so they remain
/// retryable. Canonical pre-ingest 429 (`Relay{status:429}`) was provably
/// rejected before storage — also retryable. Everything else (timeout, body
/// loss, decode error, proxy 502-504) may have crossed the relay's storage
/// boundary and must not invite an outer re-sign.
fn is_stored_event_exhaustion_ambiguous(e: &CliError) -> bool {
    match e {
        CliError::Network(net_err) => {
            // Connect is definitively pre-relay.
            if net_err.is_connect() {
                return false;
            }
            // Timeout, body, decode, request — ambiguous.
            net_err.is_timeout() || net_err.is_body() || net_err.is_decode() || net_err.is_request()
        }
        // Canonical pre-ingest 429 — relay did not store.
        CliError::Relay { status: 429, .. } => false,
        // Proxy 502-504 — relay may have accepted before the proxy failed.
        CliError::Relay {
            status: 502..=504, ..
        } => true,
        // All other variants are not retried by with_retry_body; not ambiguous.
        _ => false,
    }
}

fn is_safe_media_path_segment(sha256_ext: &str) -> bool {
    let segments: Vec<&str> = sha256_ext.split('.').collect();
    match segments.as_slice() {
        [hash] => is_lower_hex_sha256(hash),
        [hash, ext] => is_lower_hex_sha256(hash) && is_safe_media_ext(ext),
        [hash, "thumb", "webp" | "jpg"] => is_lower_hex_sha256(hash),
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
                "media path must be sha256, sha256.ext, or sha256.thumb.{webp,jpg}".to_string(),
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
            "media input must be sha256, sha256.ext, or sha256.thumb.{webp,jpg}".to_string(),
        ));
    }
    Ok(format!(
        "{}/media/{sha256_ext}",
        relay_url.trim_end_matches('/')
    ))
}

/// Blossom (BUD-01) `t=get` proof for `media_url`, scoped to its server.
fn blossom_get_event(media_url: &str) -> Result<EventBuilder, CliError> {
    blossom_event(
        "get",
        None,
        relay_server_tag(media_url)
            .ok_or_else(|| CliError::Usage(format!("invalid media URL: {media_url}")))?,
        "Get media",
    )
}

/// Blossom (BUD-02) `t=upload` proof for blob `sha256` on this relay.
fn blossom_upload_event(sha256: &str, relay_url: &str) -> Result<EventBuilder, CliError> {
    blossom_event(
        "upload",
        Some(sha256),
        relay_server_tag(relay_url)
            .ok_or_else(|| CliError::Usage(format!("invalid relay URL: {relay_url}")))?,
        "Upload file",
    )
}

fn blossom_event(
    verb: &str,
    sha256: Option<&str>,
    server: String,
    content: &str,
) -> Result<EventBuilder, CliError> {
    let exp = (nostr::Timestamp::now().as_secs() + 60).to_string();
    let tag = |parts: &[&str]| {
        Tag::parse(parts.iter().copied()).map_err(|e| CliError::Other(e.to_string()))
    };
    let mut tags = vec![tag(&["t", verb])?];
    if let Some(sha256) = sha256 {
        tags.push(tag(&["x", sha256])?);
    }
    tags.push(tag(&["expiration", &exp])?);
    tags.push(tag(&["server", &server])?);
    Ok(EventBuilder::new(Kind::from(24242), content).tags(tags))
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
        let keys = nostr::Keys::generate();
        let hash = "a".repeat(64);
        let event = blossom_get_event(&format!("https://relay.example:443/media/{hash}.jpg"))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
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

pub(crate) const QUERY_PAGE_SIZE: u32 = 500;

/// The most pages one limited query reads before failing, so a relay that
/// never runs out of events to skip cannot keep a command running forever.
pub(crate) const MAX_QUERY_PAGES: usize = 100;

fn stalled_query(detail: &str) -> CliError {
    CliError::Relay {
        status: 200,
        body: format!("query pagination stopped: {detail}"),
    }
}

/// Move the composite cursor to the page's last event, failing unless it
/// lands strictly older than the current cursor.
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
    if let (Some(until), Some(before_id)) = (
        filter.get("until").and_then(serde_json::Value::as_u64),
        filter.get("before_id").and_then(serde_json::Value::as_str),
    ) {
        // The relay pages newest first, equal timestamps by ascending id:
        // the next page lies at `created_at < until`, or at `until` past `before_id`.
        let older = created_at < until
            || (created_at == until && id.to_ascii_lowercase() > before_id.to_ascii_lowercase());
        if !older {
            return Err(stalled_query(
                "the relay did not move the cursor to older events",
            ));
        }
    }
    filter["until"] = serde_json::json!(created_at);
    filter["before_id"] = serde_json::json!(id);
    Ok(())
}

/// How a protected relay request proves key possession in `Authorization`.
enum Proof<'a> {
    /// NIP-98 over the request method, URL and body.
    Nip98 {
        method: &'a str,
        url: &'a str,
        body: Option<&'a [u8]>,
    },
    /// Blossom kind-24242 upload proof for blob `sha256`.
    BlossomUpload { sha256: &'a str },
    /// Blossom kind-24242 read proof for `url`.
    BlossomGet { url: &'a str },
}

impl<'a> Proof<'a> {
    fn post(url: &'a str, body: &'a [u8]) -> Self {
        Self::Nip98 {
            method: "POST",
            url,
            body: Some(body),
        }
    }
}

pub struct BuzzClient {
    http: reqwest::Client,
    relay_url: String, // base URL, no trailing slash, e.g. "https://relay.buzz.place"
    /// The configured URL as a WebSocket URL, path and trailing slash kept,
    /// as NIP-42 AUTH must name it.
    ws_url: String,
    /// Identity, signer, NIP-OA auth tag and NIP-FI evidence source.
    credentials: Credentials,
    /// Whether any request has carried a NIP-FI assertion, for error hints.
    assertion_sent: std::sync::atomic::AtomicBool,
}

/// A credential header value, marked sensitive so `Debug` output redacts it.
fn sensitive(value: String) -> Result<reqwest::header::HeaderValue, CliError> {
    let mut value = reqwest::header::HeaderValue::try_from(value)
        .map_err(|_| CliError::Auth("credential is not a valid header value".into()))?;
    value.set_sensitive(true);
    Ok(value)
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
    /// A client for the relay at `relay_url`, as configured.
    pub fn new(relay_url: String, credentials: Credentials) -> Result<Self, CliError> {
        let http = reqwest::Client::builder()
            // Never replay credentials to a redirect target: reqwest strips
            // `Authorization` across origins but not `x-auth-tag` or the
            // NIP-FI assertion. A 3xx surfaces as a relay error instead.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(env_duration_secs("BUZZ_TIMEOUT_SECS", 30))
            .connect_timeout(env_duration_secs("BUZZ_CONNECT_TIMEOUT_SECS", 15))
            .build()
            .map_err(|e| CliError::Other(e.to_string()))?;
        Ok(Self {
            http,
            ws_url: to_ws_url(&relay_url),
            relay_url: normalize_relay_url(&relay_url),
            credentials,
            assertion_sent: Default::default(),
        })
    }

    /// The public key this client acts as.
    pub fn public_key(&self) -> PublicKey {
        self.credentials.public_key()
    }

    /// The credentials this client acts with.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
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
        self.credentials
            .auth_tag()
            .map(|t| t.tag.as_slice())
            .and_then(|slice| slice.get(1).cloned())
    }

    /// Sign an event builder, injecting the NIP-OA auth tag if configured.
    ///
    /// All event creation should go through this method to ensure consistent
    /// auth tag injection. Callers MUST NOT add `auth` tags to the builder
    /// before calling this method.
    pub async fn sign_event(&self, builder: EventBuilder) -> Result<nostr::Event, CliError> {
        let auth_tag = self.credentials.auth_tag();
        let builder = match auth_tag {
            Some(tag) => builder.tags([tag.tag.clone()]),
            None => builder,
        };
        let event = self.credentials.sign(builder).await?;

        // Enforce: auth tags may only come from the credentials' auth tag.
        let auth_count = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .count();
        let expected = if auth_tag.is_some() { 1 } else { 0 };
        if auth_count != expected {
            return Err(CliError::Other(format!(
                "event has {auth_count} auth tags — expected {expected}; \
                 callers must not add auth tags manually"
            )));
        }

        Ok(event)
    }

    /// The one place protected relay requests get their credentials:
    /// `proof` in `Authorization`, the NIP-OA `x-auth-tag` (relay membership
    /// delegation) when configured, and the NIP-FI `Nostr-Federated-Identity`
    /// assertion when an evidence source is configured. Called once per
    /// attempt so every retry carries a fresh proof.
    async fn authorize(
        &self,
        req: reqwest::RequestBuilder,
        proof: Proof<'_>,
    ) -> Result<reqwest::RequestBuilder, CliError> {
        // Evidence first: a refresh may wait out adapter retries, and the
        // proof must be signed after it, just before sending, to stay fresh.
        // Signing may wait out throttling in turn, so evidence that is due
        // for refresh by then is fetched again, and the proof re-signed when
        // the assertion changed or that fetch aged the proof.
        let mut assertion = self.evidence().await?;
        let mut round = 1;
        let authorization = loop {
            // The proof is dated before signing, which may be held for long.
            let signed_at = crate::cached_assertion::unix_now();
            let authorization = self.proof(&proof).await?;
            let current = self.evidence().await?;
            let age = crate::cached_assertion::unix_now().saturating_sub(signed_at);
            if current == assertion && age < PROOF_MAX_AGE_SECS {
                break authorization;
            }
            if round == AUTHORIZE_ROUNDS {
                return Err(CliError::Unavailable(
                    "the NIP-FI assertion kept expiring while the proof was signed".into(),
                ));
            }
            (assertion, round) = (current, round + 1);
        };
        let mut req = req.header(reqwest::header::AUTHORIZATION, sensitive(authorization)?);
        if let Some(tag) = self.credentials.auth_tag() {
            req = req.header("x-auth-tag", sensitive(tag.json.clone())?);
        }
        if let Some(assertion) = assertion {
            req = req.header(
                FEDERATED_IDENTITY_HEADER,
                sensitive(format!("Bearer {assertion}"))?,
            );
        }
        Ok(req)
    }

    /// `proof`, signed now, as an `Authorization` value.
    async fn proof(&self, proof: &Proof<'_>) -> Result<String, CliError> {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        Ok(match *proof {
            Proof::Nip98 { method, url, body } => {
                format!("Nostr {}", self.credentials.nip98(method, url, body).await?)
            }
            Proof::BlossomUpload { sha256 } => {
                let event = self
                    .credentials
                    .sign(blossom_upload_event(sha256, &self.relay_url)?)
                    .await?;
                format!(
                    "Nostr {}",
                    URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
                )
            }
            Proof::BlossomGet { url } => {
                let event = self.credentials.sign(blossom_get_event(url)?).await?;
                format!(
                    "Nostr {}",
                    URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
                )
            }
        })
    }

    /// The WebSocket upgrade headers: the CLI's `User-Agent`, and the NIP-FI
    /// assertion when there is an evidence source.
    async fn upgrade_headers(&self) -> Result<buzz_ws_client::HeaderMap, CliError> {
        let mut headers = buzz_ws_client::HeaderMap::new();
        headers.insert(
            "user-agent",
            buzz_ws_client::HeaderValue::from_static(USER_AGENT),
        );
        if let Some(assertion) = self.evidence().await? {
            let mut value =
                buzz_ws_client::HeaderValue::from_str(&format!("Bearer {assertion}"))
                    .map_err(|_| CliError::Auth("assertion is not a valid header value".into()))?;
            value.set_sensitive(true);
            headers.insert(FEDERATED_IDENTITY_HEADER, value);
        }
        Ok(headers)
    }

    /// The NIP-FI assertion for this relay, noting that one was sent.
    async fn evidence(&self) -> Result<Option<String>, CliError> {
        let assertion = self.credentials.evidence().await?;
        if assertion.is_some() {
            self.assertion_sent
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(assertion)
    }

    /// Map a non-2xx relay response to a `CliError`.
    ///
    /// The relay's `{"error": …}` / `{"message": …}` field becomes the
    /// message. A 401 without a NIP-FI evidence source names the settings,
    /// since an enforcing relay answers every request that way.
    fn relay_error(&self, status: u16, body: String) -> CliError {
        let message = extract_relay_message_field(&body).unwrap_or(body);
        let message = message.trim_end().to_owned();
        let sent = self
            .assertion_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let desktop = self.credentials.is_desktop();
        if status == 401 && !sent && desktop {
            return CliError::Auth(format!(
                "relay requires authentication ({message}); if it enforces federated identity \
                 (NIP-FI), the Buzz desktop has no assertion for this relay: sign in to this \
                 community in Buzz, then retry"
            ));
        }
        if status == 401 && !sent && !self.credentials.has_evidence_source() {
            return CliError::Auth(format!(
                "relay requires authentication ({message}); if it enforces federated identity \
                 (NIP-FI), set {FEDERATED_IDENTITY_TOKEN_ENV} to your own NIP-FA sign-in session \
                 (a human session for this key, not an agent grant) and \
                 {FEDERATED_IDENTITY_ADAPTER_ENV} to its adapter, which mints the assertion"
            ));
        }
        CliError::Relay {
            status,
            body: message,
        }
    }

    /// Execute `op` up to `RETRY_MAX_ATTEMPTS` times, including body-transfer failures
    /// and transient relay error statuses.
    ///
    /// The closure is expected to consume the response body and return the parsed result
    /// as `T`. Retries on non-last attempts when `op` returns:
    ///
    /// - `Err(CliError::Network(e))` where `e.is_connect() || e.is_timeout() ||
    ///   e.is_request() || e.is_body() || e.is_decode()` — covers both connection
    ///   failures and mid-body TCP drops.
    /// - `Err(CliError::Relay { status: 429 | 502 | 503 | 504, .. })` — transient relay
    ///   or proxy errors. For 429 the `retry in Ns` hint from the body is used as the
    ///   delay (capped at `RETRY_IN_MAX_SECS`); all others use exponential jitter.
    ///
    /// Use this variant for all operations (reads, writes, uploads); the retry boundary
    /// covers the entire operation including response body transfer.
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
        filter: serde_json::Value,
        limit: Option<u32>,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        self.query_pages_keeping(filter, limit, None).await
    }

    /// Like `query_pages`, but with `keep`, only events it accepts count
    /// toward `limit`: paging continues past the ones it drops, always in
    /// full pages, for at most `MAX_QUERY_PAGES` pages.
    async fn query_pages_keeping(
        &self,
        mut filter: serde_json::Value,
        limit: Option<u32>,
        keep: Option<&dyn Fn(&serde_json::Value) -> bool>,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        let mut events = Vec::new();
        let mut pages = 0;

        while limit.is_none_or(|limit| events.len() < limit as usize) {
            if keep.is_some() && pages == MAX_QUERY_PAGES {
                return Err(stalled_query(&format!(
                    "read {MAX_QUERY_PAGES} pages without reaching the end of the results"
                )));
            }
            pages += 1;
            let page_limit = match (limit, keep) {
                (Some(limit), None) => {
                    (limit as usize - events.len()).min(QUERY_PAGE_SIZE as usize)
                }
                _ => QUERY_PAGE_SIZE as usize,
            };
            filter["limit"] = serde_json::json!(page_limit);

            let raw = self.query(&filter).await?;
            let page: Vec<serde_json::Value> = serde_json::from_str(&raw)
                .map_err(|e| CliError::Other(format!("failed to parse query response: {e}")))?;
            let done = page.len() < page_limit;

            if !done {
                advance_query_cursor(&mut filter, &page)?;
            }
            events.extend(page.into_iter().filter(|e| keep.is_none_or(|keep| keep(e))));
            if done {
                break;
            }
        }

        if let Some(limit) = limit {
            events.truncate(limit as usize);
        }
        Ok(events)
    }

    /// Query up to `limit` historical events that `keep` accepts, paging past
    /// the rest until the limit is met or the history runs out.
    pub async fn query_paginated_keeping(
        &self,
        filter: serde_json::Value,
        limit: u32,
        keep: impl Fn(&serde_json::Value) -> bool,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        self.query_pages_keeping(filter, Some(limit), Some(&keep))
            .await
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

    /// Query a filter exhaustively up to `max_events`.
    ///
    /// One extra event is requested so reaching the bound is reported as
    /// truncation instead of being mistaken for authoritative absence.
    pub async fn query_all_bounded(
        &self,
        filter: serde_json::Value,
        max_events: u32,
    ) -> Result<Vec<serde_json::Value>, CliError> {
        let probe_limit = max_events
            .checked_add(1)
            .ok_or_else(|| CliError::Other("query bound is too large".into()))?;
        let events = self.query_pages(filter, Some(probe_limit)).await?;
        if events.len() > max_events as usize {
            return Err(CliError::Other(format!(
                "query exceeded the exhaustive {max_events}-event bound; narrow the query or retry"
            )));
        }
        Ok(events)
    }

    /// Sign an event builder verbatim: no NIP-OA auth-tag injection, and none
    /// of [`sign_event`]'s "callers must not add auth tags" enforcement.
    ///
    /// Used only for NIP-IA identity archive/unarchive requests (kind
    /// 9035/9036), whose optional `auth` tag is a *content-level*
    /// owner-of-agent attestation about the *target* identity — unrelated to
    /// this client's own NIP-OA membership delegation (the credentials' auth tag,
    /// which [`sign_event`] injects into every other event and which
    /// `submit_event` separately attaches via the `x-auth-tag` HTTP header).
    /// Routing an identity archive request through `sign_event` would either
    /// silently drop the caller's owner attestation or double up an
    /// unrelated tag.
    pub async fn sign_event_unchecked(
        &self,
        builder: EventBuilder,
    ) -> Result<nostr::Event, CliError> {
        self.credentials.sign(builder).await
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
                let req = self
                    .authorize(
                        self.http
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .body(body.clone()),
                        Proof::post(&url, &body),
                    )
                    .await?;
                let resp = req.send().await?;
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
                let req = self
                    .authorize(
                        self.http
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .body(body.clone()),
                        Proof::post(&url, &body),
                    )
                    .await?;
                let resp = req.send().await?;
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
                let req = self
                    .authorize(
                        self.http.get(&url),
                        Proof::Nip98 {
                            method: "GET",
                            url: &url,
                            body: None,
                        },
                    )
                    .await?;
                let resp = req.send().await?;
                self.handle_response(resp).await
            }
        })
        .await
    }

    /// POST a JSON body to a relay-relative path with NIP-98 authentication.
    ///
    /// Used by `buzz gifs search` and `buzz gifs share` to reach the relay's
    /// KLIPY proxy endpoints.  Returns the raw response body as a string (may
    /// be empty for 204 No Content responses).
    pub async fn post_json_authed(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<String, CliError> {
        let url = format!("{}{path}", self.relay_url);
        let body_bytes = bytes::Bytes::from(
            serde_json::to_vec(body)
                .map_err(|e| CliError::Other(format!("request serialization failed: {e}")))?,
        );
        self.with_retry_body(|| {
            let body_bytes = body_bytes.clone();
            let url = url.clone();
            async move {
                let req = self
                    .authorize(
                        self.http
                            .post(&url)
                            .header("Content-Type", "application/json")
                            .body(body_bytes.clone()),
                        Proof::post(&url, &body_bytes),
                    )
                    .await?;
                let resp = req.send().await?;
                // 204 No Content: return empty string rather than failing on
                // an empty body that cannot be parsed as JSON.
                if resp.status() == reqwest::StatusCode::NO_CONTENT {
                    return Ok(String::new());
                }
                self.handle_response(resp).await
            }
        })
        .await
    }

    /// Send a state-changing JSON command exactly once. Ambiguous delivery
    /// never invites an automatic re-run with a newly observed version.
    pub async fn post_json_once_authed(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<String, CliError> {
        let url = format!("{}{path}", self.relay_url);
        let body = serde_json::to_vec(body).map_err(|e| CliError::Other(e.to_string()))?;
        let unknown = |detail: String| CliError::DeliveryUnknown(detail);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(env_duration_secs("BUZZ_TIMEOUT_SECS", 30))
            .connect_timeout(env_duration_secs("BUZZ_CONNECT_TIMEOUT_SECS", 15))
            .build()?;
        let req = self
            .authorize(
                http.post(&url)
                    .header("Content-Type", "application/json")
                    .body(body.clone()),
                Proof::post(&url, &body),
            )
            .await?;
        let response = req.send().await.map_err(|e| {
            if e.is_connect() || e.is_builder() {
                CliError::Network(e)
            } else {
                unknown(e.to_string())
            }
        })?;
        let status = response.status();
        let body = response.text().await.map_err(|e| unknown(e.to_string()))?;
        let message = extract_relay_message_field(&body).unwrap_or_else(|| body.clone());
        if status.is_server_error()
            || status.is_redirection()
            || (status.as_u16() == 429 && !message.starts_with("rate-limited:"))
        {
            return Err(unknown(format!("HTTP {}: {message}", status.as_u16())));
        }
        if !status.is_success() {
            return Err(self.relay_error(status.as_u16(), body));
        }
        Ok(body)
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
            let req = self
                .authorize(
                    self.http
                        .post(&url)
                        .header("Content-Type", "application/json")
                        .body(body.clone()),
                    Proof::post(&url, &body),
                )
                .await?;
            let send_result: Result<reqwest::Response, CliError> =
                req.send().await.map_err(CliError::from);

            match send_result {
                Err(e) => {
                    if let CliError::Network(ref net_err) = e {
                        // Only connect-failure is safe to retry: the relay never saw
                        // the request. Timeout and mid-request errors are ambiguous.
                        if !is_last && net_err.is_connect() {
                            tokio::time::sleep(jitter_delay(attempt)).await;
                            continue;
                        }
                        if net_err.is_connect() {
                            // Final attempt: definitively never reached the relay — retryable.
                            return Err(e);
                        }
                        if net_err.is_timeout()
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
                        // Only retry if the relay's own ingest layer rejected it:
                        // the extracted error/message field must start with
                        // "rate-limited:". A proxy-level 429 (or JSON-wrapped body
                        // whose field does not start with "rate-limited:") leaves
                        // relay execution state ambiguous.
                        let body_text = resp.text().await.unwrap_or_default();
                        let extracted = extract_relay_message_field(&body_text);
                        let msg = extracted.as_deref().unwrap_or(&body_text);
                        if msg.starts_with("rate-limited:") {
                            // Canonical pre-ingest 429: the relay provably did not execute
                            // the command. Retry while budget remains; on exhaustion return
                            // Relay(429) (retryable:true) — the caller may retry the exact
                            // same command. DeliveryUnknown is reserved for outcomes where
                            // relay execution is genuinely ambiguous (proxy 429, 502-504,
                            // timeout/body-loss after the relay may have acted).
                            if !is_last {
                                let delay = parse_retry_hint_text(msg)
                                    .map(|s| Duration::from_secs(s.min(RETRY_IN_MAX_SECS)))
                                    .unwrap_or_else(|| jitter_delay(attempt));
                                tokio::time::sleep(delay).await;
                                continue;
                            }
                            // Budget exhausted — still pre-ingest, still safe to retry.
                            return Err(CliError::Relay {
                                status: 429,
                                body: body_text,
                            });
                        }
                        // Non-canonical 429 (proxy-level or unrecognised body): outcome unknown.
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
                        return Err(self.relay_error(status, body_text));
                    }
                    return Ok(body_text);
                }
            }
        }
        unreachable!("loop exhausts all RETRY_MAX_ATTEMPTS")
    }

    /// Submit a stored event (all non-moderation kinds) with the standard retry policy.
    ///
    /// The full operation — network send AND response body read — is inside the retry
    /// boundary so that a dropped body after a 200 header is retried with the same
    /// serialized event bytes (and a fresh per-attempt NIP-98 auth event).
    ///
    /// **Exhaustion policy:** after all attempts, connect failures and canonical
    /// pre-ingest 429 remain retryable (`CliError::Network`/`CliError::Relay{429}`)
    /// because the relay provably never executed them. Any other final failure
    /// (timeout, request, body loss, decode, proxy 502-504) is ambiguous — the
    /// relay may have stored the event — so we surface `DeliveryUnknown`
    /// (retryable:false) to prevent an outer re-sign creating a duplicate write.
    /// Content-addressed uploads are exempt: same bytes ⇒ same hash, so outer
    /// re-run is safe regardless of the failure kind.
    async fn submit_stored_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let url = format!("{}/events", self.relay_url);
        let body = bytes::Bytes::from(
            serde_json::to_vec(&event)
                .map_err(|e| CliError::Other(format!("event serialization failed: {e}")))?,
        );
        // Whether any attempt reached the relay without an authoritative
        // answer. A later failure, even one before sending, cannot clear it.
        let maybe_delivered = std::sync::atomic::AtomicBool::new(false);
        let result = self
            .with_retry_body(|| {
                let body = body.clone();
                let url = url.clone();
                let maybe_delivered = &maybe_delivered;
                async move {
                    // Re-sign NIP-98 each attempt: the nonce tag generates a fresh
                    // event ID, keeping retries safe against the relay's replay guard.
                    let req = self
                        .authorize(
                            self.http
                                .post(&url)
                                .header("Content-Type", "application/json")
                                .body(body.clone()),
                            Proof::post(&url, &body),
                        )
                        .await?;
                    let sent = match req.send().await {
                        Ok(resp) => self.handle_response(resp).await,
                        Err(e) => Err(e.into()),
                    };
                    if let Err(ref e) = sent {
                        if is_stored_event_exhaustion_ambiguous(e) {
                            maybe_delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    sent
                }
            })
            .await;

        // Translate ambiguous final errors to DeliveryUnknown so an outer agent
        // following retryable:true does not re-sign and risk a duplicate write.
        // Connect failures stay Network (retryable:true) — definitively never received.
        // Canonical pre-ingest 429 (Relay{429}) stays retryable — definitively not stored.
        if let Err(ref e) = result {
            // Once an attempt may have been stored, no later answer settles
            // the outcome, except a canvas 409, which the canvas command
            // resolves by looking the canvas up.
            let settled = matches!(e, CliError::Relay { status: 409, body } if body.contains("canvas changed"));
            if is_stored_event_exhaustion_ambiguous(e)
                || (maybe_delivered.load(std::sync::atomic::Ordering::Relaxed) && !settled)
            {
                let kind_u16 = event.kind.as_u16();
                return Err(CliError::DeliveryUnknown(format!(
                    "stored event (kind {kind_u16}) outcome unknown after all attempts: {e}"
                )));
            }
        }
        result
    }

    /// Publish an ephemeral event via WebSocket with NIP-42 authentication.
    ///
    /// The relay accepts only typing (kind:20002) of the ephemeral kinds
    /// (20000–29999) over HTTP. Delegates to
    /// `buzz_ws_client::publish_event` which handles connect, NIP-42 auth,
    /// EVENT send, OK wait, and graceful close.
    pub async fn publish_ephemeral_event(&self, event: nostr::Event) -> Result<String, CliError> {
        let ws_url = &self.ws_url;
        // Two kinds of deadline: each network phase is capped as a whole,
        // writes included (the connect below, then buzz_ws_client's challenge,
        // AUTH, EVENT and close phases), and NIP-42 signing between them is
        // bounded by the signer itself. A
        // desktop provider may wait out throttling and hold `sign_event` for a
        // person's decision, which no relay budget should cut short.
        let signer = crate::credentials::CapturingSigner::new(&self.credentials);
        let ws_error = |e: buzz_ws_client::WsClientError| match e.upgrade_rejection() {
            // A denial decided on the upgrade is an HTTP response (NIP-FI).
            Some((status, body)) => self.relay_error(status, body),
            // A post-upgrade denial (e.g. a NIP-FI key mismatch) is a 1008 close.
            None if matches!(e, buzz_ws_client::WsClientError::PolicyClose(_)) => {
                CliError::Auth(e.to_string())
            }
            None => CliError::Other(e.to_string()),
        };
        //
        // The relay closes a socket that has not signed in within seconds,
        // so a slow or throttled sign-in signature reconnects and signs the
        // new challenge, a bounded number of times, with current evidence:
        // the old assertion may have expired meanwhile. A throttled sign-in
        // is waited out between connections, never on one. EVENT, signed
        // before any of this, is sent only once AUTH succeeds.
        let mut attempt = 1;
        let mut conn = loop {
            let headers = self.upgrade_headers().await?;
            let mut conn = tokio::time::timeout(
                WS_CONNECT_TIMEOUT,
                buzz_ws_client::NostrWsConnection::connect_with_headers(ws_url, headers),
            )
            .await
            .map_err(|_| CliError::Other("relay WebSocket connect timed out".into()))?
            .map_err(ws_error)?;
            let signed_in = conn
                .authenticate(&signer, self.credentials.auth_tag().map(|t| &t.tag))
                .await;
            let Err(e) = signed_in else { break conn };
            let failure = signer.take_failure();
            let lost = matches!(
                e,
                buzz_ws_client::WsClientError::ConnectionClosed
                    | buzz_ws_client::WsClientError::WebSocket(_)
            );
            let (failure, wait) = failure.unzip();
            let wait = wait.flatten();
            let throttled = matches!(failure, Some(CliError::Throttled(_)));
            let _ = conn.disconnect().await;
            if !(lost || throttled) {
                return Err(failure.unwrap_or_else(|| ws_error(e)));
            }
            if attempt == WS_SIGN_IN_ATTEMPTS {
                return Err(failure.unwrap_or_else(|| {
                    CliError::Unavailable(format!(
                        "the relay closed the WebSocket before sign-in completed, \
                         {WS_SIGN_IN_ATTEMPTS} times: {e}"
                    ))
                }));
            }
            tokio::time::sleep(wait.unwrap_or_else(|| jitter_delay(attempt - 1))).await;
            attempt += 1;
        };
        let ok = conn.send_event(event).await.map_err(ws_error)?;
        let _ = conn.disconnect().await;

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
            .map_err(|e| CliError::Usage(format!("cannot access {file_path}: {e}")))?;
        if !metadata.is_file() {
            return Err(CliError::Usage(format!("{file_path} is not a file")));
        }

        let bytes = std::fs::read(file_path)
            .map_err(|e| CliError::Usage(format!("failed to read {file_path}: {e}")))?;

        // 2. Detect MIME from magic bytes
        let mime = infer::get(&bytes)
            .map(|t| t.mime_type().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // 3. Size check. The relay decides which other types it accepts.
        let max = if mime == "video/mp4" {
            MAX_VIDEO_BYTES
        } else if MEDIA_MIMES.contains(&mime.as_str()) {
            MAX_IMAGE_BYTES
        } else {
            MAX_FILE_BYTES
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
        let upload_timeout = if max > MAX_IMAGE_BYTES {
            Duration::from_secs(600)
        } else {
            Duration::from_secs(120)
        };
        let url = format!("{}/upload", self.relay_url);
        let upload_body = bytes::Bytes::from(bytes);

        // The full upload operation — network send AND response body read — lives inside
        // with_retry_body so that a dropped body after 200 headers is retried with the
        // same file bytes and a fresh Blossom auth per attempt.
        let result: Result<BlobDescriptor, CliError> = self
            .with_retry_body(|| {
                let upload_body = upload_body.clone();
                let url = url.clone();
                let mime = mime.clone();
                let sha256 = sha256.clone();
                async move {
                    let req = self
                        .authorize(
                            self.http
                                .put(&url)
                                .timeout(upload_timeout)
                                .header("Content-Type", &mime)
                                .header("X-SHA-256", &sha256)
                                .body(upload_body),
                            Proof::BlossomUpload { sha256: &sha256 },
                        )
                        .await?;
                    let resp = req.send().await?;
                    let status = resp.status();
                    if !status.is_success() {
                        let s = status.as_u16();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(self.relay_error(s, body));
                    }
                    resp.json::<BlobDescriptor>().await.map_err(CliError::from)
                }
            })
            .await;

        // If the primary /upload endpoint definitively doesn't exist on this relay version
        // (404 or 405), fall back to the legacy /media/upload endpoint.  The 404/405 switch
        // itself is not retried; only transient failures on the selected legacy endpoint are.
        match result {
            Ok(desc) => return Ok(desc),
            Err(CliError::Relay { status: s, body: _ })
                if should_retry_legacy_upload(
                    reqwest::StatusCode::from_u16(s).unwrap_or(reqwest::StatusCode::NOT_FOUND),
                ) =>
            {
                // Fall through to legacy endpoint below.
            }
            Err(e) => return Err(e),
        }

        let legacy_url = format!("{}/media/upload", self.relay_url);
        self.with_retry_body(|| {
            let upload_body = upload_body.clone();
            let legacy_url = legacy_url.clone();
            let mime = mime.clone();
            let sha256 = sha256.clone();
            async move {
                let req = self
                    .authorize(
                        self.http
                            .put(&legacy_url)
                            .timeout(upload_timeout)
                            .header("Content-Type", &mime)
                            .header("X-SHA-256", &sha256)
                            .body(upload_body),
                        Proof::BlossomUpload { sha256: &sha256 },
                    )
                    .await?;
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(self.relay_error(status, body));
                }
                resp.json::<BlobDescriptor>().await.map_err(CliError::from)
            }
        })
        .await
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
                let req = self
                    .authorize(client.get(&url), Proof::BlossomGet { url: &url })
                    .await?;
                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(self.relay_error(status, body));
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
            return Err(self.relay_error(status, body));
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

/// Normalize raw event JSON array into the canonical Nostr event shape.
/// String signatures are preserved; absent or non-string signatures remain absent.
pub fn normalize_events(events: &[serde_json::Value]) -> String {
    let normalized: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            let mut event = serde_json::json!({
                "id": e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
                "kind": e.get("kind").and_then(|v| v.as_u64()).unwrap_or(0),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "tags": e.get("tags").cloned().unwrap_or(serde_json::json!([])),
            });
            if let Some(sig) = e.get("sig").and_then(|v| v.as_str()) {
                event["sig"] = serde_json::json!(sig);
            }
            event
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

/// Return a create-command response, injecting the entity ID **only** when the
/// relay accepted the event (`"accepted": true`). When the relay rejected the
/// event, emitting the locally-computed link would be misleading — callers
/// that copy or share the link would reference an event that was never stored.
pub fn create_response_with_id_if_accepted(resp: &str, id_key: &str, id_val: &str) -> String {
    let mut v: serde_json::Value = serde_json::from_str(resp).unwrap_or(serde_json::json!({}));
    let accepted = v.get("accepted").and_then(|a| a.as_bool()).unwrap_or(false);
    if accepted {
        v[id_key] = serde_json::json!(id_val);
    }
    v.to_string()
}

/// Print a create-command response, injecting the generated entity ID.
pub fn print_create_response(resp: &str, id_key: &str, id_val: &str) {
    println!(
        "{}",
        create_response_with_id_if_accepted(resp, id_key, id_val)
    );
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
        BuzzClient::new(
            base_url.to_string(),
            crate::credentials::Credentials::from_keys(Keys::generate()),
        )
        .unwrap()
    }

    fn client_keys(client: &BuzzClient) -> Keys {
        client.credentials().test_keys()
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
        let event = make_moderation_event(&client_keys(&client), 9040);
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

    /// A moderation command (kind 9041) that gets a relay-ingest 429 (production JSON
    /// envelope `{"error":"rate-limited: ..."}`) IS retried, and the `retry in Ns` hint
    /// is honoured.
    ///
    /// Uses a 2s hint; jitter max for attempt 0 is 0.5s, so asserting elapsed ≥ 2s
    /// cleanly distinguishes hint-honoured from jitter-fallback.
    #[tokio::test]
    async fn moderation_kind_ingest_429_is_retried_until_success() {
        let (url, attempts) = test_server(|n| {
            if n < 2 {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    // Exact production envelope: api_error() wraps every message as
                    // {"error":"..."}.  The extracted field starts with "rate-limited:"
                    // so the command is retried; the hint is honoured.
                    r#"{"error":"rate-limited: quota exceeded; retry in 2s"}"#.to_string(),
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
        let event = make_moderation_event(&client_keys(&client), 9041);
        let t0 = std::time::Instant::now();
        let result = client.submit_event(event).await;
        let elapsed = t0.elapsed();
        assert!(
            result.is_ok(),
            "expected Ok after ingest-429 retry, got {result:?}"
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

    /// A moderation command that receives the canonical pre-ingest 429 on EVERY
    /// attempt exhausts the retry budget and surfaces `CliError::Relay { status: 429 }` —
    /// NOT `DeliveryUnknown`. The relay provably never executed the command on any
    /// attempt, so the caller must be told it is safe to retry.
    #[tokio::test]
    async fn exhausted_ingest_429_returns_relay_429_retryable() {
        let (url, attempts) = test_server(|_n| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":"rate-limited: quota exceeded; retry in 0s"}"#.to_string(),
            )
        })
        .await;
        let client = test_client(&url);
        let event = make_moderation_event(&client_keys(&client), 9040);
        let err = client.submit_event(event).await.unwrap_err();

        // Must be Relay(429), not DeliveryUnknown.
        assert!(
            matches!(err, CliError::Relay { status: 429, .. }),
            "exhausted ingest 429 must surface as Relay(429), got {err:?}"
        );
        // Must NOT be retryable:false.
        assert!(
            crate::error::is_retryable_error(&err),
            "Relay(429) must be retryable; got {err:?}"
        );
        // All RETRY_MAX_ATTEMPTS must have been tried.
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "all retry attempts must fire for exhausted ingest 429"
        );
    }

    /// A moderation command (kind 9042) that gets HTTP 502 returns `DeliveryUnknown`
    /// immediately — proxy errors leave relay execution state ambiguous.
    #[tokio::test]
    async fn moderation_kind_502_returns_delivery_unknown() {
        let (url, attempts) =
            test_server(|_n| (StatusCode::BAD_GATEWAY, "bad gateway".to_string())).await;
        let client = test_client(&url);
        let event = make_moderation_event(&client_keys(&client), 9042);
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

    /// When all retry attempts are connect-failures (the relay definitively never saw
    /// the request), `submit_event` must return `CliError::Network` with
    /// `retryable:true` — not `DeliveryUnknown`.  Connect-failure is the one error
    /// condition the implementation itself identifies as confirmed-unreceived.
    #[tokio::test]
    async fn exhausted_connect_failures_return_network_retryable() {
        // Bind a port, capture the address, then drop the listener so every
        // subsequent connect attempt is refused immediately.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let base = format!("http://{addr}");
        let client = test_client(&base);
        let event = make_moderation_event(&client_keys(&client), 9040);
        let err = client.submit_event(event).await.unwrap_err();
        // Must be Network (retryable), not DeliveryUnknown (retryable:false).
        assert!(
            matches!(err, super::super::error::CliError::Network(_)),
            "exhausted connect failures must surface as Network, got {err:?}"
        );
        // Confirm the error description does not suggest ambiguous delivery.
        let description = format!("{err:?}");
        assert!(
            !description.contains("outcome unknown"),
            "connect failure must not be labeled DeliveryUnknown; got: {description}"
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
        let event = make_stored_event(&client_keys(&client));
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

    /// `submit_event` (non-moderation kind) uses `with_retry_body` — the full
    /// operation including response body read is inside the retry boundary.
    /// A partial-body drop after 200 headers must be retried with the same
    /// serialized event bytes (and a fresh NIP-98 auth per attempt).
    #[tokio::test]
    async fn stored_event_body_loss_is_retried_with_same_event_bytes() {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        let counter = Arc::new(AtomicU32::new(0));
        let counter2 = counter.clone();
        let bodies: Arc<std::sync::Mutex<Vec<Vec<u8>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies2 = bodies.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let n = counter2.fetch_add(1, Ordering::SeqCst) + 1;

                // Read the full HTTP request so we can capture the body.
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    stream.read(&mut buf),
                )
                .await;
                // Capture raw request bytes for assertion.
                let body_end = buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|i| i + 4)
                    .unwrap_or(0);
                let payload = buf[body_end..].to_vec();
                bodies2.lock().unwrap().push(payload);

                if n < 3 {
                    // Partial body drop.
                    let partial = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"partial\":";
                    let _ = stream.write_all(partial).await;
                } else {
                    let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 41\r\n\r\n{\"event_id\":\"abc\",\"accepted\":true,\"message\":\"\"}";
                    let _ = stream.write_all(ok).await;
                }
            }
        });

        let base = format!("http://{addr}");
        let client = test_client(&base);
        let event = make_stored_event(&client_keys(&client));
        let result = client.submit_event(event).await;
        assert!(
            result.is_ok(),
            "expected Ok after body-loss retries, got {result:?}"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "expected 3 attempts (2 body-loss + 1 success)"
        );
        // All three attempts must have sent the same serialized event bytes.
        let captured = bodies.lock().unwrap();
        assert_eq!(captured.len(), 3, "must have captured 3 request bodies");
        // Each attempt's payload must be identical (same signed event bytes).
        assert_eq!(
            captured[0], captured[1],
            "attempt 1 and 2 must use identical event bytes"
        );
        assert_eq!(
            captured[1], captured[2],
            "attempt 2 and 3 must use identical event bytes"
        );
    }

    /// `upload_file` uses `with_retry_body` — the full operation including response
    /// body read is inside the retry boundary.  A partial-body drop after 200 headers
    /// must be retried with identical file bytes and a fresh Blossom auth per attempt.
    #[tokio::test]
    async fn upload_body_loss_is_retried_with_same_file_bytes() {
        use std::io::Write;
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        // Write a minimal JPEG file so MIME detection works.
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // JPEG magic + JFIF app0 marker: enough for `infer` to detect image/jpeg.
        let jpeg_header: &[u8] = &[
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
        ];
        tmp.write_all(jpeg_header).unwrap();
        let file_path = tmp.path().to_str().unwrap().to_string();

        let counter = Arc::new(AtomicU32::new(0));
        let counter2 = counter.clone();
        let auth_headers: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let auth_headers2 = auth_headers.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let n = counter2.fetch_add(1, Ordering::SeqCst) + 1;

                // Read the request headers to extract the Authorization value.
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    stream.read(&mut buf),
                )
                .await;
                // Extract the Authorization header value.
                let req_str = String::from_utf8_lossy(&buf);
                let auth = req_str
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .map(|l| l.to_string())
                    .unwrap_or_default();
                auth_headers2.lock().unwrap().push(auth);

                if n < 3 {
                    // Partial body drop.
                    let partial = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"partial\":";
                    let _ = stream.write_all(partial).await;
                } else {
                    // Valid BlobDescriptor response.
                    let ok_body = r#"{"url":"https://relay.test/media/aabbcc.jpg","sha256":"aabbcc","size":12,"type":"image/jpeg","uploaded":0}"#;
                    let ok = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        ok_body.len(),
                        ok_body
                    );
                    let _ = stream.write_all(ok.as_bytes()).await;
                }
            }
        });

        let base = format!("http://{addr}");
        let client = test_client(&base);
        let result = client.upload_file(&file_path).await;
        assert!(
            result.is_ok(),
            "expected Ok after upload body-loss retries, got {result:?}"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "expected 3 upload attempts (2 body-loss + 1 success)"
        );
        // Each attempt must carry a distinct Authorization header (fresh Blossom auth).
        let auths = auth_headers.lock().unwrap();
        assert_eq!(auths.len(), 3, "must have captured 3 auth headers");
        // All three must be non-empty (auth was signed).
        assert!(
            auths.iter().all(|a| a.contains("Nostr ")),
            "each attempt must carry Nostr auth"
        );
    }

    /// When all retry attempts for a stored event end with a partial body (200
    /// headers, dropped connection), the final error must be `DeliveryUnknown`
    /// (retryable:false) — the relay may have stored the event on any attempt, so
    /// an outer re-sign would risk a duplicate visible write.  All three attempts
    /// must fire with identical serialized event bytes.
    #[tokio::test]
    async fn stored_event_all_body_losses_return_delivery_unknown() {
        use tokio::io::AsyncWriteExt;

        let bodies: Arc<std::sync::Mutex<Vec<Vec<u8>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let bodies2 = bodies.clone();
        let counter = Arc::new(AtomicU32::new(0));
        let counter2 = counter.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                counter2.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    stream.read(&mut buf),
                )
                .await;
                // Extract the request body (after the blank line separating headers).
                let raw = buf.split(|&b| b == 0).next().unwrap_or(&buf).to_vec();
                if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    bodies2.lock().unwrap().push(raw[pos + 4..].to_vec());
                }
                // Partial body: send headers + truncated body, then drop.
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"partial\":",
                    )
                    .await;
                // Drop stream — causes body-loss error on the client side.
            }
        });

        let base = format!("http://{addr}");
        let client = test_client(&base);
        let event = make_stored_event(&client_keys(&client));
        let err = client.submit_event(event).await.unwrap_err();

        // Final error must be DeliveryUnknown — relay may have accepted any attempt.
        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "all-body-loss exhaustion must return DeliveryUnknown, got {err:?}"
        );
        // All RETRY_MAX_ATTEMPTS must have fired.
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "all 3 attempts must be made before surfacing DeliveryUnknown"
        );
        // All attempts must have sent identical serialized event bytes.
        let captured = bodies.lock().unwrap();
        if captured.len() >= 2 {
            assert_eq!(
                captured[0], captured[1],
                "all attempts must use identical event bytes"
            );
        }
    }

    /// When all retry attempts for a stored event return HTTP 502, the final error
    /// must be `DeliveryUnknown` (retryable:false) — a proxy 502 may occur after
    /// the relay accepted the event.
    #[tokio::test]
    async fn stored_event_all_502s_return_delivery_unknown() {
        let (url, attempts) =
            test_server(|_n| (StatusCode::BAD_GATEWAY, "bad gateway".to_string())).await;
        let client = test_client(&url);
        let event = make_stored_event(&client_keys(&client));
        let err = client.submit_event(event).await.unwrap_err();

        assert!(
            matches!(err, CliError::DeliveryUnknown(_)),
            "all-502 exhaustion must return DeliveryUnknown, got {err:?}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "all 3 attempts must fire before surfacing DeliveryUnknown"
        );
    }

    /// The first send is stored but its answer is lost; the retry is then
    /// refused before ingest. The refusal cannot undo the first send, so the
    /// outcome stays unknown rather than a plain rejection.
    #[tokio::test]
    async fn a_403_after_a_lost_stored_send_stays_delivery_unknown() {
        let (url, attempts) = test_server(|n| match n {
            1 => (StatusCode::BAD_GATEWAY, "bad gateway".to_string()),
            _ => (
                StatusCode::FORBIDDEN,
                r#"{"error":"restricted: not allowed"}"#.to_string(),
            ),
        })
        .await;
        let client = test_client(&url);
        let event = make_stored_event(&client_keys(&client));
        let err = client.submit_event(event).await.unwrap_err();

        assert!(matches!(err, CliError::DeliveryUnknown(_)), "{err:?}");
        assert!(!crate::error::is_retryable_error(&err));
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "no retry after the 403");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_file_is_bad_input() {
        use std::os::unix::fs::PermissionsExt;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        let client = BuzzClient::new(
            "http://127.0.0.1:9".into(),
            crate::credentials::Credentials::from_keys(nostr::Keys::generate()),
        )
        .unwrap();
        let err = client
            .upload_file(file.path().to_str().unwrap())
            .await
            .unwrap_err();
        assert!(
            matches!(err, crate::error::CliError::Usage(ref m) if m.contains("failed to read")),
            "{err:?}"
        );
    }

    use super::{
        advance_query_cursor, create_response_with_id_if_accepted, extract_relay_response_field,
        normalize_events, BuzzClient, Credentials, Proof,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn normalize_events_preserves_the_complete_signed_event_shape() {
        let signed_event = EventBuilder::new(Kind::TextNote, "signed content")
            .tags([Tag::parse(["h", "channel-id"]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let mut event = serde_json::to_value(&signed_event).unwrap();
        event["relay_internal"] = serde_json::json!("excluded");

        let output: Vec<serde_json::Value> =
            serde_json::from_str(&normalize_events(&[event])).unwrap();
        let normalized = &output[0];
        let round_tripped: nostr::Event = serde_json::from_value(normalized.clone()).unwrap();

        assert_eq!(round_tripped, signed_event);
        round_tripped.verify().unwrap();
        assert!(normalized.get("sig").is_some());
        assert!(normalized.get("relay_internal").is_none());
    }

    #[test]
    fn normalize_events_omits_missing_or_non_string_signatures() {
        let output: Vec<serde_json::Value> = serde_json::from_str(&normalize_events(&[
            serde_json::json!({}),
            serde_json::json!({"sig": 42}),
        ]))
        .unwrap();

        assert!(output[0].get("sig").is_none());
        assert!(output[1].get("sig").is_none());
    }

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
    fn create_response_with_id_if_accepted_injects_id_when_accepted() {
        let raw = r#"{"event_id":"abc","accepted":true,"message":"response:{\"workflow_id\":\"relay-id\"}"}"#;
        let out = create_response_with_id_if_accepted(raw, "workflow_id", "relay-id");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // ID injected and original fields preserved when accepted.
        assert_eq!(v["workflow_id"].as_str(), Some("relay-id"));
        assert_eq!(v["event_id"].as_str(), Some("abc"));
        assert_eq!(v["accepted"].as_bool(), Some(true));
    }

    #[test]
    fn create_response_with_id_if_accepted_omits_id_when_rejected() {
        let raw = r#"{"event_id":"abc","accepted":false,"message":"duplicate"}"#;
        let out = create_response_with_id_if_accepted(raw, "workflow_id", "local-id");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // ID must not be present when relay rejected the event; emitting a
        // link to an event that was never stored would mislead callers.
        assert!(
            v.get("workflow_id").is_none(),
            "link field must be absent on rejected create"
        );
        assert_eq!(v["accepted"].as_bool(), Some(false));
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

    #[tokio::test]
    async fn sign_event_unchecked_does_not_inject_ambient_auth_tag() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            Credentials::from_keys(keys).with_auth_tag(auth_tag, auth_json),
        )
        .unwrap();

        let builder =
            EventBuilder::new(Kind::Custom(9035), "archive").tags([Tag::parse(["-"]).unwrap()]);
        let event = client.sign_event_unchecked(builder).await.unwrap();

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

    #[tokio::test]
    async fn sign_event_unchecked_preserves_callers_content_auth_tag() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            Credentials::from_keys(keys).with_auth_tag(auth_tag, auth_json),
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
        let event = client.sign_event_unchecked(builder).await.unwrap();

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

    #[tokio::test]
    async fn authorize_sets_auth_tag_header_when_configured() {
        let keys = Keys::generate();
        let (auth_tag, auth_json) = make_auth_tag();
        let client = BuzzClient::new(
            "https://test.relay".into(),
            Credentials::from_keys(keys).with_auth_tag(auth_tag, auth_json.clone()),
        )
        .unwrap();

        let url = "https://test.relay/events";
        let built = client
            .authorize(client.http.post(url), Proof::post(url, b"{}"))
            .await
            .unwrap()
            .build()
            .unwrap();
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

    #[tokio::test]
    async fn authorize_omits_optional_headers_when_not_configured() {
        let client = BuzzClient::new(
            "https://test.relay".into(),
            Credentials::from_keys(Keys::generate()),
        )
        .unwrap();

        let url = "https://test.relay/events";
        let built = client
            .authorize(client.http.post(url), Proof::post(url, b"{}"))
            .await
            .unwrap()
            .build()
            .unwrap();
        assert!(built.headers().get("authorization").is_some());
        assert!(
            built.headers().get("x-auth-tag").is_none(),
            "x-auth-tag header must not be present when no auth tag is configured"
        );
        assert!(
            built.headers().get("nostr-federated-identity").is_none(),
            "no NIP-FI header without an evidence source"
        );
    }
}

/// NIP-FI evidence on every protected route (Review-Proven Rule #2: every
/// request path that carries auth is enumerated here, plus the WebSocket
/// upgrade). One mock origin serves both the NIP-FA adapter and the relay.
#[cfg(test)]
mod federated_identity_route_tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::Request;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use nostr::{EventBuilder, Keys, Kind};

    use super::BuzzClient;
    use crate::credentials::Credentials;
    use crate::error::CliError;
    use crate::federated_identity::AdapterSession;

    use crate::federated_identity::test_support::{self, count, raw_adapter};

    const HASH: &str = "4f2b5e7c1a9d3e6f8b0c2d4e6f8a0b1c3d5e7f9a1b3c5d7e9f0a2b4c6d8e0f1a";

    #[derive(Default)]
    struct Log {
        mints: usize,
        /// The last assertion the adapter issued.
        assertion: String,
        requests: Vec<(String, String, HeaderMap)>,
    }

    /// Serve the adapter and relay on one loopback origin. Relay routes
    /// answer `deny` when set, otherwise a plausible success; `PUT /upload`
    /// is 404 so the legacy `/media/upload` fallback runs too.
    async fn origin(deny: Option<StatusCode>) -> (String, Arc<Mutex<Log>>) {
        let log: Arc<Mutex<Log>> = Arc::default();
        let state = log.clone();
        let app = axum::Router::new()
            .route(
                "/v1/identity/assertions",
                axum::routing::post({
                    let log = log.clone();
                    move |body: axum::body::Bytes| {
                        let log = log.clone();
                        async move {
                            let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let pubkey = req["nostr_pubkey"].as_str().unwrap_or_default();
                            let expires_at = nostr::Timestamp::now().as_secs() + 300;
                            let assertion = test_support::assertion(pubkey, expires_at);
                            let mut log = log.lock().unwrap();
                            log.mints += 1;
                            log.assertion = assertion.clone();
                            axum::Json(serde_json::json!({
                                "assertion": assertion,
                                "nostr_pubkey": pubkey,
                                "expires_at": expires_at,
                            }))
                        }
                    }
                }),
            )
            .fallback(move |req: Request| {
                let log = state.clone();
                async move {
                    let method = req.method().to_string();
                    let path = req.uri().path().to_owned();
                    log.lock().unwrap().requests.push((
                        method.clone(),
                        path.clone(),
                        req.headers().clone(),
                    ));
                    if let Some(status) = deny {
                        return (status, r#"{"error":"authentication required"}"#).into_response();
                    }
                    let body = match (method.as_str(), path.as_str()) {
                        ("PUT", "/upload") => return StatusCode::NOT_FOUND.into_response(),
                        // No WebSocket server: the test only needs the upgrade's headers.
                        ("GET", "/") => return StatusCode::FORBIDDEN.into_response(),
                        ("PUT", "/media/upload") => serde_json::json!({
                            "url": format!("http://x/media/{HASH}.png"),
                            "sha256": HASH,
                            "size": 8,
                            "type": "image/png",
                            "uploaded": 0,
                        })
                        .to_string(),
                        ("POST", "/query") => "[]".to_owned(),
                        ("POST", "/count") => r#"{"count":0}"#.to_owned(),
                        _ => r#"{"accepted":true}"#.to_owned(),
                    };
                    (StatusCode::OK, body).into_response()
                }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, log)
    }

    fn client(base: &str, evidence: bool) -> BuzzClient {
        if evidence {
            client_via(base, base)
        } else {
            BuzzClient::new(base.to_owned(), Credentials::from_keys(Keys::generate())).unwrap()
        }
    }

    /// A client for the relay at `relay` that mints from the adapter at `adapter`.
    fn client_via(relay: &str, adapter: &str) -> BuzzClient {
        let credentials = Credentials::from_keys(Keys::generate())
            .with_evidence(AdapterSession::new(adapter, "token", relay).unwrap());
        BuzzClient::new(relay.to_owned(), credentials).unwrap()
    }

    /// Drive every protected request path once, ignoring results (the WS
    /// publish fails by design once its upgrade headers are recorded).
    async fn exercise_every_route(client: &BuzzClient) -> Vec<Result<String, CliError>> {
        let keys = client.credentials().test_keys();
        let png = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        std::fs::write(png.path(), b"\x89PNG\r\n\x1a\n").unwrap();
        let filter = serde_json::json!({"kinds": [1]});
        let body = serde_json::json!({"k": "v"});
        let event = |kind: u16| {
            EventBuilder::new(Kind::Custom(kind), "")
                .sign_with_keys(&keys)
                .unwrap()
        };
        let mut results = vec![
            client.query(&filter).await,
            client.query_multi(&[filter.clone(), filter.clone()]).await,
            client.count(&filter).await,
            client.get_authed("/api/get").await,
            client.post_json_authed("/api/post", &body).await,
            client.post_json_once_authed("/api/post-once", &body).await,
            client.submit_event(event(1)).await,
            client.submit_event(event(9041)).await,
            client.publish_ephemeral_event(event(20001)).await,
        ];
        results.push(
            client
                .upload_file(png.path().to_str().unwrap())
                .await
                .map(|d| d.url),
        );
        results.push(
            client
                .download_media(&format!("{HASH}.png"))
                .await
                .map(|_| String::new()),
        );
        results
    }

    /// Each `(method, path)` the CLI sends on a protected route.
    const PROTECTED: &[(&str, &str)] = &[
        ("POST", "/query"),
        ("POST", "/count"),
        ("GET", "/api/get"),
        ("POST", "/api/post"),
        ("POST", "/api/post-once"),
        ("POST", "/events"),
        ("GET", "/"),
        ("PUT", "/upload"),
        ("PUT", "/media/upload"),
    ];

    fn covered(log: &Log) -> Vec<(String, String)> {
        let mut seen: Vec<(String, String)> = log
            .requests
            .iter()
            .map(|(m, p, _)| (m.clone(), p.clone()))
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }

    /// The relay proof is signed after the assertion arrives, so a slow
    /// mint (adapter retries can exceed NIP-98's 60 s window) cannot age it.
    #[tokio::test]
    async fn the_relay_proof_is_signed_after_a_slow_mint() {
        use base64::Engine;
        let minted_at: Arc<Mutex<u64>> = Arc::default();
        let proofs: Arc<Mutex<Vec<String>>> = Arc::default();
        let app = axum::Router::new()
            .route(
                "/v1/identity/assertions",
                axum::routing::post({
                    let minted_at = minted_at.clone();
                    move |body: axum::body::Bytes| {
                        let minted_at = minted_at.clone();
                        async move {
                            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                            let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let pubkey = req["nostr_pubkey"].as_str().unwrap_or_default();
                            let now = nostr::Timestamp::now().as_secs();
                            *minted_at.lock().unwrap() = now;
                            axum::Json(serde_json::json!({
                                "assertion": test_support::assertion(pubkey, now + 300),
                                "nostr_pubkey": pubkey,
                                "expires_at": now + 300,
                            }))
                        }
                    }
                }),
            )
            .fallback({
                let proofs = proofs.clone();
                move |headers: HeaderMap| {
                    let proofs = proofs.clone();
                    async move {
                        let auth = headers["authorization"].to_str().unwrap().to_owned();
                        proofs.lock().unwrap().push(auth);
                        (StatusCode::OK, "{}")
                    }
                }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        client(&base, true).get_authed("/api/get").await.unwrap();

        let proof = proofs.lock().unwrap()[0].clone();
        let json = base64::engine::general_purpose::STANDARD
            .decode(proof.trim_start_matches("Nostr "))
            .unwrap();
        let event: nostr::Event = serde_json::from_slice(&json).unwrap();
        assert!(
            event.created_at.as_secs() >= *minted_at.lock().unwrap(),
            "the proof was signed before the assertion arrived"
        );
    }

    #[tokio::test]
    async fn every_protected_route_and_the_upgrade_carry_the_assertion() {
        let (base, log) = origin(None).await;
        let results = exercise_every_route(&client(&base, true)).await;

        let log = log.lock().unwrap();
        for (method, path) in PROTECTED {
            assert!(
                covered(&log).contains(&(method.to_string(), path.to_string())),
                "{method} {path} was not exercised"
            );
        }
        assert!(
            log.requests
                .iter()
                .any(|(_, p, _)| p.starts_with("/media/") && p != "/media/upload"),
            "GET /media/<hash> was not exercised"
        );
        for (method, path, headers) in &log.requests {
            assert_eq!(
                headers
                    .get("nostr-federated-identity")
                    .and_then(|v| v.to_str().ok()),
                Some(format!("Bearer {}", log.assertion).as_str()),
                "{method} {path}"
            );
            if path != "/" {
                assert!(
                    headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.starts_with("Nostr ")),
                    "{method} {path}: Authorization proof"
                );
            }
        }
        assert_eq!(log.mints, 1, "one assertion serves the whole process");
        // Everything but the WS publish (no WS server) succeeds.
        for (i, result) in results.iter().enumerate() {
            if i != 8 {
                assert!(result.is_ok(), "route {i}: {result:?}");
            }
        }
    }

    #[tokio::test]
    async fn exempt_routes_and_relays_without_a_source_get_no_assertion() {
        let (base, log) = origin(None).await;
        let plain = client(&base, false);
        exercise_every_route(&plain).await;
        plain.get_public("/info").await.unwrap();

        {
            let log = log.lock().unwrap();
            assert_eq!(log.mints, 0);
            assert!(log.requests.len() >= PROTECTED.len());
            for (method, path, headers) in &log.requests {
                assert!(
                    headers.get("nostr-federated-identity").is_none(),
                    "{method} {path}"
                );
            }
        }

        // An exempt route never mints or presents evidence, even with a source.
        let (base, log) = origin(None).await;
        client(&base, true).get_public("/info").await.unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log.mints, 0);
        assert!(log.requests[0].2.get("nostr-federated-identity").is_none());
    }

    /// An enforcing relay's 401 with no evidence source names the settings,
    /// on HTTP and on the WebSocket upgrade, and is not retried.
    #[tokio::test]
    async fn a_401_without_a_source_names_the_evidence_settings() {
        let (base, log) = origin(Some(StatusCode::UNAUTHORIZED)).await;
        let plain = client(&base, false);
        let keys = plain.credentials().test_keys();
        let ws_event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        for err in [
            plain.query(&serde_json::json!({})).await.unwrap_err(),
            plain.publish_ephemeral_event(ws_event).await.unwrap_err(),
        ] {
            assert!(
                matches!(&err, CliError::Auth(m)
                    if m.contains("BUZZ_FEDERATED_IDENTITY_TOKEN")
                        && m.contains("BUZZ_FEDERATED_IDENTITY_ADAPTER")),
                "{err:?}"
            );
        }
        assert_eq!(log.lock().unwrap().requests.len(), 2, "401 is not retried");

        // With a source configured, a 401 is the relay's verdict, not a hint.
        let (base, _) = origin(Some(StatusCode::UNAUTHORIZED)).await;
        let err = client(&base, true)
            .query(&serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, CliError::Relay { status: 401, .. }),
            "{err:?}"
        );
    }

    /// The paths a failed mint must stop before anything reaches the relay:
    /// a read, a post-once write, a moderation command, a stored message
    /// (`messages send`) and the WebSocket upgrade (`users set-presence`).
    /// Returns each path's error and how many mints that one call made.
    async fn every_path_once(
        client: &BuzzClient,
        mints: &std::sync::atomic::AtomicUsize,
    ) -> Vec<(&'static str, CliError, usize)> {
        let keys = client.credentials().test_keys();
        let event = |kind: u16| {
            EventBuilder::new(Kind::Custom(kind), "hi")
                .sign_with_keys(&keys)
                .unwrap()
        };
        let body = serde_json::json!({"k": "v"});
        let mut out = Vec::new();
        let mut record = |path, result: Result<String, CliError>, before| {
            out.push((path, result.unwrap_err(), count(mints) - before));
        };
        let before = count(mints);
        record(
            "read",
            client.query(&serde_json::json!({"kinds": [1]})).await,
            before,
        );
        let before = count(mints);
        record(
            "post-once",
            client.post_json_once_authed("/api/post-once", &body).await,
            before,
        );
        let before = count(mints);
        record("moderation", client.submit_event(event(9041)).await, before);
        let before = count(mints);
        record("message", client.submit_event(event(9)).await, before);
        let before = count(mints);
        record(
            "websocket",
            client.publish_ephemeral_event(event(20001)).await,
            before,
        );
        out
    }

    /// An adapter 500 or 503 is retried a bounded number of times on every
    /// path, then fails as a retryable adapter error. Nothing reaches the
    /// relay, so a message is never reported as possibly delivered.
    #[tokio::test]
    async fn adapter_5xx_is_retried_on_every_path_and_never_reaches_the_relay() {
        for response in [test_support::HTTP_500, test_support::HTTP_503] {
            let (relay, log) = origin(None).await;
            let (adapter, mints) = raw_adapter(response).await;
            let client = client_via(&relay, &adapter);
            for (path, err, attempts) in every_path_once(&client, &mints).await {
                assert!(matches!(err, CliError::Adapter(_)), "{path}: {err:?}");
                assert!(crate::error::is_retryable_error(&err), "{path}");
                assert_eq!(attempts, 3, "{path}: three mints, no more");
            }
            assert!(
                log.lock().unwrap().requests.is_empty(),
                "the relay saw a request"
            );
        }
    }

    /// A first send whose outcome is lost, then an assertion refresh that
    /// fails, is still possibly delivered: the adapter error must not invite
    /// an outer re-sign.
    #[tokio::test]
    async fn a_refresh_failure_after_a_lost_send_stays_delivery_unknown() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mints = Arc::new(AtomicUsize::new(0));
        let sends = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new()
            .route(
                "/v1/identity/assertions",
                axum::routing::post({
                    let mints = mints.clone();
                    move |body: axum::body::Bytes| {
                        let mints = mints.clone();
                        async move {
                            if mints.fetch_add(1, Ordering::SeqCst) > 0 {
                                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }
                            let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let pubkey = req["nostr_pubkey"].as_str().unwrap_or_default();
                            let expires_at = nostr::Timestamp::now().as_secs() + 300;
                            axum::Json(serde_json::json!({
                                "assertion": test_support::assertion(pubkey, expires_at),
                                "nostr_pubkey": pubkey,
                                "expires_at": expires_at,
                            }))
                            .into_response()
                        }
                    }
                }),
            )
            .fallback({
                let sends = sends.clone();
                move || {
                    let sends = sends.clone();
                    async move {
                        sends.fetch_add(1, Ordering::SeqCst);
                        // The relay may have stored it; the proxy lost the answer
                        // after the assertion reached its refresh boundary.
                        crate::cached_assertion::test_clock::advance(300);
                        StatusCode::BAD_GATEWAY.into_response()
                    }
                }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = client_via(&base, &base);
        let keys = client.credentials().test_keys();
        let event = EventBuilder::new(Kind::Custom(9), "hi")
            .sign_with_keys(&keys)
            .unwrap();
        let err = client.submit_event(event).await.unwrap_err();
        assert!(matches!(err, CliError::DeliveryUnknown(_)), "{err:?}");
        assert!(!crate::error::is_retryable_error(&err));
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "one send reached the relay"
        );
        assert_eq!(
            mints.load(Ordering::SeqCst),
            4,
            "one mint, then three failed refreshes"
        );
    }

    /// A 403 whose body is cut off is one refusal on every path, not a
    /// retryable network error, and nothing reaches the relay.
    #[tokio::test]
    async fn a_truncated_adapter_403_is_a_single_refusal_on_every_path() {
        let (relay, log) = origin(None).await;
        let (adapter, mints) = raw_adapter(test_support::TRUNCATED_403).await;
        let client = client_via(&relay, &adapter);
        for (path, err, attempts) in every_path_once(&client, &mints).await {
            assert!(matches!(err, CliError::Auth(_)), "{path}: {err:?}");
            assert!(!crate::error::is_retryable_error(&err), "{path}");
            assert_eq!(attempts, 1, "{path}: one mint");
        }
        assert!(
            log.lock().unwrap().requests.is_empty(),
            "the relay saw a request"
        );
    }

    /// A relay redirect to another origin is never followed: the assertion,
    /// proof and auth tag stay with the relay that was asked.
    #[tokio::test]
    async fn a_cross_origin_relay_redirect_is_not_followed() {
        let (target, target_log) = origin(None).await;
        let location = format!("{target}/query");
        let app = axum::Router::new().fallback(move || {
            let location = location.clone();
            async move { (StatusCode::TEMPORARY_REDIRECT, [("location", location)]) }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // Mint from the target origin's adapter so the client has an assertion to leak.
        let client = client_via(&relay, &target);
        let body = serde_json::json!({"k": "v"});
        for err in [
            client.query(&serde_json::json!({})).await.unwrap_err(),
            client.get_authed("/api/get").await.unwrap_err(),
            client
                .post_json_authed("/api/post", &body)
                .await
                .unwrap_err(),
        ] {
            assert!(
                matches!(err, CliError::Relay { status: 307, .. }),
                "{err:?}"
            );
        }
        let log = target_log.lock().unwrap();
        assert_eq!(log.mints, 1);
        assert!(
            log.requests.is_empty(),
            "redirect followed: {:?}",
            covered(&log)
        );
    }

    /// A WebSocket refused after NIP-42 with a policy close (a NIP-FI key
    /// mismatch) is an auth error, exit 3, not a generic failure.
    #[tokio::test]
    async fn a_websocket_policy_close_is_an_auth_error() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::Text(r#"["AUTH","challenge"]"#.into()))
                .await
                .unwrap();
            let _auth = ws.next().await;
            let _ = ws
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Policy,
                    reason: "auth-required: key mismatch".into(),
                })))
                .await;
        });
        let client = client(&relay, false);
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&client.credentials().test_keys())
            .unwrap();
        let err = client.publish_ephemeral_event(event).await.unwrap_err();
        assert!(
            matches!(&err, CliError::Auth(m) if m.contains("key mismatch")),
            "{err:?}"
        );
        assert_eq!(crate::error::exit_code(&err), 3);
        assert!(!crate::error::is_retryable_error(&err));
    }
}

/// A desktop that stays `rate_limited`, or throttles and then holds a
/// signature, through every command path that signs or encrypts.
#[cfg(all(test, unix))]
mod desktop_throttle_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use nostr::{EventBuilder, Kind};

    use super::ws_deadline_tests::ws_relay;
    use super::BuzzClient;
    use crate::desktop_provider::{tests as dp, Provider};
    use crate::error::CliError;

    /// A desktop holding `keys` that answers `rate_limited` to `throttled`.
    async fn client(relay: &str, keys: &nostr::Keys, throttled: &'static str) -> BuzzClient {
        let answer = dp::desktop_answers(keys.clone(), vec![relay.into()]);
        let (endpoint, _) = dp::provider(move |version, request| {
            if request["method"] == throttled {
                return serde_json::json!({"result": "", "error": "rate_limited", "retry_after": 1});
            }
            answer(version, request)
        })
        .await;
        let provider = Provider::open(endpoint, "t".into(), false, relay)
            .await
            .unwrap()
            .ok()
            .unwrap();
        let credentials = crate::credentials::Credentials::from_desktop(provider)
            .await
            .unwrap();
        BuzzClient::new(relay.into(), credentials).unwrap()
    }

    /// A desktop user's 401 and 403 hints name what they can act on in Buzz,
    /// not the headless settings.
    #[tokio::test]
    async fn desktop_users_get_hints_for_the_desktop() {
        let app = axum::Router::new().fallback(|req: axum::extract::Request| async move {
            let status = if req.uri().path() == "/forbidden" {
                403
            } else {
                401
            };
            (
                axum::http::StatusCode::from_u16(status).unwrap(),
                r#"{"error":"no"}"#,
            )
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let keys = nostr::Keys::generate();
        let owner = nostr::Keys::generate();
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &keys.public_key(), "").unwrap();
        let answer = dp::desktop_answers(keys.clone(), vec![relay.clone()]);
        let (endpoint, _) =
            dp::provider(move |version, request| match request["method"].as_str() {
                Some("federated_identity") => serde_json::json!({"result": ""}),
                Some("auth_tag") => serde_json::json!({"result": tag}),
                Some("describe") => {
                    let mut described = answer(version, request);
                    let mut body: serde_json::Value =
                        serde_json::from_str(described["result"].as_str().unwrap()).unwrap();
                    body["methods"]
                        .as_array_mut()
                        .unwrap()
                        .push("auth_tag".into());
                    described["result"] = body.to_string().into();
                    described
                }
                _ => answer(version, request),
            })
            .await;
        let provider = Provider::open(endpoint, "t".into(), false, &relay)
            .await
            .unwrap()
            .ok()
            .unwrap();
        let credentials = crate::credentials::Credentials::from_desktop(provider)
            .await
            .unwrap();
        let client = BuzzClient::new(relay, credentials).unwrap();

        let err = client.get_authed("/api").await.unwrap_err();
        assert!(
            matches!(&err, CliError::Auth(m) if m.contains("in Buzz")),
            "{err:?}"
        );
        let err = client.get_authed("/forbidden").await.unwrap_err();
        let text = err.to_string();
        assert!(
            !text.contains("stale") && !text.contains("BUZZ_AUTH_TAG"),
            "{text}"
        );
    }

    /// A desktop holding `keys` whose assertions live `lifetime` s and
    /// which answers `sign_event` with `sign` (`None`: sign it), logging the
    /// sign-in challenges it signs.
    async fn desktop(
        relay: &str,
        keys: &nostr::Keys,
        assertion: impl Fn() -> String + Send + Sync + 'static,
        sign: impl Fn(&nostr::UnsignedEvent) -> Option<serde_json::Value> + Send + Sync + 'static,
    ) -> (BuzzClient, super::ws_deadline_tests::Log) {
        let signed: super::ws_deadline_tests::Log = Default::default();
        let log = signed.clone();
        let answer = dp::desktop_answers(keys.clone(), vec![relay.into()]);
        let (endpoint, _) =
            dp::provider(move |version, request| match request["method"].as_str() {
                Some("federated_identity") => {
                    serde_json::json!({ "result": assertion() })
                }
                Some("sign_event") => {
                    let unsigned = <nostr::UnsignedEvent as nostr::JsonUtil>::from_json(
                        request["params"][0].as_str().unwrap(),
                    )
                    .unwrap();
                    if let Some(refusal) = sign(&unsigned) {
                        return refusal;
                    }
                    if let Some(c) = unsigned.tags.find(nostr::TagKind::Challenge) {
                        log.lock().unwrap().push(c.content().unwrap().to_owned());
                    }
                    answer(version, request)
                }
                _ => answer(version, request),
            })
            .await;
        let provider = Provider::open(endpoint, "t".into(), false, relay)
            .await
            .unwrap()
            .ok()
            .unwrap();
        let credentials = crate::credentials::Credentials::from_desktop(provider)
            .await
            .unwrap();
        (BuzzClient::new(relay.into(), credentials).unwrap(), signed)
    }

    /// A desktop that throttles the first sign-in signature: the throttle is
    /// waited out between connections, so only the live challenge is signed,
    /// with a fresh assertion on the reconnect after the old one expired.
    ///
    /// The throttle is decided by call count, not by time since the test
    /// started: on a loaded runner the desktop setup alone could outlast a
    /// wall-clock throttle window, leaving nothing to wait out. The relay's
    /// sign-in deadline is long for the same reason; the client drops the
    /// throttled socket itself.
    #[tokio::test]
    async fn a_throttled_sign_in_reconnects_with_fresh_evidence() {
        use super::ws_deadline_tests::deadline_relay;
        let keys = nostr::Keys::generate();
        let (relay, conns, _, bearers) = deadline_relay(std::time::Duration::from_secs(5)).await;
        let signs = AtomicUsize::new(0);
        let (c, signed) = desktop(&relay, &keys, || dp::assertion(40), move |_| {
            if signs.fetch_add(1, Ordering::SeqCst) == 0 {
                crate::cached_assertion::test_clock::advance(60);
                return Some(
                    serde_json::json!({"result": "", "error": "rate_limited", "retry_after": 30}),
                );
            }
            None
        })
        .await;
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        c.publish_ephemeral_event(event).await.unwrap();
        assert_eq!(conns.load(Ordering::SeqCst), 2);
        assert_eq!(
            *signed.lock().unwrap(),
            ["c1"],
            "the dead challenge is never signed"
        );
        let bearers = bearers.lock().unwrap();
        assert_ne!(
            bearers[0], bearers[1],
            "the reconnect brings a fresh assertion"
        );
    }

    /// An HTTP relay logging each request's NIP-FI assertion.
    async fn logging_relay() -> (String, super::ws_deadline_tests::Log) {
        let seen: super::ws_deadline_tests::Log = Default::default();
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| {
            let bearer = headers["nostr-federated-identity"].to_str().unwrap();
            log.lock()
                .unwrap()
                .push(bearer.trim_start_matches("Bearer ").to_owned());
            async { axum::Json(serde_json::json!([])) }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (relay, seen)
    }

    /// A NIP-98 signature that outlives the assertion is signed again after
    /// fresh evidence, and goes out with both current.
    #[tokio::test]
    async fn a_slow_http_signature_goes_out_with_fresh_evidence() {
        let (relay, seen) = logging_relay().await;
        let signs = Arc::new(AtomicUsize::new(0));
        let n = signs.clone();
        let (c, _) = desktop(
            &relay,
            &nostr::Keys::generate(),
            || dp::assertion(40),
            move |_| {
                if n.fetch_add(1, Ordering::SeqCst) == 0 {
                    crate::cached_assertion::test_clock::advance(60);
                }
                None
            },
        )
        .await;
        c.query(&serde_json::json!({})).await.unwrap();
        assert_eq!(signs.load(Ordering::SeqCst), 2, "the proof is signed again");
        let sent = crate::cached_assertion::jws_exp(&seen.lock().unwrap()[0]).unwrap();
        assert!(
            sent > crate::cached_assertion::unix_now(),
            "the assertion sent is current"
        );
    }

    /// A post-sign refresh that waits and returns the same assertion still
    /// gets a proof signed after it.
    #[tokio::test]
    async fn a_slow_refresh_of_the_same_assertion_re_signs_the_proof() {
        let (relay, seen) = logging_relay().await;
        let exp = crate::cached_assertion::unix_now() + 600;
        let fetches = AtomicUsize::new(0);
        let same = move || {
            if fetches.fetch_add(1, Ordering::SeqCst) == 1 {
                // The desktop waits out throttling before it answers.
                crate::cached_assertion::test_clock::advance(45);
            }
            dp::assertion(exp - crate::cached_assertion::unix_now())
        };
        let signs = Arc::new(AtomicUsize::new(0));
        let n = signs.clone();
        let (c, _) = desktop(&relay, &nostr::Keys::generate(), same, move |_| {
            if n.fetch_add(1, Ordering::SeqCst) == 0 {
                crate::cached_assertion::test_clock::advance(310);
            }
            None
        })
        .await;
        c.query(&serde_json::json!({})).await.unwrap();
        assert_eq!(
            signs.load(Ordering::SeqCst),
            2,
            "the aged proof is signed again"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    /// A signature slower than a proof's lifetime is signed again even when
    /// the evidence did not change.
    #[tokio::test]
    async fn a_slow_signature_with_unchanged_evidence_is_signed_again() {
        let (relay, seen) = logging_relay().await;
        let exp = crate::cached_assertion::unix_now() + 3600;
        let signs = Arc::new(AtomicUsize::new(0));
        let n = signs.clone();
        let (c, _) = desktop(
            &relay,
            &nostr::Keys::generate(),
            move || dp::assertion(exp - crate::cached_assertion::unix_now()),
            move |_| {
                if n.fetch_add(1, Ordering::SeqCst) == 0 {
                    crate::cached_assertion::test_clock::advance(65);
                }
                None
            },
        )
        .await;
        c.query(&serde_json::json!({})).await.unwrap();
        assert_eq!(
            signs.load(Ordering::SeqCst),
            2,
            "the stale proof is signed again"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    /// Evidence that keeps expiring while the proof is signed ends retryable
    /// after three rounds, sending nothing.
    #[tokio::test]
    async fn evidence_that_keeps_expiring_ends_retryable() {
        let (relay, seen) = logging_relay().await;
        let signs = Arc::new(AtomicUsize::new(0));
        let n = signs.clone();
        let (c, _) = desktop(
            &relay,
            &nostr::Keys::generate(),
            || dp::assertion(40),
            move |_| {
                n.fetch_add(1, Ordering::SeqCst);
                crate::cached_assertion::test_clock::advance(60);
                None
            },
        )
        .await;
        let err = c.query(&serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, CliError::Unavailable(_)), "{err:?}");
        assert_eq!(signs.load(Ordering::SeqCst), 3);
        assert!(seen.lock().unwrap().is_empty());
    }

    /// An HTTP relay answering every query with no events.
    async fn http_relay() -> String {
        let app = axum::Router::new().fallback(|| async { axum::Json(serde_json::json!([])) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        relay
    }

    fn assert_throttled(err: CliError) {
        assert!(matches!(err, CliError::Throttled(_)), "{err:?}");
        assert!(crate::error::is_retryable_error(&err));
        assert_eq!(crate::error::exit_code(&err), 2);
    }

    #[tokio::test]
    async fn persistent_throttling_is_retryable_on_every_signing_path() {
        let keys = nostr::Keys::generate();
        let relay = http_relay().await;
        let c = client(&relay, &keys, "sign_event").await;

        // NIP-98 on an ordinary request.
        assert_throttled(c.query(&serde_json::json!({})).await.unwrap_err());
        // Event signing.
        assert_throttled(
            c.sign_event(EventBuilder::new(Kind::TextNote, "x"))
                .await
                .unwrap_err(),
        );

        // Memory encryption.
        let c = client(&relay, &keys, "nip44_encrypt").await;
        let owner = nostr::Keys::generate().public_key().to_hex();
        assert_throttled(
            crate::commands::mem::cmd_set(&c, "mem/a", "v", Some(&owner), false)
                .await
                .unwrap_err(),
        );
    }

    #[tokio::test]
    async fn persistent_throttling_during_nip42_is_retryable() {
        let keys = nostr::Keys::generate();
        let (relay, auths, events) = ws_relay().await;
        let c = client(&relay, &keys, "sign_event").await;
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        assert_throttled(c.publish_ephemeral_event(event).await.unwrap_err());
        assert_eq!(auths.load(Ordering::SeqCst), 0);
        assert_eq!(events.load(Ordering::SeqCst), 0);
    }

    /// NIP-42 signing that is throttled, then held for a decision, completes
    /// the publish: provider time is outside the relay's deadlines, and the
    /// relay sees one AUTH and one EVENT.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_throttled_then_held_nip42_signature_completes_without_replay() {
        let keys = nostr::Keys::generate();
        let (relay, auths, events) = ws_relay().await;
        let signs = Arc::new(AtomicUsize::new(0));
        let counted = signs.clone();
        let answer = dp::desktop_answers(keys.clone(), vec![relay.clone()]);
        let (endpoint, _) = dp::provider(move |version, request| {
            if request["method"] == "sign_event" {
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    return serde_json::json!({"result": "", "error": "rate_limited", "retry_after": 30});
                }
                // Held for a person's decision.
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            answer(version, request)
        })
        .await;
        let provider = Provider::open(endpoint, "t".into(), false, &relay)
            .await
            .unwrap()
            .ok()
            .unwrap();
        let credentials = crate::credentials::Credentials::from_desktop(provider)
            .await
            .unwrap();
        let c = BuzzClient::new(relay, credentials).unwrap();
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        c.publish_ephemeral_event(event).await.unwrap();
        assert_eq!(signs.load(Ordering::SeqCst), 2);
        assert_eq!(auths.load(Ordering::SeqCst), 1);
        assert_eq!(events.load(Ordering::SeqCst), 1);
    }
}

/// WebSocket publish deadlines, on a paused clock: a signer may take as long
/// as it needs, while each network phase, writes included, is capped.
#[cfg(test)]
mod ws_deadline_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use nostr::{Event, EventBuilder, Keys, Kind, NostrSigner, PublicKey, UnsignedEvent};
    use tokio_tungstenite::tungstenite::Message;

    use super::BuzzClient;
    use crate::credentials::Credentials;
    use crate::error::CliError;

    /// A WebSocket relay that sends a challenge, then OKs AUTH and one event.
    /// Returns its URL and the count of AUTH and EVENT frames it received.
    pub(super) async fn ws_relay() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let (auths, events) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (a, e) = (auths.clone(), events.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (a, e) = (a.clone(), e.clone());
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let _ = ws
                        .send(Message::Text(r#"["AUTH","challenge"]"#.into()))
                        .await;
                    while let Some(Ok(Message::Text(text))) = ws.next().await {
                        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                        let counter = if frame[0] == "AUTH" { &a } else { &e };
                        counter.fetch_add(1, Ordering::SeqCst);
                        let ok = serde_json::json!(["OK", frame[1]["id"], true, ""]);
                        let _ = ws.send(Message::Text(ok.to_string().into())).await;
                    }
                });
            }
        });
        (relay, auths, events)
    }

    /// Keys that take `delay` to sign, like a signer held for a decision.
    /// The clock is paused only while signing (after the real connect), so
    /// the delay passes at once unless an earlier timer fires first.
    #[derive(Debug)]
    struct Slow(Keys, Duration);

    /// Keys whose first signature takes `delay`, then sign at once.
    #[derive(Debug)]
    struct SlowFirst(Slow, std::sync::atomic::AtomicBool);

    impl NostrSigner for Slow {
        fn backend(&self) -> nostr::signer::SignerBackend<'_> {
            nostr::signer::SignerBackend::Custom("slow".into())
        }
        fn get_public_key(
            &self,
        ) -> nostr::util::BoxedFuture<'_, Result<PublicKey, nostr::signer::SignerError>> {
            self.0.get_public_key()
        }
        fn sign_event(
            &self,
            unsigned: UnsignedEvent,
        ) -> nostr::util::BoxedFuture<'_, Result<Event, nostr::signer::SignerError>> {
            Box::pin(async move {
                tokio::time::pause();
                tokio::time::sleep(self.1).await;
                tokio::time::resume();
                self.0.sign_event(unsigned).await
            })
        }
        fn nip04_encrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip04_encrypt(p, c)
        }
        fn nip04_decrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip04_decrypt(p, c)
        }
        fn nip44_encrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip44_encrypt(p, c)
        }
        fn nip44_decrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip44_decrypt(p, c)
        }
    }

    impl NostrSigner for SlowFirst {
        fn backend(&self) -> nostr::signer::SignerBackend<'_> {
            self.0.backend()
        }
        fn get_public_key(
            &self,
        ) -> nostr::util::BoxedFuture<'_, Result<PublicKey, nostr::signer::SignerError>> {
            self.0.get_public_key()
        }
        fn sign_event(
            &self,
            unsigned: UnsignedEvent,
        ) -> nostr::util::BoxedFuture<'_, Result<Event, nostr::signer::SignerError>> {
            if self.1.swap(false, Ordering::SeqCst) {
                self.0.sign_event(unsigned)
            } else {
                self.0 .0.sign_event(unsigned)
            }
        }
        fn nip04_encrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip04_encrypt(p, c)
        }
        fn nip04_decrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip04_decrypt(p, c)
        }
        fn nip44_encrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip44_encrypt(p, c)
        }
        fn nip44_decrypt<'a>(
            &'a self,
            p: &'a PublicKey,
            c: &'a str,
        ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>> {
            self.0.nip44_decrypt(p, c)
        }
    }

    /// A relay that, like buzz-relay, drops a socket not signed in within
    /// 5 s; otherwise as [`ws_relay`]. Returns its URL, its connection
    /// count, and the challenges each AUTH answered.
    /// A relay closing sockets not signed in within `deadline`: the
    /// connection count, the challenges answered and each upgrade's NIP-FI
    /// header.
    #[allow(clippy::result_large_err)] // tungstenite's handshake callback type
    pub(super) async fn deadline_relay(deadline: Duration) -> (String, Arc<AtomicUsize>, Log, Log) {
        let conns = Arc::new(AtomicUsize::new(0));
        let (answered, bearers): (Log, Log) = Default::default();
        let (n, log, seen) = (conns.clone(), answered.clone(), bearers.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let challenge = format!("c{}", n.fetch_add(1, Ordering::SeqCst));
                let (log, seen) = (log.clone(), seen.clone());
                tokio::spawn(async move {
                    let record =
                        |req: &tokio_tungstenite::tungstenite::handshake::server::Request, res| {
                            let bearer = req.headers().get("nostr-federated-identity");
                            let bearer = bearer.map(|v| v.to_str().unwrap().to_owned());
                            seen.lock().unwrap().push(bearer.unwrap_or_default());
                            Ok(res)
                        };
                    let mut ws = tokio_tungstenite::accept_hdr_async(stream, record)
                        .await
                        .unwrap();
                    let hello = serde_json::json!(["AUTH", challenge]).to_string();
                    let _ = ws.send(Message::Text(hello.into())).await;
                    let deadline = tokio::time::sleep(deadline);
                    tokio::pin!(deadline);
                    let mut authed = false;
                    loop {
                        let next = tokio::select! {
                            _ = &mut deadline, if !authed => return,
                            next = ws.next() => next,
                        };
                        let Some(Ok(Message::Text(text))) = next else {
                            return;
                        };
                        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                        if frame[0] == "AUTH" {
                            let tags = frame[1]["tags"].as_array().unwrap();
                            let c = tags.iter().find(|t| t[0] == "challenge").unwrap();
                            log.lock().unwrap().push(c[1].as_str().unwrap().to_owned());
                            authed = true;
                        }
                        let ok = serde_json::json!(["OK", frame[1]["id"], true, ""]);
                        let _ = ws.send(Message::Text(ok.to_string().into())).await;
                    }
                });
            }
        });
        (relay, conns, answered, bearers)
    }

    pub(super) type Log = Arc<std::sync::Mutex<Vec<String>>>;

    /// Every upgrade names the CLI and its version.
    #[tokio::test]
    async fn upgrade_headers_carry_the_cli_user_agent() {
        let keys = Keys::generate();
        let c = BuzzClient::new(
            "http://127.0.0.1:1".into(),
            Credentials::from_signer(Arc::new(keys.clone()), keys.public_key()),
        )
        .unwrap();
        let headers = c.upgrade_headers().await.unwrap();
        assert_eq!(headers.get("user-agent").unwrap(), super::USER_AGENT);
        assert!(super::USER_AGENT.starts_with("buzz-cli/"));
    }

    /// A sign-in signature that comes back after the relay's 5 s deadline
    /// reconnects and signs the new challenge, never the old one.
    #[tokio::test]
    async fn a_late_sign_in_reconnects_and_signs_the_new_challenge() {
        let keys = Keys::generate();
        let (relay, conns, answered, _) = deadline_relay(Duration::from_secs(5)).await;
        let signer = Arc::new(SlowFirst(
            Slow(keys.clone(), Duration::from_secs(8)),
            std::sync::atomic::AtomicBool::new(true),
        ));
        let c =
            BuzzClient::new(relay, Credentials::from_signer(signer, keys.public_key())).unwrap();
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        c.publish_ephemeral_event(event).await.unwrap();
        assert_eq!(conns.load(Ordering::SeqCst), 2);
        assert_eq!(
            *answered.lock().unwrap(),
            ["c1"],
            "only the new challenge is signed"
        );
    }

    /// A signer that is always too slow ends retryable after the bound.
    #[tokio::test]
    async fn a_sign_in_always_too_late_ends_retryable() {
        let keys = Keys::generate();
        let (relay, conns, ..) = deadline_relay(Duration::from_secs(5)).await;
        let c = client(relay, &keys, Duration::from_secs(8));
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        let err = c.publish_ephemeral_event(event).await.unwrap_err();
        assert!(crate::error::is_retryable_error(&err), "{err:?}");
        assert_eq!(
            conns.load(Ordering::SeqCst),
            super::WS_SIGN_IN_ATTEMPTS as usize
        );
    }

    fn client(relay: String, keys: &Keys, delay: Duration) -> BuzzClient {
        let signer = Arc::new(Slow(keys.clone(), delay));
        BuzzClient::new(relay, Credentials::from_signer(signer, keys.public_key())).unwrap()
    }

    /// Signing held for 300 s, past the old 225 s single cap, still publishes,
    /// once.
    #[tokio::test]
    async fn a_signature_held_past_every_relay_cap_still_publishes_once() {
        let keys = Keys::generate();
        let (relay, auths, events) = ws_relay().await;
        let c = client(relay, &keys, Duration::from_secs(300));
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        let started = tokio::time::Instant::now();
        c.publish_ephemeral_event(event).await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(300));
        assert!(
            started.elapsed() < Duration::from_secs(330),
            "real time stays short"
        );
        assert_eq!(auths.load(Ordering::SeqCst), 1);
        assert_eq!(events.load(Ordering::SeqCst), 1);
    }

    /// A relay that OKs AUTH and then stops reading stalls the EVENT write;
    /// the publish fails within its phase cap and is not sent again.
    #[tokio::test]
    async fn a_stalled_event_write_fails_within_its_phase_cap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = format!("http://{}", listener.local_addr().unwrap());
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        // The relay pauses the clock once it has OKed AUTH, so the EVENT
        // phase runs on virtual time from that mark.
        let authed = Arc::new(std::sync::Mutex::new(None));
        let mark = authed.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let mark = mark.clone();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    let _ = ws
                        .send(Message::Text(r#"["AUTH","challenge"]"#.into()))
                        .await;
                    if let Some(Ok(Message::Text(text))) = ws.next().await {
                        let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                        let ok = serde_json::json!(["OK", frame[1]["id"], true, ""]);
                        let _ = ws.send(Message::Text(ok.to_string().into())).await;
                        tokio::time::pause();
                        *mark.lock().unwrap() = Some(tokio::time::Instant::now());
                    }
                    // Hold the socket open without reading.
                    std::future::pending::<()>().await;
                    drop(ws);
                });
            }
        });
        let keys = Keys::generate();
        let c = BuzzClient::new(relay, Credentials::from_keys(keys.clone())).unwrap();
        let event = EventBuilder::new(Kind::Custom(20001), "x".repeat(32 * 1024 * 1024))
            .sign_with_keys(&keys)
            .unwrap();
        // An unbounded write would wait here for the hour, not forever.
        let err = tokio::time::timeout(Duration::from_secs(3600), c.publish_ephemeral_event(event))
            .await
            .expect("the stalled write is bounded")
            .unwrap_err();
        let stalled = authed.lock().unwrap().expect("AUTH was OKed").elapsed();
        // From the OK the client has at most the rest of the AUTH phase (the
        // paused clock jumps to its read deadline before the OK is read) and
        // the whole EVENT phase.
        use buzz_ws_client::connection::{AUTH_OK_TIMEOUT_SECS, PUBLISH_OK_TIMEOUT_SECS};
        let cap = Duration::from_secs(AUTH_OK_TIMEOUT_SECS + 1 + PUBLISH_OK_TIMEOUT_SECS + 1);
        assert!(stalled <= cap, "{stalled:?} {err:?}");
        assert!(
            matches!(err, CliError::Other(ref m) if m.contains("Timeout")),
            "{err:?}"
        );
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }

    /// The NIP-42 AUTH event names the configured community URL, path and
    /// trailing slash kept, as the relay checks it.
    #[tokio::test]
    async fn the_auth_event_names_the_relay_with_its_trailing_slash() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let named = Arc::new(std::sync::Mutex::new(None::<String>));
        let log = named.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = ws
                .send(Message::Text(r#"["AUTH","challenge"]"#.into()))
                .await;
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame[0] == "AUTH" {
                    let relay = frame[1]["tags"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|t| t[0] == "relay")
                        .map(|t| t[1].as_str().unwrap().to_owned());
                    *log.lock().unwrap() = relay;
                }
                let ok = serde_json::json!(["OK", frame[1]["id"], true, ""]);
                let _ = ws.send(Message::Text(ok.to_string().into())).await;
            }
        });
        let keys = Keys::generate();
        let c = BuzzClient::new(
            format!("http://{addr}/team/"),
            Credentials::from_keys(keys.clone()),
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(20001), "")
            .sign_with_keys(&keys)
            .unwrap();
        c.publish_ephemeral_event(event).await.unwrap();
        assert_eq!(
            named.lock().unwrap().as_deref(),
            Some(format!("ws://{addr}/team/").as_str())
        );
    }

    #[test]
    fn auth_keeps_the_configured_relay_path() {
        let client = BuzzClient::new(
            "wss://host.example/team/".into(),
            Credentials::from_keys(Keys::generate()),
        )
        .unwrap();
        assert_eq!(client.ws_url, "wss://host.example/team/");
        assert_eq!(client.relay_url, "https://host.example/team");
    }
}
