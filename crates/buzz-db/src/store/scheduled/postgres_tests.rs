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
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
        .await
        .unwrap();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
        .await
        .unwrap();
    assert_eq!(db.scheduled_list(c, &owner).await.unwrap().len(), 1);
    assert!(db.scheduled_list(c, &[7; 32]).await.unwrap().is_empty());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(2), None, at, None)
        .await
        .is_err());
    db.scheduled_mutate(c, &owner, id, Some(1), None, at, None)
        .await
        .unwrap();
    assert!(db.scheduled_list(c, &owner).await.unwrap().is_empty());
    assert!(db
        .scheduled_mutate(c, &owner, Uuid::new_v4(), None, Some(&value), at, None)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_dispatch_atomically_stores_message_and_sent_state() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp() - 60;
    let event = queued_event(&keys, channel, at);
    let value = serde_json::to_value(&event).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
        .await
        .unwrap();
    let mut claim = db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .unwrap();
    let (stored, inserted) = claim
        .persist(&db, &event, Some(channel), None)
        .await
        .unwrap();
    assert_eq!(stored.event.id, event.id);
    assert!(inserted);
    assert_eq!(
        db.scheduled_list(c, &owner).await.unwrap()[0].status,
        "sent"
    );
    assert!(db
        .scheduled_claim(c, &owner, id, at)
        .await
        .unwrap()
        .is_none());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(1), None, at, None)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_expired_dispatch_rolls_back_a_write_within_the_database_floor() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let now = chrono::Utc::now().timestamp();
    let at = now - RELEASE_WINDOW_SECS - 5;
    let event = queued_event(&keys, channel, at);
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    // Model a message accepted earlier and due during an outage.
    sqlx::query("INSERT INTO scheduled_messages(community_id,owner,id,event,event_id,send_at,next_attempt_at)
                 VALUES($1,$2,$3,$4,$5,$6,$6)")
        .bind(c.as_uuid()).bind(owner.as_slice()).bind(id)
        .bind(serde_json::to_value(&event).unwrap())
        .bind(event.id.to_bytes().as_slice()).bind(at)
        .execute(db.pool()).await.unwrap();
    let mut claim = db
        .scheduled_claim(c, &owner, id, now)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        claim.persist(&db, &event, Some(channel), None).await,
        Err(DbError::InvalidData(ref reason)) if reason == MISSED_DELIVERY
    ));
    assert!(db
        .get_event_by_id(c, &event.id.to_bytes())
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        db.scheduled_list(c, &owner).await.unwrap()[0].status,
        "scheduled"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_claim_rollback_allows_recovery_and_excludes_other_workers() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let at = chrono::Utc::now().timestamp();
    let value = serde_json::to_value(queued_event(&keys, channel, at)).unwrap();
    let id = Uuid::new_v4();
    let owner = keys.public_key().to_bytes();
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
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
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
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
    db.scheduled_mutate(c, &owner, id, Some(1), None, at, None)
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
    db.scheduled_mutate(c, &owner, id, None, Some(&value), at, None)
        .await
        .unwrap();
    crate::test_support::set_deletion_state(db.pool(), *c.as_uuid(), "fenced").await;
    assert!(db.scheduled_claim(c, &owner, id, at).await.is_err());
    assert!(db
        .scheduled_mutate(c, &owner, id, Some(1), None, at, None)
        .await
        .is_err());
    assert!(db
        .scheduled_due(at)
        .await
        .unwrap()
        .iter()
        .all(|candidate| candidate.community != c));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_replay_uses_committed_event_and_revision_after_delivery() {
    let (db, _scratch, c, keys, channel) = fixture().await;
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().timestamp();
    let initial = serde_json::to_value(queued_event(&keys, channel, now + 3600)).unwrap();
    db.scheduled_mutate(c, &owner, id, None, Some(&initial), now + 3600, None)
        .await
        .unwrap();
    assert!(db
        .scheduled_replayed(c, &owner, id, None, &initial)
        .await
        .unwrap());
    assert!(!db
        .scheduled_replayed(c, &[7; 32], id, None, &initial)
        .await
        .unwrap());
    assert!(!db
        .scheduled_replayed(
            CommunityId::from_uuid(Uuid::new_v4()),
            &owner,
            id,
            None,
            &initial
        )
        .await
        .unwrap());
    assert!(!db
        .scheduled_replayed(c, &owner, id, Some(1), &initial)
        .await
        .unwrap());
    let immediate = queued_event(&keys, channel, now - 2);
    let value = serde_json::to_value(&immediate).unwrap();
    db.scheduled_mutate(c, &owner, id, Some(1), Some(&value), now - 2, None)
        .await
        .unwrap();
    assert!(db
        .scheduled_replayed(c, &owner, id, Some(1), &value)
        .await
        .unwrap());
    assert!(!db
        .scheduled_replayed(c, &owner, id, None, &initial)
        .await
        .unwrap());
    assert!(!db
        .scheduled_replayed(c, &owner, id, Some(2), &value)
        .await
        .unwrap());
    let mut claim = db
        .scheduled_claim(c, &owner, id, now)
        .await
        .unwrap()
        .unwrap();
    claim
        .persist(&db, &immediate, Some(channel), None)
        .await
        .unwrap();
    assert!(db
        .scheduled_replayed(c, &owner, id, Some(1), &value)
        .await
        .unwrap());
    let rows = db.scheduled_list(c, &owner).await.unwrap();
    assert_eq!(rows[0].revision, 2);
    assert_eq!(rows[0].status, "sent");
    assert_eq!(rows[0].send_at, now - 2);

    let cancelled = Uuid::new_v4();
    let other = serde_json::to_value(queued_event(&keys, channel, now + 7200)).unwrap();
    db.scheduled_mutate(c, &owner, cancelled, None, Some(&other), now + 7200, None)
        .await
        .unwrap();
    db.scheduled_mutate(c, &owner, cancelled, Some(1), None, now, None)
        .await
        .unwrap();
    assert!(!db
        .scheduled_replayed(c, &owner, cancelled, None, &other)
        .await
        .unwrap());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_defer_stops_at_the_retry_limit() {
    let (db, _scratch, community, keys, channel) = fixture().await;
    let now = chrono::Utc::now().timestamp();
    let owner = keys.public_key().to_bytes();
    let id = Uuid::new_v4();
    let value = serde_json::to_value(queued_event(&keys, channel, now)).unwrap();
    db.scheduled_mutate(community, &owner, id, None, Some(&value), now, None)
        .await
        .unwrap();

    let candidate = db.scheduled_due(now).await.unwrap().remove(0);
    db.scheduled_defer(&candidate, now).await.unwrap();
    // A duplicate sweep must not burn another attempt while the row is backed off.
    db.scheduled_defer(&candidate, now).await.unwrap();
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM scheduled_messages WHERE id=$1")
        .bind(id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(attempts, 1);

    sqlx::query("UPDATE scheduled_messages SET attempts=$2,next_attempt_at=$3 WHERE id=$1")
        .bind(id)
        .bind(MAX_DELIVERY_ATTEMPTS - 1)
        .bind(now + 1)
        .execute(db.pool())
        .await
        .unwrap();

    let claim = db
        .scheduled_claim(community, &owner, id, now + 1)
        .await
        .unwrap()
        .unwrap();
    claim
        .finish("scheduled", Some("Temporary"), now + 1)
        .await
        .unwrap();
    assert!(db.scheduled_due(now + 3600).await.unwrap().is_empty());
    let item = db
        .scheduled_list(community, &owner)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(item.status, "failed");
    assert_eq!(item.error.as_deref(), Some(RETRY_EXHAUSTED));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_retention_keeps_live_rows_and_unexpired_replays() {
    let (db, _scratch, community, keys, channel) = fixture().await;
    let now = chrono::Utc::now().timestamp();
    let owner = keys.public_key().to_bytes();
    let pending = Uuid::new_v4();
    let sent = Uuid::new_v4();
    let cancelled = Uuid::new_v4();
    let future_cancelled = Uuid::new_v4();
    for (id, at) in [
        (pending, now + 1),
        (sent, now),
        (cancelled, now + 2),
        (future_cancelled, now + 86_400 * 90),
    ] {
        let value = serde_json::to_value(queued_event(&keys, channel, at)).unwrap();
        db.scheduled_mutate(community, &owner, id, None, Some(&value), at, None)
            .await
            .unwrap();
    }
    db.scheduled_claim(community, &owner, sent, now)
        .await
        .unwrap()
        .unwrap()
        .finish("sent", None, now)
        .await
        .unwrap();
    for id in [cancelled, future_cancelled] {
        db.scheduled_mutate(community, &owner, id, Some(1), None, now, None)
            .await
            .unwrap();
    }
    db.scheduled_prune_completed().await.unwrap();
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM scheduled_messages")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(retained, 4);

    // Simulate the passage of the retention window for just two rows.
    sqlx::query(
        "UPDATE scheduled_messages SET prune_at=now()-interval '1 second' WHERE id=ANY($1)",
    )
    .bind(&[sent, cancelled][..])
    .execute(db.pool())
    .await
    .unwrap();
    db.scheduled_prune_completed().await.unwrap();
    let retained: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM scheduled_messages")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert_eq!(retained.len(), 2);
    assert!(retained.contains(&pending));
    assert!(retained.contains(&future_cancelled));
    db.scheduled_mutate(
        community,
        &owner,
        future_cancelled,
        Some(1),
        None,
        now,
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scheduled_disconnect_cancels_matching_agent_and_refuses_racing_admission() {
    let (db, _scratch, community, keys, channel) = fixture().await;
    let owner = keys.public_key().to_bytes();
    let principal = Keys::generate().public_key().to_bytes();
    let now = chrono::Utc::now();
    let cutoff = chrono::DateTime::from_timestamp(now.timestamp() - 1, 0).unwrap();
    let issuer = "https://issuer.example";
    let admission = |issuer: &str, issued_at| ScheduleAdmission {
        issuer: issuer.to_owned(),
        owner: Some(principal),
        issued_at,
        admitted_at: now,
    };
    let revoked = admission(issuer, cutoff);
    let newer = admission(issuer, cutoff + chrono::Duration::milliseconds(500));
    let other_issuer = admission("https://other.example", cutoff);
    let at = now.timestamp() + 3600;
    let revoked_id = Uuid::new_v4();
    let newer_id = Uuid::new_v4();
    let other_id = Uuid::new_v4();
    for (offset, id, proof) in [
        (0, revoked_id, &revoked),
        (1, newer_id, &newer),
        (2, other_id, &other_issuer),
    ] {
        let event = serde_json::to_value(queued_event(&keys, channel, at + offset)).unwrap();
        db.scheduled_mutate(
            community,
            &owner,
            id,
            None,
            Some(&event),
            at + offset,
            Some(proof),
        )
        .await
        .unwrap();
    }
    db.scheduled_disconnect(issuer, &principal, Some(cutoff.timestamp()), now, 300)
        .await
        .unwrap();
    let ids: std::collections::BTreeSet<_> = db
        .scheduled_list(community, &owner)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, [newer_id, other_id].into_iter().collect());
    assert!(db
        .scheduled_claim(community, &owner, revoked_id, at)
        .await
        .unwrap()
        .is_none());
    let event = serde_json::to_value(queued_event(&keys, channel, at + 3)).unwrap();
    assert!(matches!(
        db.scheduled_mutate(
            community,
            &owner,
            Uuid::new_v4(),
            None,
            Some(&event),
            at + 3,
            Some(&revoked)
        )
        .await,
        Err(DbError::AccessDenied(_))
    ));
}
