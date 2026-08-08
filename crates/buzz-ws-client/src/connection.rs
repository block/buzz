use std::collections::VecDeque;
use std::time::Duration;

use buzz_core::client_identity::{
    client_header_value_for_host, may_identify_to, ClientApp, CLIENT_HEADER,
};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Keys, Tag};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::debug;
use url::Url;

use crate::error::WsClientError;
use crate::message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Build the handshake request for `url`, attaching the advisory `Buzz-Client`
/// header when the destination permits it.
///
/// Attaching the header is never allowed to fail a connection: an unshipped
/// platform or a destination outside [`may_identify_to`] simply yields a
/// request without the header.
fn client_request(
    url: &Url,
    app: ClientApp,
    app_version: Option<&str>,
) -> Result<ClientRequestBuilder, WsClientError> {
    let uri = url.as_str().parse().map_err(
        |e: tokio_tungstenite::tungstenite::http::uri::InvalidUri| {
            WsClientError::Url(e.to_string())
        },
    )?;
    let builder = ClientRequestBuilder::new(uri);
    if !may_identify_to(url) {
        return Ok(builder);
    }
    match client_header_value_for_host(app, app_version) {
        Some(value) => Ok(builder.with_header(CLIENT_HEADER, value)),
        None => Ok(builder),
    }
}

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
}

impl NostrWsConnection {
    /// Connects to the relay at `url` and performs NIP-42 authentication with `keys`.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn connect_authenticated(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<Self, WsClientError> {
        let mut conn = Self::connect(url).await?;
        conn.authenticate(keys, auth_tag).await?;
        Ok(conn)
    }

    /// Connects to the relay at `url` without performing authentication.
    ///
    /// Identifies as [`ClientApp::Cli`], which covers the `buzz` CLI and the
    /// test client. No version is sent: both inherit the workspace version,
    /// which has never been bumped (see `RELEASING.md`), so reporting it would
    /// pin every CLI connection to a fixed number forever. The relay records
    /// the version as unknown, which is accurate.
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        Self::connect_as(url, ClientApp::Cli, None).await
    }

    /// Connects to the relay at `url`, identifying as `app` in the advisory
    /// `Buzz-Client` header.
    ///
    /// Pass `app_version` only for a client with a real, independently bumped
    /// release version; pass `None` otherwise so the relay reports the version
    /// as unknown rather than as a misleading constant.
    ///
    /// The header lets the relay report which client versions and platforms
    /// its live connections come from. It is best-effort: if it cannot be
    /// built (unshipped platform) or the destination is not one this client may
    /// identify itself to, the connection proceeds without it and the relay
    /// counts it as unidentified.
    pub async fn connect_as(
        url: &str,
        app: ClientApp,
        app_version: Option<&str>,
    ) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;

        let request = client_request(&parsed, app, app_version)?;
        let (ws, _response) = connect_async(request)
            .await
            .map_err(WsClientError::WebSocket)?;

        debug!("connected to relay at {url}");

        Ok(Self {
            ws,
            buffer: VecDeque::new(),
            pending_challenge: None,
            relay_url: url.to_string(),
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
        let challenge = self
            .wait_for_auth_challenge(Duration::from_secs(AUTH_CHALLENGE_TIMEOUT_SECS))
            .await?;

        let auth_event = build_auth_event(&challenge, &self.relay_url, keys, auth_tag)?;
        let event_id = auth_event.id.to_hex();

        self.send_raw(&json!(["AUTH", auth_event])).await?;

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
        debug!("→ relay: {text}");
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
                    let msg = parse_relay_message(&text)?;
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
                    let msg = parse_relay_message(&text)?;
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
                    let msg = parse_relay_message(&text)?;
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
}

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
}
