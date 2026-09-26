use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    Db,
};
use buzz_core::CommunityId;
use nostr::{EventBuilder, Keys, Kind};
use sqlx::PgPool;
use uuid::Uuid;

async fn fixture() -> (Db, PgPool, CommunityId, Uuid, Keys, nostr::Event) {
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
    (db, pool, community, channel, actor, event)
}

async fn add_history(db: &Db, community: CommunityId, channel: Uuid) -> nostr::Event {
    let author = Keys::generate();
    let mut last = None;
    let base = nostr::Timestamp::now().as_secs();
    for i in 0..300 {
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
async fn personal_read_sidebar_marked_history_keeps_latest_but_unscanned_threads_are_unknown() {
    let (db, _pool, community, channel, actor, _) = fixture().await;
    let last = add_history(&db, community, channel).await;
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: None,
            },
            message_id: last.id.to_hex(),
        },
    )
    .await
    .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        page.channels[0].unread,
        ReadCount::AtLeast { value: 0 }
    ));
    assert_eq!(
        page.channels[0].latest_message_id.as_deref(),
        Some(last.id.to_hex().as_str())
    );
    assert!(page.channels[0].latest_message_complete);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_expired_unopened_tail_remains_unproved() {
    let (db, pool, community, channel, actor, _) = fixture().await;
    let latest = add_history(&db, community, channel).await;
    sqlx::query(
        "UPDATE events SET received_at=clock_timestamp()-interval '31 days' WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(page.channels[0].unread, ReadCount::AtLeast { value: 0 }),
        "receipt expiry of the scanned window cannot prove expiry of the author-ordered tail"
    );
    assert_eq!(
        page.channels[0].latest_message_id.as_deref(),
        Some(latest.id.to_hex().as_str())
    );
    assert!(page.channels[0].latest_message_complete);
    // A fresh receipt with old author time belongs to retention even if it is
    // invisible below this bounded window. Never certify that channel as read.
    let backfill = EventBuilder::new(Kind::Custom(9), "recently imported old message")
        .custom_created_at(nostr::Timestamp::from(
            nostr::Timestamp::now().as_secs() - 40 * 86400,
        ))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &backfill, Some(channel))
        .await
        .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        page.channels[0].unread,
        ReadCount::AtLeast { value: 0 }
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_sidebar_is_read_only_and_does_not_wait_for_account() {
    let (db, pool, community, _channel, actor, _) = fixture().await;
    db.personal_read_sidebar(
        community,
        &actor.public_key(),
        DEFAULT_RETENTION_SECONDS,
        20,
        None,
    )
    .await
    .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personal_read_accounts WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "GET must not create private state");
    db.apply_personal_read_intent(community, &actor.public_key(), &ReadIntent::CompleteImport)
        .await
        .unwrap();
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT actor FROM personal_read_accounts WHERE community_id=$1 FOR UPDATE")
        .bind(community.as_uuid())
        .fetch_all(&mut *held)
        .await
        .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert!(page.account.imported_at_ms.is_some());
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
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        page.channels[0].latest_message_id.as_deref(),
        Some(own.id.to_hex().as_str())
    );
    assert!(page.channels[0].latest_message_complete);
    sqlx::query(
        "UPDATE events SET received_at=clock_timestamp()-interval '31 days' WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        page.channels[0].latest_message_id.as_deref(),
        Some(own.id.to_hex().as_str())
    );
    assert!(page.channels[0].latest_message_complete);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_latest_old_message_is_not_an_empty_channel() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    sqlx::query(
        "UPDATE events SET received_at=clock_timestamp()-interval '31 days' WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
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
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    let old = page
        .channels
        .iter()
        .find(|c| c.channel_id == channel)
        .unwrap();
    assert_eq!(
        old.latest_message_id.as_deref(),
        Some(event.id.to_hex().as_str())
    );
    assert!(old.latest_message_complete);
    assert!(matches!(old.unread, ReadCount::Exact { value: 0 }));
    let empty = page
        .channels
        .iter()
        .find(|c| c.channel_id == empty)
        .unwrap();
    assert!(empty.latest_message_id.is_none());
    assert!(empty.latest_message_complete);
    // A long run of ineligible rows must instead report that latest is unknown.
    sqlx::query("UPDATE events SET kind=7 WHERE community_id=$1")
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    add_history(&db, community, channel).await;
    sqlx::query("UPDATE events SET kind=7 WHERE community_id=$1")
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    let unknown = page
        .channels
        .iter()
        .find(|c| c.channel_id == channel)
        .unwrap();
    assert!(unknown.latest_message_id.is_none());
    assert!(!unknown.latest_message_complete);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_frontier_retries_old_anchors_and_rejects_future_imports() {
    let (db, pool, community, channel, actor, event) = fixture().await;
    let target = ReadTarget {
        channel_id: channel,
        root_id: None,
    };
    let invalid = ReadIntent::LegacyPrefix {
        target: target.clone(),
        through_timestamp: i64::MAX,
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
    sqlx::query(
        "UPDATE events SET received_at=clock_timestamp()-interval '60 days' WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let intent = ReadIntent::MarkThrough {
        target: target.clone(),
        message_id: event.id.to_hex(),
    };
    for _ in 0..2 {
        assert_eq!(
            db.apply_personal_read_intent(community, &actor.public_key(), &intent)
                .await
                .unwrap(),
            IntentOutcome::Applied
        );
    }
    assert_eq!(
        db.apply_personal_read_intent(
            community,
            &actor.public_key(),
            &ReadIntent::LegacyPrefix {
                target,
                through_timestamp: 0
            }
        )
        .await
        .unwrap(),
        IntentOutcome::Applied
    );
    let frontier: i64 = sqlx::query_scalar(
        "SELECT through_timestamp FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2",
    )
    .bind(community.as_uuid())
    .bind(actor.public_key().to_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(frontier, event.created_at.as_secs() as i64);
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_channel_and_thread_never_inherit_each_other() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    let reply = EventBuilder::new(Kind::Custom(9), "unseen thread reply")
        .custom_created_at(nostr::Timestamp::from(base + 10))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &reply, Some(channel))
        .await
        .unwrap();
    sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
        VALUES ($1,$2,to_timestamp($3),$4,$5,$5,1)")
        .bind(community.as_uuid()).bind(reply.id.as_bytes().as_slice()).bind((base+10) as f64)
        .bind(channel).bind(root.id.as_bytes().as_slice()).execute(&pool).await.unwrap();
    let top = EventBuilder::new(Kind::Custom(9), "later timeline message")
        .custom_created_at(nostr::Timestamp::from(base + 20))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &top, Some(channel))
        .await
        .unwrap();
    let channel_target = ReadTarget {
        channel_id: channel,
        root_id: None,
    };
    assert_eq!(
        db.apply_personal_read_intent(
            community,
            &actor.public_key(),
            &ReadIntent::MarkThrough {
                target: channel_target.clone(),
                message_id: reply.id.to_hex()
            }
        )
        .await
        .unwrap(),
        IntentOutcome::Blocked
    );
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &ReadIntent::MarkThrough {
            target: channel_target,
            message_id: top.id.to_hex(),
        },
    )
    .await
    .unwrap();
    let page = db
        .personal_read_sidebar(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            20,
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(page.channels[0].unread, ReadCount::Exact { value: 1 }),
        "timeline reading must leave unseen reply unread"
    );
    let second_actor = Keys::generate();
    db.apply_personal_read_intent(
        community,
        &second_actor.public_key(),
        &ReadIntent::MarkThrough {
            target: ReadTarget {
                channel_id: channel,
                root_id: Some(root.id.to_hex()),
            },
            message_id: reply.id.to_hex(),
        },
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
async fn personal_read_contexts_bound_selectors_and_use_only_matching_frontiers() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let base = root.created_at.as_secs();
    let reply = EventBuilder::new(Kind::Custom(9), "thread only")
        .custom_created_at(nostr::Timestamp::from(base + 1))
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &reply, Some(channel))
        .await
        .unwrap();
    sqlx::query("INSERT INTO thread_metadata (community_id,event_id,event_created_at,channel_id,root_event_id,parent_event_id,depth)
        VALUES ($1,$2,to_timestamp($3),$4,$5,$5,1)")
        .bind(community.as_uuid()).bind(reply.id.as_bytes().as_slice()).bind((base+1) as f64)
        .bind(channel).bind(root.id.as_bytes().as_slice()).execute(&pool).await.unwrap();
    let queries = vec![
        ContextQuery {
            target: ReadTarget {
                channel_id: channel,
                root_id: None,
            },
            message_ids: vec![root.id.to_hex(), reply.id.to_hex(), "00".repeat(32)],
        },
        ContextQuery {
            target: ReadTarget {
                channel_id: channel,
                root_id: Some(root.id.to_hex()),
            },
            message_ids: vec![root.id.to_hex(), reply.id.to_hex()],
        },
    ];
    let page = db
        .personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap();
    let page = serde_json::to_value(page).unwrap();
    assert_eq!(page["contexts"][0]["messages"][0]["status"], "unread");
    assert_eq!(page["contexts"][0]["messages"][1]["status"], "unavailable");
    assert_eq!(page["contexts"][0]["messages"][2]["status"], "unavailable");
    assert_eq!(page["contexts"][1]["messages"][0]["status"], "unavailable");
    assert_eq!(page["contexts"][1]["messages"][1]["status"], "unread");
    assert!(page["contexts"][1]["messages"][1]["attention"].is_null());
    let accounts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM personal_read_accounts WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(accounts, 0, "context GET must never create authority");
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &ReadIntent::LegacyPrefix {
            target: queries[0].target.clone(),
            through_timestamp: (base + 10) as i64,
        },
    )
    .await
    .unwrap();
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["contexts"][0]["messages"][0]["status"], "read");
    assert_eq!(page["contexts"][1]["messages"][1]["status"], "unread");
    db.apply_personal_read_intent(
        community,
        &actor.public_key(),
        &ReadIntent::MarkThrough {
            target: queries[1].target.clone(),
            message_id: reply.id.to_hex(),
        },
    )
    .await
    .unwrap();
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["contexts"][1]["messages"][1]["status"], "read");
    let other = Keys::generate();
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &other.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["contexts"][0]["messages"][0]["status"], "unread");
    assert_eq!(page["contexts"][1]["messages"][1]["status"], "unread");
    assert!(db
        .personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &[]
        )
        .await
        .is_err());
    let oversized = vec![ContextQuery {
        target: queries[0].target.clone(),
        message_ids: vec![root.id.to_hex(); MAX_CONTEXT_MESSAGES + 1],
    }];
    assert!(db
        .personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &oversized
        )
        .await
        .is_err());
    sqlx::query("UPDATE channels SET visibility='private' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(channel)
        .execute(&pool)
        .await
        .unwrap();
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &other.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        page["contexts"],
        serde_json::json!([{"status":"unavailable"},{"status":"unavailable"}])
    );
    assert!(db
        .personal_read_accessible_contexts(community, &other.public_key(), &[channel])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_read_context_receipt_horizon_unknown_ancestry_and_deletion() {
    let (db, pool, community, channel, actor, root) = fixture().await;
    let old = EventBuilder::new(Kind::Custom(9), "old author freshly received")
        .custom_created_at(nostr::Timestamp::from(
            root.created_at.as_secs() - 40 * 86400,
        ))
        .tags([nostr::Tag::parse(["p", &actor.public_key().to_hex()]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &old, Some(channel))
        .await
        .unwrap();
    let unresolved = EventBuilder::new(Kind::Custom(9), "ancestry missing")
        .tags([nostr::Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    db.insert_event(community, &unresolved, Some(channel))
        .await
        .unwrap();
    let queries = [ContextQuery {
        target: ReadTarget {
            channel_id: channel,
            root_id: None,
        },
        message_ids: vec![old.id.to_hex(), unresolved.id.to_hex(), root.id.to_hex()],
    }];
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["contexts"][0]["messages"][0]["status"], "unread");
    assert_eq!(page["contexts"][0]["messages"][0]["attention"], true);
    assert_eq!(page["contexts"][0]["messages"][1]["status"], "unknown");
    sqlx::query(
        "UPDATE events SET received_at=now()-interval '31 days' WHERE community_id=$1 AND id=$2",
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
    let page = serde_json::to_value(
        db.personal_read_contexts(
            community,
            &actor.public_key(),
            DEFAULT_RETENTION_SECONDS,
            &queries,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(page["contexts"][0]["messages"][0]["status"], "not_counted");
    assert_eq!(page["contexts"][0]["messages"][2]["status"], "not_counted");
}
