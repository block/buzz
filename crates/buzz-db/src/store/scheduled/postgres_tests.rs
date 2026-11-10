use super::*;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};

async fn fixture() -> (Db, crate::test_support::ScratchDb, CommunityId, Keys, Uuid) {
    let (pool, scratch) =
        crate::test_support::ScratchDb::migrated(&crate::test_support::database_url(), "scheduled")
            .await;
    let db = Db::from_pool(pool);
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("scheduled-{}.example", community.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    let keys = Keys::generate();
    let channel = db
        .create_channel(
            community,
            "scheduled",
            buzz_core::channel::ChannelType::Stream,
            buzz_core::channel::ChannelVisibility::Open,
            None,
            &keys.public_key().to_bytes(),
            None,
        )
        .await
        .unwrap();
    (db, scratch, community, keys, channel.id)
}
fn queued_event(keys: &Keys, channel: Uuid, at: i64) -> Event {
    EventBuilder::new(Kind::Custom(9), "A scheduled message")
        .tags([Tag::parse(["h".to_string(), channel.to_string()]).unwrap()])
        .custom_created_at(Timestamp::from(at as u64))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_owner_isolation_revision_and_cancelled_duplicate() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp() + 3600;
    let value = serde_json::to_value(queued_event(&keys, channel, at)).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .unwrap();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .unwrap();
    assert_eq!(db.scheduled_list(c, &owner).await.unwrap().len(), 1);
    assert!(db.scheduled_list(c, &[7; 32]).await.unwrap().is_empty());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(2), None, at)
        .await
        .is_err());
    db.scheduled_mutate(c, &owner, id, Some(1), None, at)
        .await
        .unwrap();
    assert!(db.scheduled_list(c, &owner).await.unwrap().is_empty());
    assert!(db
        .scheduled_mutate(c, &owner, Uuid::new_v4(), None, Some(&value), at)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_overdue_dispatch_atomically_stores_message_and_sent_state() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp() - 7200;
    let event = queued_event(&keys, channel, at);
    let value = serde_json::to_value(&event).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    assert!(db
        .scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .is_err());
    let future = chrono::Utc::now().timestamp() + 3600;
    let initial = serde_json::to_value(queued_event(&keys, channel, future)).unwrap();
    db.scheduled_mutate(c, &owner, id, None, Some(&initial), future)
        .await
        .unwrap();
    // Simulate elapsed downtime without waiting hours. Only the fixture may
    // bypass admission; production cannot introduce a retroactive reservation.
    sqlx::query("UPDATE scheduled_messages SET event=$2,event_id=$3,send_at=$4,next_attempt_at=$4 WHERE id=$1")
        .bind(id).bind(&value).bind(event.id.to_bytes().as_slice()).bind(at).execute(db.pool()).await.unwrap();
    let fence = crate::replica_fence::ReplicaFence::new();
    let reserved = crate::replica_fence::probe_once(db.pool(), &fence)
        .await
        .unwrap();
    assert_eq!(reserved.fence_wall.timestamp(), at - 301);
    assert!(!fence.covers(chrono::DateTime::from_timestamp(at, 0).unwrap()));
    let mut claim = db
        .scheduled_claim(c, &owner, id, chrono::Utc::now().timestamp())
        .await
        .unwrap()
        .unwrap();
    sqlx::query("SET LOCAL buzz.created_at_floor='960'")
        .execute(claim.tx.as_mut().unwrap().conn())
        .await
        .unwrap();
    let (stored, inserted) = claim
        .persist(&db, &event, Some(channel), None)
        .await
        .unwrap();
    assert_eq!(stored.event.id, event.id);
    assert!(inserted);
    let rows = db.scheduled_list(c, &owner).await.unwrap();
    assert_eq!(rows[0].status, "sent");
    let released = crate::replica_fence::probe_once(db.pool(), &fence)
        .await
        .unwrap();
    assert!(released.token > reserved.token);
    assert!(released.fence_wall.timestamp() > at);
    assert!(db
        .get_event_by_id(c, &event.id.to_bytes())
        .await
        .unwrap()
        .is_some());
    assert!(db
        .scheduled_claim(c, &owner, id, chrono::Utc::now().timestamp())
        .await
        .unwrap()
        .is_none());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(1), None, at)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_claim_rollback_allows_recovery_and_excludes_other_workers() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp();
    let value = serde_json::to_value(queued_event(&keys, channel, at)).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .unwrap();
    let claim = db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .unwrap();
    assert!(db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .is_none());
    drop(claim);
    let claim = db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .unwrap();
    claim
        .finish("scheduled", Some("transient failure"), at)
        .await
        .unwrap();
    assert!(db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .is_none());
    let mut recovered = db
        .scheduled_claim(c, &owner, id, at + 10)
        .await
        .unwrap()
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(recovered.tx.as_mut().unwrap().conn())
        .await
        .unwrap();
    sqlx::query("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .execute(db.pool())
        .await
        .unwrap();
    let event: Event = serde_json::from_value(value.clone()).unwrap();
    assert!(recovered
        .persist(&db, &event, Some(channel), None)
        .await
        .is_err());
    recovered
        .finish("scheduled", Some("Persistence interrupted"), at + 10)
        .await
        .unwrap();
    let (attempts, retry): (i32, i64) =
        sqlx::query_as("SELECT attempts,next_attempt_at FROM scheduled_messages WHERE id=$1")
            .bind(id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(attempts, 2);
    assert_eq!(retry, at + 12);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_lost_claim_cannot_publish_after_successful_cancellation() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp();
    let event = queued_event(&keys, channel, at);
    let value = serde_json::to_value(&event).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .unwrap();
    let mut claim = db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(claim.tx.as_mut().unwrap().conn())
        .await
        .unwrap();
    sqlx::query("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .execute(db.pool())
        .await
        .unwrap();
    db.scheduled_mutate(c, &owner, id, Some(1), None, at)
        .await
        .unwrap();
    assert!(claim
        .persist(&db, &event, Some(channel), None)
        .await
        .is_err());
    assert!(db
        .get_event_by_id(c, &event.id.to_bytes())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_writers_and_claims_honor_community_deletion_admission() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp();
    let value = serde_json::to_value(queued_event(&keys, channel, at)).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at)
        .await
        .unwrap();
    crate::test_support::set_deletion_state(db.pool(), *c.as_uuid(), "fenced").await;
    assert!(db.scheduled_claim(c, &owner, id, at).await.is_err());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(1), None, at)
        .await
        .is_err());
    assert!(db
        .scheduled_due(at)
        .await
        .unwrap()
        .iter()
        .all(|(community, _, _, _)| *community != c));
}
