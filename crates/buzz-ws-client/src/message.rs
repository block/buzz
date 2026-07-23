use nostr::{Event, EventBuilder, Keys, RelayUrl, Tag};
use serde_json::Value;

use crate::error::WsClientError;

/// A message received from a Nostr relay.
#[derive(Debug, Clone)]
pub enum RelayMessage {
    /// An event matching an active subscription.
    Event {
        /// The subscription ID this event belongs to.
        subscription_id: String,
        /// The Nostr event payload.
        event: Box<Event>,
    },
    /// Acknowledgement of a published event.
    Ok(OkResponse),
    /// End-of-stored-events marker for a subscription.
    Eose {
        /// The subscription ID that has reached end-of-stored-events.
        subscription_id: String,
    },
    /// The relay closed a subscription, usually with an error.
    Closed {
        /// The subscription ID that was closed.
        subscription_id: String,
        /// Human-readable reason for the closure.
        message: String,
    },
    /// A human-readable notice from the relay.
    Notice {
        /// The notice text.
        message: String,
    },
    /// A NIP-42 authentication challenge from the relay.
    Auth {
        /// The challenge string to sign.
        challenge: String,
    },
}

/// The relay's response to a published event (NIP-01 `OK` message).
#[derive(Debug, Clone)]
pub struct OkResponse {
    /// Hex-encoded ID of the event that was acknowledged.
    pub event_id: String,
    /// Whether the relay accepted the event.
    pub accepted: bool,
    /// Human-readable reason string (empty when accepted without comment).
    pub message: String,
}

/// Parse a raw relay text frame into a typed [`RelayMessage`].
#[allow(clippy::result_large_err)]
pub fn parse_relay_message(text: &str) -> Result<RelayMessage, WsClientError> {
    let arr: Vec<Value> = serde_json::from_str(text)?;

    let msg_type = arr
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?;

    match msg_type {
        "EVENT" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let event: Event = serde_json::from_value(
                arr.get(2)
                    .cloned()
                    .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?,
            )?;
            Ok(RelayMessage::Event {
                subscription_id: sub_id,
                event: Box::new(event),
            })
        }
        "OK" => {
            let event_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            let message = arr
                .get(3)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Ok(OkResponse {
                event_id,
                accepted,
                message,
            }))
        }
        "EOSE" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Eose {
                subscription_id: sub_id,
            })
        }
        "CLOSED" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let message = arr
                .get(2)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Closed {
                subscription_id: sub_id,
                message,
            })
        }
        "NOTICE" => {
            let message = arr
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Notice { message })
        }
        "AUTH" => {
            let challenge = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Auth { challenge })
        }
        other => Err(WsClientError::UnexpectedMessage(format!(
            "unknown message type: {other}"
        ))),
    }
}

/// Builds a NIP-42 AUTH event, optionally injecting a NIP-OA auth tag.
///
/// The `auth_tag` parameter allows callers to attach a workspace-scoped
/// authorization tag (e.g. `["auth", "<token>"]`) alongside the standard
/// relay and challenge tags required by NIP-42.
pub fn build_auth_event(
    challenge: &str,
    relay_url: &str,
    keys: &Keys,
    auth_tag: Option<&Tag>,
) -> Result<Event, WsClientError> {
    let url = RelayUrl::parse(relay_url).map_err(|e| WsClientError::Url(e.to_string()))?;
    let builder = EventBuilder::auth(challenge, url);
    let builder = if let Some(tag) = auth_tag {
        builder.tags([tag.clone()])
    } else {
        builder
    };
    builder
        .sign_with_keys(keys)
        .map_err(|e| WsClientError::EventBuilder(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Kind;
    use serde_json::json;

    fn test_keys() -> Keys {
        Keys::generate()
    }

    /// Build a real signed Nostr event so EVENT frames carry a valid payload.
    fn make_signed_event(keys: &Keys) -> Event {
        EventBuilder::new(Kind::TextNote, "test")
            .sign_with_keys(keys)
            .expect("signing should succeed")
    }

    #[test]
    fn event_frame_parses_subscription_and_payload() {
        let keys = test_keys();
        let event = make_signed_event(&keys);
        let expected_id = event.id;
        let frame = json!(["EVENT", "sub-1", event]).to_string();

        match parse_relay_message(&frame).expect("parse EVENT frame") {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                assert_eq!(subscription_id, "sub-1");
                assert_eq!(event.id, expected_id);
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn event_frame_without_payload_is_rejected() {
        let result = parse_relay_message(r#"["EVENT","sub-1"]"#);
        assert!(matches!(result, Err(WsClientError::UnexpectedMessage(_))));
    }

    #[test]
    fn event_frame_with_malformed_payload_is_json_error() {
        let result = parse_relay_message(r#"["EVENT","sub-1",{"garbage":true}]"#);
        assert!(matches!(result, Err(WsClientError::Json(_))));
    }

    #[test]
    fn ok_frame_parses_all_fields() {
        match parse_relay_message(r#"["OK","abc123",true,"duplicate: already stored"]"#)
            .expect("parse OK frame")
        {
            RelayMessage::Ok(ok) => {
                assert_eq!(ok.event_id, "abc123");
                assert!(ok.accepted);
                assert_eq!(ok.message, "duplicate: already stored");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn ok_frame_without_flags_defaults_to_rejected_with_empty_message() {
        match parse_relay_message(r#"["OK","abc123"]"#).expect("parse short OK frame") {
            RelayMessage::Ok(ok) => {
                assert_eq!(ok.event_id, "abc123");
                assert!(!ok.accepted);
                assert_eq!(ok.message, "");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn eose_frame_parses_subscription() {
        match parse_relay_message(r#"["EOSE","sub-1"]"#).expect("parse EOSE frame") {
            RelayMessage::Eose { subscription_id } => assert_eq!(subscription_id, "sub-1"),
            other => panic!("expected Eose, got {other:?}"),
        }
    }

    #[test]
    fn eose_frame_without_subscription_is_rejected() {
        let result = parse_relay_message(r#"["EOSE"]"#);
        assert!(matches!(result, Err(WsClientError::UnexpectedMessage(_))));
    }

    #[test]
    fn closed_frame_parses_reason() {
        match parse_relay_message(r#"["CLOSED","sub-1","error: shutting down"]"#)
            .expect("parse CLOSED frame")
        {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, "sub-1");
                assert_eq!(message, "error: shutting down");
            }
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    #[test]
    fn closed_frame_without_reason_defaults_to_empty() {
        match parse_relay_message(r#"["CLOSED","sub-1"]"#).expect("parse short CLOSED frame") {
            RelayMessage::Closed { message, .. } => assert_eq!(message, ""),
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    #[test]
    fn notice_frame_parses_message() {
        match parse_relay_message(r#"["NOTICE","rate limited"]"#).expect("parse NOTICE frame") {
            RelayMessage::Notice { message } => assert_eq!(message, "rate limited"),
            other => panic!("expected Notice, got {other:?}"),
        }
    }

    #[test]
    fn auth_frame_parses_challenge() {
        match parse_relay_message(r#"["AUTH","challenge-string"]"#).expect("parse AUTH frame") {
            RelayMessage::Auth { challenge } => assert_eq!(challenge, "challenge-string"),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[test]
    fn auth_frame_without_challenge_is_rejected() {
        let result = parse_relay_message(r#"["AUTH"]"#);
        assert!(matches!(result, Err(WsClientError::UnexpectedMessage(_))));
    }

    #[test]
    fn unknown_message_type_is_rejected() {
        match parse_relay_message(r#"["REQ","sub-1",{}]"#) {
            Err(WsClientError::UnexpectedMessage(msg)) => {
                assert!(msg.contains("unknown message type: REQ"));
            }
            other => panic!("expected UnexpectedMessage, got {other:?}"),
        }
    }

    #[test]
    fn non_json_input_is_json_error() {
        let result = parse_relay_message("not json at all");
        assert!(matches!(result, Err(WsClientError::Json(_))));
    }

    #[test]
    fn json_object_input_is_json_error() {
        let result = parse_relay_message(r#"{"type":"EVENT"}"#);
        assert!(matches!(result, Err(WsClientError::Json(_))));
    }

    #[test]
    fn empty_array_is_rejected() {
        let result = parse_relay_message("[]");
        assert!(matches!(result, Err(WsClientError::UnexpectedMessage(_))));
    }

    #[test]
    fn non_string_message_type_is_rejected() {
        let result = parse_relay_message("[42]");
        assert!(matches!(result, Err(WsClientError::UnexpectedMessage(_))));
    }

    #[test]
    fn auth_event_is_signed_authentication_kind_with_nip42_tags() {
        let keys = test_keys();
        let event = build_auth_event("test-challenge", "wss://relay.example.com", &keys, None)
            .expect("build auth event");

        assert_eq!(event.kind, Kind::Authentication);
        event.verify().expect("valid signature");

        let value = serde_json::to_value(&event).expect("serialize auth event");
        let tags = value["tags"].as_array().expect("tags array");
        assert!(
            tags.contains(&json!(["challenge", "test-challenge"])),
            "missing challenge tag in {tags:?}"
        );
        assert!(
            tags.iter().any(|t| t[0] == "relay"),
            "missing relay tag in {tags:?}"
        );
    }

    #[test]
    fn auth_event_includes_optional_auth_tag() {
        let keys = test_keys();
        let auth_tag = Tag::parse(["auth", "token-123"]).expect("parse auth tag");
        let event = build_auth_event(
            "test-challenge",
            "wss://relay.example.com",
            &keys,
            Some(&auth_tag),
        )
        .expect("build auth event");

        let value = serde_json::to_value(&event).expect("serialize auth event");
        let tags = value["tags"].as_array().expect("tags array");
        assert!(
            tags.contains(&json!(["auth", "token-123"])),
            "missing auth tag in {tags:?}"
        );
    }

    #[test]
    fn auth_event_rejects_invalid_relay_url() {
        let keys = test_keys();
        let result = build_auth_event("test-challenge", "not a url", &keys, None);
        assert!(matches!(result, Err(WsClientError::Url(_))));
    }
}
