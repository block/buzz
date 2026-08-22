//! The gates that decide what a `p` tag on a reply means.
//!
//! A reply `p`-tags the author it answers, and that tag is byte-identical to a
//! typed `@mention`. Every gate here exists to tell those apart: from the role
//! marker the sender left, and failing that from the parent's author.
//!
//! Mirrors `desktop/src/features/notifications/lib/shouldNotify.ts`. The two
//! decide notification ownership independently from the same question, so a
//! divergence here makes them disagree and notify twice — or not at all.

use std::collections::HashSet;

use crate::p_tag_role::{p_tag_role, PTagRole};
use crate::unread_catch_up::{has_exact_tag, has_tag_value, thread_reference, EventView};

/// Whether this event names `self_pubkey` in a `p` tag at all.
///
/// Mirrors `hasMentionForEvent` in
/// `desktop/src/features/notifications/lib/shouldNotify.ts`. Necessary but not
/// sufficient for a mention: a reply addresses the author it answers with the
/// same tag.
pub(crate) fn has_p_tag_for(tags: &[Vec<String>], self_pubkey: &str) -> bool {
    has_tag_value(tags, "p", self_pubkey)
}

pub(crate) fn role_for(tags: &[Vec<String>], self_pubkey: &str) -> PTagRole {
    p_tag_role(tags.iter().map(|tag| tag.as_slice()), self_pubkey)
}

/// Whether someone actually mentioned the user, as opposed to answering them.
///
/// Mirrors `hasAuthoredMentionForEvent` in `shouldNotify.ts`. `parent_author` is
/// `None` for a DM, where every message `p`-tags both participants and the
/// addressing tag *is* the addressing — demoting it there would silence answers
/// to the user while letting new messages through.
pub(crate) fn has_authored_mention(
    tags: &[Vec<String>],
    self_pubkey: &str,
    parent_author: Option<&str>,
) -> bool {
    if !has_p_tag_for(tags, self_pubkey) {
        return false;
    }
    // A broadcast reply is addressed to the channel, not just to the parent's
    // author, and `should_notify` admits it before ever reading the parent.
    if has_exact_tag(tags, "broadcast", "1") {
        return true;
    }
    // The sender's own answer, when it gave one. Only a marker that is actually
    // present is authoritative; `Unknown` and `None` fall through to the parent.
    match role_for(tags, self_pubkey) {
        PTagRole::Mention => return true,
        PTagRole::Addressing => return false,
        PTagRole::Unknown | PTagRole::None => {}
    }
    let reference = thread_reference(tags);
    !(reference.parent_id.is_some()
        && parent_author.is_some_and(|author| author.eq_ignore_ascii_case(self_pubkey)))
}

/// Mirrors `isHighPriorityEventForUser` in `shouldNotify.ts`.
///
/// Fails closed where notification delivery fails open: the parent is resolved
/// for exactly the replies that tag the user, so an unresolved parent here means
/// the lookup was tried and failed. This flag is persisted and makes the
/// channel's top-level items drop out of the dock badge, so guessing "high"
/// after a relay flap would silently hide them.
pub(crate) fn is_high_priority(
    tags: &[Vec<String>],
    self_pubkey: &str,
    parent_author: Option<&str>,
) -> bool {
    if has_exact_tag(tags, "broadcast", "1") {
        return true;
    }
    if !has_p_tag_for(tags, self_pubkey) {
        return false;
    }
    match role_for(tags, self_pubkey) {
        PTagRole::Mention => return true,
        PTagRole::Addressing => return false,
        PTagRole::Unknown | PTagRole::None => {}
    }
    if thread_reference(tags).parent_id.is_some() && parent_author.is_none() {
        return false;
    }
    has_authored_mention(tags, self_pubkey, parent_author)
}

/// The sets the notify gate consults, gathered once per batch.
pub(crate) struct NotifyGate<'a> {
    /// Channels the user muted. The only thing the gate reads off the request.
    pub(crate) muted_channel_ids: &'a HashSet<String>,
    pub(crate) membership: &'a std::collections::HashMap<String, HashSet<String>>,
    pub(crate) participated: &'a HashSet<String>,
    pub(crate) authored: &'a HashSet<String>,
    pub(crate) mentioned: &'a HashSet<String>,
}

/// Mirrors `shouldNotifyForEvent` in
/// `desktop/src/features/notifications/lib/shouldNotify.ts`.
pub(crate) fn should_notify(
    event: &EventView,
    self_pubkey: &str,
    gate: &NotifyGate<'_>,
    is_dm: bool,
    parent_author: Option<&str>,
) -> bool {
    if has_exact_tag(&event.tags, "broadcast", "1") {
        return true;
    }
    let reference = thread_reference(&event.tags);
    // A reply we authored the parent of always carries our `p` tag, so that tag
    // alone cannot mean "this message mentions you". Only a real mention skips
    // the mute gates below; a reply answering us is re-admitted after them.
    // Never in a DM: there the addressing tag is the whole point, so demoting it
    // would silence answers to us while letting new messages through.
    let is_reply_to_self = !is_dm
        && reference.parent_id.is_some()
        && !self_pubkey.is_empty()
        && match role_for(&event.tags, self_pubkey) {
            PTagRole::Addressing => true,
            // The case the parent cannot decide: the recipient is both the
            // author being answered and someone typed in the body.
            PTagRole::Mention => false,
            PTagRole::Unknown | PTagRole::None => {
                parent_author.is_some_and(|author| author.eq_ignore_ascii_case(self_pubkey))
            }
        };
    if !is_reply_to_self && has_p_tag_for(&event.tags, self_pubkey) {
        return true;
    }
    let event_channel_id = event
        .tags
        .iter()
        .find(|tag| tag.first().is_some_and(|part| part == "h"))
        .and_then(|tag| tag.get(1));
    if event_channel_id.is_some_and(|id| gate.muted_channel_ids.contains(id)) {
        return false;
    }
    if reference.parent_id.is_none() {
        return true;
    }
    let Some(root_id) = reference.root_id else {
        return false;
    };
    if gate
        .membership
        .get("muted_root")
        .is_some_and(|set| set.contains(&root_id))
    {
        return false;
    }
    // Past the mute gates, a reply answering us always notifies. The
    // participated/authored sets are local and rebuilt from the unread window,
    // so on a fresh install they can be empty for a thread we started.
    if is_reply_to_self {
        return true;
    }
    gate.participated.contains(&root_id)
        || gate.membership
            .get("followed")
            .is_some_and(|set| set.contains(&root_id))
        || gate.authored.contains(&root_id)
        // Below the mute gates on purpose: being mentioned in a thread
        // subscribes you to it, but muting it afterwards still wins.
        || gate.mentioned.contains(&root_id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn reply_tags<'a>(parent: &'a str, p_tag: &[&'a str]) -> Vec<Vec<String>> {
        vec![
            vec!["e".into(), parent.into(), String::new(), "reply".into()],
            vec!["h".into(), "ch".into()],
            p_tag.iter().map(|part| (*part).to_string()).collect(),
        ]
    }

    fn event_with(tags: Vec<Vec<String>>) -> EventView {
        EventView {
            id: "r".into(),
            kind: 9,
            pubkey: "other".into(),
            content: "r".into(),
            created_at: 10,
            tags,
        }
    }

    #[test]
    fn high_priority_fails_closed_when_the_parent_is_unresolved() {
        // This flag is persisted and drops the channel's top-level items from
        // the dock badge. Missing a red dot after a relay flap is recoverable;
        // silently hiding an approval request until the channel is read is not.
        assert!(!is_high_priority(
            &reply_tags("parent", &["p", "self"]),
            "self",
            None,
        ));
    }

    #[test]
    fn high_priority_holds_for_a_reply_answering_someone_else() {
        assert!(is_high_priority(
            &reply_tags("parent", &["p", "self"]),
            "self",
            Some("third-party"),
        ));
    }

    #[test]
    fn high_priority_is_dropped_for_a_reply_answering_us() {
        assert!(!is_high_priority(
            &reply_tags("parent", &["p", "self"]),
            "self",
            Some("self"),
        ));
    }

    #[test]
    fn high_priority_reads_the_marker_before_the_parent() {
        assert!(is_high_priority(
            &reply_tags("parent", &["p", "self", "", "mention"]),
            "self",
            Some("self"),
        ));
        assert!(!is_high_priority(
            &reply_tags("parent", &["p", "self", "", "reply"]),
            "self",
            Some("third-party"),
        ));
    }

    // ---- The mute gate ----------------------------------------------------

    fn notifies(p_tag: &[&str], parent_author: Option<&str>, muted_channel: bool) -> bool {
        let mut muted: HashSet<String> = HashSet::new();
        if muted_channel {
            muted.insert("ch".to_string());
        }
        let event = event_with(reply_tags("parent", p_tag));
        should_notify(
            &event,
            "self",
            &NotifyGate {
                muted_channel_ids: &muted,
                membership: &HashMap::new(),
                participated: &HashSet::new(),
                authored: &HashSet::new(),
                mentioned: &HashSet::new(),
            },
            false,
            parent_author,
        )
    }

    #[test]
    fn a_reply_answering_us_does_not_pierce_a_muted_channel() {
        // Only a real mention skips the mute gate. Before the markers this tag
        // pierced it, which is what made a muted channel keep notifying.
        assert!(!notifies(&["p", "self"], Some("self"), true));
        assert!(!notifies(&["p", "self", "", "reply"], None, true));
    }

    #[test]
    fn a_real_mention_still_pierces_a_muted_channel() {
        assert!(notifies(&["p", "self"], Some("third-party"), true));
        assert!(notifies(&["p", "self", "", "mention"], Some("self"), true));
    }

    #[test]
    fn a_reply_answering_us_still_notifies_in_an_unmuted_channel() {
        // Re-admitted after the mute gates: the participated/authored sets are
        // rebuilt from the unread window and can be empty for our own thread.
        assert!(notifies(&["p", "self"], Some("self"), false));
    }
}
