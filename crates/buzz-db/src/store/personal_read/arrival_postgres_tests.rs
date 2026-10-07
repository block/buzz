//! Read progress follows relay arrival (`received_at`), never author time.
//! Each case sets every arrival explicitly: back-to-back inserts share a clock.
use super::{postgres_tests, projection::LATEST_PROBE, *};
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
    assert_eq!(sidebar(&db, community, &actor).await.unread, 1);
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
    assert_eq!(sidebar(&db, community, &actor).await.unread, 1);
}

/// Marking the sidebar's own latest message must clear the badge even when the
/// last arrival is not the newest by author time.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_as_read_with_the_sidebar_anchor_clears_a_late_arrival() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    post(&db, &pool, community, channel, now + 1, now - 40).await;
    post(&db, &pool, community, channel, now - 600, now - 30).await;
    assert_eq!(sidebar(&db, community, &actor).await.unread, 3);

    let anchor = sidebar(&db, community, &actor)
        .await
        .latest_message_id
        .expect("a channel with messages has a latest message");
    apply(&db, community, &actor, mark_channel_read(channel, anchor)).await;

    assert_eq!(sidebar(&db, community, &actor).await.unread, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_thread_read_with_the_sidebar_anchor_clears_a_late_reply() {
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
    reply(
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
    reply(
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
    assert_eq!((row.unread, row.unread_thread_count), (2, 1));
    assert_eq!(row.threads.len(), 1);

    let root_id = root.id.to_hex();
    let anchor = row.threads[0].latest_reply_id.clone();
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, Some(&root_id), &anchor),
    )
    .await;

    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.unread_thread_count), (0, 0));
    assert!(row.threads.is_empty());
}

/// The latest probe holds only the newest messages by author time. A late
/// arrival authored before all of them is still counted, so it must still be
/// the anchor, or Mark as read with the sidebar anchor leaves it unread.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_as_read_clears_a_late_arrival_behind_the_latest_probe() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    for i in 0..LATEST_PROBE as u64 {
        post(&db, &pool, community, channel, now - 500 + i, now - 50).await;
    }
    let anchor = sidebar(&db, community, &actor)
        .await
        .latest_message_id
        .unwrap();
    apply(&db, community, &actor, mark_channel_read(channel, anchor)).await;
    assert_eq!(sidebar(&db, community, &actor).await.unread, 0);

    let late = post(&db, &pool, community, channel, now - 600, now - 10).await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.unread, 1);
    assert_eq!(row.latest_message_id, Some(late.id.to_hex()));
    apply(
        &db,
        community,
        &actor,
        mark_channel_read(channel, late.id.to_hex()),
    )
    .await;
    assert_eq!(sidebar(&db, community, &actor).await.unread, 0);
}

/// Reactions never fill the latest probe.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn latest_message_behind_newer_reactions_is_found() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    let demoted = sqlx::query("UPDATE events SET kind=7 WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(first.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap()
        .rows_affected();
    assert_eq!(demoted, 1);
    let only = post(&db, &pool, community, channel, now - 600, now - 10).await;
    for i in 0..LATEST_PROBE as u64 {
        post_kind(&db, &pool, community, channel, 7, now - 500 + i, now - 50).await;
    }

    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.latest_message_id, Some(only.id.to_hex()));
    assert_eq!(row.latest_message_at, Some((now - 600) as i64));
    assert_eq!(row.unread, 1);
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
async fn mark_within_one_second(micros: [u32; 2], pick: usize) -> (Vec<String>, u32) {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    let second = post(&db, &pool, community, channel, now, now).await;
    let arrived = now as i64 - 30;
    arrive_exact(&pool, community, &first, arrived, micros[0]).await;
    arrive_exact(&pool, community, &second, arrived, micros[1]).await;
    let both = [&first, &second];
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
        (vec!["read".to_owned(), "unread".to_owned()], 1)
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
            (vec!["read".to_owned(), "read".to_owned()], 0),
            "marked index {pick}"
        );
    }
}
