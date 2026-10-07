//! Thread membership. A thread frontier row exists exactly for the actor's
//! threads, and only ingest creates one: read intents move existing rows.
use super::model::ELIGIBLE_KINDS;
use crate::Result;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Record who a newly stored reply makes a thread member: its author, the
/// root's author, members it mentions, and in a DM (at most nine members)
/// every member. The author's row starts at the reply, which it has read;
/// everyone else's starts just before it, so the reply is unread. Existing
/// rows are left alone.
pub(crate) async fn record_reply(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
    channel: Uuid,
    root: &[u8],
    received_at: DateTime<Utc>,
) -> Result<()> {
    if !ELIGIBLE_KINDS.contains(&i32::from(event.kind.as_u16())) {
        return Ok(());
    }
    let mentioned: Vec<Vec<u8>> = event
        .tags
        .public_keys()
        .map(|key| key.to_bytes().to_vec())
        .collect();
    // Both inserts' fence and foreign-key checks run at the end of the
    // statement, after the account rows exist.
    sqlx::query(
        "WITH joined AS (
            SELECT $4::bytea AS actor, $6::timestamptz AS through
            UNION ALL
            SELECT pubkey, $6-interval '1 microsecond' FROM events
            WHERE community_id=$1 AND channel_id=$2 AND id=$3
            UNION ALL
            SELECT cm.pubkey, $6-interval '1 microsecond'
            FROM channel_members cm JOIN channels c
                ON c.community_id=cm.community_id AND c.id=cm.channel_id
            WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND (cm.pubkey=ANY($5) OR c.channel_type='dm')
         ), members AS (
            SELECT DISTINCT ON (actor) actor, through FROM joined ORDER BY actor, through DESC
         ), accounts AS (
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT $1, actor FROM members ON CONFLICT DO NOTHING
         )
         INSERT INTO personal_read_frontiers
            (community_id, actor, channel_id, root_id, through_timestamp)
         SELECT $1, actor, $2, $3, through FROM members
         ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(root)
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(&mentioned)
    .bind(received_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
