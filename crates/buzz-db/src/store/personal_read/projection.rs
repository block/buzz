//! Sidebar read state, counted forward from each read position. A caught-up
//! reader's work is a few index probes per channel and per followed thread;
//! a reader who is behind walks only what arrived after their position, and
//! counts stop at `MAX_UNREAD_COUNT`.

use super::{model::*, writes};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection, Row};
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// Event `e` is eligible: a conversation kind, not deleted.
const ELIGIBLE: &str = "e.kind=ANY($5) AND e.deleted_at IS NULL";

/// Event `e`, with its `thread_metadata` row `tm`, is on the channel timeline:
/// not a reply, and not a NIP-10 reply whose ancestry was never recorded
/// (neither counted nor a timeline anchor, since a mark cannot validate it).
/// Parity-tested against the shared NIP-10 parser.
const TIMELINE: &str = r#"(tm.root_event_id IS NULL OR tm.root_event_id=e.id)
    AND NOT (tm.root_event_id IS NULL AND jsonb_typeof(e.tags)='array'
        AND EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) t(tag)
            WHERE jsonb_typeof(tag)='array' AND tag->>0='e' AND tag->>3='reply'
                AND (tag->>1) COLLATE "C" ~ '^[0123456789abcdefABCDEF]{64}$'))"#;

/// What the budgeted timeline walks carry out of their `LIMIT`.
const COLUMNS: &str =
    "e.id, e.pubkey, e.created_at, e.received_at, e.kind, e.deleted_at, e.tags, e.channel_id";

const EVENT_THREAD_ROW: &str = "LEFT JOIN thread_metadata tm ON tm.community_id=$1
    AND tm.channel_id=e.channel_id AND tm.event_created_at=e.created_at AND tm.event_id=e.id";

/// Replies `tm` in followed thread `tf`, with their events `e`.
const THREAD_REPLIES: &str = "thread_metadata tm JOIN events e ON e.community_id=$1
        AND e.created_at=tm.event_created_at AND e.id=tm.event_id
    WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id AND tm.channel_id=r.id
        AND tm.event_id<>tf.root_id";

impl Db {
    /// Read a bounded joined roster from the writer in one read-only snapshot.
    /// Callers must recheck admission/resource access before releasing this data.
    pub async fn personal_read_sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        limit: usize,
        after: Option<Uuid>,
    ) -> Result<SidebarPage> {
        if !(1..=MAX_CHANNELS).contains(&limit) {
            return Err(DbError::InvalidData("invalid sidebar limit".into()));
        }
        self.sidebar(community, actor, retention_seconds, limit, after, None)
            .await
    }

    /// Refresh specific joined channels in one snapshot. A requested channel
    /// absent from the result was not a joined, nondeleted channel at that cut.
    pub async fn personal_read_sidebar_channels(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        channels: &[Uuid],
    ) -> Result<SidebarPage> {
        let unique: std::collections::HashSet<_> = channels.iter().collect();
        if !(1..=MAX_CHANNELS).contains(&channels.len()) || unique.len() != channels.len() {
            return Err(DbError::InvalidData("invalid sidebar channels".into()));
        }
        self.sidebar(
            community,
            actor,
            retention_seconds,
            channels.len(),
            None,
            Some(channels),
        )
        .await
    }

    async fn sidebar(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
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
        // The horizon and every read position share one read-only cut.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        // A position is its frontier floored at the account's start (infinity
        // before the actor's first intent, so nothing counts). Positions are
        // arrival times and the indexes are by author time, which the relay
        // accepts within the skew of arrival, so a walk forward from a
        // position starts the skew before it (and never before the horizon).
        //
        // Timeline, per channel:
        // - latest_id: the newest timeline message by author time within the
        //   scan budget, then the last to arrive among the newest budget of
        //   events authored up to twice the skew before it, which holds the
        //   last arrival unless that many arrived out of order;
        // - unread: latest is someone else's, inside the horizon, and arrived
        //   after the position (an own post moves the position past it);
        // - mentions: those of them that tag the actor (event_mentions), or
        //   every one in a DM.
        // Threads: each followed thread whose newest reply could have arrived
        // after its position (one index-only probe of
        // idx_thread_metadata_window), then its last arrival the same way as
        // latest_id, and its unread replies by others.
        let sql = format!(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id, c.channel_type='dm' AS dm,
                    GREATEST(cf.through_timestamp, COALESCE($7::timestamptz, 'infinity')) AS position,
                    encode(cf.through_message_id,'hex') AS read_through_id
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                    AND cf.channel_id=c.id AND cf.root_id=''::bytea
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($3::uuid IS NULL OR c.id>$3)
                    AND ($4::uuid[] IS NULL OR c.id=ANY($4))
                ORDER BY c.id LIMIT $6
             )
             SELECT r.id, r.read_through_id, encode(l.id,'hex') AS latest_id, u.unread,
                COALESCE(m.mentions, 0) AS mentions, COALESCE(th.items, '[]') AS threads
             FROM roster r
             CROSS JOIN LATERAL (SELECT GREATEST(r.position-$8, $9) AS walk_from) w
             LEFT JOIN LATERAL (
                SELECT e.created_at FROM (
                    SELECT {COLUMNS} FROM events e WHERE e.community_id=$1 AND e.channel_id=r.id
                    ORDER BY e.created_at DESC, e.id DESC LIMIT $10
                ) e {EVENT_THREAD_ROW}
                WHERE {ELIGIBLE} AND {TIMELINE}
                ORDER BY e.created_at DESC, e.id DESC LIMIT 1
             ) n ON true
             LEFT JOIN LATERAL (
                SELECT e.id, e.pubkey, e.created_at, e.received_at FROM (
                    SELECT {COLUMNS} FROM events e WHERE e.community_id=$1 AND e.channel_id=r.id
                        AND e.created_at BETWEEN n.created_at-2*$8 AND n.created_at
                    ORDER BY e.created_at DESC, e.id DESC LIMIT $10
                ) e {EVENT_THREAD_ROW}
                WHERE {ELIGIBLE} AND {TIMELINE}
                ORDER BY e.received_at DESC, e.id LIMIT 1
             ) l ON true
             -- The last arrival is unread exactly when the timeline is: the
             -- actor's own posts read through themselves.
             CROSS JOIN LATERAL (
                SELECT COALESCE(l.received_at > r.position AND l.pubkey<>$2
                    AND l.created_at >= $9, false) AS unread
             ) u
             LEFT JOIN LATERAL (
                SELECT count(*) AS mentions FROM (
                    SELECT 1 FROM (
                        SELECT {COLUMNS} FROM events e WHERE r.dm AND e.community_id=$1
                            AND e.channel_id=r.id AND e.created_at >= w.walk_from
                        ORDER BY e.created_at, e.id LIMIT $10
                    ) e {EVENT_THREAD_ROW}
                    WHERE e.received_at > r.position AND e.pubkey<>$2 AND {ELIGIBLE} AND {TIMELINE}
                    UNION ALL
                    SELECT 1 FROM event_mentions em JOIN events e ON e.community_id=$1
                        AND e.created_at=em.event_created_at AND e.id=em.event_id {EVENT_THREAD_ROW}
                    WHERE NOT r.dm AND em.community_id=$1 AND em.pubkey_hex=$11
                        AND em.channel_id=r.id AND em.event_created_at >= w.walk_from
                        AND e.channel_id=r.id AND e.received_at > r.position
                        AND e.pubkey<>$2 AND {ELIGIBLE} AND {TIMELINE}
                    LIMIT $12
                ) x WHERE u.unread
             ) m ON true
             LEFT JOIN LATERAL (
                SELECT jsonb_agg(jsonb_build_object('root_id',encode(t.root_id,'hex'),
                        'unread',true,'mentions',t.n,'read_through_id',t.read_through_id,
                        'latest_id',t.latest_id) ORDER BY t.latest_at DESC, t.root_id) AS items
                FROM (
                    SELECT tf.root_id, encode(tf.through_message_id,'hex') AS read_through_id,
                        encode(tl.id,'hex') AS latest_id, tl.created_at AS latest_at, tc.n
                    FROM personal_read_frontiers tf
                    CROSS JOIN LATERAL (SELECT GREATEST(tf.through_timestamp, COALESCE($7, 'infinity')) AS position) p
                    CROSS JOIN LATERAL (
                        SELECT tm.event_created_at AS created_at FROM thread_metadata tm
                        WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                        ORDER BY tm.event_created_at DESC, tm.event_id LIMIT 1
                    ) tq
                    CROSS JOIN LATERAL (
                        -- Eligibility is a per-row filter, not a join, so the
                        -- planner walks the window index newest-first and stops
                        -- at the first eligible reply instead of sorting them all.
                        SELECT tm.event_created_at AS created_at FROM thread_metadata tm
                        WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id
                            AND tm.channel_id=r.id AND tm.event_id<>tf.root_id
                            AND (SELECT {ELIGIBLE} FROM events e WHERE e.community_id=$1
                                AND e.created_at=tm.event_created_at AND e.id=tm.event_id)
                        ORDER BY tm.event_created_at DESC, tm.event_id LIMIT 1
                    ) tn
                    CROSS JOIN LATERAL (
                        SELECT e.id, e.created_at, e.received_at FROM (
                            SELECT e.id, e.created_at, e.received_at FROM {THREAD_REPLIES}
                                AND tm.event_created_at BETWEEN tn.created_at-2*$8 AND tn.created_at
                                AND {ELIGIBLE}
                            ORDER BY tm.event_created_at DESC, tm.event_id LIMIT $10
                        ) e
                        ORDER BY e.received_at DESC, e.id LIMIT 1
                    ) tl
                    CROSS JOIN LATERAL (
                        SELECT count(*) AS n FROM (
                            SELECT 1 FROM {THREAD_REPLIES}
                                AND tm.event_created_at >= GREATEST(p.position-$8, $9)
                                AND e.received_at > p.position AND e.pubkey<>$2 AND {ELIGIBLE}
                            LIMIT $12
                        ) x
                    ) tc
                    WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=r.id
                        AND tf.root_id<>''::bytea
                        AND tq.created_at+$8 > p.position AND tl.received_at > p.position
                        AND tc.n > 0
                    ORDER BY tl.created_at DESC, tf.root_id LIMIT $13
                ) t
             ) th ON true
             ORDER BY r.id"#
        );
        let cutoff = DateTime::from_timestamp_millis(account.cutoff_ms)
            .ok_or_else(|| DbError::InvalidData("invalid unread cutoff".into()))?;
        // Only compile-time constants are interpolated.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(actor_bytes.as_slice())
            .bind(after)
            .bind(only)
            .bind(ELIGIBLE_KINDS.as_slice())
            .bind((limit + 1) as i64)
            .bind(account.started_at)
            .bind(skew())
            .bind(cutoff)
            .bind(MAX_TIMELINE_SCAN as i64)
            .bind(actor.to_hex())
            .bind(MAX_UNREAD_COUNT as i64)
            .bind(MAX_THREAD_SUMMARIES as i64)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            let threads: Vec<ThreadReadSummary> =
                serde_json::from_value(row.try_get("threads")?)
                    .map_err(|_| DbError::InvalidData("invalid thread summaries".into()))?;
            let mentions: i64 = row.try_get("mentions")?;
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                unread: row.try_get("unread")?,
                mentions: mentions as u32,
                read_through_id: row.try_get("read_through_id")?,
                latest_id: row.try_get("latest_id")?,
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

fn skew() -> chrono::Duration {
    chrono::Duration::seconds(i64::from(MAX_ARRIVAL_SKEW_SECONDS))
}

/// Read-time horizon and the account's start, in the caller's snapshot.
/// Frontier state is not discarded on expiry.
pub(super) async fn read_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    retention_seconds: u32,
) -> Result<ReadAccount> {
    let row = sqlx::query(
        "SELECT date_trunc('milliseconds',transaction_timestamp()-make_interval(secs=>$1::double precision)) AS cutoff,
            (SELECT started_at FROM personal_read_accounts WHERE community_id=$2 AND actor=$3) AS started_at",
    )
    .bind(f64::from(retention_seconds))
    .bind(community.as_uuid())
    .bind(actor)
    .fetch_one(conn)
    .await?;
    let cutoff: DateTime<Utc> = row.try_get("cutoff")?;
    Ok(ReadAccount {
        cutoff_ms: cutoff.timestamp_millis(),
        started_at: row.try_get("started_at")?,
    })
}
