//! NIP-25 reaction builders (kind:7 add, kind:5 remove).
//!
//! Split out of the parent `events` module to keep each builder file focused
//! and under the repo's per-file line limit. Shares the parent's `tag` helper
//! and `MAX_EMOJI_CHARS` limit so validation matches the other builders.

use nostr::{EventBuilder, EventId, Kind};

use super::{tag, MAX_EMOJI_CHARS};

/// Kind 7 — NIP-25 reaction.
///
/// `target_author` is the pubkey of the reacted-to event. NIP-25 recommends the
/// `p` tag (a SHOULD): it is what makes the reaction visible to a
/// `{"kinds":[7],"#p":[<self>]}` notification filter — the shape the relay's
/// push-lease surface advertises for kind 7. Pass the event's signer, so the
/// tag matches what every other Buzz client emits for the same target.
///
/// `.allow_self_tagging()` is required: reacting to your own message makes the
/// `p` tag match the signer, and nostr strips matching `p` tags by default.
pub fn build_reaction(
    target_event_id: EventId,
    target_author: nostr::PublicKey,
    emoji: &str,
) -> Result<EventBuilder, String> {
    if emoji.chars().count() > MAX_EMOJI_CHARS {
        return Err(format!(
            "emoji exceeds maximum length of {MAX_EMOJI_CHARS} characters"
        ));
    }
    let tags = vec![
        tag(vec!["e", &target_event_id.to_hex()])?,
        tag(vec!["p", &target_author.to_hex()])?,
    ];
    Ok(EventBuilder::new(Kind::Custom(7), emoji)
        .tags(tags)
        .allow_self_tagging())
}

/// Kind 5 — delete a reaction event.
pub fn build_remove_reaction(reaction_event_id: EventId) -> Result<EventBuilder, String> {
    let tags = vec![tag(vec!["e", &reaction_event_id.to_hex()])?];
    Ok(EventBuilder::new(Kind::Custom(5), "").tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn has_tag(ev: &nostr::Event, key: &str, val: &str) -> bool {
        ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some(key) && s.get(1).map(String::as_str) == Some(val)
        })
    }

    #[test]
    fn reaction_carries_target_e_and_p_tags() {
        let target = Keys::generate();
        let author = Keys::generate().public_key();
        let target_id = EventId::all_zeros();
        let ev = build_reaction(target_id, author, "👍")
            .unwrap()
            .sign_with_keys(&target)
            .unwrap();
        assert_eq!(ev.kind, Kind::Custom(7));
        assert!(has_tag(&ev, "e", &target_id.to_hex()));
        assert!(has_tag(&ev, "p", &author.to_hex()));
    }

    #[test]
    fn self_reaction_keeps_its_p_tag() {
        // Reacting to your own message: the `p` tag equals the signer, which
        // nostr strips unless `.allow_self_tagging()` is set. Pins that it isn't.
        let me = Keys::generate();
        let ev = build_reaction(EventId::all_zeros(), me.public_key(), "👍")
            .unwrap()
            .sign_with_keys(&me)
            .unwrap();
        assert!(has_tag(&ev, "p", &me.public_key().to_hex()));
    }

    #[test]
    fn reaction_rejects_overlong_emoji() {
        let author = Keys::generate().public_key();
        let long = "a".repeat(MAX_EMOJI_CHARS + 1);
        assert!(build_reaction(EventId::all_zeros(), author, &long).is_err());
    }

    #[test]
    fn remove_reaction_targets_the_reaction_event() {
        let reaction_id = EventId::all_zeros();
        let ev = build_remove_reaction(reaction_id)
            .unwrap()
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(ev.kind, Kind::Custom(5));
        assert!(has_tag(&ev, "e", &reaction_id.to_hex()));
    }
}
