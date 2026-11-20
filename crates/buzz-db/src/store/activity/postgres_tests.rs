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
        kinds: RowKind::LIST.to_vec(),
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

    /// One page of my mention feed.
    async fn mentions(&self, before: Option<Cursor>, limit: usize, unread: bool) -> Page {
        self.read(&Query {
            select: Select::Mentions {
                before,
                limit,
                unread,
            },
            ..all()
        })
        .await
    }

    /// `(id, unread)` of the newest mentions, or of the unread ones.
    async fn mention_ids(&self, unread: bool) -> Vec<(String, bool)> {
        self.mentions(None, DEFAULT_LIMIT, unread)
            .await
            .rows
            .into_iter()
            .map(|row| (row.id, row.unread))
            .collect()
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

    /// Age the shared rows' change stamps out of the commit-skew window, so
    /// `since` can report a row only through the person's own counter.
    async fn quiet(&self) {
        for sql in [
            "UPDATE channel_heads SET updated_at = updated_at - interval '1 hour' \
             WHERE community_id = $1",
            "UPDATE thread_heads SET updated_at = updated_at - interval '1 hour' \
             WHERE community_id = $1",
            "UPDATE channel_members SET sync_modified_at = sync_modified_at - interval '1 hour' \
             WHERE community_id = $1",
        ] {
            sqlx::query(sql)
                .bind(self.community.as_uuid())
                .execute(&self.pool)
                .await
                .expect("age shared rows");
        }
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
    assert_eq!(
        row.preview.as_ref().map(|p| p.id.clone()),
        Some(hello.id.to_hex())
    );
    assert_eq!(
        row.preview.as_ref().map(|p| p.created_at),
        Some((f.base + 20) as i64)
    );
    assert_eq!(f.list().await.totals.expect("totals").channel, 1);

    // Marking the row read marks through its preview.
    assert_eq!(f.mark_through(None, &hello).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);
    // The position never moves back.
    let newer = f.post(&f.other, "newer", 30).await;
    assert!(f.conversation().await.unread);
    assert_eq!(f.mark_through(None, &newer).await, IntentOutcome::Applied);
    assert_eq!(f.mark_through(None, &hello).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);

    // My own post is read by me, and is the preview.
    f.post(&f.other, "theirs", 40).await;
    assert!(f.conversation().await.unread);
    f.post(&f.me, "mine", 50).await;
    let row = f.conversation().await;
    assert!(!row.unread);
    assert_eq!(preview_text(&row), Some("mine"));

    // A manual unread lasts until the next mark_through.
    let flag = ReadIntent::MarkUnread {
        target: f.target(None),
        message_id: None,
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
    assert_eq!(f.list().await.totals.expect("totals").channel, 0);
    assert_eq!(
        f.intent(state(ReadState::Default)).await,
        IntentOutcome::Applied
    );
    assert!(f.conversation().await.unread);

    // A conversation with no message is listed, with no preview.
    let quiet = f.channel().await;
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
    assert_eq!(preview_text(&thread), Some("an answer"));
    assert_eq!(thread.root_id, Some(root.id.to_hex()));
    // A reply that is not broadcast is not channel activity: the
    // conversation is not unread and keeps its place and preview.
    let row = f.conversation().await;
    assert!(!row.unread && row.sort_at < thread.sort_at);
    assert_eq!(preview_text(&row), Some("my question"));

    // A reply that tags me is also a mention: counted with the list, and a
    // row of the mention feed, never of the list.
    let ping = f.event(&f.other, 9, "ping", 30, vec![f.tag_me()]);
    f.store(&ping, f.channel, Some((&root, false))).await;
    let page = f.list().await;
    let totals = page.totals.expect("totals");
    assert_eq!((totals.thread, totals.mention), (1, 1));
    // The conversation sorts by its own preview, which is older.
    let order: Vec<RowKind> = page.rows.iter().map(|row| row.kind).collect();
    assert_eq!(order, [RowKind::Thread, RowKind::Channel]);
    let feed = f.mentions(None, DEFAULT_LIMIT, false).await;
    assert!(feed.totals.is_none() && feed.marker.is_none() && feed.next.is_none());
    let [mention] = feed.rows.as_slice() else {
        panic!("one mention, got {:?}", feed.rows);
    };
    assert_eq!(mention.kind, RowKind::Mention);
    assert_eq!(mention.id, ping.id.to_hex());
    assert!(mention.unread);
    assert_eq!(mention.root_id, Some(root.id.to_hex()));
    assert_eq!(preview_text(mention), Some("ping"));
    // A mention cannot be asked for with the list.
    assert!(f
        .db
        .activity(
            f.community,
            &f.me.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &Query {
                kinds: RowKind::ALL.to_vec(),
                ..all()
            },
        )
        .await
        .is_err());

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
    assert!(!f.row(&root.id.to_hex()).await.unread);
    // The mention is read with its thread: it stays in the feed and leaves
    // the unread one.
    assert_eq!(f.mention_ids(false).await, [(ping.id.to_hex(), false)]);
    assert_eq!(f.mention_ids(true).await, []);
    assert_eq!(f.list().await.totals.expect("totals").mention, 0);

    // Someone else's thread is not mine until I am tagged, or set its state.
    let theirs = f.post(&f.other, "their topic", 40).await;
    f.reply(&f.other, &theirs, "talking to myself", 50).await;
    assert!(f
        .list()
        .await
        .rows
        .iter()
        .all(|row| row.id != theirs.id.to_hex()));
    let mute = ReadIntent::SetState {
        target: f.target(Some(&theirs)),
        state: ReadState::Muted,
    };
    assert_eq!(f.intent(mute).await, IntentOutcome::Applied);
    let muted = f.row(&theirs.id.to_hex()).await;
    assert_eq!((muted.unread, muted.state), (false, "muted"));

    // Unmuting takes it off the list, and a `since` read says so.
    let marker = f.list().await.marker.expect("marker");
    let unmute = ReadIntent::SetState {
        target: f.target(Some(&theirs)),
        state: ReadState::Default,
    };
    assert_eq!(f.intent(unmute).await, IntentOutcome::Applied);
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
        message_id: None,
    };
    assert_eq!(f.intent(flag).await, IntentOutcome::Applied);
    assert!(f.row(&theirs.id.to_hex()).await.unread);

    // A mention at the top level follows the channel's read position.
    let shout = f.event(&f.other, 9, "hey you", 60, vec![f.tag_me()]);
    f.store(&shout, f.channel, None).await;
    assert!(f.conversation().await.unread);
    assert_eq!(f.list().await.totals.expect("totals").mention, 1);
    // The unread feed skips the read mention that is newer in no way but
    // order: only the unread one passes.
    assert_eq!(f.mention_ids(true).await, [(shout.id.to_hex(), true)]);
    assert_eq!(f.mention_ids(false).await.len(), 2);
    // A deleted mention is neither listed nor counted.
    f.delete(&shout).await;
    assert_eq!(f.list().await.totals.expect("totals").mention, 0);
    assert_eq!(f.mention_ids(false).await, [(ping.id.to_hex(), false)]);

    // A deleted channel's mentions leave the totals with its rows.
    let last = f.event(&f.other, 9, "still there?", 70, vec![f.tag_me()]);
    f.store(&last, f.channel, None).await;
    assert_eq!(f.list().await.totals.expect("totals").mention, 1);
    assert!(f
        .db
        .soft_delete_channel(f.community, f.channel)
        .await
        .expect("delete channel"));
    let page = f.list().await;
    assert_eq!(page.totals.expect("totals").mention, 0);
    assert!(page.rows.iter().all(|row| row.channel_id != f.channel));
    assert_eq!(f.mention_ids(false).await, []);
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
    let mut pings = Vec::new();
    for n in 0..3u64 {
        let root = f.post(&f.me, &format!("root {n}"), 10 + n * 10).await;
        let ping = f.event(&f.other, 9, "ping", 15 + n * 10, vec![f.tag_me()]);
        f.store(&ping, f.channel, Some((&root, false))).await;
        expected.push(vec![(RowKind::Thread, root.id.to_hex())]);
        pings.push(ping.id.to_hex());
    }
    let newest_thread = expected[2][0].1.clone();
    // The channel sorts by its preview, the last root: after that root's
    // thread, whose newest reply is later, and before the older ones.
    expected.insert(2, vec![(RowKind::Channel, f.channel.to_string())]);
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

    // The mention feed pages on its own, newest first.
    pings.reverse();
    let first = f.mentions(None, 2, false).await;
    let ids = |page: &Page| -> Vec<String> { page.rows.iter().map(|row| row.id.clone()).collect() };
    assert_eq!(ids(&first), pings[..2]);
    let rest = f.mentions(first.next, 2, false).await;
    assert_eq!(ids(&rest), pings[2..]);
    assert!(rest.next.is_none());
    // A page scans a bounded run of my mentions. When none of a run can be
    // shown (here: mentions whose messages are gone), the page is empty and
    // still says where to continue, so paging reaches the ones beyond it.
    sqlx::query(
        "INSERT INTO event_mentions (community_id, pubkey_hex, event_id, event_created_at, \
            channel_id, event_kind, received_at) \
         SELECT $1, $2, sha256(n::text::bytea), now(), $3, 9, now() + n * interval '1 microsecond' \
         FROM generate_series(1, $4) AS n",
    )
    .bind(f.community.as_uuid())
    .bind(f.me.public_key().to_hex())
    .bind(f.channel)
    .bind(MENTION_SCAN)
    .execute(&f.pool)
    .await
    .expect("add unshowable mentions");
    let empty_page = f.mentions(None, 2, false).await;
    assert!(empty_page.rows.is_empty() && empty_page.next.is_some());
    assert_eq!(
        ids(&f.mentions(empty_page.next, 2, false).await),
        pings[..2]
    );

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
            },
            ..all()
        })
        .await;
    let ids: Vec<&str> = named.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [newest_thread.as_str(), f.channel.to_string().as_str()]
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
        for table in ["channel_heads", "thread_heads"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE {table} SET updated_at = updated_at - interval '1 hour' \
                 WHERE community_id = $1"
            )))
            .bind(f.community.as_uuid())
            .execute(&f.pool)
            .await
            .expect("age rows");
        }
        sqlx::query(
            "UPDATE personal_read_frontiers SET \
                through_timestamp = through_timestamp - interval '1 hour' \
             WHERE community_id = $1",
        )
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age read positions");
        // The account ages with them, so an aged mention is still newer
        // than the account's start.
        sqlx::query(
            "UPDATE personal_read_accounts SET started_at = started_at - interval '1 hour' \
             WHERE community_id = $1",
        )
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age account");
        sqlx::query(
            "UPDATE event_mentions SET received_at = received_at - interval '1 hour' \
             WHERE community_id = $1",
        )
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age mentions");
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
    // Nothing changed, so nothing is counted again.
    assert!(page.totals.is_none() && page.marker.is_some());

    // A shared change: a new message.
    f.post(&f.other, "second", 20).await;
    let page = f.read(&since(marker)).await;
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, [f.channel.to_string().as_str()]);
    assert!(page.rows[0].unread);
    assert_eq!(page.totals.expect("totals").channel, 1);
    let marker = page.marker.expect("marker");

    // A personal change: my own read intent, found by counter.
    age().await;
    assert_eq!(f.mark_through(None, &first).await, IntentOutcome::Applied);
    let page = f.read(&since(marker)).await;
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, [f.channel.to_string().as_str()]);
    let marker = page.marker.expect("marker");
    age().await;
    assert_eq!(f.read(&since(marker)).await.rows.len(), 0);

    // My own post for agents only moves my read position past "second"
    // without a counter or a new preview; the conversation is still reported,
    // now read.
    age().await;
    let aside = f.event(
        &f.me,
        9,
        "aside",
        25,
        vec![Tag::parse(["audience", "agents"]).expect("tag")],
    );
    f.store(&aside, f.channel, None).await;
    let page = f.read(&since(marker)).await;
    let ids: Vec<&str> = page.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, [f.channel.to_string().as_str()]);
    assert!(!page.rows[0].unread);
    assert_eq!(preview_text(&page.rows[0]), Some("second"));
    let marker = page.marker.expect("marker");

    // A reply inside a thread changes the thread's row, not the channel's.
    let root = f.post(&f.me, "my question", 30).await;
    let marker = f.read(&since(marker)).await.marker.expect("marker");
    age().await;
    f.reply(&f.other, &root, "an answer", 40).await;
    let page = f.read(&since(marker)).await;
    let ids: Vec<String> = page.rows.iter().map(|row| row.id.clone()).collect();
    assert_eq!(ids, [root.id.to_hex()]);
    let marker = page.marker.expect("marker");

    // A mention is not a row of the list. One that changes no row (it is
    // for agents only, so it is not the conversation's preview) still
    // refreshes the totals.
    age().await;
    let ping = f.event(
        &f.other,
        9,
        "quiet ping",
        50,
        vec![f.tag_me(), Tag::parse(["audience", "agents"]).expect("tag")],
    );
    f.store(&ping, f.channel, None).await;
    let page = f.read(&since(marker)).await;
    assert!(page.rows.is_empty());
    assert_eq!(page.totals.expect("totals").mention, 1);
    let marker = page.marker.expect("marker");
    age().await;
    assert!(f.read(&since(marker)).await.totals.is_none());
    // Marking the channel read reads the mention; the channel's row reports
    // it and the totals follow.
    assert_eq!(f.mark_through(None, &ping).await, IntentOutcome::Applied);
    let page = f.read(&since(marker)).await;
    assert!(page.rows.iter().all(|row| row.kind != RowKind::Mention));
    assert_eq!(page.totals.expect("totals").mention, 0);
    let marker = page.marker.expect("marker");

    // Joining a channel returns its row, however old its last message is,
    // rebuilt from history when it has no shared row.
    age().await;
    let joined = f.channel().await;
    let ancient = f.event(&f.other, 9, "long ago", 5, vec![]);
    f.store(&ancient, joined, None).await;
    sqlx::query("DELETE FROM channel_heads WHERE community_id = $1 AND channel_id = $2")
        .bind(f.community.as_uuid())
        .bind(joined)
        .execute(&f.pool)
        .await
        .expect("forget row");
    sqlx::query(
        "UPDATE channel_members SET sync_modified_at = clock_timestamp() \
         WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(joined)
    .execute(&f.pool)
    .await
    .expect("join now");
    let page = f.read(&since(marker)).await;
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].channel_id, joined);
    assert_eq!(preview_text(&page.rows[0]), Some("long ago"));
    let marker = page.marker.expect("marker");
    age().await;

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

    // Rejoining a channel returns its thread rows too: the client dropped
    // them when I left, and nothing else about them changed.
    let marker = f.read(&since(marker)).await.marker.expect("marker");
    age().await;
    f.db.remove_member(
        f.community,
        f.channel,
        &f.me.public_key().to_bytes(),
        &f.me.public_key().to_bytes(),
    )
    .await
    .expect("leave");
    let page = f.read(&since(marker)).await;
    assert!(page
        .removed
        .iter()
        .any(|gone| gone.id == f.channel.to_string()));
    let marker = page.marker.expect("marker");
    age().await;
    sqlx::query(
        "UPDATE channel_members SET removed_at = NULL, sync_modified_at = clock_timestamp() \
         WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(&f.pool)
    .await
    .expect("rejoin");
    let page = f.read(&since(marker)).await;
    let mut ids: Vec<String> = page.rows.iter().map(|row| row.id.clone()).collect();
    ids.sort();
    let mut expected = vec![f.channel.to_string(), root.id.to_hex()];
    expected.sort();
    assert_eq!(ids, expected);
    let marker = page.marker.expect("marker");

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
    // The preview is the edited text of the message, still under its id.
    assert_eq!(preview_text(&before.0), Some("fixed"));
    assert_eq!(
        before.0.preview.as_ref().map(|p| p.id.clone()),
        Some(typo.id.to_hex())
    );

    // History older than the tables: the first read fills the same rows.
    f.forget().await;
    let after = (f.conversation().await, f.row(&root.id.to_hex()).await);
    assert_eq!(after, before);

    // A deleted edit is never shown; the row falls back to the edit before
    // it.
    let again = f.event(
        &f.other,
        40003,
        "fixed again",
        45,
        vec![Tag::parse(["e", &typo.id.to_hex()]).expect("tag")],
    );
    f.store(&again, f.channel, None).await;
    assert_eq!(preview_text(&f.conversation().await), Some("fixed again"));
    f.delete(&again).await;
    assert_eq!(preview_text(&f.conversation().await), Some("fixed"));

    // A head that reaches its row after its edit is installed with the edit.
    // Here the head's write stands in for one from another relay that ran
    // late, after the edit's upsert found no head to apply it to.
    sqlx::query(
        "UPDATE channel_heads SET head_id = NULL, head_created_at = NULL, \
            head_received_at = NULL, head_pubkey = NULL, head_text = NULL, \
            head_edit_at = NULL, head_edit_id = NULL \
         WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(&f.pool)
    .await
    .expect("forget the head");
    let received: DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT received_at FROM events WHERE community_id = $1 AND id = $2")
            .bind(f.community.as_uuid())
            .bind(typo.id.as_bytes().as_slice())
            .fetch_one(&f.pool)
            .await
            .expect("typo arrival");
    super::writer::install_head_for_test(&f.pool, f.community, f.channel, &typo, received).await;
    let text: Option<String> = sqlx::query_scalar(
        "SELECT head_text FROM channel_heads WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .fetch_one(&f.pool)
    .await
    .expect("head text");
    assert_eq!(text.as_deref(), Some("fixed"));

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

/// A flush waits for a write that failed and is being retried, rather than
/// counting it done when its retry leaves the queue.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_flush_waits_for_retried_writes() {
    let f = fixture().await;
    f.post(&f.other, "first", 10).await;
    // A reader's rebuild holds the shared row, so the writer's attempts time
    // out on its lock and are retried.
    let mut holder = f.pool.begin().await.expect("begin holder");
    sqlx::query(
        "SELECT 1 FROM channel_heads WHERE community_id = $1 AND channel_id = $2 FOR UPDATE",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(&mut *holder)
    .await
    .expect("hold the row");
    let held = f.event(&f.other, 9, "held", 20, vec![]);
    let (_, inserted) =
        f.db.insert_event_with_thread_metadata(f.community, &held, Some(f.channel), None)
            .await
            .expect("store event");
    assert!(inserted);
    // Long enough for the first attempt to fail and its retry to start, short
    // of the attempts running out.
    assert!(!flush_pending(std::time::Duration::from_millis(900)).await);
    holder.rollback().await.expect("release the row");
    assert!(flush_pending(std::time::Duration::from_secs(10)).await);
    let head: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT head_id FROM channel_heads WHERE community_id = $1 AND channel_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .fetch_one(&f.pool)
    .await
    .expect("head");
    assert_eq!(head.as_deref(), Some(held.id.as_bytes().as_slice()));
}

/// Every writer of the activity tables goes through community admission: once
/// a community's deletion has started (quiescing) or fenced it, the
/// background writer, its distrust pass and the on-read repair are all
/// refused and leave no row. An active community's writes land in both
/// tables.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_writers_are_refused_once_deletion_starts() {
    let (db, communities) = crate::test_support::admission_fixture().await;
    let keys = Keys::generate();
    let head = EventBuilder::new(Kind::Custom(9), "head")
        .sign_with_keys(&keys)
        .expect("sign head");
    let reply = EventBuilder::new(Kind::Custom(9), "reply")
        .sign_with_keys(&keys)
        .expect("sign reply");
    let root = *head.id.as_bytes();
    let rows = |community: CommunityId| {
        let pool = db.pool.clone();
        async move {
            let mut counts = Vec::new();
            for table in ["channel_heads", "thread_heads"] {
                let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE community_id = $1"
                )))
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count rows");
                counts.push(count);
            }
            counts
        }
    };
    let assert_fenced = |error: crate::DbError| {
        assert!(
            matches!(&error, crate::DbError::AccessDenied(message) if message.contains("write-fenced")),
            "expected application admission rejection, got {error:?}"
        );
    };

    for community in &communities[1..] {
        let channel = Uuid::new_v4();
        let (written, distrusted) =
            writer::admitted_writes_for_test(&db.pool, *community, channel, &head, &reply, root)
                .await;
        assert_fenced(written.expect_err("the writer's batch must be refused"));
        assert_fenced(distrusted.expect_err("the distrust pass must be refused"));
        assert_fenced(
            db.begin_repair_for_test(*community)
                .await
                .expect_err("the on-read repair must be refused"),
        );
        assert_eq!(rows(*community).await, [0, 0]);
    }

    let active = communities[0];
    let (written, distrusted) =
        writer::admitted_writes_for_test(&db.pool, active, Uuid::new_v4(), &head, &reply, root)
            .await;
    assert!(written.expect("active batch").is_empty(), "nothing failed");
    distrusted.expect("active distrust");
    db.begin_repair_for_test(active)
        .await
        .expect("active repair");
    assert_eq!(rows(active).await, [1, 1]);
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
    assert_eq!(f.list().await.rows.len(), 2);
    assert_eq!(f.mention_ids(false).await.len(), 1);

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
    assert!(f.mention_ids(false).await.is_empty());
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

/// History stored before these tables existed: the operator's backfill script
/// and warm command put back what the relay would have written.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_backfill_restores_history_from_before_the_tables() {
    let f = fixture().await;
    f.list().await;
    let root = f.post(&f.other, "root", 10).await;
    f.reply(&f.me, &root, "mine", 20).await;
    f.reply(&f.other, &root, "answer", 30).await;
    let ping = f.event(&f.other, 9, "ping", 40, vec![f.tag_me()]);
    f.store(&ping, f.channel, None).await;
    let before = (
        f.list().await.rows,
        f.mention_ids(false).await,
        f.read(&all()).await.totals,
    );
    assert!(before.0.iter().any(|row| row.kind == RowKind::Thread));
    assert_eq!(before.1.len(), 1);

    // As the rows were before migration 0105.
    f.forget().await;
    for sql in [
        "DELETE FROM personal_read_frontiers WHERE community_id = $1 AND root_id <> ''::bytea",
        "UPDATE event_mentions SET received_at = NULL, root_id = NULL WHERE community_id = $1",
    ] {
        sqlx::query(sql)
            .bind(f.community.as_uuid())
            .execute(&f.pool)
            .await
            .expect("age rows");
    }
    assert!(f.mention_ids(false).await.is_empty());

    let heads = |table: &'static str| {
        let (pool, community) = (f.pool.clone(), f.community);
        async move {
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE community_id = $1 AND seen_at IS NOT NULL"
            )))
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count rows")
        }
    };
    assert_eq!(
        (heads("channel_heads").await, heads("thread_heads").await),
        (0, 0)
    );
    let warmed = f.db.warm_activity(f.community).await.expect("warm");
    assert_eq!((warmed.channels, warmed.threads), (1, 1));
    assert_eq!(
        (heads("channel_heads").await, heads("thread_heads").await),
        (1, 1)
    );
    // Nothing left for a second run.
    let again = f.db.warm_activity(f.community).await.expect("warm again");
    assert_eq!(again, Warmed::default());

    // A client that read the list before the backfill learns of the thread
    // from `since`, though nothing in the thread changed.
    for (table, column) in [
        ("channel_heads", "updated_at"),
        ("thread_heads", "updated_at"),
        ("channel_members", "sync_modified_at"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET {column} = {column} - interval '1 hour' \
             WHERE community_id = $1"
        )))
        .bind(f.community.as_uuid())
        .execute(&f.pool)
        .await
        .expect("age rows");
    }
    let stale = f.list().await;
    assert!(stale.rows.iter().all(|row| row.kind != RowKind::Thread));
    let marker = stale.marker.expect("marker");

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/maintenance/backfill-activity.sql");
    // Twice: a rerun changes nothing and still validates.
    for _ in 0..2 {
        let output = tokio::process::Command::new("psql")
            .args(["-X", "-A", "-t", "-q", "-v", "ON_ERROR_STOP=1", "-f"])
            .arg(&script)
            .arg("--dbname")
            .arg(crate::test_support::database_url())
            .output()
            .await
            .expect("run psql");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The validation query: nothing left to fill.
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "0|0");
    }
    let changed = f
        .read(&Query {
            select: Select::Since(marker),
            ..all()
        })
        .await;
    assert!(changed
        .rows
        .iter()
        .any(|row| row.kind == RowKind::Thread && row.id == root.id.to_hex()));

    let after = (
        f.list().await.rows,
        f.mention_ids(false).await,
        f.read(&all()).await.totals,
    );
    assert_eq!(after, before);
}

/// Reading or flagging one message on one device holds on every device: a
/// mention read on its own stops being unread without reading the rest of
/// its timeline, and marking a message unread brings it and everything after
/// it back, never reading what came before it.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_one_message_reads_and_unreads_on_its_own() {
    let f = fixture().await;
    f.list().await;
    let shout = f.event(&f.other, 9, "hey you", 20, vec![f.tag_me()]);
    f.store(&shout, f.channel, None).await;
    let later = f.post(&f.other, "later", 30).await;
    assert!(f.conversation().await.unread);
    assert_eq!(f.list().await.totals.expect("totals").mention, 1);

    let unread_from = |message: &Event| ReadIntent::MarkUnread {
        target: f.target(None),
        message_id: Some(message.id.to_hex()),
    };
    // Marking a later message unread only moves back: the earlier mention
    // stays unread.
    assert_eq!(f.intent(unread_from(&later)).await, IntentOutcome::Applied);
    assert_eq!(f.mention_ids(true).await, [(shout.id.to_hex(), true)]);
    assert_eq!(f.list().await.totals.expect("totals").mention, 1);

    f.quiet().await;
    let marker = f.list().await.marker.expect("marker");

    let mark_read = |root: Option<&Event>, message: &Event| ReadIntent::MarkRead {
        target: f.target(root),
        message_id: message.id.to_hex(),
    };
    // Reading the mention by itself leaves the rest of the channel unread.
    assert_eq!(
        f.intent(mark_read(None, &shout)).await,
        IntentOutcome::Applied
    );
    assert!(f.mention_ids(true).await.is_empty());
    assert_eq!(f.mention_ids(false).await, [(shout.id.to_hex(), false)]);
    assert!(f.conversation().await.unread);
    assert_eq!(f.list().await.totals.expect("totals").mention, 0);
    // Another device learns of it from `since`, with fresh totals, through
    // the person's counter alone: the shared rows are quiet.
    let changed = f
        .read(&Query {
            select: Select::Since(marker),
            ..all()
        })
        .await;
    assert_eq!(changed.totals.expect("totals").mention, 0);
    assert!(changed
        .rows
        .iter()
        .any(|row| row.id == f.channel.to_string()));
    // A message that does not tag me has nothing of its own to read.
    assert_eq!(
        f.intent(mark_read(None, &later)).await,
        IntentOutcome::Applied
    );
    assert!(f.conversation().await.unread);

    assert_eq!(f.mark_through(None, &later).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);

    // Mark unread from the mention: it and everything after it is unread
    // again, though the position had passed both.
    assert_eq!(f.intent(unread_from(&shout)).await, IntentOutcome::Applied);
    assert!(f.conversation().await.unread);
    assert_eq!(f.mention_ids(true).await, [(shout.id.to_hex(), true)]);
    assert_eq!(f.list().await.totals.expect("totals").mention, 1);
    // Reading through clears it all.
    assert_eq!(f.mark_through(None, &later).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);
    assert!(f.mention_ids(true).await.is_empty());
    // Mark unread from the last message flags the row even with my own
    // post on top.
    f.post(&f.me, "mine", 40).await;
    assert_eq!(f.intent(unread_from(&later)).await, IntentOutcome::Applied);
    assert!(f.conversation().await.unread);

    // A mention in a thread is read with the thread as its target; the
    // thread's own position does not move.
    let root = f.post(&f.me, "my root", 50).await;
    let tagged = f.event(
        &f.other,
        9,
        "reply for you",
        60,
        vec![
            Tag::parse(["e", &root.id.to_hex(), "", "root"]).expect("tag"),
            f.tag_me(),
        ],
    );
    f.store(&tagged, f.channel, Some((&root, false))).await;
    f.reply(&f.other, &root, "after it", 70).await;
    assert_eq!(
        f.intent(mark_read(None, &tagged)).await,
        IntentOutcome::Blocked,
        "a reply is not a message of the channel timeline"
    );
    assert_eq!(
        f.intent(mark_read(Some(&root), &tagged)).await,
        IntentOutcome::Applied
    );
    assert!(f.mention_ids(true).await.is_empty());
    assert!(f.row(&root.id.to_hex()).await.unread);

    // Operands are checked.
    let bad = ReadIntent::MarkRead {
        target: f.target(None),
        message_id: "not hex".into(),
    };
    assert_eq!(f.intent(bad).await, IntentOutcome::Invalid);
    let bad = ReadIntent::MarkUnread {
        target: f.target(None),
        message_id: Some("ab".repeat(32)),
    };
    assert_eq!(f.intent(bad).await, IntentOutcome::Blocked);
}

/// A NIP-RS frontier carries over as a read position: forward only, never
/// clearing a manual unread, and safe to replay. `finish_import` is what
/// tells clients the relay's read state can be trusted.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn activity_import_carries_nip_rs_frontiers_over() {
    let f = fixture().await;
    assert_eq!(f.list().await.imported, Some(false));
    let import = |root: Option<&Event>, offset: u64| ReadIntent::ImportThrough {
        target: f.target(root),
        through: (f.base + offset) as u32,
    };
    // Nothing in the channel is that old yet: nothing to carry over.
    assert_eq!(f.intent(import(None, 5)).await, IntentOutcome::Applied);

    f.post(&f.other, "a", 20).await;
    let root = f.post(&f.me, "my root", 25).await;
    f.post(&f.other, "b", 30).await;
    // Signed after the frontier below, and arriving before the reply.
    f.post(&f.other, "late", 38).await;
    f.reply(&f.other, &root, "reply", 35).await;
    assert!(f.conversation().await.unread);
    assert!(f.row(&root.id.to_hex()).await.unread);

    // Through 36 lands on b, so `late` stays unread. The thread reply signed
    // at 35 is not part of the channel's timeline: anchoring on it would read
    // `late`, which arrived before it.
    assert_eq!(f.intent(import(None, 36)).await, IntentOutcome::Applied);
    assert!(f.conversation().await.unread);
    f.post(&f.other, "c", 40).await;
    assert_eq!(f.intent(import(None, 40)).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);
    // An older frontier from a lagging device moves nothing back.
    assert_eq!(f.intent(import(None, 20)).await, IntentOutcome::Applied);
    assert!(!f.conversation().await.unread);

    // A thread's frontier reads the thread, not the channel.
    assert_eq!(
        f.intent(import(Some(&root), 35)).await,
        IntentOutcome::Applied
    );
    assert!(!f.row(&root.id.to_hex()).await.unread);

    // Import never clears a flag set on the relay.
    let flag = ReadIntent::MarkUnread {
        target: f.target(None),
        message_id: None,
    };
    assert_eq!(f.intent(flag).await, IntentOutcome::Applied);
    f.post(&f.other, "d", 50).await;
    assert_eq!(f.intent(import(None, 50)).await, IntentOutcome::Applied);
    assert!(f.conversation().await.unread);

    // The first finish stands; a replay is harmless.
    for _ in 0..2 {
        assert_eq!(
            f.intent(ReadIntent::FinishImport {}).await,
            IntentOutcome::Applied
        );
        assert_eq!(f.list().await.imported, Some(true));
    }
}
