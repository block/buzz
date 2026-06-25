#![deny(unsafe_code)]
#![warn(missing_docs)]

//! Minimal NIP-01 WebSocket test client for the Buzz relay.

use std::time::Duration;

use nostr::{Event, EventBuilder, Filter, Keys, Kind, Tag};
use serde_json::{json, Value};
use thiserror::Error;
use tracing::debug;

use buzz_ws_client::NostrWsConnection;
pub use buzz_ws_client::{parse_relay_message, OkResponse, RelayMessage, WsClientError};

/// Errors returned by [`BuzzTestClient`] operations.
#[derive(Debug, Error)]
pub enum TestClientError {
    /// A WebSocket transport error occurred.
    #[error("WebSocket error: {0}")]
    WebSocket(tokio_tungstenite::tungstenite::Error),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(serde_json::Error),

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

impl From<WsClientError> for TestClientError {
    fn from(e: WsClientError) -> Self {
        match e {
            WsClientError::WebSocket(e) => TestClientError::WebSocket(e),
            WsClientError::Json(e) => TestClientError::Json(e),
            WsClientError::EventBuilder(s) => TestClientError::EventBuilder(s),
            WsClientError::Url(s) => TestClientError::Url(s),
            WsClientError::Timeout => TestClientError::Timeout,
            WsClientError::ConnectionClosed => TestClientError::ConnectionClosed,
            WsClientError::UnexpectedMessage(s) => TestClientError::UnexpectedMessage(s),
            WsClientError::AuthFailed(s) => TestClientError::AuthFailed(s),
            WsClientError::EventRejected(s) => TestClientError::EventRejected(s),
            WsClientError::NoAuthChallenge => TestClientError::NoAuthChallenge,
        }
    }
}

impl From<nostr::event::builder::Error> for TestClientError {
    fn from(e: nostr::event::builder::Error) -> Self {
        TestClientError::EventBuilder(e.to_string())
    }
}

/// WebSocket test client for integration testing against a running Buzz relay.
pub struct BuzzTestClient {
    inner: NostrWsConnection,
}

impl BuzzTestClient {
    /// Connects to the relay at `url` and performs NIP-42 authentication with `keys`.
    pub async fn connect(url: &str, keys: &Keys) -> Result<Self, TestClientError> {
        let mut client = Self::connect_unauthenticated(url).await?;
        client.authenticate(keys).await?;
        Ok(client)
    }

    /// Connects to the relay at `url` without performing authentication.
    pub async fn connect_unauthenticated(url: &str) -> Result<Self, TestClientError> {
        let inner = NostrWsConnection::connect(url).await?;
        debug!("connected to relay at {url}");
        Ok(Self { inner })
    }

    /// Performs NIP-42 authentication using `keys` against the connected relay.
    pub async fn authenticate(&mut self, keys: &Keys) -> Result<(), TestClientError> {
        self.inner.authenticate(keys, None).await?;
        Ok(())
    }

    /// Sends a signed event to the relay and waits for the OK response.
    pub async fn send_event(&mut self, event: Event) -> Result<OkResponse, TestClientError> {
        Ok(self.inner.send_event(event).await?)
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
            msg.push(serde_json::to_value(&f).map_err(WsClientError::Json)?);
        }
        self.inner.send_raw(&Value::Array(msg)).await?;
        Ok(())
    }

    /// Sends a CLOSE message to cancel the subscription identified by `sub_id`.
    pub async fn close_subscription(&mut self, sub_id: &str) -> Result<(), TestClientError> {
        self.inner.send_raw(&json!(["CLOSE", sub_id])).await?;
        Ok(())
    }

    /// Sends a raw JSON value as a WebSocket text frame.
    pub async fn send_raw(&mut self, value: &serde_json::Value) -> Result<(), TestClientError> {
        self.inner.send_raw(value).await?;
        Ok(())
    }

    /// Receives the next relay message, waiting up to `timeout_dur`.
    pub async fn recv_event(
        &mut self,
        timeout_dur: Duration,
    ) -> Result<RelayMessage, TestClientError> {
        Ok(self.inner.next_event(timeout_dur).await?)
    }

    /// Collects all events for `sub_id` until EOSE is received, waiting up to `timeout_dur`.
    pub async fn collect_until_eose(
        &mut self,
        sub_id: &str,
        timeout_dur: Duration,
    ) -> Result<Vec<Event>, TestClientError> {
        let deadline = tokio::time::Instant::now() + timeout_dur;
        let mut events = Vec::new();

        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);

            if remaining.is_zero() {
                return Err(TestClientError::Timeout);
            }

            match self.inner.next_event(remaining).await? {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } if subscription_id == sub_id => {
                    events.push(*event);
                }
                RelayMessage::Eose { subscription_id } if subscription_id == sub_id => {
                    return Ok(events);
                }
                _ => {}
            }
        }
    }

    /// Closes the WebSocket connection gracefully.
    pub async fn disconnect(self) -> Result<(), TestClientError> {
        self.inner.disconnect().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag};

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
        let relay_url: RelayUrl = "ws://localhost:3000".parse().unwrap();
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
