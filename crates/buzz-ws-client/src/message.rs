use nostr::{Event, EventBuilder, Keys, RelayUrl, Tag};
use serde_json::value::RawValue;

use crate::error::WsClientError;

/// Maximum UTF-8 byte length accepted for a relay AUTH challenge.
pub const MAX_AUTH_CHALLENGE_BYTES: usize = 1024;

pub(crate) fn validate_auth_challenge(challenge: &str) -> Result<(), WsClientError> {
    let size = challenge.len();
    if size > MAX_AUTH_CHALLENGE_BYTES {
        return Err(WsClientError::AuthChallengeTooLarge {
            size,
            max: MAX_AUTH_CHALLENGE_BYTES,
        });
    }
    Ok(())
}

/// A message received from a Nostr relay.
#[derive(Debug, Clone)]
pub enum RelayMessage {
    /// An event matching an active subscription.
    Event {
        /// The subscription ID this event belongs to.
        subscription_id: String,
        /// The Nostr event payload.
        event: Box<Event>,
        /// The event object's exact JSON bytes from the relay text frame.
        raw_event_json: Box<str>,
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
    /// A NIP-45 COUNT response.
    Count {
        /// The subscription ID this count belongs to.
        subscription_id: String,
        /// The number of matching events.
        count: u64,
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
    let arr: Vec<Box<RawValue>> = serde_json::from_str(text)?;

    let msg_type = arr
        .first()
        .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
        .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?;

    match msg_type.as_str() {
        "EVENT" => {
            let sub_id = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let raw_event = arr
                .get(2)
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?;
            let event: Event = serde_json::from_str(raw_event.get()).map_err(|error| {
                WsClientError::InvalidEvent {
                    raw_event_json: raw_event.get().to_string().into_boxed_str(),
                    message: error.to_string(),
                }
            })?;
            Ok(RelayMessage::Event {
                subscription_id: sub_id,
                event: Box::new(event),
                raw_event_json: raw_event.get().to_string().into_boxed_str(),
            })
        }
        "OK" => {
            let event_id = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let accepted = arr
                .get(2)
                .and_then(|value| serde_json::from_str::<bool>(value.get()).ok())
                .unwrap_or(false);
            let message = arr
                .get(3)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .unwrap_or_default();
            Ok(RelayMessage::Ok(OkResponse {
                event_id,
                accepted,
                message,
            }))
        }
        "EOSE" => {
            let sub_id = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Eose {
                subscription_id: sub_id,
            })
        }
        "CLOSED" => {
            let sub_id = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let message = arr
                .get(2)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .unwrap_or_default();
            Ok(RelayMessage::Closed {
                subscription_id: sub_id,
                message,
            })
        }
        "NOTICE" => {
            let message = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .unwrap_or_default();
            Ok(RelayMessage::Notice { message })
        }
        "AUTH" => {
            let challenge = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            validate_auth_challenge(&challenge)?;
            Ok(RelayMessage::Auth { challenge })
        }
        "COUNT" => {
            let sub_id = arr
                .get(1)
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let count_object: serde_json::Value = serde_json::from_str(
                arr.get(2)
                    .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?
                    .get(),
            )?;
            let count = count_object
                .get("count")
                .and_then(|count| count.as_u64())
                .ok_or_else(|| WsClientError::UnexpectedMessage(text.to_string()))?;
            Ok(RelayMessage::Count {
                subscription_id: sub_id,
                count,
            })
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
    // Keep this check at the signing boundary as well as relay-message ingress:
    // callers may invoke this public helper without parsing an AUTH frame first.
    validate_auth_challenge(challenge)?;
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
    use nostr::{EventBuilder, JsonUtil, Kind};

    use super::*;

    #[test]
    fn event_message_retains_exact_raw_object_bytes() {
        let event = EventBuilder::new(Kind::TextNote, "raw relay bytes")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let raw_event = event.as_json().replacen(',', ",\n  ", 1);
        let frame = format!(r#"["EVENT","subscription",{raw_event}]"#);

        match parse_relay_message(&frame).unwrap() {
            RelayMessage::Event {
                event: parsed,
                raw_event_json,
                ..
            } => {
                assert_eq!(*parsed, event);
                assert_eq!(raw_event_json.as_ref(), raw_event);
            }
            other => panic!("expected EVENT, got {other:?}"),
        }
    }

    #[test]
    fn auth_challenge_limit_is_measured_in_utf8_bytes_at_parse_ingress() {
        let exact = "x".repeat(MAX_AUTH_CHALLENGE_BYTES);
        let oversized = "x".repeat(MAX_AUTH_CHALLENGE_BYTES + 1);

        assert!(matches!(
            parse_relay_message(&serde_json::json!(["AUTH", exact]).to_string()),
            Ok(RelayMessage::Auth { .. })
        ));
        assert!(matches!(
            parse_relay_message(&serde_json::json!(["AUTH", oversized]).to_string()),
            Err(WsClientError::AuthChallengeTooLarge {
                size: 1025,
                max: 1024
            })
        ));
    }

    #[test]
    fn auth_challenge_limit_is_rechecked_before_signing() {
        let keys = Keys::generate();
        let exact = "x".repeat(MAX_AUTH_CHALLENGE_BYTES);
        let oversized = "x".repeat(MAX_AUTH_CHALLENGE_BYTES + 1);

        assert!(build_auth_event(&exact, "wss://relay.example", &keys, None).is_ok());
        assert!(matches!(
            build_auth_event(&oversized, "wss://relay.example", &keys, None),
            Err(WsClientError::AuthChallengeTooLarge {
                size: 1025,
                max: 1024
            })
        ));
    }
}
