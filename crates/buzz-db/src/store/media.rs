//! Media reads inherit the permissions of events that publish the local URL.

use buzz_core::kind::{AUTHOR_ONLY_KINDS, KIND_AGENT_ENGRAM, P_GATED_KINDS, SHARED_GATED_KINDS};
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;

use crate::{Db, Result};

#[cfg(test)]
mod tests;

impl Db {
    /// Record possession only after the upload pipeline has hashed the bytes.
    /// An idempotent re-upload proves possession again and may add an uploader.
    #[datastore_span(name = "record_media_upload", system = "postgresql")]
    pub async fn record_media_upload(
        &self,
        community: CommunityId,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<()> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query(
            "INSERT INTO media_uploaders (community_id, sha256, pubkey) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(sha256)
        .bind(pubkey)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Find local blob references with the same extractor used by the event index.
    pub async fn local_media_hashes(
        &self,
        content: &str,
        tags: &serde_json::Value,
        host: &str,
    ) -> Result<Vec<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        Ok(sqlx::query_scalar("SELECT buzz_media_hashes($1, $2, $3)")
            .bind(content)
            .bind(tags)
            .bind(host)
            .fetch_one(&mut *connection)
            .await?)
    }

    /// Authorize a read against live primary-database membership, without caches.
    /// Unreferenced blobs retain the existing community-level read policy.
    #[datastore_span(name = "can_read_media", system = "postgresql")]
    pub async fn can_read_media(
        &self,
        community: CommunityId,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<bool> {
        self.media_access(community, sha256, pubkey, false).await
    }

    /// Authorize publication before accepting an event containing a local URL.
    /// Knowing a hash is insufficient: the author must currently have read
    /// access or have supplied the actual bytes in a successful upload.
    #[datastore_span(name = "can_reference_media", system = "postgresql")]
    pub async fn can_reference_media(
        &self,
        community: CommunityId,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<bool> {
        self.media_access(community, sha256, pubkey, true).await
    }

    async fn media_access(
        &self,
        community: CommunityId,
        sha256: &str,
        pubkey: &[u8],
        for_publication: bool,
    ) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let as_i32 = |kinds: &[u32]| kinds.iter().map(|kind| *kind as i32).collect::<Vec<_>>();
        Ok(sqlx::query_scalar(
            r#"
            WITH local_refs AS NOT MATERIALIZED (
                SELECT e.* FROM events e
                JOIN communities community ON community.id = e.community_id
                WHERE e.community_id = $1 AND buzz_media_hashes(e.content, e.tags) @> ARRAY[$2]::TEXT[]
                  AND $2 = ANY(buzz_media_hashes(e.content, e.tags, community.host))
            )
            SELECT NOT EXISTS (SELECT 1 FROM local_refs) OR EXISTS (
                SELECT 1 FROM local_refs e
                WHERE e.deleted_at IS NULL
                  AND (e.channel_id IS NULL OR EXISTS (
                      SELECT 1 FROM channels c
                      WHERE c.community_id = $1 AND c.id = e.channel_id AND c.deleted_at IS NULL
                        AND (c.visibility = 'open' OR EXISTS (
                            SELECT 1 FROM channel_members cm
                            WHERE cm.community_id = $1 AND cm.channel_id = c.id
                              AND cm.pubkey = $3 AND cm.removed_at IS NULL
                        ))
                  ))
                  AND (NOT (e.kind = ANY($4)) OR e.pubkey = $3)
                  AND (NOT (e.kind = ANY($5)) OR EXISTS (
                      SELECT 1 FROM jsonb_array_elements(e.tags) tag
                      WHERE tag->>0 = 'p' AND tag->>1 = encode($3, 'hex')
                  ))
                  AND (NOT (e.kind = ANY($6)) OR e.pubkey = $3 OR (
                      SELECT count(*) = 1 AND bool_and(tag = '["shared","true"]'::JSONB)
                      FROM jsonb_array_elements(e.tags) tag WHERE tag->>0 = 'shared'
                  ))
                  AND (e.kind <> $7 OR e.pubkey = $3 OR EXISTS (
                      SELECT 1 FROM jsonb_array_elements(e.tags) tag
                      WHERE tag->>0 = 'p' AND tag->>1 = encode($3, 'hex')
                  ))
            ) OR EXISTS (
                SELECT 1 FROM communities c
                WHERE c.id = $1 AND $2 = ANY(buzz_media_hashes(COALESCE(c.icon, ''), '[]', c.host))
            ) OR (
                $8 AND EXISTS (
                    SELECT 1 FROM media_uploaders
                    WHERE community_id = $1 AND sha256 = $2 AND pubkey = $3
                )
            )
            "#,
        )
        .bind(community.as_uuid())
        .bind(sha256)
        .bind(pubkey)
        .bind(as_i32(AUTHOR_ONLY_KINDS))
        .bind(as_i32(P_GATED_KINDS))
        .bind(as_i32(SHARED_GATED_KINDS))
        .bind(KIND_AGENT_ENGRAM as i32)
        .bind(for_publication)
        .fetch_one(&mut *connection)
        .await?)
    }
}
