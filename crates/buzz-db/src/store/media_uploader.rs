//! Who has uploaded each blob, as proof of possession for sharing it.

use buzz_datastore_tracing::datastore_span;

use crate::deletion::ServingWriteLease;
use crate::{error::Result, observability, Db};

impl Db {
    /// Record that `pubkey` uploaded the blob `sha256`; repeat uploads are
    /// no-ops. The write presents the upload's serving lease, so it may finish
    /// while the community quiesces but never after the lease is gone.
    #[datastore_span(name = "record_media_uploader", system = "postgresql")]
    pub async fn record_media_uploader(
        &self,
        lease: &ServingWriteLease,
        sha256: &str,
        pubkey: &[u8],
    ) -> Result<()> {
        let connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::EventWrite)
                .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        self.deletion_store()
            .guard_transaction_with_serving_lease(&mut tx, lease)
            .await?;
        sqlx::query(
            "INSERT INTO media_uploaders (community_id, sha256, pubkey) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(lease.community_id.as_uuid())
        .bind(sha256)
        .bind(pubkey)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod postgres_tests {
    use std::time::Duration;

    use super::*;
    use buzz_core::CommunityId;
    use sqlx::PgPool;
    use uuid::Uuid;

    const SHA: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    async fn db_with_lease() -> (Db, CommunityId, ServingWriteLease) {
        let db = Db::from_pool(
            PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("connect to test DB"),
        );
        let host = format!("uploader-{}.example", Uuid::new_v4().simple());
        let community_id = db
            .ensure_configured_community(&host)
            .await
            .expect("seed community")
            .id;
        let lease = db
            .deletion_store()
            .acquire_serving_write_lease(
                community_id,
                "media_upload",
                "media-uploader-test",
                Duration::from_secs(60),
            )
            .await
            .expect("acquire lease");
        (db, community_id, lease)
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
        let (db, community_id, lease) = db_with_lease().await;
        for _ in 0..2 {
            db.record_media_uploader(&lease, SHA, &[7; 32])
                .await
                .expect("record uploader");
        }
        assert_eq!(uploader_rows(&db, community_id).await, 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn released_lease_refuses_the_write() {
        let (db, community_id, lease) = db_with_lease().await;
        db.deletion_store()
            .release_serving_write_lease(&lease)
            .await
            .expect("release lease");
        assert!(db
            .record_media_uploader(&lease, SHA, &[7; 32])
            .await
            .is_err());
        assert_eq!(uploader_rows(&db, community_id).await, 0);
    }
}
