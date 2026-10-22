use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Keys, Tag};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest,
    handshake::client::Request,
    http::{HeaderMap, HeaderValue},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::debug;

use crate::error::{NipFiConnectError, WsClientError};
use crate::message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Seconds to wait for the relay to send the NIP-42 AUTH challenge after connecting.
pub const AUTH_CHALLENGE_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to the AUTH event.
pub const AUTH_OK_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to a published event.
pub const PUBLISH_OK_TIMEOUT_SECS: u64 = 30;

/// A NIP-42-capable WebSocket connection to a Nostr relay.
pub struct NostrWsConnection {
    ws: WsStream,
    buffer: VecDeque<RelayMessage>,
    pending_challenge: Option<String>,
    relay_url: String,
    // Only active during the opt-in NIP-FI handshake. Ordinary connections
    // retain their existing behavior; authenticated traffic is not restricted.
    auth_buffer_remaining: Option<usize>,
    protected_assertion: Option<String>,
}

impl NostrWsConnection {
    /// Sends one explicit NIP-FI bearer header on a WSS upgrade, then proves
    /// possession of `keys` using NIP-42. The relay verifies the assertion and
    /// its binding to this key; this client does not verify or mint JWTs.
    ///
    /// `url` must come from trusted application policy and contain no user
    /// information, query, or fragment. `assertion` is the complete
    /// `Bearer <compact-JWS>` field, obtained from the issuer-bound login.
    /// The compact JWS is limited to 16 KiB; claims remain opaque.
    /// No NIP-OA delegation or unauthenticated fallback is attempted.
    /// `budget` bounds the complete upgrade and authentication, including DNS
    /// and TLS, and must be nonzero. Dropping this future closes its socket.
    ///
    /// The caller owns logout/account-switch cancellation and connection
    /// lifetime. Refresh requires a new connection with a fresh assertion;
    /// there is no in-band renewal. This method publishes no user event.
    ///
    /// This crate suppresses AUTH payload logs and redacts connection errors.
    /// Hosts must disable tungstenite raw-wire TRACE for this connection:
    /// dependency tracing can dump serialized upgrade bytes, including the
    /// assertion, despite the header's sensitive marker.
    pub async fn connect_authenticated_with_nip_fi(
        url: &str,
        keys: &Keys,
        assertion: &HeaderValue,
        budget: Duration,
    ) -> Result<Self, NipFiConnectError> {
        let request = nip_fi_request(url, assertion)?;
        Self::connect_nip_fi_request(request, url, keys, assertion, budget).await
    }

    async fn connect_nip_fi_request(
        request: Request,
        url: &str,
        keys: &Keys,
        assertion: &HeaderValue,
        budget: Duration,
    ) -> Result<Self, NipFiConnectError> {
        if budget.is_zero() {
            return Err(NipFiConnectError::InvalidConfiguration);
        }
        let token = nip_fi_token(assertion)?;
        timeout(budget, async {
            let (ws, _) = connect_async(request)
                .await
                .map_err(WsClientError::WebSocket)?;
            let mut conn = Self {
                ws,
                buffer: VecDeque::new(),
                pending_challenge: None,
                relay_url: url.to_owned(),
                auth_buffer_remaining: Some(64 * 1024),
                protected_assertion: Some(token.to_owned()),
            };
            conn.authenticate_inner(keys, None, Some(token)).await?;
            conn.auth_buffer_remaining = None;
            Ok::<_, WsClientError>(conn)
        })
        .await
        .map_err(|_| NipFiConnectError::Timeout)?
        .map_err(NipFiConnectError::from)
    }

    /// Connects to the relay at `url` and performs NIP-42 authentication with `keys`.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn connect_authenticated(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<Self, WsClientError> {
        Self::connect_authenticated_with_headers(url, keys, auth_tag, HeaderMap::new()).await
    }

    /// [`Self::connect_authenticated`], sending `headers` on the WebSocket upgrade request.
    ///
    /// Use this for credentials the relay checks at upgrade time, such as a NIP-FI
    /// `Nostr-Federated-Identity` assertion that must pair with the NIP-42 AUTH key.
    pub async fn connect_authenticated_with_headers(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
        headers: HeaderMap,
    ) -> Result<Self, WsClientError> {
        let mut conn = Self::connect_with_headers(url, headers).await?;
        conn.authenticate(keys, auth_tag).await?;
        Ok(conn)
    }

    /// Connects to the relay at `url` without performing authentication.
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        Self::connect_with_headers(url, HeaderMap::new()).await
    }

    /// [`Self::connect`], sending `headers` on the WebSocket upgrade request.
    ///
    /// The headers are added after the handshake headers, so supplying a handshake
    /// header such as `Sec-WebSocket-Key` replaces it and breaks the upgrade.
    pub async fn connect_with_headers(
        url: &str,
        headers: HeaderMap,
    ) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;

        let mut request = parsed.as_str().into_client_request()?;
        request.headers_mut().extend(headers);

        let (ws, _response) = connect_async(request)
            .await
            .map_err(WsClientError::WebSocket)?;

        debug!("connected to relay at {url}");

        Ok(Self {
            ws,
            buffer: VecDeque::new(),
            pending_challenge: None,
            relay_url: url.to_string(),
            auth_buffer_remaining: None,
            protected_assertion: None,
        })
    }

    /// Performs NIP-42 authentication using `keys` against the connected relay.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn authenticate(
        &mut self,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<(), WsClientError> {
        self.authenticate_inner(keys, auth_tag, None).await
    }

    async fn authenticate_inner(
        &mut self,
        keys: &Keys,
        auth_tag: Option<&Tag>,
        protected_assertion: Option<&str>,
    ) -> Result<(), WsClientError> {
        let challenge = self
            .wait_for_auth_challenge(Duration::from_secs(AUTH_CHALLENGE_TIMEOUT_SECS))
            .await?;

        // An untrusted relay can echo our upgrade credential as its challenge.
        // Never turn that credential into a signed event or a trace record.
        if protected_assertion.is_some_and(|token| challenge.contains(token)) {
            return Err(WsClientError::AuthFailed(
                "invalid authentication challenge".into(),
            ));
        }

        let auth_event = build_auth_event(&challenge, &self.relay_url, keys, auth_tag)?;
        let event_id = auth_event.id.to_hex();

        let auth = json!(["AUTH", auth_event]);
        if protected_assertion.is_some() {
            // NIP-FI authentication traces must not contain relay-controlled
            // challenge content. Keep the ordinary connector's behavior intact.
            self.ws.send(Message::Text(auth.to_string().into())).await?;
        } else {
            self.send_raw(&auth).await?;
        }

        let ok = self
            .wait_for_ok(&event_id, Duration::from_secs(AUTH_OK_TIMEOUT_SECS))
            .await?;
        if !ok.accepted {
            return Err(WsClientError::AuthFailed(ok.message));
        }

        debug!("NIP-42 authentication successful");
        Ok(())
    }

    /// Sends a signed event to the relay and waits for the OK response.
    pub async fn send_event(&mut self, event: Event) -> Result<OkResponse, WsClientError> {
        let event_id = event.id.to_hex();
        self.send_raw(&json!(["EVENT", event])).await?;
        self.wait_for_ok(&event_id, Duration::from_secs(PUBLISH_OK_TIMEOUT_SECS))
            .await
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn next_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, WsClientError> {
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }
        self.recv_one(timeout_dur).await
    }

    /// Closes the WebSocket connection gracefully.
    pub async fn disconnect(mut self) -> Result<(), WsClientError> {
        self.ws.close(None).await?;
        Ok(())
    }

    /// Sends a raw JSON value as a WebSocket text frame.
    pub async fn send_raw(&mut self, value: &Value) -> Result<(), WsClientError> {
        let text = serde_json::to_string(value)?;
        if self.protected_assertion.is_none() {
            debug!("→ relay: {text}");
        }
        self.ws.send(Message::Text(text.into())).await?;
        Ok(())
    }

    async fn recv_one(&mut self, timeout_dur: Duration) -> Result<RelayMessage, WsClientError> {
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }

        loop {
            let raw = timeout(timeout_dur, self.ws.next())
                .await
                .map_err(|_| WsClientError::Timeout)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = self.parse_message(&text)?;
                    if let RelayMessage::Auth { ref challenge } = msg {
                        self.pending_challenge = Some(challenge.clone());
                    }
                    return Ok(msg);
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_auth_challenge(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, WsClientError> {
        if let Some(challenge) = self.pending_challenge.take() {
            return Ok(challenge);
        }

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|m| matches!(m, RelayMessage::Auth { .. }))
        {
            match self.buffer.remove(idx).unwrap() {
                RelayMessage::Auth { challenge } => return Ok(challenge),
                _ => unreachable!(),
            }
        }

        let deadline = tokio::time::Instant::now() + timeout_dur;

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(WsClientError::NoAuthChallenge);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| WsClientError::NoAuthChallenge)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    self.check_auth_buffer(&text)?;
                    let msg = self.parse_message(&text)?;
                    match msg {
                        RelayMessage::Auth { challenge } => {
                            if challenge.len() > 1024 {
                                return Err(WsClientError::AuthFailed(
                                    "challenge exceeds 1024 bytes".into(),
                                ));
                            }
                            return Ok(challenge);
                        }
                        other => self.buffer.push_back(other),
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_ok(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, WsClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|m| matches!(m, RelayMessage::Ok(ok) if ok.event_id == event_id))
        {
            match self.buffer.remove(idx).unwrap() {
                RelayMessage::Ok(ok) => return Ok(ok),
                _ => unreachable!(),
            }
        }

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(WsClientError::Timeout);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| WsClientError::Timeout)?
                .ok_or(WsClientError::ConnectionClosed)?
                .map_err(WsClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    self.check_auth_buffer(&text)?;
                    let msg = self.parse_message(&text)?;
                    match msg {
                        RelayMessage::Ok(ok) if ok.event_id == event_id => return Ok(ok),
                        RelayMessage::Auth { ref challenge } => {
                            self.pending_challenge = Some(challenge.clone());
                            self.buffer.push_back(msg);
                        }
                        other => self.buffer.push_back(other),
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    fn parse_message(&self, text: &str) -> Result<RelayMessage, WsClientError> {
        if let Some(token) = &self.protected_assertion {
            let decoded: Value = serde_json::from_str(text).map_err(|_| {
                WsClientError::AuthFailed("invalid protected relay response".to_owned())
            })?;
            if decoded.to_string().contains(token) {
                return Err(WsClientError::AuthFailed(
                    "invalid protected relay response".to_owned(),
                ));
            }
        }
        parse_relay_message(text).map_err(|error| {
            if self.protected_assertion.is_some() {
                WsClientError::AuthFailed("invalid protected relay response".to_owned())
            } else {
                error
            }
        })
    }

    fn check_auth_buffer(&mut self, text: &str) -> Result<(), WsClientError> {
        if let Some(remaining) = &mut self.auth_buffer_remaining {
            if self.buffer.len() >= 64 {
                return Err(WsClientError::UnexpectedMessage(
                    "authentication buffer limit".into(),
                ));
            }
            *remaining = remaining.checked_sub(text.len()).ok_or_else(|| {
                WsClientError::UnexpectedMessage("authentication buffer limit".into())
            })?;
        }
        Ok(())
    }
}

fn nip_fi_token(assertion: &HeaderValue) -> Result<&str, NipFiConnectError> {
    // Match the bounded compact-JWS transport shape without interpreting claims.
    let token = assertion
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.len() <= 16 * 1024)
        .ok_or(NipFiConnectError::InvalidConfiguration)?;
    let mut segments = token.split('.');
    for _ in 0..3 {
        if !segments.next().is_some_and(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }) {
            return Err(NipFiConnectError::InvalidConfiguration);
        }
    }
    if segments.next().is_some() {
        return Err(NipFiConnectError::InvalidConfiguration);
    }
    Ok(token)
}

fn nip_fi_request(url: &str, assertion: &HeaderValue) -> Result<Request, NipFiConnectError> {
    nip_fi_token(assertion)?;
    let parsed = url::Url::parse(url).map_err(|_| NipFiConnectError::InvalidConfiguration)?;
    if parsed.scheme() != "wss"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(NipFiConnectError::InvalidConfiguration);
    }
    let mut request = parsed
        .as_str()
        .into_client_request()
        .map_err(|_| NipFiConnectError::InvalidConfiguration)?;
    let mut header = assertion.clone();
    header.set_sensitive(true);
    request
        .headers_mut()
        .insert("Nostr-Federated-Identity", header);
    Ok(request)
}

#[cfg(test)]
#[path = "nip_fi_tests.rs"]
mod nip_fi_tests;

/// One-shot helper: connect, authenticate, send one event, disconnect.
///
/// Establishes a fresh WebSocket connection, completes NIP-42 authentication,
/// publishes `event`, waits for the relay's OK response, then closes the
/// connection. The entire operation is bounded by `timeout_secs`.
pub async fn publish_event(
    relay_url: &str,
    event: Event,
    keys: &Keys,
    auth_tag: Option<&Tag>,
    timeout_secs: u64,
) -> Result<OkResponse, WsClientError> {
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut conn = NostrWsConnection::connect(relay_url).await?;
        conn.authenticate(keys, auth_tag).await?;
        let ok = conn.send_event(event).await?;
        let _ = conn.disconnect().await;
        Ok::<_, WsClientError>(ok)
    })
    .await
    .map_err(|_| WsClientError::Timeout)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_challenge_timeout_meets_floor() {
        const { assert!(AUTH_CHALLENGE_TIMEOUT_SECS >= 20) };
    }

    #[test]
    fn auth_ok_timeout_meets_floor() {
        const { assert!(AUTH_OK_TIMEOUT_SECS >= 20) };
    }

    #[test]
    fn publish_ok_timeout_meets_floor() {
        const { assert!(PUBLISH_OK_TIMEOUT_SECS >= 30) };
    }

    // The handshake callback's `Result<Response, ErrorResponse>` is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn connect_with_headers_sends_headers_on_upgrade() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        use tokio_tungstenite::tungstenite::http::HeaderValue;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut received = None;
            let _ws = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &Request, response: Response| {
                    received = request.headers().get("nostr-federated-identity").cloned();
                    Ok(response)
                },
            )
            .await
            .unwrap();
            received
        });

        let mut headers = HeaderMap::new();
        headers.insert(
            "Nostr-Federated-Identity",
            HeaderValue::from_static("Bearer header.payload.signature"),
        );
        let _conn = NostrWsConnection::connect_with_headers(&url, headers)
            .await
            .unwrap();

        assert_eq!(
            server.await.unwrap(),
            Some(HeaderValue::from_static("Bearer header.payload.signature"))
        );
    }
}
