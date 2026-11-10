use super::queue::MAX_QUEUED_ROWS;
use super::*;
use crate::relay_members::{assert_membership_list_matches, is_relay_member};
use std::time::Duration;

fn request() -> ClaimRequest {
    ClaimRequest {
        pubkey: nostr::Keys::generate().public_key().to_hex(),
        policy_version: None,
        verified_owner: None,
        credential: ClaimCredential::Legacy {
            role: "member".into(),
        },
    }
}

async fn fixture() -> (Db, CommunityId, nostr::Keys) {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    (Db::from_pool(pool), community, nostr::Keys::generate())
}

async fn enqueue(db: &Db, community: CommunityId, request: &ClaimRequest) -> Uuid {
    let EnqueuedClaim::Queued(id) = db.enqueue_invite_claim(community, request).await.unwrap()
    else {
        panic!("claim must enqueue");
    };
    id
}

async fn finish(db: &Db, community: CommunityId) -> ProcessedInviteBatch {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(batch) = db.try_finish_invite_claims(community).await.unwrap() {
                break batch;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("live committed claims become finishable")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn one_open_request_commits_without_waiting_for_a_full_batch() {
    let (db, community, keys) = fixture().await;
    let claim = request();
    let id = enqueue(&db, community, &claim).await;
    assert!(db
        .invite_claim_results(community, &[id])
        .await
        .unwrap()
        .is_empty());
    let write = tokio::time::timeout(
        Duration::from_secs(2),
        db.try_admit_invite_claims(community, &keys),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(write.outcome.len(), 1);
    db.deletion_store().validate_catalog().await.unwrap();
    let external_guards: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM pg_constraint WHERE conrelid='relay_invite_claims'::regclass AND contype='f') + (SELECT count(*) FROM pg_trigger WHERE tgrelid='relay_invite_claims'::regclass AND NOT tgisinternal)")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        external_guards, 0,
        "new queue is application-admitted without triggers or foreign keys"
    );
    assert_membership_list_matches(&db.pool, community, &keys, write.snapshot.as_ref()).await;
    assert!(db
        .invite_claim_results(community, &[id])
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .pending_default_channel_join(community, &claim.pubkey)
        .await
        .unwrap()
        .is_some());
    let batch = finish(&db, community).await;
    assert!(
        db.try_finish_invite_claims(community)
            .await
            .unwrap()
            .is_none(),
        "only one finisher owns the committed work"
    );
    batch.complete().await.unwrap();

    let results = db
        .invite_claim_results(community, &[id, Uuid::new_v4()])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(
        matches!(&results[0].result, StoredClaimResult::Success(ClaimResult::Legacy(result)) if result.inserted)
    );
    let other = postgres_tests::make_test_community(&db.pool).await;
    assert!(db
        .invite_claim_results(other, &[id])
        .await
        .unwrap()
        .is_empty());
    assert!(db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn two_processors_share_global_order_and_commit_each_request_once() {
    let (db, community, keys) = fixture().await;
    let request_count = MAX_CLAIM_BATCH_SIZE + 1;
    let mut ids = Vec::new();
    for _ in 0..request_count {
        ids.push(enqueue(&db, community, &request()).await);
    }
    let (first, second) = tokio::join!(
        db.try_admit_invite_claims(community, &keys),
        db.try_admit_invite_claims(community, &keys)
    );
    let mut snapshots = Vec::new();
    let mut processed = 0;
    for write in [first.unwrap(), second.unwrap()].into_iter().flatten() {
        processed += write.outcome.len();
        snapshots.extend(write.snapshot);
    }
    if processed < request_count {
        let write = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(write) = db.try_admit_invite_claims(community, &keys).await.unwrap() {
                    break write;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        processed += write.outcome.len();
        snapshots.extend(write.snapshot);
    }
    assert_eq!(processed, request_count);
    let first_batch = finish(&db, community).await;
    assert_eq!(
        first_batch
            .claims
            .iter()
            .map(|claim| claim.id)
            .collect::<Vec<_>>(),
        ids[..MAX_CLAIM_BATCH_SIZE]
    );
    first_batch.complete().await.unwrap();
    let last_batch = finish(&db, community).await;
    assert_eq!(last_batch.claims.len(), 1);
    assert_eq!(last_batch.claims[0].id, ids[MAX_CLAIM_BATCH_SIZE]);
    last_batch.complete().await.unwrap();
    assert_eq!(snapshots.len(), 2);
    snapshots.sort_by_key(|snapshot| snapshot.event.created_at);
    assert_membership_list_matches(&db.pool, community, &keys, snapshots.last()).await;
    assert_eq!(
        db.invite_claim_results(community, &ids)
            .await
            .unwrap()
            .len(),
        request_count
    );
    let in_order: bool = sqlx::query_scalar(
        "SELECT max(completed_at) FILTER (WHERE id=ANY($2)) < max(completed_at) FILTER (WHERE id=$3) FROM relay_invite_claims WHERE community_id=$1",
    ).bind(community.as_uuid()).bind(&ids[..MAX_CLAIM_BATCH_SIZE]).bind(ids[MAX_CLAIM_BATCH_SIZE]).fetch_one(&db.pool).await.unwrap();
    assert!(
        in_order,
        "the first transaction takes the oldest full batch globally"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unknown_code_bypasses_queue_lock_and_restricted_head_does_not_poison_peer() {
    let (db, community, keys) = fixture().await;
    let mut blocker = db.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('invite-queue:' || $1::text, 0))")
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let invalid = ClaimRequest {
        credential: ClaimCredential::V2 {
            token_hash: [7; 32],
        },
        ..request()
    };
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(1),
            db.enqueue_invite_claim(community, &invalid)
        )
        .await
        .unwrap()
        .unwrap(),
        EnqueuedClaim::Invalid
    );
    blocker.rollback().await.unwrap();
    let blocked = request();
    let denied = enqueue(&db, community, &blocked).await;
    sqlx::query(
        "INSERT INTO community_bans(community_id,pubkey,banned,actor_pubkey) VALUES($1,$2,true,$2)",
    )
    .bind(community.as_uuid())
    .bind(hex::decode(&blocked.pubkey).unwrap())
    .execute(&db.pool)
    .await
    .unwrap();
    let allowed = request();
    let admitted = enqueue(&db, community, &allowed).await;
    let write = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.outcome.len(), 2);
    finish(&db, community).await.complete().await.unwrap();
    let results = db
        .invite_claim_results(community, &[denied, admitted])
        .await
        .unwrap();
    assert!(results
        .iter()
        .any(|r| r.id == denied && matches!(r.result, StoredClaimResult::Restricted)));
    assert!(results
        .iter()
        .any(|r| r.id == admitted && matches!(r.result, StoredClaimResult::Success(_))));
    assert!(!is_relay_member(&db.pool, community, &blocked.pubkey)
        .await
        .unwrap());
    assert!(is_relay_member(&db.pool, community, &allowed.pubkey)
        .await
        .unwrap());
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE community_id=$1")
        .bind(community.as_uuid())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn shared_final_slot_and_duplicate_identity_preserve_durable_outcomes() {
    let (db, community, keys) = fixture().await;
    let invite = mint_relay_invite(&db.pool, community, &request().pubkey, 3600, Some(1))
        .await
        .unwrap();
    let first = ClaimRequest {
        credential: ClaimCredential::V2 {
            token_hash: hash_v2_code(&invite.code),
        },
        ..request()
    };
    let second = ClaimRequest {
        pubkey: request().pubkey,
        ..first.clone()
    };
    let ids = [
        enqueue(&db, community, &first).await,
        enqueue(&db, community, &first).await,
        enqueue(&db, community, &second).await,
    ];
    let payload: String = sqlx::query_scalar(
        "SELECT request::text FROM relay_invite_claims WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(ids[0])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(
        !payload.contains(&invite.code),
        "the reusable bearer is never queued"
    );
    let committed = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert!(!committed.outcome.is_empty());
    finish(&db, community).await.complete().await.unwrap();
    let results = db.invite_claim_results(community, &ids).await.unwrap();
    for (id, expected) in [
        (ids[0], "joined"),
        (ids[1], "already"),
        (ids[2], "exhausted"),
    ] {
        let StoredClaimResult::Success(ClaimResult::V2(result)) =
            &results.iter().find(|r| r.id == id).unwrap().result
        else {
            panic!("v2 result");
        };
        assert!(matches!(
            (&result.outcome, expected),
            (ClaimOutcome::Joined { .. }, "joined")
                | (ClaimOutcome::AlreadyMember { .. }, "already")
                | (ClaimOutcome::Exhausted, "exhausted")
        ));
    }
    assert_eq!(
        postgres_tests::use_count(&db.pool, community, invite.invite_id).await,
        1
    );
    assert_eq!(
        serde_json::to_value(&results).unwrap(),
        serde_json::to_value(db.invite_claim_results(community, &ids).await.unwrap()).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn storage_failure_rolls_back_claims_and_queue_results_together() {
    let (db, community, keys) = fixture().await;
    let first = request();
    let bad = ClaimRequest {
        policy_version: Some("bad".into()),
        ..request()
    };
    let ids = [
        enqueue(&db, community, &first).await,
        enqueue(&db, community, &bad).await,
    ];
    assert!(db.try_admit_invite_claims(community, &keys).await.is_err());
    assert!(db
        .invite_claim_results(community, &ids)
        .await
        .unwrap()
        .is_empty());
    assert!(!is_relay_member(&db.pool, community, &first.pubkey)
        .await
        .unwrap());
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_invite_claims WHERE community_id=$1 AND result IS NOT NULL",
    )
    .bind(community.as_uuid())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stored, 0);
    // The same durable requests remain retryable after the storage defect is fixed.
    sqlx::query("UPDATE relay_invite_claims SET request=jsonb_set(request,'{policy_version}','null'::jsonb) WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(ids[1]).execute(&db.pool).await.unwrap();
    let write = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.outcome.len(), 2);
    finish(&db, community).await.complete().await.unwrap();
    assert_eq!(
        db.invite_claim_results(community, &ids)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn earliest_queue_deadline_rolls_back_a_blocked_snapshot_and_expires_result() {
    let (db, community, keys) = fixture().await;
    let claim = request();
    let id = enqueue(&db, community, &claim).await;
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()+interval '150 milliseconds' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(id).execute(&db.pool).await.unwrap();
    let mut blocker = db.pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE events IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    assert!(tokio::time::timeout(
        Duration::from_secs(2),
        db.try_admit_invite_claims(community, &keys)
    )
    .await
    .unwrap()
    .unwrap()
    .is_none());
    blocker.rollback().await.unwrap();
    assert!(!is_relay_member(&db.pool, community, &claim.pubkey)
        .await
        .unwrap());
    let results = db.invite_claim_results(community, &[id]).await.unwrap();
    assert!(matches!(results[0].result, StoredClaimResult::Expired));
    let stored: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT result FROM relay_invite_claims WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn strict_row_cap_serializes_enqueues_and_cleanup_is_bounded() {
    let (db, community, _) = fixture().await;
    let payload = serde_json::to_value(request()).unwrap();
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO relay_invite_claims(community_id,request) SELECT $1,$2 FROM generate_series(1,2047)")
        .bind(community.as_uuid()).bind(payload).execute(tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
    let first = request();
    let second = request();
    let (a, b) = tokio::join!(
        db.enqueue_invite_claim(community, &first),
        db.enqueue_invite_claim(community, &second)
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, EnqueuedClaim::Queued(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r, EnqueuedClaim::Full))
            .count(),
        1
    );
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()-interval '1 second' WHERE community_id=$1")
        .bind(community.as_uuid()).execute(&db.pool).await.unwrap();
    enqueue(&db, community, &request()).await;
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_invite_claims WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 2048 - 64 + 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn completion_retention_and_expired_pending_cleanup_are_separate() {
    let (db, community, keys) = fixture().await;
    let completed = enqueue(&db, community, &request()).await;
    let committed = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert!(!committed.outcome.is_empty());
    finish(&db, community).await.complete().await.unwrap();
    let pending = enqueue(&db, community, &request()).await;
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()-interval '1 second' WHERE community_id=$1")
        .bind(community.as_uuid()).execute(&db.pool).await.unwrap();
    db.reap_expired_invite_claims().await.unwrap();
    let results = db
        .invite_claim_results(community, &[completed, pending])
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, completed);
    assert!(matches!(results[0].result, StoredClaimResult::Success(_)));
    sqlx::query("UPDATE relay_invite_claims SET completed_at=clock_timestamp()-interval '61 seconds' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(completed).execute(&db.pool).await.unwrap();
    db.reap_expired_invite_claims().await.unwrap();
    assert!(db
        .invite_claim_results(community, &[completed])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn admission_progresses_during_followup_and_dropped_finisher_recovers_without_reapplying() {
    let (db, community, keys) = fixture().await;
    let invite = mint_relay_invite(&db.pool, community, &request().pubkey, 3600, Some(1))
        .await
        .unwrap();
    let claim = ClaimRequest {
        credential: ClaimCredential::V2 {
            token_hash: hash_v2_code(&invite.code),
        },
        verified_owner: Some(request().pubkey),
        ..request()
    };
    let id = enqueue(&db, community, &claim).await;
    let admitted = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        admitted.outcome[0].pubkey.as_slice(),
        hex::decode(&claim.pubkey).unwrap()
    );
    assert!(admitted.snapshot.is_some());
    let first = finish(&db, community).await;
    let original_result = serde_json::to_value(&first.claims[0].result).unwrap();
    let pending = db
        .pending_default_channel_join(community, &claim.pubkey)
        .await
        .unwrap()
        .unwrap();
    let next_claim = request();
    let next_id = enqueue(&db, community, &next_claim).await;
    let next = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        next.outcome[0].id, next_id,
        "admission progresses while the first finisher owns its batch"
    );
    assert!(is_relay_member(&db.pool, community, &next_claim.pubkey)
        .await
        .unwrap());
    assert!(
        db.try_finish_invite_claims(community)
            .await
            .unwrap()
            .is_none(),
        "another finisher cannot overlap the current owner"
    );
    assert!(db
        .invite_claim_results(community, &[id, next_id])
        .await
        .unwrap()
        .is_empty());
    drop(first);
    let recovered = finish(&db, community).await;
    assert_eq!(
        recovered
            .claims
            .iter()
            .map(|claim| claim.id)
            .collect::<Vec<_>>(),
        vec![id, next_id],
        "the successor coalesces all admitted work in queue order"
    );
    assert_eq!(
        serde_json::to_value(&recovered.claims[0].result).unwrap(),
        original_result
    );
    assert!(matches!(&recovered.claims[0].result,
        StoredClaimResult::Success(ClaimResult::V2(result)) if result.users_created==2 && result.owner_link_created && matches!(result.outcome,ClaimOutcome::Joined{use_count:1,..})));
    assert_eq!(
        postgres_tests::use_count(&db.pool, community, invite.invite_id).await,
        1
    );
    assert_eq!(
        db.pending_default_channel_join(community, &claim.pubkey)
            .await
            .unwrap()
            .unwrap()
            .token,
        pending.token
    );
    assert!(
        db.try_admit_invite_claims(community, &keys)
            .await
            .unwrap()
            .is_none(),
        "recovery does not reapply either membership transaction"
    );
    recovered.complete().await.unwrap();
    assert_eq!(
        db.invite_claim_results(community, &[id, next_id])
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expired_unready_receipt_survives_for_recovery_and_never_becomes_a_fake_denial() {
    let (db, community, keys) = fixture().await;
    let claim = request();
    let id = enqueue(&db, community, &claim).await;
    let write = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    assert!(db
        .invite_claim_results(community, &[id])
        .await
        .unwrap()
        .is_empty());
    drop(write);
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()-interval '1 second' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(id).execute(&db.pool).await.unwrap();
    db.reap_expired_invite_claims().await.unwrap();
    let results = db.invite_claim_results(community, &[id]).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(
        matches!(&results[0].result,StoredClaimResult::Success(ClaimResult::Legacy(result)) if result.inserted)
    );
    assert!(db
        .pending_default_channel_join(community, &claim.pubkey)
        .await
        .unwrap()
        .is_some());
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()-interval '61 seconds' WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid()).bind(id).execute(&db.pool).await.unwrap();
    db.reap_expired_invite_claims().await.unwrap();
    assert!(db
        .invite_claim_results(community, &[id])
        .await
        .unwrap()
        .is_empty());
    assert!(is_relay_member(&db.pool, community, &claim.pubkey)
        .await
        .unwrap());
    assert!(
        db.pending_default_channel_join(community, &claim.pubkey)
            .await
            .unwrap()
            .is_some(),
        "queue cleanup must not erase durable channel recovery work"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn consumed_denial_receipts_release_capacity_without_discarding_unconsumed_receipts() {
    let (db, community, _) = fixture().await;
    let capacity = MAX_QUEUED_ROWS as usize;
    let mut tx = crate::begin_community_write_transaction(
        &db.pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "INSERT INTO relay_invite_claims(community_id,request,result,completed_at) \
         SELECT $1,$2,$3,clock_timestamp() FROM generate_series(1,$4) RETURNING id",
    )
    .bind(community.as_uuid())
    .bind(serde_json::to_value(request()).unwrap())
    .bind(
        serde_json::to_value(StoredClaimResult::Success(ClaimResult::V2(
            ClaimOutcome::Revoked.into(),
        )))
        .unwrap(),
    )
    .bind(MAX_QUEUED_ROWS)
    .fetch_all(tx.conn())
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        db.enqueue_invite_claim(community, &request())
            .await
            .unwrap(),
        EnqueuedClaim::Full
    );
    let consumed = db
        .invite_claim_results(community, &ids[..capacity - 1])
        .await
        .unwrap();
    assert_eq!(consumed.len(), capacity - 1);
    assert!(consumed.iter().all(|claim| matches!(&claim.result,
        StoredClaimResult::Success(ClaimResult::V2(result)) if matches!(result.outcome, ClaimOutcome::Revoked))));
    let consumed_ids: Vec<_> = consumed.iter().map(|claim| claim.id).collect();
    assert_eq!(
        db.acknowledge_invite_claim_results(community, &consumed_ids)
            .await
            .unwrap(),
        (capacity - 1) as u64
    );
    assert!(matches!(
        db.enqueue_invite_claim(community, &request())
            .await
            .unwrap(),
        EnqueuedClaim::Queued(_)
    ));
    assert_eq!(
        db.invite_claim_results(community, &ids)
            .await
            .unwrap()
            .len(),
        1,
        "the unconsumed receipt remains available for node recovery"
    );
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_invite_claims WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        remaining, 2,
        "only the unconsumed receipt and new pending request remain"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn acknowledgment_only_removes_readable_receipts_in_its_community() {
    let (db, community, keys) = fixture().await;
    let ready = enqueue(&db, community, &request()).await;
    let _ready_admission = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    finish(&db, community).await.complete().await.unwrap();
    let unready = enqueue(&db, community, &request()).await;
    let expired_admitted = enqueue(&db, community, &request()).await;
    let _unready_admission = db
        .try_admit_invite_claims(community, &keys)
        .await
        .unwrap()
        .unwrap();
    let pending = enqueue(&db, community, &request()).await;
    let expired_pending = enqueue(&db, community, &request()).await;
    sqlx::query("UPDATE relay_invite_claims SET expires_at=clock_timestamp()-interval '1 second' WHERE community_id=$1 AND id=ANY($2)")
        .bind(community.as_uuid()).bind([expired_admitted, expired_pending].as_slice())
        .execute(&db.pool).await.unwrap();
    let all = [ready, unready, pending, expired_admitted, expired_pending];
    let other = postgres_tests::make_test_community(&db.pool).await;
    assert_eq!(
        db.acknowledge_invite_claim_results(other, &all)
            .await
            .unwrap(),
        0
    );
    let readable = db.invite_claim_results(community, &all).await.unwrap();
    assert_eq!(readable.len(), 3);
    assert_eq!(
        db.acknowledge_invite_claim_results(community, &all)
            .await
            .unwrap(),
        3
    );
    let remaining: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM relay_invite_claims WHERE community_id=$1 ORDER BY queue_order",
    )
    .bind(community.as_uuid())
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        remaining,
        vec![unready, pending],
        "active unready and unprocessed claims cannot be acknowledged"
    );
    assert!(db
        .acknowledge_invite_claim_results(
            community,
            &vec![Uuid::nil(); MAX_QUEUED_ROWS as usize + 1]
        )
        .await
        .is_err());
}
