use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, Keys, Tag};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
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
    pub async fn connect(url: &str) -> Result<Self, WsClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| WsClientError::Url(e.to_string()))?;

        let (ws, _response) = connect_async(parsed.as_str())
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

    use nostr::{EventBuilder, Kind};
    use tokio_tungstenite::accept_async;

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

    type ServerStream = WebSocketStream<tokio::net::TcpStream>;

    /// Bind an in-process WebSocket server and return its `ws://` URL plus a
    /// handle that resolves to the server side of the stream once a client
    /// completes the handshake.
    async fn spawn_test_relay() -> (String, tokio::task::JoinHandle<ServerStream>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test websocket");
        let address = listener.local_addr().expect("read test address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept test websocket");
            accept_async(stream)
                .await
                .expect("complete server websocket handshake")
        });
        (format!("ws://{address}"), server)
    }

    async fn next_test_frame(server: &mut ServerStream) -> Value {
        let message = timeout(Duration::from_secs(5), server.next())
            .await
            .expect("timed out waiting for websocket frame")
            .expect("test websocket closed")
            .expect("read test websocket frame");
        serde_json::from_str(message.to_text().expect("expected text frame"))
            .expect("parse test websocket frame")
    }

    async fn send_frame(server: &mut ServerStream, value: Value) {
        server
            .send(Message::Text(value.to_string().into()))
            .await
            .expect("send test frame");
    }

    /// Build a real signed Nostr event for publish tests.
    fn make_test_event(keys: &Keys) -> Event {
        EventBuilder::new(Kind::TextNote, "test")
            .sign_with_keys(keys)
            .expect("signing should succeed")
    }

    async fn connect_test_pair() -> (NostrWsConnection, ServerStream) {
        let (url, accept) = spawn_test_relay().await;
        let conn = NostrWsConnection::connect(&url)
            .await
            .expect("connect test client");
        let server = accept.await.expect("join test websocket server");
        (conn, server)
    }

    #[tokio::test]
    async fn connect_rejects_invalid_url() {
        let result = NostrWsConnection::connect("not a url").await;
        assert!(matches!(result, Err(WsClientError::Url(_))));
    }

    #[tokio::test]
    async fn authenticate_signs_challenge_and_accepts_ok() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();
        let pubkey = keys.public_key();

        let server_task = tokio::spawn(async move {
            let mut server = server;
            send_frame(&mut server, json!(["AUTH", "test-challenge"])).await;
            let frame = next_test_frame(&mut server).await;
            assert_eq!(frame[0], "AUTH");
            let auth_event: Event =
                serde_json::from_value(frame[1].clone()).expect("deserialize AUTH event");
            let event_id = auth_event.id.to_hex();
            send_frame(&mut server, json!(["OK", event_id, true, ""])).await;
            auth_event
        });

        conn.authenticate(&keys, None).await.expect("authenticate");

        let auth_event = server_task.await.expect("join server task");
        assert_eq!(auth_event.kind, Kind::Authentication);
        assert_eq!(auth_event.pubkey, pubkey);
        auth_event.verify().expect("valid signature");
        let tags = serde_json::to_value(&auth_event).expect("serialize AUTH event")["tags"]
            .as_array()
            .cloned()
            .expect("tags array");
        assert!(
            tags.contains(&json!(["challenge", "test-challenge"])),
            "missing challenge tag in {tags:?}"
        );
    }

    #[tokio::test]
    async fn authenticate_rejected_by_relay_fails_with_reason() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();

        let server_task = tokio::spawn(async move {
            let mut server = server;
            send_frame(&mut server, json!(["AUTH", "test-challenge"])).await;
            let frame = next_test_frame(&mut server).await;
            let event_id = frame[1]["id"].as_str().expect("auth event id").to_string();
            send_frame(
                &mut server,
                json!(["OK", event_id, false, "auth-required: bad"]),
            )
            .await;
        });

        let result = conn.authenticate(&keys, None).await;
        server_task.await.expect("join server task");
        match result {
            Err(WsClientError::AuthFailed(message)) => {
                assert_eq!(message, "auth-required: bad");
            }
            other => panic!("expected AuthFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn authenticate_buffers_earlier_messages_for_later_delivery() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();

        let server_task = tokio::spawn(async move {
            let mut server = server;
            send_frame(&mut server, json!(["NOTICE", "welcome"])).await;
            send_frame(&mut server, json!(["EOSE", "sub-1"])).await;
            send_frame(&mut server, json!(["AUTH", "test-challenge"])).await;
            let frame = next_test_frame(&mut server).await;
            let event_id = frame[1]["id"].as_str().expect("auth event id").to_string();
            send_frame(&mut server, json!(["OK", event_id, true, ""])).await;
            server
        });

        conn.authenticate(&keys, None).await.expect("authenticate");
        let _server = server_task.await.expect("join server task");

        match conn.next_event(Duration::from_secs(1)).await {
            Ok(RelayMessage::Notice { message }) => assert_eq!(message, "welcome"),
            other => panic!("expected buffered Notice first, got {other:?}"),
        }
        match conn.next_event(Duration::from_secs(1)).await {
            Ok(RelayMessage::Eose { subscription_id }) => assert_eq!(subscription_id, "sub-1"),
            other => panic!("expected buffered Eose second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn authenticate_rejects_oversized_challenge() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();

        let server_task = tokio::spawn(async move {
            let mut server = server;
            send_frame(&mut server, json!(["AUTH", "x".repeat(1025)])).await;
            server
        });

        let result = conn.authenticate(&keys, None).await;
        let _server = server_task.await.expect("join server task");
        match result {
            Err(WsClientError::AuthFailed(message)) => {
                assert_eq!(message, "challenge exceeds 1024 bytes");
            }
            other => panic!("expected AuthFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_event_returns_matching_ok_and_buffers_interleaved_messages() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();
        let event = make_test_event(&keys);
        let event_id = event.id.to_hex();

        let server_task = tokio::spawn(async move {
            let mut server = server;
            let frame = next_test_frame(&mut server).await;
            assert_eq!(frame[0], "EVENT");
            let published_id = frame[1]["id"].as_str().expect("event id").to_string();
            send_frame(&mut server, json!(["EOSE", "sub-1"])).await;
            send_frame(&mut server, json!(["OK", "deadbeef", false, "other event"])).await;
            send_frame(&mut server, json!(["OK", published_id, true, "stored"])).await;
            server
        });

        let ok = conn.send_event(event).await.expect("send event");
        let _server = server_task.await.expect("join server task");
        assert!(ok.accepted);
        assert_eq!(ok.event_id, event_id);
        assert_eq!(ok.message, "stored");

        match conn.next_event(Duration::from_secs(1)).await {
            Ok(RelayMessage::Eose { subscription_id }) => assert_eq!(subscription_id, "sub-1"),
            other => panic!("expected buffered Eose first, got {other:?}"),
        }
        match conn.next_event(Duration::from_secs(1)).await {
            Ok(RelayMessage::Ok(other_ok)) => assert_eq!(other_ok.event_id, "deadbeef"),
            other => panic!("expected buffered Ok second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_event_answers_ping_while_waiting_for_ok() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();
        let event = make_test_event(&keys);

        let server_task = tokio::spawn(async move {
            let mut server = server;
            let frame = next_test_frame(&mut server).await;
            let published_id = frame[1]["id"].as_str().expect("event id").to_string();
            server
                .send(Message::Ping(b"heartbeat".to_vec().into()))
                .await
                .expect("send ping");
            let reply = timeout(Duration::from_secs(5), server.next())
                .await
                .expect("timed out waiting for pong")
                .expect("test websocket closed")
                .expect("read pong frame");
            match reply {
                Message::Pong(data) => assert_eq!(data.as_ref(), b"heartbeat"),
                other => panic!("expected Pong, got {other:?}"),
            }
            send_frame(&mut server, json!(["OK", published_id, true, ""])).await;
            server
        });

        let ok = conn.send_event(event).await.expect("send event");
        let _server = server_task.await.expect("join server task");
        assert!(ok.accepted);
    }

    #[tokio::test]
    async fn auth_challenge_received_while_publishing_is_reused_by_authenticate() {
        let (mut conn, server) = connect_test_pair().await;
        let keys = Keys::generate();
        let event = make_test_event(&keys);

        let server_task = tokio::spawn(async move {
            let mut server = server;
            let frame = next_test_frame(&mut server).await;
            let published_id = frame[1]["id"].as_str().expect("event id").to_string();
            send_frame(&mut server, json!(["AUTH", "late-challenge"])).await;
            send_frame(&mut server, json!(["OK", published_id, true, ""])).await;
            // The client must now authenticate against the stored challenge
            // without waiting for another AUTH frame from the relay.
            let auth_frame = next_test_frame(&mut server).await;
            assert_eq!(auth_frame[0], "AUTH");
            let tags = auth_frame[1]["tags"].as_array().cloned().expect("tags");
            assert!(
                tags.contains(&json!(["challenge", "late-challenge"])),
                "missing challenge tag in {tags:?}"
            );
            let auth_id = auth_frame[1]["id"].as_str().expect("auth id").to_string();
            send_frame(&mut server, json!(["OK", auth_id, true, ""])).await;
            server
        });

        let ok = conn.send_event(event).await.expect("send event");
        assert!(ok.accepted);
        conn.authenticate(&keys, None)
            .await
            .expect("authenticate with stored challenge");
        let _server = server_task.await.expect("join server task");

        match conn.next_event(Duration::from_secs(1)).await {
            Ok(RelayMessage::Auth { challenge }) => assert_eq!(challenge, "late-challenge"),
            other => panic!("expected buffered Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn next_event_times_out_when_relay_is_silent() {
        let (mut conn, _server) = connect_test_pair().await;
        let result = conn.next_event(Duration::from_millis(50)).await;
        assert!(matches!(result, Err(WsClientError::Timeout)));
    }

    #[tokio::test]
    async fn next_event_surfaces_connection_close() {
        let (mut conn, mut server) = connect_test_pair().await;
        server.close(None).await.expect("close server side");
        let result = conn.next_event(Duration::from_secs(5)).await;
        assert!(matches!(result, Err(WsClientError::ConnectionClosed)));
    }

    #[tokio::test]
    async fn publish_event_helper_completes_full_auth_and_publish_flow() {
        let (url, accept) = spawn_test_relay().await;
        let keys = Keys::generate();
        let event = make_test_event(&keys);
        let expected_id = event.id.to_hex();

        let server_task = tokio::spawn(async move {
            let mut server = accept.await.expect("join test websocket server");
            send_frame(&mut server, json!(["AUTH", "publish-challenge"])).await;
            let auth_frame = next_test_frame(&mut server).await;
            assert_eq!(auth_frame[0], "AUTH");
            let auth_id = auth_frame[1]["id"].as_str().expect("auth id").to_string();
            send_frame(&mut server, json!(["OK", auth_id, true, ""])).await;
            let event_frame = next_test_frame(&mut server).await;
            assert_eq!(event_frame[0], "EVENT");
            let event_id = event_frame[1]["id"].as_str().expect("event id").to_string();
            send_frame(&mut server, json!(["OK", event_id, true, "stored"])).await;
            event_id
        });

        let ok = publish_event(&url, event, &keys, None, 10)
            .await
            .expect("publish event");
        let published_id = server_task.await.expect("join server task");
        assert!(ok.accepted);
        assert_eq!(ok.event_id, expected_id);
        assert_eq!(published_id, expected_id);
        assert_eq!(ok.message, "stored");
    }
}
