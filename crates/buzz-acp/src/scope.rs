//! Session scoping for ACP.
//!
//! A [`SessionScope`] is the single hashable key that identifies an ACP
//! provider session and its conversational-context boundary. It is derived
//! **once**, when an eligible event is admitted, from the operator
//! [`SessionPolicy`], whether the channel is a DM, and the event's NIP-10
//! thread tags. Later code must never re-infer scope from the last event in a
//! batch — it carries the resolved scope instead.
//!
//! Policy matrix (see the "Make ACP sessions thread-scoped" ticket):
//!
//! | Surface                             | Scope                                   |
//! | ----------------------------------- | --------------------------------------- |
//! | New top-level channel mention       | `Thread(channel_id, triggering_event)`  |
//! | Reply in a channel thread           | `Thread(channel_id, canonical_root)`    |
//! | Repeated mention in the same thread | reuse that thread scope                 |
//! | Direct message                      | `Conversation(channel_id)`              |
//!
//! Under [`SessionPolicy::Channel`] (the current default / rollback path) every
//! surface collapses to `Conversation(channel_id)`, preserving today's
//! channel-keyed behavior exactly.

use nostr::Event;
use uuid::Uuid;

use crate::queue::parse_thread_tags;

/// Operator policy controlling how ACP provider sessions are scoped.
///
/// Selected via `--session-policy` / `BUZZ_ACP_SESSION_POLICY`. Defaults to
/// [`Channel`](SessionPolicy::Channel) so the feature ships dark and can be
/// canaried, then flipped, then rolled back without code changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum SessionPolicy {
    /// Legacy behavior: one provider session per channel. Every event in a
    /// channel shares a `Conversation(channel_id)` scope.
    #[default]
    Channel,
    /// Thread-scoped: each canonical channel thread gets an isolated provider
    /// session. DMs remain conversation-scoped.
    Thread,
}

impl SessionPolicy {
    /// Append only the configured session model to the shared base instructions.
    /// The resulting base is reused by modern and legacy ACP standing context.
    pub(crate) fn append_session_model(self, base_prompt: &str) -> String {
        let session_model = match self {
            Self::Channel => include_str!("session_model_channel.md"),
            Self::Thread => include_str!("session_model_thread.md"),
        };
        format!("{}\n\n{}", base_prompt.trim_end(), session_model.trim_end())
    }
}

impl std::fmt::Display for SessionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel => f.write_str("channel"),
            Self::Thread => f.write_str("thread"),
        }
    }
}

/// A hashable ACP execution and conversational-context scope.
///
/// This is the canonical key for provider sessions, queue partitions, in-flight
/// tracking, and context gathering. The channel remains the authorization and
/// collaboration boundary; the scope is the default *execution* boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionScope {
    /// The whole channel is one session. Used for DMs always, and for every
    /// channel event under [`SessionPolicy::Channel`].
    Conversation { channel_id: Uuid },
    /// A single canonical thread within a channel, keyed by its root event id
    /// (64-char lowercase hex).
    Thread {
        channel_id: Uuid,
        root_event_id: String,
    },
}

impl SessionScope {
    /// The channel this scope belongs to. Always available — the channel is the
    /// authorization boundary regardless of scope variant.
    pub fn channel_id(&self) -> Uuid {
        match self {
            Self::Conversation { channel_id } => *channel_id,
            Self::Thread { channel_id, .. } => *channel_id,
        }
    }

    /// The canonical thread-root event id for a [`Thread`](Self::Thread) scope,
    /// or `None` for a conversation scope.
    pub fn root_event_id(&self) -> Option<&str> {
        match self {
            Self::Conversation { .. } => None,
            Self::Thread { root_event_id, .. } => Some(root_event_id),
        }
    }

    /// True when this scope is thread-scoped (not conversation-scoped).
    pub fn is_thread(&self) -> bool {
        matches!(self, Self::Thread { .. })
    }

    /// Derive the scope for an admitted event.
    ///
    /// Resolution order:
    /// 1. DMs are always [`Conversation`](Self::Conversation) — the ticket keeps
    ///    direct messages conversation-scoped regardless of policy.
    /// 2. Under [`SessionPolicy::Channel`], every channel event is
    ///    conversation-scoped (legacy / rollback behavior).
    /// 3. Under [`SessionPolicy::Thread`], a channel event with a NIP-10 root
    ///    tag scopes to that canonical root; a top-level mention (no thread
    ///    tags) opens a new thread rooted at the triggering event id.
    ///
    /// Thread roots are resolved with [`parse_thread_tags`], i.e. Buzz's shared
    /// [`buzz_core::nip10`] canonical-root rules — a malformed marker id is
    /// ignored (treated as top-level), and a lone `root` marker with no `reply`
    /// is top-level, matching relay ingest.
    ///
    /// The root id is normalized to lowercase before it becomes the scope key.
    /// The shared NIP-10 parser accepts and preserves uppercase ASCII hex
    /// (`is_ascii_hexdigit`), but the relay decodes event ids to bytes on
    /// ingest, so `AB…` and `ab…` name the *same* thread. Without normalization
    /// those equivalent spellings would hash to different `Thread` keys and
    /// split one relay thread across two ACP sessions (queue state, provider
    /// sessions, affinity, delivery ledgers). `nostr::EventId::to_hex()` is
    /// already lowercase, so the top-level path is unaffected.
    pub fn derive(policy: SessionPolicy, channel_id: Uuid, is_dm: bool, event: &Event) -> Self {
        if is_dm || policy == SessionPolicy::Channel {
            return Self::Conversation { channel_id };
        }

        let root_event_id = match parse_thread_tags(event).root_event_id {
            Some(root) => root,
            None => event.id.to_hex(),
        };
        Self::Thread {
            channel_id,
            root_event_id: root_event_id.to_ascii_lowercase(),
        }
    }

    /// A compact, log-friendly label for telemetry (e.g. `conversation` or
    /// `thread:<root8>`), never leaking full ids into high-cardinality fields.
    pub fn telemetry_label(&self) -> String {
        match self {
            Self::Conversation { .. } => "conversation".to_string(),
            Self::Thread { root_event_id, .. } => {
                let short: String = root_event_id.chars().take(8).collect();
                format!("thread:{short}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    /// Build a signed event with the given NIP-10 `e`/`p` tags.
    fn event_with_tags(tags: Vec<Vec<String>>) -> Event {
        let keys = Keys::generate();
        let tags: Vec<nostr::Tag> = tags
            .into_iter()
            .map(|t| nostr::Tag::parse(t).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::Custom(9), "hello")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn plain_event() -> Event {
        event_with_tags(vec![])
    }

    #[test]
    fn session_model_is_appended_once_and_matches_policy() {
        let base = include_str!("base_prompt.md");
        assert!(!base.contains("## Session Model"));
        for policy in [SessionPolicy::Channel, SessionPolicy::Thread] {
            let prompt = policy.append_session_model(base);
            assert!(prompt.starts_with(base.trim_end()));
            assert_eq!(prompt.matches("## Session Model").count(), 1);
            assert!(prompt.ends_with("assume the owning session has it handled."));
            assert!(prompt.contains("DMs stay one conversation"));
            assert!(prompt.contains(
                "core memory, your workspace on disk, relay access, and channel authorization"
            ));
            assert!(prompt.contains("leave execution with the owning session"));
            match policy {
                SessionPolicy::Channel => {
                    assert!(prompt.contains("one per-channel session"));
                    assert!(!prompt.contains("each thread gets its own"));
                    assert!(!prompt.contains("sibling channel thread"));
                }
                SessionPolicy::Thread => {
                    assert!(prompt.contains("each thread gets its own"));
                    assert!(prompt.contains("sibling channel thread"));
                    assert!(!prompt.contains("one per-channel session"));
                }
            }
        }
    }

    #[test]
    fn dm_is_always_conversation_scoped_under_thread_policy() {
        let ch = Uuid::new_v4();
        // Even a DM with a reply tag stays conversation-scoped.
        let root = "a".repeat(64);
        let reply = event_with_tags(vec![
            vec!["e".into(), root.clone(), String::new(), "root".into()],
            vec!["e".into(), "b".repeat(64), String::new(), "reply".into()],
        ]);
        let scope = SessionScope::derive(SessionPolicy::Thread, ch, true, &reply);
        assert_eq!(scope, SessionScope::Conversation { channel_id: ch });
    }

    #[test]
    fn channel_policy_collapses_everything_to_conversation() {
        let ch = Uuid::new_v4();
        let root = "a".repeat(64);
        let reply = event_with_tags(vec![
            vec!["e".into(), root.clone(), String::new(), "root".into()],
            vec!["e".into(), "b".repeat(64), String::new(), "reply".into()],
        ]);
        // A threaded reply under Channel policy is still conversation-scoped.
        let scope = SessionScope::derive(SessionPolicy::Channel, ch, false, &reply);
        assert_eq!(scope, SessionScope::Conversation { channel_id: ch });
        // As is a top-level mention.
        let scope = SessionScope::derive(SessionPolicy::Channel, ch, false, &plain_event());
        assert_eq!(scope, SessionScope::Conversation { channel_id: ch });
    }

    #[test]
    fn top_level_mention_opens_thread_rooted_at_trigger() {
        let ch = Uuid::new_v4();
        let ev = plain_event();
        let scope = SessionScope::derive(SessionPolicy::Thread, ch, false, &ev);
        assert_eq!(
            scope,
            SessionScope::Thread {
                channel_id: ch,
                root_event_id: ev.id.to_hex(),
            }
        );
    }

    #[test]
    fn direct_reply_to_root_scopes_to_that_root() {
        let ch = Uuid::new_v4();
        let root = "c".repeat(64);
        // A single `e` tag carrying only a `root` marker.
        let ev = event_with_tags(vec![vec![
            "e".into(),
            root.clone(),
            String::new(),
            "root".into(),
        ]]);
        // NIP-10: lone `root` with no `reply` is top-level per ingest rules, so
        // this yields a top-level scope rooted at the trigger, not `root`.
        let scope = SessionScope::derive(SessionPolicy::Thread, ch, false, &ev);
        assert_eq!(
            scope,
            SessionScope::Thread {
                channel_id: ch,
                root_event_id: ev.id.to_hex(),
            }
        );
    }

    #[test]
    fn nested_reply_scopes_to_canonical_root_not_parent() {
        let ch = Uuid::new_v4();
        let root = "c".repeat(64);
        let parent = "d".repeat(64);
        let ev = event_with_tags(vec![
            vec!["e".into(), root.clone(), String::new(), "root".into()],
            vec!["e".into(), parent.clone(), String::new(), "reply".into()],
        ]);
        let scope = SessionScope::derive(SessionPolicy::Thread, ch, false, &ev);
        // Scope keys on the canonical ROOT, never the immediate parent.
        assert_eq!(
            scope,
            SessionScope::Thread {
                channel_id: ch,
                root_event_id: root,
            }
        );
    }

    #[test]
    fn repeated_replies_in_same_thread_share_scope() {
        let ch = Uuid::new_v4();
        let root = "e".repeat(64);
        let mk_reply = || {
            event_with_tags(vec![
                vec!["e".into(), root.clone(), String::new(), "root".into()],
                vec!["e".into(), "f".repeat(64), String::new(), "reply".into()],
            ])
        };
        let a = SessionScope::derive(SessionPolicy::Thread, ch, false, &mk_reply());
        let b = SessionScope::derive(SessionPolicy::Thread, ch, false, &mk_reply());
        assert_eq!(a, b, "same-root replies must reuse the same thread scope");
    }

    #[test]
    fn different_top_level_mentions_get_distinct_scopes() {
        let ch = Uuid::new_v4();
        let a = SessionScope::derive(SessionPolicy::Thread, ch, false, &plain_event());
        let b = SessionScope::derive(SessionPolicy::Thread, ch, false, &plain_event());
        assert_ne!(
            a, b,
            "two independent top-level mentions must not share a session"
        );
    }

    #[test]
    fn mixed_case_root_spellings_share_one_thread_scope() {
        // The relay decodes event ids to bytes, so `AB…` and `ab…` name the
        // same thread. Equivalent-case root tags must resolve to the SAME
        // `SessionScope::Thread` key, or thread state would split in two.
        let ch = Uuid::new_v4();
        let root_lower = "a1b2c3d4e5f6".repeat(4) + &"0".repeat(16); // 64 hex
        assert_eq!(root_lower.len(), 64);
        let root_upper = root_lower.to_ascii_uppercase();

        let mk = |root: &str| {
            event_with_tags(vec![
                vec!["e".into(), root.to_string(), String::new(), "root".into()],
                vec!["e".into(), "f".repeat(64), String::new(), "reply".into()],
            ])
        };
        let lower = SessionScope::derive(SessionPolicy::Thread, ch, false, &mk(&root_lower));
        let upper = SessionScope::derive(SessionPolicy::Thread, ch, false, &mk(&root_upper));
        assert_eq!(
            lower, upper,
            "case-equivalent root spellings must share one thread scope"
        );
        // And the stored key is normalized to lowercase.
        assert_eq!(upper.root_event_id(), Some(root_lower.as_str()));
    }

    #[test]
    fn malformed_thread_tag_falls_back_to_top_level() {
        let ch = Uuid::new_v4();
        // A non-64-hex marker id is ignored by the shared NIP-10 resolver, so
        // the event is treated as top-level (rooted at its own id).
        let ev = event_with_tags(vec![vec![
            "e".into(),
            "not-a-valid-hex-id".into(),
            String::new(),
            "reply".into(),
        ]]);
        let scope = SessionScope::derive(SessionPolicy::Thread, ch, false, &ev);
        assert_eq!(
            scope,
            SessionScope::Thread {
                channel_id: ch,
                root_event_id: ev.id.to_hex(),
            }
        );
    }

    #[test]
    fn accessors_and_labels() {
        let ch = Uuid::new_v4();
        let conv = SessionScope::Conversation { channel_id: ch };
        assert_eq!(conv.channel_id(), ch);
        assert_eq!(conv.root_event_id(), None);
        assert!(!conv.is_thread());
        assert_eq!(conv.telemetry_label(), "conversation");

        let root = "abcdef0123456789".repeat(4); // 64 hex chars
        let thread = SessionScope::Thread {
            channel_id: ch,
            root_event_id: root.clone(),
        };
        assert_eq!(thread.channel_id(), ch);
        assert_eq!(thread.root_event_id(), Some(root.as_str()));
        assert!(thread.is_thread());
        assert_eq!(thread.telemetry_label(), "thread:abcdef01");
    }

    #[test]
    fn scope_is_hashable_and_usable_as_map_key() {
        use std::collections::HashMap;
        let ch = Uuid::new_v4();
        let mut map: HashMap<SessionScope, u32> = HashMap::new();
        let s1 = SessionScope::Thread {
            channel_id: ch,
            root_event_id: "a".repeat(64),
        };
        let s2 = SessionScope::Conversation { channel_id: ch };
        *map.entry(s1.clone()).or_insert(0) += 1;
        *map.entry(s1.clone()).or_insert(0) += 1;
        *map.entry(s2).or_insert(0) += 1;
        assert_eq!(map.get(&s1), Some(&2));
        assert_eq!(map.len(), 2);
    }
}
