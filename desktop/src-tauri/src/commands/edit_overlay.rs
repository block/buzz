//! Latest-edit overlay for read paths that project stored events directly.
//!
//! A `kind:40003` edit carries the full replacement content for the event its
//! `e` tag points at. Any reader that renders stored content without applying
//! these shows the original forever — which is merely stale for a normal
//! message, but wrong for a surface card, whose entire update model is
//! full-spec replacement.
//!
//! `created_at` is second-precision, so two edits can tie; ties break on event
//! id lexicographically. That is the same rule the channel timeline uses
//! (`formatTimelineMessages`), so every client converges on one state.

use std::collections::HashMap;

use buzz_core_pkg::kind::KIND_STREAM_MESSAGE_EDIT;
use nostr::Event;

/// Map of `target event id -> replacement content`, keeping the latest edit
/// per target under `(created_at, id)` ordering.
pub(crate) fn latest_edit_by_target(events: &[Event]) -> HashMap<String, String> {
    // (created_at, edit id, content) per target, so ties resolve deterministically.
    let mut best: HashMap<String, (u64, String, String)> = HashMap::new();

    for event in events {
        if u32::from(event.kind.as_u16()) != KIND_STREAM_MESSAGE_EDIT {
            continue;
        }
        let Some(target) = event.tags.iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts.len() >= 2 && parts[0].as_str() == "e").then(|| parts[1].to_string())
        }) else {
            continue;
        };

        let created_at = event.created_at.as_secs();
        let id = event.id.to_hex();
        let wins = best.get(&target).is_none_or(|(at, existing_id, _)| {
            created_at > *at || (created_at == *at && id > *existing_id)
        });
        if wins {
            best.insert(target, (created_at, id, event.content.clone()));
        }
    }

    best.into_iter()
        .map(|(target, (_, _, content))| (target, content))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn edit(keys: &Keys, target: &str, content: &str, created_at: u64) -> Event {
        EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE_EDIT as u16), content)
            .tags([Tag::parse(["e", target]).expect("tag")])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn latest_edit_wins_by_timestamp() {
        let keys = Keys::generate();
        let target = "a".repeat(64);
        let events = vec![
            edit(&keys, &target, "first", 1_700_000_000),
            edit(&keys, &target, "second", 1_700_000_010),
        ];
        let overlay = latest_edit_by_target(&events);
        assert_eq!(overlay.get(&target).map(String::as_str), Some("second"));
    }

    #[test]
    fn same_second_edits_break_the_tie_on_event_id() {
        let keys = Keys::generate();
        let target = "b".repeat(64);
        let a = edit(&keys, &target, "one", 1_700_000_000);
        let b = edit(&keys, &target, "two", 1_700_000_000);
        let expected = if a.id.to_hex() > b.id.to_hex() {
            "one"
        } else {
            "two"
        };

        // Order of arrival must not change the outcome.
        for events in [vec![a.clone(), b.clone()], vec![b, a]] {
            let overlay = latest_edit_by_target(&events);
            assert_eq!(overlay.get(&target).map(String::as_str), Some(expected));
        }
    }

    #[test]
    fn non_edit_events_and_untargeted_edits_are_ignored() {
        let keys = Keys::generate();
        let message = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        let untargeted = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE_EDIT as u16), "orphan")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(latest_edit_by_target(&[message, untargeted]).is_empty());
    }
}
