//! Media read authorization from where a blob is referenced.

use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Whether `reader` may read the blob `sha256`.
///
/// The uploader can always read it: uploading the exact bytes proves
/// possession. Otherwise a blob attached in a channel is readable only through
/// an attachment the reader can see: one in a channel they are an active member
/// of or that is open, or a channel-less one. Attachments count whether or not
/// the event was deleted; blobs attached in no channel stay readable to the
/// whole community. Ingest only admits an attachment whose author can read the
/// blob, so knowing a hash is not enough to widen who can read it.
///
/// A blob is attached as an `imeta` file (`x`) or video poster (`image-x`), or
/// as a `link-preview` image or favicon hash. Containment is the GIN-indexed
/// prefilter; the positional check keeps the hash in another field from
/// counting.
pub async fn can_read_blob(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
    reader: &[u8],
) -> Result<bool> {
    let imeta_entries = vec![format!("x {sha256}"), format!("image-x {sha256}")];
    let mut containments: Vec<serde_json::Value> = imeta_entries
        .iter()
        .map(|entry| serde_json::json!([["imeta", entry]]))
        .collect();
    containments.push(serde_json::json!([["link-preview", sha256]]));

    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let readable = sqlx::query_scalar(
        "WITH refs AS ( \
             SELECT DISTINCT channel_id FROM events \
             WHERE community_id = $1 \
               AND tags @> ANY($2::jsonb[]) \
               AND EXISTS (SELECT 1 FROM jsonb_array_elements(tags) AS tag \
                           WHERE (tag->>0 = 'imeta' AND tag ?| $3) \
                              OR (tag->>0 = 'link-preview' AND $4 IN (tag->>8, tag->>10))) \
         ) \
         SELECT EXISTS (SELECT 1 FROM media_uploaders \
                        WHERE community_id = $1 AND sha256 = $4 AND pubkey = $5) \
             OR NOT EXISTS (SELECT 1 FROM refs WHERE channel_id IS NOT NULL) \
             OR EXISTS (SELECT 1 FROM refs WHERE channel_id IS NULL) \
             OR EXISTS (SELECT 1 FROM refs r \
                        JOIN channels c ON c.community_id = $1 AND c.id = r.channel_id \
                        WHERE c.deleted_at IS NULL \
                          AND (c.visibility = 'open' \
                               OR EXISTS (SELECT 1 FROM channel_members cm \
                                          WHERE cm.community_id = $1 AND cm.channel_id = c.id \
                                            AND cm.pubkey = $5 AND cm.removed_at IS NULL)))",
    )
    .bind(community_id.as_uuid())
    .bind(containments)
    .bind(imeta_entries)
    .bind(sha256)
    .bind(reader)
    .fetch_one(&mut *connection)
    .await?;
    Ok(readable)
}

impl Db {
    /// Whether `reader` may read the blob; see [`can_read_blob`].
    #[datastore_span(name = "can_read_blob", system = "postgresql")]
    pub async fn can_read_blob(
        &self,
        community_id: CommunityId,
        sha256: &str,
        reader: &[u8],
    ) -> Result<bool> {
        can_read_blob(&self.pool, community_id, sha256, reader).await
    }
}
