//! Managed identity transport. Enrollment is separate from exact-event signing.
//!
//! The login owner supplies an authorization snapshot; this module never stores
//! refresh credentials, generates keys, constructs events, or publishes to a relay.
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use nostr::{Event, PublicKey, UnsignedEvent};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::event_signer::EventSigner;

/// Server-selected public identity and its community, pinned for a login lifetime.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedIdentity {
    /// Custodied Nostr public key; never a client-selected request field.
    pub pubkey: String,
    /// Community WebSocket origin.
    pub relay_ws_url: String,
    /// Community HTTPS origin.
    pub relay_http_url: String,
}

impl ManagedIdentity {
    /// Validate the identity before installing it in a login/session owner.
    pub fn validate(&self) -> Result<PublicKey, String> {
        let relay = relay_https_origin(&self.relay_http_url)?;
        if relay.path() != "/"
            || self.relay_ws_url
                != format!(
                    "wss://{}",
                    relay[url::Position::BeforeHost..url::Position::AfterPort].to_owned()
                )
            || self.pubkey.len() != 64
            || !self
                .pubkey
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(failure());
        }
        let public_key = PublicKey::from_hex(&self.pubkey).map_err(|_| failure())?;
        // from_hex only validates length/encoding; ensure must return a curve point.
        public_key.xonly().map_err(|_| failure())?;
        Ok(public_key)
    }
}

/// An explicit short-lived bearer. Intentionally neither Debug nor Serialize.
pub struct RemoteCredentials(Zeroizing<String>);

impl RemoteCredentials {
    /// Own one access token. The login owner must enforce its expiration/refresh.
    pub fn new(token: String) -> Self {
        Self(Zeroizing::new(token))
    }

    fn headers(&self) -> Result<HeaderMap, String> {
        let token = self.0.as_str();
        if token.is_empty()
            || token.len() > 16 * 1024
            || token.bytes().any(|b| b.is_ascii_whitespace())
        {
            return Err(failure());
        }
        let mut bearer =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| failure())?;
        bearer.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, bearer);
        Ok(headers)
    }
}

/// Authorization owned by one login generation, not a process-global identity.
///
/// Implementations serialize refresh and secure persistence, enforce <=300-second
/// token lifetimes, and invalidate old snapshots on logout/replacement. They must
/// never resolve another account or silently fall back to a local key.
pub trait RemoteAuthorization: Send + Sync {
    /// Fail if this captured login generation has been invalidated.
    fn check_active(&self) -> Result<(), String>;

    /// Obtain an unexpired bearer for this same captured login generation.
    fn credentials(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteCredentials, String>> + Send + '_>>;
}

/// HTTPS ensure/sign transport configured with the deployment prefix, not a key.
#[derive(Clone)]
pub struct RemoteIdentityClient {
    client: reqwest::Client,
    base: url::Url,
}

impl RemoteIdentityClient {
    /// Construct from a trusted release-selected HTTPS deployment prefix.
    /// There is no default host, account selector, route alias or local fallback.
    pub fn new(base: &str) -> Result<Self, String> {
        let base = deployment_prefix(base)?;
        Ok(Self {
            client: http_client()?,
            base,
        })
    }

    /// Create-or-return custody and initial admission/profile for the authenticated
    /// account. This is an enrollment operation, NOT a read-only session probe.
    pub async fn ensure(&self, credentials: &RemoteCredentials) -> Result<ManagedIdentity, String> {
        let identity: ManagedIdentity = self
            .post(
                "v1/buzz/identity/ensure",
                &serde_json::json!({}),
                credentials,
            )
            .await?;
        identity.validate()?;
        Ok(identity)
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        credentials: &RemoteCredentials,
    ) -> Result<T, String> {
        let body = serde_json::to_vec(body).map_err(|_| failure())?;
        if body.len() > 128 * 1024 {
            return Err(failure());
        }
        let mut response = self
            .client
            .post(self.base.join(path).map_err(|_| failure())?)
            .headers(credentials.headers()?)
            .header("Cache-Control", "no-store")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| failure())?;
        if !response.status().is_success() {
            return Err(failure());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err(failure());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| failure())
    }
}

/// A signer snapshot implementing the neutral exact-event boundary.
///
/// It holds the server-selected identity and the SAME authorization owner for its
/// entire lifetime. Event creation and publication (including retry journals)
/// remain with callers; sign never enrolls, publishes or retries a failed request.
#[derive(Clone)]
pub struct RemoteEventSigner {
    client: RemoteIdentityClient,
    public_key: PublicKey,
    identity: ManagedIdentity,
    authorization: Arc<dyn RemoteAuthorization>,
}

impl RemoteEventSigner {
    /// Bind a validated ensure response to a captured login authorization owner.
    pub fn new(
        client: RemoteIdentityClient,
        identity: &ManagedIdentity,
        authorization: Arc<dyn RemoteAuthorization>,
    ) -> Result<Self, String> {
        let public_key = identity.validate()?;
        authorization.check_active()?;
        Ok(Self {
            client,
            public_key,
            identity: identity.clone(),
            authorization,
        })
    }

    // Proof destinations are application-built, but must still belong to this
    // captured managed community. Never send an off-origin proof for signing.
    fn validate_proof_scope(&self, kind: u16, tags: &[Vec<String>]) -> Result<(), String> {
        let value = |name: &str| -> Result<&str, String> {
            let mut values = tags.iter().filter(|t| t.first().is_some_and(|s| s == name));
            let tag = values.next().ok_or_else(failure)?;
            if tag.len() != 2 || values.next().is_some() {
                return Err(failure());
            }
            Ok(tag[1].as_str())
        };
        match kind {
            22242
                if value("relay")? != self.identity.relay_ws_url
                    || tags.iter().any(|t| t.first().is_some_and(|s| s == "auth")) =>
            {
                return Err(failure());
            }
            27235 => {
                let raw = value("u")?;
                let target = url::Url::parse(raw).map_err(|_| failure())?;
                let relay =
                    url::Url::parse(&self.identity.relay_http_url).map_err(|_| failure())?;
                if target.scheme() != "https"
                    || target.origin() != relay.origin()
                    || !target.username().is_empty()
                    || target.password().is_some()
                    || target.fragment().is_some()
                    || raw.contains('\\')
                {
                    return Err(failure());
                }
            }
            24242 => {
                let relay =
                    url::Url::parse(&self.identity.relay_http_url).map_err(|_| failure())?;
                if value("server")? != &relay[url::Position::BeforeHost..url::Position::AfterPort] {
                    return Err(failure());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl EventSigner for RemoteEventSigner {
    fn public_key(&self) -> PublicKey {
        self.public_key
    }

    fn sign(
        &self,
        unsigned: UnsignedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<Event, String>> + Send + '_>> {
        Box::pin(async move {
            self.authorization.check_active()?;
            if unsigned.pubkey != self.public_key {
                return Err(failure());
            }
            let tags: Vec<Vec<String>> = unsigned
                .tags
                .iter()
                .map(|t| t.as_slice().to_vec())
                .collect();
            self.validate_proof_scope(unsigned.kind.as_u16(), &tags)?;
            let purpose = match unsigned.kind.as_u16() {
                22242 => "nip42-auth",
                27235 => "http-auth",
                24242
                    if tags
                        .iter()
                        .any(|t| t.len() == 2 && t[0] == "t" && t[1] == "upload") =>
                {
                    "media-upload"
                }
                24242 => "media-read",
                _ => "publish",
            };
            let template = serde_json::json!({"kind": unsigned.kind.as_u16(), "created_at": unsigned.created_at.as_secs(), "tags": tags, "content": unsigned.content});
            let credentials = self.authorization.credentials().await?;
            self.authorization.check_active()?;
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Reply {
                event: Event,
            }
            let reply: Reply = self
                .client
                .post(
                    "v1/buzz/identity/sign",
                    &serde_json::json!({"purpose": purpose, "event": template}),
                    &credentials,
                )
                .await?;
            self.authorization.check_active()?;
            let event = reply.event;
            if event.pubkey != self.public_key
                || event.created_at != unsigned.created_at
                || event.kind != unsigned.kind
                || event.tags != unsigned.tags
                || event.content != unsigned.content
                || event.verify().is_err()
            {
                return Err(failure());
            }
            Ok(event)
        })
    }
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| failure())
}

// Unlike release-owned signer/issuer URLs, the backend community contract
// (IdentitySigningPolicy) permits an HTTPS origin with a non-default port.
// Keep raw canonical spelling: no userinfo, query, fragment or normalized path.
fn relay_https_origin(raw: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(raw).map_err(|_| failure())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
        || parsed.port() == Some(0)
        || raw.trim_end_matches('/') != parsed.origin().ascii_serialization()
        || raw.ends_with("//")
    {
        return Err(failure());
    }
    Ok(parsed)
}

// Compare raw authority as well as parsed fields: URL parsers normalize away
// default ports, empty userinfo and backslashes. Release policy forbids all three.
pub(crate) fn trusted_https_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| failure())?;
    let authority = raw
        .strip_prefix("https://")
        .and_then(|s| s.split('/').next())
        .ok_or_else(failure)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.contains(['\\', '?', '#'])
        || raw.bytes().any(|b| b <= 32 || b >= 127)
        || authority.to_ascii_lowercase() != url.host_str().unwrap_or_default()
    {
        return Err(failure());
    }
    Ok(url)
}

pub(crate) fn deployment_prefix(raw: &str) -> Result<url::Url, String> {
    let url = trusted_https_url(raw)?;
    let path = raw
        .strip_prefix("https://")
        .and_then(|s| s.find('/').map(|i| &s[i..]))
        .ok_or_else(failure)?;
    if path == "/"
        || !path.ends_with('/')
        || path.contains("//")
        || path.contains('%')
        || path.contains("/v1/")
        || path.split('/').any(|p| p == "." || p == "..")
    {
        return Err(failure());
    }
    Ok(url)
}

fn failure() -> String {
    "Managed identity request failed; corporate login or authorization is required".into()
}

#[cfg(test)]
#[path = "remote_identity_tests.rs"]
mod tests;
