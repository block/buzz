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
    /// Advance a context through one fixed message, including equal times.
    MarkThrough {
        /// Channel or canonical thread being marked.
        target: ReadTarget,
        /// Fixed anchor; retry must not substitute the latest message.
        message_id: String,
    },
    /// Migration/recovery only: preserve an original legacy timestamp verbatim.
    LegacyPrefix {
        /// Original channel or canonical thread scope.
        target: ReadTarget,
        /// Original nonnegative second-resolution event timestamp.
        through_timestamp: i64,
    },
    /// The client declares its frozen baseline resolved, including exceptions.
    CompleteImport,
}

/// Outcome for one independent transaction, never acknowledged before commit.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IntentOutcome {
    /// The fixed frontier/import operand committed.
    Applied,
    /// Missing and forbidden contexts deliberately share one outcome.
    Blocked,
    /// Invalid operands; no changes committed for this intent.
    Invalid,
}

/// The tracking boundary and migration status for the authenticated account.
#[derive(Clone, Debug, Serialize)]
pub struct ReadAccount {
    /// Configured tracking duration in seconds.
    pub retention_seconds: u32,
    /// Read-time relay-receipt cutoff (Unix milliseconds), not a discard boundary.
    pub cutoff_ms: i64,
    /// Client-declared baseline completion; null means provisional projections.
    pub imported_at_ms: Option<i64>,
}

/// Maximum channel summaries in one sidebar page.
pub const MAX_CHANNELS: usize = 20;
/// Bounded event evidence per channel; exhaustion is never inferred at this cap.
pub const MAX_CHANNEL_SCAN: usize = 256;
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
    /// More evidence exists or ancestry/participation could not be proved.
    AtLeast {
        /// Proven lower bound, not a fabricated badge cap.
        value: u32,
    },
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
    /// Ordinary unread lower bound or exact count.
    pub unread: ReadCount,
    /// Directed unread (DM, mention/broadcast, participating-thread reply).
    pub attention: ReadCount,
    /// Latest eligible nondeleted event ID, independent of read progress, author,
    /// and the unread-tracking horizon.
    /// None proves absence only when latest_message_complete is true.
    pub latest_message_id: Option<String>,
    /// Whether the latest lookup found a result or exhausted channel history.
    /// False means the bounded probe found none, but an unexamined tail remains.
    pub latest_message_complete: bool,
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

/// Read progress and eligibility for one concrete message, not a public receipt.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MessageReadState {
    /// Missing, inaccessible, or outside the requested context. No existence oracle.
    Unavailable,
    /// Evidence cannot safely establish ancestry or eligibility.
    Unknown,
    /// Ineligible for unread counts (own, deleted, auxiliary, or outside horizon).
    NotCounted,
    /// Covered by this context's frontier.
    Read,
    /// Eligible and beyond this context's frontier.
    Unread {
        /// True for proven directed attention; null means participation unproved.
        attention: Option<bool>,
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
        /// Fixed author-time prefix; null means no read progress.
        through_timestamp: Option<i64>,
        /// Bounded explicit selectors, in request order.
        messages: Vec<ContextMessage>,
    },
}

/// Actor-private bounded context response; no cross-request snapshot guarantee.
#[derive(Debug, Serialize)]
pub struct ContextPage {
    /// Receipt-time horizon and import status at this snapshot.
    pub account: ReadAccount,
    /// One result per requested context, in request order.
    pub contexts: Vec<ContextState>,
}
