//! Which channels an uploaded blob is attached in.

use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Distinct `channel_id`s of every stored event, deleted or not, that attaches
/// the blob `sha256`; `None` is a channel-less event.
///
/// A blob is attached by a NIP-92 `imeta` tag carrying `x <sha256>` (ingest
/// rejects imeta without it) or by a `link-preview` tag carrying it as the image
/// or favicon hash. Containment is the GIN-indexed prefilter; the positional
/// check keeps a hash in some other tag from counting as an attachment.
pub async fn get_blob_reference_channels(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
) -> Result<Vec<Option<Uuid>>> {
    let imeta_entry = format!("x {sha256}");
    let containments = vec![
        serde_json::json!([["imeta", &imeta_entry]]),
        serde_json::json!([["link-preview", sha256]]),
    ];
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let channels = sqlx::query_scalar(
        "SELECT DISTINCT channel_id FROM events \
         WHERE community_id = $1 \
           AND tags @> ANY($2::jsonb[]) \
           AND EXISTS (SELECT 1 FROM jsonb_array_elements(tags) AS tag \
                       WHERE (tag->>0 = 'imeta' AND tag @> jsonb_build_array($3::text)) \
                          OR (tag->>0 = 'link-preview' AND tag @> jsonb_build_array($4::text)))",
    )
    .bind(community_id.as_uuid())
    .bind(containments)
    .bind(imeta_entry)
    .bind(sha256)
    .fetch_all(&mut *connection)
    .await?;
    Ok(channels)
}

impl Db {
    /// Channels the blob `sha256` is attached in; see [`get_blob_reference_channels`].
    #[datastore_span(name = "get_blob_reference_channels", system = "postgresql")]
    pub async fn get_blob_reference_channels(
        &self,
        community_id: CommunityId,
        sha256: &str,
    ) -> Result<Vec<Option<Uuid>>> {
        get_blob_reference_channels(&self.pool, community_id, sha256).await
    }
}
