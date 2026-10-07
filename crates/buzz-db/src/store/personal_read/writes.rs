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
    created_at: DateTime<Utc>,
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
        "SELECT e.id, e.created_at, e.received_at, tm.root_event_id, tm.depth, tm.broadcast
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
            created_at: row.try_get("created_at")?,
            received_at: row.try_get("received_at")?,
            thread,
            id,
        })
    })
    .transpose()
}

/// An eligible-kind message's author and arrival times, deleted or not,
/// without tags.
async fn anchor_times(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    id: &[u8],
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>> {
    Ok(sqlx::query_as(
        "SELECT created_at, received_at FROM events
         WHERE community_id=$1 AND channel_id=$2 AND id=$3 AND kind=ANY($4) LIMIT 1",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(id)
    .bind(ELIGIBLE_KINDS.as_slice())
    .fetch_optional(conn)
    .await?)
}

/// Where a mark through an anchor leaves the frontier: the last arrival among
/// the messages displayed (by author time, then ID) at or before the anchor
/// in `scope`. Clients name only display-order IDs, so a late arrival shown
/// above the anchor is read with it instead of leaving a badge no mark can
/// clear. Anything that arrived after the anchor was authored no earlier
/// than the skew before the anchor's arrival, which bounds the range.
/// `scope` is a thread root, empty for the channel timeline, or None for
/// the whole channel.
async fn display_through(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    anchor: (&[u8], DateTime<Utc>, DateTime<Utc>),
    scope: Option<&[u8]>,
) -> Result<DateTime<Utc>> {
    let (id, created_at, received_at) = anchor;
    let through: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(e.received_at) FROM events e
         LEFT JOIN thread_metadata tm ON tm.community_id=$1
            AND tm.event_created_at=e.created_at AND tm.event_id=e.id
         WHERE e.community_id=$1 AND e.channel_id=$2 AND e.kind=ANY($3)
            AND e.created_at >= $4::timestamptz-$7 AND e.created_at <= $5
            AND (e.created_at, e.id) <= ($5, $6) AND e.received_at > $4
            AND CASE WHEN $8::bytea IS NULL THEN true
                WHEN $8=''::bytea THEN tm.root_event_id IS NULL OR tm.root_event_id=e.id
                    OR (tm.depth=1 AND tm.broadcast)
                ELSE e.id=$8 OR tm.root_event_id=$8 END",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(ELIGIBLE_KINDS.as_slice())
    .bind(received_at)
    .bind(created_at)
    .bind(id)
    .bind(super::projection::skew())
    .bind(scope)
    .fetch_one(conn)
    .await?;
    Ok(through.map_or(received_at, |later| later.max(received_at)))
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
                let through = display_through(
                    conn,
                    community,
                    target.channel_id,
                    (msg.id.as_slice(), msg.created_at, msg.received_at),
                    Some(root.as_slice()),
                )
                .await?;
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
                .bind(through)
                .execute(&mut *conn)
                .await?;
            } else {
                if msg.id != root && msg.thread.as_ref() != Some(&root) {
                    return Ok(IntentOutcome::Blocked);
                }
                let through = display_through(
                    conn,
                    community,
                    target.channel_id,
                    (msg.id.as_slice(), msg.created_at, msg.received_at),
                    Some(root.as_slice()),
                )
                .await?;
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
                .bind(through)
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
            // Ancestry cannot change which messages a whole-channel cut
            // covers: everything displayed at or before the anchor.
            let Some((created_at, received_at)) =
                anchor_times(conn, community, *channel_id, &id).await?
            else {
                return Ok(IntentOutcome::Blocked);
            };
            let through = display_through(
                conn,
                community,
                *channel_id,
                (&id, created_at, received_at),
                None,
            )
            .await?;
            sqlx::query(
                "INSERT INTO personal_read_frontiers (community_id, actor, channel_id, root_id,
                    through_timestamp, threads_through_timestamp) VALUES ($1,$2,$3,''::bytea,$4,$4)
                 ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
                 SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, $4),
                    threads_through_timestamp=GREATEST(personal_read_frontiers.threads_through_timestamp, $4)",
            ).bind(community.as_uuid()).bind(actor).bind(channel_id).bind(through)
                .execute(&mut *conn).await?;
        }
        ReadIntent::Follow { target } | ReadIntent::Unfollow { target } => {
            if target.root_id.is_none() {
                return Ok(IntentOutcome::Invalid);
            }
            let Some(root) = valid_target(conn, community, actor, target).await? else {
                return Ok(IntentOutcome::Blocked);
            };
            let following = matches!(intent, ReadIntent::Follow { .. });
            // A new row starts caught up: at the thread's last reply, or the
            // root itself before any reply. Following again also catches up,
            // so replies that arrived while unfollowed stay muted. Unfollow
            // keeps the row and its position, so later replies don't
            // re-follow.
            sqlx::query(
                "INSERT INTO personal_read_frontiers
                    (community_id, actor, channel_id, root_id, through_timestamp, following)
                 SELECT $1, $2, $3, $4, COALESCE(
                    (SELECT max(last_reply_received_at) FROM thread_metadata
                     WHERE community_id=$1 AND channel_id=$3 AND event_id=$4 AND depth=0),
                    (SELECT max(received_at) FROM events
                     WHERE community_id=$1 AND channel_id=$3 AND id=$4)), $5
                 ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
                 SET following=excluded.following,
                    through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                        excluded.through_timestamp)
                 WHERE excluded.following AND NOT personal_read_frontiers.following
                    OR NOT excluded.following",
            )
            .bind(community.as_uuid())
            .bind(actor)
            .bind(target.channel_id)
            .bind(&root)
            .bind(following)
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
