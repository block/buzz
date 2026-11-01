//! Membership-change reads through the production add, remove and delete writes.

use super::membership_changes::{MembershipChangeReason, MembershipChanges, MAX_ROWS};
use crate::Db;
use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};
use buzz_core::CommunityId;
use chrono::{DateTime, Duration, Utc};
use nostr::Keys;
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    db: Db,
    community: CommunityId,
    owner: Keys,
    reader: Keys,
    other: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            crate::migration::run_migrations(&pool)
                .await
                .expect("migrate test DB");
        }
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("membership-changes-{}.test", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");
        Self {
            db: Db::from_pool(pool),
            community,
            owner: Keys::generate(),
            reader: Keys::generate(),
            other: Keys::generate(),
        }
    }

    /// A stream the reader and `other` have both joined.
    async fn channel(&self, visibility: ChannelVisibility) -> Uuid {
        self.typed_channel(ChannelType::Stream, visibility).await
    }

    /// A channel of `channel_type` the reader and `other` have both joined.
    async fn typed_channel(
        &self,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
    ) -> Uuid {
        let channel = self
            .db
            .create_channel(
                self.community,
                "c",
                channel_type,
                visibility,
                None,
                &self.owner.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel")
            .id;
        for member in [&self.reader, &self.other] {
            self.join(channel, member).await;
        }
        channel
    }

    async fn join(&self, channel: Uuid, member: &Keys) {
        self.db
            .add_member(
                self.community,
                channel,
                &member.public_key().to_bytes(),
                MemberRole::Member,
                Some(&self.owner.public_key().to_bytes()),
            )
            .await
            .expect("add member");
    }

    async fn remove(&self, channel: Uuid, member: &Keys) {
        self.db
            .remove_member(
                self.community,
                channel,
                &member.public_key().to_bytes(),
                &self.owner.public_key().to_bytes(),
            )
            .await
            .expect("remove member");
    }

    async fn since(&self, since: DateTime<Utc>) -> MembershipChanges {
        self.db
            .membership_changes_since(self.community, &self.reader.public_key().to_bytes(), since)
            .await
            .expect("membership changes")
    }

    /// `(channel, reason)` pairs from a read that must not overflow.
    async fn pairs(&self, since: DateTime<Utc>) -> Vec<(Uuid, MembershipChangeReason)> {
        match self.since(since).await {
            MembershipChanges::Changes { changes, .. } => changes
                .into_iter()
                .map(|c| (c.channel_id, c.reason))
                .collect(),
            MembershipChanges::Overflow => panic!("unexpected overflow"),
        }
    }
}

fn hour_ago() -> DateTime<Utc> {
    Utc::now() - Duration::hours(1)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn removed_member_is_reported_even_when_the_channel_stays_open() {
    let f = Fixture::new().await;
    let open = f.channel(ChannelVisibility::Open).await;
    let private = f.channel(ChannelVisibility::Private).await;
    f.remove(open, &f.reader).await;
    f.remove(private, &f.reader).await;
    let mut got = f.pairs(hour_ago()).await;
    got.sort_by_key(|(c, _)| *c);
    let mut want = vec![
        (open, MembershipChangeReason::Removed),
        (private, MembershipChangeReason::Removed),
    ];
    want.sort_by_key(|(c, _)| *c);
    assert_eq!(got, want);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_member_is_not_reported() {
    let f = Fixture::new().await;
    f.channel(ChannelVisibility::Private).await;
    assert!(f.pairs(hour_ago()).await.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn removals_before_since_are_excluded() {
    let f = Fixture::new().await;
    let channel = f.channel(ChannelVisibility::Private).await;
    f.remove(channel, &f.reader).await;
    let MembershipChanges::Changes { as_of, changes } = f.since(hour_ago()).await else {
        panic!("unexpected overflow");
    };
    assert_eq!(changes.len(), 1);
    assert!(changes[0].at <= as_of);
    assert!(
        f.pairs(changes[0].at).await.len() == 1,
        "since is inclusive"
    );
    assert!(f
        .pairs(changes[0].at + Duration::microseconds(1))
        .await
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rejoined_member_is_not_reported() {
    let f = Fixture::new().await;
    let channel = f.channel(ChannelVisibility::Private).await;
    f.remove(channel, &f.reader).await;
    f.join(channel, &f.reader).await;
    assert!(f.pairs(hour_ago()).await.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deleted_channel_is_reported_for_an_active_member_only() {
    let f = Fixture::new().await;
    let deleted = f.channel(ChannelVisibility::Private).await;
    let left_then_deleted = f.channel(ChannelVisibility::Private).await;
    f.remove(left_then_deleted, &f.reader).await;
    for channel in [deleted, left_then_deleted] {
        assert!(f
            .db
            .soft_delete_channel(f.community, channel)
            .await
            .expect("delete channel"));
    }
    let mut got = f.pairs(hour_ago()).await;
    got.sort_by_key(|(c, _)| *c);
    let mut want = vec![
        (deleted, MembershipChangeReason::Deleted),
        (left_then_deleted, MembershipChangeReason::Removed),
    ];
    want.sort_by_key(|(c, _)| *c);
    assert_eq!(got, want);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn another_users_removal_is_not_reported() {
    let f = Fixture::new().await;
    let channel = f.channel(ChannelVisibility::Private).await;
    f.remove(channel, &f.other).await;
    assert!(f.pairs(hour_ago()).await.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn more_than_the_cap_overflows() {
    let f = Fixture::new().await;
    // Seed removed rows directly: the cap is about row count, not write paths.
    sqlx::query(
        "WITH ch AS (INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) \
         SELECT $1, gen_random_uuid(), 'c', 'private'::channel_visibility, 'stream', $2 \
         FROM generate_series(1,$3) RETURNING id) \
         INSERT INTO channel_members(community_id,channel_id,pubkey,role,removed_at,sync_modified_at) \
         SELECT $1, id, $2, 'member', now(), now() FROM ch",
    )
    .bind(f.community.as_uuid())
    .bind(f.reader.public_key().to_bytes().as_slice())
    .bind((MAX_ROWS + 1) as i32)
    .execute(&f.db.pool)
    .await
    .expect("seed removed rows");
    assert!(matches!(
        f.since(hour_ago()).await,
        MembershipChanges::Overflow
    ));
    // The cap counts rows, so exactly MAX_ROWS still returns them all.
    sqlx::query(
        "DELETE FROM channel_members WHERE community_id=$1 AND pubkey=$2 AND channel_id = \
         (SELECT channel_id FROM channel_members WHERE community_id=$1 AND pubkey=$2 LIMIT 1)",
    )
    .bind(f.community.as_uuid())
    .bind(f.reader.public_key().to_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .expect("trim");
    match f.since(hour_ago()).await {
        MembershipChanges::Changes { changes, .. } => assert_eq!(changes.len(), MAX_ROWS),
        MembershipChanges::Overflow => panic!("exactly the cap must not overflow"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn system_channels_are_not_reported() {
    let f = Fixture::new().await;
    let removed = f
        .typed_channel(ChannelType::System, ChannelVisibility::Private)
        .await;
    let deleted = f
        .typed_channel(ChannelType::System, ChannelVisibility::Private)
        .await;
    f.remove(removed, &f.reader).await;
    f.db.soft_delete_channel(f.community, deleted)
        .await
        .expect("delete channel");
    assert_eq!(f.pairs(hour_ago()).await, vec![]);
}
