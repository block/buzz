//! Read progress follows relay arrival (`received_at`), never author time.
//! Each case sets every arrival explicitly: back-to-back inserts share a clock.
use super::{postgres_tests, *};
use crate::Db;
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

/// The shared fixture, with the actor's membership moved an hour back so
/// arrivals set within that hour fall after joining.
async fn fixture() -> (Db, PgPool, CommunityId, Uuid, Keys, nostr::Event) {
    let fixture = postgres_tests::fixture().await;
    sqlx::query(
        "UPDATE channel_members SET joined_at=now()-interval '1 hour' WHERE community_id=$1",
    )
    .bind(fixture.2.as_uuid())
    .execute(&fixture.1)
    .await
    .unwrap();
    fixture
}

/// Store `event` as having arrived at `arrived` (Unix seconds).
async fn arrive(pool: &PgPool, community: CommunityId, event: &nostr::Event, arrived: u64) {
    let updated = sqlx::query(
        "UPDATE events SET received_at=to_timestamp($3) WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(arrived as f64)
    .execute(pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(updated, 1, "arrival must land on exactly one stored event");
}

/// A message of `kind` authored at `authored` that arrives at `arrived`.
#[allow(clippy::too_many_arguments)]
async fn post_kind(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    kind: u16,
    authored: u64,
    arrived: u64,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(kind), format!("authored {authored}"))
        .custom_created_at(nostr::Timestamp::from(authored))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    arrive(pool, community, &event, arrived).await;
    event
}

async fn post(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    authored: u64,
    arrived: u64,
) -> nostr::Event {
    post_kind(db, pool, community, channel, 9, authored, arrived).await
}

/// A reply to `root` that mentions the actor, which makes it their thread.
#[allow(clippy::too_many_arguments)]
async fn reply(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    root: &nostr::Event,
    actor: &Keys,
    authored: u64,
    arrived: u64,
) -> nostr::Event {
    let event = postgres_tests::reply(
        db,
        community,
        channel,
        root,
        &Keys::generate(),
        authored,
        vec![Tag::public_key(actor.public_key())],
        false,
    )
    .await;
    arrive(pool, community, &event, arrived).await;
    event
}

async fn sidebar(db: &Db, community: CommunityId, actor: &Keys) -> ChannelReadSummary {
    postgres_tests::sidebar(db, community, actor).await
}

async fn apply(db: &Db, community: CommunityId, actor: &Keys, intent: ReadIntent) {
    let outcome = postgres_tests::apply(db, community, actor, intent).await;
    assert_eq!(outcome, IntentOutcome::Applied);
}

fn mark_through(channel: Uuid, root: Option<&str>, message: &str) -> ReadIntent {
    ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: root.map(str::to_owned),
        },
        message_id: message.to_owned(),
    }
}

fn mark_channel_read(channel: Uuid, message: String) -> ReadIntent {
    ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: message,
    }
}

/// Channel-timeline states of `messages`, in order.
async fn states(
    db: &Db,
    community: CommunityId,
    actor: &Keys,
    channel: Uuid,
    messages: &[&nostr::Event],
) -> Vec<String> {
    postgres_tests::status(db, community, actor, channel, None, messages).await
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn late_arrival_with_old_author_time_is_unread() {
    let (db, pool, community, channel, actor, read) = fixture().await;
    let now = read.created_at.as_secs();
    arrive(&pool, community, &read, now - 60).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &read.id.to_hex()),
    )
    .await;

    // Authored ten minutes before the read message, arriving after it was read.
    let late = post(&db, &pool, community, channel, now - 600, now - 30).await;

    assert_eq!(
        states(&db, community, &actor, channel, &[&read, &late]).await,
        ["read", "unread"]
    );
    assert!(sidebar(&db, community, &actor).await.unread);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn future_dated_anchor_does_not_swallow_later_arrivals() {
    let (db, pool, community, channel, actor, earlier) = fixture().await;
    let now = earlier.created_at.as_secs();
    arrive(&pool, community, &earlier, now - 60).await;
    // Stamped ten minutes ahead; the relay accepts up to fifteen.
    let ahead = post(&db, &pool, community, channel, now + 600, now - 40).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &ahead.id.to_hex()),
    )
    .await;

    let later = post(&db, &pool, community, channel, now, now - 30).await;

    assert_eq!(
        states(&db, community, &actor, channel, &[&earlier, &ahead, &later]).await,
        ["read", "read", "unread"]
    );
    assert!(sidebar(&db, community, &actor).await.unread);
}

/// Clients name only display-order IDs. Marking the newest message shown
/// must clear the badge even when the last arrival is displayed above it.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn marking_the_newest_displayed_message_clears_a_late_arrival_above_it() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    let newest = post(&db, &pool, community, channel, now + 1, now - 40).await;
    let late = post(&db, &pool, community, channel, now - 600, now - 30).await;
    assert!(sidebar(&db, community, &actor).await.unread);

    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &newest.id.to_hex()),
    )
    .await;

    let row = sidebar(&db, community, &actor).await;
    assert!(!row.unread);
    assert_eq!(
        row.read_through_id,
        Some(newest.id.to_hex()),
        "the divider stays under the newest displayed message"
    );
    assert_eq!(
        states(&db, community, &actor, channel, &[&late, &newest]).await,
        ["read", "read"]
    );
}

/// A mark reads only what is displayed at or before its anchor: a message
/// shown below it stays unread, even one that arrived first.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn marking_leaves_messages_displayed_below_the_anchor_unread() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    let below = post(&db, &pool, community, channel, now + 5, now - 50).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &first.id.to_hex()),
    )
    .await;
    assert_eq!(
        states(&db, community, &actor, channel, &[&first, &below]).await,
        ["read", "unread"]
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_channel_read_with_the_newest_displayed_message_clears_a_late_arrival() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    let newest = post(&db, &pool, community, channel, now + 1, now - 40).await;
    post(&db, &pool, community, channel, now - 600, now - 30).await;
    apply(
        &db,
        community,
        &actor,
        mark_channel_read(channel, newest.id.to_hex()),
    )
    .await;
    assert!(!sidebar(&db, community, &actor).await.unread);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_thread_read_with_the_newest_displayed_reply_clears_a_late_reply() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let now = root.created_at.as_secs();
    arrive(&pool, community, &root, now - 60).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &root.id.to_hex()),
    )
    .await;
    let newest = reply(
        &db,
        &pool,
        community,
        channel,
        &root,
        &actor,
        now + 1,
        now - 40,
    )
    .await;
    let late = reply(
        &db,
        &pool,
        community,
        channel,
        &root,
        &actor,
        now - 600,
        now - 30,
    )
    .await;
    // Ingest started the actor's thread row at real arrival time; move it
    // back to before the backdated replies.
    sqlx::query("UPDATE personal_read_frontiers SET through_timestamp=to_timestamp($3) WHERE community_id=$1 AND root_id=$2")
        .bind(community.as_uuid())
        .bind(root.id.as_bytes().as_slice())
        .bind((now - 60) as f64)
        .execute(&pool)
        .await
        .unwrap();
    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.threads.len()), (false, 1));
    assert_eq!(
        row.threads[0].latest_id,
        late.id.to_hex(),
        "the last arrival"
    );
    assert_eq!(row.threads[0].mentions, 2);

    let root_id = root.id.to_hex();
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, Some(&root_id), &newest.id.to_hex()),
    )
    .await;

    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.threads.len()), (false, 0));
}

/// Store `event` as having arrived at exactly `seconds` plus `micros`. Built
/// from integers, never a float, so microsecond order cannot hinge on rounding.
async fn arrive_exact(
    pool: &PgPool,
    community: CommunityId,
    event: &nostr::Event,
    seconds: i64,
    micros: u32,
) {
    let at = chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, micros * 1_000).unwrap();
    let updated = sqlx::query("UPDATE events SET received_at=$3 WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(at)
        .execute(pool)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(updated, 1, "arrival must land on exactly one stored event");
    let stored: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT received_at FROM events WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stored, at, "received_at must keep microseconds");
}

/// Two messages with the same author time arriving within one second; marks
/// the one at `pick` and returns both channel states.
async fn mark_within_one_second(micros: [u32; 2], pick: usize) -> (Vec<String>, bool) {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    let second = post(&db, &pool, community, channel, now, now).await;
    // Equal author times display in ID order; index 0 displays first, so a
    // mark reads only what arrived by then, not the other by display order.
    let mut both = [&first, &second];
    both.sort_by_key(|e| e.id);
    let arrived = now as i64 - 30;
    arrive_exact(&pool, community, both[0], arrived, micros[0]).await;
    arrive_exact(&pool, community, both[1], arrived, micros[1]).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &both[pick].id.to_hex()),
    )
    .await;
    (
        states(&db, community, &actor, channel, &both).await,
        sidebar(&db, community, &actor).await.unread,
    )
}

/// Mid-second stamps, so truncating or rounding the frontier to whole seconds
/// either reads the later message or leaves the anchor unread.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn arrivals_one_microsecond_apart_in_the_same_second_are_ordered() {
    assert_eq!(
        mark_within_one_second([500_000, 500_001], 0).await,
        (vec!["read".to_owned(), "unread".to_owned()], true)
    );
}

/// The Order section: everything that arrived at or before the anchor is read,
/// so an identical stamp reads both, whichever is marked.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn marking_either_of_two_identical_arrivals_reads_both() {
    for pick in [0, 1] {
        assert_eq!(
            mark_within_one_second([500_000, 500_000], pick).await,
            (vec!["read".to_owned(), "read".to_owned()], false),
            "marked index {pick}"
        );
    }
}
