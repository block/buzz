//! Repair on read: shared rows that are missing, behind the event log, or
//! name a deleted message are rebuilt from stored history before they are
//! returned. A first page and a `since` read check every row of the person's
//! list; a read of named rows checks those rows.

use buzz_core::CommunityId;
use sqlx::{PgConnection, Row as _};
use uuid::Uuid;

use super::{
    model::{FILL_SCAN, HORIZON_SECONDS, PREVIEW_CHARS},
    writer::{channel_merge_sql, not_future, thread_merge_sql},
};
use crate::{observability, store::personal_read::ELIGIBLE_KINDS, Db, Result};

/// Rows rebuilt per transaction.
const FILL_BATCH: usize = 64;
/// Most channels, and most threads, one read finds wanting. A bound on the
/// query, far above what one person has; time is what limits the rebuild.
const MAX_WANTING: i64 = 4096;
/// How long one read spends rebuilding. A batch that starts within it
/// finishes; rows not reached are served as stored (a conversation without
/// its preview, placed by its creation time) and the next read continues.
const REBUILD_BUDGET: std::time::Duration = std::time::Duration::from_millis(2500);
/// Most of a person's live threads one read checks, most recently engaged
/// first.
const MAX_THREAD_PROBES: i64 = 512;
/// Rebuilds running at once in one relay process, and how long a read waits
/// for a turn. Rebuilding takes writer connections that message writes need
/// more.
const REPAIRS_AT_ONCE: usize = 4;
const REPAIR_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Shared rows a read found wanting.
#[derive(Debug, Default)]
pub(super) struct Wanting {
    pub(super) channels: Vec<Uuid>,
    /// `(channel, thread root)`.
    pub(super) threads: Vec<(Uuid, Vec<u8>)>,
}

impl Wanting {
    pub(super) fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.threads.is_empty()
    }
}

/// The person's joined channels and listed threads whose shared row is missing,
/// behind the log, or names a deleted message. `only_*` restrict the check to
/// named rows.
///
/// "Behind" is one index probe per row, pruned to the partitions after
/// `seen_at`: is any stored event newer than the row accounts for?
pub(super) async fn wanting(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8; 32],
    only_channels: Option<&[Uuid]>,
    only_roots: Option<&[Vec<u8>]>,
) -> Result<Wanting> {
    let mut wanting = Wanting::default();
    let rows = sqlx::query(
        r#"SELECT c.id FROM channel_members cm
            JOIN channels c ON c.community_id = cm.community_id AND c.id = cm.channel_id
            LEFT JOIN channel_heads h ON h.community_id = c.community_id AND h.channel_id = c.id
            WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
                AND c.deleted_at IS NULL
                AND ($3::uuid[] IS NULL OR c.id = ANY($3))
                AND (h.channel_id IS NULL
                    OR EXISTS (SELECT 1 FROM events n
                        WHERE n.community_id = c.community_id AND n.channel_id = c.id
                            AND n.created_at > COALESCE(h.seen_at, '-infinity')
                            AND n.created_at <= clock_timestamp())
                    OR (h.head_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM events e
                        WHERE e.community_id = h.community_id AND e.created_at = h.head_created_at
                            AND e.id = h.head_id AND e.deleted_at IS NULL)))
            ORDER BY c.id LIMIT $4"#,
    )
    .bind(community.as_uuid())
    .bind(actor.as_slice())
    .bind(only_channels)
    .bind(MAX_WANTING)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        wanting.channels.push(row.try_get("id")?);
    }
    let rows = sqlx::query(
        r#"SELECT f.channel_id, f.root_id FROM (
                SELECT f.channel_id, f.root_id FROM personal_read_frontiers f
                WHERE f.community_id = $1 AND f.actor = $2 AND f.root_id <> ''::bytea
                    AND (f.state <> 'default' OR f.marked_unread
                        OR f.engaged_at >= clock_timestamp() - make_interval(secs => $5))
                    AND ($3::bytea[] IS NULL OR f.root_id = ANY($3))
                ORDER BY f.engaged_at DESC NULLS LAST LIMIT $6
            ) f
            JOIN channel_members cm ON cm.community_id = $1 AND cm.channel_id = f.channel_id
                AND cm.pubkey = $2 AND cm.removed_at IS NULL
            LEFT JOIN thread_heads h ON h.community_id = $1 AND h.root_id = f.root_id
            WHERE h.root_id IS NULL
                OR EXISTS (SELECT 1 FROM thread_metadata n
                    WHERE n.community_id = $1 AND n.root_event_id = f.root_id
                        AND n.event_created_at > COALESCE(h.seen_at, '-infinity')
                        AND n.event_created_at <= clock_timestamp())
                OR (h.activity_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM events e
                    WHERE e.community_id = $1 AND e.created_at = h.activity_created_at
                        AND e.id = h.activity_id AND e.deleted_at IS NULL))
            ORDER BY f.channel_id, f.root_id LIMIT $4"#,
    )
    .bind(community.as_uuid())
    .bind(actor.as_slice())
    .bind(only_roots)
    .bind(MAX_WANTING)
    .bind(HORIZON_SECONDS as f64)
    .bind(MAX_THREAD_PROBES)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        wanting
            .threads
            .push((row.try_get("channel_id")?, row.try_get("root_id")?));
    }
    Ok(wanting)
}

impl Db {
    /// Rebuild the rows a read found wanting. Best effort and bounded: when
    /// it cannot run or fails, the read serves what is stored (a deleted
    /// message is never returned) and the next read tries again.
    pub(super) async fn repair_activity(&self, community: CommunityId, wanting: &Wanting) {
        static REPAIRS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(REPAIRS_AT_ONCE);
        let deadline = tokio::time::Instant::now() + REBUILD_BUDGET;
        let repaired = match tokio::time::timeout(REPAIR_WAIT, REPAIRS.acquire()).await {
            // A batch still running at the deadline is abandoned and rolls
            // back, so a slow statement cannot outlast the budget.
            Ok(Ok(_turn)) => {
                match tokio::time::timeout_at(deadline, self.rebuild(community, wanting, deadline))
                    .await
                {
                    Ok(rebuilt) => rebuilt.map_err(|error| error.to_string()),
                    Err(_) => Ok(false),
                }
            }
            _ => Err("no turn".to_owned()),
        };
        match repaired {
            Ok(true) => {}
            Ok(false) => {
                metrics::counter!("buzz_db_activity_repairs_unfinished_total").increment(1);
            }
            Err(error) => {
                metrics::counter!("buzz_db_activity_repairs_skipped_total").increment(1);
                tracing::debug!(%error, %community, "activity rows not rebuilt");
            }
        }
    }

    /// Each batch is its own admitted transaction. Returns whether every row
    /// was rebuilt before `deadline`.
    async fn rebuild(
        &self,
        community: CommunityId,
        wanting: &Wanting,
        deadline: tokio::time::Instant,
    ) -> Result<bool> {
        for batch in wanting.channels.chunks(FILL_BATCH) {
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            let mut tx = self.begin_repair(community).await?;
            // Take every row first, creating missing ones, in channel order.
            // An edit stored from here on waits for this transaction, and
            // one stored before is visible to the scan.
            sqlx::query(
                "INSERT INTO channel_heads AS h (community_id, channel_id, updated_at) \
                 SELECT $1, c.id, clock_timestamp() \
                 FROM unnest($2::uuid[]) AS c(id) ORDER BY c.id \
                 ON CONFLICT (community_id, channel_id) DO UPDATE \
                    SET updated_at = h.updated_at",
            )
            .bind(community.as_uuid())
            .bind(batch)
            .execute(tx.conn())
            .await?;
            // Only while the row still names the deleted head: a message
            // stored since then, or another reader, already replaced it.
            sqlx::query(
                "UPDATE channel_heads h SET head_id = NULL, head_created_at = NULL, \
                    head_received_at = NULL, head_pubkey = NULL, head_text = NULL, \
                    head_edit_at = NULL, head_edit_id = NULL, updated_at = clock_timestamp() \
                 WHERE h.community_id = $1 AND h.channel_id = ANY($2) AND h.head_id IS NOT NULL \
                    AND NOT EXISTS (SELECT 1 FROM events e \
                        WHERE e.community_id = $1 AND e.created_at = h.head_created_at \
                            AND e.id = h.head_id AND e.deleted_at IS NULL)",
            )
            .bind(community.as_uuid())
            .bind(batch)
            .execute(tx.conn())
            .await?;
            fill_channels(tx.conn(), community, batch).await?;
            tx.commit().await?;
        }
        for batch in wanting.threads.chunks(FILL_BATCH) {
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            let (channels, roots): (Vec<Uuid>, Vec<Vec<u8>>) = batch.iter().cloned().unzip();
            let mut tx = self.begin_repair(community).await?;
            sqlx::query(
                "INSERT INTO thread_heads AS h (community_id, root_id, channel_id, updated_at) \
                 SELECT $1, t.root, t.channel, clock_timestamp() \
                 FROM unnest($2::uuid[], $3::bytea[]) AS t(channel, root) \
                 WHERE EXISTS (SELECT 1 FROM channels c \
                    WHERE c.community_id = $1 AND c.id = t.channel) \
                 ORDER BY t.root \
                 ON CONFLICT (community_id, root_id) DO UPDATE SET updated_at = h.updated_at",
            )
            .bind(community.as_uuid())
            .bind(&channels)
            .bind(&roots)
            .execute(tx.conn())
            .await?;
            sqlx::query(
                "UPDATE thread_heads h SET activity_at = NULL, activity_id = NULL, \
                    activity_created_at = NULL, activity_pubkey = NULL, activity_text = NULL, \
                    updated_at = clock_timestamp() \
                 WHERE h.community_id = $1 AND h.root_id = ANY($2) AND h.activity_id IS NOT NULL \
                    AND NOT EXISTS (SELECT 1 FROM events e \
                        WHERE e.community_id = $1 AND e.created_at = h.activity_created_at \
                            AND e.id = h.activity_id AND e.deleted_at IS NULL)",
            )
            .bind(community.as_uuid())
            .bind(&roots)
            .execute(tx.conn())
            .await?;
            fill_threads(tx.conn(), community, &channels, &roots).await?;
            tx.commit().await?;
        }
        Ok(true)
    }

    async fn begin_repair(&self, community: CommunityId) -> Result<crate::AdmittedTx> {
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        sqlx::raw_sql(
            "SET LOCAL statement_timeout = '5000ms'; SET LOCAL lock_timeout = '500ms'; \
             SET LOCAL jit = off",
        )
        .execute(tx.conn())
        .await?;
        Ok(tx)
    }
}

fn eligible() -> Vec<i32> {
    ELIGIBLE_KINDS.to_vec()
}

/// Merge each channel's stored history into its row and record how far the
/// log was read. The caller holds the rows, and this is its own statement, so
/// its snapshot sees every edit and deletion committed before they were taken.
async fn fill_channels(
    conn: &mut PgConnection,
    community: CommunityId,
    channels: &[Uuid],
) -> Result<()> {
    let sql = format!(
        r#"INSERT INTO channel_heads AS h (community_id, channel_id, head_id, head_created_at,
                head_received_at, head_pubkey, head_text, head_edit_at, head_edit_id, seen_at,
                updated_at)
            SELECT $1, c.id, t.id, t.created_at, t.received_at, t.pubkey, left(COALESCE(edit.content, t.content), $5), edit.created_at, edit.id,
                newest.created_at, clock_timestamp()
            FROM unnest($2::uuid[]) AS c(id)
            LEFT JOIN LATERAL (
                SELECT created_at FROM events
                WHERE community_id = $1 AND channel_id = c.id
                ORDER BY created_at DESC LIMIT 1
            ) newest ON true
            LEFT JOIN LATERAL (
                SELECT recent.id, recent.created_at, recent.received_at, recent.pubkey,
                    recent.content
                FROM (
                    SELECT id, created_at, received_at, pubkey, kind, deleted_at, tags, content
                    FROM events
                    WHERE community_id = $1 AND channel_id = c.id
                    ORDER BY created_at DESC, id LIMIT $4
                ) recent
                LEFT JOIN thread_metadata tm ON tm.community_id = $1
                    AND tm.event_created_at = recent.created_at AND tm.event_id = recent.id
                WHERE recent.kind = ANY($3) AND recent.deleted_at IS NULL
                    AND (tm.depth IS NULL OR tm.depth = 0 OR (tm.depth = 1 AND tm.broadcast))
                    AND NOT EXISTS (
                        SELECT 1 FROM jsonb_array_elements(recent.tags) AS tag
                        WHERE tag->>0 = 'audience' AND tag->>1 = 'agents')
                ORDER BY recent.received_at DESC, recent.id LIMIT 1
            ) t ON true
            LEFT JOIN LATERAL (
                SELECT content, created_at, id FROM events
                WHERE community_id = $1 AND channel_id = c.id AND kind = 40003
                    AND deleted_at IS NULL AND {target} = t.id
                ORDER BY created_at DESC, id DESC LIMIT 1
            ) edit ON t.id IS NOT NULL
            ORDER BY c.id
            {merge}"#,
        merge = channel_merge_sql(
            &not_future("EXCLUDED.seen_at"),
            &not_future("EXCLUDED.seen_at")
        ),
        target = crate::store::media::MEDIA_EDIT_TARGET_SQL,
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community.as_uuid())
        .bind(channels)
        .bind(eligible())
        .bind(FILL_SCAN)
        .bind(PREVIEW_CHARS as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The same for threads: the newest stored reply. A
/// thread row shows a reply as stored; edits are not applied to it.
async fn fill_threads(
    conn: &mut PgConnection,
    community: CommunityId,
    channels: &[Uuid],
    roots: &[Vec<u8>],
) -> Result<()> {
    let sql = format!(
        r#"INSERT INTO thread_heads AS h (community_id, root_id, channel_id, activity_at,
                activity_id, activity_created_at, activity_pubkey, activity_text,
                seen_at, updated_at)
            SELECT $1, t.root, t.channel, a.received_at, a.id, a.created_at, a.pubkey,
                left(a.content, $6), newest.event_created_at, clock_timestamp()
            FROM unnest($2::uuid[], $3::bytea[]) AS t(channel, root)
            LEFT JOIN LATERAL (
                SELECT event_created_at FROM thread_metadata
                WHERE community_id = $1 AND root_event_id = t.root
                ORDER BY event_created_at DESC LIMIT 1
            ) newest ON true
            LEFT JOIN LATERAL (
                SELECT e.id, e.created_at, e.received_at, e.pubkey, e.content FROM (
                    SELECT event_id, event_created_at FROM thread_metadata
                    WHERE community_id = $1 AND root_event_id = t.root
                    ORDER BY event_created_at DESC, event_id LIMIT $5
                ) recent
                JOIN events e ON e.community_id = $1 AND e.created_at = recent.event_created_at
                    AND e.id = recent.event_id
                WHERE e.kind = ANY($4) AND e.deleted_at IS NULL AND e.channel_id = t.channel
                ORDER BY e.received_at DESC, e.id LIMIT 1
            ) a ON true
            WHERE EXISTS (SELECT 1 FROM channels c WHERE c.community_id = $1 AND c.id = t.channel)
            ORDER BY t.root
            {merge}"#,
        merge = thread_merge_sql(
            &not_future("EXCLUDED.seen_at"),
            &not_future("EXCLUDED.seen_at")
        ),
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(community.as_uuid())
        .bind(channels)
        .bind(roots)
        .bind(eligible())
        .bind(FILL_SCAN)
        .bind(PREVIEW_CHARS as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
