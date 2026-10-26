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
        self.typed_channel(name, ChannelType::Stream, visibility)
            .await
    }

    /// [`Self::channel`] of `channel_type`.
    async fn typed_channel(
        &self,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
    ) -> Uuid {
        self.db
            .create_channel(
                self.community,
                name,
                channel_type,
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

    /// The named-only channels `reader` can read when a request names
    /// `named`, each with its type.
    async fn named_only(&self, reader: &Keys, named: &[Uuid]) -> Vec<(Uuid, String)> {
        self.readable(reader, named)
            .await
            .into_iter()
            .map(|channel| (channel.id, channel.channel_type))
            .collect()
    }

    /// The named-only channels `reader` can read when a request names
    /// `named`, each with whether the system-channel opt-in may add it.
    async fn opt_in(&self, reader: &Keys, named: &[Uuid]) -> Vec<(Uuid, bool)> {
        self.readable(reader, named)
            .await
            .into_iter()
            .map(|channel| (channel.id, channel.opt_in))
            .collect()
    }

    async fn readable(
        &self,
        reader: &Keys,
        named: &[Uuid],
    ) -> Vec<crate::channel_members::ReadableNamedOnlyChannel> {
        self.db
            .get_readable_named_only_channels(
                self.community,
                &reader.public_key().to_bytes(),
                named,
            )
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
    let named = f.named_only(&f.outsider, &[listed, unlisted]).await;
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
        .named_only(&stranger, &[unlisted])
        .await
        .iter()
        .any(|(id, _)| *id == unlisted));
}

/// Only the unlisted channels a request names are loaded, so the lookup stays
/// small however many unlisted chats a community has.
/// Mutation: drop `c.id = ANY($3)` from the unlisted branch of
/// `get_readable_named_only_channels` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn only_the_named_unlisted_channels_are_loaded() {
    let f = Fixture::new().await;
    let a = f.channel("a", ChannelVisibility::Open).await;
    let b = f.channel("b", ChannelVisibility::Open).await;
    f.set_unlisted(a, true).await;
    f.set_unlisted(b, true).await;

    assert_eq!(
        f.named_only(&f.outsider, &[a]).await,
        [(a, "stream".to_owned())],
        "naming A loads A and not B"
    );
    assert!(
        f.named_only(&f.outsider, &[]).await.is_empty(),
        "naming nothing loads no unlisted channel"
    );
    let both = f.named_only(&f.outsider, &[a, b]).await;
    assert!(both.contains(&(a, "stream".to_owned())));
    assert!(both.contains(&(b, "stream".to_owned())));
}

/// An unlisted open system channel is read only by naming it, for a
/// non-member: the system-channel opt-in (`#t:["system"]`, `#P`) needs every
/// readable system channel, but never gets this one. Its members keep it as
/// before, and a listed open system channel is loaded and opted in to as
/// before.
/// Mutation: drop `NOT c.unlisted OR c.id = ANY($3)` from the system branch
/// of `get_readable_named_only_channels` → RED (naming nothing loads it); make
/// its `opt_in` always true → RED (the outsider may opt in to it).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn an_unlisted_system_channel_is_read_only_by_name() {
    let f = Fixture::new().await;
    let listed = f
        .typed_channel("listed", ChannelType::System, ChannelVisibility::Open)
        .await;
    let unlisted = f
        .typed_channel("unlisted", ChannelType::System, ChannelVisibility::Open)
        .await;
    f.set_unlisted(unlisted, true).await;

    // A non-member that names nothing (an opt-in alone) gets only the listed one.
    assert_eq!(
        f.opt_in(&f.outsider, &[]).await,
        [(listed, true)],
        "opt-in discovery leaves the unlisted system channel out"
    );
    // Naming it reads it, but the opt-in still may not add it.
    let named = f.opt_in(&f.outsider, &[unlisted]).await;
    assert_eq!(named.len(), 2, "{named:?}");
    assert!(named.contains(&(listed, true)));
    assert!(named.contains(&(unlisted, false)), "readable by name only");

    // Its member keeps it, for the opt-in too, whether or not it is named.
    for names in [&[][..], &[unlisted][..]] {
        let member = f.opt_in(&f.owner, names).await;
        assert_eq!(member.len(), 2, "{member:?}");
        assert!(member.contains(&(listed, true)));
        assert!(member.contains(&(unlisted, true)), "a member opts in to it");
    }

    // Listing it again puts it back in discovery.
    f.set_unlisted(unlisted, false).await;
    let relisted = f.opt_in(&f.outsider, &[]).await;
    assert_eq!(relisted.len(), 2, "{relisted:?}");
    assert!(relisted.contains(&(unlisted, true)));
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
        !f.named_only(&f.outsider, &[chat])
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
        .named_only(&f.outsider, &[chat])
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

/// A channel can start unlisted (kind:9007 `["unlisted", "true"]`), through
/// both create paths: the event-backed create a client UUID takes, and the
/// relay-chosen-UUID create. It is never listed to non-members, even for a
/// moment, and stays named-only once it is opened.
/// Mutation: drop `unlisted` from either `INSERT INTO channels` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_created_unlisted_is_never_listed() {
    let f = Fixture::new().await;
    let owner = f.owner.public_key().to_bytes();

    let with_event = Uuid::new_v4();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(9007), "")
        .tags([
            nostr::Tag::parse(["h", &with_event.to_string()]).expect("h tag"),
            nostr::Tag::parse(["name", "solo"]).expect("name tag"),
            nostr::Tag::parse(["visibility", "open"]).expect("visibility tag"),
            nostr::Tag::parse(["unlisted", "true"]).expect("unlisted tag"),
        ])
        .sign_with_keys(&f.owner)
        .expect("sign 9007");
    let created =
        f.db.create_channel_with_event(
            f.community,
            with_event,
            crate::channel::NewChannel {
                name: "solo",
                channel_type: ChannelType::Stream,
                visibility: ChannelVisibility::Open,
                description: None,
                created_by: &owner,
                ttl_seconds: None,
                labels: &[],
                unlisted: true,
                members: &[],
            },
            &event,
        )
        .await
        .expect("create with event");
    assert!(matches!(created, crate::channel::ChannelCreate::Created(_)));

    let relay_chosen =
        f.db.create_labeled_channel(
            f.community,
            "solo-2",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
            &[],
            true,
        )
        .await
        .expect("create labeled channel")
        .id;

    let listed = f.channel("listed", ChannelVisibility::Open).await;
    assert!(
        !f.db
            .get_channel(f.community, listed)
            .await
            .expect("channel")
            .unlisted,
        "channels start listed by default"
    );

    for chat in [with_event, relay_chosen] {
        assert!(
            f.db.get_channel(f.community, chat)
                .await
                .expect("channel")
                .unlisted,
            "the setting is stored at creation"
        );
        assert!(!f.implicit(&f.outsider).await.contains(&chat));
        assert!(!f.listed(&f.outsider).await.contains(&chat));
        assert!(f
            .named_only(&f.outsider, &[chat])
            .await
            .contains(&(chat, "stream".to_owned())));
        assert!(f.implicit(&f.owner).await.contains(&chat));
    }
}
