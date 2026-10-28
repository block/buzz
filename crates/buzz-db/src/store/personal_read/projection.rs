//! Sidebar and threads read state, counted forward from each read position.
//! Each channel timeline and each followed thread is one index probe for its
//! last arrival; counts walk only what arrived after the position and stop at
//! `MAX_UNREAD_COUNT`.

use super::{model::*, writes};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{Acquire, PgConnection, Row};
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// The last eligible reply `tl` to arrive in thread `tf` (idx_thread_metadata_read_arrival).
/// Deletion is a per-row filter, not a join, so the walk stops at the first
/// living reply instead of sorting them all.
const LATEST_REPLY: &str = "SELECT tm.event_id AS id, tm.received_at, e.pubkey, e.created_at
    FROM (SELECT tm.event_id, tm.event_created_at, tm.received_at FROM thread_metadata tm
        WHERE tm.community_id=$1 AND tm.root_event_id=tf.root_id AND tm.received_at IS NOT NULL
            AND tm.channel_id=tf.channel_id
            AND (SELECT e.deleted_at IS NULL FROM events e WHERE e.community_id=$1
                AND e.created_at=tm.event_created_at AND e.id=tm.event_id)
        ORDER BY tm.received_at DESC, tm.event_id LIMIT 1) tm
    JOIN events e ON e.community_id=$1 AND e.created_at=tm.event_created_at AND e.id=tm.event_id";

/// Unread replies in thread `t` at position `t.position` that are directed at
/// the actor: every one in a DM (`t.dm`), otherwise those that tag the actor.
/// Nothing is walked unless `t.unread`. Root first: the thread's unread
/// replies (idx_thread_metadata_read_arrival), each probed against the
/// mention key, never the actor's mentions across the channel.
pub(super) const THREAD_MENTIONS: &str = "SELECT count(*) AS n FROM (
        SELECT 1 FROM thread_metadata tm JOIN events e ON e.community_id=$1
            AND e.created_at=tm.event_created_at AND e.id=tm.event_id
        WHERE t.unread AND t.dm AND tm.community_id=$1 AND tm.root_event_id=t.root_id
            AND tm.received_at > t.position AND tm.channel_id=t.channel_id
            AND e.deleted_at IS NULL AND e.pubkey<>$2
        UNION ALL
        SELECT 1 FROM (
            SELECT tm.event_id, tm.event_created_at FROM thread_metadata tm
            WHERE t.unread AND NOT t.dm AND tm.community_id=$1 AND tm.root_event_id=t.root_id
                AND tm.channel_id=t.channel_id AND tm.received_at > t.position
            OFFSET 0
        ) tm JOIN event_mentions em ON em.community_id=$1 AND em.pubkey_hex=$5
            AND em.event_created_at=tm.event_created_at AND em.event_id=tm.event_id
        JOIN events e ON e.community_id=$1 AND e.created_at=tm.event_created_at AND e.id=tm.event_id
        WHERE em.channel_id=t.channel_id AND em.event_created_at >= t.walk_from-$6
            AND e.deleted_at IS NULL AND e.pubkey<>$2
        LIMIT $7
    ) x";

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
        // The dormancy cutoff and every read position share one read-only cut.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        // A position is its frontier floored at the account's start (infinity
        // before the actor's first intent, so nothing counts).
        //
        // Timeline, per channel:
        // - latest_id: the last timeline message to arrive (idx_events_read_timeline);
        // - unread: latest is someone else's and arrived after the position
        //   (an own post moves the position past it);
        // - mentions: unread timeline messages that tag the actor, found by
        //   author time (event_mentions) from the skew before the position,
        //   or every one in a DM.
        // Threads: each live follow (explicit, or involved since the cutoff)
        // whose last reply is unread the same way, then its mentions for the
        // listed ones only. A dormant follow is filtered before its probe.
        let legacy_types = crate::store::channel::legacy_channel_type_sql("c.channel_type");
        let sql = format!(
            r#"WITH roster AS MATERIALIZED (
                SELECT c.id, c.channel_type='dm' AS dm,
                    GREATEST(cf.through_timestamp, COALESCE($3::timestamptz, 'infinity'), $4) AS position,
                    -- Mention walks start from the unfloored position: the floor
                    -- bounds arrival, and author time may lag arrival.
                    GREATEST(cf.through_timestamp, COALESCE($3::timestamptz, 'infinity')) AS walk_from,
                    encode(cf.through_message_id,'hex') AS read_through_id
                FROM channel_members cm JOIN channels c
                    ON c.community_id=cm.community_id AND c.id=cm.channel_id
                LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                    AND cf.channel_id=c.id AND cf.root_id=''::bytea
                WHERE cm.community_id=$1 AND cm.pubkey=$2 AND cm.removed_at IS NULL
                    AND c.deleted_at IS NULL AND ($8::uuid IS NULL OR c.id>$8)
                    AND ($9::uuid[] IS NULL OR c.id=ANY($9))
                    -- Other channel types only when the request names them.
                    AND ($9::uuid[] IS NOT NULL OR {legacy_types})
                ORDER BY c.id LIMIT $10
             )
             SELECT r.id, r.read_through_id, encode(l.id,'hex') AS latest_id, u.unread,
                COALESCE(m.mentions, 0) AS mentions, COALESCE(th.items, '[]') AS threads,
                COALESCE(th.followed, '[]') AS followed
             FROM roster r
             LEFT JOIN LATERAL (
                SELECT e.id, e.pubkey, e.created_at, e.received_at FROM events e
                WHERE e.community_id=$1 AND e.channel_id=r.id AND e.read_timeline
                    AND e.deleted_at IS NULL
                ORDER BY e.received_at DESC, e.id LIMIT 1
             ) l ON true
             CROSS JOIN LATERAL (
                SELECT COALESCE(l.received_at > r.position AND l.pubkey<>$2, false) AS unread
             ) u
             LEFT JOIN LATERAL (
                SELECT count(*) AS mentions FROM (
                    SELECT 1 FROM events e WHERE r.dm AND e.community_id=$1
                        AND e.channel_id=r.id AND e.read_timeline AND e.deleted_at IS NULL
                        AND e.received_at > r.position AND e.pubkey<>$2
                    UNION ALL
                    SELECT 1 FROM event_mentions em JOIN events e ON e.community_id=$1
                        AND e.created_at=em.event_created_at AND e.id=em.event_id
                    WHERE NOT r.dm AND em.community_id=$1 AND em.pubkey_hex=$5
                        AND em.channel_id=r.id AND em.event_created_at >= r.walk_from-$6
                        AND e.read_timeline AND e.deleted_at IS NULL
                        AND e.received_at > r.position AND e.pubkey<>$2
                    LIMIT $7
                ) x WHERE u.unread
             ) m ON true
             LEFT JOIN LATERAL (
                -- Every live follow is probed once; the unread rows and the
                -- followed list both come from that one probe.
                WITH f AS MATERIALIZED (
                    SELECT tf.root_id, tf.through_message_id, p.position, p.walk_from,
                        tl.id AS latest_id, tl.received_at AS latest_at, tl.pubkey AS latest_pubkey
                    FROM personal_read_frontiers tf
                    CROSS JOIN LATERAL (SELECT GREATEST(tf.through_timestamp,
                        COALESCE($3, 'infinity'), $4) AS position,
                        GREATEST(tf.through_timestamp, COALESCE($3, 'infinity')) AS walk_from) p
                    LEFT JOIN LATERAL ({LATEST_REPLY}) tl ON true
                    WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.channel_id=r.id
                        AND tf.root_id<>''::bytea AND NOT tf.unfollowed
                        AND (tf.explicit OR tf.active_at >= $4)
                )
                SELECT (SELECT jsonb_agg(jsonb_build_object('root_id',encode(t.root_id,'hex'),
                        'unread',true,'mentions',tc.n,'read_through_id',t.read_through_id,
                        'latest_id',encode(t.latest_id,'hex'))
                        ORDER BY t.latest_at DESC, t.root_id)
                    FROM (
                        SELECT f.root_id, r.id AS channel_id, r.dm, f.position, f.walk_from,
                            true AS unread, encode(f.through_message_id,'hex') AS read_through_id,
                            f.latest_id, f.latest_at
                        FROM f WHERE f.latest_at > f.position AND f.latest_pubkey<>$2
                        ORDER BY f.latest_at DESC, f.root_id LIMIT $11+1
                    ) t CROSS JOIN LATERAL ({THREAD_MENTIONS}) tc) AS items,
                    (SELECT jsonb_agg(encode(x.root_id,'hex')
                        ORDER BY x.latest_at DESC NULLS LAST, x.root_id)
                    FROM (
                        SELECT f.root_id, f.latest_at FROM f
                        ORDER BY f.latest_at DESC NULLS LAST, f.root_id LIMIT $12+1
                    ) x) AS followed
             ) th ON true
             ORDER BY r.id"#
        );
        let cutoff = DateTime::from_timestamp_millis(account.cutoff_ms)
            .ok_or_else(|| DbError::InvalidData("invalid dormancy cutoff".into()))?;
        // Only compile-time constants are interpolated.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(actor_bytes.as_slice())
            .bind(account.started_at)
            .bind(cutoff)
            .bind(actor.to_hex())
            .bind(skew())
            .bind(MAX_UNREAD_COUNT as i64)
            .bind(after)
            .bind(only)
            .bind((limit + 1) as i64)
            .bind(MAX_THREAD_SUMMARIES as i64)
            .bind(MAX_FOLLOWED_THREADS as i64)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut channels = Vec::with_capacity(limit);
        for row in rows.into_iter().take(limit) {
            let mut threads: Vec<ThreadReadSummary> =
                serde_json::from_value(row.try_get("threads")?)
                    .map_err(|_| DbError::InvalidData("invalid thread summaries".into()))?;
            // One row past the cap is probed only to report the omission.
            let threads_truncated = threads.len() > MAX_THREAD_SUMMARIES;
            threads.truncate(MAX_THREAD_SUMMARIES);
            let mut followed_threads: Vec<String> =
                serde_json::from_value(row.try_get("followed")?)
                    .map_err(|_| DbError::InvalidData("invalid followed threads".into()))?;
            let followed_threads_truncated = followed_threads.len() > MAX_FOLLOWED_THREADS;
            followed_threads.truncate(MAX_FOLLOWED_THREADS);
            let mentions: i64 = row.try_get("mentions")?;
            channels.push(ChannelReadSummary {
                channel_id: row.try_get("id")?,
                unread: row.try_get("unread")?,
                mentions: mentions as u32,
                read_through_id: row.try_get("read_through_id")?,
                latest_id: row.try_get("latest_id")?,
                threads,
                threads_truncated,
                followed_threads: Some(followed_threads),
                followed_threads_truncated: Some(followed_threads_truncated),
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
            revision: account.revision,
        })
    }

    /// Followed threads in joined channels, newest arrival first, read or
    /// not, from the writer in one read-only snapshot. A thread with no reply
    /// yet is omitted. Callers must recheck admission/resource access before
    /// releasing this data.
    pub async fn personal_read_threads(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        limit: usize,
        after: Option<&ThreadsCursor>,
    ) -> Result<ThreadsPage> {
        if !(1..=MAX_THREADS).contains(&limit) {
            return Err(DbError::InvalidData("invalid threads limit".into()));
        }
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
        let actor_bytes = actor.to_bytes();
        let account = read_account(&mut tx, community, &actor_bytes, retention_seconds).await?;
        // Every live follow is probed for its last arrival and sorted: the
        // cost grows with the actor's recent follows, not with community size.
        let legacy_types = crate::store::channel::legacy_channel_type_sql("c.channel_type");
        let sql = format!(
            r#"SELECT t.channel_id, encode(t.root_id,'hex') AS root_id, t.unread,
                tc.n AS mentions,
                encode(t.through_message_id,'hex') AS read_through_id,
                encode(t.latest_id,'hex') AS latest_id, t.latest_at
             FROM (
                SELECT tf.root_id, tf.channel_id, c.channel_type='dm' AS dm, p.position, p.walk_from,
                    tf.through_message_id, tl.id AS latest_id, tl.received_at AS latest_at,
                    (tl.received_at > p.position AND tl.pubkey<>$2) AS unread
                FROM personal_read_frontiers tf
                JOIN channel_members cm ON cm.community_id=$1 AND cm.channel_id=tf.channel_id
                    AND cm.pubkey=$2 AND cm.removed_at IS NULL
                JOIN channels c ON c.community_id=$1 AND c.id=tf.channel_id AND c.deleted_at IS NULL
                    AND {legacy_types}
                CROSS JOIN LATERAL (SELECT GREATEST(tf.through_timestamp,
                    COALESCE($3::timestamptz, 'infinity'), $4) AS position,
                    GREATEST(tf.through_timestamp, COALESCE($3::timestamptz, 'infinity')) AS walk_from) p
                CROSS JOIN LATERAL ({LATEST_REPLY}) tl
                WHERE tf.community_id=$1 AND tf.actor=$2 AND tf.root_id<>''::bytea
                    AND NOT tf.unfollowed AND (tf.explicit OR tf.active_at >= $4)
                    AND ($8::timestamptz IS NULL OR tl.received_at < $8
                        OR (tl.received_at=$8 AND tf.root_id>$9))
                ORDER BY tl.received_at DESC, tf.root_id LIMIT $10
             ) t CROSS JOIN LATERAL ({THREAD_MENTIONS}) tc
             ORDER BY t.latest_at DESC, t.root_id"#
        );
        let cutoff = DateTime::from_timestamp_millis(account.cutoff_ms)
            .ok_or_else(|| DbError::InvalidData("invalid dormancy cutoff".into()))?;
        // Only compile-time constants are interpolated.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community.as_uuid())
            .bind(actor_bytes.as_slice())
            .bind(account.started_at)
            .bind(cutoff)
            .bind(actor.to_hex())
            .bind(skew())
            .bind(MAX_UNREAD_COUNT as i64)
            .bind(after.map(|c| c.latest_at))
            .bind(after.map(|c| c.root_id.as_slice()))
            .bind((limit + 1) as i64)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let has_more = rows.len() > limit;
        let mut threads = Vec::with_capacity(limit);
        let mut last = None;
        for row in rows.into_iter().take(limit) {
            let mentions: i64 = row.try_get("mentions")?;
            let root_id: String = row.try_get("root_id")?;
            last = Some((
                row.try_get::<DateTime<Utc>, _>("latest_at")?,
                root_id.clone(),
            ));
            threads.push(FollowedThread {
                channel_id: row.try_get("channel_id")?,
                thread: ThreadReadSummary {
                    root_id,
                    unread: row.try_get("unread")?,
                    mentions: mentions as u32,
                    read_through_id: row.try_get("read_through_id")?,
                    latest_id: row.try_get("latest_id")?,
                },
            });
        }
        let next_cursor = last
            .filter(|_| has_more)
            .map(|(at, root)| format!("{}.{root}", at.timestamp_micros()));
        Ok(ThreadsPage {
            threads,
            next_cursor,
            revision: account.revision,
        })
    }
}

fn skew() -> chrono::Duration {
    chrono::Duration::seconds(i64::from(MAX_ARRIVAL_SKEW_SECONDS))
}

/// The follow dormancy cutoff and the account's start, in the caller's
/// snapshot. A dormant follow keeps its row and position.
pub(super) async fn read_account(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    retention_seconds: u32,
) -> Result<ReadAccount> {
    let row = sqlx::query(
        "SELECT date_trunc('milliseconds',transaction_timestamp()-make_interval(secs=>$1::double precision)) AS cutoff,
            a.started_at, COALESCE(a.revision, 0) AS revision
         FROM (SELECT 1) one LEFT JOIN personal_read_accounts a ON a.community_id=$2 AND a.actor=$3",
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
        revision: row.try_get("revision")?,
    })
}
