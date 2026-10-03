use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Keys, Tag};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};
use tracing::debug;

use crate::error::WsClientError;
use crate::message::{build_auth_event, parse_relay_message, OkResponse, RelayMessage};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Seconds to wait for the relay to send the NIP-42 AUTH challenge after connecting.
pub const AUTH_CHALLENGE_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to the AUTH event.
pub const AUTH_OK_TIMEOUT_SECS: u64 = 20;

/// Seconds to wait for the relay's OK response to a published event.
pub const PUBLISH_OK_TIMEOUT_SECS: u64 = 30;

/// Explicit transport and replay-queue budgets. Byte accounting measures raw
/// UTF-8 wire text, not Rust heap allocation. Message/frame limits bound parsing;
/// count and byte limits together bound buffered messages. Defaults are finite.
#[derive(Debug, Clone)]
pub struct ConnectionOptions {
    pub connect_timeout: Duration,
    pub authentication_timeout: Duration,
    pub write_timeout: Duration,
    pub close_timeout: Duration,
    pub max_frame_bytes: usize,
    pub max_message_bytes: usize,
    pub max_buffered_messages: usize,
    pub max_buffered_bytes: usize,
}
impl Default for ConnectionOptions {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(20),
            authentication_timeout: Duration::from_secs(40),
            write_timeout: Duration::from_secs(10),
            close_timeout: Duration::from_secs(5),
            max_frame_bytes: 1024 * 1024,
            max_message_bytes: 1024 * 1024,
            max_buffered_messages: 256,
            max_buffered_bytes: 8 * 1024 * 1024,
        }
    }
}
impl ConnectionOptions {
    fn validate(&self) -> Result<(), WsClientError> {
        if self.connect_timeout.is_zero()
            || self.authentication_timeout.is_zero()
            || self.write_timeout.is_zero()
            || self.close_timeout.is_zero()
            || self.max_frame_bytes == 0
            || self.max_message_bytes == 0
            || self.max_frame_bytes > self.max_message_bytes
            || self.max_buffered_messages == 0
            || self.max_buffered_bytes == 0
        {
            return Err(WsClientError::InvalidConnectionOptions);
        }
        Ok(())
    }
}
// This bounds serialized output, not the caller-owned Value/Event allocation.
struct LimitedJson {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl std::io::Write for LimitedJson {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("JSON size limit"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct BufferedMessage {
    message: RelayMessage,
    wire_bytes: usize,
}
/// A NIP-42-capable WebSocket connection to a Nostr relay.
pub struct NostrWsConnection {
    ws: Option<WsStream>,
    buffer: VecDeque<BufferedMessage>,
    buffered_bytes: usize,
    pending_challenge: Option<String>,
    relay_url: String,
    options: ConnectionOptions,
}
impl NostrWsConnection {
    /// Uses finite default connection, transport and replay budgets.
    pub async fn connect_authenticated(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<Self, WsClientError> {
        Self::connect_authenticated_with_options(url, keys, auth_tag, ConnectionOptions::default())
            .await
    }
    pub async fn connect_authenticated_with_options(
        url: &str,
        keys: &Keys,
        auth_tag: Option<&Tag>,
        options: ConnectionOptions,
    ) -> Result<Self, WsClientError> {
        let mut conn = Self::connect_with_options(url, options).await?;
        conn.authenticate(keys, auth_tag).await?;
        Ok(conn)
    }
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        Self::connect_with_options(url, ConnectionOptions::default()).await
    }
    pub async fn connect_with_options(
        url: &str,
        options: ConnectionOptions,
    ) -> Result<Self, WsClientError> {
        options.validate()?;
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;
        let config = WebSocketConfig::default()
            .max_frame_size(Some(options.max_frame_bytes))
            .max_message_size(Some(options.max_message_bytes));
        let (ws, _) = timeout(
            options.connect_timeout,
            connect_async_with_config(parsed.as_str(), Some(config), false),
        )
        .await
        .map_err(|_| WsClientError::Timeout)?
        .map_err(WsClientError::WebSocket)?;
        debug!("connected to relay");
        Ok(Self {
            ws: Some(ws),
            buffer: VecDeque::new(),
            buffered_bytes: 0,
            pending_challenge: None,
            relay_url: url.into(),
            options,
        })
    }
    fn socket(&mut self) -> Result<&mut WsStream, WsClientError> {
        self.ws.as_mut().ok_or(WsClientError::ConnectionClosed)
    }
    fn close_transport(&mut self) {
        // Drop immediately: closing must not wait on a hostile peer.
        self.ws.take();
        self.buffer.clear();
        self.buffered_bytes = 0;
        self.pending_challenge = None;
    }
    fn close_on_limit(&mut self) -> WsClientError {
        self.close_transport();
        WsClientError::ResourceLimit
    }
    fn enqueue(&mut self, message: RelayMessage, wire_bytes: usize) -> Result<(), WsClientError> {
        if self.buffer.len() >= self.options.max_buffered_messages
            || wire_bytes
                > self
                    .options
                    .max_buffered_bytes
                    .saturating_sub(self.buffered_bytes)
        {
            return Err(self.close_on_limit());
        }
        self.buffered_bytes += wire_bytes;
        self.buffer.push_back(BufferedMessage {
            message,
            wire_bytes,
        });
        Ok(())
    }
    fn remove_buffered(&mut self, index: usize) -> RelayMessage {
        let item = self.buffer.remove(index).expect("existing buffer index");
        self.buffered_bytes -= item.wire_bytes;
        item.message
    }
    fn decode(&mut self, text: &str) -> Result<RelayMessage, WsClientError> {
        if text.len() > self.options.max_message_bytes {
            return Err(self.close_on_limit());
        }
        let message = parse_relay_message(text)?;
        if matches!(&message, RelayMessage::Auth {challenge} if challenge.len()>1024) {
            return Err(self.close_on_limit());
        }
        Ok(message)
    }
    async fn receive_raw(&mut self) -> Result<Message, WsClientError> {
        match self.socket()?.next().await {
            Some(Ok(message)) => Ok(message),
            Some(Err(tokio_tungstenite::tungstenite::Error::Capacity(_))) => {
                Err(self.close_on_limit())
            }
            Some(Err(error)) => Err(WsClientError::WebSocket(error)),
            None => Err(WsClientError::ConnectionClosed),
        }
    }
    /// Performs NIP-42 authentication using `keys` against the connected relay.
    ///
    /// Pass `auth_tag` to include a NIP-OA authorization tag in the AUTH event.
    pub async fn authenticate(
        &mut self,
        keys: &Keys,
        auth_tag: Option<&Tag>,
    ) -> Result<(), WsClientError> {
        let result = timeout(
            self.options.authentication_timeout,
            self.authenticate_inner(keys, auth_tag),
        )
        .await;
        match result {
            Ok(result) => result,
            Err(_) => {
                self.close_transport();
                Err(WsClientError::Timeout)
            }
        }
    }
    async fn authenticate_inner(
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
        let deadline = Duration::from_secs(PUBLISH_OK_TIMEOUT_SECS);
        timeout(deadline, async {
            self.send_raw(&json!(["EVENT", event])).await?;
            self.wait_for_ok(&event_id, deadline).await
        })
        .await
        .map_err(|_| WsClientError::Timeout)?
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn next_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, WsClientError> {
        timeout(timeout_dur, self.recv_one())
            .await
            .map_err(|_| WsClientError::Timeout)?
    }

    /// Closes the WebSocket connection gracefully.
    pub async fn disconnect(mut self) -> Result<(), WsClientError> {
        timeout(self.options.close_timeout, self.socket()?.close(None))
            .await
            .map_err(|_| WsClientError::Timeout)??;
        Ok(())
    }

    /// Sends a raw JSON value as a WebSocket text frame.
    pub async fn send_raw(&mut self, value: &Value) -> Result<(), WsClientError> {
        let mut encoded = LimitedJson {
            bytes: Vec::new(),
            limit: self.options.max_message_bytes,
            exceeded: false,
        };
        let result = serde_json::to_writer(&mut encoded, value);
        if encoded.exceeded {
            return Err(self.close_on_limit());
        }
        result?;
        // serde_json emits valid UTF-8; no second serialization or unbounded buffer.
        let text = String::from_utf8(encoded.bytes).expect("JSON UTF-8");
        debug!(bytes = text.len(), "sending relay frame");
        match timeout(
            self.options.write_timeout,
            self.socket()?.send(Message::Text(text.into())),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                self.close_transport();
                return Err(WsClientError::Timeout);
            }
        }
        Ok(())
    }

    async fn recv_one(&mut self) -> Result<RelayMessage, WsClientError> {
        loop {
            let msg = if !self.buffer.is_empty() {
                self.remove_buffered(0)
            } else {
                match self.receive_raw().await? {
                    Message::Text(text) => self.decode(&text)?,
                    Message::Ping(data) => {
                        self.socket()?.send(Message::Pong(data)).await?;
                        continue;
                    }
                    Message::Close(_) => return Err(WsClientError::ConnectionClosed),
                    _ => continue,
                }
            };
            if let RelayMessage::Auth { ref challenge } = msg {
                self.pending_challenge = Some(challenge.clone());
            }
            return Ok(msg);
        }
    }

    async fn wait_for_auth_challenge(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, WsClientError> {
        timeout(timeout_dur, self.wait_for_auth_challenge_inner(timeout_dur))
            .await
            .map_err(|_| WsClientError::NoAuthChallenge)?
    }
    async fn wait_for_auth_challenge_inner(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, WsClientError> {
        if let Some(challenge) = self.pending_challenge.take() {
            return Ok(challenge);
        }

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|m| matches!(m.message, RelayMessage::Auth { .. }))
        {
            match self.remove_buffered(idx) {
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

            let raw = timeout(remaining, self.receive_raw())
                .await
                .map_err(|_| WsClientError::NoAuthChallenge)??;

            match raw {
                Message::Text(text) => {
                    let msg = self.decode(&text)?;
                    match msg {
                        RelayMessage::Auth { challenge } => {
                            return Ok(challenge);
                        }
                        other => self.enqueue(other, text.len())?,
                    }
                }
                Message::Ping(data) => {
                    timeout(remaining, self.socket()?.send(Message::Pong(data)))
                        .await
                        .map_err(|_| WsClientError::Timeout)??;
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
        timeout(timeout_dur, self.wait_for_ok_inner(event_id, timeout_dur))
            .await
            .map_err(|_| WsClientError::Timeout)?
    }
    async fn wait_for_ok_inner(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, WsClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;

        if let Some(idx) = self
            .buffer
            .iter()
            .position(|m| matches!(&m.message, RelayMessage::Ok(ok) if ok.event_id == event_id))
        {
            match self.remove_buffered(idx) {
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

            let raw = timeout(remaining, self.receive_raw())
                .await
                .map_err(|_| WsClientError::Timeout)??;

            match raw {
                Message::Text(text) => {
                    let msg = self.decode(&text)?;
                    match msg {
                        RelayMessage::Ok(ok) if ok.event_id == event_id => return Ok(ok),
                        other => self.enqueue(other, text.len())?,
                    }
                }
                Message::Ping(data) => {
                    timeout(remaining, self.socket()?.send(Message::Pong(data)))
                        .await
                        .map_err(|_| WsClientError::Timeout)??;
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

#[cfg(test)]
#[path = "resource_tests.rs"]
mod resource_tests;
