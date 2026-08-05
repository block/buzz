//! Canonical NIP-10 thread-reference parsing.
//!
//! Buzz emits marker-based NIP-10 tags only. This module deliberately does not
//! support the deprecated positional form.

use nostr::{Event, EventId};

/// A validated canonical NIP-10 thread reference.
///
/// Direct replies carry only a `reply` marker, in which case the root and
/// parent are the same event. Nested replies carry both `root` and `reply`
/// markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nip10Thread {
    /// Canonical root event for the thread.
    pub root_event_id: EventId,
    /// Immediate parent event being replied to.
    pub parent_event_id: EventId,
}

/// Parse Buzz's canonical marker-based NIP-10 thread shape.
///
/// Each marker candidate is validated as an [`EventId`] before it can replace
/// an earlier candidate. Consequently, a malformed duplicate never erases a
/// valid marker, while later valid duplicates retain the relay's existing
/// last-valid-wins behavior.
///
/// Accepted shapes:
///
/// - `reply` only: a direct reply, where root equals parent;
/// - `root` plus `reply`: a nested reply.
///
/// A `root` marker without a `reply` marker is not a thread reference and
/// returns `None`, matching relay ingestion.
pub fn parse_nip10_thread(event: &Event) -> Option<Nip10Thread> {
    let mut root = None;
    let mut reply = None;

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() < 4 || parts[0] != "e" {
            continue;
        }
        let Ok(candidate) = EventId::from_hex(&parts[1]) else {
            continue;
        };
        match parts[3].as_str() {
            "root" => root = Some(candidate),
            "reply" => reply = Some(candidate),
            _ => {}
        }
    }

    match (root, reply) {
        (Some(root_event_id), Some(parent_event_id)) => Some(Nip10Thread {
            root_event_id,
            parent_event_id,
        }),
        (None, Some(parent_event_id)) => Some(Nip10Thread {
            root_event_id: parent_event_id,
            parent_event_id,
        }),
        (Some(_), None) | (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(tags: Vec<Vec<String>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag structure"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(9), "test")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    fn marker(id: char, marker: &str) -> Vec<String> {
        vec![
            "e".to_string(),
            id.to_string().repeat(64),
            String::new(),
            marker.to_string(),
        ]
    }

    #[test]
    fn no_markers_is_unthreaded() {
        assert_eq!(parse_nip10_thread(&event(vec![])), None);
    }

    #[test]
    fn root_only_is_unthreaded() {
        assert_eq!(parse_nip10_thread(&event(vec![marker('a', "root")])), None);
    }

    #[test]
    fn reply_only_is_a_direct_reply() {
        let parsed = parse_nip10_thread(&event(vec![marker('a', "reply")]))
            .expect("reply-only is canonical");
        assert_eq!(parsed.root_event_id.to_hex(), "a".repeat(64));
        assert_eq!(parsed.parent_event_id, parsed.root_event_id);
    }

    #[test]
    fn root_and_reply_form_a_nested_reply_in_either_order() {
        for tags in [
            vec![marker('a', "root"), marker('b', "reply")],
            vec![marker('b', "reply"), marker('a', "root")],
        ] {
            let parsed = parse_nip10_thread(&event(tags)).expect("canonical nested reply");
            assert_eq!(parsed.root_event_id.to_hex(), "a".repeat(64));
            assert_eq!(parsed.parent_event_id.to_hex(), "b".repeat(64));
        }
    }

    #[test]
    fn malformed_duplicate_does_not_erase_valid_candidate() {
        let malformed_root = vec![
            "e".to_string(),
            "not-an-event-id".to_string(),
            String::new(),
            "root".to_string(),
        ];
        let malformed_reply = vec![
            "e".to_string(),
            "also-invalid".to_string(),
            String::new(),
            "reply".to_string(),
        ];
        let parsed = parse_nip10_thread(&event(vec![
            marker('a', "root"),
            malformed_root,
            marker('b', "reply"),
            malformed_reply,
        ]))
        .expect("valid candidates survive malformed duplicates");
        assert_eq!(parsed.root_event_id.to_hex(), "a".repeat(64));
        assert_eq!(parsed.parent_event_id.to_hex(), "b".repeat(64));
    }

    #[test]
    fn later_valid_duplicate_wins_for_each_marker() {
        let parsed = parse_nip10_thread(&event(vec![
            marker('a', "root"),
            marker('b', "reply"),
            marker('c', "root"),
            marker('d', "reply"),
        ]))
        .expect("canonical nested reply");
        assert_eq!(parsed.root_event_id.to_hex(), "c".repeat(64));
        assert_eq!(parsed.parent_event_id.to_hex(), "d".repeat(64));
    }
}
