//! Two listed open channels in a community never share a name, ignoring
//! case. Every write that can make a channel claim a listed name goes through
//! the real store functions here: create, rename, private to open, and
//! unlisted to listed. An archived channel keeps its name; a deleted one
//! frees it.

use crate::channel::ChannelUpdate;
use crate::error::DbError;
use crate::Db;
use buzz_core::channel::{ChannelType, ChannelVisibility};
use buzz_core::CommunityId;
use nostr::Keys;
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    db: Db,
    community: CommunityId,
    owner: Keys,
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
        Self {
            community: insert_community(&pool).await,
            db: Db::from_pool(pool),
            owner: Keys::generate(),
        }
    }

    async fn create(
        &self,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
    ) -> crate::error::Result<Uuid> {
        self.create_in(self.community, name, channel_type, visibility)
            .await
    }

    async fn create_in(
        &self,
        community: CommunityId,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
    ) -> crate::error::Result<Uuid> {
        self.db
            .create_channel(
                community,
                name,
                channel_type,
                visibility,
                None,
                &self.owner.public_key().to_bytes(),
                None,
            )
            .await
            .map(|channel| channel.id)
    }

    async fn open(&self, name: &str) -> Uuid {
        self.create(name, ChannelType::Stream, ChannelVisibility::Open)
            .await
            .expect("create open channel")
    }

    async fn update(&self, channel: Uuid, update: ChannelUpdate) -> crate::error::Result<()> {
        self.db
            .update_channel(self.community, channel, update)
            .await
            .map(|_| ())
    }

    async fn listing_state(&self, channel: Uuid) -> (String, String, bool) {
        let channel = self
            .db
            .get_channel(self.community, channel)
            .await
            .expect("reload channel");
        (channel.name, channel.visibility, channel.unlisted)
    }
}

async fn insert_community(pool: &PgPool) -> CommunityId {
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("channel-name-{}.test", community.as_uuid()))
        .execute(pool)
        .await
        .expect("insert community");
    community
}

fn assert_taken(result: crate::error::Result<impl std::fmt::Debug>, name: &str) {
    match result {
        Err(DbError::ChannelNameTaken(taken)) => assert_eq!(taken, name),
        other => panic!("expected {name:?} to be taken, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_listed_name_is_taken_only_for_listed_channels() {
    let f = Fixture::new().await;
    f.open("Square-iOS").await;

    // Another listed channel cannot take it, whatever its case or type, and
    // the display prefix is not part of the name.
    for (name, channel_type) in [
        ("square-ios", ChannelType::Stream),
        ("SQUARE-IOS", ChannelType::Forum),
        ("#square-ios ", ChannelType::Stream),
    ] {
        assert_taken(
            f.create(name, channel_type, ChannelVisibility::Open).await,
            buzz_core::channel::canonical_channel_name(name),
        );
    }

    // Channels the directory does not list may repeat it.
    for (channel_type, visibility) in [
        (ChannelType::Stream, ChannelVisibility::Private),
        (ChannelType::Forum, ChannelVisibility::Private),
        (ChannelType::Dm, ChannelVisibility::Private),
        (ChannelType::Workflow, ChannelVisibility::Open),
        (ChannelType::System, ChannelVisibility::Open),
    ] {
        f.create("square-ios", channel_type, visibility)
            .await
            .unwrap_or_else(|e| panic!("{channel_type:?}/{visibility:?}: {e}"));
    }

    // Names are per community.
    let other = insert_community(f.db.pool()).await;
    f.create_in(
        other,
        "square-ios",
        ChannelType::Stream,
        ChannelVisibility::Open,
    )
    .await
    .expect("another community may use the name");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_change_that_would_list_a_taken_name_changes_nothing() {
    let f = Fixture::new().await;
    f.open("payments").await;

    let private = f
        .create("payments", ChannelType::Stream, ChannelVisibility::Private)
        .await
        .expect("private channel");
    let unlisted = f.open("payments-unlisted").await;
    f.update(
        unlisted,
        ChannelUpdate {
            name: Some("payments".into()),
            unlisted: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("an unlisted channel may share the name");
    let other = f.open("payments-team").await;

    let cases = [
        (
            private,
            ChannelUpdate {
                visibility: Some("open".into()),
                ..Default::default()
            },
        ),
        (
            unlisted,
            ChannelUpdate {
                unlisted: Some(false),
                ..Default::default()
            },
        ),
        (
            other,
            ChannelUpdate {
                name: Some("PAYMENTS".into()),
                description: Some("rolled back with the name".into()),
                ..Default::default()
            },
        ),
    ];
    for (channel, update) in cases {
        let before = f.listing_state(channel).await;
        let description =
            f.db.get_channel(f.community, channel)
                .await
                .expect("reload")
                .description;
        let attempted = update.name.clone().unwrap_or_else(|| before.0.clone());
        assert_taken(f.update(channel, update).await, &attempted);
        assert_eq!(f.listing_state(channel).await, before);
        assert_eq!(
            f.db.get_channel(f.community, channel)
                .await
                .expect("reload")
                .description,
            description,
            "the whole update rolls back"
        );
    }

    // A rename to a free name, and the same name in another case, still work.
    f.update(
        other,
        ChannelUpdate {
            name: Some("payments-core".into()),
            ..Default::default()
        },
    )
    .await
    .expect("rename to a free name");
    f.update(
        other,
        ChannelUpdate {
            name: Some("Payments-Core".into()),
            ..Default::default()
        },
    )
    .await
    .expect("a channel may change the case of its own name");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_archived_channel_keeps_its_name_until_it_is_deleted() {
    let f = Fixture::new().await;
    let archived = f.open("square-ios").await;
    f.db.archive_channel(f.community, archived)
        .await
        .expect("archive");

    assert_taken(
        f.create("square-ios", ChannelType::Stream, ChannelVisibility::Open)
            .await,
        "square-ios",
    );
    f.db.unarchive_channel(f.community, archived)
        .await
        .expect("unarchive never collides");
    f.db.archive_channel(f.community, archived)
        .await
        .expect("archive again");

    assert!(f
        .db
        .soft_delete_channel(f.community, archived)
        .await
        .expect("delete"));
    f.open("square-ios").await;
}
