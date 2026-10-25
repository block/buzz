//! An unlisted open channel is left out of the reads that list channels to
//! non-members, but stays readable and joinable by id. Members still list it.
//! The flag is set through the real settings write (`update_channel`).

use crate::channel::ChannelUpdate;
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
    outsider: Keys,
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
            .bind(format!("unlisted-{}.test", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");
        Self {
            db: Db::from_pool(pool),
            community,
            owner: Keys::generate(),
            outsider: Keys::generate(),
        }
    }

    /// A channel created by `owner`, who becomes its owner member.
    async fn channel(&self, name: &str, visibility: ChannelVisibility) -> Uuid {
        self.db
            .create_channel(
                self.community,
                name,
                ChannelType::Stream,
                visibility,
                None,
                &self.owner.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel")
            .id
    }

    async fn set_unlisted(&self, channel: Uuid, unlisted: bool) -> crate::channel::ChannelRecord {
        self.db
            .update_channel(
                self.community,
                channel,
                ChannelUpdate {
                    unlisted: Some(unlisted),
                    ..Default::default()
                },
            )
            .await
            .expect("set unlisted")
    }

    async fn implicit(&self, reader: &Keys) -> Vec<Uuid> {
        self.db
            .get_accessible_channel_ids(self.community, &reader.public_key().to_bytes())
            .await
            .expect("accessible channel ids")
    }

    async fn listed(&self, reader: &Keys) -> Vec<Uuid> {
        self.db
            .get_accessible_channels(self.community, &reader.public_key().to_bytes(), None, None)
            .await
            .expect("accessible channels")
            .into_iter()
            .map(|channel| channel.channel.id)
            .collect()
    }

    async fn named_only(&self, reader: &Keys) -> Vec<(Uuid, String)> {
        self.db
            .get_readable_named_only_channels(self.community, &reader.public_key().to_bytes())
            .await
            .expect("readable named-only channels")
    }
}

/// Mutation: drop `NOT unlisted` from `get_accessible_channel_ids` or
/// `get_accessible_channels`, or the unlisted branch from
/// `get_readable_named_only_channels` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn unlisted_open_channel_is_named_only_for_non_members() {
    let f = Fixture::new().await;
    let listed = f.channel("listed", ChannelVisibility::Open).await;
    let unlisted = f.channel("unlisted", ChannelVisibility::Open).await;
    let record = f.set_unlisted(unlisted, true).await;
    assert!(record.unlisted, "update_channel returns the new setting");
    assert!(
        f.db.get_channel(f.community, unlisted)
            .await
            .expect("get channel")
            .unlisted,
        "the setting is stored"
    );

    // A non-member does not list it, but can name it.
    let implicit = f.implicit(&f.outsider).await;
    assert!(implicit.contains(&listed));
    assert!(!implicit.contains(&unlisted), "implicit scope");
    let shown = f.listed(&f.outsider).await;
    assert!(shown.contains(&listed));
    assert!(!shown.contains(&unlisted), "channel list");
    let named = f.named_only(&f.outsider).await;
    assert!(
        named.contains(&(unlisted, "stream".to_owned())),
        "readable by name: {named:?}"
    );
    assert!(
        !named.iter().any(|(id, _)| *id == listed),
        "a listed channel is not named-only"
    );

    // A member still lists it.
    assert!(f.implicit(&f.owner).await.contains(&unlisted));
    assert!(f.listed(&f.owner).await.contains(&unlisted));

    // Joining by id works as on any open channel (kind:9021 → put_member),
    // and the joiner then lists it like any member.
    let joined =
        f.db.put_member(
            f.community,
            unlisted,
            &f.outsider.public_key().to_bytes(),
            None,
            None,
        )
        .await
        .expect("self-join an unlisted open channel");
    assert_eq!(joined.role, "member");
    assert!(f.implicit(&f.outsider).await.contains(&unlisted));
    assert!(f.listed(&f.outsider).await.contains(&unlisted));

    // Listing it again puts it back for everyone.
    let stranger = Keys::generate();
    assert!(!f.implicit(&stranger).await.contains(&unlisted));
    assert!(!f.set_unlisted(unlisted, false).await.unlisted);
    assert!(f.implicit(&stranger).await.contains(&unlisted));
    assert!(f.listed(&stranger).await.contains(&unlisted));
    assert!(!f
        .named_only(&stranger)
        .await
        .iter()
        .any(|(id, _)| *id == unlisted));
}

/// The flag is stored on its own and matters only while the channel is open:
/// a private unlisted channel stays invisible and unreadable to non-members,
/// and opening it keeps it unlisted.
/// Mutation: drop `c.visibility = 'open'` from the unlisted branch of
/// `get_readable_named_only_channels` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn unlisted_private_channel_stays_private_and_opens_unlisted() {
    let f = Fixture::new().await;
    let chat = f.channel("chat", ChannelVisibility::Private).await;
    assert!(f.set_unlisted(chat, true).await.unlisted);

    assert!(!f.implicit(&f.outsider).await.contains(&chat));
    assert!(!f.listed(&f.outsider).await.contains(&chat));
    assert!(
        !f.named_only(&f.outsider)
            .await
            .iter()
            .any(|(id, _)| *id == chat),
        "a private channel is never readable by name"
    );
    assert!(f.implicit(&f.owner).await.contains(&chat));

    let opened =
        f.db.update_channel(
            f.community,
            chat,
            ChannelUpdate {
                visibility: Some("open".into()),
                ..Default::default()
            },
        )
        .await
        .expect("open the channel");
    assert!(opened.unlisted, "opening keeps the stored flag");
    assert!(!f.implicit(&f.outsider).await.contains(&chat));
    assert!(f
        .named_only(&f.outsider)
        .await
        .contains(&(chat, "stream".to_owned())));
}

/// kind:39000 publishes the flag from the stored row.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn group_metadata_publishes_the_stored_unlisted_flag() {
    let f = Fixture::new().await;
    let chat = f.channel("chat", ChannelVisibility::Open).await;
    let flag = vec!["unlisted".to_string()];
    let before = f.db.get_channel(f.community, chat).await.expect("channel");
    assert!(!crate::channel::group_metadata_tags(&before, &[]).contains(&flag));
    let after = f.set_unlisted(chat, true).await;
    assert!(crate::channel::group_metadata_tags(&after, &[]).contains(&flag));
}
