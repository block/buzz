use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum independent operations in one HTTP request.
pub const MAX_INTENTS: usize = 100;
/// Default unread-tracking duration, not event or encrypted NIP-RS retention.
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
    /// Advance the channel timeline and every thread in it through one fixed
    /// message's arrival time. The anchor may be a reply; ancestry is irrelevant.
    MarkChannelRead {
        /// Channel being marked, including all of its threads.
        channel_id: Uuid,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
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

/// The tracking boundary for the authenticated account.
#[derive(Clone, Debug, Serialize)]
pub struct ReadAccount {
    /// Configured tracking duration in seconds.
    pub retention_seconds: u32,
    /// Read-time author-time cutoff (Unix milliseconds), not a discard boundary.
    pub cutoff_ms: i64,
    /// Arrival floor of every frontier: the account's first applied read
    /// intent. `None` until then, and nothing counts as unread. Not on the wire.
    #[serde(skip)]
    pub started_at: Option<DateTime<Utc>>,
}

impl ReadAccount {
    /// The arrival every frontier is floored at: the start, or the end of time
    /// for an account that has not started.
    pub(super) fn floor(&self) -> DateTime<Utc> {
        self.started_at.unwrap_or(DateTime::<Utc>::MAX_UTC)
    }
}

/// Maximum channel summaries in one sidebar page.
pub const MAX_CHANNELS: usize = 20;
/// Maximum unread-thread summaries per channel row.
pub const MAX_THREAD_SUMMARIES: usize = 5;
/// Bounded event evidence per channel; exhaustion is never inferred at this cap.
pub const MAX_CHANNEL_SCAN: usize = 256;
/// Unread-window work budget per channel, before eligibility/ancestry joins.
pub const MAX_UNREAD_SCAN: usize = 4096;
/// Conversation kinds eligible for ordinary unread state (not edits/reactions).
pub const ELIGIBLE_KINDS: [i32; 4] = [9, 40002, 45001, 45003];

/// An honest aggregate: capped evidence cannot establish exact zero.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadCount {
    /// Exhausted the authoritative candidate set.
    Exact {
        /// Total within the tracking horizon.
        value: u32,
    },
    /// Incomplete evidence establishes no positive lower bound. No numeric value.
    Unknown,
    /// More evidence exists or ancestry/membership could not be proved.
    AtLeast {
        /// Proven lower bound, not a fabricated badge cap.
        value: u32,
    },
}

impl ReadCount {
    pub(super) fn from_evidence(value: u32, complete: bool) -> Self {
        if complete {
            Self::Exact { value }
        } else if value == 0 {
            Self::Unknown
        } else {
            Self::AtLeast { value }
        }
    }
}

/// One joined-channel summary, not a second conversation/history API.
#[derive(Debug, Serialize)]
pub struct ChannelReadSummary {
    /// Joined channel UUID.
    pub channel_id: Uuid,
    /// Existing channel name.
    pub name: String,
    /// Existing channel type.
    pub channel_type: String,
    /// Archived channels stay in the roster; presentation remains client-owned.
    pub archived: bool,
    /// Existing DM visibility preference (not an authorization decision).
    pub hidden: bool,
    /// Unread messages that count: every top-level message, and a reply only
    /// when it has a [`Reason`]. Other replies are not unread at all.
    pub unread: ReadCount,
    /// The unread subset with a [`Reason`]: everything but ordinary top-level
    /// messages. This is not Desktop notification eligibility: follows and
    /// mutes do not change this count.
    pub attention: ReadCount,
    /// Last eligible nondeleted event to arrive, inside the unread horizon when
    /// any is, whatever its author or read progress: marking through it reads
    /// the row.
    /// None proves absence only when latest_message_complete is true.
    pub latest_message_id: Option<String>,
    /// Display activity: the greatest author time (Unix seconds) among eligible
    /// nondeleted events, not necessarily latest_message_id's own. None exactly
    /// when it is None.
    pub latest_message_at: Option<i64>,
    /// Whether the latest lookup found a result or exhausted channel history.
    /// False means the bounded probe found none, but an unexamined tail remains.
    pub latest_message_complete: bool,
    /// Threads with unread replies, newest unread reply first.
    pub threads: ThreadSummaries,
}

/// A bounded, ordered list of unread threads within one channel row.
#[derive(Debug, Serialize)]
pub struct ThreadSummaries {
    /// At most MAX_THREAD_SUMMARIES, by latest_reply_at DESC then root_id ASC.
    pub items: Vec<ThreadReadSummary>,
    /// True only when evidence was exhausted and no thread was omitted.
    pub complete: bool,
}

/// Unread replies in one canonical thread; every one has a [`Reason`]. No
/// conversation bytes.
#[derive(Debug, Serialize)]
pub struct ThreadReadSummary {
    /// Canonical thread-root event ID.
    pub root_id: String,
    /// Unread replies in this thread (same definition as the row count).
    pub unread: ReadCount,
    /// Last observed unread reply to arrive: marking through it reads the thread.
    pub latest_reply_id: String,
    /// Author time (Unix seconds) of latest_reply_id.
    pub latest_reply_at: i64,
}

/// A bounded roster page, with no cross-page snapshot or removal inference.
#[derive(Debug, Serialize)]
pub struct SidebarPage {
    /// Effective read-state lifecycle for this response.
    pub account: ReadAccount,
    /// Joined channels only, never every accessible public channel.
    pub channels: Vec<ChannelReadSummary>,
    /// Exclusive UUID roster cursor. None means this roster scan exhausted.
    pub next_cursor: Option<Uuid>,
}

/// Maximum explicit contexts in one request.
pub const MAX_CONTEXTS: usize = 20;
/// Maximum explicit message selectors across the entire context request.
pub const MAX_CONTEXT_MESSAGES: usize = 100;

/// A context and concrete messages already known through Nostr history/live reads.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextQuery {
    /// Channel timeline or canonical thread, never an arbitrary filter.
    pub target: ReadTarget,
    /// Optional concrete message selectors; not an event history query.
    #[serde(default)]
    pub message_ids: Vec<String>,
}

/// Why an unread message is directed at the actor: the first that holds.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Its channel is a DM.
    Direct,
    /// It tags the actor with `p`.
    Mention,
    /// It replies to a message the actor wrote, or to one the actor also
    /// replied to, in the same channel. Only live eligible messages qualify.
    Conversation,
    /// It carries `broadcast=1`.
    Broadcast,
}

/// Read progress and eligibility for one concrete message, not a public receipt.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MessageReadState {
    /// Missing, inaccessible, or outside the requested context. No existence oracle.
    Unavailable,
    /// Evidence cannot safely establish ancestry, eligibility or membership.
    Unknown,
    /// Not unread: own, deleted, auxiliary, outside the horizon, or a reply
    /// with no [`Reason`].
    NotCounted,
    /// Covered by this context's frontier.
    Read,
    /// Counts, and is beyond this context's frontier.
    Unread {
        /// Null only for an ordinary top-level message. A broadcast reply whose
        /// membership is undecided reports `broadcast`.
        reason: Option<Reason>,
    },
}

/// An explicit message result, in request order.
#[derive(Debug, Serialize)]
pub struct ContextMessage {
    /// Requested ID, not an independently disclosed event ID.
    pub message_id: String,
    /// Actor-private state within the requested context.
    #[serde(flatten)]
    pub state: MessageReadState,
}

/// A context result. Denied and missing resources share an indistinguishable shape.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ContextState {
    /// Missing or inaccessible context.
    Unavailable,
    /// Canonical context could not be proved.
    Unknown,
    /// Context authority at the response snapshot.
    Available {
        /// Bounded explicit selectors, in request order.
        messages: Vec<ContextMessage>,
    },
}

/// Actor-private bounded context response; no cross-request snapshot guarantee.
#[derive(Debug, Serialize)]
pub struct ContextPage {
    /// Effective read-state lifecycle for this response.
    pub account: ReadAccount,
    /// One result per requested context, in request order.
    pub contexts: Vec<ContextState>,
}
