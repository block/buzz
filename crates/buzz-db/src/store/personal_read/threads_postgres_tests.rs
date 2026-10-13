//! Thread summaries, anchor validation and targeted refresh.
use super::{postgres_tests::fixture, *};
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

async fn post(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    at: u64,
    tags: Vec<Tag>,
) -> nostr::Event {
    post_as(db, community, channel, &Keys::generate(), at, tags).await
}

async fn post_as(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    author: &Keys,
    at: u64,
    tags: Vec<Tag>,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(9), format!("message at {at}"))
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(author)
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    event
}

/// A canonical reply to the root: stored event plus its thread metadata.
async fn reply(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    root: &nostr::Event,
    at: u64,
    tags: Vec<Tag>,
) -> nostr::Event {
    let event = post(db, community, channel, at, tags).await;
    link(pool, community, channel, root, &event).await;
    event
}

async fn link(
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    root: &nostr::Event,
    event: &nostr::Event,
) {
    sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
        VALUES ($1,$2,to_timestamp($3),$4,$5,$5,1)")
        .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
        .bind(event.created_at.as_secs() as f64)
        .bind(channel).bind(root.id.as_bytes().as_slice()).execute(pool).await.unwrap();
}

/// Put the actor in the root's conversation, so that plain replies to it
/// count. The actor's own reply is never unread.
async fn join(
    db: &Db,
    pool: &PgPool,
    community: CommunityId,
    channel: Uuid,
    actor: &Keys,
    root: &nostr::Event,
) {
    let at = root.created_at.as_secs();
    let own = post_as(db, community, channel, actor, at, vec![]).await;
    link(pool, community, channel, root, &own).await;
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

async fn apply(db: &Db, community: CommunityId, actor: &Keys, intent: ReadIntent) -> IntentOutcome {
    db.apply_personal_read_intent(community, &actor.public_key(), &intent)
        .await
        .unwrap()
}

fn channel_mark(channel: Uuid, message_id: String) -> ReadIntent {
    ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_id,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_through_rejects_unresolved_missing_auxiliary_and_malformed_anchors() {
    let (db, _pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    // Unresolved ancestry cannot be classified as top-level or reply.
    let orphan_parent = "ab".repeat(32);
    let orphan = post(
        &db,
        community,
        channel,
        base + 10,
        vec![Tag::parse(["e", &orphan_parent, "", "reply"]).unwrap()],
    )
    .await;
    assert!(db
        .apply_personal_read_intent(
            community,
            &actor.public_key(),
            &channel_mark(channel, orphan.id.to_hex()),
        )
        .await
        .is_err());
    let reaction = EventBuilder::new(Kind::Custom(7), "+")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &reaction, Some(channel))
        .await
        .unwrap();
    for (anchor, outcome) in [
        ("cd".repeat(32), IntentOutcome::Blocked),
        (reaction.id.to_hex(), IntentOutcome::Blocked),
        ("zz".into(), IntentOutcome::Invalid),
    ] {
        assert_eq!(
            apply(
                &db,
                community,
                &actor,
                channel_mark(channel, anchor.clone())
            )
            .await,
            outcome,
            "{anchor}"
        );
    }
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.read_through_id, None, "nothing was marked");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_frontier_never_covers_an_epoch_zero_reply() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    join(&db, &pool, community, channel, &actor, &root).await;
    let ancient = reply(&db, &pool, community, channel, &root, 0, vec![]).await;
    // A channel frontier exists, with no thread frontier: absent must not
    // read as epoch zero or as the channel's.
    apply(
        &db,
        community,
        &actor,
        channel_mark(channel, root.id.to_hex()),
    )
    .await;
    // Widen the horizon to reach the epoch, so that only a misread absent
    // frontier could hide this reply.
    let row = db
        .personal_read_sidebar(community, &actor.public_key(), u32::MAX, 20, None)
        .await
        .unwrap()
        .channels
        .remove(0);
    assert!(!row.unread);
    assert_eq!(row.threads.len(), 1, "epoch-zero reply stays unread");
    assert_eq!(row.threads[0].mentions, 1);
    assert_eq!(row.threads[0].latest_id, ancient.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_summaries_order_cap_and_anchor() {
    let (db, pool, community, channel, actor, fixture_root) = fixture().await;
    // After the fixture root, so marking the newest root covers every root.
    let base = fixture_root.created_at.as_secs() + 1;
    // One more thread than the cap.
    let n = MAX_THREAD_SUMMARIES as u64 + 1;
    let mut roots = Vec::new();
    for i in 0..n {
        let root = post(&db, community, channel, base + i, vec![]).await;
        join(&db, &pool, community, channel, &actor, &root).await;
        roots.push(root);
    }
    let newest_root = roots.last().unwrap().clone();
    apply(
        &db,
        community,
        &actor,
        channel_mark(channel, newest_root.id.to_hex()),
    )
    .await;
    // Threads 0 and 1 tie on newest reply time and the rest follow, newest
    // first. Thread 2 has one directed reply and two that arrive last,
    // together.
    let offset = |i: u64| if i < 2 { n * 10 } else { (n - i) * 10 };
    let mut anchors = Vec::new();
    for (i, root) in (0..).zip(&roots) {
        let at = base + 100 + offset(i);
        anchors.push(reply(&db, &pool, community, channel, root, at, vec![]).await);
    }
    reply(
        &db,
        &pool,
        community,
        channel,
        &roots[2],
        base + 101,
        vec![Tag::parse(["p", &actor.public_key().to_hex()]).unwrap()],
    )
    .await;
    let twin = reply(
        &db,
        &pool,
        community,
        channel,
        &roots[2],
        base + 100 + offset(2),
        vec![],
    )
    .await;
    sqlx::query(
        "UPDATE events SET received_at=(SELECT received_at FROM events WHERE id=$1) WHERE id=$2",
    )
    .bind(twin.id.as_bytes().as_slice())
    .bind(anchors[2].id.as_bytes().as_slice())
    .execute(&pool)
    .await
    .unwrap();

    let row = sidebar(&db, community, &actor).await;
    // Every root is read; the eight unread replies count only on threads.
    assert_eq!((row.unread, row.mentions), (false, 0));
    // The timeline's latest is its last top-level arrival, never a reply.
    assert_eq!(row.latest_id, Some(newest_root.id.to_hex()));
    let mut tied = [roots[0].id.to_hex(), roots[1].id.to_hex()];
    tied.sort();
    let order = |row: &ChannelReadSummary| -> Vec<_> {
        row.threads.iter().map(|t| t.root_id.clone()).collect()
    };
    let ids = |roots: &[nostr::Event]| roots.iter().map(|r| r.id.to_hex()).collect::<Vec<_>>();
    let last = roots.len() - 1;
    // The oldest unread thread is past the cap.
    assert_eq!(order(&row), [tied.to_vec(), ids(&roots[2..last])].concat());
    let third = &row.threads[2];
    assert_eq!(third.mentions, 3);
    assert_eq!(
        third.latest_id,
        std::cmp::min(anchors[2].id.to_hex(), twin.id.to_hex()),
        "equal arrivals break toward the smaller ID"
    );

    // Reading one listed thread through its anchor lists the oldest.
    let first = row.threads[0].clone_target(channel);
    assert_eq!(
        apply(&db, community, &actor, first).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        order(&row),
        [vec![tied[1].clone()], ids(&roots[2..])].concat()
    );
    let mentions: Vec<_> = row.threads.iter().map(|t| t.mentions).collect();
    let mut expected = vec![1; MAX_THREAD_SUMMARIES];
    expected[1] = 3;
    assert_eq!(mentions, expected);
}

impl ThreadReadSummary {
    fn clone_target(&self, channel: Uuid) -> ReadIntent {
        ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: Some(self.root_id.clone()),
            },
            message_id: self.latest_id.clone(),
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_on_a_never_unread_root_is_selectable_and_readable_by_itself() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    let mention = || vec![Tag::parse(["p", &actor.public_key().to_hex()]).unwrap()];
    // A diff is never unread, not even one addressed to the actor.
    let diff = EventBuilder::new(Kind::Custom(40008), "a diff")
        .tags(mention())
        .custom_created_at(nostr::Timestamp::from(base + 1))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &diff, Some(channel))
        .await
        .unwrap();
    let on_diff = reply(&db, &pool, community, channel, &diff, base + 10, mention()).await;
    let elsewhere = reply(&db, &pool, community, channel, &root, base + 20, mention()).await;

    let row = sidebar(&db, community, &actor).await;
    // The undirected fixture root is unread; the directed diff is not counted.
    assert_eq!((row.unread, row.mentions), (true, 0));
    assert_eq!(row.threads.len(), 2);
    let listed = &row.threads[1];
    assert_eq!(listed.root_id, diff.id.to_hex());
    assert_eq!(listed.latest_id, on_diff.id.to_hex());
    assert_eq!(listed.mentions, 1);

    // The diff is still no anchor; the listed reply reads its thread alone.
    let through_diff = ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: Some(listed.root_id.clone()),
        },
        message_id: diff.id.to_hex(),
    };
    assert_eq!(
        apply(&db, community, &actor, through_diff).await,
        IntentOutcome::Blocked
    );
    assert_eq!(
        apply(&db, community, &actor, listed.clone_target(channel)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.mentions), (true, 0));
    assert_eq!(row.threads.len(), 1);
    assert_eq!(row.threads[0].latest_id, elsewhere.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn targeted_sidebar_returns_only_requested_joined_channels_in_one_snapshot() {
    let (db, _pool, community, channel, actor, _) = fixture().await;
    let create = |name: &'static str, owner: Keys| {
        let db = db.clone();
        async move {
            db.create_channel(
                community,
                name,
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                &owner.public_key().to_bytes(),
                None,
            )
            .await
            .unwrap()
            .id
        }
    };
    let joined = create("joined", actor.clone()).await;
    let foreign = create("not joined", Keys::generate()).await;
    let _unrequested = create("unrequested", actor.clone()).await;
    let page = db
        .personal_read_sidebar_channels(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &[joined, foreign, channel],
        )
        .await
        .unwrap();
    let mut expected = vec![channel, joined];
    expected.sort();
    let ids: Vec<_> = page.channels.iter().map(|c| c.channel_id).collect();
    assert_eq!(ids, expected);
    assert!(page.next_cursor.is_none());
    for bad in [
        vec![],
        vec![channel, channel],
        (0..21).map(|_| Uuid::new_v4()).collect(),
    ] {
        assert!(db
            .personal_read_sidebar_channels(
                community,
                &actor.public_key(),
                DEFAULT_RETENTION_SECONDS,
                &bad
            )
            .await
            .is_err());
    }
}
