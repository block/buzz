//! Who has uploaded each blob, as proof of possession for sharing it.

use buzz_datastore_tracing::datastore_span;

use crate::deletion::ServingWriteLease;
use crate::{error::Result, observability, Db};
use buzz_core::CommunityId;

impl Db {
    /// Recover possession proof from retained server-authored upload audits.
    /// Scan a finite sequence range using audit_log's tenant-leading primary
    /// key; malformed or unattributed records never establish ownership.
    #[datastore_span(name = "backfill_media_uploaders", system = "postgresql")]
    pub(crate) async fn backfill_media_uploaders(
        &self,
        community: CommunityId,
        batch_size: i64,
    ) -> Result<u64> {
        let mut conn =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Maintenance)
                .await?;
        let upper: Option<i64> =
            sqlx::query_scalar("SELECT max(seq) FROM audit_log WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&mut *conn)
                .await?;
        drop(conn);
        let Some(upper) = upper else {
            return Ok(0);
        };
        let (mut cursor, mut added) = (0_i64, 0_u64);
        while cursor < upper {
            let mut tx = crate::begin_community_write_transaction(
                &self.pool,
                community,
                observability::WriterOperation::Maintenance,
            )
            .await?;
            let page: Vec<i64> = sqlx::query_scalar(
                "SELECT seq FROM audit_log WHERE community_id = $1 AND seq > $2 AND seq <= $3 \
                 ORDER BY seq LIMIT $4",
            )
            .bind(community.as_uuid())
            .bind(cursor)
            .bind(upper)
            .bind(batch_size)
            .fetch_all(tx.conn())
            .await?;
            let Some(&last) = page.last() else {
                break;
            };
            added += sqlx::query(
                "INSERT INTO media_uploaders (community_id, sha256, pubkey, created_at) \
                 SELECT community_id, object_id, actor_pubkey, created_at FROM audit_log \
                 WHERE community_id = $1 AND seq > $2 AND seq <= $3 \
                   AND action = 'media_uploaded' AND octet_length(actor_pubkey) = 32 \
                   AND object_id ~ '^[0-9a-f]{64}$' \
                 ORDER BY object_id, actor_pubkey ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(cursor)
            .bind(last)
            .execute(tx.conn())
            .await?
            .rows_affected();
            tx.commit().await?;
            cursor = last;
        }
        Ok(added)
    }

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
