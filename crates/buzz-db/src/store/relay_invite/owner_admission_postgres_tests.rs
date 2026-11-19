use super::revocation_postgres_tests::claim_relay_invite_outcome;
use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_policy_evidence_rolls_back_verified_owner_and_membership() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let claimant = postgres_tests::test_pubkey();
    let owner = postgres_tests::test_pubkey();
    let invite = mint_relay_invite(
        &pool,
        community,
        &postgres_tests::test_pubkey(),
        3600,
        Some(1),
    )
    .await
    .unwrap();
    assert!(claim_relay_invite_with_owner(
        &pool,
        community,
        &hash_v2_code(&invite.code),
        &claimant,
        Some("too-short"),
        Some(&owner),
        &nostr::Keys::generate()
    )
    .await
    .is_err());
    assert!(claim_relay_membership_with_owner(
        &pool,
        community,
        &claimant,
        "member",
        Some("too-short"),
        Some(&owner),
        &nostr::Keys::generate()
    )
    .await
    .is_err());
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE community_id=$1")
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
    assert!(
        !crate::relay_members::is_relay_member(&pool, community, &claimant)
            .await
            .unwrap()
    );
    assert_eq!(
        postgres_tests::use_count(&pool, community, invite.invite_id).await,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn successful_claim_reports_only_committed_user_creation() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let claimant = postgres_tests::test_pubkey();
    let owner = postgres_tests::test_pubkey();
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
    let first = claim_relay_invite_with_owner(
        &pool,
        community,
        &hash,
        &claimant,
        None,
        Some(&owner),
        &nostr::Keys::generate(),
    )
    .await
    .unwrap();
    assert!(matches!(first.outcome.outcome, ClaimOutcome::Joined { .. }));
    assert!(first.outcome.owner_link_created);
    assert_eq!(first.outcome.users_created, 2);
    let retry = claim_relay_invite_with_owner(
        &pool,
        community,
        &hash,
        &claimant,
        None,
        Some(&owner),
        &nostr::Keys::generate(),
    )
    .await
    .unwrap();
    assert!(matches!(
        retry.outcome.outcome,
        ClaimOutcome::AlreadyMember { .. }
    ));
    assert!(!retry.outcome.owner_link_created);
    assert_eq!(retry.outcome.users_created, 0);
    let legacy_agent = postgres_tests::test_pubkey();
    let legacy = claim_relay_membership_with_owner(
        &pool,
        community,
        &legacy_agent,
        "member",
        None,
        Some(&owner),
        &nostr::Keys::generate(),
    )
    .await
    .unwrap();
    assert!(legacy.outcome.inserted && legacy.outcome.owner_link_created);
    assert_eq!(legacy.outcome.users_created, 1);
    let retry = claim_relay_membership_with_owner(
        &pool,
        community,
        &legacy_agent,
        "member",
        None,
        Some(&owner),
        &nostr::Keys::generate(),
    )
    .await
    .unwrap();
    assert!(!retry.outcome.inserted && !retry.outcome.owner_link_created);
    assert_eq!(retry.outcome.users_created, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn issuer_timeout_denies_temporarily_without_revoking_invite() {
    let pool = postgres_tests::setup_pool().await;
    let community = postgres_tests::make_test_community(&pool).await;
    let issuer = postgres_tests::test_pubkey();
    let claimant = postgres_tests::test_pubkey();
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, Some(1))
        .await
        .unwrap();
    sqlx::query("INSERT INTO community_bans (community_id,pubkey,muted_until,actor_pubkey) VALUES ($1,$2,clock_timestamp()+interval '1 hour',$2)")
        .bind(community.as_uuid()).bind(hex::decode(&issuer).unwrap()).execute(&pool).await.unwrap();
    assert_eq!(
        claim_relay_invite_outcome(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::IssuerRestricted
    );
    let revoked: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM relay_invites WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(invite.invite_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked.is_none());
    sqlx::query("UPDATE community_bans SET muted_until=clock_timestamp()-interval '1 second' WHERE community_id=$1")
        .bind(community.as_uuid()).execute(&pool).await.unwrap();
    assert!(matches!(
        claim_relay_invite_outcome(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Joined { use_count: 1, .. }
    ));
}
