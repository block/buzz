//! Thread-id contract for Buzz messages: channel vs thread-in-channel.
//!
//! A Buzz message is not always addressed to a channel as a whole.
//! It can be addressed to a *thread in that channel* — a reply chain
//! rooted at one top-level event. The bridge must carry that
//! distinction through, or two different conversations in the same
//! channel will be conflated, and a reply will land at the top of the
//! channel instead of in the thread it answers.
//!
//! ## The contract
//!
//! * **Channel** identifies *where* a message lives: a channel name
//!   (e.g. `MASKY-CMO`) and, when buzz-acp frames it, a channel UUID
//!   (`#3043c06d-6cf1-42f8-b1b0-cb2d61711ea1`). A channel maps 1:1 to
//!   a Muse side chat (see [`crate::mapping`]).
//! * **Thread** identifies *which conversation within that channel* a
//!   message belongs to: `thread_root` is the full event ID of the
//!   thread's top-level event, and `reply_to` is the full event ID of
//!   the message being answered (the immediate parent, which may be
//!   the root itself for a first reply).
//! * **Thread is not a mapping key.** Two threads in the same channel
//!   route to the *same* side chat, with different thread context.
//!   Creating a side chat per thread would fragment the channel's
//!   conversation and break the 1:1 channel mapping. The side-chat
//!   key is the channel (or, for a DM, the DM conversation) alone;
//!   the thread is carried as per-turn context.
//! * **Session-title thread hints are truncated.** buzz-acp composes
//!   thread-scoped session titles as `Agent · #channel · <8-char
//!   thread root>`. Those 8 characters are a hint, not an ID: they can
//!   confirm a full thread root from the prompt (prefix match), but
//!   they can never substitute for it when correlating a reply. A
//!   full thread root must come from the prompt's `Thread root:` /
//!   `Thread:` line, or from the triggering event's own ID.
//! * **Correlation rule.** A reply to a message with event ID `E` in
//!   thread rooted at `R` is threaded to `E` (Buzz `--reply-to E`),
//!   and its thread root is `R`. A reply to a top-level message `E`
//!   (no thread root) is threaded to `E`, and its thread root becomes
//!   `E` (it starts the thread). A message with no event ID at all
//!   has no thread correlation: its reply is channel top-level.
//!
//! ## Where thread IDs come from
//!
//! buzz-acp frames each wake with a `Channel:` line and, for threaded
//! wakes, thread context in the prompt (see `parse_thread_from_text`
//! for the exact lines accepted). The ACP layer stores the parsed
//! thread context per session and updates it per turn, exactly as it
//! already does for the channel — a session reused across threads
//! must route to *this* wake's thread, not the first one it ever saw.
//! DMs use the same rule with a DM conversation instead of a channel:
//! see [`crate::dm`] for the DM event shape, which shares this
//! thread-root definition (`thread_root_or_self`).

use crate::muse_web::{parse_channel_from_session_title, parse_channel_from_text};

/// Thread context for one Buzz wake: channel, thread, and reply target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadContext {
    /// Buzz channel name, when known (e.g. `MASKY-CMO`).
    pub channel: Option<String>,
    /// Buzz channel UUID, when the framing carries one.
    pub channel_id: Option<String>,
    /// Full event ID of the thread's top-level event, when known.
    pub thread_root: Option<String>,
    /// Full event ID of the message being answered, when known.
    pub reply_to: Option<String>,
    /// True when this wake is a DM conversation, not a channel.
    pub is_dm: bool,
}

impl ThreadContext {
    /// The side-chat key for this wake: the channel name for a channel
    /// wake, or the DM channel UUID for a DM wake. The thread never
    /// appears in the key — see the module docs.
    pub fn side_chat_key(&self) -> Option<String> {
        if self.is_dm {
            self.channel_id.clone()
        } else {
            self.channel.clone()
        }
    }

    /// True when this wake is addressed to a thread (not just to the
    /// channel as a whole).
    pub fn is_threaded(&self) -> bool {
        self.thread_root.is_some() || self.reply_to.is_some()
    }

    /// The thread root this wake belongs to, for correlation: the
    /// explicit root, or the reply target itself when the wake is a
    /// top-level message (replying to it starts its thread).
    pub fn correlation_thread_root(&self) -> Option<String> {
        self.thread_root.clone().or_else(|| self.reply_to.clone())
    }
}

/// The correlated reply for one wake: where the reply must land in Buzz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadedReply {
    /// Buzz `--reply-to` target: the triggering event, when known.
    pub reply_to_event_id: Option<String>,
    /// Thread root of the reply: the wake's thread root, or the
    /// triggering event when the wake was top-level.
    pub thread_root: Option<String>,
    /// Channel name, when known.
    pub channel: Option<String>,
    /// Channel UUID, when known.
    pub channel_id: Option<String>,
}

/// Build the correlated reply for a wake, following the correlation
/// rule in the module docs. The reply text itself is not part of this
/// struct: the caller pairs it with these IDs when publishing.
pub fn correlated_reply(ctx: &ThreadContext) -> ThreadedReply {
    ThreadedReply {
        reply_to_event_id: ctx.reply_to.clone(),
        thread_root: ctx.correlation_thread_root(),
        channel: ctx.channel.clone(),
        channel_id: ctx.channel_id.clone(),
    }
}

/// Parse a thread context out of prompt / system-prompt text.
///
/// Accepted lines (first non-empty wins per field, any order):
///
/// ```text
/// Channel: MASKY-CMO (#3043c06d-6cf1-42f8-b1b0-cb2d61711ea1)
/// DM channel: <uuid>
/// Thread: <full event id>
/// Thread root: <full event id>
/// Thread id: <full event id>
/// Reply to: <full event id>
/// Reply-to: <full event id>
/// Event: <full event id>
/// ```
///
/// `Event:` is treated as the reply target (the triggering event)
/// when no explicit `Reply to:` line is present. A `Channel:` line
/// also supplies the channel UUID suffix, when present. Returns a
/// default (all-`None`) context when no line matches, rather than
/// `None`, so callers can always carry a context through — an empty
/// context means "channel top-level, no thread correlation".
pub fn parse_thread_from_text(text: &str) -> ThreadContext {
    let mut ctx = ThreadContext::default();
    if let Some(channel) = parse_channel_from_text(text) {
        ctx.channel = Some(channel);
    }
    // Channel UUID suffix, from the same `Channel:` line.
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Channel:") {
            let rest = rest.trim();
            if let Some(start) = rest.find("(#") {
                let after = &rest[start + 2..];
                if let Some(end) = after.find(')') {
                    let uuid = after[..end].trim();
                    if !uuid.is_empty() {
                        ctx.channel_id = Some(uuid.to_string());
                    }
                }
            }
            break;
        }
    }
    for line in text.lines() {
        let trimmed = line.trim();
        let field = |prefix: &str| trimmed.strip_prefix(prefix).map(str::trim);
        if let Some(value) = field("DM channel:") {
            if !value.is_empty() {
                ctx.is_dm = true;
                ctx.channel_id = Some(value.to_string());
            }
        } else if let Some(value) = field("Thread root:")
            .or_else(|| field("Thread id:"))
            .or_else(|| field("Thread:"))
        {
            if !value.is_empty() && ctx.thread_root.is_none() {
                ctx.thread_root = Some(value.to_string());
            }
        } else if let Some(value) = field("Reply to:").or_else(|| field("Reply-to:")) {
            if !value.is_empty() && ctx.reply_to.is_none() {
                ctx.reply_to = Some(value.to_string());
            }
        } else if let Some(value) = field("Event:") {
            // Triggering event: the reply target, unless an explicit
            // `Reply to:` line already supplied one.
            if !value.is_empty() && ctx.reply_to.is_none() {
                ctx.reply_to = Some(value.to_string());
            }
        }
    }
    ctx
}

/// Parse a session title into (channel, 8-char thread-root prefix).
///
/// Titles are `Agent · #channel`, or, for thread-scoped sessions,
/// `Agent · #channel · <8-char thread root>`. The third segment, when
/// present and exactly 8 characters, is the prefix — a hint to be
/// checked against a full root, never a root by itself.
pub fn parse_thread_from_session_title(title: &str) -> (Option<String>, Option<String>) {
    let channel = parse_channel_from_session_title(title);
    let parts: Vec<&str> = title.split('·').map(str::trim).collect();
    let prefix = parts.get(2).filter(|s| s.len() == 8).map(|s| s.to_string());
    (channel, prefix)
}

/// True when a full thread root matches an 8-char session-title
/// prefix (case-insensitive prefix match). A mismatch means the title
/// hint is stale or for a different thread: the full root wins.
pub fn thread_root_matches_prefix(full_root: &str, prefix: &str) -> bool {
    full_root.len() >= prefix.len() && full_root[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_context_from_channel_and_thread_lines() {
        let ctx = parse_thread_from_text(
            "Channel: MASKY-CMO (#3043c06d-6cf1-42f8-b1b0-cb2d61711ea1)\nThread root: aaaabbbbccccdddd\nReply to: 1111222233334444\nMessage: hi",
        );
        assert_eq!(ctx.channel.as_deref(), Some("MASKY-CMO"));
        assert_eq!(
            ctx.channel_id.as_deref(),
            Some("3043c06d-6cf1-42f8-b1b0-cb2d61711ea1")
        );
        assert_eq!(ctx.thread_root.as_deref(), Some("aaaabbbbccccdddd"));
        assert_eq!(ctx.reply_to.as_deref(), Some("1111222233334444"));
        assert!(ctx.is_threaded());
        // Thread does not change the side-chat key: it stays the channel.
        assert_eq!(ctx.side_chat_key().as_deref(), Some("MASKY-CMO"));
    }

    #[test]
    fn same_channel_different_threads_share_side_chat_key() {
        let a = parse_thread_from_text("Channel: MASKY-CMO\nThread root: root-a\nMessage: hi");
        let b = parse_thread_from_text("Channel: MASKY-CMO\nThread root: root-b\nMessage: hi");
        assert_ne!(a.thread_root, b.thread_root);
        assert_eq!(a.side_chat_key(), b.side_chat_key());
    }

    #[test]
    fn top_level_message_has_no_thread_but_correlates_to_itself() {
        let ctx = parse_thread_from_text("Channel: MASKY-CMO\nEvent: top-event-1\nMessage: hi");
        assert!(!ctx.thread_root.is_some());
        assert_eq!(ctx.reply_to.as_deref(), Some("top-event-1"));
        let reply = correlated_reply(&ctx);
        assert_eq!(reply.reply_to_event_id.as_deref(), Some("top-event-1"));
        // Replying to a top-level message starts its thread.
        assert_eq!(reply.thread_root.as_deref(), Some("top-event-1"));
    }

    #[test]
    fn threaded_reply_keeps_root_and_targets_parent() {
        let ctx = parse_thread_from_text(
            "Channel: MASKY-CMO\nThread root: root-1\nReply to: parent-2\nMessage: hi",
        );
        let reply = correlated_reply(&ctx);
        assert_eq!(reply.reply_to_event_id.as_deref(), Some("parent-2"));
        assert_eq!(reply.thread_root.as_deref(), Some("root-1"));
    }

    #[test]
    fn no_thread_lines_means_channel_top_level() {
        let ctx = parse_thread_from_text("Channel: MASKY-CMO\nMessage: hi");
        assert!(!ctx.is_threaded());
        assert_eq!(ctx.side_chat_key().as_deref(), Some("MASKY-CMO"));
        let reply = correlated_reply(&ctx);
        assert_eq!(reply.reply_to_event_id, None);
        assert_eq!(reply.thread_root, None);
    }

    #[test]
    fn session_title_thread_prefix_is_a_hint_not_an_id() {
        let (channel, prefix) = parse_thread_from_session_title("Muse · #MASKY-CMO · abcd1234");
        assert_eq!(channel.as_deref(), Some("MASKY-CMO"));
        assert_eq!(prefix.as_deref(), Some("abcd1234"));
        assert!(thread_root_matches_prefix("abcd1234eeeeffff", "abcd1234"));
        assert!(!thread_root_matches_prefix("zzzz1234eeeeffff", "abcd1234"));
        let (channel, prefix) = parse_thread_from_session_title("Muse · #MASKY-CMO");
        assert_eq!(channel.as_deref(), Some("MASKY-CMO"));
        assert_eq!(prefix, None);
    }

    #[test]
    fn dm_thread_uses_dm_channel_as_key_not_a_channel_name() {
        let ctx = parse_thread_from_text(
            "DM channel: dm-uuid-1\nThread root: root-1\nEvent: ev-1\nMessage: hi",
        );
        assert!(ctx.is_dm);
        assert_eq!(ctx.side_chat_key().as_deref(), Some("dm-uuid-1"));
        assert_eq!(ctx.correlation_thread_root().as_deref(), Some("root-1"));
    }
}
