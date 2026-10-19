//! Media read authorization from where each blob is shared.

use buzz_datastore_tracing::datastore_span;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Share each blob in `sha256s` into `channel_id`, or with the whole community
/// when `None`. Called with the transaction that stores the referencing event.
pub(crate) async fn record_media_shares(
    conn: &mut PgConnection,
    community_id: CommunityId,
    sha256s: &[String],
    channel_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO media_shares (community_id, sha256, channel_id) \
         SELECT $1, unnest($2::text[]), $3 \
         ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(sha256s)
    .bind(channel_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether `reader` may read every blob in `sha256s` (vacuously true if empty).
///
/// A blob is readable by its uploader (uploading the exact bytes proves
/// possession), by everyone when it is shared with the whole community or not
/// shared into any channel, and otherwise only by readers of a channel it is
/// shared into: an open channel or one they actively belong to. Shares are never
/// withdrawn when the referencing event is deleted, and ingest only stores a
/// reference whose author can already read the blob.
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
        "SELECT coalesce(bool_and( \
             EXISTS (SELECT 1 FROM media_uploaders u \
                     WHERE u.community_id = $1 AND u.sha256 = b.sha256 AND u.pubkey = $3) \
             OR EXISTS (SELECT 1 FROM media_shares s \
                        WHERE s.community_id = $1 AND s.sha256 = b.sha256 \
                          AND s.channel_id IS NULL) \
             OR NOT EXISTS (SELECT 1 FROM media_shares s \
                            WHERE s.community_id = $1 AND s.sha256 = b.sha256) \
             OR EXISTS (SELECT 1 FROM media_shares s \
                        JOIN channels c ON c.community_id = $1 AND c.id = s.channel_id \
                        WHERE s.community_id = $1 AND s.sha256 = b.sha256 \
                          AND c.deleted_at IS NULL \
                          AND (c.visibility = 'open' \
                               OR EXISTS (SELECT 1 FROM channel_members cm \
                                          WHERE cm.community_id = $1 AND cm.channel_id = c.id \
                                            AND cm.pubkey = $3 AND cm.removed_at IS NULL))) \
         ), true) \
         FROM (SELECT DISTINCT unnest($2::text[]) AS sha256) b",
    )
    .bind(community_id.as_uuid())
    .bind(sha256s)
    .bind(reader)
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
