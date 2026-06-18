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
