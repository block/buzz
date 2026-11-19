//! NIP-CP pin list writes through the production entry points.

use super::*;
use buzz_core::kind::{KIND_PIN_MESSAGE, KIND_SYSTEM_MESSAGE, KIND_UNPIN_MESSAGE};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;

struct Fixture {
    db: Db,
    community: CommunityId,
    channel: Uuid,
    relay: Keys,
}

async fn channel(pool: &PgPool, community: Uuid) -> Uuid {
    let channel = Uuid::new_v4();
    sqlx::query("INSERT INTO channels (id, community_id, name, created_by) VALUES ($1,$2,$3,$4)")
        .bind(channel)
        .bind(community)
        .bind(format!("pins-{}", channel.simple()))
        .bind(vec![7_u8; 32])
        .execute(pool)
        .await
        .expect("channel");
    channel
}

async fn fixture() -> Fixture {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("test database");
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community)
        .bind(format!("pins-{}.example", community.simple()))
        .execute(&pool)
        .await
        .expect("community");
    let channel = channel(&pool, community).await;
    Fixture {
        db: Db::from_pool(pool),
        community: CommunityId::from_uuid(community),
        channel,
        relay: Keys::generate(),
    }
}

fn event(kind: u32, channel: Uuid, target: Option<&Event>) -> Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).expect("h")];
    tags.extend(target.map(|target| Tag::parse(["e", &target.id.to_hex()]).expect("e")));
    EventBuilder::new(Kind::Custom(kind as u16), "")
        .tags(tags)
        // Two commands for one target are different events.
        .tag(Tag::parse(["client-id", &Uuid::new_v4().to_string()]).expect("client-id"))
        .sign_with_keys(&Keys::generate())
        .expect("signed event")
}

impl Fixture {
    /// Store a live message of `kind` in `channel`.
    async fn message_in(&self, channel: Uuid, kind: u32) -> Event {
        let message = event(kind, channel, None);
        self.db
            .insert_event(self.community, &message, Some(channel))
            .await
            .expect("message");
        message
    }

    async fn message(&self) -> Event {
        self.message_in(self.channel, 9).await
    }

    async fn apply(&self, command: &Event, target: &Event) -> Result<PinCommandWrite> {
        let notice =
            EventBuilder::new(Kind::Custom(KIND_SYSTEM_MESSAGE as u16), target.id.to_hex())
                .tag(Tag::parse(["h", &self.channel.to_string()]).expect("h"))
                .sign_with_keys(&self.relay)
                .expect("notice");
        let change = if command.kind.as_u16() as u32 == KIND_PIN_MESSAGE {
            PinChange::Pin { notice: &notice }
        } else {
            PinChange::Unpin
        };
        self.db
            .apply_pin_command(
                self.community,
                self.channel,
                command,
                target.id.as_bytes(),
                change,
                &self.relay,
            )
            .await
    }

    async fn pin(&self, target: &Event) -> Result<PinCommandWrite> {
        self.apply(&event(KIND_PIN_MESSAGE, self.channel, Some(target)), target)
            .await
    }

    async fn unpin(&self, target: &Event) -> Result<PinCommandWrite> {
        self.apply(
            &event(KIND_UNPIN_MESSAGE, self.channel, Some(target)),
            target,
        )
        .await
    }

    /// Every stored pin list row of the channel as `(created_at, pinned IDs)`,
    /// with the notices stored in the channel.
    async fn stored(&self) -> (Vec<(i64, Vec<String>)>, i64) {
        let lists: Vec<(i64, sqlx::types::Json<Vec<Vec<String>>>)> = sqlx::query_as(
            "SELECT extract(epoch FROM created_at)::bigint, tags FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = $3",
        )
        .bind(self.community.as_uuid())
        .bind(self.channel)
        .bind(KIND_CHANNEL_PINS as i32)
        .fetch_all(&self.db.pool)
        .await
        .expect("pin lists");
        let notices = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = $3",
        )
        .bind(self.community.as_uuid())
        .bind(self.channel)
        .bind(KIND_SYSTEM_MESSAGE as i32)
        .fetch_one(&self.db.pool)
        .await
        .expect("notices");
        let lists = lists
            .into_iter()
            .map(|(created_at, tags)| (created_at, pinned(&tags.0)))
            .collect();
        (lists, notices)
    }

    /// The channel's one pin list as `(created_at, pinned IDs)`.
    async fn list(&self) -> (i64, Vec<String>) {
        let (mut lists, _) = self.stored().await;
        assert_eq!(lists.len(), 1, "a channel keeps one pin list row");
        lists.remove(0)
    }

    async fn is_stored(&self, event: &Event) -> bool {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
        )
        .bind(self.community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&self.db.pool)
        .await
        .expect("event row")
    }

    /// Delete `target` the way a kind:5 or kind:9005 does.
    async fn delete(&self, deletion: &Event, target: &Event) -> Vec<StoredEvent> {
        let target = crate::event::DeletionTarget {
            event_id: target.id.as_bytes(),
            created_at: chrono::DateTime::from_timestamp(target.created_at.as_secs() as i64, 0)
                .expect("timestamp"),
            parent_event_id: None,
            root_event_id: None,
        };
        self.db
            .insert_deletion_event(
                self.community,
                deletion,
                Some(self.channel),
                None,
                &[target],
                &self.relay,
            )
            .await
            .expect("deletion")
            .3
    }
}

fn pinned(tags: &[Vec<String>]) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag[0] == "e")
        .map(|tag| tag[1].clone())
        .collect()
}

fn ids(events: &[&Event]) -> Vec<String> {
    events.iter().map(|event| event.id.to_hex()).collect()
}

fn denied(result: Result<PinCommandWrite>) -> String {
    match result {
        Err(DbError::AccessDenied(reason)) => reason,
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("the command was applied"),
    }
}

/// Pins go to the front, each list replaces the last with a later
/// `created_at`, and a command that has nothing to change is stored without
/// a new list or notice.
/// Mutation: apply `(Pin, Some(_))` like `(Pin, None)` → RED (a second list
/// and notice for the pin of an already pinned message).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn pins_and_unpins_rewrite_one_relay_signed_list() {
    let f = fixture().await;
    let (a, b) = (f.message().await, f.message().await);

    let first = f.pin(&a).await.expect("pin a");
    let list = first.list.expect("the pin changes the list");
    assert!(first.inserted);
    assert_eq!(list.event.pubkey, f.relay.public_key());
    assert_eq!(list.event.content, "");
    assert_eq!(
        list.event.tags.first().map(|tag| tag.as_slice().to_vec()),
        Some(vec!["d".to_owned(), f.channel.to_string()])
    );
    assert!(list.event.verify().is_ok(), "the list is signed");
    let (first_at, first_ids) = f.list().await;
    assert_eq!(first_ids, ids(&[&a]));

    assert!(f.pin(&b).await.expect("pin b").list.is_some());
    let (second_at, second_ids) = f.list().await;
    assert_eq!(second_ids, ids(&[&b, &a]), "the newest pin is first");
    assert!(second_at > first_at, "a new list is newer than the last");
    assert_eq!(f.stored().await.1, 2, "one notice for each pin");

    let again = f.pin(&a).await.expect("pin a again");
    assert!(again.inserted && again.list.is_none());
    let never = f.message().await;
    let absent = f.unpin(&never).await.expect("unpin a message never pinned");
    assert!(absent.inserted && absent.list.is_none());
    assert_eq!(f.list().await, (second_at, ids(&[&b, &a])));
    assert_eq!(f.stored().await.1, 2, "no notice when nothing changes");

    assert!(f.unpin(&a).await.expect("unpin a").list.is_some());
    assert_eq!(f.list().await.1, ids(&[&b]));
    assert!(f.unpin(&b).await.expect("unpin b").list.is_some());
    assert!(
        f.list().await.1.is_empty(),
        "the last unpin leaves an empty list"
    );
    assert_eq!(f.stored().await.1, 2, "an unpin posts no notice");
}

/// A command sent again is not applied again, so a repeated pin cannot undo
/// an unpin made in between. Two copies sent at once apply once.
/// Mutation: drop the `!inserted` return → RED (the resend pins again).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_resent_command_is_not_applied_again() {
    let f = fixture().await;
    let a = f.message().await;
    let pin = event(KIND_PIN_MESSAGE, f.channel, Some(&a));
    assert!(f.apply(&pin, &a).await.expect("pin").list.is_some());
    assert!(f.unpin(&a).await.expect("unpin").list.is_some());

    let resent = f.apply(&pin, &a).await.expect("resend");
    assert!(!resent.inserted && resent.list.is_none());
    assert_eq!(resent.stored.event.id, pin.id);
    assert!(f.list().await.1.is_empty(), "the unpin stands");
    assert_eq!(f.stored().await.1, 1, "the resend posts no notice");

    let b = f.message().await;
    let racing = event(KIND_PIN_MESSAGE, f.channel, Some(&b));
    let (one, two) = tokio::join!(f.apply(&racing, &b), f.apply(&racing, &b));
    let (one, two) = (one.expect("first copy"), two.expect("second copy"));
    assert_ne!(one.inserted, two.inserted, "one copy is the duplicate");
    assert_eq!(one.list.is_some(), one.inserted);
    assert_eq!(two.list.is_some(), two.inserted);
    assert_eq!(f.list().await.1, ids(&[&b]));
    assert_eq!(f.stored().await.1, 2);
}

/// A pin whose target is not a live message of the channel is refused and
/// leaves nothing behind: no command, no list, no notice.
/// Mutation: drop the `kind = ANY` or `channel_id` predicate from
/// `live_messages` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_pin_needs_a_live_message_of_the_channel() {
    let f = fixture().await;
    let elsewhere = channel(&f.db.pool, *f.community.as_uuid()).await;
    let deleted = f.message().await;
    f.delete(&event(5, f.channel, Some(&deleted)), &deleted)
        .await;
    for (case, target) in [
        ("another channel", f.message_in(elsewhere, 9).await),
        ("a reaction", f.message_in(f.channel, 7).await),
        ("a deleted message", deleted),
        ("a message never stored", event(9, f.channel, None)),
    ] {
        let command = event(KIND_PIN_MESSAGE, f.channel, Some(&target));
        assert_eq!(
            denied(f.apply(&command, &target).await),
            "target is not a message in this channel",
            "{case}"
        );
        assert!(!f.is_stored(&command).await, "{case}: command stored");
    }
    assert_eq!(f.stored().await, (Vec::new(), 0));
}

/// A channel holds at most [`MAX_CHANNEL_PINS`] pins. Entries whose message
/// was removed without the list being told do not count, and leave the list
/// the next time it changes.
/// Mutation: skip `live_messages(pins)` before the limit check → RED (the
/// pin after the erasure is refused); check the target only when it is not
/// listed → RED (the erased message is pinned again).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_pin_limit_counts_live_messages_only() {
    let f = fixture().await;
    let mut messages = Vec::new();
    for _ in 0..MAX_CHANNEL_PINS {
        let message = f.message().await;
        assert!(f.pin(&message).await.expect("pin").list.is_some());
        messages.push(message);
    }
    let over = f.message().await;
    let command = event(KIND_PIN_MESSAGE, f.channel, Some(&over));
    assert_eq!(denied(f.apply(&command, &over).await), "pin limit reached");
    assert!(!f.is_stored(&command).await);
    assert_eq!(f.list().await.1.len(), MAX_CHANNEL_PINS);

    // An account erasure removes messages in bulk and leaves the list alone.
    sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(f.community.as_uuid())
        .bind(messages[0].id.as_bytes().as_slice())
        .execute(&f.db.pool)
        .await
        .expect("erase a pinned message");
    // Still listed, but no longer a message that can be pinned.
    let again = event(KIND_PIN_MESSAGE, f.channel, Some(&messages[0]));
    assert_eq!(
        denied(f.apply(&again, &messages[0]).await),
        "target is not a message in this channel"
    );
    assert!(!f.is_stored(&again).await);
    assert!(f
        .pin(&over)
        .await
        .expect("pin after erasure")
        .list
        .is_some());
    let list = f.list().await.1;
    assert_eq!(list.len(), MAX_CHANNEL_PINS);
    assert_eq!(list[0], over.id.to_hex());
    assert!(!list.contains(&messages[0].id.to_hex()));
}

/// Pins sent at the same time all land: no command overwrites another's.
/// Mutation: read the list before `take_replacement_lock` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_pins_all_land() {
    let f = fixture().await;
    let mut messages = Vec::new();
    for _ in 0..8 {
        messages.push(f.message().await);
    }
    let writes = futures_util::future::join_all(messages.iter().map(|message| f.pin(message)));
    for write in writes.await {
        assert!(write.expect("pin").list.is_some());
    }
    let mut list = f.list().await.1;
    list.sort();
    let mut expected = ids(&messages.iter().collect::<Vec<_>>());
    expected.sort();
    assert_eq!(list, expected);
}

/// Deleting a pinned message rewrites the list in the deletion's own
/// transaction; a resend, or the deletion of an unpinned message, does not.
/// Mutation: drop the `unpin_deleted_message` call from
/// `insert_deletion_event` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn deleting_a_pinned_message_unpins_it() {
    let f = fixture().await;
    let (a, b, unpinned) = (f.message().await, f.message().await, f.message().await);
    f.pin(&a).await.expect("pin a");
    f.pin(&b).await.expect("pin b");
    let before = f.list().await.0;

    let deletion = event(5, f.channel, Some(&a));
    let lists = f.delete(&deletion, &a).await;
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].event.pubkey, f.relay.public_key());
    let (after, list) = f.list().await;
    assert_eq!(list, ids(&[&b]));
    assert!(after > before);

    assert!(f.delete(&deletion, &a).await.is_empty(), "a resend");
    let other = event(9005, f.channel, Some(&unpinned));
    assert!(f.delete(&other, &unpinned).await.is_empty());
    assert_eq!(f.list().await, (after, ids(&[&b])));
}

/// Account erasure leaves the ID and timestamp of a recent command. The sweep
/// removes those an hour old in every community, and only those. Nothing has
/// to happen in the channel: it is archived and takes no further command.
/// Mutation: drop the `kind`, `pubkey` or `created_at` predicate from the
/// sweep's `DELETE` → RED (a message, a command or a young record is removed);
/// delete nothing → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn erased_command_records_expire_after_an_hour() {
    let f = fixture().await;
    let neighbor = fixture().await;
    let message = f.message().await;
    let command = event(KIND_PIN_MESSAGE, f.channel, Some(&message));
    f.apply(&command, &message).await.expect("pin");
    sqlx::query("UPDATE channels SET archived_at = now() WHERE id = $1")
        .bind(f.channel)
        .execute(&f.db.pool)
        .await
        .expect("archive");

    // (community, kind, age in minutes, erased, kept)
    let rows = [
        (&f, KIND_PIN_MESSAGE, 61, true, false),
        (&f, KIND_UNPIN_MESSAGE, 61, true, false),
        (&f, KIND_PIN_MESSAGE, 59, true, true),
        (&f, KIND_PIN_MESSAGE, 61, false, true),
        (&f, 9, 61, true, true),
        (&neighbor, KIND_PIN_MESSAGE, 61, true, false),
        (&neighbor, KIND_PIN_MESSAGE, 59, true, true),
        (&neighbor, KIND_UNPIN_MESSAGE, 61, false, true),
    ];
    let mut ids = Vec::new();
    for (owner, kind, minutes, erased, _) in rows {
        let id = Uuid::new_v4().as_bytes().repeat(2);
        let author: &[u8] = if erased { &[] } else { &[7; 32] };
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, \
             channel_id, deleted_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $3, $8, $9)",
        )
        .bind(owner.community.as_uuid())
        .bind(&id)
        .bind(author)
        .bind(chrono::Utc::now() - chrono::Duration::minutes(minutes))
        .bind(kind as i32)
        .bind(sqlx::types::Json(Vec::<Vec<String>>::new()))
        .bind("")
        .bind(owner.channel)
        .bind(chrono::Utc::now())
        .execute(&f.db.pool)
        .await
        .expect("record");
        ids.push(id);
    }

    let removed =
        f.db.reap_expired_pin_command_records()
            .await
            .expect("sweep");
    assert!(removed >= 3, "removed {removed}");
    for ((owner, kind, minutes, erased, kept), id) in rows.into_iter().zip(ids) {
        let stored: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
        )
        .bind(owner.community.as_uuid())
        .bind(&id)
        .fetch_one(&f.db.pool)
        .await
        .expect("row");
        assert_eq!(
            stored, kept,
            "kind {kind}, {minutes} minutes old, erased: {erased}"
        );
    }
    assert!(f.is_stored(&command).await, "a live command stays");
}
