//! One stored row per channel for sidebar lists: last activity and the newest
//! top-level message, so a list read is a join and never a per-channel scan.
//!
//! The event log is the truth and a row is a cache of it that proves its own
//! freshness: `seen_at` is the newest channel event the row accounts for, and
//! a read asks the log, with one index probe per channel, whether anything
//! newer exists. A row that is behind is rebuilt from the log before it is
//! returned. So no writer has to be reliable for a read to be right:
//!
//! - **Ingest.** [`stage`] and [`seen`] record a stored event on its
//!   [`AdmittedTx`]. Once that commits, [`submit`] hands the result to a
//!   background writer that coalesces per channel and writes a moment later.
//!   No message transaction holds or waits for a channel's row. A lost write
//!   only means the next read rebuilds that row.
//! - **Edit.** An edit of the current head replaces its stored text.
//! - **Fill.** A read that meets a row that is behind, or whose head is
//!   deleted, takes the row and merges the channel's stored history into it.
//!
//! Ingest and fill merge with the same "newer wins" statement, so their
//! order never matters.
//!
//! Rows are display state. They never grant access: readers join current
//! membership, and callers recheck it before release.

use std::collections::BTreeMap;

use buzz_core::{
    kind::{
        KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_DIFF,
        KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_V2,
    },
    CommunityId,
};
use chrono::{DateTime, Utc};
use nostr::Event;
use serde::Serialize;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::{observability, store::event::ThreadMetadataParams, AdmittedTx, Db, DbError, Result};

/// Kinds that count as channel activity, at any thread depth.
pub const ACTIVITY_KINDS: [u32; 5] = [
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_POST,
    KIND_FORUM_COMMENT,
];
/// Kinds that can be a channel's head when top-level.
pub const HEAD_KINDS: [u32; 4] = [
    KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
    KIND_STREAM_MESSAGE_DIFF,
    KIND_FORUM_POST,
];
/// Characters of head text stored and returned.
pub const HEAD_TEXT_CHARS: usize = 256;
/// Most channels one read returns.
pub const MAX_CHANNELS: usize = 1000;
/// Most channels one read may name.
pub const MAX_CHANNEL_IDS: usize = 100;
/// Newest events per channel the fill examines. A channel whose newest
/// top-level message is older than this many events fills with no head.
pub const FILL_SCAN: i64 = 512;
/// Channels filled per statement.
const FILL_BATCH: usize = 100;
/// A row is stamped just before its transaction commits; a reader's `since`
/// is widened by this much so a slow commit is not missed.
const COMMIT_SKEW_SECONDS: i64 = 30;

/// How long committed contributions coalesce before they are written.
const WRITE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
/// How long a background write waits for a row a reader is rebuilding.
const WRITE_LOCK_TIMEOUT: &str = "250ms";
/// Times a failed row write is tried before it is left to the next read.
const WRITE_ATTEMPTS: u8 = 3;
/// Reads that may rebuild rows at once in this process. Rebuilding uses
/// writer connections, which message writes need more.
const REPAIRS_AT_ONCE: usize = 4;
/// How long a read waits for its turn to rebuild before serving stored rows.
const REPAIR_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// Bound on one community's background write.
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Most channels held in memory between writes. Past it, contributions are
/// dropped and counted, and reads rebuild those rows.
const PENDING_CHANNELS: usize = 8192;
/// Most edits staged for one channel between writes.
const MAX_STAGED_EDITS: usize = 32;

/// What one transaction contributes to one channel's row.
#[derive(Debug, Default)]
pub(crate) struct Staged {
    /// Failed writes of this entry so far.
    attempts: u8,
    /// Newest channel event of any kind.
    seen: Option<DateTime<Utc>>,
    activity: Option<(DateTime<Utc>, [u8; 32])>,
    head: Option<StagedHead>,
    /// Edits of a possible head.
    edits: Vec<StagedEdit>,
}

#[derive(Debug)]
struct StagedEdit {
    target: [u8; 32],
    id: [u8; 32],
    at: DateTime<Utc>,
    text: String,
}

#[derive(Debug)]
struct StagedHead {
    id: [u8; 32],
    at: DateTime<Utc>,
    pubkey: [u8; 32],
    text: String,
}

fn head_text(content: &str) -> String {
    content.chars().take(HEAD_TEXT_CHARS).collect()
}

/// The app convention for a message only agents need: not shown in previews.
fn agents_only(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("audience")
            && parts.get(1).map(String::as_str) == Some("agents")
    })
}

/// The first valid 64-hex `e` tag in signed order, as the edit lookup uses.
fn edit_target(event: &Event) -> Option<[u8; 32]> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e") {
            return None;
        }
        let bytes = hex::decode(parts.get(1)?).ok()?;
        <[u8; 32]>::try_from(bytes).ok()
    })
}

/// Record that a just-inserted event of any kind belongs to `channel`, so the
/// row accounts for it. Every channel event insert calls this through
/// `after_admitted_insert`.
pub(crate) fn seen(tx: &mut AdmittedTx, created_at: DateTime<Utc>, channel_id: Option<Uuid>) {
    let Some(channel) = channel_id else { return };
    let staged = tx.channel_head(channel);
    if staged.seen.is_none_or(|at| created_at > at) {
        staged.seen = Some(created_at);
    }
}

/// Record a just-inserted event for its channel's row. Call only when the
/// INSERT affected a row. `thread` is the event's thread metadata: top-level
/// follows the channel window's rule (no metadata, depth 0, or a depth-1
/// reply broadcast to the channel).
pub(crate) fn stage(
    tx: &mut AdmittedTx,
    event: &Event,
    created_at: DateTime<Utc>,
    channel_id: Option<Uuid>,
    thread: Option<&ThreadMetadataParams<'_>>,
) {
    let Some(channel) = channel_id else { return };
    let kind = u32::from(event.kind.as_u16());
    if kind == KIND_STREAM_MESSAGE_EDIT {
        if let Some(target) = edit_target(event) {
            let edits = &mut tx.channel_head(channel).edits;
            if edits.len() < MAX_STAGED_EDITS {
                edits.push(StagedEdit {
                    target,
                    id: *event.id.as_bytes(),
                    at: created_at,
                    text: head_text(&event.content),
                });
            }
        }
        return;
    }
    if !ACTIVITY_KINDS.contains(&kind) {
        return;
    }
    let pubkey = event.pubkey.to_bytes();
    let id = *event.id.as_bytes();
    let top_level =
        thread.is_none_or(|meta| meta.depth == 0 || (meta.depth == 1 && meta.broadcast));
    let staged = tx.channel_head(channel);
    if staged.activity.is_none_or(|(at, _)| created_at >= at) {
        staged.activity = Some((created_at, pubkey));
    }
    if top_level && HEAD_KINDS.contains(&kind) && !agents_only(event) {
        let newer = staged
            .head
            .as_ref()
            .is_none_or(|head| created_at > head.at || (created_at == head.at && id < head.id));
        if newer {
            staged.head = Some(StagedHead {
                id,
                at: created_at,
                pubkey,
                text: head_text(&event.content),
            });
        }
    }
}

/// "Newer wins" for every part of a row, shared by ingest and fill. Within
/// one second the lower id is the head, as in the channel window, and the
/// higher id is the edit, as in the timeline. `updated_at` moves only when a
/// returned field changes, so `since` is not woken by reactions.
///
/// `SEEN_NEW` is how far the writer has read the channel's events and
/// `SEEN_FIRST` what it may claim for a row that has no `seen_at` yet. Only
/// the fill has read the history, so only the fill sets the first value:
/// a row made by ingest stays unverified until a read fills it.
const MERGE_TEMPLATE: &str = r#"
    ON CONFLICT (community_id, channel_id) DO UPDATE SET
        seen_at = CASE WHEN SEEN_NEW > h.seen_at THEN SEEN_NEW
            WHEN h.seen_at IS NULL THEN SEEN_FIRST ELSE h.seen_at END,
        activity_at = CASE WHEN EXCLUDED.activity_at >= COALESCE(h.activity_at, '-infinity')
            THEN EXCLUDED.activity_at ELSE h.activity_at END,
        activity_pubkey = CASE WHEN EXCLUDED.activity_at >= COALESCE(h.activity_at, '-infinity')
            THEN EXCLUDED.activity_pubkey ELSE h.activity_pubkey END,
        head_id = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            THEN EXCLUDED.head_id ELSE h.head_id END,
        head_at = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            THEN EXCLUDED.head_at ELSE h.head_at END,
        head_pubkey = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            THEN EXCLUDED.head_pubkey ELSE h.head_pubkey END,
        head_text = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            OR (EXCLUDED.head_id = h.head_id AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id)
                > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\x')))
            THEN EXCLUDED.head_text ELSE h.head_text END,
        head_edit_at = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            OR (EXCLUDED.head_id = h.head_id AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id)
                > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\x')))
            THEN EXCLUDED.head_edit_at ELSE h.head_edit_at END,
        head_edit_id = CASE WHEN (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            OR (EXCLUDED.head_id = h.head_id AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id)
                > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\x')))
            THEN EXCLUDED.head_edit_id ELSE h.head_edit_id END,
        updated_at = CASE WHEN EXCLUDED.activity_at >= COALESCE(h.activity_at, '-infinity')
            OR (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
            OR (EXCLUDED.head_id = h.head_id AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id)
                > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\x')))
            THEN clock_timestamp() ELSE h.updated_at END
    WHERE SEEN_NEW > h.seen_at OR (h.seen_at IS NULL AND SEEN_FIRST IS NOT NULL)
        OR EXCLUDED.activity_at >= COALESCE(h.activity_at, '-infinity')
        OR (EXCLUDED.head_at, h.head_id) > (COALESCE(h.head_at, '-infinity'), EXCLUDED.head_id)
            OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL)
        OR (EXCLUDED.head_id = h.head_id AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id)
                > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\x')))
"#;

/// A signed time may be ahead of the clock. A row never claims to account
/// for the future, or one fast clock would hide every message stored until
/// then; the row instead reads as behind until that event is due.
fn not_future(seen: &str) -> String {
    format!("(CASE WHEN {seen} > clock_timestamp() THEN clock_timestamp() ELSE {seen} END)")
}

fn merge_sql(seen_new: &str, seen_first: &str) -> String {
    MERGE_TEMPLATE
        .replace("SEEN_NEW", seen_new)
        .replace("SEEN_FIRST", seen_first)
}

impl Staged {
    /// Fold a later transaction's contribution into this one, newer wins.
    fn merge(&mut self, other: Staged) {
        if let Some((at, pubkey)) = other.activity {
            if self.activity.is_none_or(|(mine, _)| at >= mine) {
                self.activity = Some((at, pubkey));
            }
        }
        if let Some(head) = other.head {
            let newer = self
                .head
                .as_ref()
                .is_none_or(|mine| head.at > mine.at || (head.at == mine.at && head.id < mine.id));
            if newer {
                self.head = Some(head);
            }
        }
        if let Some(at) = other.seen {
            if self.seen.is_none_or(|mine| at > mine) {
                self.seen = Some(at);
            }
        }
        self.attempts = self.attempts.max(other.attempts);
        // Only the latest edit of a message matters, so a stream of edits to
        // one message stays one entry.
        for edit in other.edits {
            let same = self
                .edits
                .iter()
                .position(|mine| mine.target == edit.target);
            match same {
                Some(index) => {
                    let mine = &mut self.edits[index];
                    if (edit.at, &edit.id) > (mine.at, &mine.id) {
                        *mine = edit;
                    }
                }
                None if self.edits.len() < MAX_STAGED_EDITS => self.edits.push(edit),
                None => metrics::counter!("buzz_db_channel_head_failures_total").increment(1),
            }
        }
    }
}

/// Committed contributions not yet written, per community.
#[derive(Default)]
struct Pending {
    communities: BTreeMap<Uuid, (sqlx::PgPool, BTreeMap<Uuid, Staged>)>,
    channels: usize,
    /// Count of hand-offs accepted so far.
    submitted: u64,
    /// Whether a drain task is running.
    draining: bool,
}

struct Writer {
    pending: std::sync::Mutex<Pending>,
    /// The `submitted` count that has been written or given up on.
    applied: tokio::sync::watch::Sender<u64>,
}

fn writer() -> &'static Writer {
    static WRITER: std::sync::OnceLock<Writer> = std::sync::OnceLock::new();
    WRITER.get_or_init(|| Writer {
        pending: std::sync::Mutex::new(Pending::default()),
        applied: tokio::sync::watch::channel(0).0,
    })
}

fn lock_pending(writer: &Writer) -> std::sync::MutexGuard<'_, Pending> {
    // The map is consistent at every unlock, so a poisoned lock is usable.
    writer
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Clears `draining` even when the runtime drops the drain task mid-write.
struct Draining;

impl Drop for Draining {
    fn drop(&mut self) {
        lock_pending(writer()).draining = false;
    }
}

/// Hand a committed transaction's contributions to the background writer.
///
/// Never waits and never fails the caller: the message is already committed.
/// Contributions to one channel coalesce in memory, and a short-lived task
/// writes them a moment later in its own admitted transactions. So a busy
/// channel costs one row write per interval per relay process, and no message
/// transaction ever holds or waits for a channel's row lock. Anything lost
/// here (a failure, the bound, a process exit) is rebuilt by the next read.
pub(crate) fn submit(pool: &sqlx::PgPool, community: CommunityId, staged: BTreeMap<Uuid, Staged>) {
    let writer = writer();
    let mut pending = lock_pending(writer);
    pending.submitted += 1;
    enqueue(&mut pending, pool, *community.as_uuid(), staged);
    if !pending.draining {
        // `commit` is async, so a runtime is always present here. The guard
        // is made now, so a task dropped before its first poll still clears
        // the flag.
        pending.draining = true;
        tokio::spawn(drain(Draining));
    }
}

fn enqueue(
    pending: &mut Pending,
    pool: &sqlx::PgPool,
    community: Uuid,
    staged: BTreeMap<Uuid, Staged>,
) {
    let mut dropped = 0u64;
    for (channel, staged) in staged {
        let total = pending.channels;
        let (_, channels) = pending
            .communities
            .entry(community)
            .or_insert_with(|| (pool.clone(), BTreeMap::new()));
        match channels.entry(channel) {
            std::collections::btree_map::Entry::Occupied(mut row) => row.get_mut().merge(staged),
            std::collections::btree_map::Entry::Vacant(row) if total < PENDING_CHANNELS => {
                row.insert(staged);
                pending.channels += 1;
            }
            std::collections::btree_map::Entry::Vacant(_) => dropped += 1,
        }
    }
    if dropped > 0 {
        metrics::counter!("buzz_db_channel_head_failures_total").increment(dropped);
    }
}

async fn drain(draining: Draining) {
    let writer = writer();
    loop {
        tokio::time::sleep(WRITE_INTERVAL).await;
        let (communities, submitted) = {
            let mut pending = lock_pending(writer);
            if pending.communities.is_empty() {
                // Checked under the lock `submit` takes: nothing is stranded.
                pending.draining = false;
                std::mem::forget(draining);
                return;
            }
            pending.channels = 0;
            (std::mem::take(&mut pending.communities), pending.submitted)
        };
        for (community_id, (pool, mut channels)) in communities {
            let community = CommunityId::from_uuid(community_id);
            let write = tokio::time::timeout(WRITE_TIMEOUT, write(&pool, community, &channels));
            let failed: Vec<Uuid> = match write.await {
                Ok(Ok(failed)) => failed,
                Ok(Err(error)) => {
                    tracing::warn!(%error, %community, "channel head rows not written");
                    channels.keys().copied().collect()
                }
                Err(_) => {
                    tracing::warn!(%community, "channel head rows not written: timed out");
                    channels.keys().copied().collect()
                }
            };
            if failed.is_empty() {
                continue;
            }
            // Try again next interval, merged with whatever arrived since,
            // so a later write cannot move `seen_at` past a missed message.
            // After the last attempt the row is left to the next read.
            let mut again = BTreeMap::new();
            for channel in failed {
                let Some(mut staged) = channels.remove(&channel) else {
                    continue;
                };
                staged.attempts += 1;
                if staged.attempts < WRITE_ATTEMPTS {
                    again.insert(channel, staged);
                } else {
                    metrics::counter!("buzz_db_channel_head_failures_total").increment(1);
                }
            }
            enqueue(&mut lock_pending(writer), &pool, community_id, again);
        }
        writer.applied.send_replace(submitted);
    }
}

/// Wait until every hand-off this process has accepted is written or given
/// up on, so a test observes the writer rather than a read's repair.
#[cfg(test)]
pub(crate) async fn flush() {
    let writer = writer();
    let target = lock_pending(writer).submitted;
    for _ in 0..2000 {
        let idle = lock_pending(writer).communities.is_empty();
        if idle && *writer.applied.borrow() >= target {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Returns the channels whose rows were not written.
async fn write(
    pool: &sqlx::PgPool,
    community: CommunityId,
    channels: &BTreeMap<Uuid, Staged>,
) -> Result<Vec<Uuid>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::EventWrite,
    )
    .await?;
    // A reader rebuilding a row holds it; do not queue the batch behind it.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET LOCAL lock_timeout = '{WRITE_LOCK_TIMEOUT}'"
    )))
    .execute(tx.conn())
    .await?;
    let failed = apply_staged(tx.conn(), community, channels).await?;
    tx.commit().await?;
    Ok(failed)
}

/// Write every staged channel in sorted order, so two writers touching the
/// same channels cannot deadlock. Each runs in its own savepoint: a failure
/// is rolled back and returned, and the others still land.
async fn apply_staged(
    conn: &mut PgConnection,
    community: CommunityId,
    staged: &BTreeMap<Uuid, Staged>,
) -> Result<Vec<Uuid>> {
    let mut failed = Vec::new();
    for (&channel, staged) in staged {
        sqlx::query("SAVEPOINT channel_head")
            .execute(&mut *conn)
            .await?;
        match apply_one(conn, community, channel, staged).await {
            Ok(()) => {
                sqlx::query("RELEASE SAVEPOINT channel_head")
                    .execute(&mut *conn)
                    .await?;
            }
            Err(error) => {
                if error.is_statement_cancelled() {
                    return Err(error);
                }
                sqlx::query("ROLLBACK TO SAVEPOINT channel_head")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("RELEASE SAVEPOINT channel_head")
                    .execute(&mut *conn)
                    .await?;
                tracing::warn!(%error, channel_id = %channel, "channel head update failed");
                failed.push(channel);
            }
        }
    }
    Ok(failed)
}

async fn apply_one(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    staged: &Staged,
) -> Result<()> {
    if staged.seen.is_some() || staged.activity.is_some() || staged.head.is_some() {
        let sql = format!(
            "INSERT INTO channel_heads AS h (community_id, channel_id, activity_at, \
                activity_pubkey, head_id, head_at, head_pubkey, head_text, seen_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, clock_timestamp()) {merge}",
            merge = merge_sql(&not_future("$9::timestamptz"), "NULL::timestamptz"),
        );
        let head = staged.head.as_ref();
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(channel)
            .bind(staged.activity.map(|(at, _)| at))
            .bind(staged.activity.map(|(_, pubkey)| pubkey.to_vec()))
            .bind(head.map(|head| head.id.to_vec()))
            .bind(head.map(|head| head.at))
            .bind(head.map(|head| head.pubkey.to_vec()))
            .bind(head.map(|head| head.text.as_str()))
            .bind(staged.seen)
            .execute(&mut *conn)
            .await?;
    }
    for edit in &staged.edits {
        // An upsert, not an UPDATE: it waits for a fill that holds the row,
        // so an edit stored while a fill runs is never lost. Within one
        // second the higher edit id wins, as in the timeline.
        sqlx::query(
            "INSERT INTO channel_heads AS h (community_id, channel_id, updated_at) \
             VALUES ($1, $2, clock_timestamp()) \
             ON CONFLICT (community_id, channel_id) DO UPDATE SET \
                head_text = $4, head_edit_at = $5, head_edit_id = $6, \
                updated_at = clock_timestamp() \
             WHERE h.head_id = $3 AND ($5, $6::bytea) > \
                (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\\x'))",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(edit.target.as_slice())
        .bind(&edit.text)
        .bind(edit.at)
        .bind(edit.id.as_slice())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The newest top-level message of a channel, for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Head {
    /// Event id, lowercase hex.
    pub id: String,
    /// Author, lowercase hex.
    pub pubkey: String,
    /// The author's signed `created_at`, Unix seconds.
    pub created_at: i64,
    /// The first [`HEAD_TEXT_CHARS`] characters, with its latest edit applied.
    pub text: String,
    /// Whether `text` comes from an edit.
    pub edited: bool,
}

/// One channel's row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelHead {
    /// The channel.
    pub channel_id: Uuid,
    /// Newest message at any depth, Unix seconds; `None` when there is none.
    pub activity_at: Option<i64>,
    /// Who posted it, lowercase hex.
    pub activity_pubkey: Option<String>,
    /// `None` when the channel has no top-level message to show.
    pub head: Option<Head>,
}

/// One read of a member's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelHeads {
    /// Pass as the next read's `since`, Unix seconds.
    pub as_of: i64,
    /// Rows, most recent activity first.
    pub channels: Vec<ChannelHead>,
    /// More than [`MAX_CHANNELS`] channels matched; the rest are older.
    pub truncated: bool,
}

struct Read {
    rows: Vec<ChannelHead>,
    /// Joined channels whose row is behind the event log.
    behind: Vec<Uuid>,
    /// `(channel, head id)` of rows whose head is deleted.
    stale: Vec<(Uuid, Vec<u8>)>,
    truncated: bool,
}

impl Db {
    /// The actor's joined channels' rows, most recent activity first.
    ///
    /// `since` returns only rows changed at or after it; `only` restricts to
    /// those channels. A requested channel absent from the result was not a
    /// joined, nondeleted channel. Rows that are behind the event log, or whose
    /// head is deleted, are repaired first, within the caller's time budget.
    /// Callers must recheck admission and membership before releasing rows.
    pub async fn channel_heads(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        since: Option<i64>,
        only: Option<&[Uuid]>,
    ) -> Result<ChannelHeads> {
        if only.is_some_and(|ids| ids.is_empty() || ids.len() > MAX_CHANNEL_IDS) {
            return Err(DbError::InvalidData("invalid channel list".into()));
        }
        let since = match since {
            Some(seconds) => Some(
                DateTime::from_timestamp(seconds.saturating_sub(COMMIT_SKEW_SECONDS), 0)
                    .ok_or(DbError::InvalidTimestamp(seconds))?,
            ),
            None => None,
        };
        let actor = actor.to_bytes();
        let as_of = Utc::now().timestamp();
        let mut read = self
            .read_channel_heads(community, &actor, since, only)
            .await?;
        if !read.behind.is_empty() || !read.stale.is_empty() {
            // Best effort, and bounded: rebuilding takes writer connections
            // that message writes need more. When it cannot run or fails,
            // serve what is stored (deleted heads are never returned); the
            // rows stay behind, so the next read tries again.
            static REPAIRS: tokio::sync::Semaphore =
                tokio::sync::Semaphore::const_new(REPAIRS_AT_ONCE);
            let repaired = match tokio::time::timeout(REPAIR_WAIT, REPAIRS.acquire()).await {
                Ok(Ok(_turn)) => self
                    .repair_channel_heads(community, &read.behind, &read.stale)
                    .await
                    .map_err(|error| error.to_string()),
                _ => Err("no turn".to_owned()),
            };
            if let Err(error) = repaired {
                metrics::counter!("buzz_db_channel_head_repairs_skipped_total").increment(1);
                tracing::debug!(%error, %community, "channel head rows not rebuilt");
            }
            read = self
                .read_channel_heads(community, &actor, since, only)
                .await?;
        }
        Ok(ChannelHeads {
            as_of,
            channels: read.rows,
            truncated: read.truncated,
        })
    }

    async fn read_channel_heads(
        &self,
        community: CommunityId,
        actor: &[u8; 32],
        since: Option<DateTime<Utc>>,
        only: Option<&[Uuid]>,
    ) -> Result<Read> {
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        // The head join carries the partition key, so each row is one point
        // lookup. `behind` is one index probe per channel, pruned to the
        // partitions after `seen_at`: is any stored event newer than the row
        // accounts for? A row that is behind or names a deleted head always
        // matches `since`: neither stamps the row.
        let rows = sqlx::query(
            r#"SELECT q.* FROM (
                SELECT c.id AS channel_id,
                    h.activity_at, h.activity_pubkey, h.head_id, h.head_at, h.head_pubkey,
                    h.head_text, h.head_edit_at IS NOT NULL AS edited, h.updated_at,
                    (h.head_id IS NOT NULL AND (e.id IS NULL OR e.deleted_at IS NOT NULL)) AS gone,
                    EXISTS (
                        SELECT 1 FROM events n
                        WHERE n.community_id = c.community_id AND n.channel_id = c.id
                            AND n.created_at > COALESCE(h.seen_at, '-infinity')
                    ) AS behind
                FROM channel_members cm
                JOIN channels c ON c.community_id = cm.community_id AND c.id = cm.channel_id
                LEFT JOIN channel_heads h ON h.community_id = c.community_id AND h.channel_id = c.id
                LEFT JOIN events e ON e.community_id = h.community_id
                    AND e.created_at = h.head_at AND e.id = h.head_id
                WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL
                    AND ($3::uuid[] IS NULL OR c.id = ANY($3))
                ) q
                WHERE $4::timestamptz IS NULL OR q.updated_at >= $4 OR q.behind OR q.gone
                ORDER BY q.activity_at DESC NULLS LAST, q.channel_id
                LIMIT $5"#,
        )
        .bind(community.as_uuid())
        .bind(actor.as_slice())
        .bind(only)
        .bind(since)
        .bind(MAX_CHANNELS as i64 + 1)
        .fetch_all(&mut *conn)
        .await?;
        let truncated = rows.len() > MAX_CHANNELS;
        let mut read = Read {
            rows: Vec::with_capacity(rows.len().min(MAX_CHANNELS)),
            behind: Vec::new(),
            stale: Vec::new(),
            truncated,
        };
        for row in rows.into_iter().take(MAX_CHANNELS) {
            let channel_id: Uuid = row.try_get("channel_id")?;
            let head_id: Option<Vec<u8>> = row.try_get("head_id")?;
            let gone: bool = row.try_get("gone")?;
            if row.try_get::<bool, _>("behind")? {
                read.behind.push(channel_id);
            }
            if gone {
                if let Some(id) = head_id.clone() {
                    read.stale.push((channel_id, id));
                }
            }
            let head = match (head_id, gone) {
                (Some(id), false) => Some(Head {
                    id: hex::encode(id),
                    pubkey: hex::encode(row.try_get::<Vec<u8>, _>("head_pubkey")?),
                    created_at: row.try_get::<DateTime<Utc>, _>("head_at")?.timestamp(),
                    text: row
                        .try_get::<Option<String>, _>("head_text")?
                        .unwrap_or_default(),
                    edited: row.try_get("edited")?,
                }),
                _ => None,
            };
            read.rows.push(ChannelHead {
                channel_id,
                activity_at: row
                    .try_get::<Option<DateTime<Utc>>, _>("activity_at")?
                    .map(|at| at.timestamp()),
                activity_pubkey: row
                    .try_get::<Option<Vec<u8>>, _>("activity_pubkey")?
                    .map(hex::encode),
                head,
            });
        }
        Ok(read)
    }

    /// Clear deleted heads, then merge stored history into every row that
    /// needs it. Each batch is its own admitted transaction.
    async fn repair_channel_heads(
        &self,
        community: CommunityId,
        behind: &[Uuid],
        stale: &[(Uuid, Vec<u8>)],
    ) -> Result<()> {
        let mut channels: Vec<Uuid> = behind.to_vec();
        channels.extend(stale.iter().map(|(channel, _)| *channel));
        channels.sort_unstable();
        channels.dedup();
        for batch in channels.chunks(FILL_BATCH) {
            let mut tx = crate::begin_community_write_transaction(
                &self.pool,
                community,
                observability::WriterOperation::SubscriptionHistory,
            )
            .await?;
            sqlx::query("SET LOCAL statement_timeout = '5000ms'")
                .execute(tx.conn())
                .await?;
            sqlx::query("SET LOCAL lock_timeout = '500ms'")
                .execute(tx.conn())
                .await?;
            sqlx::query("SET LOCAL jit = off")
                .execute(tx.conn())
                .await?;
            // Take every row first, creating missing ones, in channel
            // order. An edit stored from here on waits for this transaction,
            // and one stored before is visible to the scan.
            sqlx::query(
                "INSERT INTO channel_heads AS h (community_id, channel_id, updated_at) \
                 SELECT $1, c.id, clock_timestamp() \
                 FROM unnest($2::uuid[]) AS c(id) ORDER BY c.id \
                 ON CONFLICT (community_id, channel_id) DO UPDATE \
                    SET updated_at = h.updated_at",
            )
            .bind(community.as_uuid())
            .bind(batch)
            .execute(tx.conn())
            .await?;
            let (stale_channels, stale_heads): (Vec<Uuid>, Vec<Vec<u8>>) = stale
                .iter()
                .filter(|(channel, _)| batch.contains(channel))
                .cloned()
                .unzip();
            if !stale_channels.is_empty() {
                // Only while the row still names the deleted head: a message
                // stored since then, or another reader, already replaced it.
                sqlx::query(
                    "UPDATE channel_heads h SET head_id = NULL, head_at = NULL, \
                        head_pubkey = NULL, head_text = NULL, head_edit_at = NULL, \
                        head_edit_id = NULL, updated_at = clock_timestamp() \
                     FROM unnest($2::uuid[], $3::bytea[]) AS s(channel_id, head_id) \
                     WHERE h.community_id = $1 AND h.channel_id = s.channel_id \
                       AND h.head_id = s.head_id",
                )
                .bind(community.as_uuid())
                .bind(&stale_channels)
                .bind(&stale_heads)
                .execute(tx.conn())
                .await?;
            }
            fill(tx.conn(), community, batch).await?;
            tx.commit().await?;
        }
        Ok(())
    }
}

/// Merge each channel's stored history into its row and record how far the
/// log was read. The caller holds the rows, and this is its own statement, so
/// its snapshot sees every edit and deletion committed before they were taken.
async fn fill(conn: &mut PgConnection, community: CommunityId, channels: &[Uuid]) -> Result<()> {
    let activity: Vec<i32> = ACTIVITY_KINDS.iter().map(|kind| *kind as i32).collect();
    let heads: Vec<i32> = HEAD_KINDS.iter().map(|kind| *kind as i32).collect();
    let sql = format!(
        r#"INSERT INTO channel_heads AS h (community_id, channel_id, activity_at,
                activity_pubkey, head_id, head_at, head_pubkey, head_text, head_edit_at,
                head_edit_id, seen_at, updated_at)
            SELECT $1, c.id, a.created_at, a.pubkey, t.id, t.created_at, t.pubkey,
                left(COALESCE(edit.content, t.content), $6), edit.created_at, edit.id,
                newest.created_at, clock_timestamp()
            FROM unnest($2::uuid[]) AS c(id)
            LEFT JOIN LATERAL (
                SELECT created_at FROM events
                WHERE community_id = $1 AND channel_id = c.id
                ORDER BY created_at DESC LIMIT 1
            ) newest ON true
            LEFT JOIN LATERAL (
                SELECT recent.created_at, recent.pubkey FROM (
                    SELECT created_at, pubkey, kind, deleted_at FROM events
                    WHERE community_id = $1 AND channel_id = c.id
                    ORDER BY created_at DESC, id LIMIT $5
                ) recent
                WHERE recent.kind = ANY($3) AND recent.deleted_at IS NULL
                ORDER BY recent.created_at DESC LIMIT 1
            ) a ON true
            LEFT JOIN LATERAL (
                SELECT recent.id, recent.created_at, recent.pubkey, recent.content FROM (
                    SELECT id, created_at, pubkey, kind, deleted_at, tags, content FROM events
                    WHERE community_id = $1 AND channel_id = c.id
                    ORDER BY created_at DESC, id LIMIT $5
                ) recent
                LEFT JOIN thread_metadata tm ON tm.community_id = $1
                    AND tm.event_created_at = recent.created_at AND tm.event_id = recent.id
                WHERE recent.kind = ANY($4) AND recent.deleted_at IS NULL
                    AND (tm.depth IS NULL OR tm.depth = 0 OR (tm.depth = 1 AND tm.broadcast))
                    AND NOT EXISTS (
                        SELECT 1 FROM jsonb_array_elements(recent.tags) AS tag
                        WHERE tag->>0 = 'audience' AND tag->>1 = 'agents')
                ORDER BY recent.created_at DESC, recent.id LIMIT 1
            ) t ON true
            LEFT JOIN LATERAL (
                SELECT content, created_at, id FROM events
                WHERE community_id = $1 AND channel_id = c.id AND kind = 40003
                    AND deleted_at IS NULL AND {target} = t.id
                ORDER BY created_at DESC, id DESC LIMIT 1
            ) edit ON t.id IS NOT NULL
            ORDER BY c.id
            {merge}"#,
        merge = merge_sql(
            &not_future("EXCLUDED.seen_at"),
            &not_future("EXCLUDED.seen_at")
        ),
        target = crate::store::media::MEDIA_EDIT_TARGET_SQL,
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community.as_uuid())
        .bind(channels)
        .bind(&activity)
        .bind(&heads)
        .bind(FILL_SCAN)
        .bind(HEAD_TEXT_CHARS as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
