//! Forward counting from read positions. A caught-up reader examines almost
//! nothing: each count reads only what arrived after its position, and stops
//! at [`UNREAD_CAP`].

use super::{model::*, writes};
use buzz_core::CommunityId;
use serde::Deserialize;
use sqlx::{Acquire, Row};
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// A message's place, given its `thread_metadata` row `tm`: on the timeline
/// (top-level, or a depth-1 reply broadcast to the channel, exactly as the
/// timeline query shows it), or else in its canonical thread.
const ON_TIMELINE: &str = "(tm.root_event_id IS NULL OR tm.root_event_id=e.id
    OR (tm.depth=1 AND tm.broadcast))";

/// Whether event `e` is directed at the actor (hex `$hex`) by its tags.
const DIRECTED_TAGS: &str = "EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) t(tag)
    WHERE (tag->>0='p' AND lower(tag->>1)=$hex) OR (tag->>0='broadcast' AND tag->>1='1'))";

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
        // Unread is subtraction: the channel's counter less the actor's
        // position (starting and joining write one, so a started member
        // without a row has read nothing), and each of the actor's threads' reply count less its
        // position. Nothing counts before the account starts. Attention needs
        // the messages themselves, so it walks forward from the position
        // through the numbered messages after it, only when something is
        // unread, and stops at the cap. A whole-channel mark through the
        // counter's latest message reads every counted message.
        let sql = format!(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id, c.name, c.channel_type::text AS channel_type,
                    c.archived_at IS NOT NULL AS archived, cm.hidden_at IS NOT NULL AS hidden,
                    s.started IS NOT NULL AS started,
                    COALESCE(k.timeline_seq, 0) AS timeline_seq,
                    COALESCE(cf.through_seq, 0) AS through_seq,
                    COALESCE(cf.through_channel_seq, 0) AS through_channel_seq,
                    COALESCE(cf.through_timestamp, '-infinity') AS position,
                    encode(k.latest_id, 'hex') AS latest_id
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                CROSS JOIN (SELECT (SELECT started_at FROM personal_read_accounts
                    WHERE community_id=$1 AND actor=$2) AS started) s
                LEFT JOIN personal_read_counters k ON k.community_id=cm.community_id
                    AND k.channel_id=c.id
                LEFT JOIN personal_read_frontiers cf ON cf.community_id=cm.community_id
                    AND cf.actor=cm.pubkey AND cf.channel_id=c.id AND cf.root_id=''::bytea
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($3::uuid IS NULL OR c.id>$3)
                    AND ($4::uuid[] IS NULL OR c.id=ANY($4))
                ORDER BY c.id LIMIT $5
             ), counted AS (
                SELECT r.*, CASE WHEN r.started
                    THEN GREATEST(r.timeline_seq - r.through_seq, 0) ELSE 0 END AS unread
                FROM roster r
             )
             SELECT r.id, r.name, r.channel_type, r.archived, r.hidden,
                r.latest_id AS latest_message_id, latest.at AS latest_message_at,
                LEAST(r.unread, $9)::int AS unread,
                CASE WHEN r.channel_type='dm' THEN LEAST(r.unread, $9)::int
                    ELSE COALESCE(attention.n, 0) END AS attention,
                COALESCE(threads.items, '[]') AS threads
             FROM counted r
             LEFT JOIN LATERAL (
                SELECT max(extract(epoch FROM created_at)::bigint) AS at
                FROM (SELECT created_at FROM events
                    WHERE community_id=$1 AND channel_id=r.id AND kind=ANY($6) AND deleted_at IS NULL
                    ORDER BY created_at DESC LIMIT $7) newest
             ) latest ON true
             LEFT JOIN LATERAL (
                SELECT count(*)::int AS n FROM (
                    SELECT 1 FROM events e
                    LEFT JOIN thread_metadata tm ON tm.community_id=$1
                        AND tm.event_created_at=e.created_at AND tm.event_id=e.id
                    WHERE r.unread > 0 AND r.channel_type<>'dm'
                        AND e.community_id=$1 AND e.channel_id=r.id
                        AND e.created_at >= r.position-$8 AND e.channel_seq > r.through_channel_seq
                        AND e.kind=ANY($6) AND e.pubkey<>$2 AND {ON_TIMELINE} AND {DIRECTED_TAGS}
                    ORDER BY e.created_at LIMIT $9
                ) directed
             ) attention ON true
             LEFT JOIN LATERAL (
                SELECT jsonb_agg(jsonb_build_object('root_id',encode(tf.root_id,'hex'),
                        'unread',LEAST(root.reply_seq - tf.through_seq, $9),
                        'latest_reply_id',encode(root.reply_last_id,'hex'),
                        'latest_reply_at',extract(epoch FROM last.event_created_at)::bigint)) AS items
                FROM personal_read_frontiers tf
                JOIN thread_metadata root ON root.community_id=$1 AND root.event_id=tf.root_id
                    AND root.depth=0
                CROSS JOIN LATERAL (SELECT event_created_at FROM thread_metadata
                    WHERE community_id=$1 AND event_id=root.reply_last_id LIMIT 1) last
                WHERE r.started AND tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=r.id
                    AND tf.root_id<>''::bytea AND root.reply_seq > tf.through_seq
             ) threads ON true
             ORDER BY r.id"#
        )
        .replace("$hex", "$10");
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
            .bind(i64::from(UNREAD_CAP))
            .bind(actor.to_hex())
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            let threads: Vec<ThreadRow> = serde_json::from_value(row.try_get("threads")?)
                .map_err(|_| DbError::InvalidData("invalid thread summaries".into()))?;
            let thread_unread: u32 = threads.iter().map(|t| t.unread).sum();
            let unread = row.try_get::<i32, _>("unread")? as u32;
            let attention = row.try_get::<i32, _>("attention")? as u32;
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                name: row.try_get("name")?,
                channel_type: row.try_get("channel_type")?,
                archived: row.try_get("archived")?,
                hidden: row.try_get("hidden")?,
                unread: clamp(unread + thread_unread),
                attention: clamp(attention + thread_unread),
                unread_thread_count: clamp(threads.len() as u32),
                latest_message_id: row.try_get("latest_message_id")?,
                latest_message_at: row.try_get("latest_message_at")?,
                threads: summarize(threads.into_iter().map(ThreadRow::into).collect()),
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

fn clamp(n: u32) -> u32 {
    n.min(UNREAD_CAP)
}

#[derive(Deserialize)]
struct ThreadRow {
    root_id: String,
    unread: u32,
    latest_reply_id: String,
    latest_reply_at: i64,
}

impl From<ThreadRow> for ThreadReadSummary {
    fn from(t: ThreadRow) -> Self {
        Self {
            root_id: t.root_id,
            unread: clamp(t.unread),
            latest_reply_id: t.latest_reply_id,
            latest_reply_at: t.latest_reply_at,
        }
    }
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
            unread: 1,
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
