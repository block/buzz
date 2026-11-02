//! Hides the relay's co-founder wake messages from every native read.
//!
//! In co-founder mode the relay wakes Scout with a kind 9 message in the DM
//! between Scout and its owner. The message is for Scout's harness, never for a
//! person: it must not reach the conversation, Activity, unread counts,
//! notifications or search. The renderer drops it at its own relay session
//! (`shared/lib/cofounderWake.ts`); this is the same rule for the reads the
//! native side makes (`POST /query` and the unread catch-up).
//!
//! A wake is a stream message with the `buzz:cofounder-wake` tag that is signed
//! by the relay itself, so a member cannot hide their own message by copying
//! the tag. While the relay's key cannot be read the author cannot be checked
//! and a tagged message stays hidden (failing closed hides one message; failing
//! open shows the owner a message nobody was meant to read).

use buzz_core_pkg::kind::{KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2};
use nostr::Event;

use crate::app_state::AppState;

/// The tag the relay puts on a wake message.
pub(crate) const COFOUNDER_WAKE_TAG: &str = "buzz:cofounder-wake";

/// Whether the event is a stream message (v1 or v2) that carries the wake tag.
pub(crate) fn carries_wake_tag(event: &Event) -> bool {
    let kind = u32::from(event.kind.as_u16());
    (kind == KIND_STREAM_MESSAGE || kind == KIND_STREAM_MESSAGE_V2)
        && event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some(COFOUNDER_WAKE_TAG))
}

/// Whether the event is a co-founder wake, given the relay's own key (`None`
/// while it is unknown).
pub(crate) fn is_wake_event(event: &Event, relay_self: Option<&str>) -> bool {
    if !carries_wake_tag(event) {
        return false;
    }
    match relay_self {
        Some(relay_self) => event.pubkey.to_hex().eq_ignore_ascii_case(relay_self),
        None => true,
    }
}

/// Drops the wake messages from a read. The relay's key is only looked up when
/// a tagged message is present, so an ordinary read costs one scan of the tags.
pub(crate) async fn without_wakes(state: &AppState, events: Vec<Event>) -> Vec<Event> {
    if !events.iter().any(carries_wake_tag) {
        return events;
    }
    let relay_self = crate::commands::identity_archive::fetch_relay_self(state)
        .await
        .ok()
        .flatten();
    events
        .into_iter()
        .filter(|event| !is_wake_event(event, relay_self.as_deref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn message(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("tag"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(kind), "Wake from the relay")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn only_the_relays_own_tagged_message_is_a_wake() {
        let relay = Keys::generate();
        let member = Keys::generate();
        let relay_hex = relay.public_key().to_hex();
        let wake = message(&relay, 9, &[&["h", "dm"], &[COFOUNDER_WAKE_TAG, "mode_on"]]);
        assert!(is_wake_event(&wake, Some(&relay_hex)));
        assert!(
            is_wake_event(&wake, None),
            "an unreadable relay key fails closed"
        );

        let copied = message(
            &member,
            9,
            &[&["h", "dm"], &[COFOUNDER_WAKE_TAG, "mode_on"]],
        );
        assert!(
            !is_wake_event(&copied, Some(&relay_hex)),
            "a member copying the tag must not make a message invisible"
        );

        let plain = message(&relay, 9, &[&["h", "dm"]]);
        assert!(!is_wake_event(&plain, Some(&relay_hex)));
        let other_kind = message(&relay, 1, &[&[COFOUNDER_WAKE_TAG, "mode_on"]]);
        assert!(!is_wake_event(&other_kind, Some(&relay_hex)));
    }
}
