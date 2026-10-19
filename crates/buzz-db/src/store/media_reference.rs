//! Where each media blob is shared, and media read authorization from it.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// SQL predicate: whether the reader `$3` may read the blob `b.sha256` in community `$1`.
///
/// Everyone may when the blob is shared with the whole community, and readers
/// of a channel it is shared into (open, or one they actively belong to). A
/// blob not yet shared into any channel is readable only by its uploader.
/// Shares are never withdrawn.
macro_rules! readable_sql {
    () => {
        "( \
    EXISTS (SELECT 1 FROM media_shares s \
               WHERE s.community_id = $1 AND s.sha256 = b.sha256 AND s.channel_id IS NULL) \
    OR (NOT EXISTS (SELECT 1 FROM media_shares s \
                    WHERE s.community_id = $1 AND s.sha256 = b.sha256 \
                      AND s.channel_id IS NOT NULL) \
        AND EXISTS (SELECT 1 FROM media_uploaders u \
                    WHERE u.community_id = $1 AND u.sha256 = b.sha256 AND u.pubkey = $3)) \
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

/// Whether `reader` may read the blob `sha256`; see `readable_sql`.
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

    /// Record the shares of every event already stored in an active community,
    /// deleted events included, `batch_size` events per transaction; returns the
    /// number of shares added.
    ///
    /// Shares come from the same rule as live writes. History predates the
    /// readability check, so its references are recorded as they stand.
    /// Idempotent and resumable: rerunning only fills shares still missing.
    #[datastore_span(name = "backfill_media_shares", system = "postgresql")]
    pub async fn backfill_media_shares(&self, batch_size: i64) -> Result<u64> {
        let communities: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM communities WHERE deleted_at IS NULL AND deletion_state = 'active' \
             ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut added = 0;
        for community in communities {
            let community = CommunityId::from_uuid(community);
            let mut cursor: Option<(DateTime<Utc>, Vec<u8>)> = None;
            loop {
                let rows = sqlx::query(
                    "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, \
                            channel_id \
                     FROM events \
                     WHERE community_id = $1 \
                       AND ($2::timestamptz IS NULL OR (created_at, id) > ($2, $3)) \
                     ORDER BY created_at, id LIMIT $4",
                )
                .bind(community.as_uuid())
                .bind(cursor.as_ref().map(|(at, _)| *at))
                .bind(
                    cursor
                        .as_ref()
                        .map(|(_, id)| id.clone())
                        .unwrap_or_default(),
                )
                .bind(batch_size)
                .fetch_all(&self.pool)
                .await?;
                let Some(last) = rows.last() else { break };
                cursor = Some((last.try_get("created_at")?, last.try_get("id")?));
                let full_batch = i64::try_from(rows.len()).unwrap_or(i64::MAX) == batch_size;

                let mut tx = crate::begin_community_write_transaction(
                    &self.pool,
                    community,
                    crate::observability::WriterOperation::Maintenance,
                )
                .await?;
                for row in rows {
                    let Some(stored) = crate::event::row_to_stored_event(row)? else {
                        continue;
                    };
                    let hashes = buzz_core::media::shared_blob_hashes(&stored.event);
                    if hashes.is_empty() {
                        continue;
                    }
                    added += sqlx::query(
                        "INSERT INTO media_shares (community_id, sha256, channel_id) \
                         SELECT $1, unnest($2::text[]), $3 ON CONFLICT DO NOTHING",
                    )
                    .bind(community.as_uuid())
                    .bind(&hashes)
                    .bind(stored.channel_id)
                    .execute(tx.conn())
                    .await?
                    .rows_affected();
                }
                tx.commit().await?;
                if !full_batch {
                    break;
                }
            }
        }
        Ok(added)
    }
}

#[cfg(test)]
mod postgres_tests {
    use std::time::Duration;

    use super::*;
    use crate::channel::{ChannelType, ChannelVisibility};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::PgPool;

    async fn fixture() -> (Db, CommunityId) {
        let db = Db::from_pool(
            PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("connect to test DB"),
        );
        let host = format!("media-shares-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("seed community")
            .id;
        (db, community)
    }

    async fn channel(
        db: &Db,
        community: CommunityId,
        owner: &Keys,
        visibility: ChannelVisibility,
    ) -> Uuid {
        db.create_channel(
            community,
            &format!("media-{}", Uuid::new_v4().simple()),
            ChannelType::Stream,
            visibility,
            None,
            &owner.public_key().to_bytes(),
            None,
        )
        .await
        .expect("create channel")
        .id
    }

    fn attachment(keys: &Keys, sha256: &str, channel: Option<Uuid>) -> nostr::Event {
        let mut tags = vec![Tag::parse(["imeta", &format!("url /media/{sha256}.jpg")]).unwrap()];
        if let Some(channel) = channel {
            tags.push(Tag::parse(["h", &channel.to_string()]).unwrap());
        }
        EventBuilder::new(Kind::from(9), "attachment")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    async fn shares(db: &Db, community: CommunityId, sha256: &str) -> Vec<Option<Uuid>> {
        let mut shares: Vec<Option<Uuid>> = sqlx::query_scalar(
            "SELECT channel_id FROM media_shares WHERE community_id = $1 AND sha256 = $2",
        )
        .bind(community.as_uuid())
        .bind(sha256)
        .fetch_all(&db.pool)
        .await
        .expect("read shares");
        shares.sort();
        shares
    }

    fn unique_sha() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    async fn upload(db: &Db, community: CommunityId, uploader: &Keys, sha256: &str) {
        let lease = db
            .deletion_store()
            .acquire_serving_write_lease(
                community,
                "media_upload",
                "media-shares-test",
                Duration::from_secs(60),
            )
            .await
            .expect("acquire lease");
        db.record_media_uploader(&lease, sha256, &uploader.public_key().to_bytes())
            .await
            .expect("record uploader");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stored_references_share_only_blobs_their_author_can_read() {
        let (db, community) = fixture().await;
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let sha = unique_sha();
        let private = channel(&db, community, &alice, ChannelVisibility::Private).await;
        let bobs = channel(&db, community, &bob, ChannelVisibility::Open).await;

        upload(&db, community, &alice, &sha).await;
        db.insert_event(
            community,
            &attachment(&alice, &sha, Some(private)),
            Some(private),
        )
        .await
        .expect("alice posts privately");
        assert_eq!(shares(&db, community, &sha).await, vec![Some(private)]);

        db.insert_event(community, &attachment(&bob, &sha, None), None)
            .await
            .expect("bob references the hash");
        assert_eq!(
            shares(&db, community, &sha).await,
            vec![Some(private)],
            "a hash alone must not share a blob its author cannot read"
        );

        upload(&db, community, &bob, &sha).await;
        db.insert_event(community, &attachment(&bob, &sha, Some(bobs)), Some(bobs))
            .await
            .expect("bob posts his copy");
        let mut expected = vec![Some(private), Some(bobs)];
        expected.sort();
        assert_eq!(
            shares(&db, community, &sha).await,
            expected,
            "an uploader may share their own bytes"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn backfill_records_shares_of_stored_events() {
        let (db, community) = fixture().await;
        let alice = Keys::generate();
        let sha = unique_sha();
        let private = channel(&db, community, &alice, ChannelVisibility::Private).await;
        db.insert_event(
            community,
            &attachment(&alice, &sha, Some(private)),
            Some(private),
        )
        .await
        .expect("alice posts privately");
        let mut tx = crate::begin_community_write_transaction(
            &db.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await
        .expect("admit");
        sqlx::query("DELETE FROM media_shares WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(tx.conn())
            .await
            .expect("forget shares");
        tx.commit().await.expect("commit");

        db.backfill_media_shares(100).await.expect("backfill");
        assert_eq!(shares(&db, community, &sha).await, vec![Some(private)]);
    }
}
