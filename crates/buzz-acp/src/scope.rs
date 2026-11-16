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
//! Under [`SessionPolicy::Author`] every row above additionally carries the
//! triggering event's author pubkey (as resolved by the inbound author gate),
//! so two people addressing the agent in the same thread (or the same group
//! DM) talk to two separate sessions.
//!
//! Under [`SessionPolicy::Channel`] (the current default / rollback path) every
//! surface collapses to `Conversation(channel_id)`, preserving today's
//! channel-keyed behavior exactly.

use nostr::Event;
use uuid::Uuid;

use crate::queue::{reply_thread, ResolvedEdit};

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
    /// Per-person: like `thread`, but the session key also carries the author
    /// pubkey of the triggering event. In channels every (thread, author) pair
    /// is its own session; in DMs every (conversation, author) pair is, so a
    /// group DM no longer merges several people into one session.
    Author,
}

impl SessionPolicy {
    /// Append only the configured session model to the shared base instructions.
    /// The resulting base is reused by modern and legacy ACP standing context.
    pub(crate) fn append_session_model(self, base_prompt: &str) -> String {
        let session_model = match self {
            Self::Channel => include_str!("session_model_channel.md"),
            Self::Thread => include_str!("session_model_thread.md"),
            Self::Author => include_str!("session_model_author.md"),
        };
        format!("{}\n\n{}", base_prompt.trim_end(), session_model.trim_end())
    }
}

impl std::fmt::Display for SessionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Channel => f.write_str("channel"),
            Self::Thread => f.write_str("thread"),
            Self::Author => f.write_str("author"),
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
    /// The whole channel is one session. Used for DMs under the `channel` and
    /// `thread` policies, and for every channel event under
    /// [`SessionPolicy::Channel`].
    Conversation { channel_id: Uuid },
    /// A single canonical thread within a channel, keyed by its root event id
    /// (64-char lowercase hex).
    Thread {
        channel_id: Uuid,
        root_event_id: String,
    },
    /// One person's conversation in a DM, keyed by the author pubkey (64-char
    /// lowercase hex). Produced for DMs under [`SessionPolicy::Author`].
    AuthorConversation { channel_id: Uuid, author: String },
    /// One person's view of a single canonical thread, keyed by the thread
    /// root and the author pubkey. Produced for channel events under
    /// [`SessionPolicy::Author`]. Behaves like [`Thread`](Self::Thread) for
    /// every thread-aware consumer (`root_event_id`, `is_thread`).
    AuthorThread {
        channel_id: Uuid,
        root_event_id: String,
        author: String,
    },
}

impl SessionScope {
    /// The channel this scope belongs to. Always available — the channel is the
    /// authorization boundary regardless of scope variant.
    pub fn channel_id(&self) -> Uuid {
        match self {
            Self::Conversation { channel_id } => *channel_id,
            Self::Thread { channel_id, .. } => *channel_id,
            Self::AuthorConversation { channel_id, .. } => *channel_id,
            Self::AuthorThread { channel_id, .. } => *channel_id,
        }
    }

    /// The author pubkey (lowercase hex) this scope is bound to, or `None` for
    /// scopes that are not per-person.
    pub fn author(&self) -> Option<&str> {
        match self {
            Self::Conversation { .. } | Self::Thread { .. } => None,
            Self::AuthorConversation { author, .. } => Some(author),
            Self::AuthorThread { author, .. } => Some(author),
        }
    }

    /// The canonical thread-root event id for a [`Thread`](Self::Thread) scope,
    /// or `None` for a conversation scope.
    pub fn root_event_id(&self) -> Option<&str> {
        match self {
            Self::Conversation { .. } | Self::AuthorConversation { .. } => None,
            Self::Thread { root_event_id, .. } | Self::AuthorThread { root_event_id, .. } => {
                Some(root_event_id)
            }
        }
    }

    /// True when this scope is thread-scoped (not conversation-scoped).
    pub fn is_thread(&self) -> bool {
        matches!(self, Self::Thread { .. } | Self::AuthorThread { .. })
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
    /// 4. Under [`SessionPolicy::Author`], the same resolution runs and the
    ///    result additionally carries the author pubkey (here the event
    ///    signer; see [`derive_routed`](Self::derive_routed)): DMs become
    ///    [`AuthorConversation`](Self::AuthorConversation) and channel events
    ///    become [`AuthorThread`](Self::AuthorThread).
    ///
    /// Thread roots are resolved with [`crate::queue::parse_thread_tags`], i.e. Buzz's shared
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
    ///
    /// Under the `author` policy this binds the session to the event *signer*.
    /// That is fine for owner controls, which only compare surfaces; an
    /// admitted listener event must go through
    /// [`derive_routed`](Self::derive_routed) with the gate-resolved author, or
    /// a relay-signed workflow message would bind to the relay key.
    pub fn derive(policy: SessionPolicy, channel_id: Uuid, is_dm: bool, event: &Event) -> Self {
        Self::derive_routed(
            policy,
            channel_id,
            is_dm,
            event,
            None,
            &event.pubkey.to_hex(),
        )
    }

    /// [`derive`](Self::derive) for an admitted event whose reply routing may
    /// come from an edited original message.
    ///
    /// A kind:40003 edit belongs to its original message's thread, never to a
    /// new thread rooted at the auxiliary edit event: a threaded original
    /// scopes to its root, and a top-level original (or one that could not be
    /// fetched) scopes to the thread rooted at the original itself.
    ///
    /// `author` is the principal the inbound author gate resolved for the
    /// event: the signer, or the attributed owner of a verified relay-signed
    /// workflow message, as lowercase hex from `PublicKey::to_hex`. Only
    /// [`SessionPolicy::Author`] reads it, so a workflow dispatch joins its
    /// owner's session rather than one shared by every workflow the relay
    /// signs.
    pub fn derive_routed(
        policy: SessionPolicy,
        channel_id: Uuid,
        is_dm: bool,
        event: &Event,
        edit: Option<&ResolvedEdit>,
        author: &str,
    ) -> Self {
        let author = (policy == SessionPolicy::Author).then(|| author.to_owned());

        if is_dm || policy == SessionPolicy::Channel {
            return match author {
                Some(author) => Self::AuthorConversation { channel_id, author },
                None => Self::Conversation { channel_id },
            };
        }

        let root_event_id = reply_thread(event, edit);
        match author {
            Some(author) => Self::AuthorThread {
                channel_id,
                root_event_id,
                author,
            },
            None => Self::Thread {
                channel_id,
                root_event_id,
            },
        }
    }

    /// True when `other` names the same conversation surface as `self`: the
    /// same channel and the same thread root (or both conversation-level),
    /// ignoring any per-author binding. Under `channel`/`thread` policy this
    /// is plain equality; under `author` policy it groups the per-person
    /// sessions of one thread, so an owner control (`!cancel`, `!rotate`)
    /// addressed to a thread reaches every person's session in it.
    pub fn same_surface(&self, other: &SessionScope) -> bool {
        self.channel_id() == other.channel_id() && self.root_event_id() == other.root_event_id()
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
            Self::AuthorConversation { author, .. } => {
                let who: String = author.chars().take(8).collect();
                format!("conversation@{who}")
            }
            Self::AuthorThread {
                root_event_id,
                author,
                ..
            } => {
                let short: String = root_event_id.chars().take(8).collect();
                let who: String = author.chars().take(8).collect();
                format!("thread:{short}@{who}")
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
        for policy in [
            SessionPolicy::Channel,
            SessionPolicy::Thread,
            SessionPolicy::Author,
        ] {
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
                SessionPolicy::Author => {
                    assert!(prompt.contains("each (thread, person) pair gets its own"));
                    assert!(prompt.contains("Session author"));
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

    fn thread_root(scope: &SessionScope) -> String {
        match scope {
            SessionScope::Thread { root_event_id, .. } => root_event_id.clone(),
            other => panic!("expected thread scope, got {other:?}"),
        }
    }

    #[test]
    fn edit_scopes_to_original_thread_never_the_edit_event() {
        use crate::edit_routing::test_support::{edit_event, message};
        let ch = Uuid::new_v4();
        let root = "ab".repeat(32);

        // Threaded original: the edit shares the original's thread session.
        let threaded = message(Some(&root));
        let edit = edit_event(&threaded.id.to_hex(), &[]);
        let resolved = ResolvedEdit {
            target_event_id: threaded.id.to_hex(),
            target_thread_tags: crate::queue::parse_thread_tags(&threaded),
        };
        let scope = SessionScope::derive_routed(
            SessionPolicy::Thread,
            ch,
            false,
            &edit,
            Some(&resolved),
            &edit.pubkey.to_hex(),
        );
        assert_eq!(thread_root(&scope), root);

        // Top-level original: the thread rooted at the original itself.
        let top = message(None);
        let edit = edit_event(&top.id.to_hex(), &[]);
        let resolved = ResolvedEdit {
            target_event_id: top.id.to_hex(),
            target_thread_tags: crate::queue::parse_thread_tags(&top),
        };
        let scope = SessionScope::derive_routed(
            SessionPolicy::Thread,
            ch,
            false,
            &edit,
            Some(&resolved),
            &edit.pubkey.to_hex(),
        );
        assert_eq!(thread_root(&scope), top.id.to_hex());

        // Unresolved original: still the target, never the edit event.
        let target = "cd".repeat(32);
        let edit = edit_event(&target, &[]);
        let scope = SessionScope::derive_routed(
            SessionPolicy::Thread,
            ch,
            false,
            &edit,
            None,
            &edit.pubkey.to_hex(),
        );
        assert_eq!(thread_root(&scope), target);
        assert_ne!(thread_root(&scope), edit.id.to_hex());
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

    /// Build a signed event from a specific key with the given NIP-10 tags.
    fn event_from(keys: &Keys, tags: Vec<Vec<String>>) -> Event {
        let tags: Vec<nostr::Tag> = tags
            .into_iter()
            .map(|t| nostr::Tag::parse(t).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::Custom(9), "hello")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn author_policy_dm_is_per_person_conversation() {
        let ch = Uuid::new_v4();
        let alice = Keys::generate();
        let bob = Keys::generate();
        let root = "a".repeat(64);
        let reply_tags = || {
            vec![
                vec!["e".into(), root.clone(), String::new(), "root".into()],
                vec!["e".into(), "b".repeat(64), String::new(), "reply".into()],
            ]
        };
        let a1 = SessionScope::derive(SessionPolicy::Author, ch, true, &event_from(&alice, vec![]));
        let a2 = SessionScope::derive(
            SessionPolicy::Author,
            ch,
            true,
            &event_from(&alice, reply_tags()),
        );
        let b = SessionScope::derive(
            SessionPolicy::Author,
            ch,
            true,
            &event_from(&bob, reply_tags()),
        );
        assert_eq!(
            a1,
            SessionScope::AuthorConversation {
                channel_id: ch,
                author: alice.public_key().to_hex(),
            }
        );
        // Thread tags inside a DM do not split one person's conversation.
        assert_eq!(a1, a2);
        // A second person in the same (group) DM gets a separate session.
        assert_ne!(a1, b);
        assert_eq!(b.author(), Some(bob.public_key().to_hex().as_str()));
        assert!(!a1.is_thread());
        assert_eq!(a1.root_event_id(), None);
        assert_eq!(a1.channel_id(), ch);
    }

    #[test]
    fn author_policy_channel_thread_is_per_person_and_per_root() {
        let ch = Uuid::new_v4();
        let alice = Keys::generate();
        let bob = Keys::generate();
        let root = "c".repeat(64);
        let reply = |keys: &Keys| {
            event_from(
                keys,
                vec![
                    vec![
                        "e".into(),
                        root.to_ascii_uppercase(),
                        String::new(),
                        "root".into(),
                    ],
                    vec!["e".into(), "d".repeat(64), String::new(), "reply".into()],
                ],
            )
        };
        let a1 = SessionScope::derive(SessionPolicy::Author, ch, false, &reply(&alice));
        let a2 = SessionScope::derive(SessionPolicy::Author, ch, false, &reply(&alice));
        let b = SessionScope::derive(SessionPolicy::Author, ch, false, &reply(&bob));
        assert_eq!(
            a1,
            SessionScope::AuthorThread {
                channel_id: ch,
                root_event_id: root.clone(),
                author: alice.public_key().to_hex(),
            }
        );
        // Same person, same thread → same session (root normalized to lowercase).
        assert_eq!(a1, a2);
        // Another person in the same thread → a distinct session.
        assert_ne!(a1, b);
        // Thread-aware consumers see the same root either way.
        assert_eq!(a1.root_event_id(), b.root_event_id());
        assert_eq!(a1.root_event_id(), Some(root.as_str()));
        assert!(a1.is_thread());
        assert_eq!(a1.author(), Some(alice.public_key().to_hex().as_str()));

        // A top-level mention opens a per-person thread rooted at the trigger.
        let top = event_from(&alice, vec![]);
        let scope = SessionScope::derive(SessionPolicy::Author, ch, false, &top);
        assert_eq!(
            scope,
            SessionScope::AuthorThread {
                channel_id: ch,
                root_event_id: top.id.to_hex(),
                author: alice.public_key().to_hex(),
            }
        );
    }

    #[test]
    fn author_policy_labels_and_non_author_scopes_have_no_author() {
        let ch = Uuid::new_v4();
        let root = "abcdef0123456789".repeat(4);
        let who = "0123456789abcdef".repeat(4);
        let conv = SessionScope::AuthorConversation {
            channel_id: ch,
            author: who.clone(),
        };
        assert_eq!(conv.telemetry_label(), "conversation@01234567");
        let thread = SessionScope::AuthorThread {
            channel_id: ch,
            root_event_id: root.clone(),
            author: who.clone(),
        };
        assert_eq!(thread.telemetry_label(), "thread:abcdef01@01234567");
        assert_eq!(SessionScope::Conversation { channel_id: ch }.author(), None);
        assert_eq!(
            SessionScope::Thread {
                channel_id: ch,
                root_event_id: root,
            }
            .author(),
            None
        );
        assert_eq!(SessionPolicy::Author.to_string(), "author");
    }

    #[test]
    fn same_surface_ignores_author_but_not_root_or_channel() {
        let ch = Uuid::new_v4();
        let root = "a".repeat(64);
        let alice = "1".repeat(64);
        let bob = "2".repeat(64);
        let plain = SessionScope::Thread {
            channel_id: ch,
            root_event_id: root.clone(),
        };
        let a = SessionScope::AuthorThread {
            channel_id: ch,
            root_event_id: root.clone(),
            author: alice.clone(),
        };
        let b = SessionScope::AuthorThread {
            channel_id: ch,
            root_event_id: root.clone(),
            author: bob.clone(),
        };
        assert!(a.same_surface(&b));
        assert!(a.same_surface(&plain));
        assert!(plain.same_surface(&plain));
        assert!(!a.same_surface(&SessionScope::AuthorThread {
            channel_id: ch,
            root_event_id: "b".repeat(64),
            author: alice.clone(),
        }));
        assert!(!a.same_surface(&SessionScope::AuthorThread {
            channel_id: Uuid::new_v4(),
            root_event_id: root.clone(),
            author: alice.clone(),
        }));
        // Conversation-level scopes group by channel only.
        let conv = SessionScope::Conversation { channel_id: ch };
        let conv_a = SessionScope::AuthorConversation {
            channel_id: ch,
            author: alice,
        };
        let conv_b = SessionScope::AuthorConversation {
            channel_id: ch,
            author: bob,
        };
        assert!(conv_a.same_surface(&conv_b));
        assert!(conv_a.same_surface(&conv));
        assert!(!conv_a.same_surface(&a));
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
