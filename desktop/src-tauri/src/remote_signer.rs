//! Captured bbidentity event and purpose-specific crypto HTTP adapters. It never publishes events or accesses local keys.

use std::{fmt, time::Duration};

use nostr::{
    signer::SignerBackend, util::BoxedFuture, Event, NostrSigner, PublicKey, SignerError,
    UnsignedEvent,
};
use reqwest::{header::HeaderValue, Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::signer_config::{parse_api_base, SignerConfig};

mod capabilities;

const SESSION_HEADER: &str = "X-BB-Session-Credential";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Read the native build settings. This does not activate a signer or log in.
pub fn build_signer_config() -> Result<SignerConfig, &'static str> {
    SignerConfig::parse(
        option_env!("BUZZ_DESKTOP_BUILD_SIGNER_MODE"),
        option_env!("BUZZ_DESKTOP_BUILD_SIGNER_API_BASE").filter(|base| !base.is_empty()),
    )
}

/// Safe, body-free errors: credentials, request content, and server diagnostics
/// are deliberately excluded from both Display and Debug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSignerError {
    /// Invalid API base, credential header, or HTTP client configuration.
    Configuration,
    /// Session is no longer authenticated or lacks the required capability.
    Authentication(u16),
    /// Server-side HTTP failure; the caller may choose to retry.
    TransientStatus(u16),
    /// Other HTTP failure, including redirects (which are never followed).
    HttpStatus(u16),
    /// Network/TLS/response stream failure.
    Transport,
    /// The bounded HTTP deadline expired.
    Timeout,
    /// The response cannot be decoded as the required capability shape.
    MalformedResponse,
    /// Response exceeded the capability-specific streamed body bound.
    ResponseTooLarge,
    /// The request or response does not match the pinned author/template.
    EventMismatch,
    /// Nostr ID or BIP340 signature verification failed.
    InvalidSignature,
    /// Input violates the capability protocol bounds.
    InvalidInput,
    /// Capability response does not match the requested binding.
    CapabilityMismatch,
    /// The API cannot perform the requested cryptographic operation.
    UnsupportedCapability,
}

impl fmt::Display for RemoteSignerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("remote signer: invalid capability input"),
            Self::CapabilityMismatch => f.write_str("remote signer: capability response mismatch"),
            Self::Configuration => f.write_str("remote signer: invalid configuration"),
            Self::Authentication(code) => {
                write!(f, "remote signer: authentication failed (HTTP {code})")
            }
            Self::TransientStatus(code) => write!(f, "remote signer: transient HTTP {code}"),
            Self::HttpStatus(code) => write!(f, "remote signer: HTTP {code}"),
            Self::Transport => f.write_str("remote signer: transport failure"),
            Self::Timeout => f.write_str("remote signer: request timed out"),
            Self::MalformedResponse => f.write_str("remote signer: malformed response"),
            Self::ResponseTooLarge => f.write_str("remote signer: oversized response"),
            Self::EventMismatch => {
                f.write_str("remote signer: event does not match pinned author or requested fields")
            }
            Self::InvalidSignature => f.write_str("remote signer: invalid event ID or signature"),
            Self::UnsupportedCapability => f.write_str("remote signer: unsupported capability"),
        }
    }
}

impl std::error::Error for RemoteSignerError {}

pub(crate) fn transport_error(error: reqwest::Error) -> RemoteSignerError {
    if error.is_timeout() {
        RemoteSignerError::Timeout
    } else {
        RemoteSignerError::Transport
    }
}

/// Immutable authenticated-session binding. The caller must obtain the opaque
/// bbidentity credential through login/exchange + /auth/me and authenticate the
/// expected remote public key. It need not (and must not implicitly) be a local key.
/// A replacement login must create a new binding; old signers never read session state.
pub struct RemoteSignerSession {
    credential: HeaderValue,
    public_key: PublicKey,
}

impl RemoteSignerSession {
    /// Capture an opaque session credential, never an Auth0 bearer token or nsec.
    /// This validates header syntax, not server-side authentication or expiry.
    pub fn new(
        credential: &str,
        expected_public_key: PublicKey,
    ) -> Result<Self, RemoteSignerError> {
        if credential.is_empty() || credential.trim() != credential {
            return Err(RemoteSignerError::Configuration);
        }
        let mut credential =
            HeaderValue::from_str(credential).map_err(|_| RemoteSignerError::Configuration)?;
        credential.set_sensitive(true);
        Ok(Self {
            credential,
            public_key: expected_public_key,
        })
    }
}

impl fmt::Debug for RemoteSignerSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteSignerSession")
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

/// An owned signing and crypto capability pinned to one API origin, session, and author.
/// Dropping an in-flight signing future releases the request (no detached task,
/// retry, local-key fallback, session refresh, or event publication).
pub struct RemoteSigner {
    client: Client,
    endpoint: Url,
    session: RemoteSignerSession,
    validity: Option<crate::builderlab::session::SessionValidity>,
}

impl fmt::Debug for RemoteSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteSigner")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl RemoteSigner {
    /// Construct against the complete deployment API base. For example,
    /// `https://host/cash-app/goose` or `https://host/api/goose`.
    /// A dedicated native client prevents redirects from forwarding the credential.
    pub fn new(api_base: &str, session: RemoteSignerSession) -> Result<Self, RemoteSignerError> {
        let mut endpoint =
            parse_api_base(api_base).map_err(|_| RemoteSignerError::Configuration)?;
        endpoint.set_path(&format!(
            "{}/v1/buzz/identity/sign",
            endpoint.path().trim_end_matches('/')
        ));
        let client = bounded_client(&endpoint)?;
        Ok(Self {
            client,
            endpoint,
            session,
            validity: None,
        })
    }

    /// Sign and centrally validate the exact requested event. Typed errors remain
    /// available here; rust-nostr's SignerError exposes their safe Display text.
    pub(crate) fn with_validity(
        mut self,
        validity: crate::builderlab::session::SessionValidity,
    ) -> Self {
        self.validity = Some(validity);
        self
    }

    /// Sign and verify the exact event under this captured session.
    pub async fn sign(&self, unsigned: UnsignedEvent) -> Result<Event, RemoteSignerError> {
        self.run(self.sign_inner(unsigned)).await
    }

    async fn run<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, RemoteSignerError>>,
    ) -> Result<T, RemoteSignerError> {
        let Some(validity) = &self.validity else {
            return work.await;
        };
        validity
            .check()
            .map_err(|_| RemoteSignerError::Authentication(401))?;
        let result = tokio::select! { biased;
            _ = validity.canceled() => Err(RemoteSignerError::Authentication(401)),
            result = work => result,
        };
        validity
            .check()
            .map_err(|_| RemoteSignerError::Authentication(401))?;
        if matches!(result, Err(RemoteSignerError::Authentication(_))) {
            validity.cancel.cancel();
        }
        result
    }

    async fn sign_inner(&self, unsigned: UnsignedEvent) -> Result<Event, RemoteSignerError> {
        if unsigned.pubkey != self.session.public_key {
            return Err(RemoteSignerError::EventMismatch);
        }
        unsigned
            .verify_id()
            .map_err(|_| RemoteSignerError::EventMismatch)?;
        #[derive(Serialize)]
        struct Request<'a> {
            created_at: u64,
            kind: u16,
            tags: &'a nostr::Tags,
            content: &'a str,
        }
        let body = serde_json::to_vec(&Request {
            created_at: unsigned.created_at.as_secs(),
            kind: unsigned.kind.as_u16(),
            tags: &unsigned.tags,
            content: &unsigned.content,
        })
        .map_err(|_| RemoteSignerError::Configuration)?;
        // The response echoes the template plus fixed-size hex ID/author/signature.
        // Allow worst-case JSON escaping (six bytes per input byte) and envelope
        // overhead, without introducing a new event-size/kind/timestamp policy.
        let limit = body.len().saturating_mul(6).saturating_add(4096);
        let bytes = self.post(self.endpoint.clone(), body, limit).await?;
        // Parse numeric fields strictly: rust-nostr's Kind deserializer casts
        // u64 to u16, which would otherwise accept a different wire-level kind.
        #[derive(Deserialize)]
        struct Response {
            id: nostr::EventId,
            pubkey: PublicKey,
            created_at: u64,
            kind: u16,
            tags: nostr::Tags,
            content: String,
            sig: nostr::secp256k1::schnorr::Signature,
        }
        let response: Response =
            serde_json::from_slice(&bytes).map_err(|_| RemoteSignerError::MalformedResponse)?;
        let event = Event::new(
            response.id,
            response.pubkey,
            nostr::Timestamp::from(response.created_at),
            nostr::Kind::from(response.kind),
            response.tags,
            response.content,
            response.sig,
        );
        if event.pubkey != self.session.public_key
            || event.created_at != unsigned.created_at
            || event.kind != unsigned.kind
            || event.tags != unsigned.tags
            || event.content != unsigned.content
        {
            return Err(RemoteSignerError::EventMismatch);
        }
        event
            .verify()
            .map_err(|_| RemoteSignerError::InvalidSignature)?;
        Ok(event)
    }
    async fn post(
        &self,
        endpoint: Url,
        body: Vec<u8>,
        limit: usize,
    ) -> Result<Vec<u8>, RemoteSignerError> {
        let response = self
            .client
            .post(endpoint)
            .header(SESSION_HEADER, self.session.credential.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        bounded_body(response, limit).await
    }
}

/// Dedicated credential transport: no redirects/retries and a whole-request bound.
pub(crate) fn bounded_client(base: &url::Url) -> Result<Client, RemoteSignerError> {
    Client::builder()
        .default_headers(staging_routing_headers(base))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(REQUEST_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| RemoteSignerError::Configuration)
}

/// Temporary personal Playpen routing, scoped to the exact staging API. Never
/// attach it to a relay or a different deployment; build-time routing can follow.
fn staging_routing_headers(base: &url::Url) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    if base.scheme() == "https"
        && base.host_str() == Some("test.blockstaging.build")
        && base.port_or_known_default() == Some(443)
        && (base.path() == "/api/goose" || base.path().starts_with("/api/goose/"))
    {
        headers.insert(
            "baggage",
            reqwest::header::HeaderValue::from_static("kgoose-builderbot-playpen=baxen"),
        );
    }
    headers
}

#[cfg(test)]
#[test]
fn playpen_routing_is_scoped_to_exact_staging_origin_and_api_path() {
    for (url, expected) in [
        ("https://test.blockstaging.build/api/goose", true),
        (
            "https://test.blockstaging.build/api/goose/v1/buzz/identity/sign",
            true,
        ),
        ("https://test.blockstaging.build/api/goose-other", false),
        ("https://test.blockstaging.build/relay", false),
        ("https://test.blockstaging.build:444/api/goose", false),
        ("https://prod.block.build/api/goose", false),
        ("http://127.0.0.1/api/goose", false),
    ] {
        assert_eq!(
            staging_routing_headers(&url::Url::parse(url).unwrap()).contains_key("baggage"),
            expected,
            "{url}"
        );
    }
}

/// Body-free status classification and bounded streamed collection for auth/crypto.
pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, RemoteSignerError> {
    let status = response.status();
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(RemoteSignerError::Authentication(status.as_u16()));
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(RemoteSignerError::TransientStatus(status.as_u16()));
    }
    if !status.is_success() {
        return Err(RemoteSignerError::HttpStatus(status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|len| len > limit as u64)
    {
        return Err(RemoteSignerError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(RemoteSignerError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl NostrSigner for RemoteSigner {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("bbidentity-http".into())
    }

    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async { Ok(self.session.public_key) })
    }

    fn sign_event(&self, unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async move { self.sign(unsigned).await.map_err(SignerError::backend) })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async {
            Err(SignerError::backend(
                RemoteSignerError::UnsupportedCapability,
            ))
        })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async {
            Err(SignerError::backend(
                RemoteSignerError::UnsupportedCapability,
            ))
        })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        peer: &'a PublicKey,
        plaintext: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.encrypt(peer, plaintext)
                .await
                .map_err(SignerError::backend)
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        peer: &'a PublicKey,
        ciphertext: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.decrypt(peer, ciphertext)
                .await
                .map_err(SignerError::backend)
        })
    }
}

#[cfg(test)]
#[path = "remote_signer_tests.rs"]
mod tests;
