//! Client-side NIP-FI assertion lifetime and destination binding.
//!
//! This is not a JWT signature verifier. The adapter supplies assertions over a
//! trusted authenticated API; the relay remains the verifier. Keep this session
//! separate from local signing, and never put its credential in event tags.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::RwLock,
};

use nostr::PublicKey;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    http::{HeaderValue, Request},
};
use url::Url;

/// Current Unix seconds, failing closed if the clock is unavailable.
pub fn unix_now() -> Result<u64, IdentityError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| IdentityError::Unavailable)
}

/// The only permitted assertion carrier.
pub const IDENTITY_HEADER: &str = "nostr-federated-identity";

/// Fixed, credential-free client errors; safe for UI and diagnostics.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    /// Required authority is absent; prompt for login, not local-key fallback.
    #[error("enterprise sign-in required")]
    LoginRequired,
    /// Renewal is needed before another request can start.
    #[error("enterprise assertion expired")]
    Expired,
    /// The requested proof key differs from the assertion's key.
    #[error("enterprise assertion does not match signing identity")]
    KeyMismatch,
    /// A replaced login attempt must not install its result.
    #[error("enterprise authentication changed")]
    Superseded,
    /// Invalid local configuration, API response, or request destination.
    #[error("invalid enterprise authentication configuration or response")]
    Invalid,
    /// Lock failure must not downgrade to ordinary Nostr admission.
    #[error("enterprise authentication unavailable")]
    Unavailable,
}

/// An opaque short-lived assertion received from the trusted adapter.
///
/// No Serialize implementation or public token accessor: renderer state and
/// diagnostics should only receive readiness/expiry, never the JWT.
pub struct Assertion {
    header: HeaderValue,
    pubkey: PublicKey,
    expires_at: u64,
    binding: Option<(String, String, String)>,
}

impl fmt::Debug for Assertion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Assertion([REDACTED])")
    }
}

impl Assertion {
    /// Pin issuer, subject, and audience for this signing key until explicit logout.
    pub(crate) fn bind(mut self, issuer: String, subject: String, audience: String) -> Self {
        self.binding = Some((issuer, subject, audience));
        self
    }

    /// Construct from a trusted adapter result. `expires_at` must be no later
    /// than the token's effective deadline; API integration owns that check.
    pub fn new(
        jwt: &str,
        pubkey: PublicKey,
        expires_at: u64,
        now: u64,
    ) -> Result<Self, IdentityError> {
        if jwt.len() > 16 * 1024
            || jwt.split('.').count() != 3
            || jwt.split('.').any(|part| {
                part.is_empty()
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            return Err(IdentityError::Invalid);
        }
        if expires_at <= now {
            return Err(IdentityError::Expired);
        }
        let mut header =
            HeaderValue::from_str(&format!("Bearer {jwt}")).map_err(|_| IdentityError::Invalid)?;
        header.set_sensitive(true);
        Ok(Self {
            header,
            pubkey,
            expires_at,
            binding: None,
        })
    }
}

#[derive(Default)]
struct State {
    generation: u64,
    assertions: HashMap<PublicKey, Assertion>,
}

/// Process-owned assertion slot for one configured enterprise realm.
///
/// Destinations are explicit origins, never learned from NIP-11, an assertion,
/// an arbitrary image URL, or renderer input. Empty origins means OSS/off mode.
/// Multiple origins require explicit deployment configuration (e.g. git host).
#[derive(Default)]
pub struct IdentitySession {
    origins: HashSet<String>,
    state: RwLock<State>,
}

fn origin(value: &str) -> Result<String, IdentityError> {
    let mut url = Url::parse(value).map_err(|_| IdentityError::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(IdentityError::Invalid);
    }
    match url.scheme() {
        "wss" => {
            url.set_scheme("https")
                .map_err(|_| IdentityError::Invalid)?;
        }
        "ws" => {
            url.set_scheme("http").map_err(|_| IdentityError::Invalid)?;
        }
        "http" | "https" => {}
        _ => return Err(IdentityError::Invalid),
    }
    if url.host_str().is_none() {
        return Err(IdentityError::Invalid);
    }
    // Plaintext is solely for explicit loopback fixtures, never corporate hosts.
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        return Err(IdentityError::Invalid);
    }
    Ok(url.origin().ascii_serialization())
}

impl IdentitySession {
    /// Configure the enterprise destinations. No dynamic transport downgrade.
    pub fn new(origins: &[&str]) -> Result<Self, IdentityError> {
        Ok(Self {
            origins: origins
                .iter()
                .map(|value| origin(value))
                .collect::<Result<_, _>>()?,
            state: RwLock::default(),
        })
    }

    /// Whether this destination belongs to the explicitly configured realm.
    pub fn protects(&self, destination: &str) -> Result<bool, IdentityError> {
        if self.origins.is_empty() {
            return Ok(false);
        }
        Ok(self.origins.contains(&origin(destination)?))
    }

    /// Read the effective expiry without exposing the assertion.
    pub fn expires_at(&self, key: PublicKey) -> Result<Option<u64>, IdentityError> {
        Ok(self
            .state
            .read()
            .map_err(|_| IdentityError::Unavailable)?
            .assertions
            .get(&key)
            .map(|a| a.expires_at))
    }

    /// Transport lease: closes on logout, key replacement, or this token's expiry.
    /// Renewal does not extend a previously admitted socket's lease.
    pub fn lease_ended(
        &self,
        destination: &str,
        key: PublicKey,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let protected = self.protects(destination) != Ok(false);
        let generation = self.generation();
        let deadline = self.expires_at(key).ok().flatten().unwrap_or(0);
        async move {
            if !protected {
                std::future::pending::<()>().await;
                return;
            }
            loop {
                if self.generation() != generation || unix_now().unwrap_or(u64::MAX) >= deadline {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    }

    /// Begin login/logout/identity replacement, invalidating outstanding results.
    /// Existing transport owners must also cancel sockets and in-flight work.
    pub fn invalidate(&self) -> Result<u64, IdentityError> {
        let mut state = self.state.write().map_err(|_| IdentityError::Unavailable)?;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(IdentityError::Unavailable)?;
        state.assertions.clear();
        Ok(state.generation)
    }

    /// Read a generation before asynchronous renewal; does not discard a still
    /// valid assertion while the adapter is temporarily unavailable.
    pub fn generation(&self) -> Result<u64, IdentityError> {
        Ok(self
            .state
            .read()
            .map_err(|_| IdentityError::Unavailable)?
            .generation)
    }

    /// Install only into the login generation that requested the assertion.
    pub fn install(&self, generation: u64, assertion: Assertion) -> Result<(), IdentityError> {
        let mut state = self.state.write().map_err(|_| IdentityError::Unavailable)?;
        if generation != state.generation {
            return Err(IdentityError::Superseded);
        }
        if state.assertions.len() >= 256 && !state.assertions.contains_key(&assertion.pubkey) {
            return Err(IdentityError::Unavailable);
        }
        if let Some(previous) = state.assertions.get(&assertion.pubkey) {
            if previous.binding != assertion.binding {
                return Err(IdentityError::KeyMismatch);
            }
        }
        state.assertions.insert(assertion.pubkey, assertion);
        Ok(())
    }

    /// Obtain the assertion immediately before sending a matching possession
    /// proof. Other origins get no credential; configured origins fail closed.
    pub fn header(
        &self,
        destination: &str,
        proof_key: PublicKey,
        now: u64,
    ) -> Result<Option<HeaderValue>, IdentityError> {
        if self.origins.is_empty() {
            return Ok(None);
        }
        if !self.origins.contains(&origin(destination)?) {
            return Ok(None);
        }
        let state = self.state.read().map_err(|_| IdentityError::Unavailable)?;
        let assertion = state
            .assertions
            .get(&proof_key)
            .ok_or(if state.assertions.is_empty() {
                IdentityError::LoginRequired
            } else {
                IdentityError::KeyMismatch
            })?;
        if now >= assertion.expires_at {
            return Err(IdentityError::Expired);
        }
        if assertion.pubkey != proof_key {
            return Err(IdentityError::KeyMismatch);
        }
        Ok(Some(assertion.header.clone()))
    }

    /// Build a native upgrade request without exposing the JWT to JavaScript.
    pub fn websocket_request(
        &self,
        destination: &str,
        proof_key: PublicKey,
        now: u64,
    ) -> Result<Request<()>, IdentityError> {
        let mut request = destination
            .into_client_request()
            .map_err(|_| IdentityError::Invalid)?;
        if let Some(header) = self.header(destination, proof_key, now)? {
            request.headers_mut().insert(IDENTITY_HEADER, header);
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[tokio::test]
    #[allow(clippy::result_large_err)] // tungstenite's server callback fixes the error type.
    async fn native_connection_sends_assertion_only_on_upgrade() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let session = IdentitySession::new(&[&url]).unwrap();
        let key = Keys::generate().public_key();
        session
            .install(0, Assertion::new("aaa.bbb.ccc", key, 20, 10).unwrap())
            .unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    assert_eq!(request.headers()[IDENTITY_HEADER], "Bearer aaa.bbb.ccc");
                    assert!(request.uri().query().is_none());
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let frame = socket.next().await.unwrap().unwrap();
            assert!(frame.is_close());
        });
        let request = session.websocket_request(&url, key, 10).unwrap();
        let connection = crate::NostrWsConnection::connect_request(&url, request)
            .await
            .unwrap();
        connection.disconnect().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn off_mode_is_unchanged() {
        let key = Keys::generate().public_key();
        assert!(IdentitySession::default()
            .header("ws://fixture", key, 10)
            .unwrap()
            .is_none());
    }

    #[test]
    fn configured_origin_requires_matching_unexpired_evidence() {
        let key = Keys::generate().public_key();
        let session = IdentitySession::new(&["https://relay.example"]).unwrap();
        assert_eq!(
            session.header("wss://relay.example", key, 10),
            Err(IdentityError::LoginRequired)
        );
        session
            .install(0, Assertion::new("aaa.bbb.ccc", key, 20, 10).unwrap())
            .unwrap();
        let request = session
            .websocket_request("wss://relay.example/huddle/x/audio", key, 19)
            .unwrap();
        assert_eq!(request.headers()[IDENTITY_HEADER], "Bearer aaa.bbb.ccc");
        assert!(request.headers()[IDENTITY_HEADER].is_sensitive());
        assert_eq!(
            session.header("https://relay.example/query", key, 20),
            Err(IdentityError::Expired)
        );
        assert_eq!(
            session.header(
                "https://relay.example/query",
                Keys::generate().public_key(),
                19
            ),
            Err(IdentityError::KeyMismatch)
        );
        assert!(session
            .header("https://other.example/media/x", key, 19)
            .unwrap()
            .is_none());
        assert!(session
            .header("https://relay.example:444/query", key, 19)
            .unwrap()
            .is_none());
    }

    #[test]
    fn logout_fences_pending_login_and_same_key_replacement() {
        let session = IdentitySession::new(&["https://relay.example"]).unwrap();
        let generation = session.generation().unwrap();
        session.invalidate().unwrap();
        let assertion =
            Assertion::new("aaa.bbb.ccc", Keys::generate().public_key(), 20, 10).unwrap();
        assert_eq!(
            session.install(generation, assertion),
            Err(IdentityError::Superseded)
        );
    }

    #[test]
    fn configuration_and_token_errors_do_not_echo_credentials() {
        assert!(IdentitySession::new(&["http://relay.example"]).is_err());
        assert!(IdentitySession::new(&["https://user:secret@relay.example"]).is_err());
        let key = Keys::generate().public_key();
        for token in [
            "",
            "aaa.bbb",
            "aaa..ccc",
            "aaa.bbb.ccc\r\n",
            "aaa.bbb.ccc extra",
        ] {
            assert!(Assertion::new(token, key, 20, 10).is_err());
        }
        assert_eq!(
            format!("{:?}", Assertion::new("aaa.bbb.ccc", key, 20, 10).unwrap()),
            "Assertion([REDACTED])"
        );
    }
}
