use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sqlx::PgPool;
use uuid::Uuid;

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
    let event = EventBuilder::new(Kind::Custom(9), "read this, not its sibling")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &event, Some(channel))
        .await
        .unwrap();
    start_before_everything(&pool, community, &actor.public_key()).await;
    (db, pool, community, channel, actor, event)
}

/// Start an account before any fixture arrival, so unread semantics can be
/// tested without the start floor covering every message.
pub(super) async fn start_before_everything(
    pool: &PgPool,
    community: CommunityId,
    actor: &nostr::PublicKey,
) {
    sqlx::query(
        "INSERT INTO personal_read_accounts (community_id,actor,started_at)
         VALUES ($1,$2,'1900-01-01T00:00:00Z')",
    )
    .bind(community.as_uuid())
    .bind(actor.to_bytes().as_slice())
    .execute(pool)
    .await
    .unwrap();
}

/// Move every community message past the default horizon by author time, the
/// only clock the unread window reads.
async fn expire(pool: &PgPool, community: CommunityId) {
    sqlx::query("UPDATE events SET created_at=created_at-interval '31 days' WHERE community_id=$1")
        .bind(community.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

async fn sidebar(db: &Db, community: CommunityId, actor: &Keys) -> SidebarPage {
    db.personal_read_sidebar(
        community,
        &actor.public_key(),
        DEFAULT_RETENTION_SECONDS,
        20,
        None,
    )
    .await
    .unwrap()
}

fn mark_channel(channel: Uuid, message: &nostr::Event) -> ReadIntent {
    ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_id: message.id.to_hex(),
    }
}

fn mark_thread(channel: Uuid, root: &nostr::Event, message: &nostr::Event) -> ReadIntent {
    ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: Some(root.id.to_hex()),
        },
        message_id: message.id.to_hex(),
    }
}

/// Address every stored community message to `actor`, so the channel's
/// mention count is exactly its unread top-level count.
async fn mention_everywhere(pool: &PgPool, community: CommunityId, actor: &Keys) {
    sqlx::query("UPDATE events SET tags=$2 WHERE community_id=$1")
        .bind(community.as_uuid())
        .bind(serde_json::json!([["p", actor.public_key().to_hex()]]))
        .execute(pool)
        .await
        .unwrap();
}

async fn add_history(db: &Db, community: CommunityId, channel: Uuid, count: usize) -> nostr::Event {
    let author = Keys::generate();
    let mut last = None;
    let base = nostr::Timestamp::now().as_secs();
    for i in 0..count as u64 {
        let event = EventBuilder::new(Kind::Custom(9), format!("history {i}"))
            .custom_created_at(nostr::Timestamp::from(base + i))
            .sign_with_keys(&author)
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        last = Some(event);
    }
    last.unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_marked_history_past_the_scan_cap_is_read_and_keeps_latest() {
    let (db, _pool, community, channel, actor, _) = fixture().await;
    let last = add_history(&db, community, channel, MAX_UNREAD_SCAN + 44).await;
    let outcome = db
        .apply_personal_read_intent(
            community,
            &actor.public_key(),
            &mark_channel(channel, &last),
        )
        .await
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Applied);
    // The scan never reaches the oldest 45 messages; nothing is unread.
    let row = &sidebar(&db, community, &actor).await.channels[0];
    assert!(!row.unread);
    assert_eq!(row.mentions, 0);
    assert_eq!(row.latest_id.as_deref(), Some(last.id.to_hex().as_str()));
    assert_eq!(row.read_through_id, row.latest_id);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_author_window_excludes_expired_and_late_old_messages() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let latest = add_history(&db, community, channel, MAX_UNREAD_SCAN + 44).await;
    expire(&pool, community).await;
    let row = &sidebar(&db, community, &actor).await.channels[0];
    assert!(
        !row.unread,
        "an empty author-time window has nothing unread"
    );
    assert_eq!(row.latest_id.as_deref(), Some(latest.id.to_hex().as_str()));
    // Acceptance time is irrelevant: a message accepted now with an author
    // time beyond the horizon stays out; one authored now is counted.
    let now = nostr::Timestamp::now().as_secs();
    for (age, unread, mentions) in [(40 * 86400, false, 0), (0, true, 1)] {
        let event = EventBuilder::new(Kind::Custom(9), "accepted now")
            .tags([Tag::public_key(actor.public_key())])
            .custom_created_at(nostr::Timestamp::from(now - age))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        let row = &sidebar(&db, community, &actor).await.channels[0];
        assert_eq!((row.unread, row.mentions), (unread, mentions), "age={age}");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_window_budget_counts_boundary_and_ineligible_tail() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    // The fixture contributes one event, so this is exactly the evidence budget.
    add_history(&db, community, channel, MAX_UNREAD_SCAN - 1).await;
    mention_everywhere(&pool, community, &actor).await;
    let mentions = |page: SidebarPage| page.channels[0].mentions;
    assert_eq!(
        mentions(sidebar(&db, community, &actor).await),
        MAX_UNREAD_SCAN as u32
    );
    let overflow = EventBuilder::new(Kind::Custom(9), "one beyond the budget")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &overflow, Some(channel))
        .await
        .unwrap();
    mention_everywhere(&pool, community, &actor).await;
    // The budget bounds the count; the unexamined message is left out.
    assert_eq!(
        mentions(sidebar(&db, community, &actor).await),
        MAX_UNREAD_SCAN as u32
    );
    // Expire all but 601 messages. Of these, 300 are own and 300 deleted.
    // Eligibility is downstream of the bounded unread window, not the old 256 cap.
    sqlx::query("WITH ranked AS (SELECT created_at,id,row_number() OVER (ORDER BY created_at DESC,id) AS n FROM events WHERE community_id=$1 AND channel_id=$2)
        UPDATE events e SET created_at=CASE WHEN r.n>601 THEN e.created_at-interval '31 days' ELSE e.created_at END,
          pubkey=CASE WHEN r.n<=300 THEN $3 ELSE e.pubkey END,
          deleted_at=CASE WHEN r.n>300 AND r.n<=600 THEN now() ELSE NULL END
        FROM ranked r WHERE e.community_id=$1 AND e.created_at=r.created_at AND e.id=r.id")
        .bind(community.as_uuid()).bind(channel).bind(actor.public_key().to_bytes().as_slice()).execute(&pool).await.unwrap();
    let page = sidebar(&db, community, &actor).await;
    assert!(page.channels[0].unread);
    assert_eq!(mentions(page), 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_is_read_only_and_does_not_wait_for_account() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    unstart(&pool, community).await;
    sidebar(&db, community, &actor).await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personal_read_accounts WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "GET must not create private state");
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &mark_channel(channel, &event),
    )
    .await
    .unwrap();
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT actor FROM personal_read_accounts WHERE community_id=$1 FOR UPDATE")
        .bind(community.as_uuid())
        .fetch_all(&mut *held)
        .await
        .unwrap();
    assert!(!sidebar(&db, community, &actor).await.channels[0].unread);
    held.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_intent_does_not_lock_shared_conversation_rows() {
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
        &ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: None,
            },
            message_id: event.id.to_hex(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(result, IntentOutcome::Applied));
    // Exercise the actual event-insert TTL trigger while private progress is
    // uncommitted, then verify event deletion can update the observed row.
    let incoming = EventBuilder::new(Kind::Custom(9), "concurrent ephemeral ingest")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        db.insert_event(community, &incoming, Some(channel)),
    )
    .await
    .unwrap()
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
async fn personal_read_diff_alone_leaves_the_sidebar_row_unchanged() {
    let (db, _, community, channel, actor, _) = fixture().await;
    let mut rows = Vec::new();
    for diff in [false, true] {
        if diff {
            // Newest in the channel and addressed to the actor: as loud as a
            // diff can be.
            let at = nostr::Timestamp::now().as_secs() + 5;
            let event = EventBuilder::new(Kind::Custom(40008), "a diff")
                .custom_created_at(nostr::Timestamp::from(at))
                .tags([nostr::Tag::parse(["p", &actor.public_key().to_hex()]).unwrap()])
                .sign_with_keys(&Keys::generate())
                .unwrap();
            db.insert_event(community, &event, Some(channel))
                .await
                .unwrap();
        }
        let page = sidebar(&db, community, &actor).await;
        assert!(page.channels[0].unread);
        rows.push(serde_json::to_value(&page.channels[0]).unwrap());
    }
    assert_eq!(rows[0], rows[1], "not unread, not a mention, not latest");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_latest_includes_own_and_excludes_deleted_auxiliary() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let base = nostr::Timestamp::now().as_secs();
    let own = EventBuilder::new(Kind::Custom(9), "own latest")
        .custom_created_at(nostr::Timestamp::from(base + 1))
        .sign_with_keys(&actor)
        .unwrap();
    db.insert_event(community, &own, Some(channel))
        .await
        .unwrap();
    for (offset, kind) in [(2, 9), (4, 7)] {
        let event = EventBuilder::new(Kind::Custom(kind), format!("ineligible {offset}"))
            .custom_created_at(nostr::Timestamp::from(base + offset))
            .sign_with_keys(&actor)
            .unwrap();
        db.insert_event(community, &event, Some(channel))
            .await
            .unwrap();
        if offset == 2 {
            sqlx::query(
                "UPDATE events SET deleted_at=clock_timestamp() WHERE community_id=$1 AND id=$2",
            )
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    let latest = |page: SidebarPage| page.channels[0].latest_id.clone();
    assert_eq!(
        latest(sidebar(&db, community, &actor).await),
        Some(own.id.to_hex())
    );
    expire(&pool, community).await;
    assert_eq!(
        latest(sidebar(&db, community, &actor).await),
        Some(own.id.to_hex())
    );
}

/// A reply marker without canonical ancestry cannot anchor a timeline mark, so
/// it is never the channel's latest: marking through latest must succeed.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_latest_skips_a_reply_without_ancestry() {
    let (db, pool, community, channel, actor, first) = fixture().await;
    let orphan = EventBuilder::new(Kind::Custom(9), "reply to a missing parent")
        .tags([nostr::Tag::parse(["e", &"ab".repeat(32), "", "reply"]).unwrap()])
        .custom_created_at(nostr::Timestamp::from(first.created_at.as_secs() + 1))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &orphan, Some(channel))
        .await
        .unwrap();
    let latest = |page: SidebarPage| page.channels[0].latest_id.clone();
    assert_eq!(
        latest(sidebar(&db, community, &actor).await),
        Some(first.id.to_hex())
    );
    // The shallow probe applies the same rule once the horizon is empty.
    expire(&pool, community).await;
    assert_eq!(
        latest(sidebar(&db, community, &actor).await),
        Some(first.id.to_hex())
    );
    let outcome = db
        .apply_personal_read_intent(
            community,
            &actor.public_key(),
            &mark_channel(channel, &first),
        )
        .await
        .unwrap();
    assert_eq!(outcome, IntentOutcome::Applied);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_latest_old_message_is_not_an_empty_channel() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    expire(&pool, community).await;
    let empty = db
        .create_channel(
            community,
            "truly empty",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let page = sidebar(&db, community, &actor).await;
    let old = page
        .channels
        .iter()
        .find(|c| c.channel_id == channel)
        .unwrap();
    assert_eq!(old.latest_id.as_deref(), Some(event.id.to_hex().as_str()));
    assert!(!old.unread);
    let empty = page
        .channels
        .iter()
        .find(|c| c.channel_id == empty)
        .unwrap();
    assert!(empty.latest_id.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_frontier_is_monotonic_and_rejects_malformed_anchors() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    unstart(&pool, community).await;
    let read_through = || async {
        sidebar(&db, community, &actor).await.channels[0]
            .read_through_id
            .clone()
    };
    assert_eq!(read_through().await, None, "never marked");
    let target = ReadTarget {
        channel_id: channel,
        root_id: None,
    };
    let invalid = ReadIntent::MarkThrough {
        target: target.clone(),
        message_id: "not an event id".into(),
    };
    assert_eq!(
        db.apply_personal_read_intent(community, &actor.public_key(), &invalid)
            .await
            .unwrap(),
        IntentOutcome::Invalid
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personal_read_accounts WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "invalid intent rolls back account creation");
    let intent = mark_channel(channel, &event);
    for _ in 0..2 {
        assert_eq!(
            db.apply_personal_read_intent(community, &actor.public_key(), &intent)
                .await
                .unwrap(),
            IntentOutcome::Applied
        );
    }
    assert_eq!(read_through().await, Some(event.id.to_hex()));
    // Arrives after `event`; its earlier author time must not matter.
    let later = EventBuilder::new(Kind::Custom(9), "later")
        .custom_created_at(nostr::Timestamp::from(event.created_at.as_secs() - 10))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &later, Some(channel))
        .await
        .unwrap();
    for (anchor, why) in [
        (&later, "a later arrival advances the frontier"),
        (&event, "an earlier anchor applies without moving it back"),
    ] {
        assert_eq!(
            db.apply_personal_read_intent(
                community,
                &actor.public_key(),
                &mark_channel(channel, anchor)
            )
            .await
            .unwrap(),
            IntentOutcome::Applied,
            "{why}"
        );
    }
    let at_later_arrival: bool = sqlx::query_scalar(
        "SELECT f.through_timestamp=e.received_at FROM personal_read_frontiers f, events e
         WHERE f.community_id=$1 AND f.actor=$2 AND e.community_id=$1 AND e.id=$3",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .bind(later.id.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(at_later_arrival);
    assert_eq!(
        read_through().await,
        Some(later.id.to_hex()),
        "the anchor follows the frontier, not the last intent"
    );
    let stranger = Keys::generate();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(stranger.public_key().to_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    let sparse: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('personal_read_seen')::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sparse.is_none());
}

/// Undo the fixture's start, for tests of what creates an account.
async fn unstart(pool: &PgPool, community: CommunityId) {
    sqlx::query("DELETE FROM personal_read_accounts WHERE community_id=$1")
        .bind(community.as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

async fn started_at(
    pool: &PgPool,
    community: CommunityId,
    actor: &nostr::PublicKey,
) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar(
        "SELECT started_at FROM personal_read_accounts WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(actor.to_bytes().as_slice())
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_first_applied_intent_starts_the_account_once() {
    let (db, pool, community, channel, _, event) = fixture().await;
    let intent = ReadIntent::MarkThrough {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_id: event.id.to_hex(),
    };
    // The fixture actor is already started; the channel is open to anyone.
    let actor = Keys::generate().public_key();
    assert!(sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT actor FROM personal_read_accounts WHERE community_id=$1 AND actor=$2"
    )
    .bind(community.as_uuid())
    .bind(actor.to_bytes().as_slice())
    .fetch_optional(&pool)
    .await
    .unwrap()
    .is_none());
    for _ in 0..2 {
        assert_eq!(
            db.apply_personal_read_intent(community, &actor, &intent)
                .await
                .unwrap(),
            IntentOutcome::Applied
        );
    }
    let first = started_at(&pool, community, &actor).await;
    assert!(
        first.is_some(),
        "the first applied intent starts the account"
    );

    // An account can exist before its actor starts; the first intent starts
    // it, and later intents never move the start.
    let pending = Keys::generate().public_key();
    sqlx::query("INSERT INTO personal_read_accounts (community_id,actor) VALUES ($1,$2)")
        .bind(community.as_uuid())
        .bind(pending.to_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(started_at(&pool, community, &pending).await, None);
    db.apply_personal_read_intent(community, &pending, &intent)
        .await
        .unwrap();
    let pending_start = started_at(&pool, community, &pending).await;
    assert!(pending_start >= first);
    db.apply_personal_read_intent(community, &pending, &intent)
        .await
        .unwrap();
    assert_eq!(started_at(&pool, community, &pending).await, pending_start);
    assert_eq!(started_at(&pool, community, &actor).await, first);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_counts_only_arrivals_after_the_account_starts() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    unstart(&pool, community).await;
    let unread = |page: &SidebarPage, id: Uuid| {
        let row = page.channels.iter().find(|c| c.channel_id == id).unwrap();
        (row.unread, row.mentions)
    };
    // Before the first read intent nothing counts, not the 30-day window.
    assert_eq!(
        unread(&sidebar(&db, community, &actor).await, channel),
        (false, 0)
    );

    // The first intent, in another channel, starts the account everywhere.
    let other = db
        .create_channel(
            community,
            "elsewhere",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &actor.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let elsewhere = EventBuilder::new(Kind::Custom(9), "elsewhere")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &elsewhere, Some(other))
        .await
        .unwrap();
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &mark_channel(other, &elsewhere),
    )
    .await
    .unwrap();
    // A channel never marked starts caught up: earlier arrivals stay read.
    assert_eq!(
        unread(&sidebar(&db, community, &actor).await, channel),
        (false, 0)
    );

    // Arrivals after the start count.
    let later = EventBuilder::new(Kind::Custom(9), "after the start")
        .tags([Tag::public_key(actor.public_key())])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &later, Some(channel))
        .await
        .unwrap();
    assert_eq!(
        unread(&sidebar(&db, community, &actor).await, channel),
        (true, 1)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_unstarted_account_is_caught_up_past_the_scan_cap() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    unstart(&pool, community).await;
    let last = add_history(&db, community, channel, MAX_UNREAD_SCAN + 44).await;
    let page = sidebar(&db, community, &actor).await;
    let row = &page.channels[0];
    // The start floor covers every arrival, examined by the scan or not.
    assert!(!row.unread);
    assert_eq!(row.mentions, 0);
    assert!(row.threads.is_empty());
    assert_eq!(row.latest_id.as_deref(), Some(last.id.to_hex().as_str()));
    assert_eq!(row.read_through_id, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_channel_and_thread_never_inherit_each_other() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    // Directed, so they count outside the actor's conversations.
    let reply_at = |at: u64| {
        let (db, pool) = (db.clone(), pool.clone());
        let mention = Tag::public_key(actor.public_key());
        let root = root.id;
        async move {
            let reply = EventBuilder::new(Kind::Custom(9), "unseen thread reply")
                .tags([mention])
                .custom_created_at(nostr::Timestamp::from(at))
                .sign_with_keys(&Keys::generate())
                .unwrap();
            db.insert_event(community, &reply, Some(channel))
                .await
                .unwrap();
            sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
                VALUES ($1,$2,to_timestamp($3),$4,$5,$5,1)")
                .bind(community.as_uuid()).bind(reply.id.as_bytes().as_slice()).bind(at as f64)
                .bind(channel).bind(root.as_bytes().as_slice()).execute(&pool).await.unwrap();
            reply
        }
    };
    let reply = reply_at(base + 10).await;
    let top = EventBuilder::new(Kind::Custom(9), "later timeline message")
        .custom_created_at(nostr::Timestamp::from(base + 20))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &top, Some(channel))
        .await
        .unwrap();
    // Arrives last, and still is not the timeline's latest.
    let newer = reply_at(base + 30).await;
    let apply = |intent: ReadIntent| {
        let db = db.clone();
        let actor = actor.public_key();
        async move {
            db.apply_personal_read_intent(community, &actor, &intent)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        apply(mark_channel(channel, &reply)).await,
        IntentOutcome::Blocked,
        "a reply is never a timeline anchor"
    );
    let row = |page: SidebarPage| serde_json::to_value(&page.channels[0]).unwrap();
    let thread = |mentions: u32, read_through: Option<&nostr::Event>| {
        serde_json::json!({"root_id":root.id.to_hex(),"unread":true,"mentions":mentions,
            "read_through_id":read_through.map(|e| e.id.to_hex()),"latest_id":newer.id.to_hex()})
    };
    let expected =
        |read_through: Option<&nostr::Event>, unread: bool, threads: serde_json::Value| {
            serde_json::json!({"channel_id":channel,"unread":unread,"mentions":0,
            "read_through_id":read_through.map(|e| e.id.to_hex()),
            "latest_id":top.id.to_hex(),"threads":threads})
        };
    // Unread replies show only on their thread row, never the channel's.
    assert_eq!(
        row(sidebar(&db, community, &actor).await),
        expected(None, true, serde_json::json!([thread(2, None)]))
    );
    assert_eq!(
        apply(mark_channel(channel, &top)).await,
        IntentOutcome::Applied
    );
    assert_eq!(
        row(sidebar(&db, community, &actor).await),
        expected(Some(&top), false, serde_json::json!([thread(2, None)])),
        "timeline reading must leave unseen replies unread"
    );
    // Thread marks move only the thread; an earlier anchor (the root) applies
    // without moving it back.
    for anchor in [&reply, &root] {
        assert_eq!(
            apply(mark_thread(channel, &root, anchor)).await,
            IntentOutcome::Applied
        );
        assert_eq!(
            row(sidebar(&db, community, &actor).await),
            expected(
                Some(&top),
                false,
                serde_json::json!([thread(1, Some(&reply))])
            )
        );
    }
    let second_actor = Keys::generate();
    db.apply_personal_read_intent(
        community,
        &second_actor.public_key(),
        &mark_thread(channel, &root, &reply),
    )
    .await
    .unwrap();
    let roots: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT root_id FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(second_actor.public_key().to_bytes().as_slice())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        roots,
        vec![root.id.as_bytes().to_vec()],
        "thread reading never creates a channel frontier"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_author_horizon_unresolved_ancestry_and_deletion() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let mention = Tag::public_key(actor.public_key());
    let old = EventBuilder::new(Kind::Custom(9), "directed, about to expire")
        .tags([mention.clone()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &old, Some(channel))
        .await
        .unwrap();
    // Claims to reply, with no recorded ancestry: left out, though directed.
    let unresolved = EventBuilder::new(Kind::Custom(9), "ancestry missing")
        .tags([
            Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap(),
            mention,
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &unresolved, Some(channel))
        .await
        .unwrap();
    let row = &sidebar(&db, community, &actor).await.channels[0];
    assert_eq!((row.unread, row.mentions), (true, 1));
    assert!(row.threads.is_empty());
    sqlx::query(
        "UPDATE events SET created_at=created_at-interval '31 days' WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(old.id.as_bytes().as_slice())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(root.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let row = &sidebar(&db, community, &actor).await.channels[0];
    assert_eq!((row.unread, row.mentions), (false, 0));
}
