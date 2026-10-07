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
/// Starting reads every joined channel and existing thread through its
/// current count.
pub(super) async fn lock_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
) -> Result<()> {
    let started = sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor,started_at) VALUES ($1,$2,now())
        ON CONFLICT (community_id,actor) DO UPDATE SET started_at=now()
        WHERE personal_read_accounts.started_at IS NULL RETURNING actor",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_optional(&mut *conn)
    .await?
    .is_some();
    sqlx::query(
        "SELECT actor FROM personal_read_accounts WHERE community_id=$1 AND actor=$2
         FOR NO KEY UPDATE",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_one(&mut *conn)
    .await?;
    if started {
        sqlx::query(
            "INSERT INTO personal_read_frontiers (community_id, actor, channel_id,
                through_seq, through_channel_seq, through_timestamp)
             SELECT $1, $2, cm.channel_id, COALESCE(k.timeline_seq,0), COALESCE(k.channel_seq,0), now()
             FROM channel_members cm LEFT JOIN personal_read_counters k
                ON k.community_id=cm.community_id AND k.channel_id=cm.channel_id
             WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
             ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
                through_seq=EXCLUDED.through_seq, through_channel_seq=EXCLUDED.through_channel_seq,
                through_timestamp=EXCLUDED.through_timestamp",
        )
        .bind(community.as_uuid())
        .bind(actor)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE personal_read_frontiers tf SET through_seq=root.reply_seq,
                through_channel_seq=COALESCE(root.reply_channel_seq, tf.through_channel_seq),
                through_timestamp=now()
             FROM thread_metadata root
             WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.root_id<>''::bytea
                AND root.community_id=$1 AND root.event_id=tf.root_id AND root.depth=0",
        )
        .bind(community.as_uuid())
        .bind(actor)
        .execute(&mut *conn)
        .await?;
    }
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
    /// Numbers, absent for messages stored before numbering.
    timeline_seq: Option<i64>,
    channel_seq: Option<i64>,
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
        "SELECT e.id, e.received_at, e.timeline_seq, e.channel_seq,
             tm.root_event_id, tm.depth, tm.broadcast
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
            timeline_seq: row.try_get("timeline_seq")?,
            channel_seq: row.try_get("channel_seq")?,
            thread,
            id,
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

/// A thread's count through channel cut `cut`: its counted replies numbered
/// at or before it. Equal to the count ingest assigned, since both follow the
/// channel's numbering. The root's latest reply answers the usual case.
const THREAD_SEQ_AT: &str = "CASE WHEN root.reply_channel_seq IS NULL
        OR root.reply_channel_seq <= $cut THEN root.reply_seq
    ELSE (SELECT count(*) FROM thread_metadata tm JOIN events e ON e.community_id=tm.community_id
        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
        WHERE tm.community_id=root.community_id AND tm.root_event_id=root.event_id
            AND tm.event_id<>root.event_id AND NOT (tm.depth=1 AND tm.broadcast)
            AND e.kind=ANY($kinds) AND e.channel_seq <= $cut) END";

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
            let (Some(timeline_seq), Some(channel_seq)) = (msg.timeline_seq, msg.channel_seq)
            else {
                // Stored before numbering: already read.
                return Ok(IntentOutcome::Applied);
            };
            if root.is_empty() {
                if !msg.on_timeline {
                    return Ok(IntentOutcome::Blocked);
                }
                sqlx::query(
                    "INSERT INTO personal_read_frontiers (community_id, actor, channel_id,
                        through_seq, through_channel_seq, through_timestamp)
                     VALUES ($1,$2,$3,$4,$5,$6)
                     ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
                        through_seq=GREATEST(personal_read_frontiers.through_seq, $4),
                        through_channel_seq=GREATEST(personal_read_frontiers.through_channel_seq, $5),
                        through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, $6)",
                )
                .bind(community.as_uuid())
                .bind(actor)
                .bind(target.channel_id)
                .bind(timeline_seq)
                .bind(channel_seq)
                .bind(msg.received_at)
                .execute(&mut *conn)
                .await?;
            } else {
                if msg.id != root && msg.thread.as_ref() != Some(&root) {
                    return Ok(IntentOutcome::Blocked);
                }
                // Reading a thread never joins it: only the actor's threads
                // have a row to advance.
                let sql = format!(
                    "UPDATE personal_read_frontiers tf SET
                        through_seq=GREATEST(tf.through_seq, {THREAD_SEQ_AT}),
                        through_channel_seq=GREATEST(tf.through_channel_seq, $5),
                        through_timestamp=GREATEST(tf.through_timestamp, $6)
                     FROM thread_metadata root
                     WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=$3 AND tf.root_id=$4
                        AND root.community_id=$1 AND root.event_id=$4 AND root.depth=0"
                )
                .replace("$cut", "$5")
                .replace("$kinds", "$7");
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(community.as_uuid())
                    .bind(actor)
                    .bind(target.channel_id)
                    .bind(&root)
                    .bind(channel_seq)
                    .bind(msg.received_at)
                    .bind(ELIGIBLE_KINDS.as_slice())
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
            // Only the anchor's numbers matter: ancestry cannot change which
            // messages a whole-channel cut covers.
            let Some(msg) =
                message(conn, community, *channel_id, &id, Some(&ELIGIBLE_KINDS)).await?
            else {
                return Ok(IntentOutcome::Blocked);
            };
            let (Some(timeline_seq), Some(cut)) = (msg.timeline_seq, msg.channel_seq) else {
                return Ok(IntentOutcome::Applied);
            };
            sqlx::query(
                "INSERT INTO personal_read_frontiers (community_id, actor, channel_id,
                    through_seq, through_channel_seq, through_timestamp)
                 VALUES ($1,$2,$3,$4,$5,$6)
                 ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
                    through_seq=GREATEST(personal_read_frontiers.through_seq, $4),
                    through_channel_seq=GREATEST(personal_read_frontiers.through_channel_seq, $5),
                    through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, $6)",
            )
            .bind(community.as_uuid())
            .bind(actor)
            .bind(channel_id)
            .bind(timeline_seq)
            .bind(cut)
            .bind(msg.received_at)
            .execute(&mut *conn)
            .await?;
            // The cut covers every one of the actor's threads in the channel.
            let sql = format!(
                "UPDATE personal_read_frontiers tf SET
                    through_seq=GREATEST(tf.through_seq, {THREAD_SEQ_AT}),
                    through_channel_seq=GREATEST(tf.through_channel_seq, $4),
                    through_timestamp=GREATEST(tf.through_timestamp, $5)
                 FROM thread_metadata root
                 WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=$3
                    AND tf.root_id<>''::bytea AND root.community_id=$1
                    AND root.event_id=tf.root_id AND root.depth=0"
            )
            .replace("$cut", "$4")
            .replace("$kinds", "$6");
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(community.as_uuid())
                .bind(actor)
                .bind(channel_id)
                .bind(cut)
                .bind(msg.received_at)
                .bind(ELIGIBLE_KINDS.as_slice())
                .execute(&mut *conn)
                .await?;
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
