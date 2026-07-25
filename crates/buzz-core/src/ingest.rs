//! Deterministic relay event classification and effective-state reduction.
//!
//! This module contains the portable part of relay ingestion. It performs no
//! I/O and does not decide authorization or signature validity. Adapters verify
//! and authorize an event first, ask this module for a decision, establish any
//! required durability barrier, and then update their effective projection.

use nostr::{Alphabet, Event, PublicKey, SingleLetterTag, TagKind};

use crate::StoredEvent;

/// The stable identity of a replaceable event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplacementKey {
    /// A NIP-01 replaceable stream identified by author and kind.
    Replaceable {
        /// Event author.
        author: PublicKey,
        /// Event kind.
        kind: u16,
    },
    /// A NIP-01 parameterized replaceable stream identified by author, kind,
    /// and `d` tag.
    Parameterized {
        /// Event author.
        author: PublicKey,
        /// Event kind.
        kind: u16,
        /// The first `d` tag value, or the empty string when absent.
        identifier: String,
    },
}

/// Portable classification of a verified event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventClass {
    /// A durable event identified by its event ID.
    Regular,
    /// A durable event that competes with an existing replacement stream.
    Replaceable(ReplacementKey),
    /// A live-only event that must not enter durable or effective history.
    Ephemeral,
}

/// Normative decision for a verified, policy-admitted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDecision {
    /// Append the event durably and make it effective.
    Stored,
    /// The event ID was accepted previously.
    Duplicate,
    /// The event lost deterministic ordering within its replacement stream.
    Superseded,
    /// Publish the event live without storing it.
    Ephemeral,
}

/// Returns whether `kind` is in NIP-01's ephemeral range.
pub fn is_ephemeral_kind(kind: u16) -> bool {
    (20_000..30_000).contains(&kind)
}

/// Returns the replacement identity for an event, if it is replaceable.
pub fn replacement_key(event: &Event) -> Option<ReplacementKey> {
    let kind = event.kind.as_u16();
    if kind == 0 || kind == 3 || (10_000..20_000).contains(&kind) {
        return Some(ReplacementKey::Replaceable {
            author: event.pubkey,
            kind,
        });
    }
    if (30_000..40_000).contains(&kind) {
        let identifier = event
            .tags
            .filter(TagKind::SingleLetter(SingleLetterTag::lowercase(
                Alphabet::D,
            )))
            .find_map(|tag| tag.content())
            .unwrap_or_default()
            .to_string();
        return Some(ReplacementKey::Parameterized {
            author: event.pubkey,
            kind,
            identifier,
        });
    }
    None
}

/// Classifies an event using only immutable event fields.
pub fn classify_event(event: &Event) -> EventClass {
    if is_ephemeral_kind(event.kind.as_u16()) {
        EventClass::Ephemeral
    } else if let Some(key) = replacement_key(event) {
        EventClass::Replaceable(key)
    } else {
        EventClass::Regular
    }
}

/// Returns whether a replacement candidate wins over the current event.
///
/// Newer `created_at` wins. Equal timestamps use the lexicographically smaller
/// event ID as the deterministic NIP-01 tie-break.
pub fn replacement_candidate_wins(candidate: &Event, current: &Event) -> bool {
    candidate.created_at > current.created_at
        || (candidate.created_at == current.created_at
            && candidate.id.to_hex() < current.id.to_hex())
}

/// Reduces a verified, policy-admitted event against effective relay state.
///
/// `previously_accepted` must reflect the adapter's durable accepted-ID
/// history, not only its current effective projection. This preserves duplicate
/// idempotency for an older event that has since been replaced.
pub fn decide_event(
    effective_events: &[StoredEvent],
    previously_accepted: bool,
    candidate: &Event,
) -> EventDecision {
    if previously_accepted
        || effective_events
            .iter()
            .any(|stored| stored.event.id == candidate.id)
    {
        return EventDecision::Duplicate;
    }

    match classify_event(candidate) {
        EventClass::Ephemeral => EventDecision::Ephemeral,
        EventClass::Regular => EventDecision::Stored,
        EventClass::Replaceable(candidate_key) => {
            let current = effective_events
                .iter()
                .find(|stored| replacement_key(&stored.event).as_ref() == Some(&candidate_key));
            match current {
                Some(current) if !replacement_candidate_wins(candidate, &current.event) => {
                    EventDecision::Superseded
                }
                _ => EventDecision::Stored,
            }
        }
    }
}

/// Applies a stored event to an in-memory effective projection.
///
/// Adapters should call this only after their durability barrier succeeds.
/// Duplicate, superseded, and ephemeral candidates leave the projection
/// unchanged. The returned decision describes the applied outcome.
pub fn apply_effective_event(
    effective_events: &mut Vec<StoredEvent>,
    candidate: StoredEvent,
) -> EventDecision {
    let decision = decide_event(effective_events, false, &candidate.event);
    if decision != EventDecision::Stored {
        return decision;
    }

    let replacement_index = replacement_key(&candidate.event).and_then(|candidate_key| {
        effective_events
            .iter()
            .position(|stored| replacement_key(&stored.event).as_ref() == Some(&candidate_key))
    });
    if let Some(index) = replacement_index {
        effective_events[index] = candidate;
    } else {
        effective_events.push(candidate);
    }
    EventDecision::Stored
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::*;

    fn stored(event: Event) -> StoredEvent {
        StoredEvent::with_received_at(event, Utc::now(), None, true)
    }

    fn signed_event(keys: &Keys, kind: u16, content: &str, created_at: Timestamp) -> Event {
        EventBuilder::new(Kind::Custom(kind), content)
            .custom_created_at(created_at)
            .sign_with_keys(keys)
            .expect("test event signs")
    }

    #[test]
    fn classifies_regular_replaceable_parameterized_and_ephemeral_events() {
        let keys = Keys::generate();
        let timestamp = Timestamp::now();
        let regular = signed_event(&keys, 1, "regular", timestamp);
        let replaceable = signed_event(&keys, 10_000, "replaceable", timestamp);
        let parameterized = EventBuilder::new(Kind::Custom(30_000), "parameterized")
            .tag(Tag::parse(["d", "coherence"]).expect("test tag parses"))
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
            .expect("test event signs");
        let ephemeral = signed_event(&keys, 20_000, "ephemeral", timestamp);

        assert_eq!(classify_event(&regular), EventClass::Regular);
        assert!(matches!(
            classify_event(&replaceable),
            EventClass::Replaceable(ReplacementKey::Replaceable { kind: 10_000, .. })
        ));
        assert!(matches!(
            classify_event(&parameterized),
            EventClass::Replaceable(ReplacementKey::Parameterized {
                kind: 30_000,
                ref identifier,
                ..
            }) if identifier == "coherence"
        ));
        assert_eq!(classify_event(&ephemeral), EventClass::Ephemeral);
    }

    #[test]
    fn replacement_ordering_prefers_newer_then_lexicographically_smaller_id() {
        let keys = Keys::generate();
        let timestamp = Timestamp::now();
        let older = signed_event(
            &keys,
            10_000,
            "older",
            Timestamp::from(timestamp.as_secs().saturating_sub(1)),
        );
        let first_tie = signed_event(&keys, 10_000, "first tie", timestamp);
        let second_tie = signed_event(&keys, 10_000, "second tie", timestamp);
        let (smaller, larger) = if first_tie.id.to_hex() < second_tie.id.to_hex() {
            (first_tie, second_tie)
        } else {
            (second_tie, first_tie)
        };

        assert!(replacement_candidate_wins(&smaller, &older));
        assert!(replacement_candidate_wins(&smaller, &larger));
        assert!(!replacement_candidate_wins(&larger, &smaller));
    }

    #[test]
    fn reduction_preserves_duplicate_and_replacement_semantics() {
        let keys = Keys::generate();
        let now = Timestamp::now();
        let older = signed_event(
            &keys,
            10_000,
            "older",
            Timestamp::from(now.as_secs().saturating_sub(1)),
        );
        let newer = signed_event(&keys, 10_000, "newer", now);
        let mut effective = Vec::new();

        assert_eq!(
            apply_effective_event(&mut effective, stored(older.clone())),
            EventDecision::Stored
        );
        assert_eq!(
            decide_event(&effective, true, &older),
            EventDecision::Duplicate
        );
        assert_eq!(
            apply_effective_event(&mut effective, stored(newer.clone())),
            EventDecision::Stored
        );
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].event, newer);
        assert_eq!(
            decide_event(&effective, true, &older),
            EventDecision::Duplicate
        );
        assert_eq!(
            decide_event(&effective, false, &older),
            EventDecision::Superseded
        );
    }

    #[test]
    fn ephemeral_events_never_enter_the_effective_projection() {
        let event = signed_event(&Keys::generate(), 29_999, "live only", Timestamp::now());
        let mut effective = Vec::new();

        assert_eq!(
            apply_effective_event(&mut effective, stored(event)),
            EventDecision::Ephemeral
        );
        assert!(effective.is_empty());
    }
}
