//! Read state that ingest writes. A thread frontier row exists only for a
//! thread its actor follows; only this module creates one.
use super::model::ELIGIBLE_KINDS;
use crate::{AdmittedTx, Result};
use chrono::{DateTime, Utc};
use nostr::Event;
use uuid::Uuid;

/// Where a newly stored channel message sits.
#[derive(Clone, Copy)]
pub(crate) enum Place<'a> {
    /// A top-level timeline message.
    TopLevel,
    /// A reply in the thread rooted at `root`.
    Reply { root: &'a [u8] },
}

/// Record what a newly stored eligible message changes, in its transaction:
/// - its author has read through it: the timeline for a top-level message,
///   otherwise the thread, which the author then follows;
/// - each channel member it mentions follows its thread (for a top-level
///   message, the thread rooted at it);
/// - a reply makes the root's author, and in a DM every member, follow too.
///
/// Everyone but the author starts just before the message, so it is unread
/// to them. A follower keeps an existing position.
pub(crate) async fn record_message(
    tx: &mut AdmittedTx,
    event: &Event,
    channel: Uuid,
    place: Place<'_>,
    received_at: DateTime<Utc>,
) -> Result<()> {
    if !ELIGIBLE_KINDS.contains(&i32::from(event.kind.as_u16())) {
        return Ok(());
    }
    let community = tx.community();
    let id = event.id.as_bytes().as_slice();
    let mentioned: Vec<Vec<u8>> = event
        .tags
        .public_keys()
        .map(|key| key.to_bytes().to_vec())
        .collect();
    let (read, thread, reply) = match place {
        Place::TopLevel => (&[][..], id, false),
        Place::Reply { root } => (root, root, true),
    };
    // Accounts, then frontiers, each in key order, so concurrent messages that
    // involve the same people cannot deadlock: the frontier insert joins the
    // finished account insert. Foreign-key checks run at the end of the
    // statement, after the account rows exist.
    sqlx::query(
        "WITH involved (actor, root_id, through, message) AS (
            SELECT $4::bytea, $5::bytea, $6::timestamptz, $3::bytea
            UNION ALL
            SELECT cm.pubkey, $7, $6-interval '1 microsecond', NULL FROM channel_members cm
            WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND cm.pubkey=ANY($8) AND cm.pubkey<>$4
            UNION ALL
            SELECT e.pubkey, $7, $6-interval '1 microsecond', NULL FROM events e
            WHERE $9 AND e.community_id=$1 AND e.channel_id=$2 AND e.id=$7 AND e.pubkey<>$4
            UNION ALL
            SELECT cm.pubkey, $7, $6-interval '1 microsecond', NULL
            FROM channel_members cm JOIN channels c
                ON c.community_id=cm.community_id AND c.id=cm.channel_id
            WHERE $9 AND cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND c.channel_type='dm' AND cm.pubkey<>$4
         ), rows AS (
            -- An existing follower keeps their position, so is not touched.
            -- Existence probes are LIMIT 1 laterals: always one index lookup.
            SELECT DISTINCT ON (i.actor, i.root_id) i.* FROM involved i
            LEFT JOIN LATERAL (
                SELECT true AS found FROM personal_read_frontiers f WHERE f.community_id=$1
                    AND f.actor=i.actor AND f.channel_id=$2 AND f.root_id=i.root_id LIMIT 1
            ) f ON true
            WHERE i.message IS NOT NULL OR f.found IS NULL
            ORDER BY i.actor, i.root_id
         ), accounts AS (
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT $1, r.actor FROM (SELECT DISTINCT actor FROM rows) r
            LEFT JOIN LATERAL (
                SELECT true AS found FROM personal_read_accounts a
                WHERE a.community_id=$1 AND a.actor=r.actor LIMIT 1
            ) a ON true
            WHERE a.found IS NULL ORDER BY r.actor
            ON CONFLICT DO NOTHING RETURNING 1
         )
         INSERT INTO personal_read_frontiers
            (community_id, actor, channel_id, root_id, through_timestamp, through_message_id)
         SELECT $1, actor, $2, root_id, through, message
         FROM rows CROSS JOIN (SELECT count(*) FROM accounts) written
         ORDER BY actor, root_id
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
         SET through_timestamp=GREATEST(personal_read_frontiers.through_timestamp,
                excluded.through_timestamp),
            through_message_id=CASE
                WHEN excluded.through_timestamp > personal_read_frontiers.through_timestamp
                THEN excluded.through_message_id ELSE personal_read_frontiers.through_message_id END
         WHERE excluded.through_message_id IS NOT NULL",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(id)
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(read)
    .bind(received_at)
    .bind(thread)
    .bind(&mentioned)
    .bind(reply)
    .execute(tx.conn())
    .await?;
    Ok(())
}
