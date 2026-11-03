//! Guarded task archive and restore, the `nexus.task` hierarchy projection,
//! and the authoritative lifecycle state read.
//!
//! Every channel writer takes its locks in this order and never takes an
//! earlier lock after a later one:
//!
//! 1. community admission (the transaction begin);
//! 2. task family locks, sorted by family root;
//! 3. channel gates, sorted by channel: EXCLUSIVE for a writer that changes
//!    archive state, SHARED for a write that [`crate::ArchiveGate::Refuse`] guards;
//! 4. channel membership locks, sorted by channel;
//! 5. channel TTL locks, sorted by channel;
//! 6. channel rows, sorted by channel;
//! 7. the record, canvas or replacement coordinate lock, then its head.
//!
//! A writer skips the steps it does not need. A family is keyed by its root.
//! Every edge write takes the family locks of the child's and the new
//! parent's roots, so a holder of a root's lock sees a stable topology for
//! that family.
//!
//! The gate makes "archived" final for refused writes. An archive writer
//! holds the gate exclusive until it commits. A refused write takes the gate
//! shared as the first lock of its own transaction and then re-reads
//! `archived_at` in a new statement. So the write either commits before the
//! archive starts, or it sees the archive and stores nothing.
use crate::{AdmittedTx, DbError, Result};
use buzz_core::lifecycle::{lifecycle_digest, LifecycleScope, TokenEntry};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Longest parent chain a task family may have. An edge write that would
/// make a deeper chain is refused, so every walk is bounded.
pub const MAX_TASK_DEPTH: usize = 256;

/// Most channels one state read returns.
pub const MAX_STATE_CHANNELS: usize = 1000;

/// Refusal for deleting a task, as a channel or as a record, while it has
/// child tasks.
pub const CHILD_TASKS_DELETE: &str = "a task with child tasks cannot be deleted";

/// A fresh lifecycle token from the OS CSPRNG. Every lifecycle writer sets
/// one in the same transaction as its change.
pub(crate) fn new_token() -> Uuid {
    Uuid::new_v4()
}

fn family_lock_key(community: CommunityId, root: Uuid) -> String {
    format!("buzz_task_family:{}:{root}", community.as_uuid())
}

/// The per-channel archive gate. Archive and restore take it exclusive.
pub(crate) fn gate_lock_key(community: CommunityId, channel: Uuid) -> String {
    format!("buzz_channel_gate:{}:{channel}", community.as_uuid())
}

/// The refusal for a write into an archived channel, as
/// [`DbError::AccessDenied`].
pub const CHANNEL_ARCHIVED: &str = "channel is archived";

/// The archive policy of one channel write. Every function that writes
/// into a channel takes one; there is no default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveGate {
    /// The write is refused in an archived channel. It takes each channel's
    /// gate SHARED as the first lock of its transaction, then re-reads
    /// `archived_at` and fails with [`DbError::AccessDenied`]
    /// ([`CHANNEL_ARCHIVED`]) if any channel is archived. Client writes use
    /// this.
    Refuse,
    /// The write is allowed in an archived channel and takes no gate: relay
    /// notices, a member leaving by themselves, per-user view state and
    /// operator moderation.
    Exempt,
}

/// The write gates one transaction holds, from [`lock_write_gates`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteGates {
    gate: ArchiveGate,
    channels: BTreeSet<Uuid>,
}

impl WriteGates {
    /// No gates: an [`ArchiveGate::Exempt`] write.
    pub fn exempt() -> Self {
        Self {
            gate: ArchiveGate::Exempt,
            channels: BTreeSet::new(),
        }
    }

    /// Whether the write is [`ArchiveGate::Exempt`].
    pub fn is_exempt(&self) -> bool {
        self.gate == ArchiveGate::Exempt
    }

    /// Whether this transaction may write into `channel`: the write is
    /// exempt, or it holds `channel`'s gate.
    pub fn covers(&self, channel: Uuid) -> bool {
        self.gate == ArchiveGate::Exempt || self.channels.contains(&channel)
    }
}

/// Take the write gates of `channels` on `tx` under `gate`, then refuse if
/// any of them is archived. [`ArchiveGate::Exempt`] takes nothing.
///
/// Call this as the first lock of the write transaction (after the family
/// locks of a task record), and only once: a gate taken later in the same
/// transaction could deadlock with an archive writer. A
/// transaction-scoped primitive, such as
/// [`crate::event::insert_event_in_transaction`], takes no gate: its
/// transaction owner calls this first.
pub async fn lock_write_gates(
    tx: &mut AdmittedTx,
    gate: ArchiveGate,
    channels: &[Uuid],
) -> Result<WriteGates> {
    if gate == ArchiveGate::Exempt {
        return Ok(WriteGates::exempt());
    }
    let channels: BTreeSet<Uuid> = channels.iter().copied().collect();
    lock_shared_gates(tx, &channels).await?;
    if !archived_among(tx, &channels).await?.is_empty() {
        return Err(DbError::AccessDenied(CHANNEL_ARCHIVED.into()));
    }
    Ok(WriteGates { gate, channels })
}

/// Take the gates of `channels` SHARED, sorted, in one statement.
pub(crate) async fn lock_shared_gates(
    tx: &mut AdmittedTx,
    channels: &BTreeSet<Uuid>,
) -> Result<()> {
    let community = tx.community();
    let keys: Vec<_> = channels
        .iter()
        .map(|channel| gate_lock_key(community, *channel))
        .collect();
    advisory_locks(tx, &keys, true).await
}

/// The archived channels among `channels`. Run it as its own statement
/// after the gates, so it reads every archive committed before them.
pub(crate) async fn archived_among(
    tx: &mut AdmittedTx,
    channels: &BTreeSet<Uuid>,
) -> Result<BTreeSet<Uuid>> {
    if channels.is_empty() {
        return Ok(BTreeSet::new());
    }
    let community = tx.community();
    let ids: Vec<Uuid> = channels.iter().copied().collect();
    let archived: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM channels \
         WHERE community_id = $1 AND id = ANY($2) AND archived_at IS NOT NULL",
    )
    .bind(community.as_uuid())
    .bind(&ids)
    .fetch_all(tx.conn())
    .await?;
    Ok(archived.into_iter().collect())
}

/// Take transaction-scoped advisory locks on `keys`, in order, in one
/// statement.
async fn advisory_locks(tx: &mut AdmittedTx, keys: &[String], shared: bool) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let sql = if shared {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended(k, 0)) \
         FROM unnest($1::text[]) WITH ORDINALITY AS t(k, n) ORDER BY n"
    } else {
        "SELECT pg_advisory_xact_lock(hashtextextended(k, 0)) \
         FROM unnest($1::text[]) WITH ORDINALITY AS t(k, n) ORDER BY n"
    };
    sqlx::query(sql).bind(keys).execute(tx.conn()).await?;
    Ok(())
}

async fn lock_families(tx: &mut AdmittedTx, roots: &BTreeSet<Uuid>) -> Result<()> {
    let community = tx.community();
    let keys: Vec<_> = roots
        .iter()
        .map(|root| family_lock_key(community, *root))
        .collect();
    advisory_locks(tx, &keys, false).await
}

/// Take the gates of `channels` EXCLUSIVE, sorted, in one statement. Every
/// writer that changes archive state holds them until it commits.
pub(crate) async fn lock_gates(tx: &mut AdmittedTx, channels: &BTreeSet<Uuid>) -> Result<()> {
    let community = tx.community();
    let keys: Vec<_> = channels
        .iter()
        .map(|channel| gate_lock_key(community, *channel))
        .collect();
    advisory_locks(tx, &keys, false).await
}

async fn lock_ttls_shared(tx: &mut AdmittedTx, channels: &BTreeSet<Uuid>) -> Result<()> {
    let community = tx.community();
    let keys: Vec<_> = channels
        .iter()
        .map(|channel| crate::channel::channel_ttl_lock_key(community, *channel))
        .collect();
    advisory_locks(tx, &keys, true).await
}

/// Whether `channel` is a task channel: it has a projection row.
pub(crate) async fn is_task_channel(tx: &mut AdmittedTx, channel: Uuid) -> Result<bool> {
    Ok(projected_parent(tx, channel).await?.is_some())
}

/// The projected parent of `channel`: `None` when the channel has no
/// projection row, `Some(None)` for a root.
async fn projected_parent(tx: &mut AdmittedTx, channel: Uuid) -> Result<Option<Option<Uuid>>> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT parent_channel_id FROM artifact_parents \
         WHERE community_id = $1 AND child_channel_id = $2",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_optional(tx.conn())
    .await?)
}

/// Whether any projection row names `channel` as its parent.
pub(crate) async fn has_child_tasks(tx: &mut AdmittedTx, channel: Uuid) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM artifact_parents \
         WHERE community_id = $1 AND parent_channel_id = $2)",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_one(tx.conn())
    .await?)
}

/// The ancestors of a channel, nearest first, from the projection.
struct Walk {
    ancestors: Vec<Uuid>,
    /// `false` when the chain is deeper than [`MAX_TASK_DEPTH`] or ends at a
    /// channel without a projection row. Edge writes keep both from
    /// happening; a request that meets one is refused, never guessed.
    intact: bool,
}

impl Walk {
    fn root(&self, channel: Uuid) -> Uuid {
        self.ancestors.last().copied().unwrap_or(channel)
    }
}

async fn walk_up(tx: &mut AdmittedTx, channel: Uuid) -> Result<Walk> {
    let community = tx.community();
    let rows = sqlx::query(
        "WITH RECURSIVE up(id, depth) AS ( \
             SELECT parent_channel_id, 1 FROM artifact_parents \
              WHERE community_id = $1 AND child_channel_id = $2 \
                AND parent_channel_id IS NOT NULL \
           UNION ALL \
             SELECT ap.parent_channel_id, up.depth + 1 FROM up \
               JOIN artifact_parents AS ap \
                 ON ap.community_id = $1 AND ap.child_channel_id = up.id \
              WHERE ap.parent_channel_id IS NOT NULL AND up.depth <= $3 \
         ) \
         SELECT up.id, EXISTS (SELECT 1 FROM artifact_parents AS r \
                 WHERE r.community_id = $1 AND r.child_channel_id = up.id) AS projected \
           FROM up ORDER BY up.depth",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(MAX_TASK_DEPTH as i32)
    .fetch_all(tx.conn())
    .await?;
    let mut ancestors = Vec::with_capacity(rows.len());
    let mut intact = rows.len() <= MAX_TASK_DEPTH;
    for row in rows {
        intact &= row.try_get::<bool, _>("projected")?;
        ancestors.push(row.try_get("id")?);
    }
    Ok(Walk { ancestors, intact })
}

/// The descendants of `channel` from the projection, or `None` when there
/// are more than `cap` or the subtree is deeper than [`MAX_TASK_DEPTH`].
async fn walk_down(tx: &mut AdmittedTx, channel: Uuid, cap: usize) -> Result<Option<Vec<Uuid>>> {
    let community = tx.community();
    let rows = sqlx::query(
        "WITH RECURSIVE down(id, depth) AS ( \
             SELECT child_channel_id, 1 FROM artifact_parents \
              WHERE community_id = $1 AND parent_channel_id = $2 \
           UNION ALL \
             SELECT ap.child_channel_id, down.depth + 1 FROM down \
               JOIN artifact_parents AS ap \
                 ON ap.community_id = $1 AND ap.parent_channel_id = down.id \
              WHERE down.depth <= $3 \
         ) \
         SELECT id, depth FROM down LIMIT $4",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(MAX_TASK_DEPTH as i32)
    .bind(i64::try_from(cap.saturating_add(1)).unwrap_or(i64::MAX))
    .fetch_all(tx.conn())
    .await?;
    if rows.len() > cap {
        return Ok(None);
    }
    let mut descendants = Vec::with_capacity(rows.len());
    for row in rows {
        if row.try_get::<i32, _>("depth")? as usize > MAX_TASK_DEPTH {
            return Ok(None);
        }
        descendants.push(row.try_get("id")?);
    }
    Ok(Some(descendants))
}

/// A locked channel row's lifecycle state.
#[derive(Clone, Copy)]
struct RowState {
    archived: bool,
    deleted: bool,
    token: Option<Uuid>,
    ttl: bool,
    created_by_actor: bool,
}

impl RowState {
    fn active(&self) -> bool {
        !self.archived && !self.deleted
    }
}

/// Lock `channels` in id order and read their state. A purged channel has
/// no entry.
async fn lock_rows(
    tx: &mut AdmittedTx,
    channels: &BTreeSet<Uuid>,
    actor: &[u8],
    share: bool,
) -> Result<BTreeMap<Uuid, RowState>> {
    let community = tx.community();
    let ids: Vec<Uuid> = channels.iter().copied().collect();
    let sql = if share {
        "SELECT id, archived_at IS NOT NULL AS archived, deleted_at IS NOT NULL AS deleted, \
                lifecycle_token, ttl_seconds IS NOT NULL AS ttl, created_by = $3 AS mine \
           FROM channels WHERE community_id = $1 AND id = ANY($2) ORDER BY id FOR SHARE"
    } else {
        "SELECT id, archived_at IS NOT NULL AS archived, deleted_at IS NOT NULL AS deleted, \
                lifecycle_token, ttl_seconds IS NOT NULL AS ttl, created_by = $3 AS mine \
           FROM channels WHERE community_id = $1 AND id = ANY($2) ORDER BY id FOR NO KEY UPDATE"
    };
    let rows = sqlx::query(sql)
        .bind(community.as_uuid())
        .bind(&ids)
        .bind(actor)
        .fetch_all(tx.conn())
        .await?;
    let mut states = BTreeMap::new();
    for row in rows {
        states.insert(
            row.try_get("id")?,
            RowState {
                archived: row.try_get("archived")?,
                deleted: row.try_get("deleted")?,
                token: row.try_get("lifecycle_token")?,
                ttl: row.try_get("ttl")?,
                created_by_actor: row.try_get("mine")?,
            },
        );
    }
    Ok(states)
}

/// The channels in `channels` where `actor` lacks today's archive right:
/// an owner or admin membership, or ownership of an owner-role agent member.
async fn unauthorized(tx: &mut AdmittedTx, channels: &[Uuid], actor: &[u8]) -> Result<bool> {
    let community = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM unnest($2::uuid[]) AS c(id) WHERE NOT ( \
             EXISTS (SELECT 1 FROM channel_members AS m \
                      WHERE m.community_id = $1 AND m.channel_id = c.id AND m.pubkey = $3 \
                        AND m.removed_at IS NULL AND m.role IN ('owner', 'admin')) \
          OR EXISTS (SELECT 1 FROM channel_members AS m \
                       JOIN users AS u ON u.community_id = m.community_id AND u.pubkey = m.pubkey \
                      WHERE m.community_id = $1 AND m.channel_id = c.id \
                        AND m.removed_at IS NULL AND m.role = 'owner' \
                        AND u.agent_owner_pubkey = $3)))",
    )
    .bind(community.as_uuid())
    .bind(channels)
    .bind(actor)
    .fetch_one(tx.conn())
    .await?)
}

/// The result of a guarded lifecycle request.
#[derive(Debug)]
pub struct LifecycleWrite {
    /// The stored kind:9002 event.
    pub stored: StoredEvent,
    /// `false` when the event was already stored: it applied the first time.
    pub inserted: bool,
    /// Channels whose archive state changed, the target first, each with
    /// the kind:40099 notice stored for it. Empty for a fence and for a
    /// duplicate.
    pub changed: Vec<(Uuid, Event)>,
}

/// A guarded archive or restore request.
pub struct StateRequest<'a> {
    /// The target channel S.
    pub target: Uuid,
    /// The signed kind:9002.
    pub event: &'a Event,
    /// `true` archives, `false` restores.
    pub archive: bool,
    /// Which channels a restore writes. Archive is always `Subtree`.
    pub scope: LifecycleScope,
    /// The digest the client previewed.
    pub expect: [u8; 32],
    /// Most channels the request may lock.
    pub max_channels: usize,
}

enum Attempt<T> {
    Done(T),
    /// The family root moved before its lock was taken.
    Retry,
}

/// Apply a guarded archive or restore (see [`buzz_core::lifecycle`]).
/// A family that changes root twice before its lock is taken is refused
/// with [`DbError::LifecycleConflict`].
pub(crate) async fn apply_lifecycle_state(
    pool: &PgPool,
    push_enqueue: Option<crate::runtime::push_enqueue::PushEnqueue>,
    community: CommunityId,
    request: &StateRequest<'_>,
    relay_keys: &Keys,
) -> Result<LifecycleWrite> {
    for _ in 0..2 {
        let mut tx = crate::begin_community_event_write_transaction(
            pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        tx.set_push_enqueue(push_enqueue.clone());
        if let Attempt::Done(write) = state_attempt(tx, request, relay_keys).await? {
            return Ok(write);
        }
    }
    Err(DbError::LifecycleConflict)
}

async fn state_attempt(
    mut tx: AdmittedTx,
    request: &StateRequest<'_>,
    relay_keys: &Keys,
) -> Result<Attempt<LifecycleWrite>> {
    let target = request.target;
    let actor = request.event.pubkey.to_bytes();
    let root = walk_up(&mut tx, target).await?.root(target);
    lock_families(&mut tx, &BTreeSet::from([root])).await?;
    let walk = walk_up(&mut tx, target).await?;
    if walk.root(target) != root {
        return Ok(Attempt::Retry);
    }
    // The family is stable from here on.
    let projected = projected_parent(&mut tx, target).await?.is_some();
    let needs_descendants = request.archive || request.scope == LifecycleScope::Subtree;
    let descendants = if projected && needs_descendants {
        walk_down(&mut tx, target, request.max_channels).await?
    } else {
        Some(Vec::new())
    };
    let mut rows: BTreeSet<Uuid> = BTreeSet::from([target]);
    let mut writable = rows.clone();
    let usable = projected && walk.intact;
    if usable {
        rows.extend(&walk.ancestors);
        if !request.archive {
            writable.extend(&walk.ancestors);
        }
        if let Some(descendants) = &descendants {
            rows.extend(descendants);
            writable.extend(descendants);
        }
    }
    let too_large = descendants.is_none() || rows.len() > request.max_channels;
    if too_large {
        // Refused below; lock only the target so the duplicate check runs.
        rows = BTreeSet::from([target]);
        writable = rows.clone();
    }
    lock_gates(&mut tx, &writable).await?;
    lock_ttls_shared(&mut tx, &writable).await?;
    let states = lock_rows(&mut tx, &rows, &actor, false).await?;
    let target_state = states
        .get(&target)
        .filter(|state| !state.deleted)
        .copied()
        .ok_or(DbError::ChannelNotFound(target))?;

    let (stored, inserted) = crate::event::insert_event_with_thread_metadata_tx(
        &mut tx,
        request.event,
        Some(target),
        None,
    )
    .await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(Attempt::Done(LifecycleWrite {
            stored,
            inserted,
            changed: Vec::new(),
        }));
    }
    // The same state refusals as an unguarded request.
    match (target_state.archived, request.archive) {
        (true, true) => return Err(DbError::AccessDenied("channel is archived".into())),
        (false, false) => return Err(DbError::AccessDenied("channel is not archived".into())),
        _ => {}
    }
    if too_large {
        return Err(DbError::AccessDenied(
            "lifecycle request exceeds max_subtree_channels".into(),
        ));
    }
    if !usable {
        return Err(DbError::LifecycleConflict);
    }
    let (compared, affected) = digest_sets(
        request,
        &walk.ancestors,
        descendants.as_deref().unwrap_or_default(),
        &states,
    )
    .ok_or(DbError::LifecycleConflict)?;
    let digest = lifecycle_digest(request.archive, request.scope, target, compared, &affected);
    if digest != request.expect {
        return Err(DbError::LifecycleConflict);
    }
    let mut changed: Vec<Uuid> = affected.iter().map(|entry| entry.channel).collect();
    if unauthorized(&mut tx, &changed, &actor).await? {
        return Err(DbError::AccessDenied(
            "actor not authorized for every affected channel".into(),
        ));
    }
    write_states(&mut tx, &changed, request.archive).await?;
    // Target first, for the caller's ordered side effects.
    changed.sort_by_key(|channel| *channel != target);
    let now = chrono::Utc::now();
    let mut notices = Vec::with_capacity(changed.len());
    for channel in changed {
        let notice = system_notice(relay_keys, channel, target, request, now)?;
        let (_, message_inserted) =
            crate::event::insert_event_in_transaction(&mut tx, &notice, Some(channel)).await?;
        if message_inserted {
            crate::insert_mentions_in_transaction(&mut tx, &notice, Some(channel)).await?;
        }
        notices.push((channel, notice));
    }
    tx.commit().await?;
    Ok(Attempt::Done(LifecycleWrite {
        stored,
        inserted,
        changed: notices,
    }))
}

/// The compared entry and the affected entries of a guarded request, from
/// locked rows. `None` when the topology does not match the rows (for
/// example a deleted ancestor), which is refused as a conflict.
fn digest_sets(
    request: &StateRequest<'_>,
    ancestors: &[Uuid],
    descendants: &[Uuid],
    states: &BTreeMap<Uuid, RowState>,
) -> Option<(Option<TokenEntry>, Vec<TokenEntry>)> {
    let entry = |channel: Uuid| {
        states.get(&channel).map(|state| TokenEntry {
            channel,
            token: state.token,
        })
    };
    let target = states.get(&request.target)?;
    let mut affected = vec![TokenEntry {
        channel: request.target,
        token: target.token,
    }];
    let mut above = ancestors.iter();
    if !request.archive {
        // The minimum archived ancestor chain.
        let mut next = above.clone();
        while let Some(channel) = next.next() {
            let state = states.get(channel)?;
            if !state.archived {
                break;
            }
            if state.deleted {
                return None;
            }
            affected.push(entry(*channel)?);
            above = next.clone();
        }
    }
    let mut compared = None;
    for channel in above {
        let state = states.get(channel)?;
        if state.deleted {
            return None;
        }
        if state.active() {
            compared = entry(*channel);
            break;
        }
    }
    if request.archive || request.scope == LifecycleScope::Subtree {
        for channel in descendants {
            let state = states.get(channel)?;
            let writes = if request.archive {
                state.active()
            } else {
                state.archived && !state.deleted
            };
            if writes {
                affected.push(entry(*channel)?);
            }
        }
    }
    Some((compared, affected))
}

async fn write_states(tx: &mut AdmittedTx, channels: &[Uuid], archive: bool) -> Result<()> {
    let community = tx.community();
    let tokens: Vec<Uuid> = channels.iter().map(|_| new_token()).collect();
    let sql = if archive {
        "UPDATE channels AS ch SET archived_at = NOW(), lifecycle_token = t.token \
           FROM unnest($2::uuid[], $3::uuid[]) AS t(id, token) \
          WHERE ch.community_id = $1 AND ch.id = t.id \
            AND ch.deleted_at IS NULL AND ch.archived_at IS NULL"
    } else {
        "UPDATE channels AS ch SET archived_at = NULL, lifecycle_token = t.token, \
                ttl_deadline = CASE \
                    WHEN ch.ttl_seconds IS NOT NULL \
                        THEN NOW() + (ch.ttl_seconds || ' seconds')::interval \
                    ELSE ch.ttl_deadline \
                END \
           FROM unnest($2::uuid[], $3::uuid[]) AS t(id, token) \
          WHERE ch.community_id = $1 AND ch.id = t.id \
            AND ch.deleted_at IS NULL AND ch.archived_at IS NOT NULL"
    };
    let updated = sqlx::query(sql)
        .bind(community.as_uuid())
        .bind(channels)
        .bind(&tokens)
        .execute(tx.conn())
        .await?
        .rows_affected();
    // The rows are locked and were checked above.
    if updated != channels.len() as u64 {
        return Err(DbError::InvalidData(
            "guarded lifecycle write changed an unexpected row count".into(),
        ));
    }
    Ok(())
}

/// The kind:40099 notice for one changed channel. Channels other than the
/// target name it as `cascade_root`.
fn system_notice(
    relay_keys: &Keys,
    channel: Uuid,
    target: Uuid,
    request: &StateRequest<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Event> {
    let mut content = serde_json::json!({
        "type": if request.archive { "channel_archived" } else { "channel_unarchived" },
        "actor": request.event.pubkey.to_hex(),
    });
    if channel != target {
        content["cascade_root"] = serde_json::Value::String(target.to_string());
    }
    let tag =
        Tag::parse(["h", &channel.to_string()]).map_err(|e| DbError::InvalidData(e.to_string()))?;
    EventBuilder::new(Kind::Custom(40099), content.to_string())
        .tags([tag])
        .custom_created_at(nostr::Timestamp::from(now.timestamp() as u64))
        .sign_with_keys(relay_keys)
        .map_err(|e| DbError::InvalidData(e.to_string()))
}

/// Apply a fence: replace the target's token if it still equals
/// `expect_target`. It takes the target's shared TTL lock, then its row, and
/// changes nothing else.
pub(crate) async fn apply_lifecycle_fence(
    pool: &PgPool,
    push_enqueue: Option<crate::runtime::push_enqueue::PushEnqueue>,
    community: CommunityId,
    target: Uuid,
    event: &Event,
    expect_target: Option<Uuid>,
) -> Result<LifecycleWrite> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    tx.set_push_enqueue(push_enqueue);
    let actor = event.pubkey.to_bytes();
    let channels = BTreeSet::from([target]);
    lock_ttls_shared(&mut tx, &channels).await?;
    let state = lock_rows(&mut tx, &channels, &actor, false)
        .await?
        .remove(&target)
        .filter(|state| !state.deleted)
        .ok_or(DbError::ChannelNotFound(target))?;
    let (stored, inserted) =
        crate::event::insert_event_with_thread_metadata_tx(&mut tx, event, Some(target), None)
            .await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(LifecycleWrite {
            stored,
            inserted,
            changed: Vec::new(),
        });
    }
    if unauthorized(&mut tx, &[target], &actor).await? {
        return Err(DbError::AccessDenied(
            "actor not authorized for every affected channel".into(),
        ));
    }
    if state.token != expect_target {
        return Err(DbError::LifecycleConflict);
    }
    sqlx::query("UPDATE channels SET lifecycle_token = $3 WHERE community_id = $1 AND id = $2")
        .bind(tx.community().as_uuid())
        .bind(target)
        .bind(new_token())
        .execute(tx.conn())
        .await?;
    tx.commit().await?;
    Ok(LifecycleWrite {
        stored,
        inserted,
        changed: Vec::new(),
    })
}

/// How an accepted `nexus.task` revision changes the projection.
pub(crate) enum EdgeChange {
    /// Insert or replace the channel's row.
    Upsert {
        child: Uuid,
        parent: Option<Uuid>,
        artifact: Uuid,
    },
    /// Remove the channel's row.
    Remove { child: Uuid },
}

/// The families an edge write holds, and what it read under them.
pub(crate) struct EdgeLocks {
    child_parent: Option<Option<Uuid>>,
    new_parent_chain: Option<Walk>,
    rows: BTreeMap<Uuid, RowState>,
}

/// Lock what a `nexus.task` revision for `child` with parent `parent`
/// needs: the families of both, the write gates `gates` (shared, empty for
/// an exempt write), the TTL locks and the rows (`FOR SHARE`). Returns `None`
/// when a family root moved before its lock was taken; the caller retries
/// once.
pub(crate) async fn lock_task_edge(
    tx: &mut AdmittedTx,
    child: Uuid,
    parent: Option<Uuid>,
    author: &[u8],
    gates: &BTreeSet<Uuid>,
) -> Result<Option<EdgeLocks>> {
    let roots = |child_walk: &Walk, parent_walk: Option<&Walk>| {
        let mut roots = BTreeSet::from([child_walk.root(child)]);
        if let (Some(parent), Some(walk)) = (parent, parent_walk) {
            roots.insert(walk.root(parent));
        }
        roots
    };
    let child_walk = walk_up(tx, child).await?;
    let parent_walk = match parent {
        Some(parent) => Some(walk_up(tx, parent).await?),
        None => None,
    };
    let locked = roots(&child_walk, parent_walk.as_ref());
    lock_families(tx, &locked).await?;
    let child_walk = walk_up(tx, child).await?;
    let parent_walk = match parent {
        Some(parent) => Some(walk_up(tx, parent).await?),
        None => None,
    };
    if roots(&child_walk, parent_walk.as_ref()) != locked {
        return Ok(None);
    }
    lock_shared_gates(tx, gates).await?;
    let mut channels = BTreeSet::from([child]);
    channels.extend(parent);
    lock_ttls_shared(tx, &channels).await?;
    let rows = lock_rows(tx, &channels, author, true).await?;
    Ok(Some(EdgeLocks {
        child_parent: projected_parent(tx, child).await?,
        new_parent_chain: parent_walk,
        rows,
    }))
}

impl EdgeLocks {
    /// The child's projected parent, if it has a row.
    pub(crate) fn current_parent(&self) -> Option<Option<Uuid>> {
        self.child_parent
    }
}

/// Check a `nexus.task` revision against the locked family. `Err` carries a
/// public refusal.
pub(crate) async fn check_task_edge(
    tx: &mut AdmittedTx,
    locks: &EdgeLocks,
    child: Uuid,
    parent: Option<Uuid>,
    delete: bool,
) -> Result<std::result::Result<(), &'static str>> {
    let Some(child_row) = locks.rows.get(&child).filter(|row| !row.deleted) else {
        return Ok(Err("task channel is not live"));
    };
    if !child_row.created_by_actor {
        return Ok(Err("nexus.task author must own its channel"));
    }
    if delete {
        if has_child_tasks(tx, child).await? {
            return Ok(Err(CHILD_TASKS_DELETE));
        }
        return Ok(Ok(()));
    }
    if child_row.archived {
        return Ok(Err("task channel is not live"));
    }
    if child_row.ttl {
        return Ok(Err("task channels cannot have a ttl"));
    }
    let Some(parent) = parent else {
        return Ok(Ok(()));
    };
    let Some(parent_row) = locks.rows.get(&parent).filter(|row| row.active()) else {
        return Ok(Err("parent must be a live task"));
    };
    if !parent_row.created_by_actor {
        return Ok(Err("parent task has another owner"));
    }
    if parent_row.ttl || projected_parent(tx, parent).await?.is_none() {
        return Ok(Err("parent must be a live task"));
    }
    let Some(chain) = locks.new_parent_chain.as_ref().filter(|walk| walk.intact) else {
        return Ok(Err("parent must be a live task"));
    };
    if chain.ancestors.contains(&child) {
        return Ok(Err("task parent would form a cycle"));
    }
    // The deepest descendant of the child ends up this far below the root.
    let height = if locks.current_parent().is_some() {
        subtree_height(tx, child).await?
    } else {
        Some(0)
    };
    match height {
        Some(height) if chain.ancestors.len() + 1 + height < MAX_TASK_DEPTH => Ok(Ok(())),
        _ => Ok(Err("task hierarchy is too deep")),
    }
}

/// The depth of the deepest projected descendant of `channel` (0 for a
/// leaf), or `None` beyond [`MAX_TASK_DEPTH`].
async fn subtree_height(tx: &mut AdmittedTx, channel: Uuid) -> Result<Option<usize>> {
    let community = tx.community();
    let height: Option<i32> = sqlx::query_scalar(
        "WITH RECURSIVE down(id, depth) AS ( \
             SELECT child_channel_id, 1 FROM artifact_parents \
              WHERE community_id = $1 AND parent_channel_id = $2 \
           UNION ALL \
             SELECT ap.child_channel_id, down.depth + 1 FROM down \
               JOIN artifact_parents AS ap \
                 ON ap.community_id = $1 AND ap.parent_channel_id = down.id \
              WHERE down.depth <= $3 \
         ) \
         SELECT max(depth) FROM down",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(MAX_TASK_DEPTH as i32)
    .fetch_one(tx.conn())
    .await?;
    let height = height.unwrap_or(0) as usize;
    Ok((height <= MAX_TASK_DEPTH).then_some(height))
}

/// Write an accepted revision's projection change.
pub(crate) async fn write_task_edge(tx: &mut AdmittedTx, change: EdgeChange) -> Result<()> {
    let community = tx.community();
    match change {
        EdgeChange::Upsert {
            child,
            parent,
            artifact,
        } => {
            sqlx::query(
                "INSERT INTO artifact_parents \
                     (community_id, child_channel_id, parent_channel_id, artifact_id) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (community_id, child_channel_id) DO UPDATE \
                 SET parent_channel_id = EXCLUDED.parent_channel_id, \
                     artifact_id = EXCLUDED.artifact_id",
            )
            .bind(community.as_uuid())
            .bind(child)
            .bind(parent)
            .bind(artifact)
            .execute(tx.conn())
            .await?;
        }
        EdgeChange::Remove { child } => {
            remove_task_edge(tx, child).await?;
        }
    }
    Ok(())
}

/// Remove `channel`'s projection row, if any.
pub(crate) async fn remove_task_edge(tx: &mut AdmittedTx, channel: Uuid) -> Result<()> {
    let community = tx.community();
    sqlx::query("DELETE FROM artifact_parents WHERE community_id = $1 AND child_channel_id = $2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// One channel's lifecycle state, as the state read returns it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ChannelLifecycleState {
    /// The channel.
    pub id: Uuid,
    /// When it was archived, in unix seconds, or `None` when active.
    pub archived_at: Option<i64>,
    /// Whether it is deleted. Only this marks a channel as gone.
    pub deleted: bool,
    /// Its lifecycle token, or `None` if never set.
    pub lifecycle_token: Option<Uuid>,
}

impl crate::Db {
    /// Apply a guarded archive or restore. See [`apply_lifecycle_state`].
    pub async fn apply_lifecycle_state(
        &self,
        community: CommunityId,
        request: &StateRequest<'_>,
        relay_keys: &Keys,
    ) -> Result<LifecycleWrite> {
        apply_lifecycle_state(
            &self.pool,
            self.push_enqueue.clone(),
            community,
            request,
            relay_keys,
        )
        .await
    }

    /// Apply a lifecycle fence. See [`apply_lifecycle_fence`].
    pub async fn apply_lifecycle_fence(
        &self,
        community: CommunityId,
        target: Uuid,
        event: &Event,
        expect_target: Option<Uuid>,
    ) -> Result<LifecycleWrite> {
        apply_lifecycle_fence(
            &self.pool,
            self.push_enqueue.clone(),
            community,
            target,
            event,
            expect_target,
        )
        .await
    }

    /// The lifecycle state of the `channels` that `actor` created or is an
    /// active member of, sorted by id, from one statement. Other ids are
    /// omitted, which means "unknown", never "gone".
    pub async fn lifecycle_state(
        &self,
        community: CommunityId,
        actor: &[u8],
        channels: &[Uuid],
    ) -> Result<Vec<ChannelLifecycleState>> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT c.id, EXTRACT(EPOCH FROM c.archived_at)::bigint AS archived_at, \
                    c.deleted_at IS NOT NULL AS deleted, c.lifecycle_token \
               FROM channels AS c \
              WHERE c.community_id = $1 AND c.id = ANY($2) \
                AND (c.created_by = $3 OR EXISTS ( \
                     SELECT 1 FROM channel_members AS m \
                      WHERE m.community_id = c.community_id AND m.channel_id = c.id \
                        AND m.pubkey = $3 AND m.removed_at IS NULL)) \
              ORDER BY c.id",
        )
        .bind(community.as_uuid())
        .bind(channels)
        .bind(actor)
        .fetch_all(&mut *conn)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ChannelLifecycleState {
                    id: row.try_get("id")?,
                    archived_at: row.try_get("archived_at")?,
                    deleted: row.try_get("deleted")?,
                    lifecycle_token: row.try_get("lifecycle_token")?,
                })
            })
            .collect()
    }
}
