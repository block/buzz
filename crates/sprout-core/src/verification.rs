//! `verify_event()` is CPU-bound (Schnorr). In async contexts call it via
//! `tokio::task::spawn_blocking` — never directly on an async task.

use nostr::{Event, EventId};

use crate::error::VerificationError;

/// Verifies the event ID hash and Schnorr signature.
///
/// CPU-bound — call via `tokio::task::spawn_blocking` in async contexts.
pub fn verify_event(event: &Event) -> Result<(), VerificationError> {
    if !event.verify_id() {
        let computed = EventId::new(
            &event.pubkey,
            &event.created_at,
            &event.kind,
            &event.tags,
            &event.content,
        )
        .to_hex();
        return Err(VerificationError::InvalidId {
            computed,
            got: event.id.to_hex(),
        });
    }

    if !event.verify_signature() {
        return Err(VerificationError::InvalidSignature);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind};

    fn make_valid_event() -> Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::TextNote, "test content")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign")
    }

    #[test]
    fn rejects_tampered_id() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "original")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).expect("parse");
        json["content"] = serde_json::Value::String("tampered".to_string());
        let tampered = Event::from_json(json.to_string()).expect("parse");
        assert!(matches!(
            verify_event(&tampered),
            Err(VerificationError::InvalidId { .. })
        ));
    }

    #[test]
    fn rejects_tampered_signature() {
        let event = make_valid_event();
        let mut json: serde_json::Value = serde_json::from_str(&event.as_json()).expect("parse");
        json["sig"] = serde_json::Value::String("0".repeat(128));
        let tampered = Event::from_json(json.to_string()).expect("parse");
        assert!(verify_event(&tampered).is_err());
    }
}
