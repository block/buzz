//! Unread presence from read positions. Every fact is an existence probe
//! that stops at the first qualifying row, so a caught-up reader examines
//! almost nothing and nothing is counted.

use super::{model::*, writes};
use buzz_core::CommunityId;
use sqlx::{Acquire, Row};
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// A message's place, given its `thread_metadata` row `tm`: on the timeline
/// (top-level, or a depth-1 reply broadcast to the channel, exactly as the
/// timeline query shows it), or else in its canonical thread.
const ON_TIMELINE: &str = "(tm.root_event_id IS NULL OR tm.root_event_id=e.id
    OR (tm.depth=1 AND tm.broadcast))";

/// Event `e` is eligible and unread against `r.position`, and `tm` is its
/// thread row. Author time ranges from the position less the arrival skew.
const UNREAD_EVENT: &str = "e.created_at >= r.position-$8 AND e.received_at > r.position
    AND e.kind=ANY($6) AND e.pubkey<>$2 AND e.deleted_at IS NULL";

const EVENT_THREAD_ROW: &str = "LEFT JOIN thread_metadata tm ON tm.community_id=$1
    AND tm.event_created_at=e.created_at AND tm.event_id=e.id";

impl Db {
    /// Read a bounded joined roster from the writer in one read-only snapshot.
    /// Callers must recheck admission/resource access before releasing this data.
    pub async fn personal_read_sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        limit: usize,
        after: Option<Uuid>,
    ) -> Result<SidebarPage> {
        if !(1..=MAX_CHANNELS).contains(&limit) {
            return Err(DbError::InvalidData("invalid sidebar limit".into()));
        }
        self.sidebar(community, actor, limit, after, None).await
    }

    /// Refresh specific joined channels in one snapshot. A requested channel
    /// absent from the result was not a joined, nondeleted channel at that cut.
    pub async fn personal_read_sidebar_channels(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        channels: &[Uuid],
    ) -> Result<SidebarPage> {
        let unique: std::collections::HashSet<_> = channels.iter().collect();
        if !(1..=MAX_CHANNELS).contains(&channels.len()) || unique.len() != channels.len() {
            return Err(DbError::InvalidData("invalid sidebar channels".into()));
        }
        self.sidebar(community, actor, channels.len(), None, Some(channels))
            .await
    }

    async fn sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        limit: usize,
        after: Option<Uuid>,
        only: Option<&[Uuid]>,
    ) -> Result<SidebarPage> {
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
        // Every position is floored at the account's start (infinity before
        // the actor's first intent, so nothing counts). A channel's is its
        // frontier, never earlier than joining: a new member starts caught
        // up. A thread row exists only for the actor's threads (see
        // `membership`), and a whole-channel cut covers them all.
        //
        // Each fact is the first qualifying row of one index range:
        // - ordinary: the channel's timeline after the position, skipping
        //   directed messages (idx_events_community_channel_created);
        // - mention: the actor's own mentions (idx_event_mentions_pubkey_created);
        // - broadcast: the channel's depth-1 replies (idx_thread_metadata_channel_depth);
        // - direct: any unread timeline message in a DM;
        // - threads: roots in the channel whose last reply arrived after the
        //   actor's position, then their unread replies (anchor and display time).
        // The latest message is the last arrival among the newest messages and
        // every unread message found, so marking through it reads what was found.
        let sql = format!(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id, c.name, c.channel_type::text AS channel_type,
                    c.archived_at IS NOT NULL AS archived, cm.hidden_at IS NOT NULL AS hidden,
                    GREATEST(cf.through_timestamp, cm.joined_at, s.started) AS position,
                    GREATEST(cf.threads_through_timestamp, s.started) AS threads_floor
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                CROSS JOIN (SELECT COALESCE((SELECT started_at FROM personal_read_accounts
                    WHERE community_id=$1 AND actor=$2), 'infinity') AS started) s
                LEFT JOIN personal_read_frontiers cf ON cf.community_id=cm.community_id
                    AND cf.actor=cm.pubkey AND cf.channel_id=c.id AND cf.root_id=''::bytea
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($3::uuid IS NULL OR c.id>$3)
                    AND ($4::uuid[] IS NULL OR c.id=ANY($4))
                ORDER BY c.id LIMIT $5
             )
             SELECT r.id, r.name, r.channel_type, r.archived, r.hidden,
                (SELECT v.id FROM (VALUES (latest.arrival, latest.id), (ordinary.arrival, ordinary.id),
                        (directed.arrival, directed.id), (threads.arrival, threads.id)) v(arrival, id)
                    WHERE v.id IS NOT NULL ORDER BY v.arrival DESC, v.id LIMIT 1) AS latest_message_id,
                latest.at AS latest_message_at,
                ordinary.id IS NOT NULL AS unread,
                directed.id IS NOT NULL OR threads.id IS NOT NULL AS attention,
                COALESCE(threads.items, '[]') AS threads
             FROM roster r
             LEFT JOIN LATERAL (
                SELECT (array_agg(encode(id,'hex') ORDER BY received_at DESC,id))[1] AS id,
                    max(received_at) AS arrival, max(extract(epoch FROM created_at)::bigint) AS at
                FROM (SELECT id,created_at,received_at FROM events
                    WHERE community_id=$1 AND channel_id=r.id AND kind=ANY($6) AND deleted_at IS NULL
                    ORDER BY created_at DESC LIMIT $7) newest
             ) latest ON true
             LEFT JOIN LATERAL (
                SELECT encode(e.id,'hex') AS id, e.received_at AS arrival
                FROM events e {EVENT_THREAD_ROW}
                WHERE r.channel_type<>'dm' AND e.community_id=$1 AND e.channel_id=r.id
                    AND {UNREAD_EVENT} AND {ON_TIMELINE}
                    AND NOT (tm.depth=1 AND tm.broadcast) IS TRUE
                    AND NOT EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) t(tag)
                        WHERE tag->>0='p' AND lower(tag->>1)=$hex)
                ORDER BY e.created_at LIMIT 1
             ) ordinary ON true
             LEFT JOIN LATERAL (
                SELECT id, arrival FROM (
                    (SELECT encode(e.id,'hex') AS id, e.received_at AS arrival
                    FROM events e {EVENT_THREAD_ROW}
                    WHERE r.channel_type='dm' AND e.community_id=$1 AND e.channel_id=r.id
                        AND {UNREAD_EVENT} AND {ON_TIMELINE}
                    ORDER BY e.created_at LIMIT 1)
                    UNION ALL
                    (SELECT encode(e.id,'hex'), e.received_at
                    FROM event_mentions m JOIN events e ON e.community_id=$1
                        AND e.created_at=m.event_created_at AND e.id=m.event_id
                    {EVENT_THREAD_ROW}
                    WHERE r.channel_type<>'dm' AND m.community_id=$1 AND m.pubkey_hex=$hex
                        AND m.event_created_at >= r.position-$8 AND m.channel_id=r.id
                        AND e.channel_id=r.id AND {UNREAD_EVENT} AND {ON_TIMELINE}
                    LIMIT 1)
                    UNION ALL
                    (SELECT encode(e.id,'hex'), e.received_at
                    FROM thread_metadata tm JOIN events e ON e.community_id=$1
                        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
                    WHERE r.channel_type<>'dm' AND tm.community_id=$1 AND tm.channel_id=r.id
                        AND tm.depth=1 AND tm.broadcast AND tm.event_created_at >= r.position-$8
                        AND {UNREAD_EVENT}
                    LIMIT 1)
                ) found ORDER BY arrival DESC LIMIT 1
             ) directed ON true
             LEFT JOIN LATERAL (
                SELECT jsonb_agg(jsonb_build_object('root_id',encode(tf.root_id,'hex'),
                        'latest_reply_id',n.id,'latest_reply_at',n.at)) AS items,
                    (array_agg(n.id ORDER BY n.arrival DESC,n.id))[1] AS id, max(n.arrival) AS arrival
                FROM personal_read_frontiers tf
                JOIN thread_metadata root ON root.community_id=$1 AND root.channel_id=r.id
                    AND root.depth=0 AND root.event_id=tf.root_id
                CROSS JOIN LATERAL (SELECT GREATEST(tf.through_timestamp, r.threads_floor) AS position) p
                CROSS JOIN LATERAL (
                    SELECT (array_agg(encode(e.id,'hex') ORDER BY e.received_at DESC,e.id))[1] AS id,
                        max(extract(epoch FROM e.created_at)::bigint) AS at,
                        max(e.received_at) AS arrival
                    FROM thread_metadata tm JOIN events e ON e.community_id=$1
                        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
                    WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                        AND tm.channel_id=r.id AND tm.event_id<>tf.root_id
                        AND tm.event_created_at >= p.position-$8
                        AND e.received_at > p.position AND e.kind=ANY($6) AND e.pubkey<>$2
                        AND e.deleted_at IS NULL AND NOT (tm.depth=1 AND tm.broadcast)
                ) n
                WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=r.id
                    AND tf.root_id<>''::bytea AND root.last_reply_received_at > p.position
                    AND n.id IS NOT NULL
             ) threads ON true
             ORDER BY r.id"#
        )
        .replace("$hex", "$9");
        // Only compile-time constants are interpolated.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(actor.to_bytes().as_slice())
            .bind(after)
            .bind(only)
            .bind((limit + 1) as i64)
            .bind(ELIGIBLE_KINDS.as_slice())
            .bind(LATEST_PROBE as i64)
            .bind(skew())
            .bind(actor.to_hex())
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            let threads: Vec<ThreadReadSummary> = serde_json::from_value(row.try_get("threads")?)
                .map_err(|_| DbError::InvalidData("invalid thread summaries".into()))?;
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                name: row.try_get("name")?,
                channel_type: row.try_get("channel_type")?,
                archived: row.try_get("archived")?,
                hidden: row.try_get("hidden")?,
                unread: row.try_get("unread")?,
                attention: row.try_get("attention")?,
                latest_message_id: row.try_get("latest_message_id")?,
                latest_message_at: row.try_get("latest_message_at")?,
                threads: summarize(threads),
            });
        }
        let next_cursor = if has_more {
            channels.last().map(|c| c.channel_id)
        } else {
            None
        };
        Ok(SidebarPage {
            channels,
            next_cursor,
        })
    }
}

/// Newest messages, by author time, that join the counted ones as latest candidates.
pub(super) const LATEST_PROBE: usize = 32;

pub(super) fn skew() -> chrono::Duration {
    chrono::Duration::seconds(i64::from(MAX_ARRIVAL_SKEW_SECONDS))
}

/// Order newest unread reply first (root ID breaks ties) and cap the list.
pub(super) fn summarize(mut items: Vec<ThreadReadSummary>) -> Vec<ThreadReadSummary> {
    items.sort_unstable_by(|a, b| {
        b.latest_reply_at
            .cmp(&a.latest_reply_at)
            .then_with(|| a.root_id.cmp(&b.root_id))
    });
    items.truncate(MAX_THREAD_SUMMARIES);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(root: u8, at: i64) -> ThreadReadSummary {
        ThreadReadSummary {
            root_id: hex::encode([root; 32]),
            latest_reply_id: hex::encode([root; 32]),
            latest_reply_at: at,
        }
    }

    #[test]
    fn thread_summaries_order_newest_first_break_ties_by_root_and_cap_at_five() {
        // Literal contract boundaries deliberately do not derive from the constant.
        for count in [0_u8, 1, 4, 5, 6, 7] {
            // Descending roots with pairwise-equal times exercise the tie-break.
            let items: Vec<_> = (0..count)
                .rev()
                .map(|i| item(i, i64::from(i / 2)))
                .collect();
            let summaries = summarize(items);
            let roots: Vec<_> = summaries.iter().map(|t| t.root_id.clone()).collect();
            let mut expected: Vec<_> = (0..count).map(|i| (i64::from(i / 2), i)).collect();
            expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let expected: Vec<_> = expected
                .into_iter()
                .take(5)
                .map(|(_, i)| hex::encode([i; 32]))
                .collect();
            assert_eq!(roots, expected, "count {count}");
        }
    }
}
