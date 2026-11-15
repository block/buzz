use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum independent operations in one HTTP request.
pub const MAX_INTENTS: usize = 100;
/// Default unread-tracking duration, not event or encrypted NIP-RS retention.
pub const DEFAULT_RETENTION_SECONDS: u32 = 30 * 24 * 60 * 60;
/// Message kinds that count as activity and can be unread or marked through
/// (not edits or reactions).
pub const ELIGIBLE_KINDS: [i32; 4] = [9, 40002, 45001, 45003];

/// A channel or canonical thread; absence of a root denotes only the channel timeline.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ReadTarget {
    /// Channel UUID, interpreted only in the authenticated community.
    pub channel_id: Uuid,
    /// Canonical thread-root event ID, when targeting one thread.
    pub root_id: Option<String>,
}

/// How much of a conversation or thread reaches the person.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadState {
    /// A conversation is unread on any new message; a thread while the
    /// person is part of it.
    Default,
    /// Never unread; mentions are still counted.
    Muted,
}

impl ReadState {
    /// The stored and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ReadState::Default => "default",
            ReadState::Muted => "muted",
        }
    }
}

/// Fixed operands make retries converge without a server operation journal.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadIntent {
    /// Advance a context through one fixed message, including equal
    /// arrivals, and clear a manual unread. The position never moves back.
    MarkThrough {
        /// Channel or canonical thread being marked.
        target: ReadTarget,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
    /// Flag a context unread until the next `mark_through`.
    MarkUnread {
        /// Channel or canonical thread being flagged.
        target: ReadTarget,
    },
    /// Follow, mute or reset a context.
    SetState {
        /// Channel or canonical thread.
        target: ReadTarget,
        /// The new state.
        state: ReadState,
    },
}

impl ReadIntent {
    /// The context the intent changes.
    pub fn target(&self) -> &ReadTarget {
        match self {
            ReadIntent::MarkThrough { target, .. }
            | ReadIntent::MarkUnread { target }
            | ReadIntent::SetState { target, .. } => target,
        }
    }
}

/// Outcome for one independent transaction, never acknowledged before commit.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IntentOutcome {
    /// The intent committed.
    Applied,
    /// Missing and forbidden contexts deliberately share one outcome.
    Blocked,
    /// Invalid operands; no changes committed for this intent.
    Invalid,
}
