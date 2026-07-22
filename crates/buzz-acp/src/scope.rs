//! Conversation scope — the typed key for ACP session routing.
//!
//! The harness keys session affinity, turn counters, queueing, in-flight
//! tracking, and invalidation by [`ConversationScope`] instead of the bare
//! channel UUID, so separate threads inside one channel get isolated
//! ACP/Hermes sessions (Discord-style thread isolation) while unthreaded
//! channel traffic and DMs keep channel-level continuity.
//!
//! This is agent conversation routing only: channel identity, subscriptions,
//! and NIP-29 authorization remain keyed by the `h`-tag channel UUID.

use nostr::EventId;
use uuid::Uuid;

use crate::queue::parse_thread_tags;

/// The conversation a Buzz event belongs to.
///
/// Two shapes exist within a channel:
///
/// - **Channel scope** (`thread_root == None`) — unthreaded channel messages
///   and all DM traffic. One continuous conversation per channel, matching
///   the pre-thread-scoping behavior.
/// - **Thread scope** (`thread_root == Some(root)`) — thread replies in a
///   regular channel. Each canonical NIP-10 thread root gets its own
///   isolated conversation: its own ACP session, turn counter, queue lane,
///   and in-flight slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConversationScope {
    /// The channel this conversation lives in (NIP-29 `h` tag UUID).
    pub channel_id: Uuid,
    /// Canonical thread root event ID, when the conversation is a thread.
    pub thread_root: Option<EventId>,
}

impl ConversationScope {
    /// Channel-level scope: unthreaded messages, DMs, control fallback.
    pub fn channel(channel_id: Uuid) -> Self {
        Self {
            channel_id,
            thread_root: None,
        }
    }

    /// Thread-level scope for a specific canonical root event.
    pub fn thread(channel_id: Uuid, root: EventId) -> Self {
        Self {
            channel_id,
            thread_root: Some(root),
        }
    }

    /// Derive the scope for an inbound event.
    ///
    /// `thread_scoped` must be `false` for DM channels — and for channels
    /// whose type could not be determined — so those keep channel-level
    /// continuity. When `true`:
    ///
    /// - A forum post (kind 45001) IS a thread root: it scopes to its own
    ///   event ID, so the root post and its later comments share one
    ///   conversation, while separate forum posts in the same channel stay
    ///   isolated from each other.
    /// - Otherwise the canonical NIP-10 root from the event's `e` tags (via
    ///   [`parse_thread_tags`], the single thread parser in this crate)
    ///   selects the thread scope — this covers stream thread replies (kind
    ///   9) and forum comments (kind 45003), whose root marker carries the
    ///   forum post's ID and therefore resolves to the same scope as the
    ///   post itself.
    /// - Events with no root tag, or with a root that is not a valid event
    ///   ID, fall back to channel scope.
    pub fn for_event(channel_id: Uuid, event: &nostr::Event, thread_scoped: bool) -> Self {
        if !thread_scoped {
            return Self::channel(channel_id);
        }
        if event.kind.as_u16() as u32 == buzz_core::kind::KIND_FORUM_POST {
            return Self::thread(channel_id, event.id);
        }
        let root = parse_thread_tags(event)
            .root_event_id
            .as_deref()
            .and_then(|hex| EventId::from_hex(hex).ok());
        match root {
            Some(root) => Self::thread(channel_id, root),
            None => Self::channel(channel_id),
        }
    }

    /// Whether this scope identifies a thread conversation.
    #[cfg(test)]
    pub fn is_thread(&self) -> bool {
        self.thread_root.is_some()
    }
}

impl std::fmt::Display for ConversationScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.thread_root {
            Some(root) => write!(f, "{}#{}", self.channel_id, root.to_hex()),
            None => write!(f, "{}", self.channel_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn make_event_kind(kind: u16, tags: Vec<Vec<String>>) -> nostr::Event {
        let keys = Keys::generate();
        let tags: Vec<Tag> = tags
            .into_iter()
            .map(|t| Tag::parse(&t).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::Custom(kind), "hello")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign test event")
    }

    fn make_event(tags: Vec<Vec<String>>) -> nostr::Event {
        make_event_kind(9, tags)
    }

    fn root_hex() -> String {
        "a".repeat(64)
    }

    #[test]
    fn unthreaded_event_gets_channel_scope() {
        let ch = Uuid::new_v4();
        let event = make_event(vec![]);
        let scope = ConversationScope::for_event(ch, &event, true);
        assert_eq!(scope, ConversationScope::channel(ch));
        assert!(!scope.is_thread());
    }

    #[test]
    fn root_marker_selects_thread_scope() {
        let ch = Uuid::new_v4();
        let event = make_event(vec![vec!["e".into(), root_hex(), "".into(), "root".into()]]);
        let scope = ConversationScope::for_event(ch, &event, true);
        let expected_root = EventId::from_hex(&root_hex()).expect("valid hex");
        assert_eq!(scope, ConversationScope::thread(ch, expected_root));
        assert!(scope.is_thread());
    }

    #[test]
    fn reply_marker_only_uses_reply_as_canonical_root() {
        // parse_thread_tags treats a lone "reply" marker as root == parent.
        let ch = Uuid::new_v4();
        let event = make_event(vec![vec![
            "e".into(),
            root_hex(),
            "".into(),
            "reply".into(),
        ]]);
        let scope = ConversationScope::for_event(ch, &event, true);
        let expected_root = EventId::from_hex(&root_hex()).expect("valid hex");
        assert_eq!(scope, ConversationScope::thread(ch, expected_root));
    }

    #[test]
    fn root_and_reply_markers_use_root() {
        let ch = Uuid::new_v4();
        let parent = "b".repeat(64);
        let event = make_event(vec![
            vec!["e".into(), root_hex(), "".into(), "root".into()],
            vec!["e".into(), parent, "".into(), "reply".into()],
        ]);
        let scope = ConversationScope::for_event(ch, &event, true);
        let expected_root = EventId::from_hex(&root_hex()).expect("valid hex");
        assert_eq!(scope.thread_root, Some(expected_root));
    }

    #[test]
    fn dm_or_unknown_channel_stays_channel_scoped_even_with_thread_tags() {
        let ch = Uuid::new_v4();
        let event = make_event(vec![vec!["e".into(), root_hex(), "".into(), "root".into()]]);
        let scope = ConversationScope::for_event(ch, &event, false);
        assert_eq!(scope, ConversationScope::channel(ch));
    }

    #[test]
    fn malformed_root_hex_falls_back_to_channel_scope() {
        let ch = Uuid::new_v4();
        let event = make_event(vec![vec![
            "e".into(),
            "not-hex".into(),
            "".into(),
            "root".into(),
        ]]);
        let scope = ConversationScope::for_event(ch, &event, true);
        assert_eq!(scope, ConversationScope::channel(ch));
    }

    #[test]
    fn same_thread_same_scope_distinct_threads_distinct_scopes() {
        let ch = Uuid::new_v4();
        let root_a = vec!["e".into(), "a".repeat(64), "".into(), "root".into()];
        let root_b = vec!["e".into(), "b".repeat(64), "".into(), "root".into()];
        let a1 = ConversationScope::for_event(ch, &make_event(vec![root_a.clone()]), true);
        let a2 = ConversationScope::for_event(ch, &make_event(vec![root_a]), true);
        let b = ConversationScope::for_event(ch, &make_event(vec![root_b]), true);
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert_eq!(a1.channel_id, b.channel_id);
    }

    #[test]
    fn forum_post_scopes_to_its_own_event_id() {
        let ch = Uuid::new_v4();
        let post = make_event_kind(45001, vec![]);
        let scope = ConversationScope::for_event(ch, &post, true);
        assert_eq!(scope, ConversationScope::thread(ch, post.id));
    }

    #[test]
    fn forum_comment_resolves_to_the_posts_scope() {
        let ch = Uuid::new_v4();
        let post = make_event_kind(45001, vec![]);
        let post_scope = ConversationScope::for_event(ch, &post, true);
        // A comment (45003) whose canonical root marker names the post.
        let comment = make_event_kind(
            45003,
            vec![vec!["e".into(), post.id.to_hex(), "".into(), "root".into()]],
        );
        let comment_scope = ConversationScope::for_event(ch, &comment, true);
        assert_eq!(
            comment_scope, post_scope,
            "forum root and its comments must share one conversation"
        );
    }

    #[test]
    fn separate_forum_posts_are_isolated() {
        let ch = Uuid::new_v4();
        let post_a = make_event_kind(45001, vec![]);
        let post_b = make_event_kind(45001, vec![]);
        let scope_a = ConversationScope::for_event(ch, &post_a, true);
        let scope_b = ConversationScope::for_event(ch, &post_b, true);
        assert_ne!(scope_a, scope_b);
        assert_eq!(scope_a.channel_id, scope_b.channel_id);
    }

    #[test]
    fn forum_post_in_unscoped_channel_stays_channel_scoped() {
        let ch = Uuid::new_v4();
        let post = make_event_kind(45001, vec![]);
        let scope = ConversationScope::for_event(ch, &post, false);
        assert_eq!(scope, ConversationScope::channel(ch));
    }

    #[test]
    fn display_includes_root_for_threads() {
        let ch = Uuid::new_v4();
        let root = EventId::from_hex(&"c".repeat(64)).expect("valid hex");
        assert_eq!(ConversationScope::channel(ch).to_string(), ch.to_string());
        assert_eq!(
            ConversationScope::thread(ch, root).to_string(),
            format!("{}#{}", ch, "c".repeat(64))
        );
    }
}
