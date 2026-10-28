use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum independent operations in one HTTP request.
pub const MAX_INTENTS: usize = 100;
/// Default time an implicit thread follow stays live without the actor's
/// involvement; not event or encrypted NIP-RS retention.
pub const DEFAULT_RETENTION_SECONDS: u32 = 30 * 24 * 60 * 60;

/// A channel or canonical thread; absence of a root denotes only the channel timeline.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadTarget {
    /// Channel UUID, interpreted only in the authenticated community.
    pub channel_id: Uuid,
    /// Canonical thread-root event ID, when targeting one thread.
    pub root_id: Option<String>,
}

/// Fixed operands make retries converge without a server operation journal.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadIntent {
    /// Advance a context through one fixed message, including equal arrivals.
    MarkThrough {
        /// Channel or canonical thread being marked.
        target: ReadTarget,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
    /// Follow a thread. A new follow, or one after an unfollow, starts now;
    /// an existing follow keeps its position.
    Follow {
        /// The thread; a channel-only target is invalid.
        target: ReadTarget,
    },
    /// Stop following a thread until the actor posts in it or is mentioned
    /// there again.
    Unfollow {
        /// The thread; a channel-only target is invalid.
        target: ReadTarget,
    },
}

impl ReadIntent {
    /// The intent's target.
    pub fn target(&self) -> &ReadTarget {
        match self {
            ReadIntent::MarkThrough { target, .. }
            | ReadIntent::Follow { target }
            | ReadIntent::Unfollow { target } => target,
        }
    }
}

/// Outcome for one independent transaction, never acknowledged before commit.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IntentOutcome {
    /// The fixed frontier operand committed.
    Applied,
    /// Missing and forbidden contexts deliberately share one outcome.
    Blocked,
    /// Invalid operands; no changes committed for this intent.
    Invalid,
}

/// The tracking boundary for the authenticated account. Not on the wire.
#[derive(Clone, Debug)]
pub(super) struct ReadAccount {
    /// Implicit follows not involved since this instant (Unix milliseconds) are
    /// dormant; nothing is discarded.
    pub(super) cutoff_ms: i64,
    /// Arrival floor of every frontier: the account's first applied read
    /// intent. `None` until then, and nothing counts as unread.
    pub(super) started_at: Option<DateTime<Utc>>,
    /// Applied intents so far, in the same snapshot.
    pub(super) revision: i64,
}

/// Maximum channel summaries in one sidebar page.
pub const MAX_CHANNELS: usize = 20;
/// Maximum thread rows per channel row. A POST can return rows for
/// `MAX_INTENTS` channels, and they must fit the 1 MiB response limit.
pub const MAX_THREAD_SUMMARIES: usize = 25;
/// Counts stop here: a count of 100 means at least 100.
pub const MAX_UNREAD_COUNT: usize = 100;
/// The relay accepts author times up to this far from arrival either way.
/// Read positions are arrival times and mentions are indexed by author time,
/// so a mention walk starts this much before the position.
pub const MAX_ARRIVAL_SKEW_SECONDS: u32 = 15 * 60;
/// Conversation kinds eligible for ordinary unread state (not edits/reactions).
pub const ELIGIBLE_KINDS: [i32; 4] = [9, 40002, 45001, 45003];

/// One joined channel's timeline read state. A channel and each of its
/// threads keep independent read positions.
#[derive(Debug, Serialize)]
pub struct ChannelReadSummary {
    /// Joined channel UUID.
    pub channel_id: Uuid,
    /// Whether an unread top-level message was found. Thread replies count
    /// only on their thread row.
    pub unread: bool,
    /// Unread top-level messages directed at the actor: every one in a DM,
    /// otherwise those that tag the actor with `p`.
    pub mentions: u32,
    /// The anchor of the timeline's frontier: the marked or own top-level
    /// message that arrived last. None until either exists.
    pub read_through_id: Option<String>,
    /// Last eligible top-level message to arrive, whatever its author or read
    /// progress: marking the timeline through it reads the timeline.
    pub latest_id: Option<String>,
    /// Followed threads with unread replies, newest arrival first.
    pub threads: Vec<ThreadReadSummary>,
    /// Whether unread followed threads past `MAX_THREAD_SUMMARIES` were left
    /// out of `threads`.
    pub threads_truncated: bool,
    /// Every live (explicit, or not dormant) follow in this channel, read or
    /// not and including threads without a reply yet, newest reply first, at
    /// most [`MAX_FOLLOWED_THREADS`]: the relay's follow decision. A root not
    /// listed here is not followed unless `followed_threads_truncated`.
    /// Omitted (None) from write responses, which would otherwise exceed the
    /// response limit; GET sidebar rows always carry it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_threads: Option<Vec<String>>,
    /// Whether the cap cut `followed_threads` off. Absent with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followed_threads_truncated: Option<bool>,
}

/// Maximum roots in a channel row's `followed_threads`.
pub const MAX_FOLLOWED_THREADS: usize = 100;

/// One thread's read state within a channel row. No conversation bytes.
#[derive(Debug, Deserialize, Serialize)]
pub struct ThreadReadSummary {
    /// Canonical thread-root event ID.
    pub root_id: String,
    /// Whether the last reply to arrive is unread.
    pub unread: bool,
    /// Unread replies directed at the actor: every one in a DM, otherwise
    /// those that tag the actor with `p`.
    pub mentions: u32,
    /// The anchor of the thread's frontier: the marked or own reply that
    /// arrived last. None for a follower who has read nothing yet.
    pub read_through_id: Option<String>,
    /// Last reply to arrive: marking through it reads the thread.
    pub latest_id: String,
}

/// A bounded roster page, with no cross-page snapshot or removal inference.
#[derive(Debug, Serialize)]
pub struct SidebarPage {
    /// Joined channels only, never every accessible public channel.
    pub channels: Vec<ChannelReadSummary>,
    /// Exclusive UUID roster cursor. None means this roster scan exhausted.
    pub next_cursor: Option<Uuid>,
    /// The actor's applied-intent count in this page's snapshot.
    #[serde(rename = "read_revision")]
    pub revision: i64,
}

/// Maximum rows in one threads page.
pub const MAX_THREADS: usize = 50;

/// One followed thread in the threads view.
#[derive(Debug, Serialize)]
pub struct FollowedThread {
    /// Joined channel UUID.
    pub channel_id: Uuid,
    /// The thread's read state; `unread` may be false here.
    #[serde(flatten)]
    pub thread: ThreadReadSummary,
}

/// A page of followed threads, newest arrival first.
#[derive(Debug, Serialize)]
pub struct ThreadsPage {
    /// Followed threads in joined channels.
    pub threads: Vec<FollowedThread>,
    /// Opaque exclusive cursor. None means this scan exhausted.
    pub next_cursor: Option<String>,
    /// The actor's applied-intent count in this page's snapshot.
    #[serde(rename = "read_revision")]
    pub revision: i64,
}

/// Where a threads page resumes: after this arrival and root.
#[derive(Clone, Debug)]
pub struct ThreadsCursor {
    pub(super) latest_at: DateTime<Utc>,
    pub(super) root_id: Vec<u8>,
}

impl ThreadsCursor {
    /// Parse a `next_cursor` value; None if it is not one.
    pub fn parse(value: &str) -> Option<Self> {
        let (micros, root) = value.split_once('.')?;
        Some(Self {
            latest_at: DateTime::from_timestamp_micros(micros.parse().ok()?)?,
            root_id: super::writes::event_id(root)?,
        })
    }
}
