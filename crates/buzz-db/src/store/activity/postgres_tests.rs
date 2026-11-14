//! PostgreSQL contract for the activity list: ingest upkeep, unread, read
//! intents, mentions, paging, `since`, repair on read and scoping.
use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    event::ThreadMetadataParams,
    migration,
    personal_read::{IntentOutcome, ReadIntent, ReadState, ReadTarget, DEFAULT_RETENTION_SECONDS},
    Db,
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
    /// The person whose list is read.
    me: Keys,
    /// Someone else in every channel.
    other: Keys,
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
        .ensure_configured_community(&format!("activity-{}.local", Uuid::new_v4()))
        .await
        .expect("create community")
        .id;
    let mut f = Fixture {
        db,
        pool,
        community,
        channel: Uuid::nil(),
        me: Keys::generate(),
        other: Keys::generate(),
        base: Timestamp::now().as_secs() - 600,
    };
    f.channel = f.channel().await;
    f
}

fn all() -> Query {
    Query {
        kinds: RowKind::ALL.to_vec(),
        unread_only: false,
        select: Select::Page {
            before: None,
            limit: DEFAULT_LIMIT,
        },
    }
}

impl Fixture {
    /// A channel both people are in.
    async fn channel(&self) -> Uuid {
        let id = self
            .db
            .create_channel(
                self.community,
                &format!("activity-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &self.other.public_key().to_bytes(),
                None,
            )
            .await
            .expect("create channel")
            .id;
        for member in [&self.me, &self.other] {
            sqlx::query(
                "INSERT INTO channel_members (community_id, channel_id, pubkey) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(self.community.as_uuid())
            .bind(id)
            .bind(member.public_key().to_bytes().as_slice())
            .execute(&self.pool)
            .await
            .expect("join channel");
        }
        id
    }

    fn event(&self, author: &Keys, kind: u16, text: &str, offset: u64, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::Custom(kind), text)
            .tags(tags)
            .custom_created_at(Timestamp::from(self.base + offset))
            .sign_with_keys(author)
            .expect("sign test event")
    }

    /// Store a message as the relay does: the event, then its mention rows
    /// in a later transaction.
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
        crate::insert_mentions(&self.pool, self.community, event, Some(channel))
            .await
            .expect("store mentions");
        // Tests observe the background writer, not only a read's repair.
        flush().await;
        // Arrival times order the list; keep them distinct.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    async fn post(&self, author: &Keys, text: &str, offset: u64) -> Event {
        let event = self.event(author, 9, text, offset, vec![]);
        self.store(&event, self.channel, None).await;
        event
    }

    async fn reply(&self, author: &Keys, parent: &Event, text: &str, offset: u64) -> Event {
        let event = self.event(author, 9, text, offset, vec![]);
        self.store(&event, self.channel, Some((parent, false)))
            .await;
        event
    }

    fn tag_me(&self) -> Tag {
        Tag::parse(["p", &self.me.public_key().to_hex()]).expect("tag")
    }

    async fn read(&self, query: &Query) -> Page {
        self.db
            .activity(
                self.community,
                &self.me.public_key(),
                DEFAULT_RETENTION_SECONDS,
                query,
            )
            .await
            .expect("read activity")
    }

    async fn list(&self) -> Page {
        self.read(&all()).await
    }

    /// The one row with this id.
    async fn row(&self, id: &str) -> Row {
        let rows: Vec<Row> = self
            .list()
            .await
            .rows
            .into_iter()
            .filter(|row| row.id == id)
            .collect();
        assert_eq!(rows.len(), 1, "one row with id {id}");
        rows.into_iter().next().expect("row")
    }

    async fn conversation(&self) -> Row {
        self.row(&self.channel.to_string()).await
    }

    fn target(&self, root: Option<&Event>) -> ReadTarget {
        ReadTarget {
            channel_id: self.channel,
            root_id: root.map(|root| root.id.to_hex()),
        }
    }

    async fn intent(&self, intent: ReadIntent) -> IntentOutcome {
        self.db
            .apply_personal_read_intent(self.community, &self.me.public_key(), &intent)
            .await
            .expect("apply intent")
    }

    async fn mark_through(&self, root: Option<&Event>, message: &Event) -> IntentOutcome {
        self.intent(ReadIntent::MarkThrough {
            target: self.target(root),
            message_id: message.id.to_hex(),
        })
        .await
    }

    /// Forget shared rows, as for history older than the tables.
    async fn forget(&self) {
        for table in ["channel_heads", "thread_heads"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {table} WHERE community_id = $1"
            )))
            .bind(self.community.as_uuid())
            .execute(&self.pool)
            .await
            .expect("forget rows");
        }
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

fn preview_text(row: &Row) -> Option<&str> {
    row.preview.as_ref().map(|preview| preview.text.as_str())
}

/// A conversation goes unread on someone else's message and is read by
/// marking through it; flags and states change what unread means.
async fn conversation_unread_follows_read_intents() {
    let f = fixture().await;
    f.post(&f.other, "before I started", 10).await;
    // The first read starts the account: nothing older is unread.
    let page = f.list().await;
    assert_eq!(page.rows.len(), 1);
    assert!(!page.rows[0].unread);
    assert_eq!(page.totals.expect("totals"), Totals::default());
    assert!(page.marker.is_some());

    let hello = f.post(&f.other, "hello", 20).await;
    let row = f.conversation().await;
    assert_eq!(row.kind, RowKind::Channel);
    assert!(row.unread);
    assert_eq!(preview_text(&row), Some("hello"));
    assert_eq!(row.activity.id.as_deref(), Some(hello.id.to_hex().as_str()));
    assert_eq!(
        row.preview.as_ref().map(|p| p.created_at),
        Some((f.base + 20) as i64)
    );
    assert_eq!(f.list().await.totals.expect("totals").channel.unread, 1);

    // Marking the row read marks through its preview.
    assert_eq!(f.mark_through(None, &hello).await, IntentOutcome::Applied);
    let row = f.conversation().await;
    assert!(!row.unread);
    assert_eq!(row.read_through_id, Some(hello.id.to_hex()));
    // The position never moves back.
    let newer = f.post(&f.other, "newer", 30).await;
    assert_eq!(f.mark_through(None, &newer).await, IntentOutcome::Applied);
    assert_eq!(f.mark_through(None, &hello).await, IntentOutcome::Applied);
    assert_eq!(
        f.conversation().await.read_through_id,
        Some(newer.id.to_hex())
    );

    // My own post is read by me, and is the preview.
    f.post(&f.other, "theirs", 40).await;
    assert!(f.conversation().await.unread);
    let mine = f.post(&f.me, "mine", 50).await;
    let row = f.conversation().await;
    assert!(!row.unread);
    assert_eq!(preview_text(&row), Some("mine"));
    assert_eq!(row.read_through_id, Some(mine.id.to_hex()));

    // A manual unread lasts until the next mark_through.
    let flag = ReadIntent::MarkUnread {
        target: f.target(None),
    };
    assert_eq!(f.intent(flag).await, IntentOutcome::Applied);
    assert!(f.conversation().await.unread);
    // My own later post moves my position but leaves the flag: its write runs
    // after the post committed and cannot tell which came first.
    let again = f.post(&f.me, "mine again", 35).await;
    assert!(f.conversation().await.unread);
    assert_eq!(f.mark_through(None, &again).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);

    // Muted is never unread; back to default it is again.
    let state = |state| ReadIntent::SetState {
        target: f.target(None),
        state,
    };
    assert_eq!(
        f.intent(state(ReadState::Muted)).await,
        IntentOutcome::Applied
    );
    f.post(&f.other, "while muted", 60).await;
    let row = f.conversation().await;
    assert_eq!((row.unread, row.state), (false, "muted"));
    assert_eq!(f.list().await.totals.expect("totals").channel.unread, 0);
    assert_eq!(
        f.intent(state(ReadState::Default)).await,
        IntentOutcome::Applied
    );
    assert!(f.conversation().await.unread);

    // The unread filter lists only unread rows.
    let quiet = f.channel().await;
    let unread_only = Query {
        unread_only: true,
        ..all()
    };
    let rows = f.read(&unread_only).await.rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].channel_id, f.channel);
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .any(|row| row.channel_id == quiet && row.preview.is_none()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_conversation_unread_follows_read_intents() {
    conversation_unread_follows_read_intents().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_activity_conversation_unread_follows_read_intents() {
    conversation_unread_follows_read_intents().await;
}

/// A reply pulls the parent's author and the people it tags into the thread;
/// the thread row and the mention row follow the thread's read position.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_threads_and_mentions_follow_the_thread_position() {
    let f = fixture().await;
    f.list().await;
    let root = f.post(&f.me, "my question", 10).await;
    // Nobody replied: no thread row.
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.kind != RowKind::Thread));

    let answer = f.reply(&f.other, &root, "an answer", 20).await;
    let thread = f.row(&root.id.to_hex()).await;
    assert_eq!(thread.kind, RowKind::Thread);
    assert!(thread.unread);
    assert_eq!(thread.reply_count, Some(1));
    assert_eq!(preview_text(&thread), Some("an answer"));
    assert_eq!(thread.root_id, Some(root.id.to_hex()));
    // A reply that is not broadcast is not channel activity to read.
    assert!(!f.conversation().await.unread);

    // A reply that tags me is also a mention row, counted on its thread.
    let ping = f.event(&f.other, 9, "ping", 30, vec![f.tag_me()]);
    f.store(&ping, f.channel, Some((&root, false))).await;
    let page = f.list().await;
    let totals = page.totals.expect("totals");
    assert_eq!((totals.thread.unread, totals.thread.mentions), (1, 1));
    assert_eq!(totals.mention.unread, 1);
    let mention = f.row(&ping.id.to_hex()).await;
    assert_eq!(mention.kind, RowKind::Mention);
    assert!(mention.unread);
    assert_eq!(mention.root_id, Some(root.id.to_hex()));
    assert_eq!(preview_text(&mention), Some("ping"));
    let thread = f.row(&root.id.to_hex()).await;
    assert_eq!((thread.mentions, thread.reply_count), (1, Some(2)));
    // One message is the newest activity of all three rows: at the same
    // arrival a conversation sorts first, then a thread, then a mention.
    let order: Vec<RowKind> = page.rows.iter().map(|row| row.kind).collect();
    assert_eq!(order, [RowKind::Channel, RowKind::Thread, RowKind::Mention]);

    // Reading the thread through the first reply leaves the ping unread.
    assert_eq!(
        f.mark_through(Some(&root), &answer).await,
        IntentOutcome::Applied
    );
    assert!(f.row(&root.id.to_hex()).await.unread);
    assert_eq!(
        f.mark_through(Some(&root), &ping).await,
        IntentOutcome::Applied
    );
    let thread = f.row(&root.id.to_hex()).await;
    assert_eq!((thread.unread, thread.mentions), (false, 0));
    assert!(!f.row(&ping.id.to_hex()).await.unread);
    assert_eq!(f.list().await.totals.expect("totals").mention.unread, 0);

    // Someone else's thread is not mine until I am tagged or follow it.
    let theirs = f.post(&f.other, "their topic", 40).await;
    f.reply(&f.other, &theirs, "talking to myself", 50).await;
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.id != theirs.id.to_hex()));
    let follow = ReadIntent::SetState {
        target: f.target(Some(&theirs)),
        state: ReadState::Following,
    };
    assert_eq!(f.intent(follow).await, IntentOutcome::Applied);
    let followed = f.row(&theirs.id.to_hex()).await;
    assert_eq!((followed.unread, followed.state), (true, "following"));

    // Unfollowing takes it off the list, and a `since` read says so.
    let marker = f.list().await.marker.expect("marker");
    let unfollow = ReadIntent::SetState {
        target: f.target(Some(&theirs)),
        state: ReadState::Default,
    };
    assert_eq!(f.intent(unfollow).await, IntentOutcome::Applied);
    let page = f
        .read(&Query {
            select: Select::Since(marker),
            ..all()
        })
        .await;
    assert_eq!(
        page.removed,
        [Removed {
            kind: RowKind::Thread,
            id: theirs.id.to_hex()
        }]
    );
    assert!(page.rows.iter().all(|row| row.id != theirs.id.to_hex()));
    // A flagged thread is listed, and rebuilt when its shared row is missing.
    f.forget().await;
    let flag = ReadIntent::MarkUnread {
        target: f.target(Some(&theirs)),
    };
    assert_eq!(f.intent(flag).await, IntentOutcome::Applied);
    assert!(f.row(&theirs.id.to_hex()).await.unread);

    // A mention at the top level counts on the channel.
    let shout = f.event(&f.other, 9, "hey you", 60, vec![f.tag_me()]);
    f.store(&shout, f.channel, None).await;
    let row = f.conversation().await;
    assert_eq!((row.unread, row.mentions), (true, 1));
    // A deleted mention is neither listed nor counted.
    f.delete(&shout).await;
    assert_eq!(f.conversation().await.mentions, 0);
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.id != shout.id.to_hex()));

    // A deleted channel's mentions leave the totals with its rows.
    let last = f.event(&f.other, 9, "still there?", 70, vec![f.tag_me()]);
    f.store(&last, f.channel, None).await;
    assert_eq!(f.list().await.totals.expect("totals").mention.unread, 1);
    assert!(f
        .db
        .soft_delete_channel(f.community, f.channel)
        .await
        .expect("delete channel"));
    let page = f.list().await;
    assert_eq!(page.totals.expect("totals").mention.unread, 0);
    assert!(page.rows.iter().all(|row| row.channel_id != f.channel));
}

/// Marking is bound to the timeline the anchor is in.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_mark_through_takes_only_an_anchor_of_its_timeline() {
    let f = fixture().await;
    let marker = f.list().await.marker.expect("marker");
    let root = f.post(&f.other, "root", 10).await;
    let reply = f.reply(&f.other, &root, "reply", 20).await;
    let broadcast = f.event(&f.other, 9, "also to channel", 30, vec![]);
    f.store(&broadcast, f.channel, Some((&root, true))).await;

    // A broadcast reply is the channel's newest top-level message.
    let row = f.conversation().await;
    assert_eq!(preview_text(&row), Some("also to channel"));
    assert_eq!(f.mark_through(None, &reply).await, IntentOutcome::Blocked);
    assert_eq!(
        f.mark_through(None, &broadcast).await,
        IntentOutcome::Applied
    );
    assert!(!f.conversation().await.unread);

    let elsewhere = f.post(&f.other, "another root", 40).await;
    assert_eq!(
        f.mark_through(Some(&elsewhere), &reply).await,
        IntentOutcome::Blocked
    );
    let invalid = ReadIntent::MarkThrough {
        target: f.target(None),
        message_id: "nope".into(),
    };
    assert_eq!(f.intent(invalid).await, IntentOutcome::Invalid);
    // Only the applied intent moved the person's counter.
    assert_eq!(f.list().await.marker.expect("marker").seq, marker.seq + 1);
}

/// Pages walk the whole list once, in order, across kinds.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_pages_cover_every_row_once_in_order() {
    let f = fixture().await;
    f.list().await;
    let mut expected = Vec::new();
    for n in 0..3u64 {
        let root = f.post(&f.me, &format!("root {n}"), 10 + n * 10).await;
        let ping = f.event(&f.other, 9, "ping", 15 + n * 10, vec![f.tag_me()]);
        f.store(&ping, f.channel, Some((&root, false))).await;
        // Same arrival: the thread sorts before the mention.
        expected.push(vec![
            (RowKind::Thread, root.id.to_hex()),
            (RowKind::Mention, ping.id.to_hex()),
        ]);
    }
    let newest_thread = expected[2][0].1.clone();
    // The channel's newest activity is the last ping, and at the same
    // arrival a conversation sorts first.
    expected.push(vec![(RowKind::Channel, f.channel.to_string())]);
    // A conversation with no message sorts by when it was created.
    let empty = f.channel().await;
    expected.push(vec![(RowKind::Channel, empty.to_string())]);
    expected.reverse();
    let expected: Vec<(RowKind, String)> = expected.into_iter().flatten().collect();

    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = f
            .read(&Query {
                select: Select::Page { before, limit: 2 },
                ..all()
            })
            .await;
        assert!(page.rows.len() <= 2);
        // Totals and the marker come with the first page only.
        assert_eq!(page.totals.is_some(), seen.is_empty());
        assert_eq!(page.marker.is_some(), seen.is_empty());
        seen.extend(page.rows.iter().map(|row| (row.kind, row.id.clone())));
        before = page.next;
        if before.is_none() {
            break;
        }
    }
    assert_eq!(seen, expected);

    // A kind filter pages the same way.
    let threads = f
        .read(&Query {
            kinds: vec![RowKind::Thread],
            ..all()
        })
        .await;
    assert_eq!(threads.rows.len(), 3);
    // Named rows, in list order.
    let named = f
        .read(&Query {
            select: Select::Ids {
                channels: vec![f.channel],
                roots: vec![hex::decode(&newest_thread)
                    .expect("hex")
                    .try_into()
                    .expect("id")],
                mentions: vec![],
            },
            ..all()
        })
        .await;
    let ids: Vec<&str> = named.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [f.channel.to_string().as_str(), newest_thread.as_str()]
    );
    assert!(named.totals.is_none());
}

/// A `since` read returns what changed after its marker, and what left.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_since_returns_changes_and_removals() {
    let f = fixture().await;
    let quiet = f.channel().await;
    let first = f.post(&f.other, "first", 10).await;
    let marker = f.list().await.marker.expect("marker");
    let since = |marker: Marker| Query {
        select: Select::Since(marker),
        ..all()
    };
    // Rows stamped inside the commit-skew window are sent again: at least
    // once, never missed. Age them out to see the quiet case.
    let age = || async {
        sqlx::query(
            "UPDATE channel_heads SET updated_at = updated_at - interval '1 hour' \
             WHERE community_id = $1",
        )
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age rows");
        sqlx::query(
            "UPDATE channel_members SET sync_modified_at = sync_modified_at - interval '1 hour' \
             WHERE community_id = $1",
        )
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age memberships");
    };
    age().await;
    let page = f.read(&since(marker)).await;
    assert!(page.rows.is_empty() && page.removed.is_empty() && !page.reset);
    assert!(page.totals.is_some());

    // A shared change: a new message.
    f.post(&f.other, "second", 20).await;
    let page = f.read(&since(marker)).await;
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, [f.channel.to_string().as_str()]);
    assert!(page.rows[0].unread);
    let marker = page.marker.expect("marker");

    // A personal change: my own read intent, found by counter.
    age().await;
    assert_eq!(f.mark_through(None, &first).await, IntentOutcome::Applied);
    let page = f.read(&since(marker)).await;
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].read_through_id, Some(first.id.to_hex()));
    let marker = page.marker.expect("marker");
    assert_eq!(f.read(&since(marker)).await.rows.len(), 0);

    // Leaving a channel removes its row.
    f.db.remove_member(
        f.community,
        quiet,
        &f.me.public_key().to_bytes(),
        &f.me.public_key().to_bytes(),
    )
    .await
    .expect("leave");
    let page = f.read(&since(marker)).await;
    assert_eq!(
        page.removed,
        [Removed {
            kind: RowKind::Channel,
            id: quiet.to_string()
        }]
    );
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.channel_id != quiet));

    // A marker past the horizon, or from another counter, starts over.
    let old = Marker {
        seq: marker.seq,
        micros: marker.micros - (HORIZON_SECONDS + 60) * 1_000_000,
    };
    assert!(f.read(&since(old)).await.reset);
    let ahead = Marker {
        seq: marker.seq + 100,
        ..marker
    };
    assert!(f.read(&since(ahead)).await.reset);
}

/// Shared rows are a cache: a read rebuilds what is missing or stale.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_read_rebuilds_missing_and_stale_rows() {
    let f = fixture().await;
    f.list().await;
    let root = f.post(&f.me, "root", 10).await;
    let reply = f.reply(&f.other, &root, "reply", 20).await;
    let typo = f.post(&f.other, "typo", 30).await;
    let edit = f.event(
        &f.other,
        40003,
        "fixed",
        40,
        vec![Tag::parse(["e", &typo.id.to_hex()]).expect("tag")],
    );
    f.store(&edit, f.channel, None).await;
    let before = (f.conversation().await, f.row(&root.id.to_hex()).await);
    assert_eq!(preview_text(&before.0), Some("fixed"));
    assert!(before.0.preview.as_ref().expect("preview").edited);
    // Edits are not activity.
    assert_eq!(
        before.0.activity.id.as_deref(),
        Some(typo.id.to_hex().as_str())
    );

    // History older than the tables: the first read fills the same rows.
    f.forget().await;
    let after = (f.conversation().await, f.row(&root.id.to_hex()).await);
    assert_eq!(after, before);

    // A deleted message is never shown; the row falls back.
    f.delete(&typo).await;
    let row = f.conversation().await;
    assert_eq!(preview_text(&row), Some("root"));
    f.delete(&reply).await;
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.kind != RowKind::Thread));

    // A message stored while the writer was down is found by the next read.
    let lost = f.event(&f.other, 9, "lost update", 50, vec![]);
    f.store(&lost, f.channel, None).await;
    f.forget().await;
    assert_eq!(preview_text(&f.conversation().await), Some("lost update"));
}

/// Rows never cross communities or outlive membership.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_lists_only_current_memberships_of_one_community() {
    let f = fixture().await;
    f.list().await;
    let root = f.post(&f.me, "root", 10).await;
    let ping = f.event(&f.other, 9, "ping", 20, vec![f.tag_me()]);
    f.store(&ping, f.channel, Some((&root, false))).await;
    assert_eq!(f.list().await.rows.len(), 3);

    let elsewhere =
        f.db.ensure_configured_community(&format!("activity-other-{}.local", Uuid::new_v4()))
            .await
            .expect("create community")
            .id;
    let other =
        f.db.activity(
            elsewhere,
            &f.me.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &all(),
        )
        .await
        .expect("read other community");
    assert!(other.rows.is_empty());

    f.db.remove_member(
        f.community,
        f.channel,
        &f.me.public_key().to_bytes(),
        &f.me.public_key().to_bytes(),
    )
    .await
    .expect("leave");
    let page = f.list().await;
    assert!(page.rows.is_empty());
    assert_eq!(page.totals.expect("totals"), Totals::default());
    // Intents on a private channel I left are refused; an open one I can
    // still read keeps taking them.
    assert_eq!(f.mark_through(None, &root).await, IntentOutcome::Applied);
    sqlx::query("UPDATE channels SET visibility = 'private' WHERE id = $1")
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .expect("make private");
    assert_eq!(f.mark_through(None, &root).await, IntentOutcome::Blocked);
}
