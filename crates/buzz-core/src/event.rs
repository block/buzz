//! Relay-side event wrapper.
//!
//! [`StoredEvent`] wraps a [`nostr::Event`] with relay-assigned metadata
//! (receive time, channel scope, verification status).

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A Nostr event with relay-assigned metadata.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    /// The underlying Nostr event.
    pub event: nostr::Event,
    /// Wall-clock time the relay received this event.
    pub received_at: DateTime<Utc>,
    /// Channel scope; `None` for global/DM events.
    pub channel_id: Option<Uuid>,
    verified: bool,
}

impl StoredEvent {
    /// Creates a new `StoredEvent` with `received_at` set to now and `verified = false`.
    pub fn new(event: nostr::Event, channel_id: Option<Uuid>) -> Self {
        Self {
            event,
            received_at: Utc::now(),
            channel_id,
            verified: false,
        }
    }

    /// Returns whether this event's signature has been verified.
    pub fn is_verified(&self) -> bool {
        self.verified
    }

    /// Creates a `StoredEvent` with an explicit `received_at` timestamp and verification status.
    pub fn with_received_at(
        event: nostr::Event,
        received_at: DateTime<Utc>,
        channel_id: Option<Uuid>,
        verified: bool,
    ) -> Self {
        Self {
            event,
            received_at,
            channel_id,
            verified,
        }
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, JsonUtil, Keys, Kind};

    fn make_event() -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::TextNote, "hello buzz")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign")
    }

    #[test]
    fn tampered_signature_fails_verify() {
        let event = make_event();
        let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).expect("parse");
        json["sig"] = serde_json::Value::String("0".repeat(128));
        let tampered = nostr::Event::from_json(json.to_string()).expect("parse");
        assert!(tampered.verify_id());
        assert!(!tampered.verify_signature());
    }
}
