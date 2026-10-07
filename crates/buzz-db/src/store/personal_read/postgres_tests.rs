//! Forward counts, read positions, and explicit selectors.
use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use buzz_core::{channel::MemberRole, CommunityId};
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

/// An open channel the actor created, read state started an hour ago, and one
/// message from someone else that arrived after the actor joined.
pub(super) async fn fixture() -> (Db, PgPool, CommunityId, Uuid, Keys, nostr::Event) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community(&format!("personal-read-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let actor = Keys::generate();
    let channel = db
        .create_channel(
            community,
            "private reads",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    start(&pool, community, &actor).await;
    let event = post(&db, community, channel, &Keys::generate(), now(), vec![]).await;
    (db, pool, community, channel, actor, event)
}

/// Start `who`'s read state an hour ago, as a first intent would have.
pub(super) async fn start(pool: &PgPool, community: CommunityId, who: &Keys) {
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor,started_at)
         VALUES ($1,$2,now()-interval '1 hour')",
    )
    .bind(community.as_uuid())
    .bind(who.public_key().to_bytes().as_slice())
    .execute(pool)
    .await
    .unwrap();
}

pub(super) fn now() -> u64 {
    nostr::Timestamp::now().as_secs()
}

pub(super) fn mention(who: &Keys) -> Vec<Tag> {
    vec![Tag::public_key(who.public_key())]
}

/// A top-level message authored at `at`, stored through ordinary ingest.
pub(super) async fn post(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    author: &Keys,
    at: u64,
    tags: Vec<Tag>,
) -> nostr::Event {
    post_kind(db, community, channel, author, 9, at, tags).await
}

pub(super) async fn post_kind(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    author: &Keys,
    kind: u16,
    at: u64,
    tags: Vec<Tag>,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(kind), format!("message at {at}"))
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(author)
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    event
}

/// A depth-1 reply to `root` through the ingest path that records membership.
#[allow(clippy::too_many_arguments)]
pub(super) async fn reply(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    root: &nostr::Event,
    author: &Keys,
    at: u64,
    tags: Vec<Tag>,
    broadcast: bool,
) -> nostr::Event {
    let event = EventBuilder::new(Kind::Custom(9), format!("reply at {at}"))
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(at))
        .sign_with_keys(author)
        .unwrap();
    let stamp = |e: &nostr::Event| {
        chrono::DateTime::from_timestamp(e.created_at.as_secs() as i64, 0).unwrap()
    };
    db.insert_event_with_thread_metadata(
        community,
        &event,
        Some(channel),
        Some(crate::event::ThreadMetadataParams {
            event_id: event.id.as_bytes(),
            event_created_at: stamp(&event),
            channel_id: channel,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(stamp(root)),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(stamp(root)),
            depth: 1,
            broadcast,
        }),
    )
    .await
    .unwrap();
    event
}

pub(super) async fn sidebar(db: &Db, community: CommunityId, actor: &Keys) -> ChannelReadSummary {
    db.personal_read_sidebar(community, &actor.public_key(), 20, None)
        .await
        .unwrap()
        .channels
        .remove(0)
}

pub(super) async fn apply(
    db: &Db,
    community: CommunityId,
    actor: &Keys,
    intent: ReadIntent,
) -> IntentOutcome {
    db.apply_personal_read_intent(community, &actor.public_key(), &intent)
        .await
        .unwrap()
}

/// Mark the channel timeline, or `root`'s thread, through `message`.
pub(super) fn mark(
    channel: Uuid,
    root: Option<&nostr::Event>,
    message: &nostr::Event,
) -> ReadIntent {
    ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: root.map(|r| r.id.to_hex()),
        },
        message_id: message.id.to_hex(),
    }
}

pub(super) fn channel_read(channel: Uuid, message: &nostr::Event) -> ReadIntent {
    ReadIntent::MarkChannelRead {
        channel_id: channel,
        message_id: message.id.to_hex(),
    }
}

/// States of `messages` in the timeline, or `root`'s thread, as wire JSON.
pub(super) async fn states(
    db: &Db,
    community: CommunityId,
    actor: &Keys,
    channel: Uuid,
    root: Option<&nostr::Event>,
    messages: &[&nostr::Event],
) -> Vec<serde_json::Value> {
    let query = ContextQuery {
        target: ReadTarget {
            channel_id: channel,
            root_id: root.map(|r| r.id.to_hex()),
        },
        message_ids: messages.iter().map(|e| e.id.to_hex()).collect(),
    };
    let page = db
        .personal_read_contexts(community, &actor.public_key(), &[query])
        .await
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    page["contexts"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let mut m = m.clone();
            m.as_object_mut().unwrap().remove("message_id");
            m
        })
        .collect()
}

pub(super) async fn status(
    db: &Db,
    community: CommunityId,
    actor: &Keys,
    channel: Uuid,
    root: Option<&nostr::Event>,
    messages: &[&nostr::Event],
) -> Vec<String> {
    states(db, community, actor, channel, root, messages)
        .await
        .iter()
        .map(|m| m["status"].as_str().unwrap().to_owned())
        .collect()
}

/// Started accounts; ingest's membership accounts are not started.
async fn accounts(pool: &PgPool, community: CommunityId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM personal_read_accounts WHERE community_id=$1 AND started_at IS NOT NULL",
    )
        .bind(community.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_row_has_the_wire_shape() {
    let (db, _, community, channel, actor, _) = fixture().await;
    let author = Keys::generate();
    let base = now();
    let mut last = None;
    for i in 0..120 {
        last = Some(post(&db, community, channel, &author, base + i, vec![]).await);
    }
    let row = sidebar(&db, community, &actor).await;
    let wire = serde_json::to_value(&row).unwrap();
    let mut keys: Vec<_> = wire.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "archived",
            "channel_id",
            "channel_type",
            "hidden",
            "mentions",
            "name",
            "read_through_id",
            "threads",
            "unread"
        ]
    );
    assert_eq!(wire["unread"], true);
    assert_eq!(wire["mentions"], 0);
    assert_eq!(
        wire["read_through_id"],
        serde_json::Value::Null,
        "nothing read yet"
    );
    let last = last.unwrap();
    assert_eq!(
        apply(&db, community, &actor, mark(channel, None, &last)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &actor).await;
    assert!(!row.unread);
    assert_eq!(row.read_through_id, Some(last.id.to_hex()));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn joining_starts_caught_up_and_counts_only_what_arrives_after() {
    let (db, pool, community, channel, owner, event) = fixture().await;
    assert!(sidebar(&db, community, &owner).await.unread);
    let joiner = Keys::generate();
    start(&pool, community, &joiner).await;
    let before = accounts(&pool, community).await;
    db.add_member(
        community,
        channel,
        &joiner.public_key().to_bytes(),
        MemberRole::Member,
        None,
    )
    .await
    .unwrap();
    let row = sidebar(&db, community, &joiner).await;
    assert_eq!(
        (row.unread, row.mentions),
        (false, 0),
        "history before joining is read"
    );
    assert_eq!(row.read_through_id, Some(event.id.to_hex()));
    assert_eq!(
        status(&db, community, &joiner, channel, None, &[&event]).await,
        ["read"]
    );
    assert_eq!(
        accounts(&pool, community).await,
        before,
        "joining writes nothing"
    );
    let later = post(
        &db,
        community,
        channel,
        &Keys::generate(),
        now(),
        mention(&joiner),
    )
    .await;
    let row = sidebar(&db, community, &joiner).await;
    assert_eq!(
        (row.unread, row.mentions),
        (true, 1),
        "a mention is unread and counted"
    );
    assert_eq!(
        states(&db, community, &joiner, channel, None, &[&later]).await,
        [serde_json::json!({"status":"unread","reason":"mention"})]
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rejoining_starts_caught_up_again() {
    let (db, pool, community, channel, owner, _) = fixture().await;
    let member = Keys::generate();
    let pk = member.public_key().to_bytes();
    start(&pool, community, &member).await;
    db.add_member(community, channel, &pk, MemberRole::Member, None)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE channel_members SET joined_at=now()-interval '1 hour'
         WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(pk.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    // The member follows a thread; the follow survives leaving.
    let root = post(&db, community, channel, &member, now(), vec![]).await;
    reply(
        &db,
        community,
        channel,
        &root,
        &member,
        now(),
        vec![],
        false,
    )
    .await;
    db.remove_member(community, channel, &pk, &pk)
        .await
        .unwrap();
    let away = post(&db, community, channel, &Keys::generate(), now(), vec![]).await;
    let away_reply = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        now(),
        mention(&member),
        false,
    )
    .await;
    // Re-adding an active member (a role change) must not move the position.
    let owner_pk = owner.public_key().to_bytes();
    db.add_member(
        community,
        channel,
        &owner_pk,
        MemberRole::Owner,
        Some(&owner_pk),
    )
    .await
    .unwrap();
    assert!(sidebar(&db, community, &owner).await.unread);
    db.add_member(community, channel, &pk, MemberRole::Member, None)
        .await
        .unwrap();
    let row = sidebar(&db, community, &member).await;
    assert!(!row.unread, "messages from while away are read");
    assert!(row.threads.is_empty(), "so are replies in followed threads");
    assert_eq!(
        status(&db, community, &member, channel, None, &[&away]).await,
        ["read"]
    );
    assert_eq!(
        status(
            &db,
            community,
            &member,
            channel,
            Some(&root),
            &[&away_reply]
        )
        .await,
        ["read"]
    );
    post(&db, community, channel, &Keys::generate(), now(), vec![]).await;
    assert!(sidebar(&db, community, &member).await.unread);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn read_state_starts_caught_up_at_the_first_intent() {
    let (db, pool, community, channel, _, _) = fixture().await;
    let member = Keys::generate();
    db.add_member(
        community,
        channel,
        &member.public_key().to_bytes(),
        MemberRole::Member,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE channel_members SET joined_at=now()-interval '1 hour' WHERE community_id=$1 AND pubkey=$2")
        .bind(community.as_uuid())
        .bind(member.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    // Ingest gives the member a thread row, and so an account, unstarted.
    let root = post(&db, community, channel, &member, now(), vec![]).await;
    reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        now(),
        vec![],
        false,
    )
    .await;
    let before = post(
        &db,
        community,
        channel,
        &Keys::generate(),
        now(),
        mention(&member),
    )
    .await;
    let row = sidebar(&db, community, &member).await;
    assert_eq!((row.unread, row.mentions, row.threads.len()), (false, 0, 0));
    assert_eq!(
        status(&db, community, &member, channel, None, &[&before]).await,
        ["not_counted"]
    );

    // Any first intent starts every position, caught up, at that moment.
    assert_eq!(
        apply(&db, community, &member, mark(channel, None, &root)).await,
        IntentOutcome::Applied
    );
    let row = sidebar(&db, community, &member).await;
    assert_eq!((row.unread, row.threads.len()), (false, 0));
    assert_eq!(
        status(&db, community, &member, channel, None, &[&before]).await,
        ["read"]
    );
    reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        now(),
        vec![],
        false,
    )
    .await;
    post(&db, community, channel, &Keys::generate(), now(), vec![]).await;
    let row = sidebar(&db, community, &member).await;
    assert_eq!((row.unread, row.mentions, row.threads.len()), (true, 0, 1));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sidebar_is_read_only_and_does_not_wait_for_account() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    sidebar(&db, community, &actor).await;
    assert_eq!(
        accounts(&pool, community).await,
        1,
        "GET must not create private state"
    );
    apply(&db, community, &actor, channel_read(channel, &event)).await;
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT actor FROM personal_read_accounts WHERE community_id=$1 FOR UPDATE")
        .bind(community.as_uuid())
        .fetch_all(&mut *held)
        .await
        .unwrap();
    assert!(!sidebar(&db, community, &actor).await.unread);
    held.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn intent_does_not_lock_shared_conversation_rows_or_block_thread_ingest() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    sqlx::query("UPDATE channels SET ttl_seconds=86400 WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    let mut held = pool.begin().await.unwrap();
    super::writes::lock_account(&mut held, community, &actor.public_key().to_bytes())
        .await
        .unwrap();
    let result = super::writes::apply(
        &mut held,
        community,
        &actor.public_key().to_bytes(),
        &mark(channel, None, &event),
    )
    .await
    .unwrap();
    assert!(matches!(result, IntentOutcome::Applied));
    // Ingest that creates the locked actor's thread row must not wait on the
    // uncommitted intent, nor must the event-insert TTL trigger.
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reply(
            &db,
            community,
            channel,
            &event,
            &Keys::generate(),
            now(),
            mention(&actor),
            false,
        ),
    )
    .await
    .unwrap();
    let mut legacy = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='100ms'")
        .execute(&mut *legacy)
        .await
        .unwrap();
    sqlx::query("UPDATE channels SET ttl_deadline=clock_timestamp()+interval '1 day' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(channel).execute(&mut *legacy).await.unwrap();
    sqlx::query("UPDATE events SET deleted_at=clock_timestamp() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *legacy)
        .await
        .unwrap();
    legacy.rollback().await.unwrap();
    held.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn diff_alone_leaves_the_sidebar_row_unchanged() {
    let (db, _, community, channel, actor, _) = fixture().await;
    let before = serde_json::to_value(sidebar(&db, community, &actor).await).unwrap();
    // Newest in the channel and addressed to the actor: as loud as a diff gets.
    post_kind(
        &db,
        community,
        channel,
        &Keys::generate(),
        40008,
        now() + 5,
        mention(&actor),
    )
    .await;
    let after = serde_json::to_value(sidebar(&db, community, &actor).await).unwrap();
    assert_eq!(before["unread"], true);
    assert_eq!(before, after, "not unread, not a mention");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn posting_marks_read_and_read_through_skips_deleted_and_auxiliary() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let base = now();
    let own = post(&db, community, channel, &actor, base + 1, vec![]).await;
    let deleted = post(&db, community, channel, &actor, base + 2, vec![]).await;
    post_kind(&db, community, channel, &actor, 7, base + 4, vec![]).await;
    sqlx::query("UPDATE events SET deleted_at=clock_timestamp() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(deleted.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let row = sidebar(&db, community, &actor).await;
    assert!(!row.unread, "posting read through everything before it");
    assert_eq!(
        row.read_through_id,
        Some(own.id.to_hex()),
        "not deleted, not a reaction"
    );
    // An empty channel has nothing read.
    db.create_channel(
        community,
        "truly empty",
        ChannelType::Stream,
        ChannelVisibility::Open,
        None,
        &actor.public_key().to_bytes(),
        None,
    )
    .await
    .unwrap();
    let page = db
        .personal_read_sidebar(community, &actor.public_key(), 20, None)
        .await
        .unwrap();
    let empty = page
        .channels
        .iter()
        .find(|c| c.channel_id != channel)
        .unwrap();
    assert!(!empty.unread && empty.mentions == 0 && empty.read_through_id.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_unread_follows_timeline_arrivals_only() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = now();
    let other = Keys::generate();
    let last = |pool: PgPool| async move {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT last_timeline_received_at FROM channels WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(channel)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(
        apply(&db, community, &actor, mark(channel, None, &root)).await,
        IntentOutcome::Applied
    );
    let caught_up = last(pool.clone()).await;
    assert!(caught_up.is_some(), "the fixture's message set it");
    assert!(!sidebar(&db, community, &actor).await.unread);

    // A thread reply is not on the timeline: the channel stays read.
    reply(
        &db,
        community,
        channel,
        &root,
        &other,
        base + 1,
        vec![],
        false,
    )
    .await;
    assert_eq!(last(pool.clone()).await, caught_up);
    assert!(!sidebar(&db, community, &actor).await.unread);

    // A reaction (not an eligible kind) doesn't move it; a broadcast reply,
    // which shows on the timeline, does.
    post_kind(&db, community, channel, &other, 7, base + 2, vec![]).await;
    assert_eq!(last(pool.clone()).await, caught_up);
    let broadcast = reply(
        &db,
        community,
        channel,
        &root,
        &other,
        base + 3,
        vec![],
        true,
    )
    .await;
    assert!(last(pool.clone()).await > caught_up);
    assert!(sidebar(&db, community, &actor).await.unread);

    // Deleting the only unread timeline message leaves the channel read: the
    // probe confirms what the arrival time suggests.
    sqlx::query("UPDATE events SET deleted_at=clock_timestamp() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(broadcast.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    assert!(!sidebar(&db, community, &actor).await.unread);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn top_level_message_with_depth_zero_metadata_marks_read_and_follows() {
    // Workflow messages store a depth-0 metadata row with no parent.
    let (db, _, community, channel, actor, _) = fixture().await;
    let base = now();
    let author = Keys::generate();
    db.add_member(
        community,
        channel,
        &author.public_key().to_bytes(),
        MemberRole::Member,
        None,
    )
    .await
    .unwrap();
    let insert = |who: Keys, at: u64, tags: Vec<Tag>| {
        let db = db.clone();
        async move {
            let event = EventBuilder::new(Kind::Custom(9), format!("workflow at {at}"))
                .tags(tags)
                .custom_created_at(nostr::Timestamp::from(at))
                .sign_with_keys(&who)
                .unwrap();
            db.insert_event_with_thread_metadata(
                community,
                &event,
                Some(channel),
                Some(crate::event::ThreadMetadataParams {
                    event_id: event.id.as_bytes(),
                    event_created_at: chrono::DateTime::from_timestamp(at as i64, 0).unwrap(),
                    channel_id: channel,
                    parent_event_id: None,
                    parent_event_created_at: None,
                    root_event_id: None,
                    root_event_created_at: None,
                    depth: 0,
                    broadcast: false,
                }),
            )
            .await
            .unwrap();
            event
        }
    };
    let own = insert(actor.clone(), base + 1, vec![]).await;
    let row = sidebar(&db, community, &actor).await;
    assert!(!row.unread, "posting read through it");
    assert_eq!(row.read_through_id, Some(own.id.to_hex()));

    let root = insert(author.clone(), base + 2, mention(&actor)).await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!((row.unread, row.mentions), (true, 1));
    reply(
        &db,
        community,
        channel,
        &root,
        &author,
        base + 3,
        vec![],
        false,
    )
    .await;
    let row = sidebar(&db, community, &actor).await;
    assert_eq!(row.threads.len(), 1, "the mention followed its thread");
    assert_eq!(row.threads[0].root_id, root.id.to_hex());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn position_is_monotonic_and_rejects_malformed_anchors() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    let invalid = ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_id: "not an event id".into(),
    };
    assert_eq!(
        apply(&db, community, &actor, invalid).await,
        IntentOutcome::Invalid
    );
    assert_eq!(
        accounts(&pool, community).await,
        1,
        "invalid intent rolls back"
    );
    for _ in 0..2 {
        assert_eq!(
            apply(&db, community, &actor, mark(channel, None, &event)).await,
            IntentOutcome::Applied
        );
    }
    // Arrives after `event`; its earlier author time must not matter.
    let later = post(
        &db,
        community,
        channel,
        &Keys::generate(),
        now() - 10,
        vec![],
    )
    .await;
    assert!(sidebar(&db, community, &actor).await.unread);
    for anchor in [&later, &event] {
        assert_eq!(
            apply(&db, community, &actor, mark(channel, None, anchor)).await,
            IntentOutcome::Applied
        );
    }
    assert!(
        !sidebar(&db, community, &actor).await.unread,
        "never moves back"
    );
    let at_later: bool = sqlx::query_scalar(
        "SELECT f.through_timestamp=e.received_at FROM personal_read_frontiers f, events e
         WHERE f.community_id=$1 AND f.actor=$2 AND e.community_id=$1 AND e.id=$3",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .bind(later.id.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(at_later);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn contexts_bound_selectors_and_share_one_state_per_message() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = now();
    let broadcast = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base,
        vec![],
        true,
    )
    .await;
    let thread_only = reply(
        &db,
        community,
        channel,
        &root,
        &Keys::generate(),
        base,
        vec![],
        false,
    )
    .await;
    let both = [&root, &broadcast, &thread_only];
    // A broadcast reply is on the timeline and in its thread; a plain reply
    // is only in the thread, and the actor is not in it.
    assert_eq!(
        status(&db, community, &actor, channel, None, &both).await,
        ["unread", "unread", "unavailable"]
    );
    assert_eq!(
        status(&db, community, &actor, channel, Some(&root), &both).await,
        ["unavailable", "unread", "not_counted"]
    );
    assert!(sidebar(&db, community, &actor).await.unread);
    assert_eq!(
        accounts(&pool, community).await,
        1,
        "context GET creates nothing"
    );
    // A broadcast reply is a valid timeline anchor.
    assert_eq!(
        apply(&db, community, &actor, mark(channel, None, &broadcast)).await,
        IntentOutcome::Applied
    );
    assert_eq!(
        status(&db, community, &actor, channel, Some(&root), &both).await,
        ["unavailable", "read", "not_counted"]
    );
    let empty: [ContextQuery; 0] = [];
    assert!(db
        .personal_read_contexts(community, &actor.public_key(), &empty)
        .await
        .is_err());
    let oversized = [ContextQuery {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_ids: vec![root.id.to_hex(); MAX_CONTEXT_MESSAGES + 1],
    }];
    assert!(db
        .personal_read_contexts(community, &actor.public_key(), &oversized)
        .await
        .is_err());
    // A deleted message is not counted.
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(root.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let other = Keys::generate();
    assert_eq!(
        status(&db, community, &other, channel, None, &[&root]).await,
        ["not_counted"]
    );
    sqlx::query("UPDATE channels SET visibility='private' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    let page = db
        .personal_read_contexts(
            community,
            &other.public_key(),
            &[ContextQuery {
                target: ReadTarget {
                    channel_id: channel,
                    root_id: None,
                },
                message_ids: vec![root.id.to_hex()],
            }],
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(page).unwrap()["contexts"],
        serde_json::json!([{"status":"unavailable"}])
    );
    assert!(db
        .personal_read_accessible_contexts(community, &other.public_key(), &[channel])
        .await
        .unwrap()
        .is_empty());
}
