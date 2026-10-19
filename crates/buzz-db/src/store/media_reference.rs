//! Media read authorization from where each blob is shared.

use buzz_datastore_tracing::datastore_span;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// SQL predicate: whether the reader `$3` may read the blob `b.sha256` in community `$1`.
///
/// Everyone may when the blob is shared with the whole community or into no
/// channel; otherwise only readers of a channel it is shared into, open or one
/// they actively belong to. Shares are never withdrawn.
macro_rules! readable_sql {
    () => {
        "( \
    EXISTS (SELECT 1 FROM media_shares s \
               WHERE s.community_id = $1 AND s.sha256 = b.sha256 AND s.channel_id IS NULL) \
    OR NOT EXISTS (SELECT 1 FROM media_shares s \
                   WHERE s.community_id = $1 AND s.sha256 = b.sha256 \
                     AND s.channel_id IS NOT NULL) \
    OR EXISTS (SELECT 1 FROM media_shares s \
               JOIN channels c ON c.community_id = $1 AND c.id = s.channel_id \
               WHERE s.community_id = $1 AND s.sha256 = b.sha256 AND c.deleted_at IS NULL \
                 AND (c.visibility = 'open' \
                      OR EXISTS (SELECT 1 FROM channel_members cm \
                                 WHERE cm.community_id = $1 AND cm.channel_id = c.id \
                                   AND cm.pubkey = $3 AND cm.removed_at IS NULL))))"
    };
}

/// Share into `channel_id` (or with the whole community when `None`) each blob
/// in `sha256s` that `author` can read or has uploaded (uploading the exact
/// bytes proves possession). Any other blob is left unshared, so a reference
/// built from a leaked hash shows it to no one new. Called with the
/// transaction that stores the referencing event.
pub(crate) async fn record_media_shares(
    conn: &mut PgConnection,
    community_id: CommunityId,
    sha256s: &[String],
    author: &[u8],
    channel_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(concat!(
        "INSERT INTO media_shares (community_id, sha256, channel_id) \
         SELECT $1, b.sha256, $4 FROM (SELECT DISTINCT unnest($2::text[]) AS sha256) b \
         WHERE EXISTS (SELECT 1 FROM media_uploaders u \
                       WHERE u.community_id = $1 AND u.sha256 = b.sha256 AND u.pubkey = $3) \
            OR ",
        readable_sql!(),
        " ON CONFLICT DO NOTHING"
    ))
    .bind(community_id.as_uuid())
    .bind(sha256s)
    .bind(author)
    .bind(channel_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether `reader` may read the blob `sha256`.
pub async fn can_read_blob(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
    reader: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let readable = sqlx::query_scalar(concat!(
        "SELECT ",
        readable_sql!(),
        " FROM (SELECT $2::text AS sha256) b"
    ))
    .bind(community_id.as_uuid())
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
