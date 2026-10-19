//! Media read authorization from where a blob is referenced.

use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;

use crate::{error::Result, Db};
use buzz_core::kind::{KIND_EMOJI_LIST, KIND_EMOJI_SET};
use buzz_core::CommunityId;

/// Whether `reader` may read every blob in `sha256s` (vacuously true if empty).
///
/// A blob is readable by its uploader (uploading the exact bytes proves
/// possession), and by everyone while it is a current avatar or in a live custom
/// emoji set or list. Otherwise a blob attached in a channel is readable only
/// through an attachment the reader can see: one in a channel they are an
/// active member of or that is open, or a channel-less one. Attachments count
/// whether or not the event was deleted; a blob attached nowhere stays readable
/// to the whole community.
///
/// A blob is attached as an `imeta` file (`x`) or video poster (`image-x`), or
/// as a `link-preview` image or favicon hash; containment is the GIN-indexed
/// prefilter and the positional check keeps the hash in another field from
/// counting. Avatars and emoji name a blob by a standalone 64-hex token. Ingest
/// admits every such reference only if its author can already read the blob, so
/// knowing a hash is not enough to change who can read it.
pub async fn can_read_blobs(
    pool: &PgPool,
    community_id: CommunityId,
    sha256s: &[String],
    reader: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let readable = sqlx::query_scalar(
        "WITH blobs AS (SELECT DISTINCT unnest($2::text[]) AS sha256), \
         refs AS ( \
             SELECT b.sha256, e.channel_id FROM blobs b \
             JOIN events e ON e.community_id = $1 \
              AND (e.tags @> jsonb_build_array(jsonb_build_array('imeta', 'x ' || b.sha256)) \
                OR e.tags @> jsonb_build_array(jsonb_build_array('imeta', 'image-x ' || b.sha256)) \
                OR e.tags @> jsonb_build_array(jsonb_build_array('link-preview', b.sha256))) \
              AND EXISTS (SELECT 1 FROM jsonb_array_elements(e.tags) AS tag \
                          WHERE (tag->>0 = 'imeta' \
                                 AND tag ?| ARRAY['x ' || b.sha256, 'image-x ' || b.sha256]) \
                             OR (tag->>0 = 'link-preview' \
                                 AND b.sha256 IN (tag->>8, tag->>10))) \
         ) \
         SELECT coalesce(bool_and(CASE \
             WHEN EXISTS (SELECT 1 FROM media_uploaders u WHERE u.community_id = $1 \
                          AND u.sha256 = b.sha256 AND u.pubkey = $3) THEN true \
             WHEN NOT EXISTS (SELECT 1 FROM refs r \
                              WHERE r.sha256 = b.sha256 AND r.channel_id IS NOT NULL) THEN true \
             WHEN EXISTS (SELECT 1 FROM refs r \
                          WHERE r.sha256 = b.sha256 AND r.channel_id IS NULL) THEN true \
             WHEN EXISTS (SELECT 1 FROM refs r \
                          JOIN channels c ON c.community_id = $1 AND c.id = r.channel_id \
                          WHERE r.sha256 = b.sha256 AND c.deleted_at IS NULL \
                            AND (c.visibility = 'open' \
                                 OR EXISTS (SELECT 1 FROM channel_members cm \
                                            WHERE cm.community_id = $1 AND cm.channel_id = c.id \
                                              AND cm.pubkey = $3 AND cm.removed_at IS NULL))) \
                 THEN true \
             ELSE EXISTS (SELECT 1 FROM users WHERE community_id = $1 \
                          AND avatar_url ~ ('(^|[^0-9a-f])' || b.sha256 || '([^0-9a-f]|$)')) \
               OR EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND kind IN ($4, $5) \
                          AND deleted_at IS NULL \
                          AND tags::text ~ ('(^|[^0-9a-f])' || b.sha256 || '([^0-9a-f]|$)')) \
         END), true) FROM blobs b",
    )
    .bind(community_id.as_uuid())
    .bind(sha256s)
    .bind(reader)
    .bind(KIND_EMOJI_SET as i32)
    .bind(KIND_EMOJI_LIST as i32)
    .fetch_one(&mut *connection)
    .await?;
    Ok(readable)
}

impl Db {
    /// Whether `reader` may read every listed blob; see [`can_read_blobs`].
    #[datastore_span(name = "can_read_blobs", system = "postgresql")]
    pub async fn can_read_blobs(
        &self,
        community_id: CommunityId,
        sha256s: &[String],
        reader: &[u8],
    ) -> Result<bool> {
        can_read_blobs(&self.pool, community_id, sha256s, reader).await
    }
}
