//! PostgreSQL contract for stored channel rows: ingest upkeep, the top-level
//! rule, edits, fill of older history, deleted-head repair and scoping.
use super::channel_heads::{ChannelHead, ChannelHeads};
use crate::{
    channel::{ChannelType, ChannelVisibility},
    event::ThreadMetadataParams,
    migration, Db,
};
use buzz_core::CommunityId;
use chrono::DateTime;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::PgPool;
use uuid::Uuid;

struct Fixture {
    db: Db,
    pool: PgPool,
    community: CommunityId,
    channel: Uuid,
    member: Keys,
    /// A signed time inside the ingest floor; tests add small offsets.
    base: u64,
}

async fn fixture() -> Fixture {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect to isolated database");
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() == Ok("migration") {
        migration::run_migrations(&pool)
            .await
            .expect("apply embedded migrations");
    }
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community(&format!("channel-heads-{}.local", Uuid::new_v4()))
        .await
        .expect("create community")
        .id;
    let member = Keys::generate();
    let channel = channel(&db, &pool, community, &member).await;
    Fixture {
        db,
        pool,
        community,
        channel,
        member,
        base: Timestamp::now().as_secs() - 600,
    }
}

async fn channel(db: &Db, pool: &PgPool, community: CommunityId, member: &Keys) -> Uuid {
    let id = db
        .create_channel(
            community,
            &format!("heads-{}", Uuid::new_v4().simple()),
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &member.public_key().to_bytes(),
            None,
        )
        .await
        .expect("create channel")
        .id;
    sqlx::query(
        "INSERT INTO channel_members (community_id, channel_id, pubkey) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(member.public_key().to_bytes().as_slice())
    .execute(pool)
    .await
    .expect("join channel");
    id
}

fn message(kind: u16, text: &str, at: u64, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::Custom(kind), text)
        .tags(tags)
        .custom_created_at(Timestamp::from(at))
        .sign_with_keys(&Keys::generate())
        .expect("sign test event")
}

impl Fixture {
    async fn post(&self, text: &str, offset: u64) -> Event {
        let event = message(9, text, self.base + offset, vec![]);
        self.store(&event, self.channel, None).await;
        event
    }

    async fn store(&self, event: &Event, channel: Uuid, reply: Option<(&Event, bool)>) {
        let at = |event: &Event| {
            DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).expect("timestamp")
        };
        let meta = reply.map(|(parent, broadcast)| ThreadMetadataParams {
            event_id: event.id.as_bytes(),
            event_created_at: at(event),
            channel_id: channel,
            parent_event_id: Some(parent.id.as_bytes()),
            parent_event_created_at: Some(at(parent)),
            root_event_id: Some(parent.id.as_bytes()),
            root_event_created_at: Some(at(parent)),
            depth: 1,
            broadcast,
        });
        let (_, inserted) = self
            .db
            .insert_event_with_thread_metadata(self.community, event, Some(channel), meta)
            .await
            .expect("store event");
        assert!(inserted);
        // Tests observe the background writer, not only a read's repair.
        super::channel_heads::flush().await;
    }

    /// The stored row as written, without a read repairing it.
    async fn stored(&self) -> (Option<String>, Option<i64>) {
        sqlx::query_as(
            "SELECT head_text, extract(epoch FROM seen_at)::bigint FROM channel_heads \
             WHERE community_id = $1 AND channel_id = $2",
        )
        .bind(self.community.as_uuid())
        .bind(self.channel)
        .fetch_one(&self.pool)
        .await
        .expect("stored row")
    }

    async fn read(&self) -> ChannelHeads {
        self.db
            .channel_heads(self.community, &self.member.public_key(), None, None)
            .await
            .expect("read channel heads")
    }

    async fn row(&self) -> ChannelHead {
        let mut page = self.read().await;
        assert_eq!(page.channels.len(), 1);
        page.channels.remove(0)
    }

    /// Forget stored rows, as for history older than the table.
    async fn forget(&self) {
        sqlx::query("DELETE FROM channel_heads WHERE community_id = $1")
            .bind(self.community.as_uuid())
            .execute(&self.pool)
            .await
            .expect("forget rows");
    }

    async fn delete(&self, event: &Event) {
        assert!(self
            .db
            .soft_delete_event_and_update_thread(
                self.community,
                event.id.as_bytes().as_slice(),
                None,
                None
            )
            .await
            .expect("delete event"));
    }
}

fn head_text(row: &ChannelHead) -> Option<&str> {
    row.head.as_ref().map(|head| head.text.as_str())
}

async fn ingest_follows_the_channel_window_rule() {
    let f = fixture().await;
    let empty = f.row().await;
    assert_eq!((empty.activity_at, empty.head), (None, None));

    let first = f.post("first", 10).await;
    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 10) as i64));
    assert_eq!(row.activity_pubkey, Some(first.pubkey.to_hex()));
    let head = row.head.expect("head");
    assert_eq!(
        (
            head.id,
            head.pubkey,
            head.created_at,
            head.text,
            head.edited
        ),
        (
            first.id.to_hex(),
            first.pubkey.to_hex(),
            (f.base + 10) as i64,
            "first".to_owned(),
            false
        )
    );

    // A thread reply is activity, not the head.
    let reply = message(9, "in thread", f.base + 20, vec![]);
    f.store(&reply, f.channel, Some((&first, false))).await;
    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 20) as i64));
    assert_eq!(row.activity_pubkey, Some(reply.pubkey.to_hex()));
    assert_eq!(head_text(&row), Some("first"));

    // A reply broadcast to the channel is the head.
    let broadcast = message(9, "also to channel", f.base + 30, vec![]);
    f.store(&broadcast, f.channel, Some((&first, true))).await;
    assert_eq!(head_text(&f.row().await), Some("also to channel"));

    // A message only agents need is activity, not the head.
    let audience = Tag::parse(["audience", "agents"]).expect("tag");
    let quiet = message(9, "agent chatter", f.base + 40, vec![audience]);
    f.store(&quiet, f.channel, None).await;
    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 40) as i64));
    assert_eq!(head_text(&row), Some("also to channel"));

    // Other kinds and late older messages change nothing.
    f.store(&message(7, "+", f.base + 50, vec![]), f.channel, None)
        .await;
    f.post("late and old", 5).await;
    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 40) as i64));
    assert_eq!(head_text(&row), Some("also to channel"));

    // Within one second the lower id is the head, in either arrival order.
    let mut pair = [
        message(9, "tie a", f.base + 60, vec![]),
        message(9, "tie b", f.base + 60, vec![]),
    ];
    pair.sort_by_key(|event| event.id);
    f.store(&pair[0], f.channel, None).await;
    f.store(&pair[1], f.channel, None).await;
    assert_eq!(f.row().await.head.expect("head").id, pair[0].id.to_hex());
    let mut pair = [
        message(9, "tie c", f.base + 70, vec![]),
        message(9, "tie d", f.base + 70, vec![]),
    ];
    pair.sort_by_key(|event| event.id);
    f.store(&pair[1], f.channel, None).await;
    f.store(&pair[0], f.channel, None).await;
    assert_eq!(f.row().await.head.expect("head").id, pair[0].id.to_hex());

    // Text is cut by characters.
    f.post(&"é".repeat(400), 80).await;
    assert_eq!(head_text(&f.row().await), Some("é".repeat(256).as_str()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_ingest_follows_the_channel_window_rule() {
    ingest_follows_the_channel_window_rule().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_channel_heads_ingest_follows_the_channel_window_rule() {
    ingest_follows_the_channel_window_rule().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_edit_of_the_head_replaces_its_text() {
    let f = fixture().await;
    let old = f.post("older", 10).await;
    let head = f.post("typo", 20).await;
    let edit = |target: &Event, text: &str, offset: u64| {
        let tag = Tag::parse(["e", &target.id.to_hex()]).expect("tag");
        message(40003, text, f.base + offset, vec![tag])
    };
    f.store(&edit(&old, "not shown", 30), f.channel, None).await;
    let row = f.row().await;
    assert_eq!(head_text(&row), Some("typo"));
    assert!(!row.head.expect("head").edited);

    f.store(&edit(&head, "fixed", 40), f.channel, None).await;
    // An older edit arriving late does not win.
    f.store(&edit(&head, "stale edit", 35), f.channel, None)
        .await;
    let row = f.row().await;
    // Edits are not activity.
    assert_eq!(row.activity_at, Some((f.base + 20) as i64));
    let shown = row.head.expect("head");
    assert_eq!(
        (shown.id, shown.text.as_str(), shown.edited),
        (head.id.to_hex(), "fixed", true)
    );

    // Two edits in one second: the higher id wins, in either arrival order.
    let mut pair = [edit(&head, "tie a", 50), edit(&head, "tie b", 50)];
    pair.sort_by_key(|event| event.id);
    f.store(&pair[1], f.channel, None).await;
    f.store(&pair[0], f.channel, None).await;
    assert_eq!(head_text(&f.row().await), Some(pair[1].content.as_str()));
    // The fill picks the same one from stored history.
    f.forget().await;
    assert_eq!(head_text(&f.row().await), Some(pair[1].content.as_str()));
    for event in &pair {
        f.delete(event).await;
    }

    // The fill reads the remaining edit from stored history.
    f.forget().await;
    let shown = f.row().await.head.expect("head");
    assert_eq!((shown.text.as_str(), shown.edited), ("fixed", true));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_first_read_fills_history_older_than_the_row() {
    let f = fixture().await;
    let first = f.post("old top level", 10).await;
    let reply = message(9, "old reply", f.base + 20, vec![]);
    f.store(&reply, f.channel, Some((&first, false))).await;
    f.forget().await;

    // A new reply creates a row that knows only itself and claims nothing
    // about older history.
    let late = message(9, "new reply", f.base + 30, vec![]);
    f.store(&late, f.channel, Some((&first, false))).await;
    assert_eq!(f.stored().await, (None, None));

    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 30) as i64));
    assert_eq!(head_text(&row), Some("old top level"));
    assert_eq!(
        f.stored().await,
        (Some("old top level".to_owned()), Some((f.base + 30) as i64))
    );

    // A row that accounts for every stored event is served as it is.
    sqlx::query("UPDATE channel_heads SET head_text = 'kept' WHERE community_id = $1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("mark row");
    assert_eq!(head_text(&f.row().await), Some("kept"));

    // With no row at all, the read fills from history alone.
    f.forget().await;
    let row = f.row().await;
    assert_eq!(row.activity_at, Some((f.base + 30) as i64));
    assert_eq!(row.activity_pubkey, Some(late.pubkey.to_hex()));
    assert_eq!(head_text(&row), Some("old top level"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_deleted_head_falls_back_to_the_previous_message() {
    let f = fixture().await;
    f.post("stays", 10).await;
    let gone = f.post("deleted", 20).await;
    assert_eq!(head_text(&f.row().await), Some("deleted"));
    f.delete(&gone).await;
    assert_eq!(head_text(&f.row().await), Some("stays"));
    // A reconnecting reader is told, although deletion stamps no row.
    f.post("newest", 30).await;
    let as_of = f.read().await.as_of;
    sqlx::query(
        "UPDATE channel_heads SET updated_at = updated_at - interval '1 hour' \
         WHERE community_id = $1",
    )
    .bind(f.community.as_uuid())
    .execute(&f.pool)
    .await
    .expect("age rows");
    let newest: Vec<u8> =
        sqlx::query_scalar("SELECT head_id FROM channel_heads WHERE community_id = $1")
            .bind(f.community.as_uuid())
            .fetch_one(&f.pool)
            .await
            .expect("head");
    assert!(f
        .db
        .soft_delete_event_and_update_thread(f.community, &newest, None, None)
        .await
        .expect("delete newest"));
    let delta =
        f.db.channel_heads(f.community, &f.member.public_key(), Some(as_of), None)
            .await
            .expect("delta");
    assert_eq!(delta.channels.len(), 1);
    assert_eq!(head_text(&delta.channels[0]), Some("stays"));

    // An unverified row naming a deleted head is repaired in the same read.
    let unfilled = f.post("unfilled and deleted", 40).await;
    sqlx::query("UPDATE channel_heads SET seen_at = NULL WHERE community_id = $1")
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("unfill");
    f.delete(&unfilled).await;
    assert_eq!(head_text(&f.row().await), Some("stays"));

    // The repair is stored, not redone on every read.
    let stored: Option<String> = sqlx::query_scalar(
        "SELECT head_text FROM channel_heads WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .fetch_one(&f.pool)
    .await
    .expect("row");
    assert_eq!(stored.as_deref(), Some("stays"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_since_returns_only_changed_rows() {
    let f = fixture().await;
    let other = channel(&f.db, &f.pool, f.community, &f.member).await;
    f.post("one", 10).await;
    f.store(&message(9, "two", f.base + 20, vec![]), other, None)
        .await;
    let page = f.read().await;
    assert!(!page.truncated);
    // Most recent activity first.
    assert_eq!(
        page.channels
            .iter()
            .map(|c| c.channel_id)
            .collect::<Vec<_>>(),
        vec![other, f.channel]
    );
    // Rows last written outside the commit-skew window are not returned.
    sqlx::query(
        "UPDATE channel_heads SET updated_at = updated_at - interval '1 hour' \
         WHERE community_id = $1",
    )
    .bind(f.community.as_uuid())
    .execute(&f.pool)
    .await
    .expect("age rows");
    let delta = |since| {
        let f = &f;
        async move {
            f.db.channel_heads(f.community, &f.member.public_key(), Some(since), None)
                .await
                .expect("delta")
                .channels
        }
    };
    assert!(delta(page.as_of).await.is_empty());
    f.post("three", 30).await;
    let changed = delta(page.as_of).await;
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].channel_id, f.channel);
    assert_eq!(head_text(&changed[0]), Some("three"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_returns_only_current_memberships() {
    let f = fixture().await;
    let left = channel(&f.db, &f.pool, f.community, &f.member).await;
    let deleted = channel(&f.db, &f.pool, f.community, &f.member).await;
    let stranger = Keys::generate();
    let foreign = channel(&f.db, &f.pool, f.community, &stranger).await;
    for (index, id) in [f.channel, left, deleted, foreign].into_iter().enumerate() {
        f.store(
            &message(9, "hello", f.base + index as u64, vec![]),
            id,
            None,
        )
        .await;
    }
    sqlx::query(
        "UPDATE channel_members SET removed_at = clock_timestamp() \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(f.community.as_uuid())
    .bind(left)
    .bind(f.member.public_key().to_bytes().as_slice())
    .execute(&f.pool)
    .await
    .expect("leave");
    assert!(f
        .db
        .soft_delete_channel(f.community, deleted)
        .await
        .expect("delete channel"));

    let page = f.read().await;
    assert_eq!(
        page.channels
            .iter()
            .map(|c| c.channel_id)
            .collect::<Vec<_>>(),
        vec![f.channel]
    );
    // Asking by id returns only joined channels, never another's row.
    let asked =
        f.db.channel_heads(
            f.community,
            &f.member.public_key(),
            None,
            Some(&[f.channel, left, deleted, foreign]),
        )
        .await
        .expect("by id");
    assert_eq!(asked.channels.len(), 1);
    assert_eq!(asked.channels[0].channel_id, f.channel);
    let theirs =
        f.db.channel_heads(f.community, &stranger.public_key(), None, None)
            .await
            .expect("stranger");
    assert_eq!(
        theirs
            .channels
            .iter()
            .map(|c| c.channel_id)
            .collect::<Vec<_>>(),
        vec![foreign]
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_ingest_writes_the_row_without_a_read() {
    let f = fixture().await;
    f.post("first", 10).await;
    f.row().await;
    // Once a read has filled the row, the writer alone keeps it current.
    f.post("second", 20).await;
    assert_eq!(
        f.stored().await,
        (Some("second".to_owned()), Some((f.base + 20) as i64))
    );
    f.store(&message(7, "+", f.base + 30, vec![]), f.channel, None)
        .await;
    assert_eq!(
        f.stored().await,
        (Some("second".to_owned()), Some((f.base + 30) as i64))
    );

    // A fast clock cannot make the row account for the future: it stays
    // behind until that event is due, so nothing stored meanwhile is hidden.
    f.store(&message(7, "+", f.base + 900, vec![]), f.channel, None)
        .await;
    let (_, seen) = f.stored().await;
    assert!(seen.expect("seen") < (f.base + 700) as i64);
    f.row().await;
    let (_, seen) = f.stored().await;
    assert!(seen.expect("seen") < (f.base + 700) as i64);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_heads_lost_row_update_is_repaired_by_the_next_read() {
    let f = fixture().await;
    f.post("before", 10).await;
    assert_eq!(head_text(&f.row().await), Some("before"));
    let as_of = f.read().await.as_of;
    sqlx::query(
        "UPDATE channel_heads SET updated_at = updated_at - interval '1 hour' \
         WHERE community_id = $1",
    )
    .bind(f.community.as_uuid())
    .execute(&f.pool)
    .await
    .expect("age rows");

    // The message commits although its row update cannot be written.
    sqlx::query("ALTER TABLE channel_heads RENAME TO channel_heads_away")
        .execute(&f.pool)
        .await
        .expect("break upkeep");
    let event = f.post("still stored", 20).await;
    sqlx::query("ALTER TABLE channel_heads_away RENAME TO channel_heads")
        .execute(&f.pool)
        .await
        .expect("restore");
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = $2")
            .bind(f.community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(&f.pool)
            .await
            .expect("count");
    assert_eq!(stored, 1);
    assert_eq!(
        f.stored().await,
        (Some("before".to_owned()), Some((f.base + 10) as i64))
    );

    // The read sees an event newer than the row accounts for and rebuilds
    // it, for a reconnecting reader too, and stores the result.
    let delta =
        f.db.channel_heads(f.community, &f.member.public_key(), Some(as_of), None)
            .await
            .expect("delta");
    assert_eq!(delta.channels.len(), 1);
    assert_eq!(head_text(&delta.channels[0]), Some("still stored"));
    assert_eq!(
        f.stored().await,
        (Some("still stored".to_owned()), Some((f.base + 20) as i64))
    );
}
