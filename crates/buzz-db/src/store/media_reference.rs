//! Where an uploaded blob is referenced, for media read authorization.

use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Image extensions a video poster (`imeta image`) may carry; ingest rejects others.
const POSTER_EXTS: [&str; 4] = ["jpg", "png", "gif", "webp"];

/// Distinct `channel_id`s of every stored event, deleted or not, that attaches
/// the blob `sha256`; `None` is a channel-less event.
///
/// A blob is attached as an `imeta` file (`x <sha256>`, required by ingest), as
/// a video poster (`image <url>`, relative or on `media_base_url` as ingest
/// requires), or as a `link-preview` image or favicon hash. Containment is the
/// GIN-indexed prefilter; the positional check keeps the hash appearing in some
/// other field from counting as an attachment.
pub async fn get_blob_reference_channels(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
    media_base_url: &str,
) -> Result<Vec<Option<Uuid>>> {
    let base = media_base_url.trim_end_matches('/');
    let mut imeta_entries = vec![format!("x {sha256}")];
    for ext in POSTER_EXTS {
        imeta_entries.push(format!("image /media/{sha256}.{ext}"));
        imeta_entries.push(format!("image {base}/{sha256}.{ext}"));
    }
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
    let channels = sqlx::query_scalar(
        "SELECT DISTINCT channel_id FROM events \
         WHERE community_id = $1 \
           AND tags @> ANY($2::jsonb[]) \
           AND EXISTS (SELECT 1 FROM jsonb_array_elements(tags) AS tag \
                       WHERE (tag->>0 = 'imeta' AND tag ?| $3) \
                          OR (tag->>0 = 'link-preview' AND $4 IN (tag->>8, tag->>10)))",
    )
    .bind(community_id.as_uuid())
    .bind(containments)
    .bind(imeta_entries)
    .bind(sha256)
    .fetch_all(&mut *connection)
    .await?;
    Ok(channels)
}

/// Whether the blob `sha256` is a current profile avatar or in a live custom
/// emoji set, both of which every community member must be able to load.
///
/// Neither reference is indexed by hash, so callers should consult this only
/// after the channel references alone would refuse a read.
pub async fn is_blob_publicly_referenced(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
) -> Result<bool> {
    let pattern = format!("%{sha256}%");
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let referenced = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE community_id = $1 AND avatar_url LIKE $2) \
             OR EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND kind = $3 \
                        AND deleted_at IS NULL AND tags::text LIKE $2)",
    )
    .bind(community_id.as_uuid())
    .bind(pattern)
    .bind(buzz_core::kind::KIND_EMOJI_SET as i32)
    .fetch_one(&mut *connection)
    .await?;
    Ok(referenced)
}

impl Db {
    /// Channels the blob `sha256` is attached in; see [`get_blob_reference_channels`].
    #[datastore_span(name = "get_blob_reference_channels", system = "postgresql")]
    pub async fn get_blob_reference_channels(
        &self,
        community_id: CommunityId,
        sha256: &str,
        media_base_url: &str,
    ) -> Result<Vec<Option<Uuid>>> {
        get_blob_reference_channels(&self.pool, community_id, sha256, media_base_url).await
    }

    /// Whether the blob is an avatar or custom emoji; see [`is_blob_publicly_referenced`].
    #[datastore_span(name = "is_blob_publicly_referenced", system = "postgresql")]
    pub async fn is_blob_publicly_referenced(
        &self,
        community_id: CommunityId,
        sha256: &str,
    ) -> Result<bool> {
        is_blob_publicly_referenced(&self.pool, community_id, sha256).await
    }
}
