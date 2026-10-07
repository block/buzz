//! Sidebar read state from read positions. `unread` is an existence probe
//! that stops at the first qualifying row, and `mentions` walks only the
//! actor's mentions past the position, so a caught-up reader examines almost
//! nothing. Posting marks the author read at ingest, so no probe filters out
//! the actor's own messages.

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
const UNREAD_EVENT: &str = "e.created_at >= r.position-$7 AND e.received_at > r.position
    AND e.kind=ANY($6) AND e.deleted_at IS NULL";

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
        // up. A thread's is its row, which exists only for threads the actor
        // follows (see `membership`), and a whole-channel cut and joining
        // cover them all, so a follow that survived leaving does not bring
        // back replies from the absence.
        //
        // - unread: the first timeline message past the position
        //   (idx_events_community_channel_created);
        // - mentions: the actor's mentions in that scope that arrived past
        //   the position (idx_event_mentions_scope_received);
        // - read_through_id: the newest timeline message, by author time, that
        //   arrived at or before the position. Anything that arrived by then
        //   was authored no later than the position plus the skew;
        // - threads: followed roots whose last reply arrived after the
        //   thread position and that have an unread reply (an existence
        //   probe), then their mentions; `latest_id` is the last reply in
        //   display order (idx_thread_metadata_window), so no part of a
        //   thread row grows with its unread count.
        let sql = format!(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id, c.name, c.channel_type::text AS channel_type,
                    c.archived_at IS NOT NULL AS archived, cm.hidden_at IS NOT NULL AS hidden,
                    GREATEST(cf.through_timestamp, cm.joined_at, s.started) AS position,
                    GREATEST(cf.threads_through_timestamp, cm.joined_at, s.started) AS threads_floor
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
                EXISTS (SELECT 1 FROM events e {EVENT_THREAD_ROW}
                    WHERE e.community_id=$1 AND e.channel_id=r.id
                        AND {UNREAD_EVENT} AND {ON_TIMELINE}) AS unread,
                (SELECT count(*) FROM event_mentions m JOIN events e ON e.community_id=$1
                        AND e.created_at=m.event_created_at AND e.id=m.event_id
                    WHERE m.community_id=$1 AND m.pubkey_hex=$8 AND m.channel_id=r.id
                        AND m.root_id IS NULL AND m.received_at > r.position
                        AND e.channel_id=r.id AND {UNREAD_EVENT}) AS mentions,
                (SELECT encode(e.id,'hex') FROM events e {EVENT_THREAD_ROW}
                    WHERE e.community_id=$1 AND e.channel_id=r.id
                        AND e.created_at <= r.position+$7 AND e.received_at <= r.position
                        AND e.kind=ANY($6) AND e.deleted_at IS NULL AND {ON_TIMELINE}
                    ORDER BY e.created_at DESC, e.id LIMIT 1) AS read_through_id,
                COALESCE(threads.items, '[]') AS threads
             FROM roster r
             LEFT JOIN LATERAL (
                SELECT jsonb_agg(jsonb_build_object('root_id',encode(tf.root_id,'hex'),
                        'unread',true,'mentions',nm.mentions,'read_through_id',rt.id,
                        'latest_id',n.id) ORDER BY n.at DESC, tf.root_id) AS items
                FROM personal_read_frontiers tf
                JOIN thread_metadata root ON root.community_id=$1 AND root.channel_id=r.id
                    AND root.depth=0 AND root.event_id=tf.root_id
                CROSS JOIN LATERAL (SELECT GREATEST(tf.through_timestamp, r.threads_floor) AS position) p
                CROSS JOIN LATERAL (
                    SELECT encode(tm.event_id,'hex') AS id, tm.event_created_at AS at
                    FROM thread_metadata tm JOIN events e ON e.community_id=$1
                        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
                    WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                        AND tm.channel_id=r.id AND tm.event_id<>tf.root_id
                        AND e.kind=ANY($6) AND e.deleted_at IS NULL
                    ORDER BY tm.event_created_at DESC, tm.event_id LIMIT 1
                ) n
                CROSS JOIN LATERAL (
                    SELECT count(*) AS mentions
                    FROM event_mentions m JOIN events e ON e.community_id=$1
                        AND e.created_at=m.event_created_at AND e.id=m.event_id
                    WHERE m.community_id=$1 AND m.pubkey_hex=$8 AND m.channel_id=r.id
                        AND m.root_id=tf.root_id AND m.received_at > p.position
                        AND e.kind=ANY($6) AND e.deleted_at IS NULL
                ) nm
                LEFT JOIN LATERAL (
                    SELECT encode(tm.event_id,'hex') AS id
                    FROM thread_metadata tm JOIN events e ON e.community_id=$1
                        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
                    WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                        AND tm.channel_id=r.id AND tm.event_id<>tf.root_id
                        AND tm.event_created_at <= p.position+$7
                        AND e.received_at <= p.position AND e.kind=ANY($6) AND e.deleted_at IS NULL
                    ORDER BY tm.event_created_at DESC, tm.event_id LIMIT 1
                ) rt ON true
                WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=r.id
                    AND tf.root_id<>''::bytea AND tf.following
                    AND root.last_reply_received_at > p.position
                    AND EXISTS (SELECT 1 FROM thread_metadata tm JOIN events e
                        ON e.community_id=$1 AND e.created_at=tm.event_created_at
                            AND e.id=tm.event_id
                        WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                            AND tm.channel_id=r.id AND tm.event_id<>tf.root_id
                            AND tm.event_created_at >= p.position-$7
                            AND e.received_at > p.position AND e.kind=ANY($6)
                            AND e.deleted_at IS NULL AND NOT (tm.depth=1 AND tm.broadcast))
             ) threads ON true
             ORDER BY r.id"#
        );
        // Only compile-time constants are interpolated.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(actor.to_bytes().as_slice())
            .bind(after)
            .bind(only)
            .bind((limit + 1) as i64)
            .bind(ELIGIBLE_KINDS.as_slice())
            .bind(skew())
            .bind(actor.to_hex())
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            let threads: Vec<ThreadReadSummary> =
                serde_json::from_value(row.try_get("threads")?)
                    .map_err(|_| DbError::InvalidData("invalid thread summaries".into()))?;
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                name: row.try_get("name")?,
                channel_type: row.try_get("channel_type")?,
                archived: row.try_get("archived")?,
                hidden: row.try_get("hidden")?,
                unread: row.try_get("unread")?,
                mentions: row.try_get("mentions")?,
                read_through_id: row.try_get("read_through_id")?,
                threads,
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

pub(super) fn skew() -> chrono::Duration {
    chrono::Duration::seconds(i64::from(MAX_ARRIVAL_SKEW_SECONDS))
}
