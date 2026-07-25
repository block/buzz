//! Shared authenticated HTTP and WebSocket transport for Buzz relays.
//!
//! This crate owns relay URL normalization, client credentials, request
//! authentication, and transport errors. Feature-specific filters, event
//! builders, output formatting, and user-interface state belong to consumers.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use std::collections::HashSet;
use std::time::Duration;

use base64::engine::general_purpose::{
    STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD,
};
use base64::Engine;
use nostr::{Event, EventBuilder, JsonUtil, Keys, PublicKey, Tag};
use sha2::{Digest, Sha256};
use url::Url;

/// Typed message emitted by an authenticated relay subscription.
pub use buzz_ws_client::RelayMessage;

/// Retry settings used by relay transport operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum number of attempts, including the initial request.
    pub max_attempts: u32,
    /// Defensive ceiling for a relay-provided retry delay.
    pub max_retry_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            max_retry_delay: Duration::from_secs(30),
        }
    }
}

/// Configuration for a [`BuzzClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuzzClientConfig {
    /// Relay URL in HTTP(S) or WebSocket form.
    pub relay_url: String,
    /// Maximum time allowed to establish an HTTP connection.
    pub connect_timeout: Duration,
    /// Maximum total time allowed for one HTTP request.
    pub request_timeout: Duration,
    /// Retry settings for transient transport failures.
    pub retry_policy: RetryPolicy,
}

impl BuzzClientConfig {
    /// Creates a configuration with the standard CLI-compatible timeouts.
    pub fn new(relay_url: impl Into<String>) -> Self {
        Self {
            relay_url: relay_url.into(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(30),
            retry_policy: RetryPolicy::default(),
        }
    }
}

/// Parsed signing credentials and optional NIP-OA owner attestation.
#[derive(Clone)]
pub struct BuzzIdentity {
    keys: Keys,
    auth_tag: Option<Tag>,
    auth_tag_json: Option<String>,
}

impl BuzzIdentity {
    /// Parses a private key and validates an optional NIP-OA auth tag.
    pub fn parse(private_key: &str, auth_tag_json: Option<&str>) -> Result<Self, ClientError> {
        let keys =
            Keys::parse(private_key).map_err(|error| ClientError::InvalidKey(error.to_string()))?;
        Self::from_keys(keys, auth_tag_json)
    }

    /// Builds an identity from parsed keys and validates an optional NIP-OA auth tag.
    pub fn from_keys(keys: Keys, auth_tag_json: Option<&str>) -> Result<Self, ClientError> {
        let auth_tag_json = auth_tag_json
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let auth_tag = auth_tag_json
            .map(|json| {
                let tag = buzz_sdk::nip_oa::parse_auth_tag(json)
                    .map_err(|error| ClientError::InvalidAuthTag(error.to_string()))?;
                buzz_sdk::nip_oa::verify_auth_tag(json, &keys.public_key())
                    .map_err(|error| ClientError::InvalidAuthTag(error.to_string()))?;
                Ok::<_, ClientError>(tag)
            })
            .transpose()?;

        Ok(Self {
            keys,
            auth_tag,
            auth_tag_json: auth_tag_json.map(str::to_owned),
        })
    }

    /// Returns the public key for this identity.
    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }
}

/// Errors produced by shared Buzz client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The relay URL was malformed or used an unsupported scheme.
    #[error("invalid relay URL: {0}")]
    InvalidUrl(String),
    /// The private key could not be parsed.
    #[error("invalid private key: {0}")]
    InvalidKey(String),
    /// The NIP-OA auth tag was malformed or did not verify for the identity.
    #[error("invalid auth tag: {0}")]
    InvalidAuthTag(String),
    /// Media input, metadata, or a media path failed local validation.
    #[error("invalid media: {0}")]
    InvalidMedia(String),
    /// An event or HTTP authorization event could not be signed.
    #[error("signing failed: {0}")]
    Signing(String),
    /// An HTTP request failed at the network layer.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    /// A WebSocket transport operation failed.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] buzz_ws_client::WsClientError),
    /// The relay returned a non-successful HTTP response.
    #[error("relay returned HTTP {status}: {message}")]
    Relay {
        /// HTTP status code.
        status: u16,
        /// Relay-provided message, or the raw response when no message was available.
        message: String,
        /// Relay-provided retry delay, when present and valid.
        retry_after: Option<Duration>,
    },
    /// The relay explicitly rejected a submitted event.
    #[error("relay rejected event {event_id}: {message}")]
    Rejected {
        /// Hex event identifier.
        event_id: String,
        /// Relay rejection message.
        message: String,
    },
    /// A successful response did not match the expected relay protocol shape.
    #[error("relay protocol error: {0}")]
    Protocol(String),
    /// JSON serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// An operation exceeded its configured deadline.
    #[error("operation timed out")]
    Timeout,
    /// A non-idempotent event may have reached the relay before delivery failed.
    #[error("delivery of event {event_id} is unknown: {reason}")]
    DeliveryUnknown {
        /// Hex event identifier.
        event_id: String,
        /// Description of the ambiguous transport failure.
        reason: String,
    },
}

/// Relay acknowledgement for a submitted event.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubmitEventResponse {
    /// Hex identifier of the submitted event.
    pub event_id: String,
    /// Whether the relay accepted the event.
    pub accepted: bool,
    /// Relay-provided acknowledgement or rejection message.
    pub message: String,
}

/// Descriptor returned by a successful Blossom upload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlobDescriptor {
    /// Public URL of the uploaded blob.
    pub url: String,
    /// Hex-encoded SHA-256 of the file content.
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
    /// MIME type reported by the relay.
    #[serde(rename = "type")]
    pub mime_type: String,
    /// Unix timestamp when the file was uploaded.
    pub uploaded: i64,
    /// Optional image dimensions formatted as `<width>x<height>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<String>,
    /// Optional blurhash placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blurhash: Option<String>,
    /// Optional thumbnail URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// Optional audio or video duration in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// Bytes and response metadata returned by an authenticated media download.
#[derive(Debug, Clone)]
pub struct DownloadedMedia {
    /// Resolved, same-relay media URL.
    pub url: String,
    /// Downloaded content.
    pub bytes: bytes::Bytes,
    /// Response MIME type when supplied by the relay.
    pub mime_type: Option<String>,
    /// Response content length when supplied by the relay.
    pub content_length: Option<u64>,
}

/// Authenticated client for a single Buzz relay.
#[derive(Clone)]
pub struct BuzzClient {
    http: reqwest::Client,
    relay_http_url: String,
    relay_ws_url: String,
    identity: BuzzIdentity,
    retry_policy: RetryPolicy,
}

impl BuzzClient {
    /// Creates a client with a newly constructed HTTP transport.
    pub fn new(config: BuzzClientConfig, identity: BuzzIdentity) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .map_err(ClientError::Network)?;
        Self::with_http_client(config, identity, http)
    }

    fn with_http_client(
        config: BuzzClientConfig,
        identity: BuzzIdentity,
        http: reqwest::Client,
    ) -> Result<Self, ClientError> {
        let (relay_http_url, relay_ws_url) = normalize_relay_urls(&config.relay_url)?;
        Ok(Self {
            http,
            relay_http_url,
            relay_ws_url,
            identity,
            retry_policy: config.retry_policy,
        })
    }

    /// Returns the client's signing public key.
    pub fn public_key(&self) -> PublicKey {
        self.identity.public_key()
    }

    /// Returns the identity keys for feature-specific encryption operations.
    ///
    /// Generic signing should use [`BuzzClient::sign_event`], which enforces
    /// the configured NIP-OA auth-tag invariant.
    pub fn keys(&self) -> &Keys {
        &self.identity.keys
    }

    /// Returns the validated NIP-OA auth tag configured for this identity.
    pub fn auth_tag(&self) -> Option<&Tag> {
        self.identity.auth_tag.as_ref()
    }

    /// Returns the normalized HTTP relay base URL.
    pub fn relay_http_url(&self) -> &str {
        &self.relay_http_url
    }

    /// Returns the normalized WebSocket relay base URL.
    pub fn relay_ws_url(&self) -> &str {
        &self.relay_ws_url
    }

    /// Signs an event after enforcing exactly one configured NIP-OA auth tag.
    pub fn sign_event(&self, builder: EventBuilder) -> Result<Event, ClientError> {
        let builder = match &self.identity.auth_tag {
            Some(tag) => builder.tags([tag.clone()]),
            None => builder,
        };
        let event = builder
            .sign_with_keys(&self.identity.keys)
            .map_err(|error| ClientError::Signing(error.to_string()))?;
        let auth_count = event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
            .count();
        let expected = usize::from(self.identity.auth_tag.is_some());
        if auth_count != expected {
            return Err(ClientError::Signing(format!(
                "event has {auth_count} auth tags; expected {expected}"
            )));
        }
        Ok(event)
    }

    /// Signs an event without injecting this identity's NIP-OA auth tag.
    ///
    /// This exceptional path is for protocols whose event carries a distinct
    /// content-level `auth` tag, such as NIP-IA identity archive commands.
    /// Normal events must use [`BuzzClient::sign_event`].
    pub fn sign_event_with_content_auth(
        &self,
        builder: EventBuilder,
    ) -> Result<Event, ClientError> {
        builder
            .sign_with_keys(&self.identity.keys)
            .map_err(|error| ClientError::Signing(error.to_string()))
    }

    fn sign_nip98(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<String, ClientError> {
        let mut tags = vec![
            parse_tag(["u", url])?,
            parse_tag(["method", method])?,
            parse_tag(["nonce", &uuid::Uuid::new_v4().to_string()])?,
        ];
        if let Some(body) = body {
            let payload_hash = hex::encode(Sha256::digest(body));
            tags.push(parse_tag(["payload", &payload_hash])?);
        }
        let event = EventBuilder::new(nostr::Kind::Custom(27235), "")
            .tags(tags)
            .sign_with_keys(&self.identity.keys)
            .map_err(|error| ClientError::Signing(error.to_string()))?;
        Ok(format!(
            "Nostr {}",
            BASE64_STANDARD.encode(event.as_json().as_bytes())
        ))
    }

    fn authenticated_request(
        &self,
        request: reqwest::RequestBuilder,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<reqwest::RequestBuilder, ClientError> {
        let request = request.header("Authorization", self.sign_nip98(method, url, body)?);
        Ok(match &self.identity.auth_tag_json {
            Some(auth_tag_json) => request.header("x-auth-tag", auth_tag_json),
            None => request,
        })
    }

    /// Queries the relay and returns raw JSON event values.
    ///
    /// Multiple filters are OR-ed according to Nostr REQ semantics.
    pub async fn query_values(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, ClientError> {
        let body = serde_json::to_vec(filters)?;
        let response = self.post_json_with_retry("/query", &body).await?;
        let value: serde_json::Value = serde_json::from_slice(&response)?;
        value
            .as_array()
            .cloned()
            .ok_or_else(|| ClientError::Protocol("query response must be a JSON array".to_string()))
    }

    /// Counts events matching one or more OR-ed Nostr filters.
    pub async fn count(&self, filters: &[serde_json::Value]) -> Result<u64, ClientError> {
        let body = serde_json::to_vec(filters)?;
        let response = self.post_json_with_retry("/count", &body).await?;
        let value: serde_json::Value = serde_json::from_slice(&response)?;
        value
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ClientError::Protocol(
                    "count response must contain an unsigned integer `count`".to_string(),
                )
            })
    }

    /// Fetches an unauthenticated public relay endpoint such as NIP-11 `/info`.
    pub async fn get_public(&self, path: &str) -> Result<String, ClientError> {
        self.get_with_retry(path, false).await
    }

    /// Fetches a NIP-98-authenticated relay endpoint.
    pub async fn get_authenticated(&self, path: &str) -> Result<String, ClientError> {
        self.get_with_retry(path, true).await
    }

    /// Posts a JSON value to a NIP-98-authenticated relay endpoint.
    ///
    /// Feature-specific request and response models remain the caller's
    /// responsibility; this method owns authentication, retries, and JSON
    /// transport.
    pub async fn post_json_value(
        &self,
        path: &str,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        validate_root_relative_path(path)?;
        let body = serde_json::to_vec(value)?;
        let response = self.post_json_with_retry(path, &body).await?;
        serde_json::from_slice(&response).map_err(Into::into)
    }

    /// Submits a signed stored event through the relay's HTTP bridge.
    ///
    /// Non-idempotent moderation commands use a stricter retry policy and
    /// return [`ClientError::DeliveryUnknown`] whenever relay execution may
    /// have occurred without a trustworthy acknowledgement.
    pub async fn submit_event(&self, event: Event) -> Result<SubmitEventResponse, ClientError> {
        if is_non_idempotent_kind(event.kind.as_u16()) {
            self.submit_non_idempotent_event(event).await
        } else {
            self.submit_stored_event(event).await
        }
    }

    /// Publishes a signed ephemeral event through an authenticated WebSocket.
    pub async fn publish_ephemeral(
        &self,
        event: Event,
    ) -> Result<SubmitEventResponse, ClientError> {
        const PUBLISH_TIMEOUT_SECS: u64 = 75;

        let event_id = event.id.to_hex();
        let acknowledgement = buzz_ws_client::publish_event(
            &self.relay_ws_url,
            event,
            &self.identity.keys,
            self.identity.auth_tag.as_ref(),
            PUBLISH_TIMEOUT_SECS,
        )
        .await
        .map_err(map_websocket_error)?;
        if !acknowledgement.accepted {
            return Err(ClientError::Rejected {
                event_id,
                message: acknowledgement.message,
            });
        }
        Ok(SubmitEventResponse {
            event_id: acknowledgement.event_id,
            accepted: true,
            message: acknowledgement.message,
        })
    }

    /// Opens an authenticated Nostr subscription for one or more filters.
    pub async fn subscribe(
        &self,
        subscription_id: &str,
        filters: &[nostr::Filter],
    ) -> Result<RelaySubscription, ClientError> {
        const CONNECT_AND_AUTH_TIMEOUT: Duration = Duration::from_secs(45);

        if subscription_id.is_empty() {
            return Err(ClientError::Protocol(
                "subscription ID must not be empty".to_string(),
            ));
        }
        let mut connection = tokio::time::timeout(
            CONNECT_AND_AUTH_TIMEOUT,
            buzz_ws_client::NostrWsConnection::connect_authenticated(
                &self.relay_ws_url,
                &self.identity.keys,
                self.identity.auth_tag.as_ref(),
            ),
        )
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(map_websocket_error)?;
        connection
            .send_raw(&subscription_request(subscription_id, filters))
            .await
            .map_err(map_websocket_error)?;
        Ok(RelaySubscription {
            connection,
            subscription_id: subscription_id.to_string(),
        })
    }

    /// Uploads bytes with authenticated Blossom transport.
    ///
    /// The shared transport accepts any syntactically valid MIME type. Product
    /// allowlists, file selection, and attachment size policy remain with the
    /// caller. Uploads use a bounded timeout selected from the MIME family.
    pub async fn upload_bytes(
        &self,
        bytes: Vec<u8>,
        mime_type: &str,
    ) -> Result<BlobDescriptor, ClientError> {
        validate_mime_type(mime_type)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let body = bytes::Bytes::from(bytes);
        let expected_size = body.len() as u64;
        let primary = self
            .upload_to_endpoint("/upload", body.clone(), mime_type, &sha256)
            .await;
        let descriptor = match primary {
            Ok(descriptor) => descriptor,
            Err(ClientError::Relay {
                status: 404 | 405, ..
            }) => {
                self.upload_to_endpoint("/media/upload", body.clone(), mime_type, &sha256)
                    .await?
            }
            Err(error) => return Err(error),
        };
        if descriptor.sha256 != sha256 || descriptor.size != expected_size {
            return Err(ClientError::Protocol(format!(
                "upload descriptor does not match content hash or size (expected {sha256}, {expected_size} bytes)"
            )));
        }
        Ok(descriptor)
    }

    /// Downloads a same-relay Blossom blob using authenticated `get` transport.
    ///
    /// `input` may be a full same-origin `/media/` URL or a safe
    /// `sha256[.ext]` media identifier. Redirects are disabled so credentials
    /// cannot be forwarded to another origin.
    pub async fn download_media(&self, input: &str) -> Result<DownloadedMedia, ClientError> {
        let url = media_url_from_input(&self.relay_http_url, input)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ClientError::Network)?;
        let attempts = self.retry_policy.max_attempts.max(1);

        for attempt in 0..attempts {
            let auth = self.sign_blossom_get(&url)?;
            let request = client.get(&url).header("Authorization", auth);
            let request = match &self.identity.auth_tag_json {
                Some(auth_tag_json) => request.header("x-auth-tag", auth_tag_json),
                None => request,
            };
            let result = async {
                let response = request.send().await.map_err(map_network_error)?;
                let status = response.status();
                let mime_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let content_length = response.content_length();
                let response_body = response.bytes().await.map_err(map_network_error)?;
                if !status.is_success() {
                    return Err(relay_error(
                        status,
                        &response_body,
                        self.retry_policy.max_retry_delay,
                    ));
                }
                Ok(DownloadedMedia {
                    url: url.clone(),
                    bytes: response_body,
                    mime_type,
                    content_length,
                })
            }
            .await;

            match result {
                Ok(download) => return Ok(download),
                Err(error) if attempt + 1 < attempts && is_transient(&error) => {
                    tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                }
                Err(error) => return Err(error),
            }
        }

        Err(ClientError::Protocol(
            "retry loop ended without an attempt".to_string(),
        ))
    }

    /// Queries stored events across the relay's composite pagination cursor.
    ///
    /// `limit = None` reads until the relay returns a short page. A zero limit
    /// returns immediately without issuing a request.
    pub async fn query_paginated(
        &self,
        mut filter: serde_json::Value,
        limit: Option<u32>,
    ) -> Result<Vec<serde_json::Value>, ClientError> {
        const PAGE_SIZE: usize = 500;

        if limit == Some(0) {
            return Ok(Vec::new());
        }
        if !filter.is_object() {
            return Err(ClientError::Protocol(
                "paginated query filter must be a JSON object".to_string(),
            ));
        }

        let mut events = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut previous_cursor: Option<(u64, String)> = None;

        while limit.is_none_or(|limit| events.len() < limit as usize) {
            let page_limit = limit
                .map(|limit| (limit as usize - events.len()).min(PAGE_SIZE))
                .unwrap_or(PAGE_SIZE);
            filter["limit"] = serde_json::json!(page_limit);

            let page = self.query_values(std::slice::from_ref(&filter)).await?;
            if page.len() > page_limit {
                return Err(ClientError::Protocol(format!(
                    "query returned {} events for page limit {page_limit}",
                    page.len()
                )));
            }
            if page.is_empty() {
                break;
            }

            let cursor = query_cursor(&page)?;
            if previous_cursor.as_ref() == Some(&cursor) {
                return Err(ClientError::Protocol(
                    "query pagination cursor did not advance".to_string(),
                ));
            }
            previous_cursor = Some(cursor.clone());

            for event in &page {
                let id = event_id(event)?;
                if seen_ids.insert(id.to_string()) {
                    events.push(event.clone());
                }
            }

            if page.len() < page_limit {
                break;
            }
            filter["until"] = serde_json::json!(cursor.0);
            filter["before_id"] = serde_json::json!(cursor.1);
        }

        if let Some(limit) = limit {
            events.truncate(limit as usize);
        }
        Ok(events)
    }

    async fn post_json_with_retry(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
        let attempts = self.retry_policy.max_attempts.max(1);
        for attempt in 0..attempts {
            let result = self.post_json_once(path, body).await;
            match result {
                Ok(response) => return Ok(response),
                Err(error) if attempt + 1 < attempts && is_transient(&error) => {
                    tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(ClientError::Protocol(
            "retry loop ended without an attempt".to_string(),
        ))
    }

    async fn get_with_retry(&self, path: &str, authenticated: bool) -> Result<String, ClientError> {
        validate_root_relative_path(path)?;
        let url = format!("{}{path}", self.relay_http_url);
        let attempts = self.retry_policy.max_attempts.max(1);

        for attempt in 0..attempts {
            let request = self.http.get(&url);
            let request = if authenticated {
                self.authenticated_request(request, "GET", &url, None)?
            } else {
                request.header("Accept", "application/nostr+json")
            };
            let result = async {
                let response = request.send().await.map_err(map_network_error)?;
                let status = response.status();
                let response_body = response.bytes().await.map_err(map_network_error)?;
                if !status.is_success() {
                    return Err(relay_error(
                        status,
                        &response_body,
                        self.retry_policy.max_retry_delay,
                    ));
                }
                Ok(String::from_utf8_lossy(&response_body).into_owned())
            }
            .await;

            match result {
                Ok(body) => return Ok(body),
                Err(error) if attempt + 1 < attempts && is_transient(&error) => {
                    tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                }
                Err(error) => return Err(error),
            }
        }

        Err(ClientError::Protocol(
            "retry loop ended without an attempt".to_string(),
        ))
    }

    async fn post_json_once(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
        let url = format!("{}{path}", self.relay_http_url);
        let request = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_vec());
        let response = self
            .authenticated_request(request, "POST", &url, Some(body))?
            .send()
            .await
            .map_err(map_network_error)?;
        let status = response.status();
        let response_body = response.bytes().await.map_err(map_network_error)?;
        if status.is_success() {
            return Ok(response_body.to_vec());
        }

        let raw = String::from_utf8_lossy(&response_body).into_owned();
        let message = extract_relay_message(&raw).unwrap_or_else(|| raw.clone());
        let retry_after = (status == reqwest::StatusCode::TOO_MANY_REQUESTS)
            .then(|| parse_retry_hint_text(&message))
            .flatten()
            .map(|seconds| Duration::from_secs(seconds).min(self.retry_policy.max_retry_delay));
        Err(ClientError::Relay {
            status: status.as_u16(),
            message,
            retry_after,
        })
    }

    async fn submit_stored_event(&self, event: Event) -> Result<SubmitEventResponse, ClientError> {
        let event_id = event.id.to_hex();
        let body = serde_json::to_vec(&event)?;
        let response = match self.post_json_with_retry("/events", &body).await {
            Ok(response) => response,
            Err(error) if delivery_may_be_ambiguous(&error) => {
                return Err(ClientError::DeliveryUnknown {
                    event_id,
                    reason: error.to_string(),
                });
            }
            Err(error) => return Err(error),
        };
        parse_submit_response(&response).map_err(|error| match error {
            ClientError::Rejected { .. } => error,
            error => ClientError::DeliveryUnknown {
                event_id,
                reason: format!("relay acknowledgement was malformed: {error}"),
            },
        })
    }

    async fn submit_non_idempotent_event(
        &self,
        event: Event,
    ) -> Result<SubmitEventResponse, ClientError> {
        let event_id = event.id.to_hex();
        let body = serde_json::to_vec(&event)?;
        let url = format!("{}/events", self.relay_http_url);
        let attempts = self.retry_policy.max_attempts.max(1);

        for attempt in 0..attempts {
            let request = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone());
            let response = match self
                .authenticated_request(request, "POST", &url, Some(&body))?
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) if error.is_connect() => {
                    let error = ClientError::Network(error);
                    if attempt + 1 < attempts {
                        tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => {
                    return Err(ClientError::DeliveryUnknown {
                        event_id,
                        reason: error.to_string(),
                    });
                }
            };
            let status = response.status();
            let response_body =
                response
                    .bytes()
                    .await
                    .map_err(|error| ClientError::DeliveryUnknown {
                        event_id: event_id.clone(),
                        reason: format!("response body transfer failed: {error}"),
                    })?;

            if !status.is_success() {
                let error = relay_error(status, &response_body, self.retry_policy.max_retry_delay);
                let canonical_rate_limit = matches!(
                    &error,
                    ClientError::Relay {
                        status: 429,
                        message,
                        ..
                    } if message.starts_with("rate-limited:")
                );
                if canonical_rate_limit {
                    if attempt + 1 < attempts {
                        tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                        continue;
                    }
                    return Err(error);
                }
                if matches!(
                    &error,
                    ClientError::Relay {
                        status: 429 | 502..=504,
                        ..
                    }
                ) {
                    return Err(ClientError::DeliveryUnknown {
                        event_id,
                        reason: error.to_string(),
                    });
                }
                return Err(error);
            }

            return parse_submit_response(&response_body).map_err(|error| match error {
                ClientError::Rejected { .. } => error,
                error => ClientError::DeliveryUnknown {
                    event_id,
                    reason: format!("relay acknowledgement was malformed: {error}"),
                },
            });
        }

        Err(ClientError::Protocol(
            "retry loop ended without an attempt".to_string(),
        ))
    }

    async fn upload_to_endpoint(
        &self,
        path: &str,
        body: bytes::Bytes,
        mime_type: &str,
        sha256: &str,
    ) -> Result<BlobDescriptor, ClientError> {
        let url = format!("{}{path}", self.relay_http_url);
        let attempts = self.retry_policy.max_attempts.max(1);

        for attempt in 0..attempts {
            let auth = self.sign_blossom_upload(sha256, mime_type)?;
            let request = self
                .http
                .put(&url)
                .timeout(upload_timeout(mime_type))
                .header("Authorization", auth)
                .header("Content-Type", mime_type)
                .header("X-SHA-256", sha256)
                .body(body.clone());
            let request = match &self.identity.auth_tag_json {
                Some(auth_tag_json) => request.header("x-auth-tag", auth_tag_json),
                None => request,
            };
            let result = async {
                let response = request.send().await.map_err(map_network_error)?;
                let status = response.status();
                let response_body = response.bytes().await.map_err(map_network_error)?;
                if !status.is_success() {
                    return Err(relay_error(
                        status,
                        &response_body,
                        self.retry_policy.max_retry_delay,
                    ));
                }
                serde_json::from_slice(&response_body).map_err(ClientError::Serialization)
            }
            .await;

            match result {
                Ok(descriptor) => return Ok(descriptor),
                Err(error) if attempt + 1 < attempts && is_transient(&error) => {
                    tokio::time::sleep(self.retry_delay(attempt, &error)).await;
                }
                Err(error) => return Err(error),
            }
        }

        Err(ClientError::Protocol(
            "retry loop ended without an attempt".to_string(),
        ))
    }

    fn sign_blossom_upload(&self, sha256: &str, mime_type: &str) -> Result<String, ClientError> {
        let expiration = nostr::Timestamp::now().as_secs()
            + if mime_type.starts_with("video/") {
                3_600
            } else {
                600
            };
        let mut tags = vec![
            parse_tag(["t", "upload"])?,
            parse_tag(["x", sha256])?,
            parse_tag(["expiration", &expiration.to_string()])?,
        ];
        if let Some(server) = relay_server_tag(&self.relay_http_url) {
            tags.push(parse_tag(["server", &server])?);
        }
        blossom_auth_header(&self.identity.keys, "Upload file", tags)
    }

    fn sign_blossom_get(&self, media_url: &str) -> Result<String, ClientError> {
        let expiration = nostr::Timestamp::now().as_secs() + 600;
        let server = relay_server_tag(media_url).ok_or_else(|| {
            ClientError::InvalidMedia(format!("media URL has no relay authority: {media_url}"))
        })?;
        blossom_auth_header(
            &self.identity.keys,
            "Get media",
            vec![
                parse_tag(["t", "get"])?,
                parse_tag(["expiration", &expiration.to_string()])?,
                parse_tag(["server", &server])?,
            ],
        )
    }

    fn retry_delay(&self, attempt: u32, error: &ClientError) -> Duration {
        if let ClientError::Relay {
            retry_after: Some(retry_after),
            ..
        } = error
        {
            return *retry_after;
        }

        let multiplier = 3_u32.saturating_pow(attempt);
        let ceiling = Duration::from_millis(500)
            .saturating_mul(multiplier)
            .min(self.retry_policy.max_retry_delay);
        ceiling.mul_f64(rand::random::<f64>())
    }
}

/// Normalizes an HTTP(S) or WebSocket relay URL to its WebSocket form.
///
/// The result applies the same validation and loopback normalization used by
/// [`BuzzClient`] construction.
pub fn normalize_relay_ws_url(relay_url: &str) -> Result<String, ClientError> {
    normalize_relay_urls(relay_url).map(|(_, ws_url)| ws_url)
}

fn normalize_relay_urls(relay_url: &str) -> Result<(String, String), ClientError> {
    let relay_url = relay_url.trim();
    if relay_url.is_empty() {
        return Err(ClientError::InvalidUrl("URL is empty".to_string()));
    }
    let mut parsed =
        Url::parse(relay_url).map_err(|error| ClientError::InvalidUrl(error.to_string()))?;
    if parsed.cannot_be_a_base() || parsed.host_str().is_none() {
        return Err(ClientError::InvalidUrl(
            "URL must include a relay host".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ClientError::InvalidUrl(
            "URL must not include user credentials".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ClientError::InvalidUrl(
            "URL must not include a query or fragment".to_string(),
        ));
    }

    let http_scheme = match parsed.scheme() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        scheme => {
            return Err(ClientError::InvalidUrl(format!(
                "unsupported scheme {scheme:?}"
            )))
        }
    };
    parsed
        .set_scheme(http_scheme)
        .map_err(|()| ClientError::InvalidUrl("could not normalize URL scheme".to_string()))?;
    if matches!(parsed.host_str(), Some("localhost" | "::1" | "[::1]")) {
        parsed
            .set_host(Some("127.0.0.1"))
            .map_err(|error| ClientError::InvalidUrl(error.to_string()))?;
    }

    let http_url = parsed.as_str().trim_end_matches('/').to_string();
    let ws_scheme = if http_scheme == "https" { "wss" } else { "ws" };
    parsed
        .set_scheme(ws_scheme)
        .map_err(|()| ClientError::InvalidUrl("could not normalize URL scheme".to_string()))?;
    let ws_url = parsed.as_str().trim_end_matches('/').to_string();
    Ok((http_url, ws_url))
}

fn parse_tag<const N: usize>(values: [&str; N]) -> Result<Tag, ClientError> {
    Tag::parse(values).map_err(|error| ClientError::Signing(error.to_string()))
}

fn map_network_error(error: reqwest::Error) -> ClientError {
    if error.is_timeout() && !error.is_connect() {
        ClientError::Timeout
    } else {
        ClientError::Network(error)
    }
}

fn is_transient(error: &ClientError) -> bool {
    match error {
        ClientError::Timeout => true,
        ClientError::Network(error) => {
            error.is_connect()
                || error.is_request()
                || error.is_body()
                || error.is_decode()
                || error.is_timeout()
        }
        ClientError::Relay { status, .. } => matches!(status, 429 | 502..=504),
        _ => false,
    }
}

fn extract_relay_message(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
}

fn parse_retry_hint_text(text: &str) -> Option<u64> {
    const PREFIX: &str = "retry in ";
    let after_prefix = text
        .find(PREFIX)
        .map(|index| &text[index + PREFIX.len()..])?;
    let digit_count = after_prefix.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 || after_prefix.as_bytes().get(digit_count) != Some(&b's') {
        return None;
    }
    after_prefix[..digit_count].parse().ok()
}

fn query_cursor(page: &[serde_json::Value]) -> Result<(u64, String), ClientError> {
    let event = page
        .last()
        .ok_or_else(|| ClientError::Protocol("query page was empty".to_string()))?;
    let created_at = event
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            ClientError::Protocol("query event is missing a valid `created_at`".to_string())
        })?;
    Ok((created_at, event_id(event)?.to_string()))
}

fn event_id(event: &serde_json::Value) -> Result<&str, ClientError> {
    event
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| ClientError::Protocol("query event is missing a valid `id`".to_string()))
}

fn is_non_idempotent_kind(kind: u16) -> bool {
    matches!(kind, 9040..=9044)
}

fn delivery_may_be_ambiguous(error: &ClientError) -> bool {
    match error {
        ClientError::Timeout => true,
        ClientError::Network(error) => !error.is_connect(),
        ClientError::Relay {
            status: 429,
            message,
            ..
        } => !message.starts_with("rate-limited:"),
        ClientError::Relay {
            status: 502..=504, ..
        } => true,
        _ => false,
    }
}

fn relay_error(
    status: reqwest::StatusCode,
    response_body: &[u8],
    max_retry_delay: Duration,
) -> ClientError {
    let raw = String::from_utf8_lossy(response_body).into_owned();
    let message = extract_relay_message(&raw).unwrap_or_else(|| raw.clone());
    let retry_after = (status == reqwest::StatusCode::TOO_MANY_REQUESTS)
        .then(|| parse_retry_hint_text(&message))
        .flatten()
        .map(|seconds| Duration::from_secs(seconds).min(max_retry_delay));
    ClientError::Relay {
        status: status.as_u16(),
        message,
        retry_after,
    }
}

fn parse_submit_response(body: &[u8]) -> Result<SubmitEventResponse, ClientError> {
    let response: SubmitEventResponse = serde_json::from_slice(body)?;
    if !response.accepted {
        return Err(ClientError::Rejected {
            event_id: response.event_id,
            message: response.message,
        });
    }
    Ok(response)
}

fn validate_mime_type(mime_type: &str) -> Result<(), ClientError> {
    if mime_type.trim().is_empty() || mime_type != mime_type.trim() || !mime_type.contains('/') {
        return Err(ClientError::InvalidMedia(format!(
            "invalid MIME type {mime_type:?}"
        )));
    }
    reqwest::header::HeaderValue::from_str(mime_type)
        .map(|_| ())
        .map_err(|error| ClientError::InvalidMedia(format!("invalid MIME type: {error}")))
}

fn validate_root_relative_path(path: &str) -> Result<(), ClientError> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains("://") {
        return Err(ClientError::Protocol(
            "relay path must be root-relative".to_string(),
        ));
    }
    Ok(())
}

fn upload_timeout(mime_type: &str) -> Duration {
    if mime_type.starts_with("video/") {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(120)
    }
}

fn relay_server_tag(relay_url: &str) -> Option<String> {
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    (!authority.is_empty()).then_some(authority)
}

fn blossom_auth_header(keys: &Keys, content: &str, tags: Vec<Tag>) -> Result<String, ClientError> {
    let event = EventBuilder::new(nostr::Kind::Custom(24242), content)
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|error| ClientError::Signing(error.to_string()))?;
    Ok(format!(
        "Nostr {}",
        BASE64_URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
    ))
}

fn media_url_from_input(relay_url: &str, input: &str) -> Result<String, ClientError> {
    let input = input.trim();
    if input.starts_with("http://") || input.starts_with("https://") {
        let media = Url::parse(input)
            .map_err(|error| ClientError::InvalidMedia(format!("invalid media URL: {error}")))?;
        let relay =
            Url::parse(relay_url).map_err(|error| ClientError::InvalidUrl(error.to_string()))?;
        if media.query().is_some() || media.fragment().is_some() {
            return Err(ClientError::InvalidMedia(
                "media URL must not include a query or fragment".to_string(),
            ));
        }
        let segment = media.path().strip_prefix("/media/").ok_or_else(|| {
            ClientError::InvalidMedia("media URL must use a /media/ path".to_string())
        })?;
        if !is_safe_media_path_segment(segment) {
            return Err(ClientError::InvalidMedia(
                "media path must be sha256, sha256.ext, or sha256.thumb.jpg".to_string(),
            ));
        }
        if media.scheme() != relay.scheme()
            || media.host_str() != relay.host_str()
            || media.port_or_known_default() != relay.port_or_known_default()
        {
            return Err(ClientError::InvalidMedia(
                "refusing to authenticate a non-relay media origin".to_string(),
            ));
        }
        return Ok(input.to_string());
    }
    if input.contains("://") {
        return Err(ClientError::InvalidMedia(
            "media URL must use HTTP or HTTPS".to_string(),
        ));
    }

    let segment = input.trim_start_matches("/media/");
    if !is_safe_media_path_segment(segment) {
        return Err(ClientError::InvalidMedia(
            "media input must be sha256, sha256.ext, or sha256.thumb.jpg".to_string(),
        ));
    }
    Ok(format!("{relay_url}/media/{segment}"))
}

fn is_safe_media_path_segment(segment: &str) -> bool {
    let parts: Vec<&str> = segment.split('.').collect();
    match parts.as_slice() {
        [hash] => is_lower_hex_sha256(hash),
        [hash, extension] => is_lower_hex_sha256(hash) && is_safe_media_extension(extension),
        [hash, "thumb", "jpg"] => is_lower_hex_sha256(hash),
        _ => false,
    }
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_safe_media_extension(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn subscription_request(subscription_id: &str, filters: &[nostr::Filter]) -> serde_json::Value {
    let mut request = Vec::with_capacity(filters.len() + 2);
    request.push(serde_json::json!("REQ"));
    request.push(serde_json::json!(subscription_id));
    request.extend(filters.iter().map(|filter| serde_json::json!(filter)));
    serde_json::Value::Array(request)
}

fn map_websocket_error(error: buzz_ws_client::WsClientError) -> ClientError {
    if matches!(error, buzz_ws_client::WsClientError::Timeout) {
        ClientError::Timeout
    } else {
        ClientError::WebSocket(error)
    }
}

/// Active authenticated relay subscription.
///
/// Dropping the value closes its socket. Call [`RelaySubscription::cancel`]
/// when the relay should also receive an explicit Nostr `CLOSE` request.
pub struct RelaySubscription {
    connection: buzz_ws_client::NostrWsConnection,
    subscription_id: String,
}

impl RelaySubscription {
    /// Returns the Nostr subscription identifier.
    pub fn id(&self) -> &str {
        &self.subscription_id
    }

    /// Waits for the next typed relay message up to `timeout`.
    pub async fn next_event(&mut self, timeout: Duration) -> Result<RelayMessage, ClientError> {
        self.connection
            .next_event(timeout)
            .await
            .map_err(map_websocket_error)
    }

    /// Sends a Nostr `CLOSE` request and disconnects with a bounded deadline.
    pub async fn cancel(mut self) -> Result<(), ClientError> {
        const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

        self.connection
            .send_raw(&serde_json::json!(["CLOSE", self.subscription_id]))
            .await
            .map_err(map_websocket_error)?;
        tokio::time::timeout(DISCONNECT_TIMEOUT, self.connection.disconnect())
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(map_websocket_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_with_auth() -> (BuzzIdentity, String) {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let auth_json = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "kind=9")
            .expect("test auth tag should be valid");
        let identity = BuzzIdentity::from_keys(agent, Some(&auth_json))
            .expect("test identity should be valid");
        (identity, auth_json)
    }

    fn test_client(identity: BuzzIdentity) -> BuzzClient {
        BuzzClient::new(BuzzClientConfig::new("wss://relay.example/"), identity)
            .expect("test client should be valid")
    }

    #[test]
    fn normalizes_http_and_websocket_urls() {
        let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
        let client = test_client(identity);
        assert_eq!(client.relay_http_url(), "https://relay.example");
        assert_eq!(client.relay_ws_url(), "wss://relay.example");

        let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
        let client =
            BuzzClient::new(BuzzClientConfig::new("http://relay.example/"), identity).unwrap();
        assert_eq!(client.relay_http_url(), "http://relay.example");
        assert_eq!(client.relay_ws_url(), "ws://relay.example");
    }

    #[test]
    fn normalizes_loopback_hosts_to_ipv4() {
        for relay_url in ["http://localhost:3000/", "ws://[::1]:3000"] {
            let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
            let client = BuzzClient::new(BuzzClientConfig::new(relay_url), identity).unwrap();
            assert_eq!(client.relay_http_url(), "http://127.0.0.1:3000");
            assert_eq!(client.relay_ws_url(), "ws://127.0.0.1:3000");
        }
    }

    #[test]
    fn rejects_malformed_or_unsupported_urls() {
        for relay_url in [
            "",
            "not a URL",
            "ftp://relay.example",
            "https://",
            "https://user@relay.example",
            "https://relay.example?tenant=other",
        ] {
            let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
            assert!(
                BuzzClient::new(BuzzClientConfig::new(relay_url), identity).is_err(),
                "{relay_url:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_invalid_keys_and_auth_tags() {
        assert!(matches!(
            BuzzIdentity::parse("not-a-private-key", None),
            Err(ClientError::InvalidKey(_))
        ));
        assert!(matches!(
            BuzzIdentity::from_keys(Keys::generate(), Some("not-json")),
            Err(ClientError::InvalidAuthTag(_))
        ));

        let owner = Keys::generate();
        let agent = Keys::generate();
        let other_agent = Keys::generate();
        let auth_json =
            buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
        assert!(matches!(
            BuzzIdentity::from_keys(other_agent, Some(&auth_json)),
            Err(ClientError::InvalidAuthTag(_))
        ));
    }

    #[test]
    fn sign_event_enforces_auth_tag_count() {
        let (identity, _) = identity_with_auth();
        let client = test_client(identity);
        let event = client
            .sign_event(EventBuilder::text_note("hello"))
            .expect("configured auth tag should be injected");
        assert_eq!(
            event
                .tags
                .iter()
                .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
                .count(),
            1
        );

        let duplicate = Tag::parse(["auth", &"a".repeat(64), "", &"b".repeat(128)]).unwrap();
        assert!(matches!(
            client.sign_event(EventBuilder::text_note("hello").tags([duplicate])),
            Err(ClientError::Signing(_))
        ));

        let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
        let client = test_client(identity);
        assert!(client
            .sign_event(
                EventBuilder::text_note("hello").tags([Tag::parse([
                    "auth",
                    &"a".repeat(64),
                    "",
                    &"b".repeat(128)
                ])
                .unwrap()])
            )
            .is_err());
    }

    #[test]
    fn nip98_contains_method_url_payload_hash_and_unique_nonce() {
        let identity = BuzzIdentity::from_keys(Keys::generate(), None).unwrap();
        let client = test_client(identity);
        let body = br#"{"kinds":[9]}"#;
        let first = decode_nip98(
            &client
                .sign_nip98("POST", "https://relay.example/query", Some(body))
                .unwrap(),
        );
        let second = decode_nip98(
            &client
                .sign_nip98("POST", "https://relay.example/query", Some(body))
                .unwrap(),
        );
        first.verify().unwrap();
        assert_eq!(first.kind, nostr::Kind::Custom(27235));
        assert_eq!(tag_value(&first, "u"), Some("https://relay.example/query"));
        assert_eq!(tag_value(&first, "method"), Some("POST"));
        let expected_payload = hex::encode(Sha256::digest(body));
        assert_eq!(
            tag_value(&first, "payload"),
            Some(expected_payload.as_str())
        );
        assert_ne!(tag_value(&first, "nonce"), tag_value(&second, "nonce"));
    }

    #[test]
    fn authenticated_request_forwards_original_auth_tag_json() {
        let (identity, auth_json) = identity_with_auth();
        let client = test_client(identity);
        let request = client
            .authenticated_request(
                client.http.post("https://relay.example/query"),
                "POST",
                "https://relay.example/query",
                Some(b"[]"),
            )
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request
                .headers()
                .get("x-auth-tag")
                .and_then(|value| value.to_str().ok()),
            Some(auth_json.as_str())
        );
        assert!(request.headers().contains_key("Authorization"));
    }

    #[test]
    fn media_upload_timeouts_are_bounded_by_mime_family() {
        assert_eq!(upload_timeout("image/png"), Duration::from_secs(120));
        assert_eq!(upload_timeout("application/pdf"), Duration::from_secs(120));
        assert_eq!(upload_timeout("video/mp4"), Duration::from_secs(600));
    }

    fn decode_nip98(header: &str) -> Event {
        let encoded = header
            .strip_prefix("Nostr ")
            .expect("NIP-98 header should use Nostr scheme");
        let json = BASE64_STANDARD
            .decode(encoded)
            .expect("NIP-98 payload should be base64");
        Event::from_json(json).expect("NIP-98 payload should be an event")
    }

    fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
        event.tags.iter().find_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some(name))
                .then(|| values.get(1).map(String::as_str))
                .flatten()
        })
    }
}
