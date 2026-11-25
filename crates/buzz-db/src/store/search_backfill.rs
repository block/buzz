//! Search backfill: queues the keys of events stored before the search index
//! existed, so they reach it through the same outbox as live writes.
//!
//! Each community's `search_backfill` row holds a cursor that moves in the
//! same transaction as the batch it queues, so a restart resumes from the
//! last committed batch. A community without a row starts from its newest
//! event: deleting the row is how an operator backfills again. Queuing an
//! indexed event again only rewrites its document.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::search_outbox::SEARCHABLE_KINDS;
use crate::Result;

/// Communities a step may pass over because their deletion fenced them
/// between candidate selection and admission.
const STEP_COMMUNITY_CANDIDATES: usize = 8;

/// What one [`step`] queued for a community, and where its backfill stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillStep {
    /// The community the step backfilled.
    pub community: CommunityId,
    /// Event keys this step queued.
    pub queued: u64,
    /// Event keys queued since the community's backfill started.
    pub total_queued: i64,
    /// Creation time of the oldest event queued so far: everything newer has
    /// been queued.
    pub cursor: Option<DateTime<Utc>>,
    /// Whether no older event is left to queue.
    pub completed: bool,
}

/// Whether the outbox holds fewer than `bound` rows the indexer can drain.
/// Rows of a community fenced for deletion are left out: claims skip them
/// until the purge removes them, so counting them could hold back every
/// other community's backfill for as long as the deletion takes. Reads at
/// most `bound` rows, so it stays cheap however large the queue grows.
pub async fn outbox_below(pool: &PgPool, bound: i64) -> Result<bool> {
    let depth: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ( \
             SELECT 1 FROM search_outbox o \
             JOIN communities c ON c.id = o.community_id \
             WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL \
             LIMIT $1 \
         ) AS bounded",
    )
    .bind(bound)
    .fetch_one(pool)
    .await?;
    Ok(depth < bound)
}

/// Every active community and whether its backfill has queued all of its
/// stored events.
pub async fn progress(pool: &PgPool) -> Result<Vec<(CommunityId, bool)>> {
    let rows: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT c.id, b.completed_at IS NOT NULL FROM communities c \
         LEFT JOIN search_backfill b ON b.community_id = c.id \
         WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, completed)| (CommunityId::from_uuid(id), completed))
        .collect())
}

/// Forget every community's backfill progress, so each is backfilled again
/// from its newest event, and return how many rows were removed. Before
/// creating a new, empty index: the progress described the index it
/// replaces. Deleting rows
/// needs no community admission: it only removes rows that a community
/// deletion's purge removes too.
pub async fn reset(pool: &PgPool) -> Result<u64> {
    let reset = sqlx::query("DELETE FROM search_backfill")
        .execute(pool)
        .await?;
    Ok(reset.rows_affected())
}

/// Queue up to `limit` more event keys for the active community not in
/// `skip` whose backfill is incomplete and least recently advanced, newest
/// events first, and advance its cursor.
///
/// Returns `None` when every active community's backfill is complete.
/// Communities take turns, one batch each, so a large one does not hold
/// back the rest.
pub async fn step(pool: &PgPool, limit: i64, skip: &[CommunityId]) -> Result<Option<BackfillStep>> {
    let mut refused: Vec<Uuid> = skip.iter().map(|community| *community.as_uuid()).collect();
    for _ in 0..STEP_COMMUNITY_CANDIDATES {
        let candidate: Option<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM communities c \
             LEFT JOIN search_backfill b ON b.community_id = c.id \
             WHERE c.deletion_state = 'active' AND c.deleted_at IS NULL \
               AND b.completed_at IS NULL AND c.id <> ALL($1) \
             ORDER BY b.updated_at NULLS FIRST, c.id LIMIT 1",
        )
        .bind(&refused)
        .fetch_optional(pool)
        .await?;
        let Some(id) = candidate else {
            return Ok(None);
        };
        let community = CommunityId::from_uuid(id);
        let Some(mut tx) = crate::begin_community_sweep_transaction(
            pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?
        else {
            refused.push(id);
            continue;
        };
        let marker = sqlx::query(
            "SELECT cursor_created_at, cursor_event_id, completed_at FROM search_backfill \
             WHERE community_id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(tx.conn())
        .await?;
        let mut cursor: Option<(DateTime<Utc>, Vec<u8>)> = None;
        if let Some(marker) = marker {
            if marker
                .try_get::<Option<DateTime<Utc>>, _>("completed_at")?
                .is_some()
            {
                // Completed since the candidate read.
                refused.push(id);
                continue;
            }
            let at: Option<DateTime<Utc>> = marker.try_get("cursor_created_at")?;
            let event: Option<Vec<u8>> = marker.try_get("cursor_event_id")?;
            cursor = at.zip(event);
        }

        // The events primary key (community_id, created_at, id), scanned
        // backwards from the cursor; other kinds are skipped as it goes.
        let below_cursor = if cursor.is_some() {
            "AND (e.created_at, e.id) < ($4, $5)"
        } else {
            ""
        };
        let sql = format!(
            "WITH batch AS ( \
                 SELECT e.id, e.created_at FROM events e \
                 WHERE e.community_id = $1 AND e.kind = ANY($2) AND e.deleted_at IS NULL \
                   {below_cursor} \
                 ORDER BY e.created_at DESC, e.id DESC LIMIT $3 \
             ), queued AS ( \
                 INSERT INTO search_outbox (community_id, event_id, event_created_at) \
                 SELECT $1, id, created_at FROM batch \
                 RETURNING event_id, event_created_at \
             ) \
             SELECT count(*) AS queued, \
                    min(event_created_at) AS oldest_at, \
                    (array_agg(event_id ORDER BY event_created_at, event_id))[1] AS oldest_id \
             FROM queued"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(SEARCHABLE_KINDS.as_slice())
            .bind(limit);
        if let Some((at, event)) = &cursor {
            query = query.bind(at).bind(event);
        }
        let row = query.fetch_one(tx.conn()).await?;
        let queued: i64 = row.try_get("queued")?;
        let oldest_at: Option<DateTime<Utc>> = row.try_get("oldest_at")?;
        let oldest_id: Option<Vec<u8>> = row.try_get("oldest_id")?;
        let completed = queued < limit;

        let progress = sqlx::query(
            "INSERT INTO search_backfill \
                 (community_id, cursor_created_at, cursor_event_id, enqueued, completed_at) \
             VALUES ($1, $2, $3, $4, CASE WHEN $5 THEN now() END) \
             ON CONFLICT (community_id) DO UPDATE SET \
                 cursor_created_at = COALESCE(EXCLUDED.cursor_created_at, \
                                              search_backfill.cursor_created_at), \
                 cursor_event_id = COALESCE(EXCLUDED.cursor_event_id, \
                                            search_backfill.cursor_event_id), \
                 enqueued = search_backfill.enqueued + EXCLUDED.enqueued, \
                 completed_at = EXCLUDED.completed_at, \
                 updated_at = now() \
             RETURNING enqueued, cursor_created_at",
        )
        .bind(id)
        .bind(oldest_at)
        .bind(oldest_id)
        .bind(queued)
        .bind(completed)
        .fetch_one(tx.conn())
        .await?;
        let step = BackfillStep {
            community,
            queued: u64::try_from(queued).unwrap_or_default(),
            total_queued: progress.try_get("enqueued")?,
            cursor: progress.try_get("cursor_created_at")?,
            completed,
        };
        tx.commit().await?;
        return Ok(Some(step));
    }
    Ok(None)
}
