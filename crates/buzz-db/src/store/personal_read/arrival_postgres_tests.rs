//! Read progress follows relay arrival (`received_at`), never author time.
//! Each case sets every arrival explicitly: back-to-back inserts share a clock.
use super::{postgres_tests::fixture, *};
use crate::Db;
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

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

/// A message authored at `authored` that arrives at `arrived`. It mentions the
/// actor, so the channel's mention count is exactly its unread top-level count.
async fn post(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    actor: &Keys,
    authored: u64,
    arrived: u64,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(9), format!("authored {authored}"))
        .tags([Tag::public_key(actor.public_key())])
        .custom_created_at(nostr::Timestamp::from(authored))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    arrive(pool, community, &event, arrived).await;
    event
}

/// A reply to `root` that mentions the actor, so it counts without membership.
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
    let event = post(db, pool, community, channel, actor, authored, arrived).await;
    sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
        VALUES ($1,$2,to_timestamp($3),$4,$5,$5,1)")
        .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
        .bind(event.created_at.as_secs() as f64)
        .bind(channel).bind(root.id.as_bytes().as_slice()).execute(pool).await.unwrap();
    event
}

async fn sidebar(db: &Db, community: CommunityId, actor: &Keys) -> ChannelReadSummary {
    db.personal_read_sidebar(
        community,
        &actor.public_key(),
        DEFAULT_RETENTION_SECONDS,
        20,
        None,
    )
    .await
    .unwrap()
    .channels
    .remove(0)
}

/// The channel row's top-level state: (unread, mentions).
async fn counts(db: &Db, community: CommunityId, actor: &Keys) -> (bool, u32) {
    let row = sidebar(db, community, actor).await;
    (row.unread, row.mentions)
}

async fn apply(db: &Db, community: CommunityId, actor: &Keys, intent: ReadIntent) {
    let outcome = db
        .apply_personal_read_intent(community, &actor.public_key(), &intent)
        .await
        .unwrap();
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
    assert_eq!(counts(&db, community, &actor).await, (false, 0));

    // Authored ten minutes before the read message, arriving after it was read.
    let late = post(&db, &pool, community, channel, &actor, now - 600, now - 30).await;

    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.mentions), (true, 1));
    assert_eq!(row.latest_id, Some(late.id.to_hex()));
    assert_eq!(row.read_through_id, Some(read.id.to_hex()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn future_dated_anchor_does_not_swallow_later_arrivals() {
    let (db, pool, community, channel, actor, earlier) = fixture().await;
    let now = earlier.created_at.as_secs();
    arrive(&pool, community, &earlier, now - 60).await;
    // Stamped ten minutes ahead; the relay accepts up to fifteen.
    let ahead = post(&db, &pool, community, channel, &actor, now + 600, now - 40).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &ahead.id.to_hex()),
    )
    .await;
    assert_eq!(counts(&db, community, &actor).await, (false, 0));

    post(&db, &pool, community, channel, &actor, now, now - 30).await;

    assert_eq!(counts(&db, community, &actor).await, (true, 1));
}

/// Marking the sidebar's own latest message must clear the badge even when the
/// last arrival is not the newest by author time.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_as_read_with_the_sidebar_anchor_clears_a_late_arrival() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    post(&db, &pool, community, channel, &actor, now + 1, now - 40).await;
    let last = post(&db, &pool, community, channel, &actor, now - 600, now - 30).await;
    let row = sidebar(&db, community, &actor).await;
    // The fixture message is unread too, but mentions no one.
    assert_eq!((row.unread, row.mentions), (true, 2));
    assert_eq!(row.latest_id, Some(last.id.to_hex()));

    let anchor = row
        .latest_id
        .expect("a channel with messages has a latest message");
    apply(&db, community, &actor, mark_through(channel, None, &anchor)).await;

    assert_eq!(counts(&db, community, &actor).await, (false, 0));
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
    let last = reply(
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
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        (row.unread, row.mentions),
        (false, 0),
        "replies are not top-level"
    );
    assert_eq!(row.threads.len(), 1);
    assert_eq!(row.threads[0].mentions, 2);
    assert_eq!(row.threads[0].latest_id, last.id.to_hex());

    let root_id = root.id.to_hex();
    let anchor = row.threads[0].latest_id.clone();
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, Some(&root_id), &anchor),
    )
    .await;

    let row = sidebar(&db, community, &actor).await;
    assert!(!row.unread);
    assert!(row.threads.is_empty());
}

/// The shallow latest probe reads the newest `MAX_CHANNEL_SCAN + 1` events by
/// author time, of any kind. A late arrival with an older author time than
/// that many events is counted unread, so the unread scan must supply the
/// anchor or Mark as read with the sidebar anchor leaves it.
async fn mark_as_read_behind_fillers(fillers: u64) -> (bool, u32) {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let now = first.created_at.as_secs();
    arrive(&pool, community, &first, now - 60).await;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &first.id.to_hex()),
    )
    .await;
    for i in 0..fillers {
        // Reactions are not counted, but the probe's LIMIT sees them.
        let filler = EventBuilder::new(Kind::Custom(7), "+")
            .custom_created_at(nostr::Timestamp::from(now - 500 + i))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        db.insert_event(community, &filler, Some(channel))
            .await
            .unwrap();
        arrive(&pool, community, &filler, now - 50).await;
    }
    post(&db, &pool, community, channel, &actor, now - 600, now - 10).await;
    assert_eq!(counts(&db, community, &actor).await, (true, 1));

    let anchor = sidebar(&db, community, &actor)
        .await
        .latest_id
        .expect("a channel with messages has a latest message");
    apply(&db, community, &actor, mark_through(channel, None, &anchor)).await;
    counts(&db, community, &actor).await
}

/// Control: with one slot to spare the late arrival is inside the probe.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_as_read_clears_a_late_arrival_inside_the_latest_probe() {
    let fillers = MAX_CHANNEL_SCAN as u64 - 1;
    assert_eq!(mark_as_read_behind_fillers(fillers).await, (false, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_as_read_clears_a_late_arrival_outside_the_latest_probe() {
    let fillers = MAX_CHANNEL_SCAN as u64;
    assert_eq!(mark_as_read_behind_fillers(fillers).await, (false, 0));
}

/// When the newest 257 events are all uncounted, the only message is still in
/// the unread scan: the summary reports it as latest and unread.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn latest_message_behind_a_full_probe_of_reactions_is_found() {
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
    let only = post(&db, &pool, community, channel, &actor, now - 600, now - 10).await;
    for i in 0..MAX_CHANNEL_SCAN as u64 {
        let filler = EventBuilder::new(Kind::Custom(7), "+")
            .custom_created_at(nostr::Timestamp::from(now - 500 + i))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        db.insert_event(community, &filler, Some(channel))
            .await
            .unwrap();
        arrive(&pool, community, &filler, now - 50).await;
    }

    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.latest_id, Some(only.id.to_hex()));
    assert_eq!((row.unread, row.mentions), (true, 1));
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

/// Two mentioning messages with the same author time arriving within one
/// second; marks the one at `pick`, then the other. Returns the mention count
/// after the first mark, and the anchor (index) the frontier ends with.
async fn mark_within_one_second(micros: [u32; 2], pick: usize) -> (u32, usize) {
    let (db, pool, community, channel, actor, older) = fixture().await;
    let now = older.created_at.as_secs();
    let arrived = now as i64 - 30;
    // The fixture message arrives first, so either mark covers it.
    arrive_exact(&pool, community, &older, arrived - 1, 0).await;
    let first = post(&db, &pool, community, channel, &actor, now, now).await;
    let second = post(&db, &pool, community, channel, &actor, now, now).await;
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
    let mentions = sidebar(&db, community, &actor).await.mentions;
    apply(
        &db,
        community,
        &actor,
        mark_through(channel, None, &both[1 - pick].id.to_hex()),
    )
    .await;
    let anchor = sidebar(&db, community, &actor).await.read_through_id;
    let anchor = both
        .iter()
        .position(|e| Some(e.id.to_hex()) == anchor)
        .expect("the anchor is one of the marked messages");
    (mentions, anchor)
}

/// Mid-second stamps, so truncating or rounding the frontier to whole seconds
/// either reads the later message or leaves the anchor unread. The later
/// arrival then becomes the anchor.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn arrivals_one_microsecond_apart_in_the_same_second_are_ordered() {
    assert_eq!(mark_within_one_second([500_000, 500_001], 0).await, (1, 1));
}

/// The Order section: everything that arrived at or before the anchor is read,
/// so an identical stamp reads both, whichever is marked. An equal arrival
/// keeps the first anchor.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn marking_either_of_two_identical_arrivals_reads_both() {
    for pick in [0, 1] {
        assert_eq!(
            mark_within_one_second([500_000, 500_000], pick).await,
            (0, pick),
            "marked index {pick}"
        );
    }
}
