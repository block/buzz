//! A live stream or forum channel's name is unique within its community,
//! compared case-insensitively on the canonical name. DM, workflow and
//! system channel names are derived or internal, so they do not compete.
//! A soft-deleted channel frees its name; an archived one keeps it.

use crate::channel::ChannelUpdate;
use crate::{Db, DbError};
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
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("chname-{}.test", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");
        Self {
            db: Db::from_pool(pool),
            community,
            owner: Keys::generate(),
        }
    }

    async fn create(&self, name: &str, channel_type: ChannelType) -> crate::Result<Uuid> {
        self.db
            .create_channel(
                self.community,
                name,
                channel_type,
                ChannelVisibility::Open,
                None,
                &self.owner.public_key().to_bytes(),
                None,
            )
            .await
            .map(|channel| channel.id)
    }

    async fn rename(&self, channel: Uuid, name: &str) -> crate::Result<()> {
        self.db
            .update_channel(
                self.community,
                channel,
                ChannelUpdate {
                    name: Some(name.to_owned()),
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
    }
}

fn assert_name_taken<T: std::fmt::Debug>(result: crate::Result<T>, context: &str) {
    match result {
        Err(DbError::ChannelNameTaken(_)) => {}
        other => panic!("{context}: expected ChannelNameTaken, got {other:?}"),
    }
}

/// Creating over a live stream/forum name is refused — case-insensitively and
/// on the canonical name — while non-competing types and other communities
/// stay unaffected, and a soft delete frees the name where an archive does not.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_name_uniqueness_lifecycle() {
    let f = Fixture::new().await;

    let general = f
        .create("general", ChannelType::Stream)
        .await
        .expect("create general");
    assert_name_taken(
        f.create("general", ChannelType::Stream).await,
        "exact duplicate",
    );
    assert_name_taken(
        f.create("General", ChannelType::Stream).await,
        "case-insensitive duplicate",
    );
    assert_name_taken(
        f.create("#general ", ChannelType::Forum).await,
        "canonicalized duplicate across competing types",
    );

    // DM, workflow and system names are derived or internal: no competition
    // in either direction.
    f.create("general", ChannelType::Dm)
        .await
        .expect("dm may reuse the name");
    f.create("general", ChannelType::Workflow)
        .await
        .expect("workflow may reuse the name");
    f.create("ops", ChannelType::System)
        .await
        .expect("create system channel");
    f.create("ops", ChannelType::Stream)
        .await
        .expect("stream may use a system channel's name");

    // The same name is fine in another community.
    let other = Fixture::new().await;
    other
        .create("general", ChannelType::Stream)
        .await
        .expect("other community");

    // Archived channels can come back, so the name stays reserved.
    f.db.archive_channel(f.community, general)
        .await
        .expect("archive");
    assert_name_taken(
        f.create("general", ChannelType::Stream).await,
        "archived channel keeps its name",
    );

    // A soft delete frees the name.
    f.db.soft_delete_channel(f.community, general)
        .await
        .expect("delete");
    f.create("general", ChannelType::Stream)
        .await
        .expect("deleted channel frees its name");
}

/// Renaming onto a live competing name is refused; renaming a channel to its
/// own name (for example, changing only the case) stays allowed.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn rename_respects_name_uniqueness() {
    let f = Fixture::new().await;
    let news = f
        .create("news", ChannelType::Stream)
        .await
        .expect("create news");
    f.create("random", ChannelType::Forum)
        .await
        .expect("create random");

    assert_name_taken(f.rename(news, "Random").await, "rename onto a taken name");
    f.rename(news, "News")
        .await
        .expect("case change of own name");
    f.rename(news, "daily-news")
        .await
        .expect("rename to a free name");

    // A non-competing channel may take a competing channel's name.
    let dm = f.create("alice", ChannelType::Dm).await.expect("create dm");
    f.rename(dm, "daily-news")
        .await
        .expect("dm rename does not compete");
}

/// A resend of the same create (same UUID, same name) stays idempotent and
/// is not misread as a name conflict.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn create_with_id_resend_is_not_a_conflict() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let owner = f.owner.public_key().to_bytes();
    let create = || {
        f.db.create_channel_with_id(
            f.community,
            id,
            "release-train",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
    };
    let (_, created) = create().await.expect("first create");
    assert!(created, "first create inserts");
    let (record, created) = create().await.expect("resend is idempotent");
    assert!(!created, "resend does not insert");
    assert_eq!(record.id, id);

    // A different UUID with the same name is a conflict.
    assert_name_taken(
        f.db.create_channel_with_id(
            f.community,
            Uuid::new_v4(),
            "release-train",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &f.owner.public_key().to_bytes(),
            None,
        )
        .await,
        "same name under a new UUID",
    );
}
