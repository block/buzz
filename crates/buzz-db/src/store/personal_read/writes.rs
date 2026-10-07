use super::model::*;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection, Row};
use uuid::Uuid;

use crate::{observability, Db, Result};

pub(super) fn event_id(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 || value.bytes().any(|b| !b.is_ascii_hexdigit()) {
        return None;
    }
    hex::decode(value).ok()
}

pub(super) async fn deadlines(conn: &mut PgConnection) -> Result<()> {
    sqlx::query("SET LOCAL statement_timeout = '2000ms'")
        .execute(&mut *conn)
        .await?;
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Start the account if this is the actor's first read intent, then serialize
/// private frontier writes, never shared conversation rows. NO KEY UPDATE
/// leaves ingest's foreign-key checks (which create thread rows) unblocked.
pub(super) async fn lock_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor,started_at) VALUES ($1,$2,now())
        ON CONFLICT (community_id,actor) DO UPDATE SET started_at=now()
        WHERE personal_read_accounts.started_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "SELECT actor FROM personal_read_accounts WHERE community_id=$1 AND actor=$2
         FOR NO KEY UPDATE",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_one(conn)
    .await?;
    Ok(())
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
    /// Canonical thread root, when the message is a reply.
    thread: Option<Vec<u8>>,
    /// Shown on the channel timeline: not a reply, or a broadcast depth-1 reply.
    on_timeline: bool,
}

/// A channel event and its place. `kinds` bounds the lookup: an anchor must be
/// a kind that can be unread; a thread root need not be.
async fn message(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    id: &[u8],
    kinds: Option<&[i32]>,
) -> Result<Option<Message>> {
    let row = sqlx::query(
        "SELECT e.id, e.received_at, tm.root_event_id, tm.depth, tm.broadcast
         FROM events e LEFT JOIN thread_metadata tm ON tm.community_id=e.community_id
             AND tm.event_created_at=e.created_at AND tm.event_id=e.id AND tm.channel_id=e.channel_id
         WHERE e.community_id=$1 AND e.channel_id=$2 AND e.id=$3
             AND ($4::int4[] IS NULL OR e.kind=ANY($4))
         LIMIT 1",
    ).bind(community.as_uuid()).bind(channel).bind(id).bind(kinds).fetch_optional(&mut *conn).await?;
    row.map(|row| {
        let id: Vec<u8> = row.try_get("id")?;
        let root: Option<Vec<u8>> = row.try_get("root_event_id")?;
        let thread = root.filter(|root| root != &id);
        let broadcast_reply = row.try_get::<Option<i32>, _>("depth")? == Some(1)
            && row.try_get::<Option<bool>, _>("broadcast")? == Some(true);
        Ok(Message {
            on_timeline: thread.is_none() || broadcast_reply,
            received_at: row.try_get("received_at")?,
            thread,
            id,
        })
    })
    .transpose()
}

/// An eligible-kind message's arrival time, deleted or not, without tags.
async fn anchor_received_at(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    id: &[u8],
) -> Result<Option<DateTime<Utc>>> {
    Ok(sqlx::query_scalar(
        "SELECT received_at FROM events
         WHERE community_id=$1 AND channel_id=$2 AND id=$3 AND kind=ANY($4) LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(id)
    .bind(ELIGIBLE_KINDS.as_slice())
    .fetch_optional(conn)
    .await?)
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
        // that is never unread itself (a diff).
        let Some(msg) = message(conn, community, target.channel_id, &root, None).await? else {
            return Ok(None);
        };
        if msg.thread.is_some() {
            return Ok(None);
        }
    }
    Ok(Some(root))
}

pub(super) async fn apply(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    intent: &ReadIntent,
) -> Result<IntentOutcome> {
    match intent {
        ReadIntent::MarkThrough { target, message_id } => {
            let Some(id) = event_id(message_id) else {
                return Ok(IntentOutcome::Invalid);
            };
            let Some(root) = valid_target(conn, community, actor, target).await? else {
                return Ok(IntentOutcome::Blocked);
            };
            let Some(msg) = message(
                conn,
                community,
                target.channel_id,
                &id,
                Some(&ELIGIBLE_KINDS),
            )
            .await?
            else {
                return Ok(IntentOutcome::Blocked);
            };
            if root.is_empty() {
                if !msg.on_timeline {
                    return Ok(IntentOutcome::Blocked);
                }
                sqlx::query(
                    "INSERT INTO personal_read_frontiers
                     (community_id, actor, channel_id, root_id, through_timestamp)
                     VALUES ($1,$2,$3,''::bytea,$4)
                     ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
                     SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, $4)",
                )
                .bind(community.as_uuid())
                .bind(actor)
                .bind(target.channel_id)
                .bind(msg.received_at)
                .execute(&mut *conn)
                .await?;
            } else {
                if msg.id != root && msg.thread.as_ref() != Some(&root) {
                    return Ok(IntentOutcome::Blocked);
                }
                // Reading a thread never joins it: only the actor's threads
                // have a row to advance.
                sqlx::query(
                    "UPDATE personal_read_frontiers
                     SET through_timestamp=GREATEST(through_timestamp, $5)
                     WHERE community_id=$1 AND actor=$2 AND channel_id=$3 AND root_id=$4",
                )
                .bind(community.as_uuid())
                .bind(actor)
                .bind(target.channel_id)
                .bind(&root)
                .bind(msg.received_at)
                .execute(&mut *conn)
                .await?;
            }
        }
        ReadIntent::MarkChannelRead {
            channel_id,
            message_id,
        } => {
            let Some(id) = event_id(message_id) else {
                return Ok(IntentOutcome::Invalid);
            };
            if !access(conn, community, actor, *channel_id).await? {
                return Ok(IntentOutcome::Blocked);
            }
            // Only the anchor's arrival matters: ancestry cannot change which
            // messages a whole-channel cut covers.
            let Some(through) = anchor_received_at(conn, community, *channel_id, &id).await? else {
                return Ok(IntentOutcome::Blocked);
            };
            sqlx::query(
                "INSERT INTO personal_read_frontiers (community_id, actor, channel_id, root_id,
                    through_timestamp, threads_through_timestamp) VALUES ($1,$2,$3,''::bytea,$4,$4)
                 ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
                 SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, $4),
                    threads_through_timestamp=GREATEST(personal_read_frontiers.threads_through_timestamp, $4)",
            ).bind(community.as_uuid()).bind(actor).bind(channel_id).bind(through)
                .execute(&mut *conn).await?;
        }
    }
    Ok(IntentOutcome::Applied)
}

impl Db {
    /// Apply one independent intent. Blocked/invalid intents roll back *all*
    /// private account changes. Retrying after an
    /// ambiguous commit is safe because only fixed max operands are used.
    pub async fn apply_personal_read_intent(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        intent: &ReadIntent,
    ) -> Result<IntentOutcome> {
        let mut conn =
            observability::acquire_writer(&self.pool, observability::WriterOperation::EventWrite)
                .await?;
        let mut tx = conn.begin().await?;
        deadlines(&mut tx).await?;
        let actor = actor.to_bytes();
        lock_account(&mut tx, community, &actor).await?;
        let outcome = apply(&mut tx, community, &actor, intent).await?;
        match outcome {
            IntentOutcome::Applied => tx.commit().await?,
            IntentOutcome::Blocked | IntentOutcome::Invalid => tx.rollback().await?,
        }
        Ok(outcome)
    }
}
