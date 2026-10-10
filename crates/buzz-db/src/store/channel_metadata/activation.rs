//! Bounded startup audit for an offline NIP-CL cutover.

use nostr::PublicKey;
use uuid::Uuid;

use crate::{Db, DbError, Result};
use buzz_core::CommunityId;

impl Db {
    /// Page live channel IDs for an operator's community-scoped metadata repair.
    ///
    /// Includes archived channels, excludes tombstones, and never uses the
    /// interactive catalog's 1,000-row cap. Continue from the last returned ID.
    pub async fn channel_metadata_repair_page(
        &self,
        community: CommunityId,
        after: Uuid,
    ) -> Result<Vec<Uuid>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT id FROM channels WHERE community_id = $1 AND id > $2 \
             AND deleted_at IS NULL ORDER BY id LIMIT 128",
        )
        .bind(community.as_uuid())
        .bind(after)
        .fetch_all(&mut *connection)
        .await?)
    }

    /// Verify label integrity without requiring freshness of ordinary metadata.
    ///
    /// Existing heads must contain the trusted canonical current label set. A
    /// missing head is allowed only for an unlabeled channel: ordinary creation
    /// can commit before its separate publisher, even with label commands off.
    /// Topic/archive changes also commit before publication and cannot be a
    /// routine restart prerequisite. Full operator repair still checks all tags.
    ///
    /// Operator-only, cross-community startup audit. Call before serving when
    /// enabling NIP-CL, after old writers have been externally fenced and repair
    /// has completed. This is not a fleet fence: an old binary can still corrupt
    /// projections if its database access has not been revoked. Each comparison
    /// uses the same scoped admitted transaction as publication; no captured
    /// snapshot or read-replica state establishes readiness.
    pub async fn verify_channel_metadata_activation(&self, relay: PublicKey) -> Result<u64> {
        let mut cursor = (Uuid::nil(), Uuid::nil());
        let mut checked = 0;
        loop {
            let channels: Vec<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT community_id, id FROM channels WHERE deleted_at IS NULL \
                 AND (community_id, id) > ($1, $2) \
                 ORDER BY community_id, id LIMIT 128",
            )
            .bind(cursor.0)
            .bind(cursor.1)
            .fetch_all(&self.pool)
            .await?;
            if channels.is_empty() {
                return Ok(checked);
            }
            for (community, channel) in channels {
                cursor = (community, channel);
                let write = self
                    .begin_channel_metadata_write(CommunityId::from_uuid(community), channel, relay)
                    .await?;
                let stale = if write.previous.is_none() && write.labels().values().is_empty() {
                    // Still reject a channel deleted after enumeration.
                    write.channel()?;
                    false
                } else {
                    !write.has_current_label_snapshot().await?
                };
                write.rollback().await?;
                if stale {
                    return Err(DbError::InvalidData(format!(
                        "NIP-CL activation requires metadata repair for community {community} channel {channel}"
                    )));
                }
                checked += 1;
            }
        }
    }
}
