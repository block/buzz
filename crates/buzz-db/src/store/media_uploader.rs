//! Who has uploaded each blob, as proof of possession for media reads.

use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Record that `pubkey` uploaded the blob `sha256`; repeat uploads are no-ops.
pub async fn record_media_uploader(
    pool: &PgPool,
    community_id: CommunityId,
    sha256: &str,
    pubkey: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO media_uploaders (community_id, sha256, pubkey) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(sha256)
    .bind(pubkey)
    .execute(pool)
    .await?;
    Ok(())
}

impl Db {
    /// See [`record_media_uploader`].
    #[datastore_span(name = "record_media_uploader", system = "postgresql")]
    pub async fn record_media_uploader(
        &self,
        community_id: CommunityId,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<()> {
        record_media_uploader(&self.pool, community_id, sha256, pubkey).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use uuid::Uuid;

    const SHA: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    async fn community(db: &Db) -> CommunityId {
        let host = format!("uploader-{}.example", Uuid::new_v4().simple());
        db.ensure_configured_community(&host)
            .await
            .expect("seed community")
            .id
    }

    async fn uploader_rows(db: &Db, community_id: CommunityId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM media_uploaders WHERE community_id = $1")
            .bind(community_id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count uploaders")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn repeat_upload_records_one_row() {
        let db = Db::from_pool(
            PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("connect to test DB"),
        );
        let community_id = community(&db).await;
        for _ in 0..2 {
            db.record_media_uploader(community_id, SHA, &[7; 32])
                .await
                .expect("record uploader");
        }
        assert_eq!(uploader_rows(&db, community_id).await, 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_community_refuses_new_uploaders() {
        let db = Db::from_pool(
            PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("connect to test DB"),
        );
        let community_id = community(&db).await;
        crate::test_support::quiesce_community_for_tests(&db.pool, community_id).await;
        assert!(db
            .record_media_uploader(community_id, SHA, &[7; 32])
            .await
            .is_err());
        assert_eq!(uploader_rows(&db, community_id).await, 0);
    }
}
