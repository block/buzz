//! Whole-channel reads, thread summaries and targeted refresh.
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

fn channel_read(channel: Uuid, message: &nostr::Event) -> ReadIntent {
    ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: message.id.to_hex(),
    }
}

fn exact(count: &ReadCount) -> Option<u32> {
    match count {
        ReadCount::Exact { value } => Some(*value),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_channel_read_covers_every_thread_through_a_reply_anchor() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    join(&db, &pool, community, channel, &actor, &root).await;
    let base = root.created_at.as_secs();
    let first = reply(&db, &pool, community, channel, &root, base + 10, vec![]).await;
    post(&db, community, channel, base + 20, vec![]).await;
    let second = reply(&db, &pool, community, channel, &root, base + 30, vec![]).await;

    // A reply anchor is valid for the whole channel, unlike channel mark_through.
    let channel_only = ReadTarget {
        channel_id: channel,
        root_id: None,
    };
    let through_reply = ReadIntent::MarkThrough {
        target: channel_only,
        message_id: first.id.to_hex(),
    };
    assert_eq!(
        apply(&db, community, &actor, through_reply).await,
        IntentOutcome::Blocked
    );
    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &first)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        exact(&row.unread),
        Some(2),
        "top at +20 and reply at +30 remain"
    );
    assert_eq!(row.threads.items.len(), 1);
    assert_eq!(row.threads.items[0].latest_reply_id, second.id.to_hex());
    assert!(row.threads.complete);

    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &second)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(
        exact(&row.unread),
        Some(0),
        "every thread is covered by the cut"
    );
    assert!(row.threads.items.is_empty() && row.threads.complete);

    // A reply that arrives after the cut is unread, however far backdated.
    let late = reply(&db, &pool, community, channel, &root, base + 5, vec![]).await;
    let newer = reply(&db, &pool, community, channel, &root, base + 40, vec![]).await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(exact(&row.unread), Some(2));
    assert_eq!(row.threads.items[0].latest_reply_id, newer.id.to_hex());

    // Contexts apply the same effective thread frontier.
    let page = db
        .personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &[ContextQuery {
                target: ReadTarget {
                    channel_id: channel,
                    root_id: Some(root.id.to_hex()),
                },
                message_ids: vec![second.id.to_hex(), late.id.to_hex(), newer.id.to_hex()],
            }],
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(&page).unwrap();
    assert!(wire["contexts"][0].get("through_timestamp").is_none());
    assert_eq!(wire["contexts"][0]["messages"][0]["status"], "read");
    assert_eq!(wire["contexts"][0]["messages"][1]["status"], "unread");
    assert_eq!(wire["contexts"][0]["messages"][2]["status"], "unread");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mark_channel_read_validates_anchor_without_ancestry_and_stays_independent() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    // Unresolved ancestry blocks ordinary marks, but not an arrival-only cut.
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
            &ReadIntent::MarkThrough {
                target: ReadTarget {
                    channel_id: channel,
                    root_id: None
                },
                message_id: orphan.id.to_hex(),
            },
        )
        .await
        .is_err());
    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &orphan)).await,
        IntentOutcome::Applied
    );

    // Missing, auxiliary and malformed anchors.
    let missing = ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: "cd".repeat(32),
    };
    assert_eq!(
        apply(&db, community, &actor, missing).await,
        IntentOutcome::Blocked
    );
    let malformed = ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: "zz".into(),
    };
    assert_eq!(
        apply(&db, community, &actor, malformed).await,
        IntentOutcome::Invalid
    );
    let reaction = EventBuilder::new(Kind::Custom(7), "+")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &reaction, Some(channel))
        .await
        .unwrap();
    assert_eq!(
        apply(&db, community, &actor, channel_read(channel, &reaction)).await,
        IntentOutcome::Blocked
    );

    // Ordinary channel marks never set the whole-channel cut.
    let other = Keys::generate();
    assert_eq!(
        apply(
            &db,
            community,
            &other,
            ReadIntent::MarkThrough {
                target: ReadTarget {
                    channel_id: channel,
                    root_id: None
                },
                message_id: root.id.to_hex(),
            },
        )
        .await,
        IntentOutcome::Applied
    );
    let cuts: Vec<Option<chrono::DateTime<chrono::Utc>>> = sqlx::query_scalar(
        "SELECT threads_through_timestamp FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(other.public_key().to_bytes().as_slice())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(cuts, vec![None]);

    // The schema admits the cut on channel rows only.
    let thread_cut = sqlx::query(
        "UPDATE personal_read_frontiers SET threads_through_timestamp=now()
         WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await;
    assert!(thread_cut.is_ok(), "channel row accepts the cut");
    let root_id = root.id.to_hex();
    let thread = ReadTarget {
        channel_id: channel,
        root_id: Some(root_id),
    };
    let mark = ReadIntent::MarkThrough {
        target: thread,
        message_id: root.id.to_hex(),
    };
    assert_eq!(
        apply(&db, community, &actor, mark).await,
        IntentOutcome::Applied
    );
    assert!(sqlx::query(
        "UPDATE personal_read_frontiers SET threads_through_timestamp=now()
         WHERE community_id=$1 AND actor=$2 AND root_id<>''::bytea",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn absent_whole_channel_cut_never_covers_epoch_zero_replies() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    join(&db, &pool, community, channel, &actor, &root).await;
    reply(&db, &pool, community, channel, &root, 0, vec![]).await;
    // A channel row exists, with no whole-channel cut: NULL must stay NULL.
    apply(
        &db,
        community,
        &actor,
        ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: None,
            },
            message_id: root.id.to_hex(),
        },
    )
    .await;
    // Widen the horizon to reach the epoch, so that only a NULL cut read as
    // zero could hide this reply.
    let row = db
        .personal_read_sidebar(community, &actor.public_key(), u32::MAX, 20, None)
        .await
        .unwrap()
        .channels
        .remove(0);
    assert_eq!(exact(&row.unread), Some(1), "epoch-zero reply stays unread");
    assert_eq!(row.threads.items[0].latest_reply_at, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_summaries_order_cap_anchor_and_sum_to_reply_unread() {
    let (db, pool, community, channel, actor, fixture_root) = fixture().await;
    // After the fixture root, so marking the newest root covers every root.
    let base = fixture_root.created_at.as_secs() + 1;
    let mut roots = Vec::new();
    for i in 0..6 {
        let root = post(&db, community, channel, base + i, vec![]).await;
        join(&db, &pool, community, channel, &actor, &root).await;
        roots.push(root);
    }
    let newest_root = roots.last().unwrap().clone();
    apply(
        &db,
        community,
        &actor,
        ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: None,
            },
            message_id: newest_root.id.to_hex(),
        },
    )
    .await;
    // Threads 0 and 1 tie on newest reply time; thread 2 has one directed
    // reply and two that arrive last, together.
    let mut anchors = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        let at = base + 100 + [50, 50, 40, 30, 20, 10][i];
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
        base + 140,
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
    assert_eq!(exact(&row.unread), Some(8));
    assert!(!row.threads.complete, "six unread threads exceed the cap");
    let mut tied = [roots[0].id.to_hex(), roots[1].id.to_hex()];
    tied.sort();
    let order: Vec<_> = row
        .threads
        .items
        .iter()
        .map(|t| t.root_id.clone())
        .collect();
    assert_eq!(
        order,
        vec![
            tied[0].clone(),
            tied[1].clone(),
            roots[2].id.to_hex(),
            roots[3].id.to_hex(),
            roots[4].id.to_hex(),
        ]
    );
    let third = &row.threads.items[2];
    assert_eq!(exact(&third.unread), Some(3));
    assert_eq!(third.latest_reply_at, (base + 140) as i64);
    assert_eq!(
        third.latest_reply_id,
        std::cmp::min(anchors[2].id.to_hex(), twin.id.to_hex()),
        "equal arrivals break toward the smaller ID"
    );
    // The row's anchor is its last arrival; its activity is the greatest
    // author time, another message's.
    assert_eq!(row.latest_message_id.as_ref(), Some(&third.latest_reply_id));
    assert_eq!(row.latest_message_at, Some((base + 150) as i64));

    // Reading one listed thread through its anchor completes the list.
    let first = row.threads.items[0].clone_target(channel);
    assert_eq!(
        apply(&db, community, &actor, first).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert!(row.threads.complete);
    assert_eq!(row.threads.items.len(), 5);
    let sum: u32 = row
        .threads
        .items
        .iter()
        .map(|t| exact(&t.unread).unwrap())
        .sum();
    assert_eq!(
        Some(sum),
        exact(&row.unread),
        "complete summaries account for every reply"
    );
}

impl ThreadReadSummary {
    fn clone_target(&self, channel: Uuid) -> ReadIntent {
        ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: Some(self.root_id.clone()),
            },
            message_id: self.latest_reply_id.clone(),
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
    assert_eq!(exact(&row.unread), Some(3), "the root and two replies");
    assert_eq!(row.threads.items.len(), 2);
    let listed = &row.threads.items[1];
    assert_eq!(listed.root_id, diff.id.to_hex());
    assert_eq!(listed.latest_reply_id, on_diff.id.to_hex());

    // The listed thread is a usable context; the diff stays uncounted.
    let thread = ReadTarget {
        channel_id: channel,
        root_id: Some(listed.root_id.clone()),
    };
    let timeline = ReadTarget {
        channel_id: channel,
        root_id: None,
    };
    let queries = [(&thread, &on_diff), (&timeline, &diff)].map(|(target, message)| ContextQuery {
        target: target.clone(),
        message_ids: vec![message.id.to_hex()],
    });
    let page = db
        .personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap();
    let wire = serde_json::to_value(&page).unwrap();
    assert_eq!(
        wire["contexts"][0]["messages"][0],
        serde_json::json!({"message_id":on_diff.id.to_hex(),"status":"unread","reason":"mention"})
    );
    assert_eq!(wire["contexts"][1]["messages"][0]["status"], "not_counted");

    // The diff is still no anchor; the listed reply reads its thread alone.
    let through_diff = ReadIntent::MarkThrough {
        target: thread,
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
    assert_eq!(exact(&row.unread), Some(2));
    assert_eq!(row.threads.items.len(), 1);
    assert_eq!(row.threads.items[0].latest_reply_id, elsewhere.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_summaries_are_incomplete_when_tag_evidence_could_hide_a_root() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    join(&db, &pool, community, channel, &actor, &root).await;
    let listed = reply(
        &db,
        &pool,
        community,
        channel,
        &root,
        root.created_at.as_secs() + 1,
        vec![],
    )
    .await;
    let hidden = post(
        &db,
        community,
        channel,
        root.created_at.as_secs() + 2,
        vec![],
    )
    .await;
    sqlx::query("UPDATE events SET tags=$3 WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(hidden.id.as_bytes().as_slice())
        .bind(serde_json::json!([["e", "x".repeat(9000)]]))
        .execute(&pool)
        .await
        .unwrap();
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.threads.items.len(), 1);
    assert_eq!(row.threads.items[0].latest_reply_id, listed.id.to_hex());
    assert!(!row.threads.complete);
    assert!(!matches!(
        row.threads.items[0].unread,
        ReadCount::Exact { .. }
    ));
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
