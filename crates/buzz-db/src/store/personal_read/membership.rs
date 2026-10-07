//! Ingest's share of read state: numbering each eligible message under its
//! channel's counter, posting marking the author's position, and thread
//! membership. A thread frontier row exists exactly for the actor's threads,
//! and only ingest creates one: read intents move existing rows.
use super::model::ELIGIBLE_KINDS;
use crate::Result;
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{PgConnection, Postgres, Row, Transaction};
use uuid::Uuid;

/// The numbers a message takes if it is stored.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Numbers {
    /// The channel's timeline count once stored: its own number on the timeline.
    pub timeline: i64,
    /// Its number among every eligible message in the channel.
    pub channel: i64,
    /// Shown on the timeline: top-level, or a broadcast depth-1 reply.
    pub on_timeline: bool,
}

/// Lock the channel's counter until commit and return the numbers an eligible
/// message would take. The lock is what makes numbers follow commit order.
pub(crate) async fn reserve(
    conn: &mut PgConnection,
    community: CommunityId,
    event: &Event,
    channel: Option<Uuid>,
    on_timeline: bool,
) -> Result<Option<Numbers>> {
    let Some(channel) = channel else {
        return Ok(None);
    };
    if !ELIGIBLE_KINDS.contains(&i32::from(event.kind.as_u16())) {
        return Ok(None);
    }
    let row = sqlx::query(
        "INSERT INTO personal_read_counters (community_id, channel_id) VALUES ($1, $2)
         ON CONFLICT (community_id, channel_id)
            DO UPDATE SET channel_seq = personal_read_counters.channel_seq
         RETURNING timeline_seq, channel_seq",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Some(Numbers {
        timeline: row.try_get::<i64, _>("timeline_seq")? + i64::from(on_timeline),
        channel: row.try_get::<i64, _>("channel_seq")? + 1,
        on_timeline,
    }))
}

/// Advance the counter for a stored message. Posting reads the author's
/// timeline through it once the author has started.
pub(crate) async fn commit(
    conn: &mut PgConnection,
    community: CommunityId,
    event: &Event,
    channel: Uuid,
    numbers: Numbers,
    received_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "UPDATE personal_read_counters SET timeline_seq=$3, channel_seq=$4, latest_id=$5
         WHERE community_id=$1 AND channel_id=$2",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(numbers.timeline)
    .bind(numbers.channel)
    .bind(event.id.as_bytes().as_slice())
    .execute(&mut *conn)
    .await?;
    if numbers.on_timeline {
        sqlx::query(
            "INSERT INTO personal_read_frontiers (community_id, actor, channel_id,
                through_seq, through_channel_seq, through_timestamp)
             SELECT $1, $2, $3, $4, $5, $6 FROM personal_read_accounts
             WHERE community_id=$1 AND actor=$2 AND started_at IS NOT NULL
             ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
                through_seq=GREATEST(personal_read_frontiers.through_seq, EXCLUDED.through_seq),
                through_channel_seq=GREATEST(personal_read_frontiers.through_channel_seq,
                    EXCLUDED.through_channel_seq),
                through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                    EXCLUDED.through_timestamp)",
        )
        .bind(community.as_uuid())
        .bind(event.pubkey.to_bytes().as_slice())
        .bind(channel)
        .bind(numbers.timeline)
        .bind(numbers.channel)
        .bind(received_at)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Record who a newly stored reply makes a thread member: its author, the
/// root's author, members it mentions, and in a DM (at most nine members)
/// every member. `thread_seq` is the thread's count once the reply is stored.
/// The author reads through the reply (posting marks read, also on an
/// existing row); everyone else's new row starts just before it, so the reply
/// is unread. Other existing rows are left alone.
pub(crate) async fn record_reply(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    event: &Event,
    channel: Uuid,
    root: &[u8],
    numbers: Numbers,
    thread_seq: i64,
    counts_in_thread: bool,
    received_at: DateTime<Utc>,
) -> Result<()> {
    let mentioned: Vec<Vec<u8>> = event
        .tags
        .public_keys()
        .map(|key| key.to_bytes().to_vec())
        .collect();
    let before = thread_seq - i64::from(counts_in_thread);
    // Both inserts' fence and foreign-key checks run at the end of the
    // statement, after the account rows exist.
    sqlx::query(
        "WITH joined AS (
            SELECT $4::bytea AS actor, $7::bigint AS seq, $8::bigint AS cseq, $6::timestamptz AS at
            UNION ALL
            SELECT pubkey, $9, $8-1, $6-interval '1 microsecond' FROM events
            WHERE community_id=$1 AND channel_id=$2 AND id=$3
            UNION ALL
            SELECT cm.pubkey, $9, $8-1, $6-interval '1 microsecond'
            FROM channel_members cm JOIN channels c
                ON c.community_id=cm.community_id AND c.id=cm.channel_id
            WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND (cm.pubkey=ANY($5) OR c.channel_type='dm')
         ), members AS (
            SELECT DISTINCT ON (actor) actor, seq, cseq, at FROM joined ORDER BY actor, cseq DESC
         ), accounts AS (
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT $1, actor FROM members ON CONFLICT DO NOTHING
         )
         INSERT INTO personal_read_frontiers (community_id, actor, channel_id, root_id,
            through_seq, through_channel_seq, through_timestamp)
         SELECT $1, actor, $2, $3, seq, cseq, at FROM members
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE SET
            through_seq=GREATEST(personal_read_frontiers.through_seq, EXCLUDED.through_seq),
            through_channel_seq=GREATEST(personal_read_frontiers.through_channel_seq,
                EXCLUDED.through_channel_seq),
            through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                EXCLUDED.through_timestamp)
         WHERE personal_read_frontiers.actor=$4",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(root)
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(&mentioned)
    .bind(received_at)
    .bind(thread_seq)
    .bind(numbers.channel)
    .bind(before)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
