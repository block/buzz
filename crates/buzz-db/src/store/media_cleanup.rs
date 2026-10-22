//! Bounded, tenant-admitted reclamation of grants whose sources are gone.

use std::collections::BTreeMap;

use buzz_datastore_tracing::datastore_span;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Db, Result};

/// Volatile primary-key position for the periodic media-share reaper.
/// Restarting a scan is safe: cleanup never creates an authorization grant.
#[derive(Clone, Debug, FromRow)]
pub struct MediaShareCleanupCursor {
    community_id: Uuid,
    sha256: String,
    event_id: Vec<u8>,
}

/// Committed cleanup and the next scan position; `None` starts another cycle.
pub struct MediaShareCleanupBatch {
    /// Number of inactive grant rows removed.
    pub removed: u64,
    /// Continue after this key, or wrap to the beginning on the next tick.
    pub next_cursor: Option<MediaShareCleanupCursor>,
}

impl Db {
    /// Inspect at most `batch_size` grants in primary-key order and remove
    /// inactive ones under each community's write admission.
    ///
    /// Keep the first channel grant per blob, even if inactive: otherwise
    /// removing its last historical channel row would restore uploader preview
    /// access. This bounds dead channel history to one row per blob; public
    /// profile/emoji grants need no such marker. Live sources are never removed.
    /// Tenant-local errors retry in a later scan cycle, as do skipped locks.
    #[datastore_span(name = "reap_inactive_media_shares", system = "postgresql")]
    pub async fn reap_inactive_media_shares(
        &self,
        after: Option<&MediaShareCleanupCursor>,
        batch_size: u16,
    ) -> Result<MediaShareCleanupBatch> {
        if !(1..=1000).contains(&batch_size) {
            return Err(crate::DbError::InvalidData(
                "media cleanup batch size must be between 1 and 1000".into(),
            ));
        }
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Maintenance,
        )
        .await?;
        // Intentional fleet-wide candidate discovery, bounded by the leading
        // primary key. It reads only keys; every mutation is admitted below.
        let rows = sqlx::query_as::<_, MediaShareCleanupCursor>(
            "SELECT community_id, sha256, event_id FROM media_shares \
             WHERE (community_id, sha256, event_id) > ($1, $2, $3) \
             ORDER BY community_id, sha256, event_id LIMIT $4",
        )
        .bind(after.map_or(Uuid::nil(), |key| key.community_id))
        .bind(after.map_or("", |key| key.sha256.as_str()))
        .bind(after.map_or(&[][..], |key| key.event_id.as_slice()))
        .bind(i64::from(batch_size))
        .fetch_all(&mut *conn)
        .await?;
        drop(conn);
        let next_cursor = (rows.len() == usize::from(batch_size))
            .then(|| rows.last().cloned())
            .flatten();
        let mut groups = BTreeMap::<Uuid, (Vec<String>, Vec<Vec<u8>>)>::new();
        for row in rows {
            let (hashes, ids) = groups.entry(row.community_id).or_default();
            hashes.push(row.sha256);
            ids.push(row.event_id);
        }
        let groups = &groups;
        let removed = crate::sweep_admitted_communities(
            &self.pool,
            groups.keys().copied(),
            crate::observability::WriterOperation::Maintenance,
            "media_share_reap",
            |community, mut tx| async move {
                sqlx::raw_sql("SET LOCAL lock_timeout = '1s'; SET LOCAL statement_timeout = '5s'")
                    .execute(tx.conn())
                    .await?;
                let (hashes, ids) = &groups[&community];
                let removed = sqlx::query(concat!(
                    "DELETE FROM media_shares s \
                     USING unnest($2::text[], $3::bytea[]) AS batch(sha256, event_id) \
                     WHERE s.community_id = $1 AND s.sha256 = batch.sha256 \
                       AND s.event_id = batch.event_id \
                       AND NOT EXISTS (SELECT 1 FROM events e \
                           WHERE e.community_id = s.community_id AND e.id = s.event_id \
                             AND e.created_at = s.event_created_at AND ",
                    crate::store::media_reference::live_source_sql!(),
                    ") AND (s.channel_id IS NULL OR EXISTS ( \
                           SELECT 1 FROM media_shares previous \
                           WHERE previous.community_id = s.community_id \
                             AND previous.sha256 = s.sha256 AND previous.channel_id IS NOT NULL \
                             AND previous.event_id < s.event_id))",
                ))
                .bind(community)
                .bind(hashes)
                .bind(ids)
                .execute(tx.conn())
                .await?
                .rows_affected();
                tx.commit().await?;
                Ok(removed)
            },
        )
        .await
        .into_iter()
        .sum();
        Ok(MediaShareCleanupBatch {
            removed,
            next_cursor,
        })
    }
}
