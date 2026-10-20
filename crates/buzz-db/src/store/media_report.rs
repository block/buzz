//! Private, temporary media access for review of an open event report.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use uuid::Uuid;

use crate::{Db, Result};

impl Db {
    /// Find an open report of a live event that actually granted this file.
    ///
    /// The caller must authorize ViewQueue and durably audit the access before
    /// serving bytes. Report notes, blob-only reports, and unrelated references
    /// never supply attachment provenance. Reads use the writer for revocation.
    #[datastore_span(name = "open_media_report", system = "postgresql")]
    pub async fn open_media_report(
        &self,
        community: CommunityId,
        sha256: &str,
    ) -> Result<Option<(Uuid, Vec<u8>)>> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        Ok(sqlx::query_as(
            "SELECT r.id, r.target_event_id FROM moderation_reports r \
             JOIN media_shares s ON s.community_id = r.community_id \
               AND s.event_id = r.target_event_id AND s.sha256 = $2 \
             JOIN events e ON e.community_id = s.community_id AND e.id = s.event_id \
               AND e.created_at = s.event_created_at AND e.deleted_at IS NULL \
             WHERE r.community_id = $1 AND r.target_kind = 'event' AND r.status = 'open' \
             ORDER BY r.created_at, r.id LIMIT 1",
        )
        .bind(community.as_uuid())
        .bind(sha256)
        .fetch_optional(&mut *conn)
        .await?)
    }
}
