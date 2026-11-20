//! Ingest: what a stored event contributes to the shared rows and to
//! people's own rows, staged on its transaction and written after commit.

use std::collections::{BTreeMap, BTreeSet};

use buzz_core::{kind::KIND_STREAM_MESSAGE_EDIT, CommunityId};
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgConnection, Row as _};
use uuid::Uuid;

use super::model::PREVIEW_CHARS;
use crate::{
    observability,
    store::{event::ThreadMetadataParams, personal_read::ELIGIBLE_KINDS},
    AdmittedTx, Result,
};

/// How long committed contributions coalesce before they are written.
const WRITE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
/// How long a background write waits for a row a reader is rebuilding.
const WRITE_LOCK_TIMEOUT: &str = "250ms";
/// Times a failed write is tried before shared rows are left to the next read.
const WRITE_ATTEMPTS: u8 = 8;
/// Bound on one community's background write.
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Most channels held in memory between writes. Past it, contributions are
/// dropped and counted, and reads rebuild the shared rows.
const PENDING_CHANNELS: usize = 8192;
/// Most edits, threads, people and parents staged for one channel between
/// writes.
const MAX_STAGED_EDITS: usize = 32;
const MAX_STAGED_THREADS: usize = 256;
const MAX_STAGED_PEOPLE: usize = 1024;
/// Most people one message pulls into its thread by tagging them.
const MAX_TAGGED: usize = 64;

/// Shared-row contributions dropped: a bound was hit or every attempt failed.
/// The next read that checks the row rebuilds it.
const SHARED_DROPPED: &str = "buzz_db_activity_write_failures_total";
/// Writes of people's own rows dropped: an author's read position, or a
/// person's part in a thread. Nothing rebuilds these, so this is the count of
/// threads that may be missing from someone's list until their next reply.
const PEOPLE_DROPPED: &str = "buzz_db_activity_people_writes_dropped_total";
/// Channels whose contributions were still in memory when the shutdown flush
/// ran out of time.
const UNWRITTEN_AT_SHUTDOWN: &str = "buzz_db_activity_unwritten_at_shutdown_total";

type Id = [u8; 32];
/// `(person, thread root)`; no root is the conversation.
type PersonScope = (Id, Option<Id>);

/// One stored message, as a shared row keeps it.
#[derive(Debug, Clone)]
pub(super) struct Message {
    pub(super) id: Id,
    pub(super) created_at: DateTime<Utc>,
    pub(super) received_at: DateTime<Utc>,
    pub(super) pubkey: Id,
    pub(super) text: String,
}

impl Message {
    /// Later arrival wins; of two with the same arrival, the lower id.
    fn newer_than(&self, other: &Message) -> bool {
        self.received_at > other.received_at
            || (self.received_at == other.received_at && self.id < other.id)
    }
}

fn keep_newer(slot: &mut Option<Message>, message: Message) {
    if slot.as_ref().is_none_or(|mine| message.newer_than(mine)) {
        *slot = Some(message);
    }
}

fn keep_later(slot: &mut Option<DateTime<Utc>>, at: DateTime<Utc>) {
    if slot.is_none_or(|mine| at > mine) {
        *slot = Some(at);
    }
}

#[derive(Debug)]
struct StagedEdit {
    target: Id,
    id: Id,
    at: DateTime<Utc>,
    text: String,
}

#[derive(Debug, Default)]
struct StagedThread {
    /// Author time of the newest reply of any kind.
    seen: Option<DateTime<Utc>>,
    newest: Option<Message>,
}

/// What one transaction contributes to one channel's rows.
#[derive(Debug, Default)]
pub(crate) struct Staged {
    /// Failed writes of this entry so far.
    attempts: u8,
    /// Author time of the newest channel event of any kind.
    seen: Option<DateTime<Utc>>,
    /// Newest top-level message.
    head: Option<Message>,
    /// Edits of a possible head.
    edits: Vec<StagedEdit>,
    /// Newest reply per thread root.
    threads: BTreeMap<Id, StagedThread>,
    /// People's own rows, by `(person, thread root)`; no root is the
    /// conversation. The value is the read position a post moves the author
    /// to; `None` only marks the person as part of the thread.
    people: BTreeMap<PersonScope, Option<(DateTime<Utc>, Id)>>,
    /// `(thread root, parent id, parent author time)`: the parent's author is
    /// part of the thread.
    parents: BTreeSet<(Id, Id, DateTime<Utc>)>,
}

fn preview_text(content: &str) -> String {
    content.chars().take(PREVIEW_CHARS).collect()
}

fn id32(bytes: &[u8]) -> Option<Id> {
    bytes.try_into().ok()
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
fn edit_target(event: &Event) -> Option<Id> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e") {
            return None;
        }
        id32(&hex::decode(parts.get(1)?).ok()?)
    })
}

/// People the event tags, as the mention index reads them.
fn tagged(event: &Event) -> impl Iterator<Item = Id> + '_ {
    event.tags.iter().filter_map(move |tag| {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("p")
            || buzz_core::filter::is_workflow_owner_attribution(event, tag)
        {
            return None;
        }
        id32(&hex::decode(parts.get(1)?).ok()?)
    })
}

/// Record that a just-inserted event of any kind belongs to `channel`, so the
/// channel's row accounts for it. Every channel event insert calls this
/// through `after_admitted_insert`.
pub(crate) fn seen(tx: &mut AdmittedTx, created_at: DateTime<Utc>, channel_id: Option<Uuid>) {
    let Some(channel) = channel_id else { return };
    keep_later(&mut tx.channel_activity(channel).seen, created_at);
}

/// Record a just-inserted event for its channel's rows. Call only when the
/// INSERT affected a row. `thread` is the event's thread metadata. A message
/// is top-level under the channel window's rule: no metadata, depth 0, or a
/// depth-1 reply broadcast to the channel.
pub(crate) fn stage(
    tx: &mut AdmittedTx,
    event: &Event,
    created_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    channel_id: Option<Uuid>,
    thread: Option<&ThreadMetadataParams<'_>>,
) {
    let id = *event.id.as_bytes();
    let reply = thread.filter(|meta| meta.depth >= 1);
    let root = reply
        .and_then(|meta| meta.root_event_id)
        .and_then(id32)
        .filter(|root| *root != id);
    tx.record_arrival(crate::runtime::Arrival {
        event_id: id,
        received_at,
        root,
    });
    let Some(channel) = channel_id else { return };
    let kind = u32::from(event.kind.as_u16());
    let staged = tx.channel_activity(channel);
    if let Some(root) = root {
        if staged.threads.len() < MAX_STAGED_THREADS || staged.threads.contains_key(&root) {
            keep_later(
                &mut staged.threads.entry(root).or_default().seen,
                created_at,
            );
        }
    }
    if kind == KIND_STREAM_MESSAGE_EDIT {
        if let Some(target) = edit_target(event) {
            if staged.edits.len() < MAX_STAGED_EDITS {
                staged.edits.push(StagedEdit {
                    target,
                    id,
                    at: created_at,
                    text: preview_text(&event.content),
                });
            } else {
                metrics::counter!(SHARED_DROPPED).increment(1);
            }
        }
        return;
    }
    if !ELIGIBLE_KINDS.contains(&(kind as i32)) {
        return;
    }
    let author = event.pubkey.to_bytes();
    let message = Message {
        id,
        created_at,
        received_at,
        pubkey: author,
        text: preview_text(&event.content),
    };
    let top_level =
        thread.is_none_or(|meta| meta.depth == 0 || (meta.depth == 1 && meta.broadcast));
    if top_level && !agents_only(event) {
        keep_newer(&mut staged.head, message.clone());
    }
    if let Some(thread) = root.and_then(|root| staged.threads.get_mut(&root)) {
        keep_newer(&mut thread.newest, message);
    }

    // The author has read their own message. A broadcast reply sits in both
    // the thread and the channel timeline.
    let mut scopes = vec![root];
    if root.is_some() && top_level {
        scopes.push(None);
    }
    for scope in scopes {
        staged.engage((author, scope), Some((received_at, id)));
    }
    let Some(root) = root else { return };
    for person in tagged(event).filter(|p| *p != author).take(MAX_TAGGED) {
        staged.engage((person, Some(root)), None);
    }
    if let Some((parent, at)) =
        reply.and_then(|meta| Some((id32(meta.parent_event_id?)?, meta.parent_event_created_at?)))
    {
        staged.parent((root, parent, at));
    }
}

impl Staged {
    fn engage(&mut self, key: (Id, Option<Id>), read: Option<(DateTime<Utc>, Id)>) {
        if self.people.len() >= MAX_STAGED_PEOPLE && !self.people.contains_key(&key) {
            metrics::counter!(PEOPLE_DROPPED).increment(1);
            return;
        }
        let slot = self.people.entry(key).or_default();
        if let Some(read) = read {
            if slot.is_none_or(|mine| read.0 > mine.0) {
                *slot = Some(read);
            }
        }
    }

    fn parent(&mut self, parent: (Id, Id, DateTime<Utc>)) {
        if self.parents.len() < MAX_STAGED_PEOPLE || self.parents.contains(&parent) {
            self.parents.insert(parent);
        } else {
            metrics::counter!(PEOPLE_DROPPED).increment(1);
        }
    }

    fn has_people(&self) -> bool {
        !self.people.is_empty() || !self.parents.is_empty()
    }

    /// Fold a later transaction's contribution into this one, newer wins.
    fn merge(&mut self, other: Staged) {
        if let Some(message) = other.head {
            keep_newer(&mut self.head, message);
        }
        if let Some(at) = other.seen {
            keep_later(&mut self.seen, at);
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
                None => metrics::counter!(SHARED_DROPPED).increment(1),
            }
        }
        for (root, thread) in other.threads {
            if self.threads.len() >= MAX_STAGED_THREADS && !self.threads.contains_key(&root) {
                metrics::counter!(SHARED_DROPPED).increment(1);
                continue;
            }
            let mine = self.threads.entry(root).or_default();
            if let Some(at) = thread.seen {
                keep_later(&mut mine.seen, at);
            }
            if let Some(message) = thread.newest {
                keep_newer(&mut mine.newest, message);
            }
        }
        for (key, read) in other.people {
            self.engage(key, read);
        }
        for parent in other.parents {
            self.parent(parent);
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
    /// The `submitted` count that has been written or given up on, with
    /// nothing of it left to retry.
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

/// Whether [`submit`] hands work to the background writer. On by default;
/// the relay turns it off at startup while `/buzz/v1` is off.
static WRITER_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn the background activity writer on or off for this process.
///
/// Off, committed messages leave the shared and personal rows alone and the
/// writer takes no serving-pool connections. A read rebuilds the shared rows.
/// Nothing rebuilds the personal ones (authors' read positions, who is part of
/// a thread), so turning the API on later is the same as starting from tables
/// created empty: run the backfill, which restores 30 days of them, and
/// `warm_activity` first. Call it once, before serving.
pub fn set_writer_enabled(enabled: bool) {
    WRITER_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Hand a committed transaction's contributions to the background writer.
///
/// Never waits and never fails the caller: the message is already committed.
/// Contributions to one channel coalesce in memory, and a short-lived task
/// writes them a moment later in its own admitted transactions. So a busy
/// channel costs one write per interval per relay process, and no message
/// transaction ever holds or waits for a shared row. A shared row lost here
/// (a failure, the bound, a process exit) is rebuilt by the next read. With
/// the writer off ([`set_writer_enabled`]) the contributions are dropped.
pub(crate) fn submit(pool: &sqlx::PgPool, community: CommunityId, staged: BTreeMap<Uuid, Staged>) {
    if !WRITER_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
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
    let (mut dropped, mut people_dropped) = (Vec::new(), 0u64);
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
            std::collections::btree_map::Entry::Vacant(_) => {
                dropped.push(channel);
                people_dropped += u64::from(staged.has_people());
            }
        }
    }
    if !dropped.is_empty() {
        metrics::counter!(SHARED_DROPPED).increment(dropped.len() as u64);
        // Their rows no longer follow the event log: the next read of each
        // rebuilds it.
        distrust_later(pool.clone(), CommunityId::from_uuid(community), dropped);
    }
    if people_dropped > 0 {
        metrics::counter!(PEOPLE_DROPPED).increment(people_dropped);
    }
}

/// Mark the channels' rows unverified without holding up the writer.
fn distrust_later(pool: sqlx::PgPool, community: CommunityId, channels: Vec<Uuid>) {
    tokio::spawn(async move {
        if let Err(error) = distrust(pool, community, channels).await {
            tracing::warn!(%error, %community, "activity rows not marked unverified");
        }
    });
}

async fn drain(draining: Draining) {
    let writer = writer();
    loop {
        tokio::time::sleep(WRITE_INTERVAL).await;
        let (communities, submitted) = {
            let mut pending = lock_pending(writer);
            if pending.communities.is_empty() {
                // Checked under the lock `submit` takes: nothing is stranded,
                // and nothing is in flight, so every hand-off is done.
                writer.applied.send_replace(pending.submitted);
                pending.draining = false;
                std::mem::forget(draining);
                return;
            }
            pending.channels = 0;
            (std::mem::take(&mut pending.communities), pending.submitted)
        };
        // Whether any of this round's work went back on the queue.
        let mut requeued = false;
        for (community_id, (pool, mut channels)) in communities {
            let community = CommunityId::from_uuid(community_id);
            let write = tokio::time::timeout(WRITE_TIMEOUT, write(&pool, community, &channels));
            // `(channel, whether its people's rows are unwritten)`.
            let all = |channels: &BTreeMap<Uuid, Staged>| -> Vec<(Uuid, bool)> {
                channels.keys().map(|channel| (*channel, true)).collect()
            };
            let failed: Vec<(Uuid, bool)> = match write.await {
                Ok(Ok(failed)) => failed,
                Ok(Err(error)) => {
                    tracing::warn!(%error, %community, "activity rows not written");
                    all(&channels)
                }
                Err(_) => {
                    tracing::warn!(%community, "activity rows not written: timed out");
                    all(&channels)
                }
            };
            if failed.is_empty() {
                continue;
            }
            // Try again next interval, merged with whatever arrived since,
            // so a later write cannot move `seen_at` past a missed message.
            // After the last attempt the rows are left to the next read.
            let mut again = BTreeMap::new();
            let mut given_up = Vec::new();
            for (channel, people_unwritten) in failed {
                let Some(mut staged) = channels.remove(&channel) else {
                    continue;
                };
                staged.attempts += 1;
                if staged.attempts < WRITE_ATTEMPTS {
                    again.insert(channel, staged);
                } else {
                    metrics::counter!(SHARED_DROPPED).increment(1);
                    if people_unwritten && staged.has_people() {
                        metrics::counter!(PEOPLE_DROPPED).increment(1);
                    }
                    given_up.push(channel);
                }
            }
            if !given_up.is_empty() {
                distrust_later(pool.clone(), community, given_up);
            }
            requeued |= !again.is_empty();
            enqueue(&mut lock_pending(writer), &pool, community_id, again);
        }
        // Hand-offs are done only once no part of them waits for a retry. A
        // later round that takes the retry off the queue reports them, after
        // its own write, so a flush never sees one in flight as finished.
        if !requeued {
            writer.applied.send_replace(submitted);
        }
    }
}

/// Wait, for at most `timeout`, until every hand-off this process has accepted
/// is written or given up on. Returns whether it all was.
///
/// A relay calls this once it has stopped serving, so a restart does not lose
/// the contributions of its last moments. Those still unwritten at the timeout
/// are counted and left to the next read's rebuild.
pub async fn flush_pending(timeout: std::time::Duration) -> bool {
    let writer = writer();
    let target = lock_pending(writer).submitted;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let (idle, waiting) = {
            let pending = lock_pending(writer);
            (pending.communities.is_empty(), pending.channels)
        };
        if idle && *writer.applied.borrow() >= target {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            // A write in flight has left the map; count it as one.
            metrics::counter!(UNWRITTEN_AT_SHUTDOWN).increment(waiting.max(1) as u64);
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// [`flush_pending`] for tests, so one observes the writer rather than a
/// read's repair.
#[cfg(test)]
pub(crate) async fn flush() {
    flush_pending(std::time::Duration::from_secs(10)).await;
}

/// Write one message's channel head as the writer does, on its own, for a
/// test that orders it after a later edit's upsert.
#[cfg(test)]
pub(super) async fn install_head_for_test(
    pool: &sqlx::PgPool,
    community: CommunityId,
    channel: Uuid,
    event: &Event,
    received_at: DateTime<Utc>,
) {
    let staged = Staged {
        head: Some(Message {
            id: *event.id.as_bytes(),
            created_at: DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
                .expect("timestamp"),
            received_at,
            pubkey: event.pubkey.to_bytes(),
            text: preview_text(&event.content),
        }),
        ..Staged::default()
    };
    let mut conn = pool.acquire().await.expect("acquire connection");
    apply_one(&mut conn, community, channel, &staged)
        .await
        .expect("install head");
}

/// The writer's two transactions for one channel, a batch with a head and a
/// thread reply and then the distrust that follows a failed batch, for the
/// admission test. Returns each one's result.
#[cfg(test)]
pub(super) async fn admitted_writes_for_test(
    pool: &sqlx::PgPool,
    community: CommunityId,
    channel: Uuid,
    head: &nostr::Event,
    reply: &nostr::Event,
    root: Id,
) -> (Result<Vec<(Uuid, bool)>>, Result<()>) {
    let message = |event: &nostr::Event| Message {
        id: *event.id.as_bytes(),
        created_at: DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .expect("timestamp"),
        received_at: Utc::now(),
        pubkey: event.pubkey.to_bytes(),
        text: preview_text(&event.content),
    };
    let staged = Staged {
        head: Some(message(head)),
        threads: BTreeMap::from([(
            root,
            StagedThread {
                seen: None,
                newest: Some(message(reply)),
            },
        )]),
        ..Staged::default()
    };
    let written = write(pool, community, &BTreeMap::from([(channel, staged)])).await;
    let distrusted = distrust(pool.clone(), community, vec![channel]).await;
    (written, distrusted)
}

/// Returns the channels whose rows were not written, each with whether its
/// people's rows are among them.
async fn write(
    pool: &sqlx::PgPool,
    community: CommunityId,
    channels: &BTreeMap<Uuid, Staged>,
) -> Result<Vec<(Uuid, bool)>> {
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
) -> Result<Vec<(Uuid, bool)>> {
    let mut failed = Vec::new();
    for (&channel, staged) in staged {
        // People's own rows first and on their own: nothing rebuilds them,
        // so a shared row held by a reader's rebuild must not cost them.
        let (mut ok, mut people_unwritten) = (true, false);
        for people in [true, false] {
            sqlx::query("SAVEPOINT activity_rows")
                .execute(&mut *conn)
                .await?;
            let applied = if people {
                apply_people(conn, community, channel, staged).await
            } else {
                apply_one(conn, community, channel, staged).await
            };
            if let Err(error) = &applied {
                if error.is_statement_cancelled() {
                    return applied.map(|()| failed);
                }
                sqlx::query("ROLLBACK TO SAVEPOINT activity_rows")
                    .execute(&mut *conn)
                    .await?;
                tracing::warn!(%error, channel_id = %channel, people, "activity row update failed");
                ok = false;
                people_unwritten |= people;
            }
            sqlx::query("RELEASE SAVEPOINT activity_rows")
                .execute(&mut *conn)
                .await?;
        }
        if !ok {
            failed.push((channel, people_unwritten));
        }
    }
    Ok(failed)
}

/// Mark rows this process gave up writing as unverified, so the next read
/// rebuilds them even after a later write lands. `updated_at` moves too, so a
/// `since` read that looks up only changed rows finds them. Waits longer for
/// the rows than a write does; if it also fails, the rows wait for the next
/// message.
async fn distrust(pool: sqlx::PgPool, community: CommunityId, channels: Vec<Uuid>) -> Result<()> {
    let mut tx = crate::begin_community_write_transaction(
        &pool,
        community,
        observability::WriterOperation::EventWrite,
    )
    .await?;
    sqlx::raw_sql("SET LOCAL lock_timeout = '5000ms'; SET LOCAL statement_timeout = '10000ms'")
        .execute(tx.conn())
        .await?;
    sqlx::query(
        "UPDATE channel_heads SET seen_at = NULL, updated_at = clock_timestamp() \
         WHERE community_id = $1 AND channel_id = ANY($2) AND seen_at IS NOT NULL",
    )
    .bind(community.as_uuid())
    .bind(&channels)
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "UPDATE thread_heads SET seen_at = NULL, updated_at = clock_timestamp() \
         WHERE community_id = $1 AND channel_id = ANY($2) AND seen_at IS NOT NULL",
    )
    .bind(community.as_uuid())
    .bind(&channels)
    .execute(tx.conn())
    .await?;
    tx.commit().await
}

/// A signed time may be ahead of the clock. A row never claims to account
/// for the future, or one fast clock would hide every message stored until
/// then; the row instead reads as behind until that event is due.
pub(super) fn not_future(seen: &str) -> String {
    format!("(CASE WHEN {seen} > clock_timestamp() THEN clock_timestamp() ELSE {seen} END)")
}

/// How far the writer has read the log (`seen_new`), and what it may claim
/// for a row that has no `seen_at` yet (`seen_first`). Only the fill has read
/// the history, so only the fill sets the first value: a row made by ingest
/// stays unverified until a read fills it. Returns the `seen_at` value and
/// the condition under which it changes.
fn seen_sql(seen_new: &str, seen_first: &str) -> (String, String) {
    (
        format!(
            "CASE WHEN {seen_new} > h.seen_at THEN {seen_new} \
             WHEN h.seen_at IS NULL THEN {seen_first} ELSE h.seen_at END"
        ),
        format!("{seen_new} > h.seen_at OR (h.seen_at IS NULL AND {seen_first} IS NOT NULL)"),
    )
}

/// "Newer wins" for every part of a channel row, shared by ingest and fill,
/// so their order never matters. Later arrival wins and, within one arrival
/// time, the lower id; of two edits the later, then the higher id, as in the
/// timeline. `updated_at` moves only when a returned field changes, so
/// `since` is not woken by reactions or by replies inside a thread.
///
/// Every assignment is spelled out, not generated, so the event-floor source
/// policy can read the `SET` list and see that it never touches `events`.
pub(super) fn channel_merge_sql(seen_new: &str, seen_first: &str) -> String {
    const HEAD: &str = "((EXCLUDED.head_received_at, h.head_id) \
        > (COALESCE(h.head_received_at, '-infinity'), EXCLUDED.head_id) \
        OR (EXCLUDED.head_id IS NOT NULL AND h.head_id IS NULL))";
    const EDIT: &str = "(EXCLUDED.head_id = h.head_id \
        AND (EXCLUDED.head_edit_at, EXCLUDED.head_edit_id) \
        > (COALESCE(h.head_edit_at, '-infinity'), COALESCE(h.head_edit_id, '\\x')))";
    let (seen, seen_changed) = seen_sql(seen_new, seen_first);
    let text = format!("({HEAD} OR {EDIT})");
    format!(
        "ON CONFLICT (community_id, channel_id) DO UPDATE SET \
            seen_at = {seen}, \
            head_id = CASE WHEN {HEAD} THEN EXCLUDED.head_id ELSE h.head_id END, \
            head_created_at = CASE WHEN {HEAD} \
                THEN EXCLUDED.head_created_at ELSE h.head_created_at END, \
            head_received_at = CASE WHEN {HEAD} \
                THEN EXCLUDED.head_received_at ELSE h.head_received_at END, \
            head_pubkey = CASE WHEN {HEAD} THEN EXCLUDED.head_pubkey ELSE h.head_pubkey END, \
            head_text = CASE WHEN {text} THEN EXCLUDED.head_text ELSE h.head_text END, \
            head_edit_at = CASE WHEN {text} THEN EXCLUDED.head_edit_at ELSE h.head_edit_at END, \
            head_edit_id = CASE WHEN {text} THEN EXCLUDED.head_edit_id ELSE h.head_edit_id END, \
            updated_at = CASE WHEN {text} THEN clock_timestamp() ELSE h.updated_at END \
         WHERE {seen_changed} OR {text}"
    )
}

/// The same for a thread row.
pub(super) fn thread_merge_sql(seen_new: &str, seen_first: &str) -> String {
    const ACTIVITY: &str = "((EXCLUDED.activity_at, h.activity_id) \
        > (COALESCE(h.activity_at, '-infinity'), EXCLUDED.activity_id) \
        OR (EXCLUDED.activity_id IS NOT NULL AND h.activity_id IS NULL))";
    let (seen, seen_changed) = seen_sql(seen_new, seen_first);
    format!(
        "ON CONFLICT (community_id, root_id) DO UPDATE SET \
            seen_at = {seen}, \
            activity_at = CASE WHEN {ACTIVITY} \
                THEN EXCLUDED.activity_at ELSE h.activity_at END, \
            activity_id = CASE WHEN {ACTIVITY} \
                THEN EXCLUDED.activity_id ELSE h.activity_id END, \
            activity_created_at = CASE WHEN {ACTIVITY} \
                THEN EXCLUDED.activity_created_at ELSE h.activity_created_at END, \
            activity_pubkey = CASE WHEN {ACTIVITY} \
                THEN EXCLUDED.activity_pubkey ELSE h.activity_pubkey END, \
            activity_text = CASE WHEN {ACTIVITY} \
                THEN EXCLUDED.activity_text ELSE h.activity_text END, \
            updated_at = CASE WHEN {ACTIVITY} THEN clock_timestamp() ELSE h.updated_at END \
         WHERE {seen_changed} OR {ACTIVITY}"
    )
}

async fn apply_one(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    staged: &Staged,
) -> Result<()> {
    if staged.seen.is_some() || staged.head.is_some() {
        // A head goes in with its latest stored edit. Another relay may have
        // written that edit while this head was still on its way, and its
        // upsert found no head to apply it to. One probe of the edit-target
        // index.
        let sql = format!(
            "INSERT INTO channel_heads AS h (community_id, channel_id, head_id, \
                head_created_at, head_received_at, head_pubkey, head_text, head_edit_at, \
                head_edit_id, seen_at, updated_at) \
             SELECT $1, $2, $3, $4, $5, $6, COALESCE(left(edit.content, $9), $7), \
                edit.created_at, edit.id, NULL, clock_timestamp() \
             FROM (SELECT) one \
             LEFT JOIN LATERAL ( \
                SELECT content, created_at, id FROM events \
                WHERE community_id = $1 AND channel_id = $2 AND kind = 40003 \
                    AND deleted_at IS NULL AND {target} = $3::bytea \
                ORDER BY created_at DESC, id DESC LIMIT 1 \
             ) edit ON $3::bytea IS NOT NULL {merge}",
            merge = channel_merge_sql(&not_future("$8::timestamptz"), "NULL::timestamptz"),
            target = crate::store::media::MEDIA_EDIT_TARGET_SQL,
        );
        let head = staged.head.as_ref();
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(channel)
            .bind(head.map(|m| m.id.to_vec()))
            .bind(head.map(|m| m.created_at))
            .bind(head.map(|m| m.received_at))
            .bind(head.map(|m| m.pubkey.to_vec()))
            .bind(head.map(|m| m.text.as_str()))
            .bind(staged.seen)
            .bind(PREVIEW_CHARS as i32)
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
    for (root, thread) in &staged.threads {
        let sql = format!(
            "INSERT INTO thread_heads AS h (community_id, root_id, channel_id, activity_at, \
                activity_id, activity_created_at, activity_pubkey, activity_text, \
                seen_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, clock_timestamp()) {merge}",
            merge = thread_merge_sql(&not_future("$9::timestamptz"), "NULL::timestamptz"),
        );
        let newest = thread.newest.as_ref();
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(root.as_slice())
            .bind(channel)
            .bind(newest.map(|m| m.received_at))
            .bind(newest.map(|m| m.id.to_vec()))
            .bind(newest.map(|m| m.created_at))
            .bind(newest.map(|m| m.pubkey.to_vec()))
            .bind(newest.map(|m| m.text.as_str()))
            .bind(thread.seen)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Move authors' read positions and mark the people a reply pulls into its
/// thread. Rows are made only for a channel that exists and, for anyone but
/// the author, only for its current members.
async fn apply_people(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    staged: &Staged,
) -> Result<()> {
    if staged.people.is_empty() && staged.parents.is_empty() {
        return Ok(());
    }
    let mut actors: Vec<Option<Vec<u8>>> = Vec::new();
    let mut parents: Vec<Option<Vec<u8>>> = Vec::new();
    let mut parent_ats: Vec<Option<DateTime<Utc>>> = Vec::new();
    let mut roots: Vec<Vec<u8>> = Vec::new();
    let mut read_ats: Vec<Option<DateTime<Utc>>> = Vec::new();
    let mut read_ids: Vec<Option<Vec<u8>>> = Vec::new();
    for ((actor, root), read) in &staged.people {
        actors.push(Some(actor.to_vec()));
        parents.push(None);
        parent_ats.push(None);
        roots.push(root.map(|root| root.to_vec()).unwrap_or_default());
        read_ats.push(read.map(|(at, _)| at));
        read_ids.push(read.map(|(_, id)| id.to_vec()));
    }
    for (root, parent, at) in &staged.parents {
        actors.push(None);
        parents.push(Some(parent.to_vec()));
        parent_ats.push(Some(*at));
        roots.push(root.to_vec());
        read_ats.push(None);
        read_ids.push(None);
    }
    // One row per (person, scope), the latest read position first. The
    // parent's author is read from the stored parent.
    let rows = sqlx::query(
        "SELECT DISTINCT ON (r.actor, t.root) r.actor, t.root, t.read_at, t.read_id \
         FROM unnest($3::bytea[], $4::bytea[], $5::timestamptz[], $6::bytea[], \
                $7::timestamptz[], $8::bytea[]) \
            AS t(actor, parent, parent_at, root, read_at, read_id) \
         CROSS JOIN LATERAL (SELECT COALESCE(t.actor, (SELECT e.pubkey FROM events e \
            WHERE e.community_id = $1 AND e.created_at = t.parent_at AND e.id = t.parent \
                AND e.channel_id = $2)) AS actor) r \
         WHERE r.actor IS NOT NULL \
            AND EXISTS (SELECT 1 FROM channels c WHERE c.community_id = $1 AND c.id = $2) \
            AND (t.read_at IS NOT NULL OR EXISTS (SELECT 1 FROM channel_members m \
                WHERE m.community_id = $1 AND m.channel_id = $2 AND m.pubkey = r.actor \
                    AND m.removed_at IS NULL)) \
         ORDER BY r.actor, t.root, t.read_at DESC NULLS LAST",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(&actors)
    .bind(&parents)
    .bind(&parent_ats)
    .bind(&roots)
    .bind(&read_ats)
    .bind(&read_ids)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut actors: Vec<Vec<u8>> = Vec::with_capacity(rows.len());
    let mut roots: Vec<Vec<u8>> = Vec::with_capacity(rows.len());
    let mut read_ats: Vec<Option<DateTime<Utc>>> = Vec::with_capacity(rows.len());
    let mut read_ids: Vec<Option<Vec<u8>>> = Vec::with_capacity(rows.len());
    for row in rows {
        actors.push(row.try_get("actor")?);
        roots.push(row.try_get("root")?);
        read_ats.push(row.try_get("read_at")?);
        read_ids.push(row.try_get("read_id")?);
    }
    // Accounts first, as a read intent takes them: account, then frontier.
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id, actor) \
         SELECT DISTINCT $1, a.actor FROM unnest($2::bytea[]) AS a(actor) ORDER BY a.actor \
         ON CONFLICT (community_id, actor) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(&actors)
    .execute(&mut *conn)
    .await?;
    // `seq` is left alone: a `since` read finds these rows through the shared
    // row this same transaction updates. A manual unread is left alone: this
    // write runs after its message committed and cannot tell whether the flag
    // was set before or after it. Only `mark_through` clears it.
    sqlx::query(
        "INSERT INTO personal_read_frontiers AS f (community_id, actor, channel_id, root_id, \
            through_timestamp, through_message_id, engaged_at) \
         SELECT $1, t.actor, $2, t.root, t.read_at, t.read_id, clock_timestamp() \
         FROM unnest($3::bytea[], $4::bytea[], $5::timestamptz[], $6::bytea[]) \
            AS t(actor, root, read_at, read_id) \
         ORDER BY t.actor, t.root \
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET \
            through_message_id = CASE WHEN EXCLUDED.through_timestamp \
                    > COALESCE(f.through_timestamp, '-infinity') \
                THEN EXCLUDED.through_message_id ELSE f.through_message_id END, \
            through_timestamp = GREATEST(f.through_timestamp, EXCLUDED.through_timestamp), \
            engaged_at = EXCLUDED.engaged_at",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(&actors)
    .bind(&roots)
    .bind(&read_ats)
    .bind(&read_ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
