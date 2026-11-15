use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Characters of message text stored and returned as a preview.
pub const PREVIEW_CHARS: usize = 256;
/// Rows in a page when the caller names no limit.
pub const DEFAULT_LIMIT: usize = 100;
/// Most rows in one page.
pub const MAX_LIMIT: usize = 500;
/// Most changed rows a `since` read returns per kind group. Past it the
/// read answers `reset` and the caller starts over from a snapshot.
pub const MAX_SINCE_ROWS: usize = 1000;
/// Largest unread mention total reported.
pub const MAX_MENTIONS: u32 = 99;
/// How far back a `since` marker may reach, and how long a thread in the
/// default state stays live after the person last engaged with it.
pub const HORIZON_SECONDS: i64 = 30 * 24 * 60 * 60;
/// Newest events per channel, or replies per thread, the fill examines.
pub const FILL_SCAN: i64 = 512;
/// The person's newest mentions examined for the unread mention total, and
/// the mentions one page of the mention feed scans.
pub const MENTION_SCAN: i64 = 2000;
/// A row is stamped just before its transaction commits; a `since` read is
/// widened by this much so a slow commit is not missed.
pub const COMMIT_SKEW_SECONDS: i64 = 30;

/// What a row is. `Channel`, `Dm` and `Solo` are conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// A stream or forum channel.
    Channel,
    /// A direct message conversation.
    Dm,
    /// A Solo chat: a private channel labelled `buzz.solo`.
    Solo,
    /// A thread the person is part of.
    Thread,
    /// One message that tags the person.
    Mention,
}

impl RowKind {
    /// Every kind, in wire order.
    pub const ALL: [RowKind; 5] = [
        RowKind::Channel,
        RowKind::Dm,
        RowKind::Solo,
        RowKind::Thread,
        RowKind::Mention,
    ];

    /// The kinds of the list: conversations and threads. Mentions are a feed
    /// of their own.
    pub const LIST: [RowKind; 4] = [
        RowKind::Channel,
        RowKind::Dm,
        RowKind::Solo,
        RowKind::Thread,
    ];

    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            RowKind::Channel => "channel",
            RowKind::Dm => "dm",
            RowKind::Solo => "solo",
            RowKind::Thread => "thread",
            RowKind::Mention => "mention",
        }
    }

    /// Parse a wire name.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Whether the row is a conversation: its id is a channel UUID.
    pub fn is_conversation(self) -> bool {
        matches!(self, RowKind::Channel | RowKind::Dm | RowKind::Solo)
    }

    /// Position of the kind's group in the sort order, after activity time.
    pub(super) fn rank(self) -> u8 {
        match self {
            RowKind::Channel | RowKind::Dm | RowKind::Solo => 0,
            RowKind::Thread => 1,
            RowKind::Mention => 2,
        }
    }
}

/// The message a row shows. Marking the row read marks through `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    /// Event id, lowercase hex.
    pub id: String,
    /// Author, lowercase hex.
    pub pubkey: String,
    /// The author's signed `created_at`, Unix seconds. Display only.
    pub created_at: i64,
    /// The first [`PREVIEW_CHARS`] characters, after the latest edit.
    pub text: String,
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    /// What the row is.
    pub kind: RowKind,
    /// Channel UUID, thread root id or mention event id.
    pub id: String,
    /// The channel the row is in.
    pub channel_id: Uuid,
    /// Thread root, for a thread row and for a mention inside a thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    /// The sort key, Unix seconds: relay arrival of the message in `preview`.
    /// A conversation with no such message carries its creation time.
    pub at: i64,
    /// The message to show: a conversation's newest top-level message, a
    /// thread's newest reply, or the mention itself.
    pub preview: Option<Preview>,
    /// Whether the row needs the person's attention.
    pub unread: bool,
    /// `default` or `muted`.
    pub state: &'static str,
    /// Arrival in microseconds: the exact sort key, for cursors.
    #[serde(skip)]
    pub(super) sort_at: i64,
}

impl Row {
    /// Where the page after this row starts.
    pub fn cursor(&self) -> Cursor {
        Cursor {
            micros: self.sort_at,
            rank: self.kind.rank(),
            id: self.id.clone(),
        }
    }
}

/// A row that left the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removed {
    /// What the row was.
    pub kind: RowKind,
    /// Its id.
    pub id: String,
}

/// Unread rows of each kind, over the whole set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Channels.
    pub channel: u32,
    /// Direct messages.
    pub dm: u32,
    /// Solo chats.
    pub solo: u32,
    /// Threads.
    pub thread: u32,
    /// Unread mentions, at most [`MAX_MENTIONS`].
    pub mention: u32,
}

/// Where a `since` read resumes: the person's change counter and the
/// database time of the read that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    /// `personal_read_accounts.seq` at the read.
    pub seq: i64,
    /// Database time of the read, Unix microseconds.
    pub micros: i64,
}

impl Marker {
    /// The opaque wire form.
    pub fn encode(self) -> String {
        format!("{}.{}", self.seq, self.micros)
    }

    /// Parse the wire form.
    pub fn parse(value: &str) -> Option<Self> {
        let (seq, micros) = value.split_once('.')?;
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !digits(seq) || !digits(micros) {
            return None;
        }
        Some(Self {
            seq: seq.parse().ok()?,
            micros: micros.parse().ok()?,
        })
    }
}

/// Where the next older page starts: after this row in sort order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// The last row's arrival, Unix microseconds.
    pub micros: i64,
    /// The last row's kind group.
    pub rank: u8,
    /// The last row's id.
    pub id: String,
}

/// Which rows a read wants.
#[derive(Debug, Clone)]
pub enum Select {
    /// The newest rows, or those after `before`.
    Page {
        /// Resume after this row.
        before: Option<Cursor>,
        /// Rows wanted.
        limit: usize,
    },
    /// Rows changed since a marker.
    Since(Marker),
    /// The person's mentions, newest first: a feed of its own, paged on
    /// demand and never part of a `since` read.
    Mentions {
        /// Resume after this row.
        before: Option<Cursor>,
        /// Rows wanted.
        limit: usize,
        /// Only mentions not yet read.
        unread: bool,
    },
    /// Named conversation and thread rows: what a write hands back.
    Ids {
        /// Conversations.
        channels: Vec<Uuid>,
        /// Thread roots.
        roots: Vec<[u8; 32]>,
    },
}

/// One read of the list.
#[derive(Debug, Clone)]
pub struct Query {
    /// Conversation and thread kinds wanted. Ignored by [`Select::Mentions`].
    pub kinds: Vec<RowKind>,
    /// Which rows.
    pub select: Select,
}

/// The result of one read.
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// Rows, newest activity first.
    pub rows: Vec<Row>,
    /// Rows that left the list; `since` reads only.
    pub removed: Vec<Removed>,
    /// Totals over the whole set: with a first page, and with a `since`
    /// read that found a change.
    pub totals: Option<Totals>,
    /// Where the next older page starts.
    pub next: Option<Cursor>,
    /// Pass as the next read's `since`.
    pub marker: Option<Marker>,
    /// The marker was too old, or too much changed: start over.
    pub reset: bool,
}

/// Database facts every query of one read shares.
#[derive(Debug, Clone, Copy)]
pub(super) struct Snapshot {
    /// The account's start: the floor of every read position. `None` until
    /// the account starts, and nothing is unread.
    pub(super) started_at: Option<DateTime<Utc>>,
    /// The account's change counter.
    pub(super) seq: i64,
    /// Database time of the read.
    pub(super) now: DateTime<Utc>,
}
