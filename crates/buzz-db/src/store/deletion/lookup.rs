//! Community-ID lookup for parent deletion workflows.

use super::*;

impl DeletionStore {
    /// Find the current request, including terminal requests. If only aborted
    /// requests exist, return the latest one; callers must not silently recreate it.
    pub async fn find_for_community(
        &self,
        community: CommunityId,
    ) -> Result<Option<DeletionRequest>> {
        let row = sqlx::query(
            "SELECT * FROM community_deletion_requests WHERE community_id = $1
             ORDER BY (stage = 'aborted'), created_at DESC, id DESC LIMIT 1",
        )
        .bind(community.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_request).transpose()
    }
}
