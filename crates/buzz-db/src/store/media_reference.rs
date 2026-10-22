//! Where each media blob is shared, and media read authorization from it.

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::{error::Result, Db};
use buzz_core::CommunityId;

/// Both the reference and its edited message must survive. The target lookup
/// uses its recorded timestamp to reach the original event's partition.
macro_rules! live_source_sql {
    () => {
        "e.deleted_at IS NULL AND (e.kind <> 40003 OR EXISTS ( \
           SELECT 1 FROM events target \
           WHERE target.community_id = s.community_id AND target.id = s.target_event_id \
             AND target.created_at = s.target_event_created_at AND target.deleted_at IS NULL \
             AND target.kind <> 40003 AND target.channel_id IS NOT DISTINCT FROM e.channel_id))"
    };
}
pub(crate) use live_source_sql;

/// SQL predicate: whether the reader `$3` may read the blob `b.sha256` in community `$1`.
///
/// A share admits everyone when it is community-wide, or readers of its
/// channel (open, or one they actively belong to). An unknown or unshared hash
/// grants nothing. Deleted or purged source events grant nothing.
macro_rules! readable_sql {
    () => {
        concat!(
            "( \
    EXISTS (SELECT 1 FROM media_shares s \
               JOIN events e ON e.community_id = s.community_id AND e.id = s.event_id \
                 AND e.created_at = s.event_created_at AND ",
            crate::store::media_reference::live_source_sql!(),
            " \
               WHERE s.community_id = $1 AND s.sha256 = b.sha256 AND s.channel_id IS NULL) \
    OR EXISTS (SELECT 1 FROM media_shares s \
               JOIN events e ON e.community_id = s.community_id AND e.id = s.event_id \
                 AND e.created_at = s.event_created_at AND ",
            crate::store::media_reference::live_source_sql!(),
            " \
               JOIN channels c ON c.community_id = $1 AND c.id = s.channel_id \
               WHERE s.community_id = $1 AND s.sha256 = b.sha256 AND c.deleted_at IS NULL \
                 AND (c.visibility = 'open' \
                      OR EXISTS (SELECT 1 FROM channel_members cm \
                                 WHERE cm.community_id = $1 AND cm.channel_id = c.id \
                                   AND cm.pubkey = $3 AND cm.removed_at IS NULL))))"
        )
    };
}

/// Share into `channel_id` (or with the whole community when `None`) each blob
/// in `sha256s` that the event's author can read or has uploaded (uploading
/// the exact bytes proves possession). An agent's profile picture may also
/// use its registered owner's upload proof. Any other blob is left unshared,
/// so a reference built from a leaked hash shows it to no one new. Called with the
/// transaction that stores the referencing event; returns the shares added.
pub(crate) async fn record_media_shares(
    conn: &mut PgConnection,
    community_id: CommunityId,
    sha256s: &[String],
    event: &nostr::Event,
    channel_id: Option<Uuid>,
) -> Result<u64> {
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(crate::DbError::InvalidTimestamp(created_at_secs))?;
    let target = super::media_grant::edit_target(conn, community_id, event, channel_id).await?;
    if event.kind.as_u16() == 40003 && target.is_none() {
        return Ok(0);
    }
    let added = sqlx::query(concat!(
        "INSERT INTO media_shares (community_id, sha256, channel_id, event_id, event_created_at, \
                                  target_event_id, target_event_created_at) \
         SELECT $1, b.sha256, $4, $6, $7, $8, $9 FROM (SELECT DISTINCT unnest($2::text[]) AS sha256) b \
         WHERE (EXISTS (SELECT 1 FROM media_uploaders u \
                       WHERE u.community_id = $1 AND u.sha256 = b.sha256 AND u.pubkey = $3) \
            OR (b.sha256 = $5 AND EXISTS ( \
                SELECT 1 FROM users agent JOIN media_uploaders u \
                  ON u.community_id = agent.community_id AND u.pubkey = agent.agent_owner_pubkey \
                WHERE agent.community_id = $1 AND agent.pubkey = $3 AND u.sha256 = b.sha256)) \
            OR ",
        readable_sql!(),
        ") AND EXISTS (SELECT 1 FROM events e \
                       WHERE e.community_id = $1 AND e.id = $6 AND e.created_at = $7 \
                         AND e.deleted_at IS NULL) \
         ORDER BY b.sha256 ON CONFLICT DO NOTHING"
    ))
    .bind(community_id.as_uuid())
    .bind(sha256s)
    .bind(event.pubkey.as_bytes().as_slice())
    .bind(channel_id)
    .bind(buzz_core::media::profile_picture_blob_hash(event))
    .bind(event.id.as_bytes().as_slice())
    .bind(created_at)
    .bind(target.as_ref().map(|(id, _)| id))
    .bind(target.as_ref().map(|(_, at)| *at))
    .execute(conn)
    .await?
    .rows_affected();
    Ok(added)
}

/// Whether `reader` has a readable share, or uploaded a blob not yet shared
/// into any channel. Upload proof alone does not bypass existing channel shares.
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
        " OR (NOT EXISTS (SELECT 1 FROM media_shares s \
                          WHERE s.community_id = $1 AND s.sha256 = b.sha256 \
                            AND s.channel_id IS NOT NULL) \
              AND EXISTS (SELECT 1 FROM media_uploaders u \
                          WHERE u.community_id = $1 AND u.sha256 = b.sha256 AND u.pubkey = $3)) \
          FROM (SELECT $2::text AS sha256) b"
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

    /// Record the shares of undeleted events stored in an active community,
    /// reading `batch_size` events at a time; returns
    /// the number of shares added.
    ///
    /// Shares come from the same rule as live writes. Events received before
    /// share recording was installed predate the readability check, so their
    /// references are recorded as they stand; later events get the live,
    /// checked recording using the author's current permissions. Earlier refusals
    /// are not remembered: any run can grant old references after access changes.
    /// Rows are deduplicated, but authorization decisions are not preserved.
    #[datastore_span(name = "backfill_media_shares", system = "postgresql")]
    pub async fn backfill_media_shares(&self, batch_size: i64) -> Result<u64> {
        if batch_size <= 0 {
            return Err(crate::DbError::InvalidData(
                "batch size must be positive".into(),
            ));
        }
        let communities: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, media_shares_recording_started_at FROM communities \
             WHERE deleted_at IS NULL AND deletion_state = 'active' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut added = 0;
        for (community, installed) in communities {
            let community = CommunityId::from_uuid(community);
            let mut cursor: Option<(DateTime<Utc>, Vec<u8>)> = None;
            loop {
                let rows = sqlx::query(
                    "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, \
                            channel_id \
                     FROM events \
                     WHERE community_id = $1 AND deleted_at IS NULL \
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

                let mut history = Vec::new();
                let mut recorded_live = Vec::new();
                for row in rows {
                    let received_at: DateTime<Utc> = row.try_get("received_at")?;
                    let Some(stored) = crate::event::row_to_stored_event(row)? else {
                        continue;
                    };
                    let hashes = buzz_core::media::shared_blob_hashes(&stored.event);
                    if hashes.is_empty() {
                        continue;
                    }
                    if received_at < installed {
                        history.push(stored);
                    } else {
                        recorded_live.push((stored.event, hashes, stored.channel_id));
                    }
                }
                if !history.is_empty() {
                    let mut tx = self.begin_media_share_transaction(community).await?;
                    added +=
                        super::media_grant::record_history(tx.conn(), community, &history).await?;
                    tx.commit().await?;
                }
                for (event, hashes, channel) in recorded_live {
                    let mut tx = self.begin_media_share_transaction(community).await?;
                    added +=
                        record_media_shares(tx.conn(), community, &hashes, &event, channel).await?;
                    tx.commit().await?;
                }
                if !full_batch {
                    break;
                }
            }
        }
        Ok(added)
    }

    async fn begin_media_share_transaction(
        &self,
        community: CommunityId,
    ) -> Result<crate::AdmittedTx> {
        crate::begin_community_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await
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
        EventBuilder::new(Kind::from(9), format!("attachment {}", Uuid::new_v4()))
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    async fn shares(db: &Db, community: CommunityId, sha256: &str) -> Vec<Option<Uuid>> {
        let mut shares: Vec<Option<Uuid>> = sqlx::query_scalar(
            "SELECT DISTINCT channel_id FROM media_shares WHERE community_id = $1 AND sha256 = $2",
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

        for uploader_present in [false, true] {
            if uploader_present {
                upload(&db, community, &alice, &sha).await;
            }
            db.insert_event(community, &attachment(&bob, &sha, None), None)
                .await
                .expect("bob references an unshared hash");
            assert!(
                shares(&db, community, &sha).await.is_empty(),
                "a hash alone grants nothing, even before its uploader posts it"
            );
        }
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
    async fn backfill_trusts_history_and_rechecks_recorded_events() {
        check_backfill().await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn deleting_a_reference_revokes_only_its_own_media_grant() {
        let (db, community) = fixture().await;
        let (owner, reader) = (Keys::generate(), Keys::generate());
        let open = channel(&db, community, &owner, ChannelVisibility::Open).await;
        for channel_id in [Some(open), None] {
            let sha = unique_sha();
            upload(&db, community, &owner, &sha).await;
            let first = attachment(&owner, &sha, channel_id);
            let last = attachment(&owner, &sha, channel_id);
            for event in [&first, &last] {
                db.insert_event(community, event, channel_id).await.unwrap();
            }
            let sources: Vec<Vec<u8>> = sqlx::query_scalar(
                "SELECT event_id FROM media_shares WHERE community_id = $1 AND sha256 = $2",
            )
            .bind(community.as_uuid())
            .bind(&sha)
            .fetch_all(&db.pool)
            .await
            .unwrap();
            assert_eq!(sources.len(), 2, "each message must retain its own grant");
            for event in [&first, &last] {
                assert!(sources.contains(&event.id.to_bytes().to_vec()));
            }
            let edit_only = if let Some(channel) = channel_id {
                let edit_sha = unique_sha();
                upload(&db, community, &owner, &edit_sha).await;
                for target in [&first, &last] {
                    let edit = EventBuilder::new(
                        Kind::from(40003),
                        format!("retained /media/{sha}, added /media/{edit_sha}"),
                    )
                    .tags([
                        Tag::parse(["h", &channel.to_string()]).unwrap(),
                        Tag::parse(["e", &target.id.to_hex()]).unwrap(),
                    ])
                    .sign_with_keys(&owner)
                    .unwrap();
                    db.insert_event(community, &edit, channel_id).await.unwrap();
                    // The backfill must also reject surviving legacy edits
                    // once their logical message has disappeared.
                    let mut tx = db.begin_media_share_transaction(community).await.unwrap();
                    sqlx::query(
                        "UPDATE events SET received_at = (SELECT media_shares_recording_started_at \
                         FROM communities WHERE id = $1) - interval '1 second' \
                         WHERE community_id = $1 AND id = $2",
                    )
                    .bind(community.as_uuid())
                    .bind(edit.id.as_bytes().as_slice())
                    .execute(tx.conn())
                    .await
                    .unwrap();
                    tx.commit().await.unwrap();
                }
                assert!(readable_share(&db, community, &edit_sha, &reader).await);
                Some(edit_sha)
            } else {
                None
            };
            assert!(readable_share(&db, community, &sha, &reader).await);
            for (event, still_shared) in [(&first, true), (&last, false)] {
                assert!(
                    db.soft_delete_event_and_update_thread(
                        community,
                        event.id.as_bytes(),
                        None,
                        None,
                    )
                    .await
                    .unwrap()
                );
                assert_eq!(
                    readable_share(&db, community, &sha, &reader).await,
                    still_shared,
                    "only an undeleted source message can grant access"
                );
                if let Some(edit_sha) = &edit_only {
                    assert_eq!(
                        readable_share(&db, community, edit_sha, &reader).await,
                        still_shared,
                        "edit-only attachments must share the target message's lifetime"
                    );
                }
            }
            db.insert_event(community, &attachment(&reader, &sha, None), None)
                .await
                .unwrap();
            assert!(!readable_share(&db, community, &sha, &reader).await);
            db.backfill_media_shares(1).await.unwrap();
            assert!(!readable_share(&db, community, &sha, &reader).await);
            if let Some(edit_sha) = &edit_only {
                assert!(!readable_share(&db, community, edit_sha, &reader).await);
            }

            let purged = attachment(&owner, &sha, channel_id);
            db.insert_event(community, &purged, channel_id)
                .await
                .unwrap();
            assert!(readable_share(&db, community, &sha, &reader).await);
            let mut tx = db.begin_media_share_transaction(community).await.unwrap();
            sqlx::query("DELETE FROM events WHERE community_id = $1 AND id = $2")
                .bind(community.as_uuid())
                .bind(purged.id.as_bytes().as_slice())
                .execute(tx.conn())
                .await
                .unwrap();
            tx.commit().await.unwrap();
            assert!(
                !readable_share(&db, community, &sha, &reader).await,
                "a purged source cannot leave a dangling grant"
            );
            assert!(reap_all(&db).await > 0);
            assert_eq!(
                shares(&db, community, &sha).await.len(),
                usize::from(channel_id.is_some()),
                "cleanup retains at most one channel boundary, never public history"
            );
            if let Some(edit_sha) = &edit_only {
                assert_eq!(
                    shares(&db, community, edit_sha).await.len(),
                    1,
                    "inactive edit grants compact to one channel marker"
                );
            }
            assert!(!readable_share(&db, community, &sha, &reader).await);
            assert_eq!(reap_all(&db).await, 0, "cleanup is idempotent");
        }

        // Fleet cleanup skips a quiescing community while reclaiming another
        // community's dead copy of the same hash and keeping its live copy.
        let sha = unique_sha();
        upload(&db, community, &owner, &sha).await;
        let retired = attachment(&owner, &sha, None);
        db.insert_event(community, &retired, None).await.unwrap();
        db.soft_delete_event_and_update_thread(community, retired.id.as_bytes(), None, None)
            .await
            .unwrap();
        let mut lifecycle = db.pool.begin().await.unwrap();
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', '0', true)",
        )
        .bind(community.as_uuid().to_string())
        .execute(&mut *lifecycle)
        .await
        .unwrap();
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&mut *lifecycle)
            .await
            .unwrap();
        lifecycle.commit().await.unwrap();
        let (other, other_community) = fixture().await;
        upload(&other, other_community, &owner, &sha).await;
        let old = attachment(&owner, &sha, None);
        let current = attachment(&owner, &sha, None);
        for event in [&old, &current] {
            other
                .insert_event(other_community, event, None)
                .await
                .unwrap();
        }
        other
            .soft_delete_event_and_update_thread(other_community, old.id.as_bytes(), None, None)
            .await
            .unwrap();
        assert!(reap_all(&db).await > 0);
        assert_eq!(
            shares(&db, community, &sha).await.len(),
            1,
            "cleanup may not write into a quiescing community"
        );
        assert_eq!(shares(&other, other_community, &sha).await.len(), 1);
        assert!(readable_share(&other, other_community, &sha, &reader).await);
    }

    async fn readable_share(db: &Db, community: CommunityId, sha: &str, reader: &Keys) -> bool {
        sqlx::query_scalar(concat!(
            "SELECT ",
            readable_sql!(),
            " FROM (SELECT $2::text AS sha256) b"
        ))
        .bind(community.as_uuid())
        .bind(sha)
        .bind(reader.public_key().to_bytes().to_vec())
        .fetch_one(&db.pool)
        .await
        .unwrap()
    }

    async fn reap_all(db: &Db) -> u64 {
        let mut cursor = None;
        let mut removed = 0;
        // One row per page exercises progress across live rows and retained
        // channel markers as well as rows deleted by a previous page.
        for _ in 0..10_000 {
            let batch = db
                .reap_inactive_media_shares(cursor.as_ref(), 1)
                .await
                .unwrap();
            removed += batch.removed;
            cursor = batch.next_cursor;
            if cursor.is_none() {
                return removed;
            }
        }
        panic!("cleanup must finish its bounded primary-key scan");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replacement_preserves_only_the_media_it_continues_to_reference() {
        let (db, community) = fixture().await;
        let viewer = Keys::generate();
        for kind in [Kind::Metadata, Kind::from(30030)] {
            let (uploader, publisher) = (Keys::generate(), Keys::generate());
            let sha = unique_sha();
            upload(&db, community, &uploader, &sha).await;
            let base = nostr::Timestamp::now().as_secs();
            let make_for = |sha: &str, keys: &Keys, kind: Kind, step: u64, picture: bool| {
                let url = if picture {
                    format!("/media/{sha}.jpg")
                } else {
                    String::new()
                };
                let mut tags = Vec::new();
                let content = if kind == Kind::Metadata {
                    serde_json::json!({"picture": url, "name": format!("profile-{step}")})
                        .to_string()
                } else {
                    tags.push(Tag::parse(["d", "media-continuity"]).unwrap());
                    if picture {
                        tags.push(Tag::parse(["emoji", "sample", &url]).unwrap());
                    }
                    String::new()
                };
                EventBuilder::new(kind, content)
                    .tags(tags)
                    .custom_created_at(nostr::Timestamp::from_secs(base + step))
                    .sign_with_keys(keys)
                    .unwrap()
            };
            let make = |keys: &Keys, kind: Kind, step: u64, picture: bool| {
                make_for(&sha, keys, kind, step, picture)
            };
            let publish = async |event: &nostr::Event| {
                if event.kind == Kind::Metadata {
                    db.replace_addressable_event(community, event, None)
                        .await
                        .unwrap()
                        .1
                } else {
                    db.replace_parameterized_event(community, event, "media-continuity", None)
                        .await
                        .unwrap()
                        .1
                }
            };
            assert!(publish(&make(&uploader, Kind::Metadata, 0, true)).await);
            assert!(publish(&make(&publisher, kind, 0, true)).await);
            assert!(publish(&make(&uploader, Kind::Metadata, 1, false)).await);
            assert!(readable_share(&db, community, &sha, &viewer).await);
            assert!(publish(&make(&publisher, kind, 1, true)).await);
            assert!(reap_all(&db).await > 0);
            assert_eq!(
                shares(&db, community, &sha).await.len(),
                1,
                "cleanup keeps the surviving profile/emoji head"
            );
            assert!(
                readable_share(&db, community, &sha, &viewer).await,
                "a replacement must carry forward an authorized reference"
            );
            assert!(publish(&make(&publisher, kind, 2, false)).await);
            assert!(!readable_share(&db, community, &sha, &viewer).await);
            assert!(publish(&make(&publisher, kind, 3, true)).await);
            assert!(
                !readable_share(&db, community, &sha, &viewer).await,
                "a later replacement cannot revive a previously removed grant"
            );

            // An unbackfilled legacy head has neither proof nor shares. A
            // normal name-only update must retain the eligible old reference.
            let legacy_keys = Keys::generate();
            let legacy_sha = unique_sha();
            let legacy_make =
                |step, picture| make_for(&legacy_sha, &legacy_keys, kind, step, picture);
            let legacy = legacy_make(0, true);
            assert!(publish(&legacy).await);
            assert!(!readable_share(&db, community, &legacy_sha, &viewer).await);
            let mut tx = db.begin_media_share_transaction(community).await.unwrap();
            sqlx::query(
                "UPDATE events SET received_at = (SELECT media_shares_recording_started_at \
                 FROM communities WHERE id = $1) - interval '1 second' \
                 WHERE community_id = $1 AND id = $2",
            )
            .bind(community.as_uuid())
            .bind(legacy.id.as_bytes().as_slice())
            .execute(tx.conn())
            .await
            .unwrap();
            tx.commit().await.unwrap();
            assert!(publish(&legacy_make(1, true)).await);
            assert!(reap_all(&db).await > 0);
            assert_eq!(shares(&db, community, &legacy_sha).await.len(), 1);
            db.backfill_media_shares(1).await.unwrap();
            assert!(
                readable_share(&db, community, &legacy_sha, &viewer).await,
                "a retained legacy avatar/emoji must survive replacement before backfill"
            );
            assert!(publish(&legacy_make(2, false)).await);
            assert!(!readable_share(&db, community, &legacy_sha, &viewer).await);
            assert!(publish(&legacy_make(3, true)).await);
            assert!(!readable_share(&db, community, &legacy_sha, &viewer).await);
            assert!(reap_all(&db).await > 0);
            assert!(
                shares(&db, community, &legacy_sha).await.is_empty(),
                "replaced avatar/emoji heads must not accumulate inactive rows"
            );
            assert!(shares(&db, community, &sha).await.is_empty());
        }
    }

    async fn check_backfill() {
        let (db, community) = fixture().await;
        let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (sha, historical, deleted_history) = (unique_sha(), unique_sha(), unique_sha());
        let private = channel(&db, community, &alice, ChannelVisibility::Private).await;
        upload(&db, community, &alice, &sha).await;
        db.insert_event(
            community,
            &attachment(&alice, &sha, Some(private)),
            Some(private),
        )
        .await
        .expect("alice posts privately");
        db.insert_event(community, &attachment(&bob, &sha, None), None)
            .await
            .expect("bob references the hash");
        let old = attachment(&carol, &historical, None);
        db.insert_event(community, &old, None)
            .await
            .expect("carol posts before share recording");
        let deleted = attachment(&carol, &deleted_history, None);
        db.insert_event(community, &deleted, None).await.unwrap();
        db.soft_delete_event_and_update_thread(community, deleted.id.as_bytes(), None, None)
            .await
            .unwrap();

        let mut tx = crate::begin_community_write_transaction(
            &db.pool,
            community,
            crate::observability::WriterOperation::Maintenance,
        )
        .await
        .expect("admit");
        sqlx::query(
            "UPDATE events SET received_at = 'epoch' WHERE community_id = $1 AND id = ANY($2)",
        )
        .bind(community.as_uuid())
        .bind(vec![
            old.id.to_bytes().to_vec(),
            deleted.id.to_bytes().to_vec(),
        ])
        .execute(tx.conn())
        .await
        .expect("age carol's event");
        sqlx::query("DELETE FROM media_shares WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(tx.conn())
            .await
            .expect("forget shares");
        tx.commit().await.expect("commit");

        db.backfill_media_shares(1).await.expect("backfill");
        assert_eq!(db.backfill_media_shares(1).await.expect("rerun"), 0);
        assert_eq!(
            shares(&db, community, &sha).await,
            vec![Some(private)],
            "a reference live recording refused must stay unshared"
        );
        assert_eq!(shares(&db, community, &historical).await, vec![None]);
        assert!(
            shares(&db, community, &deleted_history).await.is_empty(),
            "legacy deleted messages must never gain a share"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn migration_schema_backfill_trusts_history_and_rechecks_recorded_events() {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to migration DB");
        crate::migration::run_migrations_through(&pool, 68)
            .await
            .expect("migrate");
        pool.close().await;
        check_backfill().await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn agent_profile_picture_uses_only_its_registered_owners_upload_proof() {
        let (db, community) = fixture().await;
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (avatar, secret, unknown) = (unique_sha(), unique_sha(), unique_sha());
        for sha in [&avatar, &secret] {
            upload(&db, community, &owner, sha).await;
        }
        let profile = |picture: &str| {
            EventBuilder::new(
                Kind::Metadata,
                serde_json::json!({
                    "name": Uuid::new_v4().to_string(),
                    "picture": format!("/media/{picture}.jpg"),
                    "about": format!("/media/{secret}.jpg"),
                })
                .to_string(),
            )
            .tags([Tag::parse(["p", &owner.public_key().to_hex()]).unwrap()])
            .sign_with_keys(&agent)
            .unwrap()
        };
        db.insert_event(community, &profile(&avatar), None)
            .await
            .unwrap();
        assert!(
            shares(&db, community, &avatar).await.is_empty(),
            "an unverified owner tag grants nothing"
        );
        for keys in [&owner, &agent] {
            db.ensure_user(community, &keys.public_key().to_bytes())
                .await
                .unwrap();
        }
        db.set_agent_owner(
            community,
            &agent.public_key().to_bytes(),
            &owner.public_key().to_bytes(),
        )
        .await
        .unwrap();
        db.insert_event(community, &profile(&avatar), None)
            .await
            .unwrap();
        assert_eq!(shares(&db, community, &avatar).await, vec![None]);
        assert!(
            shares(&db, community, &secret).await.is_empty(),
            "owner proof applies only to picture"
        );
        db.insert_event(community, &profile(&unknown), None)
            .await
            .unwrap();
        assert!(
            shares(&db, community, &unknown).await.is_empty(),
            "the owner must have uploaded these bytes"
        );
        db.insert_event(community, &attachment(&agent, &secret, None), None)
            .await
            .unwrap();
        assert!(
            shares(&db, community, &secret).await.is_empty(),
            "ordinary events cannot borrow the owner's uploads"
        );
    }
}
