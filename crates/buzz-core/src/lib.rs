#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `sprout-core` — zero-I/O foundation types for the Sprout relay.
//!
//! Provides [`StoredEvent`], filter matching, kind constants, and event
//! verification. All other Sprout crates depend on this one.

/// Channel and membership enums shared across crates.
pub mod channel;
/// NIP-AE Agent Engrams — slug grammar, conversation key, d-tag derivation,
/// body parse/serialize, envelope build/validate, head selection.
pub mod engram;
/// Relay-side error types.
pub mod error;
/// Relay-side event wrapper with verification tracking.
pub mod event;
/// NIP-01 subscription filter matching.
pub mod filter;
/// Git permission types — ref patterns, protection rules, policy evaluation.
pub mod git_perms;
/// Sprout kind number registry — custom event type constants.
pub mod kind;
/// Network utilities — SSRF-safe IP classification.
pub mod network;
/// Agent observer frame helpers.
pub mod observer;
/// NIP-AB device pairing — crypto primitives, message types, and errors.
pub mod pairing;
/// Presence status types shared across crates.
pub mod presence;
/// Schnorr signature and event ID verification.
pub mod verification;

pub use error::VerificationError;
pub use event::StoredEvent;
pub use nostr::{Event, EventId, Filter, Keys, Kind, PublicKey};
pub use presence::PresenceStatus;
pub use verification::verify_event;

#[cfg(any(test, feature = "test-utils"))]
/// Test helper utilities for creating events and stored events.
pub mod test_helpers {
    use crate::StoredEvent;
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind};

    /// Create a signed test event with the given kind and random keys.
    pub fn make_event(kind: Kind) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(kind, "test")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign")
    }

    /// Create a signed test event with the given keys and kind.
    pub fn make_event_with_keys(keys: &Keys, kind: Kind) -> nostr::Event {
        EventBuilder::new(kind, "test")
            .tags([])
            .sign_with_keys(keys)
            .expect("sign")
    }

    /// Create a [`StoredEvent`] wrapper around a test event.
    pub fn make_stored_event(kind: Kind, channel_id: Option<uuid::Uuid>) -> StoredEvent {
        StoredEvent::with_received_at(make_event(kind), Utc::now(), channel_id, true)
    }
}
