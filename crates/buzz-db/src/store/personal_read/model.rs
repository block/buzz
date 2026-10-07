use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum independent operations in one HTTP request.
pub const MAX_INTENTS: usize = 100;

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
    /// Advance a context through one fixed message: everything displayed at
    /// or before it in that context, and anything with an equal arrival.
    MarkThrough {
        /// Channel or canonical thread being marked.
        target: ReadTarget,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
    /// Advance the channel timeline and every thread in it through one fixed
    /// message: everything in the channel displayed at or before it. The
    /// anchor may be a reply; ancestry is irrelevant.
    MarkChannelRead {
        /// Channel being marked, including all of its threads.
        channel_id: Uuid,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
    /// Follow a thread. A thread you did not follow starts caught up.
    Follow {
        /// The thread; a target without a root is invalid.
        target: ReadTarget,
    },
    /// Stop following a thread. Sticky: later replies and mentions do not
    /// re-follow; your own reply or `follow` does.
    Unfollow {
        /// The thread; a target without a root is invalid.
        target: ReadTarget,
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

/// Maximum channel summaries in one sidebar page.
pub const MAX_CHANNELS: usize = 20;
/// Ingest rejects author times further than this from relay time, so anything
/// that arrived after a position has an author time no earlier than this
/// before it. Forward scans range over author time and filter on arrival.
pub const MAX_ARRIVAL_SKEW_SECONDS: u32 = 900;
/// Conversation kinds eligible for ordinary unread state (not edits/reactions).
pub const ELIGIBLE_KINDS: [i32; 4] = [9, 40002, 45001, 45003];

/// One joined-channel summary, not a second conversation/history API.
/// Clients receive message IDs only; positions stay relay-internal.
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
    /// The channel timeline has a message past the actor's position. Thread
    /// replies never set it; they show on their thread row.
    pub unread: bool,
    /// Unread timeline messages that mention the actor. Exact, uncapped.
    pub mentions: i64,
    /// The last timeline message, in display order, at or before the actor's
    /// position: the "new" divider goes below it. None when there is none.
    pub read_through_id: Option<String>,
    /// Followed threads with an unread reply, newest unread reply first.
    pub threads: Vec<ThreadReadSummary>,
}

/// One followed thread with an unread reply. No conversation bytes.
#[derive(Debug, Deserialize, Serialize)]
pub struct ThreadReadSummary {
    /// Canonical thread-root event ID.
    pub root_id: String,
    /// A reply past the actor's thread position. Listed rows are unread.
    pub unread: bool,
    /// Unread replies that mention the actor. Exact, uncapped.
    pub mentions: i64,
    /// The last reply, in display order, at or before the actor's thread
    /// position. None when the actor has read no reply.
    pub read_through_id: Option<String>,
    /// Last unread reply to arrive: marking through it reads the thread, and
    /// clients fetch it by ID for a preview.
    pub latest_id: String,
}

/// A bounded roster page, with no cross-page snapshot or removal inference.
#[derive(Debug, Serialize)]
pub struct SidebarPage {
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
    /// It is a reply in one of the actor's threads.
    Conversation,
    /// It carries `broadcast=1`.
    Broadcast,
}

/// Read progress and eligibility for one concrete message, not a public receipt.
/// A message has one state whichever of its contexts asks.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MessageReadState {
    /// Missing, inaccessible, or outside the requested context. No existence oracle.
    Unavailable,
    /// Not unread: own, deleted, auxiliary, a reply outside the actor's
    /// threads, or anything before the actor's first read intent.
    NotCounted,
    /// At or before the position it counts against.
    Read,
    /// Counts, and arrived after the position it counts against.
    Unread {
        /// Null only for an ordinary timeline message.
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
    /// Context authority at the response snapshot.
    Available {
        /// Bounded explicit selectors, in request order.
        messages: Vec<ContextMessage>,
    },
}

/// Actor-private bounded context response; no cross-request snapshot guarantee.
#[derive(Debug, Serialize)]
pub struct ContextPage {
    /// One result per requested context, in request order.
    pub contexts: Vec<ContextState>,
}
