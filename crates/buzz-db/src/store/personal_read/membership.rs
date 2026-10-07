//! Read state written by ingest: posting marks the author read, and replies
//! and mentions follow threads. A thread frontier row exists for each thread
//! the actor follows or has unfollowed (`following=false`, so unfollow
//! sticks); only ingest and follow intents create one.
use super::model::ELIGIBLE_KINDS;
use crate::Result;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Where a newly stored channel message sits.
#[derive(Clone, Copy)]
pub(crate) enum Place<'a> {
    /// A top-level timeline message.
    TopLevel,
    /// A reply in `root`'s thread; `broadcast` when it also shows on the
    /// timeline (a depth-1 reply broadcast to the channel).
    Reply { root: &'a [u8], broadcast: bool },
}

/// Record what a newly stored message changes:
/// - its author has read through it, in each context it shows in, and a
///   reply (re-)follows the author;
/// - members it mentions follow the thread it is in, or the thread rooted at
///   it;
/// - a reply also makes the root's author, and in a DM (at most nine
///   members) every member, follow the thread.
///
/// A new mention re-follows an unfollowed thread, catching up to just before
/// the mention: chatter from while unfollowed stays muted, but the mention
/// shows. A mentioned member already following keeps their position. Anyone
/// else the reply makes follow keeps an existing row as it is, so an
/// unfollow sticks for ordinary replies.
///
/// Everyone but the author starts just before the message, so it is unread.
pub(crate) async fn record_message(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
    channel: Uuid,
    place: Place<'_>,
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
    let (root, reply, on_timeline) = match place {
        Place::TopLevel => (event.id.as_bytes().as_slice(), false, true),
        Place::Reply { root, broadcast } => (root, true, broadcast),
    };
    // Rank 0 is the replying author, who follows (again) and has read; 1 a
    // mentioned member, who follows again if they had unfollowed; 2 anyone
    // else the reply makes follow, whose existing rows are left alone. Every insert's fence and foreign-key
    // checks run at the end of the statement, after the account rows exist.
    sqlx::query(
        "WITH joined AS (
            SELECT $4::bytea AS actor, $6::timestamptz AS through, 0 AS rank WHERE $8
            UNION ALL
            SELECT cm.pubkey, $6-interval '1 microsecond', 1 FROM channel_members cm
            WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND cm.pubkey=ANY($5) AND cm.pubkey<>$4
            UNION ALL
            SELECT pubkey, $6-interval '1 microsecond', 2 FROM events
            WHERE $8 AND community_id=$1 AND channel_id=$2 AND id=$3
            UNION ALL
            SELECT cm.pubkey, $6-interval '1 microsecond', 2
            FROM channel_members cm JOIN channels c
                ON c.community_id=cm.community_id AND c.id=cm.channel_id
            WHERE $8 AND cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND c.channel_type='dm'
         ), members AS (
            SELECT DISTINCT ON (actor) actor, through, rank FROM joined ORDER BY actor, rank
         ), accounts AS (
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT $1, actor FROM members UNION SELECT $1, $4
            ON CONFLICT DO NOTHING
         ), timeline AS (
            INSERT INTO personal_read_frontiers
                (community_id, actor, channel_id, root_id, through_timestamp)
            SELECT $1, $4, $2, ''::bytea, $6 WHERE $7
            ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
            SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                excluded.through_timestamp)
         ), replied AS (
            INSERT INTO personal_read_frontiers
                (community_id, actor, channel_id, root_id, through_timestamp)
            SELECT $1, actor, $2, $3, through FROM members WHERE rank=0
            ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
            SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                excluded.through_timestamp), following=true
         ), mentioned AS (
            INSERT INTO personal_read_frontiers
                (community_id, actor, channel_id, root_id, through_timestamp)
            SELECT $1, actor, $2, $3, through FROM members WHERE rank=1
            ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
            SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                excluded.through_timestamp), following=true
            WHERE NOT personal_read_frontiers.following
         )
         INSERT INTO personal_read_frontiers
            (community_id, actor, channel_id, root_id, through_timestamp)
         SELECT $1, actor, $2, $3, through FROM members WHERE rank=2
         ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(root)
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(&mentioned)
    .bind(received_at)
    .bind(on_timeline)
    .bind(reply)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
