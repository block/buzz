//! HTTPS corporate signer transport. No local key fallback and no secret-key export.
use std::time::Duration;

use nostr::Event;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::WsClientError;

/// Identity and community pinned by the corporate login lifecycle.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseSession {
    /// Custodied Nostr public key, never selected by a signing request.
    pub pubkey: String,
    /// Community WebSocket origin.
    pub relay_ws_url: String,
    /// Community HTTPS origin.
    pub relay_http_url: String,
}

/// Exact retryable NIP-01 template; signed fields are never sent to the service.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnterpriseTemplate {
    /// Nostr event kind.
    pub kind: u16,
    /// Preserve across retries rather than reconstructing from the current time.
    pub created_at: u64,
    /// Ordered Nostr tags.
    pub tags: Vec<Vec<String>>,
    /// Event content.
    pub content: String,
}

/// Ephemeral explicit credentials. Deliberately has no Debug/Serialize implementation.
pub struct EnterpriseCredentials {
    session: Zeroizing<String>,
    corporate: Zeroizing<String>,
}

impl EnterpriseCredentials {
    /// Own credentials for a request; the login owner is responsible for refresh and secure persistence.
    pub fn new(session: String, corporate: String) -> Self {
        Self {
            session: Zeroizing::new(session),
            corporate: Zeroizing::new(corporate),
        }
    }

    fn headers(&self) -> Result<HeaderMap, WsClientError> {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("x-bb-session-credential", self.session.as_str()),
            ("x-buzz-corporate-authorization", self.corporate.as_str()),
        ] {
            if value.is_empty() || value.len() > 16 * 1024 {
                return Err(failure());
            }
            let mut header = HeaderValue::from_str(value).map_err(|_| failure())?;
            header.set_sensitive(true);
            headers.insert(name, header);
        }
        Ok(headers)
    }
}

/// Shared native HTTPS transport for desktop/CLI integrations. It never owns or generates a Nostr key.
pub struct EnterpriseSigner {
    client: reqwest::Client,
    base: String,
    expected: EnterpriseSession,
}

impl EnterpriseSigner {
    /// Construct from trusted managed configuration and a login-pinned identity.
    pub fn new(base: &str, expected: EnterpriseSession) -> Result<Self, WsClientError> {
        let base_url = url::Url::parse(base).map_err(|_| failure())?;
        if !https(&base_url) {
            return Err(failure());
        }
        let relay = url::Url::parse(&expected.relay_http_url).map_err(|_| failure())?;
        if !https(&relay)
            || relay.path() != "/"
            || expected.relay_ws_url
                != format!(
                    "wss://{}",
                    relay[url::Position::BeforeHost..url::Position::AfterPort].to_owned()
                )
        {
            return Err(failure());
        }
        if expected.pubkey.len() != 64
            || !expected
                .pubkey
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(failure());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| failure())?;
        Ok(Self {
            client,
            base: base_url.as_str().trim_end_matches('/').to_owned(),
            expected,
        })
    }

    /// Return the pinned identity; fresh corporate authorization is still checked on every signing request.
    pub fn identity(&self) -> &EnterpriseSession {
        &self.expected
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
        credentials: &EnterpriseCredentials,
    ) -> Result<T, WsClientError> {
        let mut response = self
            .client
            .post(format!("{}/v1/buzz/enterprise-signer/{path}", self.base))
            .headers(credentials.headers()?)
            .header("Cache-Control", "no-store")
            .json(&body)
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

    /// Confirm provisioning and reject account/community changes during credential refresh.
    pub async fn session(&self, credentials: &EnterpriseCredentials) -> Result<(), WsClientError> {
        let actual: EnterpriseSession = self
            .post("session", serde_json::json!({}), credentials)
            .await?;
        if actual != self.expected {
            return Err(failure());
        }
        Ok(())
    }

    /// Sign and verify an exact template; persist the result before publishing and replay it after ambiguous ACKs.
    pub async fn sign(
        &self,
        template: &EnterpriseTemplate,
        credentials: &EnterpriseCredentials,
    ) -> Result<Event, WsClientError> {
        let purpose = match template.kind {
            22242 => "nip42-auth",
            27235 => "http-auth",
            24242
                if template
                    .tags
                    .iter()
                    .any(|t| t.len() == 2 && t[0] == "t" && t[1] == "upload") =>
            {
                "media-upload"
            }
            24242 => "media-read",
            _ => "publish",
        };
        #[derive(Deserialize)]
        struct Reply {
            event: Event,
        }
        let reply: Reply = self
            .post(
                "events/sign",
                serde_json::json!({"event": template, "purpose": purpose}),
                credentials,
            )
            .await?;
        let event = reply.event;
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        if event.pubkey.to_hex() != self.expected.pubkey
            || event.kind.as_u16() != template.kind
            || event.created_at.as_secs() != template.created_at
            || event.content != template.content
            || tags != template.tags
            || event.verify().is_err()
        {
            return Err(failure());
        }
        Ok(event)
    }
}

fn https(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
fn failure() -> WsClientError {
    WsClientError::AuthFailed(
        "Enterprise signing failed; corporate login or authorization is required".to_owned(),
    )
}

#[cfg(test)]
#[path = "enterprise_tests.rs"]
mod tests;
