#![deny(unsafe_code)]
#![warn(missing_docs)]

//! Minimal NIP-01 WebSocket test client for the Sprout relay.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, RelayUrl, Tag, Url};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::debug;

pub use sprout_mcp::relay_client::{parse_relay_message, OkResponse, RelayMessage};

/// Errors returned by [`SproutTestClient`] operations.
#[derive(Debug, Error)]
pub enum TestClientError {
    /// A WebSocket transport error occurred.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Failed to build a Nostr event.
    #[error("Nostr event builder error: {0}")]
    EventBuilder(String),

    /// Failed to parse a URL.
    #[error("URL parse error: {0}")]
    Url(String),

    /// The relay did not respond within the expected time.
    #[error("Timeout waiting for relay message")]
    Timeout,

    /// The WebSocket connection was closed before the operation completed.
    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    /// The relay sent a message that was not expected at this point.
    #[error("Unexpected relay message: {0}")]
    UnexpectedMessage(String),

    /// NIP-42 authentication was rejected by the relay.
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// The relay rejected the submitted event.
    #[error("Event rejected by relay: {0}")]
    EventRejected(String),

    /// No NIP-42 AUTH challenge was received from the relay.
    #[error("No AUTH challenge received from relay")]
    NoAuthChallenge,
}

impl From<nostr::event::builder::Error> for TestClientError {
    fn from(e: nostr::event::builder::Error) -> Self {
        TestClientError::EventBuilder(e.to_string())
    }
}

// Map RelayClientError → TestClientError for parse_relay_message calls.
impl From<sprout_mcp::relay_client::RelayClientError> for TestClientError {
    fn from(e: sprout_mcp::relay_client::RelayClientError) -> Self {
        use sprout_mcp::relay_client::RelayClientError as E;
        match e {
            E::WebSocket(e) => TestClientError::WebSocket(e),
            E::Json(e) => TestClientError::Json(e),
            E::Timeout => TestClientError::Timeout,
            E::ConnectionClosed => TestClientError::ConnectionClosed,
            E::UnexpectedMessage(m) => TestClientError::UnexpectedMessage(m),
            E::AuthFailed(m) => TestClientError::AuthFailed(m),
            E::NoAuthChallenge => TestClientError::NoAuthChallenge,
            other => TestClientError::UnexpectedMessage(other.to_string()),
        }
    }
}

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// WebSocket test client for integration testing against a running Sprout relay.
pub struct SproutTestClient {
    ws: WsStream,
    buffer: VecDeque<RelayMessage>,
    pending_challenge: Option<String>,
    relay_url: String,
}

impl SproutTestClient {
    /// Connects to the relay at `url` and performs NIP-42 authentication with `keys`.
    pub async fn connect(url: &str, keys: &Keys) -> Result<Self, TestClientError> {
        let mut client = Self::connect_unauthenticated(url).await?;
        client.authenticate(keys).await?;
        Ok(client)
    }

    /// Connects to the relay at `url` without performing authentication.
    pub async fn connect_unauthenticated(url: &str) -> Result<Self, TestClientError> {
        let parsed = url
            .parse::<url::Url>()
            .map_err(|e| TestClientError::Url(e.to_string()))?;

        let (ws, _response) = connect_async(parsed.as_str())
            .await
            .map_err(TestClientError::WebSocket)?;

        debug!("connected to relay at {url}");

        Ok(Self {
            ws,
            buffer: VecDeque::new(),
            pending_challenge: None,
            relay_url: url.to_string(),
        })
    }

    /// Performs NIP-42 authentication using `keys` against the connected relay.
    pub async fn authenticate(&mut self, keys: &Keys) -> Result<(), TestClientError> {
        let challenge = self.wait_for_auth_challenge(Duration::from_secs(5)).await?;

        let relay_url =
            RelayUrl::parse(&self.relay_url).map_err(|e| TestClientError::Url(e.to_string()))?;

        let auth_event = EventBuilder::auth(&challenge, relay_url).sign_with_keys(keys)?;
        let event_id = auth_event.id.to_hex();

        self.send_raw(&json!(["AUTH", auth_event])).await?;

        let ok = self.wait_for_ok(&event_id, Duration::from_secs(5)).await?;
        if !ok.accepted {
            return Err(TestClientError::AuthFailed(ok.message));
        }

        debug!("NIP-42 authentication successful");
        Ok(())
    }

    /// Sends a signed event to the relay and waits for the OK response.
    pub async fn send_event(&mut self, event: Event) -> Result<OkResponse, TestClientError> {
        let event_id = event.id.to_hex();
        self.send_raw(&json!(["EVENT", event])).await?;
        self.wait_for_ok(&event_id, Duration::from_secs(10)).await
    }

    /// Builds and sends a text message event to `channel_id` using the given `kind`.
    pub async fn send_text_message(
        &mut self,
        keys: &Keys,
        channel_id: &str,
        content: &str,
        kind: u16,
    ) -> Result<OkResponse, TestClientError> {
        let h_tag = Tag::parse(["h", channel_id])
            .map_err(|e| TestClientError::EventBuilder(e.to_string()))?;
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags([h_tag])
            .sign_with_keys(keys)?;
        self.send_event(event).await
    }

    /// Sends a REQ message to open a subscription with the given `sub_id` and `filters`.
    pub async fn subscribe(
        &mut self,
        sub_id: &str,
        filters: Vec<Filter>,
    ) -> Result<(), TestClientError> {
        let mut msg: Vec<Value> = Vec::with_capacity(2 + filters.len());
        msg.push(json!("REQ"));
        msg.push(json!(sub_id));
        for f in filters {
            msg.push(serde_json::to_value(&f)?);
        }
        self.send_raw(&Value::Array(msg)).await
    }

    /// Sends a CLOSE message to cancel the subscription identified by `sub_id`.
    pub async fn close_subscription(&mut self, sub_id: &str) -> Result<(), TestClientError> {
        self.send_raw(&json!(["CLOSE", sub_id])).await
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn recv_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, TestClientError> {
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }
        self.recv_one(timeout_dur).await
    }

    /// Collects all events for `sub_id` until EOSE is received, waiting up to `timeout_dur`.
    pub async fn collect_until_eose(
        &mut self,
        sub_id: &str,
        timeout_dur: Duration,
    ) -> Result<Vec<Event>, TestClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;
        let mut events = Vec::new();

        let old_buffer = std::mem::take(&mut self.buffer);
        let mut found_eose = false;
        for msg in old_buffer {
            if found_eose {
                self.buffer.push_back(msg);
                continue;
            }
            match msg {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } if subscription_id == sub_id => {
                    events.push(*event);
                }
                RelayMessage::Eose { subscription_id } if subscription_id == sub_id => {
                    found_eose = true;
                }
                other => self.buffer.push_back(other),
            }
        }
        if found_eose {
            return Ok(events);
        }

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(TestClientError::Timeout);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| TestClientError::Timeout)?
                .ok_or(TestClientError::ConnectionClosed)?
                .map_err(TestClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = parse_relay_message(&text)?;
                    match msg {
                        RelayMessage::Event {
                            subscription_id,
                            event,
                        } if subscription_id == sub_id => {
                            events.push(*event);
                        }
                        RelayMessage::Eose { subscription_id } if subscription_id == sub_id => {
                            return Ok(events);
                        }
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
                Message::Close(_) => return Err(TestClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    /// Closes the WebSocket connection gracefully.
    pub async fn disconnect(mut self) -> Result<(), TestClientError> {
        self.ws.close(None).await?;
        Ok(())
    }

    async fn send_raw(&mut self, value: &Value) -> Result<(), TestClientError> {
        let text = serde_json::to_string(value)?;
        debug!("→ relay: {text}");
        self.ws.send(Message::Text(text.into())).await?;
        Ok(())
    }

    async fn recv_one(&mut self, timeout_dur: Duration) -> Result<RelayMessage, TestClientError> {
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(msg);
        }

        loop {
            let raw = timeout(timeout_dur, self.ws.next())
                .await
                .map_err(|_| TestClientError::Timeout)?
                .ok_or(TestClientError::ConnectionClosed)?
                .map_err(TestClientError::WebSocket)?;

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
                Message::Close(_) => return Err(TestClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_auth_challenge(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<String, TestClientError> {
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
                return Err(TestClientError::NoAuthChallenge);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| TestClientError::NoAuthChallenge)?
                .ok_or(TestClientError::ConnectionClosed)?
                .map_err(TestClientError::WebSocket)?;

            match raw {
                Message::Text(text) => {
                    let msg = parse_relay_message(&text)?;
                    match msg {
                        RelayMessage::Auth { challenge } => return Ok(challenge),
                        other => self.buffer.push_back(other),
                    }
                }
                Message::Ping(data) => {
                    self.ws.send(Message::Pong(data)).await?;
                }
                Message::Close(_) => return Err(TestClientError::ConnectionClosed),
                _ => {}
            }
        }
    }

    async fn wait_for_ok(
        &mut self,
        event_id: &str,
        timeout_dur: Duration,
    ) -> Result<OkResponse, TestClientError> {
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
                return Err(TestClientError::Timeout);
            }

            let raw = timeout(remaining, self.ws.next())
                .await
                .map_err(|_| TestClientError::Timeout)?
                .ok_or(TestClientError::ConnectionClosed)?
                .map_err(TestClientError::WebSocket)?;

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
                Message::Close(_) => return Err(TestClientError::ConnectionClosed),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn parse_relay_messages() {
        struct Case {
            json: &'static str,
            check: fn(RelayMessage),
        }

        let cases = vec![
            Case {
                json: r#"["OK","abc123",true,""]"#,
                check: |msg| match msg {
                    RelayMessage::Ok(ok) => {
                        assert_eq!(ok.event_id, "abc123");
                        assert!(ok.accepted);
                        assert_eq!(ok.message, "");
                    }
                    _ => panic!("expected Ok"),
                },
            },
            Case {
                json: r#"["OK","def456",false,"blocked: not authorized"]"#,
                check: |msg| match msg {
                    RelayMessage::Ok(ok) => {
                        assert_eq!(ok.event_id, "def456");
                        assert!(!ok.accepted);
                        assert_eq!(ok.message, "blocked: not authorized");
                    }
                    _ => panic!("expected Ok"),
                },
            },
            Case {
                json: r#"["EOSE","sub1"]"#,
                check: |msg| match msg {
                    RelayMessage::Eose { subscription_id } => assert_eq!(subscription_id, "sub1"),
                    _ => panic!("expected Eose"),
                },
            },
            Case {
                json: r#"["NOTICE","hello from relay"]"#,
                check: |msg| match msg {
                    RelayMessage::Notice { message } => assert_eq!(message, "hello from relay"),
                    _ => panic!("expected Notice"),
                },
            },
            Case {
                json: r#"["AUTH","deadbeef1234"]"#,
                check: |msg| match msg {
                    RelayMessage::Auth { challenge } => assert_eq!(challenge, "deadbeef1234"),
                    _ => panic!("expected Auth"),
                },
            },
            Case {
                json: r#"["CLOSED","sub2","auth-required: must authenticate"]"#,
                check: |msg| match msg {
                    RelayMessage::Closed {
                        subscription_id,
                        message,
                    } => {
                        assert_eq!(subscription_id, "sub2");
                        assert_eq!(message, "auth-required: must authenticate");
                    }
                    _ => panic!("expected Closed"),
                },
            },
        ];

        for case in cases {
            let msg = parse_relay_message(case.json).expect(case.json);
            (case.check)(msg);
        }
    }

    #[test]
    fn parse_unknown_message_type_errors() {
        let result = parse_relay_message(r#"["UNKNOWN","data"]"#);
        assert!(result.is_err());
    }

    #[test]
    fn auth_event_has_relay_and_challenge_tags() {
        let keys = Keys::generate();
        let relay_url: Url = "ws://localhost:3000".parse().unwrap();
        let event = EventBuilder::auth("test-challenge", relay_url)
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(event.kind, Kind::Authentication);

        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
            .collect();

        assert!(
            tags.iter().any(|t| t.len() >= 2 && t[0] == "relay"),
            "missing relay tag"
        );
        assert!(
            tags.iter()
                .any(|t| t.len() >= 2 && t[0] == "challenge" && t[1] == "test-challenge"),
            "missing challenge tag"
        );
    }

    #[test]
    fn text_event_carries_h_tag() {
        let keys = Keys::generate();
        let channel_id = "my-channel-123";
        let h_tag = Tag::parse(["h", channel_id]).unwrap();
        let event = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([h_tag])
            .sign_with_keys(&keys)
            .unwrap();

        assert_eq!(event.kind, Kind::Custom(9));
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
            .collect();

        assert!(
            tags.iter()
                .any(|t| t.len() >= 2 && t[0] == "h" && t[1] == channel_id),
            "missing h tag"
        );
    }
}
