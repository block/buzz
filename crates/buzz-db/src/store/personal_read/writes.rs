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

/// Serialize private frontier/import writes, never shared conversation rows.
pub(super) async fn lock_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor) VALUES ($1,$2)
        ON CONFLICT (community_id,actor) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "SELECT actor FROM personal_read_accounts WHERE community_id=$1 AND actor=$2 FOR UPDATE",
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
    timestamp: i64,
    root: Option<Vec<u8>>,
}

async fn message(
    conn: &mut PgConnection,
    community: CommunityId,
    channel: Uuid,
    id: &[u8],
) -> Result<Option<Message>> {
    let row = sqlx::query(
        "SELECT e.id, e.created_at, e.tags, tm.root_event_id
         FROM events e LEFT JOIN thread_metadata tm ON tm.community_id=e.community_id
             AND tm.event_created_at=e.created_at AND tm.event_id=e.id AND tm.channel_id=e.channel_id
         WHERE e.community_id=$1 AND e.channel_id=$2 AND e.id=$3
             AND e.kind=ANY($4)
         LIMIT 1",
    ).bind(community.as_uuid()).bind(channel).bind(id).bind(ELIGIBLE_KINDS.as_slice()).fetch_optional(&mut *conn).await?;
    row.map(|row| {
        let timestamp: DateTime<Utc> = row.try_get("created_at")?;
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
            timestamp: timestamp.timestamp(),
            root,
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
        // Deleted roots still own living replies. Validate actual ancestry,
        // not absence of metadata: a missing index row is not a top-level proof.
        let Some(msg) = message(conn, community, target.channel_id, &root).await? else {
            return Ok(None);
        };
        if msg.root.as_ref().is_some_and(|r| r != &msg.id) {
            return Ok(None);
        }
    }
    Ok(Some(root))
}

async fn frontier(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    target: &ReadTarget,
    root: &[u8],
    timestamp: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO personal_read_frontiers
         (community_id, actor, channel_id, root_id, through_timestamp) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
         SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp, EXCLUDED.through_timestamp)",
    ).bind(community.as_uuid()).bind(actor).bind(target.channel_id).bind(root).bind(timestamp)
        .execute(&mut *conn).await?;
    Ok(())
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
            let Some(msg) = message(conn, community, target.channel_id, &id).await? else {
                return Ok(IntentOutcome::Blocked);
            };
            let is_reply = msg.root.as_ref().is_some_and(|r| r != &msg.id);
            if (root.is_empty() && is_reply)
                || (!root.is_empty() && msg.id != root && msg.root.as_ref() != Some(&root))
            {
                return Ok(IntentOutcome::Blocked);
            }
            frontier(conn, community, actor, target, &root, msg.timestamp).await?;
        }
        ReadIntent::LegacyPrefix {
            target,
            through_timestamp,
        } => {
            let latest_allowed: i64 = sqlx::query_scalar(
                "SELECT floor(extract(epoch FROM clock_timestamp()))::bigint + 900",
            )
            .fetch_one(&mut *conn)
            .await?;
            if *through_timestamp < 0 || *through_timestamp > latest_allowed {
                return Ok(IntentOutcome::Invalid);
            }
            let Some(root) = valid_target(conn, community, actor, target).await? else {
                return Ok(IntentOutcome::Blocked);
            };
            // Import fixed prefixes only; receipt-time horizon does not alter
            // the original author-time operand.
            frontier(conn, community, actor, target, &root, *through_timestamp).await?;
        }
        ReadIntent::CompleteImport => {
            sqlx::query("UPDATE personal_read_accounts SET imported_at=COALESCE(imported_at,clock_timestamp())
                WHERE community_id=$1 AND actor=$2")
                .bind(community.as_uuid()).bind(actor).execute(&mut *conn).await?;
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
