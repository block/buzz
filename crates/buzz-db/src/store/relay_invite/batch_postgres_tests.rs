use super::*;
use crate::relay_members::{assert_membership_list_matches, is_relay_member};

fn legacy(pubkey: String) -> ClaimRequest {
    ClaimRequest {
        pubkey,
        policy_version: None,
        verified_owner: None,
        credential: ClaimCredential::Legacy {
            role: "member".into(),
        },
    }
}

fn v2(pubkey: String, hash: [u8; 32]) -> ClaimRequest {
    ClaimRequest {
        credential: ClaimCredential::V2 { token_hash: hash },
        ..legacy(pubkey)
    }
}

async fn commit(
    pool: &PgPool,
    community: CommunityId,
    requests: &[ClaimRequest],
    keys: &nostr::Keys,
) -> Result<MembershipWrite<Vec<Result<ClaimResult>>>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    lock_admission(tx.conn(), community).await?;
    let applied = apply_claim_batch(&mut tx, requests, keys).await?;
    tx.commit().await?;
    Ok(applied.into_committed())
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mixed_claims_share_one_atomic_snapshot_and_keep_pending_tokens() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let keys = nostr::Keys::generate();
    let invite = mint_relay_invite(
        &pool,
        community,
        &postgres_tests::test_pubkey(),
        3600,
        Some(2),
    )
    .await
    .unwrap();
    let requests = [
        legacy(postgres_tests::test_pubkey()),
        ClaimRequest {
            policy_version: Some("a".repeat(64)),
            verified_owner: Some(postgres_tests::test_pubkey()),
            ..v2(postgres_tests::test_pubkey(), hash_v2_code(&invite.code))
        },
    ];
    let write = commit(&pool, community, &requests, &keys).await.unwrap();
    assert!(matches!(&write.outcome[0], Ok(ClaimResult::Legacy(result)) if result.inserted));
    assert!(
        matches!(&write.outcome[1], Ok(ClaimResult::V2(result)) if matches!(result.outcome, ClaimOutcome::Joined { use_count: 1, .. }))
    );
    assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
    let (members, pending): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(auto_join_pending) FROM relay_members WHERE community_id=$1",
    )
    .bind(community.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((members, pending), (2, 2));
    assert!(
        matches!(&write.outcome[1], Ok(ClaimResult::V2(result)) if result.owner_link_created && result.users_created == 2)
    );
    let policy: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM join_policy_acceptances WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community.as_uuid())
    .bind(&requests[1].pubkey)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(policy, 1);

    let lists: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=13534")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(lists, 1, "the batch signs one list, not one per claim");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn lone_claim_commits_and_repeated_claim_keeps_its_snapshot() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let requests = [legacy(postgres_tests::test_pubkey())];
    let keys = nostr::Keys::generate();
    let first = commit(&pool, community, &requests, &keys).await.unwrap();
    assert_membership_list_matches(&pool, community, &keys, first.snapshot.as_ref()).await;
    let retry = commit(&pool, community, &requests, &keys).await.unwrap();
    assert!(matches!(&retry.outcome[0], Ok(ClaimResult::Legacy(result)) if !result.inserted));
    assert!(retry.snapshot.is_none());
    assert_membership_list_matches(&pool, community, &keys, first.snapshot.as_ref()).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn shared_invite_final_slot_is_consumed_once_in_a_batch() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let invite = mint_relay_invite(
        &pool,
        community,
        &postgres_tests::test_pubkey(),
        3600,
        Some(1),
    )
    .await
    .unwrap();
    let hash = hash_v2_code(&invite.code);
    let requests = [
        v2(postgres_tests::test_pubkey(), hash),
        v2(postgres_tests::test_pubkey(), hash),
    ];
    let keys = nostr::Keys::generate();
    let write = commit(&pool, community, &requests, &keys).await.unwrap();
    assert!(
        matches!(&write.outcome[0], Ok(ClaimResult::V2(result)) if matches!(result.outcome, ClaimOutcome::Joined { use_count: 1, .. }))
    );
    assert!(
        matches!(&write.outcome[1], Ok(ClaimResult::V2(result)) if result.outcome == ClaimOutcome::Exhausted)
    );
    assert!(!is_relay_member(&pool, community, &requests[1].pubkey)
        .await
        .unwrap());
    assert_eq!(
        postgres_tests::use_count(&pool, community, invite.invite_id).await,
        1
    );
    assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expected_denials_do_not_poison_other_claims_or_create_users() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let restricted = postgres_tests::test_pubkey();
    crate::moderation::ban_member(
        &pool,
        community,
        &hex::decode(&restricted).unwrap(),
        &[7; 32],
        None,
        None,
    )
    .await
    .unwrap();
    let agent = postgres_tests::test_pubkey();
    let owner = postgres_tests::test_pubkey();
    sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2)")
        .bind(community.as_uuid())
        .bind(hex::decode(&owner).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (community_id,pubkey,agent_owner_pubkey) VALUES ($1,$2,$3)")
        .bind(community.as_uuid())
        .bind(hex::decode(&agent).unwrap())
        .bind(hex::decode(&owner).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let mismatch_owner = postgres_tests::test_pubkey();
    let invite = mint_relay_invite(
        &pool,
        community,
        &postgres_tests::test_pubkey(),
        3600,
        Some(1),
    )
    .await
    .unwrap();
    let requests = [
        legacy(restricted),
        ClaimRequest {
            verified_owner: Some(mismatch_owner.clone()),
            ..v2(agent, hash_v2_code(&invite.code))
        },
        legacy(postgres_tests::test_pubkey()),
    ];
    let keys = nostr::Keys::generate();
    let write = commit(&pool, community, &requests, &keys).await.unwrap();
    assert!(matches!(
        &write.outcome[0],
        Err(crate::DbError::InviteRestricted)
    ));
    assert!(matches!(
        &write.outcome[1],
        Err(crate::DbError::InviteOwnerMismatch)
    ));
    assert!(matches!(&write.outcome[2], Ok(ClaimResult::Legacy(result)) if result.inserted));
    let unexpected: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE community_id=$1 AND pubkey=$2)",
    )
    .bind(community.as_uuid())
    .bind(hex::decode(mismatch_owner).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!unexpected);
    assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unexpected_storage_failure_rolls_back_every_claim_and_snapshot() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let invite = mint_relay_invite(
        &pool,
        community,
        &postgres_tests::test_pubkey(),
        3600,
        Some(1),
    )
    .await
    .unwrap();
    let requests = [
        ClaimRequest {
            verified_owner: Some(postgres_tests::test_pubkey()),
            policy_version: Some("b".repeat(64)),
            ..v2(postgres_tests::test_pubkey(), hash_v2_code(&invite.code))
        },
        ClaimRequest {
            policy_version: Some("too-short".into()),
            ..legacy(postgres_tests::test_pubkey())
        },
    ];
    let result = commit(&pool, community, &requests, &nostr::Keys::generate()).await;
    assert!(result.is_err());
    assert_eq!(
        postgres_tests::use_count(&pool, community, invite.invite_id).await,
        0
    );
    let members: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_members WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let lists: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=13534")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((members, lists), (0, 0));
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE community_id=$1")
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let policies: i64 =
        sqlx::query_scalar("SELECT count(*) FROM join_policy_acceptances WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((users, policies), (0, 0));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn snapshot_failure_rolls_back_all_claims() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let keys = nostr::Keys::generate();
    let requests = [
        legacy(postgres_tests::test_pubkey()),
        legacy(postgres_tests::test_pubkey()),
    ];
    let key = crate::replaceable::event_replacement_lock_key(
        community,
        13534,
        keys.public_key().to_bytes().as_slice(),
        None,
    );
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let mut tx = crate::begin_community_write_transaction(
        &pool,
        community,
        crate::observability::WriterOperation::Authorization,
    )
    .await
    .unwrap();
    lock_admission(tx.conn(), community).await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='30ms'")
        .execute(tx.conn())
        .await
        .unwrap();
    let result = apply_claim_batch(&mut tx, &requests, &keys).await;
    tx.rollback().await.unwrap();
    blocker.rollback().await.unwrap();
    assert!(
        matches!(result, Err(crate::DbError::Sqlx(sqlx::Error::Database(ref error))) if error.code().as_deref()==Some("55P03"))
    );
    for request in requests {
        assert!(!is_relay_member(&pool, community, &request.pubkey)
            .await
            .unwrap());
    }
    let lists: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=13534")
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(lists, 0);
}
