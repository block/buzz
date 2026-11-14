use super::model::*;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::{observability, AdmittedTx, Db, Result};

pub(crate) fn event_id(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 || value.bytes().any(|b| !b.is_ascii_hexdigit()) {
        return None;
    }
    hex::decode(value).ok()
}

pub(crate) async fn deadlines(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SET LOCAL statement_timeout = '2000ms'")
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Take the person's account row, starting the account if this is their
/// first personal write, and return the next value of their change counter.
/// Holding the row serializes the person's own writes, never shared
/// conversation rows, so counter order is commit order.
pub(super) async fn lock_account(
    tx: &mut AdmittedTx,
    community: CommunityId,
    actor: &[u8],
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO personal_read_accounts AS a (community_id,actor,started_at,seq)
         VALUES ($1,$2,now(),1)
         ON CONFLICT (community_id,actor) DO UPDATE
         SET started_at=COALESCE(a.started_at, now()), seq=a.seq+1
         RETURNING seq",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_one(tx.conn())
    .await?)
}

/// Resource access is independent of roster membership. Do not row-lock shared
/// conversation tables: private progress must not serialize legacy ingest or
/// deletion. A racing revoke hides projections; it need not erase private intent.
async fn access(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    channel: Uuid,
) -> Result<bool> {
    let visibility: Option<String> = sqlx::query_scalar(
        "SELECT visibility::text FROM channels
         WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_optional(&mut *conn)
    .await?;
    match visibility.as_deref() {
        None => Ok(false),
        Some("open") => Ok(true),
        Some(_) => Ok(sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT pubkey FROM channel_members WHERE community_id=$1 AND channel_id=$2
             AND pubkey=$3 AND removed_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .bind(actor)
        .fetch_optional(&mut *conn)
        .await?
        .is_some()),
    }
}

struct Message {
    id: Vec<u8>,
    received_at: DateTime<Utc>,
    root: Option<Vec<u8>>,
    /// A depth-1 reply broadcast to the channel: part of both timelines.
    broadcast: bool,
}

/// A channel event and its canonical ancestry. `kinds` bounds the lookup: an
/// anchor must be a kind that can be unread; a thread root need not be.
async fn message(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    id: &[u8],
    kinds: Option<&[i32]>,
) -> Result<Option<Message>> {
    let row = sqlx::query(
        "SELECT e.id, e.received_at, e.tags, tm.root_event_id,
             COALESCE(tm.depth = 1 AND tm.broadcast, false) AS broadcast
         FROM events e LEFT JOIN thread_metadata tm ON tm.community_id=e.community_id
             AND tm.event_created_at=e.created_at AND tm.event_id=e.id AND tm.channel_id=e.channel_id
         WHERE e.community_id=$1 AND e.channel_id=$2 AND e.id=$3
             AND ($4::int4[] IS NULL OR e.kind=ANY($4))
         LIMIT 1",
    ).bind(community.as_uuid()).bind(channel).bind(id).bind(kinds).fetch_optional(&mut *conn).await?;
    row.map(|row| {
        let received_at: DateTime<Utc> = row.try_get("received_at")?;
        let id: Vec<u8> = row.try_get("id")?;
        let root: Option<Vec<u8>> = row.try_get("root_event_id")?;
        let tags: serde_json::Value = row.try_get("tags")?;
        let tags: Vec<Vec<String>> = serde_json::from_value(tags)
            .map_err(|_| crate::DbError::InvalidData("invalid message tags".into()))?;
        let markers =
            buzz_core::nip10::parse_thread_markers_from_parts(tags.iter().map(Vec::as_slice));
        if markers.resolve().is_some() && root.is_none() {
            return Err(crate::DbError::InvalidData(
                "unresolved message ancestry".into(),
            ));
        }
        Ok(Message {
            id,
            received_at,
            root,
            broadcast: row.try_get("broadcast")?,
        })
    })
    .transpose()
}

pub(super) async fn valid_target(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    target: &ReadTarget,
) -> Result<Option<Vec<u8>>> {
    let root = match &target.root_id {
        Some(root) => match event_id(root) {
            Some(id) => id,
            None => return Ok(None),
        },
        None => Vec::new(),
    };
    if !access(conn, community, actor, target.channel_id).await? {
        return Ok(None);
    }
    if !root.is_empty() {
        // Deleted roots still own living replies, and so do roots of a kind
        // that is never unread itself (a diff). Validate actual ancestry, not
        // absence of metadata: a missing index row is not a top-level proof.
        let Some(msg) = message(conn, community, target.channel_id, &root, None).await? else {
            return Ok(None);
        };
        if msg.root.as_ref().is_some_and(|r| r != &msg.id) {
            return Ok(None);
        }
    }
    Ok(Some(root))
}

/// What an applied intent does to the context's row.
enum Change<'a> {
    Through(&'a Message),
    Unread,
    State(ReadState),
}

/// Write the person's row for a context, stamped with their counter. A read
/// position only advances: the anchor follows the greatest arrival, and an
/// equal arrival keeps the existing anchor.
async fn write(
    tx: &mut AdmittedTx,
    community: CommunityId,
    actor: &[u8],
    target: &ReadTarget,
    root: &[u8],
    seq: i64,
    change: Change<'_>,
) -> Result<()> {
    let (through, unread, state) = match change {
        Change::Through(msg) => (Some(msg), Some(false), None),
        Change::Unread => (None, Some(true), None),
        Change::State(state) => (None, None, Some(state.as_str())),
    };
    sqlx::query(
        "INSERT INTO personal_read_frontiers AS f
         (community_id, actor, channel_id, root_id, through_timestamp, through_message_id,
            marked_unread, state, seq)
         VALUES ($1,$2,$3,$4,$5,$6,COALESCE($7,false),COALESCE($8,'default'),$9)
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
         SET through_message_id=CASE
                WHEN EXCLUDED.through_timestamp > COALESCE(f.through_timestamp,'-infinity')
                THEN EXCLUDED.through_message_id ELSE f.through_message_id END,
            through_timestamp=GREATEST(f.through_timestamp, EXCLUDED.through_timestamp),
            marked_unread=COALESCE($7, f.marked_unread),
            state=COALESCE($8, f.state),
            seq=EXCLUDED.seq",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .bind(target.channel_id)
    .bind(root)
    .bind(through.map(|msg| msg.received_at))
    .bind(through.map(|msg| msg.id.as_slice()))
    .bind(unread)
    .bind(state)
    .bind(seq)
    .execute(tx.conn())
    .await?;
    Ok(())
}

pub(super) async fn apply(
    tx: &mut AdmittedTx,
    community: CommunityId,
    actor: &[u8],
    seq: i64,
    intent: &ReadIntent,
) -> Result<IntentOutcome> {
    let target = intent.target();
    if let ReadIntent::MarkThrough { message_id, .. } = intent {
        if event_id(message_id).is_none() {
            return Ok(IntentOutcome::Invalid);
        }
    }
    let Some(root) = valid_target(tx.conn(), community, actor, target).await? else {
        return Ok(IntentOutcome::Blocked);
    };
    match intent {
        ReadIntent::MarkThrough { message_id, .. } => {
            let id = event_id(message_id).unwrap_or_default();
            let Some(msg) = message(
                tx.conn(),
                community,
                target.channel_id,
                &id,
                Some(&ELIGIBLE_KINDS),
            )
            .await?
            else {
                return Ok(IntentOutcome::Blocked);
            };
            // A channel is marked through a message of its own timeline: a
            // top-level message or a broadcast reply. A thread through its
            // root or one of its replies.
            let is_reply = msg.root.as_ref().is_some_and(|r| r != &msg.id);
            if (root.is_empty() && is_reply && !msg.broadcast)
                || (!root.is_empty() && msg.id != root && msg.root.as_ref() != Some(&root))
            {
                return Ok(IntentOutcome::Blocked);
            }
            write(
                tx,
                community,
                actor,
                target,
                &root,
                seq,
                Change::Through(&msg),
            )
            .await?;
        }
        ReadIntent::MarkUnread { .. } => {
            write(tx, community, actor, target, &root, seq, Change::Unread).await?;
        }
        ReadIntent::SetState { state, .. } => {
            write(
                tx,
                community,
                actor,
                target,
                &root,
                seq,
                Change::State(*state),
            )
            .await?;
        }
    }
    Ok(IntentOutcome::Applied)
}

impl Db {
    /// Apply one independent intent. Blocked/invalid intents roll back *all*
    /// private account changes, the counter included. Retrying after an
    /// ambiguous commit is safe because only fixed operands are used.
    pub async fn apply_personal_read_intent(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        intent: &ReadIntent,
    ) -> Result<IntentOutcome> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            observability::WriterOperation::EventWrite,
        )
        .await?;
        deadlines(tx.conn()).await?;
        let actor = actor.to_bytes();
        let seq = lock_account(&mut tx, community, &actor).await?;
        let outcome = apply(&mut tx, community, &actor, seq, intent).await?;
        match outcome {
            IntentOutcome::Applied => tx.commit().await?,
            IntentOutcome::Blocked | IntentOutcome::Invalid => tx.rollback().await?,
        }
        Ok(outcome)
    }
}
