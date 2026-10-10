//! Force both serialization orders through supported lifecycle writers. Lock
//! observations, not sleeps or task start order, establish the schedule.

use std::time::Duration;

use buzz_core::channel::MemberRole;

use super::*;

#[derive(Clone, Copy, Debug)]
enum Transition {
    RemoveAdmin,
    Archive,
    Delete,
}

impl Transition {
    async fn apply(
        self,
        db: &Db,
        community: CommunityId,
        channel: Uuid,
        admin: &Keys,
        relay: &Keys,
    ) {
        match self {
            Self::RemoveAdmin => db
                .remove_member(
                    community,
                    channel,
                    admin.public_key().as_bytes(),
                    admin.public_key().as_bytes(),
                )
                .await
                .unwrap(),
            Self::Archive => db.archive_channel(community, channel).await.unwrap(),
            Self::Delete => {
                assert!(db.soft_delete_channel(community, channel).await.unwrap());
                db.soft_delete_discovery_events(community, channel, relay.public_key().as_bytes())
                    .await
                    .unwrap();
            }
        }
    }
}

async fn admin(db: &Db, community: CommunityId, channel: Uuid, owner: &Keys) -> Keys {
    let admin = Keys::generate();
    db.add_member(
        community,
        channel,
        admin.public_key().as_bytes(),
        MemberRole::Admin,
        Some(owner.public_key().as_bytes()),
    )
    .await
    .unwrap();
    admin
}

async fn blocked_pid(db: &Db, blocker: i32) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let pid: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
                 AND $1 = ANY(pg_blocking_pids(pid)) ORDER BY pid LIMIT 1",
            )
            .bind(blocker)
            .fetch_optional(db.pool())
            .await
            .unwrap();
            if let Some(pid) = pid {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supported writer must actually wait behind the selected lock")
}

async fn assert_rejected_without_effects(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    relay: &Keys,
    event: &Event,
    expected_evidence: Option<bool>,
) {
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    let labels = write.labels().clone();
    let head = write.previous.as_ref().map(|stored| stored.event.id);
    assert_eq!(
        write.apply_command(event, Some(&fields())).await.unwrap(),
        CommandApplication::Restricted
    );
    assert_eq!(write.labels(), &labels);
    assert_eq!(write.previous.as_ref().map(|stored| stored.event.id), head);
    assert!(write.commit().await.unwrap().is_none());
    assert_eq!(evidence(db, community, event).await, expected_evidence);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_commit_precedes_waiting_removal_archive_and_deletion() {
    let (db, community) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    for transition in [
        Transition::RemoveAdmin,
        Transition::Archive,
        Transition::Delete,
    ] {
        let channel = Uuid::new_v4();
        let create = command(&owner, channel, 9007, &[("label", "original")]);
        apply(&db, community, &relay, &create).await;
        let admin = admin(&db, community, channel, &owner).await;
        let event = command(&admin, channel, 9002, &[("add-label", "committed")]);
        let mut write = db
            .begin_channel_metadata_write(community, channel, relay.public_key())
            .await
            .unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(write.tx.conn())
            .await
            .unwrap();
        let finish = async {
            blocked_pid(&db, pid).await;
            assert_eq!(
                write.apply_command(&event, None).await.unwrap(),
                CommandApplication::Applied
            );
            snapshot(&mut write, &relay).await;
            assert!(write.commit().await.unwrap().is_some());
        };
        tokio::join!(
            transition.apply(&db, community, channel, &admin, &relay),
            finish
        );
        assert_eq!(evidence(&db, community, &event).await, Some(true));
        assert_rejected_without_effects(&db, community, channel, &relay, &event, Some(true)).await;
        let new = command(&admin, channel, 9002, &[("add-label", "denied")]);
        assert_rejected_without_effects(&db, community, channel, &relay, &new, None).await;
        let mut write = db
            .begin_channel_metadata_write(community, channel, relay.public_key())
            .await
            .unwrap();
        assert_eq!(write.labels().values(), &["committed", "original"]);
        if matches!(transition, Transition::Delete) {
            assert!(write.previous.is_none());
            assert!(write.needs_snapshot().await.is_err());
        }
        write.rollback().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn metadata_queued_after_removal_archive_and_deletion_rechecks_state() {
    let (db, community) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    for transition in [
        Transition::RemoveAdmin,
        Transition::Archive,
        Transition::Delete,
    ] {
        let channel = Uuid::new_v4();
        let create = command(&owner, channel, 9007, &[("label", "original")]);
        apply(&db, community, &relay, &create).await;
        let admin = admin(&db, community, channel, &owner).await;
        let prior = command(&admin, channel, 9002, &[("add-label", "original")]);
        apply(&db, community, &relay, &prior).await;
        let event = command(&admin, channel, 9002, &[("add-label", "denied")]);

        // Test-only latch stops the real writer after it acquired its authority
        // lock (removal) or queued for the channel row (archive/deletion).
        let mut latch = db.pool().begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *latch)
            .await
            .unwrap();
        match transition {
            Transition::RemoveAdmin => {
                sqlx::query("SELECT 1 FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 FOR UPDATE")
                    .bind(community.as_uuid()).bind(channel).bind(admin.public_key().as_bytes().as_slice())
                    .execute(&mut *latch).await.unwrap();
            }
            Transition::Archive | Transition::Delete => {
                sqlx::query(
                    "SELECT 1 FROM channels WHERE community_id=$1 AND id=$2 FOR NO KEY UPDATE",
                )
                .bind(community.as_uuid())
                .bind(channel)
                .execute(&mut *latch)
                .await
                .unwrap();
            }
        }
        let schedule = async {
            let writer_pid = blocked_pid(&db, pid).await;
            let release = async {
                blocked_pid(&db, writer_pid).await;
                latch.commit().await.unwrap();
            };
            let (result, ()) = tokio::join!(
                db.begin_channel_metadata_write(community, channel, relay.public_key()),
                release
            );
            let mut write = result.unwrap();
            assert_eq!(
                write.apply_command(&event, None).await.unwrap(),
                CommandApplication::Restricted
            );
            assert_eq!(write.labels().values(), &["original"]);
            assert!(write.commit().await.unwrap().is_none());
        };
        tokio::join!(
            transition.apply(&db, community, channel, &admin, &relay),
            schedule
        );
        assert_eq!(evidence(&db, community, &event).await, None);
        assert_rejected_without_effects(&db, community, channel, &relay, &prior, Some(true)).await;
        if !matches!(transition, Transition::RemoveAdmin) {
            assert_rejected_without_effects(&db, community, channel, &relay, &create, Some(true))
                .await;
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_snapshot_rolls_back_mutation_and_evidence_then_exact_retry_applies_once() {
    let (db, community) = database(false).await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    let create = command(&owner, channel, 9007, &[("label", "original")]);
    let original = apply(&db, community, &relay, &create).await.unwrap();
    let event = command(&owner, channel, 9002, &[("add-label", "new")]);
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(&event, None).await.unwrap(),
        CommandApplication::Applied
    );
    // A stale signed projection cannot authorize a partial commit.
    assert!(write.store_snapshot(&original, 512 * 1024).await.is_err());
    assert!(write.commit().await.is_err());
    assert_eq!(evidence(&db, community, &event).await, None);
    let write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(write.labels().values(), &["original"]);
    assert_eq!(write.previous.as_ref().unwrap().event.id, original.id);
    write.rollback().await.unwrap();
    let applied = apply(&db, community, &relay, &event).await.unwrap();
    let mut write = db
        .begin_channel_metadata_write(community, channel, relay.public_key())
        .await
        .unwrap();
    assert_eq!(
        write.apply_command(&event, None).await.unwrap(),
        CommandApplication::Committed
    );
    assert_eq!(write.previous.as_ref().unwrap().event.id, applied.id);
    assert!(write.commit().await.unwrap().is_none());
}
