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
    /// Read one message without moving the context's read position. Only
    /// what the list draws per message changes: a mention of the person
    /// stops being unread. A message that does not tag them changes nothing.
    MarkRead {
        /// Channel or canonical thread the message belongs to.
        target: ReadTarget,
        /// The message read.
        message_id: String,
    },
    /// Flag a context unread until the next `mark_through`. With a message,
    /// also move the read position back to just before it, if it had
    /// passed it: that message and everything after it is unread again,
    /// mentions included. Nothing before it is read by this.
    MarkUnread {
        /// Channel or canonical thread being flagged.
        target: ReadTarget,
        /// The first message to show unread.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    /// Carry a NIP-RS read frontier over: advance the context through its
    /// newest message signed at or before `through` (Unix seconds, the
    /// NIP-RS frontier's own clock). Never moves the position back and never
    /// clears a manual unread, so it is safe to replay from every device.
    ImportThrough {
        /// Channel or canonical thread the frontier belongs to.
        target: ReadTarget,
        /// The NIP-RS frontier.
        through: u32,
    },
    /// Record that this person's NIP-RS read state has been carried over.
    /// Until then a client does not treat the relay as the source of truth.
    FinishImport {},
    /// Follow, mute or reset a context.
    SetState {
        /// Channel or canonical thread.
        target: ReadTarget,
        /// The new state.
        state: ReadState,
    },
}

impl ReadIntent {
    /// The context the intent changes; none for an account-wide intent.
    pub fn target(&self) -> Option<&ReadTarget> {
        match self {
            ReadIntent::MarkThrough { target, .. }
            | ReadIntent::MarkRead { target, .. }
            | ReadIntent::MarkUnread { target, .. }
            | ReadIntent::ImportThrough { target, .. }
            | ReadIntent::SetState { target, .. } => Some(target),
            ReadIntent::FinishImport {} => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn intents_parse_strictly() {
        let target = json!({"channel_id": Uuid::nil()});
        let parse = |value: serde_json::Value| serde_json::from_value::<ReadIntent>(value);
        assert!(matches!(
            parse(json!({"type":"mark_unread","target":target})),
            Ok(ReadIntent::MarkUnread {
                message_id: None,
                ..
            })
        ));
        assert!(matches!(
            parse(json!({"type":"mark_unread","target":target,"message_id":"ab"})),
            Ok(ReadIntent::MarkUnread {
                message_id: Some(_),
                ..
            })
        ));
        assert!(matches!(
            parse(json!({"type":"mark_read","target":target,"message_id":"ab"})),
            Ok(ReadIntent::MarkRead { .. })
        ));
        assert!(parse(json!({"type":"mark_read","target":target})).is_err());
        assert!(matches!(
            parse(json!({"type":"import_through","target":target,"through":1_700_000_000u32})),
            Ok(ReadIntent::ImportThrough {
                through: 1_700_000_000,
                ..
            })
        ));
        assert!(parse(json!({"type":"import_through","target":target,"through":-1})).is_err());
        let finish = parse(json!({"type":"finish_import"})).expect("finish");
        assert!(finish.target().is_none());
        assert!(parse(json!({"type":"finish_import","target":target})).is_err());
    }
}
