//! Bridge wire protocol v1 — JSON frames over a carrier stream.
//!
//! The remote transport speaks a deliberately small protocol: every frame is
//! a JSON object with a `type` field, and the payload of the `event` frames
//! is the *same signed Nostr event JSON* that flows over the relay
//! WebSocket. The carrier is either a WebSocket (one frame per text
//! message) or a Unix domain socket (one frame per LF-terminated line).
//! See `PROTOCOL.md` in this crate for the full specification and a
//! bridge-implementor's guide.
//!
//! Forward compatibility: receivers ignore frames whose `type` they do not
//! recognize and ignore unknown fields inside known frames. Breaking changes
//! bump [`PROTOCOL_VERSION`], negotiated in the `hello`/`hello_ack`
//! handshake.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{SignedEvent, Subscription, TransportError};

/// Bridge protocol version negotiated in the `hello`/`hello_ack` handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum size of a single frame (WebSocket message or socket line),
/// matching the relay's default inbound frame cap
/// (`DEFAULT_MAX_FRAME_BYTES` in `buzz-relay`), so any event the relay
/// accepts also fits in one bridge frame.
pub const MAX_FRAME_BYTES: usize = 512 * 1024;

/// Frames sent by the Buzz client to the bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// First frame on every connection: protocol version + client identity.
    Hello {
        /// Protocol version the client speaks ([`PROTOCOL_VERSION`]).
        version: u32,
        /// Hex-encoded public key the client publishes events as.
        pubkey: String,
        /// Optional bearer token. On WebSocket carriers the same token also
        /// travels as an `Authorization` header; on raw-socket carriers
        /// (which have no headers) this field is the only place it appears.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
    /// Declare interest in a channel's events (advisory — see
    /// [`Subscription`]).
    Subscribe {
        /// The channel whose events should be delivered.
        channel_id: Uuid,
        /// Event kinds to deliver. Absent/`null` = all kinds; an explicit
        /// empty list matches nothing (the NIP-01 edge case).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kinds: Option<Vec<u32>>,
        /// Only deliver events that `p`-tag the `hello` pubkey.
        #[serde(default)]
        require_mention: bool,
        /// Replay stored events created at or after this Unix timestamp.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay_since: Option<u64>,
    },
    /// Withdraw interest in a channel's events.
    Unsubscribe {
        /// The channel to stop delivering.
        channel_id: Uuid,
    },
    /// A signed event published by the client.
    Event {
        /// The signed event (see [`SignedEvent`] for the JSON shape).
        event: Box<SignedEvent>,
    },
}

impl ClientFrame {
    /// Build the `subscribe` frame for a [`Subscription`].
    pub fn subscribe(subscription: &Subscription) -> Self {
        Self::Subscribe {
            channel_id: subscription.channel_id,
            kinds: subscription.kinds.clone(),
            require_mention: subscription.require_mention,
            replay_since: subscription.replay_since,
        }
    }
}

/// Frames sent by the bridge to the Buzz client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeFrame {
    /// Handshake acknowledgment. Must be the bridge's first frame.
    HelloAck {
        /// Protocol version the bridge speaks. Must match the client's.
        version: u32,
    },
    /// An inbound signed event, tagged with its channel.
    Event {
        /// Which channel this event belongs to.
        channel_id: Uuid,
        /// The signed event (see [`SignedEvent`] for the JSON shape).
        event: Box<SignedEvent>,
    },
    /// Optional acknowledgment of a client-published event.
    Ok {
        /// Hex ID of the event being acknowledged.
        event_id: String,
        /// Whether the bridge accepted the event.
        accepted: bool,
        /// Human-readable detail (empty when accepted).
        #[serde(default)]
        message: String,
    },
    /// Optional end-of-replay marker after a `subscribe` with `replay_since`.
    Eose {
        /// The channel whose replay finished.
        channel_id: Uuid,
    },
    /// Human-readable diagnostic from the bridge.
    Notice {
        /// The diagnostic text.
        message: String,
    },
}

/// Frame `type` values this version knows in the client → bridge direction.
const CLIENT_FRAME_TYPES: &[&str] = &["hello", "subscribe", "unsubscribe", "event"];

/// Frame `type` values this version knows in the bridge → client direction.
const BRIDGE_FRAME_TYPES: &[&str] = &["hello_ack", "event", "ok", "eose", "notice"];

/// Encode a client frame as a JSON text frame.
pub fn encode_client_frame(frame: &ClientFrame) -> Result<String, TransportError> {
    serde_json::to_string(frame).map_err(|e| TransportError::Codec(e.to_string()))
}

/// Encode a bridge frame as a JSON text frame (for bridge implementations
/// and tests).
pub fn encode_bridge_frame(frame: &BridgeFrame) -> Result<String, TransportError> {
    serde_json::to_string(frame).map_err(|e| TransportError::Codec(e.to_string()))
}

/// Parse a bridge → client frame.
///
/// Returns `Ok(None)` for frames whose `type` this version does not know —
/// they must be ignored for forward compatibility. Malformed JSON or a known
/// `type` with an invalid body is an error.
pub fn parse_bridge_frame(text: &str) -> Result<Option<BridgeFrame>, TransportError> {
    parse_frame(text, BRIDGE_FRAME_TYPES)
}

/// Parse a client → bridge frame (for bridge implementations and tests).
///
/// Same unknown-`type` tolerance as [`parse_bridge_frame`].
pub fn parse_client_frame(text: &str) -> Result<Option<ClientFrame>, TransportError> {
    parse_frame(text, CLIENT_FRAME_TYPES)
}

fn parse_frame<T: serde::de::DeserializeOwned>(
    text: &str,
    known_types: &[&str],
) -> Result<Option<T>, TransportError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| TransportError::Codec(format!("invalid JSON frame: {e}")))?;
    let Some(frame_type) = value
        .get("type")
        .and_then(|t| t.as_str())
        .map(str::to_owned)
    else {
        return Err(TransportError::Codec("frame missing \"type\" field".into()));
    };
    if !known_types.contains(&frame_type.as_str()) {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| TransportError::Codec(format!("invalid {frame_type} frame: {e}")))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind};

    use super::*;

    fn signed_event() -> SignedEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "hello")
            .sign_with_keys(&keys)
            .unwrap();
        SignedEvent::from_nostr(&event).unwrap()
    }

    #[test]
    fn hello_round_trips() {
        let frame = ClientFrame::Hello {
            version: PROTOCOL_VERSION,
            pubkey: "ab".repeat(32),
            token: None,
        };
        let text = encode_client_frame(&frame).unwrap();
        assert!(text.contains("\"type\":\"hello\""));
        assert!(!text.contains("token"), "absent token must be omitted");
        assert_eq!(parse_client_frame(&text).unwrap(), Some(frame));

        let with_token = ClientFrame::Hello {
            version: PROTOCOL_VERSION,
            pubkey: "ab".repeat(32),
            token: Some("sekrit".into()),
        };
        let text = encode_client_frame(&with_token).unwrap();
        assert_eq!(parse_client_frame(&text).unwrap(), Some(with_token));
    }

    #[test]
    fn subscribe_frame_matches_subscription() {
        let sub = Subscription {
            channel_id: Uuid::new_v4(),
            kinds: Some(vec![9, 40002]),
            require_mention: true,
            replay_since: Some(1_700_000_000),
        };
        let text = encode_client_frame(&ClientFrame::subscribe(&sub)).unwrap();
        match parse_client_frame(&text).unwrap() {
            Some(ClientFrame::Subscribe {
                channel_id,
                kinds,
                require_mention,
                replay_since,
            }) => {
                assert_eq!(channel_id, sub.channel_id);
                assert_eq!(kinds, sub.kinds);
                assert!(require_mention);
                assert_eq!(replay_since, sub.replay_since);
            }
            other => panic!("expected subscribe frame, got {other:?}"),
        }
    }

    #[test]
    fn subscribe_optional_fields_default() {
        let channel_id = Uuid::new_v4();
        let text = format!(r#"{{"type":"subscribe","channel_id":"{channel_id}"}}"#);
        match parse_client_frame(&text).unwrap() {
            Some(ClientFrame::Subscribe {
                kinds,
                require_mention,
                replay_since,
                ..
            }) => {
                assert_eq!(kinds, None);
                assert!(!require_mention);
                assert_eq!(replay_since, None);
            }
            other => panic!("expected subscribe frame, got {other:?}"),
        }
    }

    #[test]
    fn event_frames_carry_the_wire_event_json() {
        let event = signed_event();
        let text = encode_bridge_frame(&BridgeFrame::Event {
            channel_id: Uuid::new_v4(),
            event: Box::new(event.clone()),
        })
        .unwrap();
        // The embedded event is the unmodified signed-event JSON.
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["event"],
            serde_json::to_value(&event).unwrap(),
            "event payload must be the unmodified signed-event JSON"
        );
        match parse_bridge_frame(&text).unwrap() {
            Some(BridgeFrame::Event { event: parsed, .. }) => assert_eq!(*parsed, event),
            other => panic!("expected event frame, got {other:?}"),
        }
    }

    #[test]
    fn unknown_frame_type_is_ignored() {
        let text = r#"{"type":"totally_new_frame","payload":42}"#;
        assert_eq!(parse_bridge_frame(text).unwrap(), None);
        assert_eq!(parse_client_frame(text).unwrap(), None);
    }

    #[test]
    fn unknown_fields_in_known_frames_are_ignored() {
        let text = r#"{"type":"hello_ack","version":1,"server":"slack-bridge/0.1"}"#;
        assert_eq!(
            parse_bridge_frame(text).unwrap(),
            Some(BridgeFrame::HelloAck { version: 1 })
        );
    }

    #[test]
    fn missing_type_is_an_error() {
        assert!(matches!(
            parse_bridge_frame(r#"{"version":1}"#),
            Err(TransportError::Codec(_))
        ));
    }

    #[test]
    fn malformed_known_frame_is_an_error() {
        assert!(matches!(
            parse_bridge_frame(r#"{"type":"hello_ack"}"#),
            Err(TransportError::Codec(_))
        ));
    }

    #[test]
    fn ok_message_defaults_to_empty() {
        let text = r#"{"type":"ok","event_id":"abc","accepted":true}"#;
        match parse_bridge_frame(text).unwrap() {
            Some(BridgeFrame::Ok {
                accepted, message, ..
            }) => {
                assert!(accepted);
                assert!(message.is_empty());
            }
            other => panic!("expected ok frame, got {other:?}"),
        }
    }
}
