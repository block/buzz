use super::*;
use crate::moderation::{ban_member, unban_member};
use uuid::Uuid;

async fn fixture() -> (PgPool, CommunityId, String, String) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("invite-{}.example", id.simple()))
        .execute(&pool)
        .await
        .unwrap();
    (
        pool,
        CommunityId::from_uuid(id),
        hex::encode([1; 32]),
        hex::encode([2; 32]),
    )
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn ban_permanently_revokes_issued_invites_but_not_another_tenant() {
    let (pool, community, issuer, claimant) = fixture().await;
    let other_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(other_id)
        .bind(format!("other-{}.example", other_id.simple()))
        .execute(&pool)
        .await
        .unwrap();
    let other = CommunityId::from_uuid(other_id);
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    let unaffected = mint_relay_invite(&pool, other, &issuer, 3600, None)
        .await
        .unwrap();
    ban_member(&pool, community, &[1; 32], &[3; 32], None, None)
        .await
        .unwrap();
    assert!(matches!(
        mint_relay_invite(&pool, community, &issuer, 3600, None).await,
        Err(crate::DbError::InviteRestricted)
    ));
    assert!(unban_member(&pool, community, &[1; 32], &[3; 32])
        .await
        .unwrap());
    assert_eq!(
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Revoked
    );
    assert!(matches!(
        claim_relay_invite(
            &pool,
            other,
            &hash_v2_code(&unaffected.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Joined { .. }
    ));
    let renewed = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    assert!(matches!(
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&renewed.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Joined { .. }
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn nonmember_restrictions_cover_both_claim_formats_and_owner() {
    let (pool, community, issuer, claimant) = fixture().await;
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, Some(1))
        .await
        .unwrap();
    ban_member(&pool, community, &[2; 32], &[3; 32], None, None)
        .await
        .unwrap();
    assert_eq!(
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Restricted
    );
    assert!(matches!(
        crate::relay_members::claim_relay_membership(&pool, community, &claimant, "member", None)
            .await,
        Err(crate::DbError::InviteRestricted)
    ));
    let agent = hex::encode([4; 32]);
    sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2),($1,$3)")
        .bind(community.as_uuid())
        .bind(&[2u8; 32][..])
        .bind(&[4u8; 32][..])
        .execute(&pool)
        .await
        .unwrap();
    crate::user::set_agent_owner(&pool, community, &[4; 32], &[2; 32])
        .await
        .unwrap();
    assert_eq!(
        claim_relay_invite(&pool, community, &hash_v2_code(&invite.code), &agent, None)
            .await
            .unwrap(),
        ClaimOutcome::Restricted
    );
    assert!(
        !crate::relay_members::is_relay_member(&pool, community, &claimant)
            .await
            .unwrap()
    );
    let count: i32 =
        sqlx::query_scalar("SELECT use_count FROM relay_invites WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(invite.invite_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn issuer_agent_revocation_survives_owner_unban() {
    let (pool, community, issuer, claimant) = fixture().await;
    sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2),($1,$3)")
        .bind(community.as_uuid())
        .bind(&[1u8; 32][..])
        .bind(&[4u8; 32][..])
        .execute(&pool)
        .await
        .unwrap();
    crate::user::set_agent_owner(&pool, community, &[1; 32], &[4; 32])
        .await
        .unwrap();
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    ban_member(&pool, community, &[4; 32], &[3; 32], None, None)
        .await
        .unwrap();
    unban_member(&pool, community, &[4; 32], &[3; 32])
        .await
        .unwrap();
    assert_eq!(
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Revoked
    );
}

async fn queued_claim(
    pool: &PgPool,
    community: CommunityId,
    code: &str,
    claimant: String,
) -> tokio::task::JoinHandle<Result<ClaimOutcome>> {
    let name = format!("invite-race-{}", Uuid::new_v4());
    let connection_name = name.clone();
    let claim_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |conn, _| {
            let name = connection_name.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('application_name',$1,false)")
                    .bind(name)
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let hash = hash_v2_code(code);
    let claim = tokio::spawn(async move {
        claim_relay_invite(&claim_pool, community, &hash, &claimant, None).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let waiting: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE a.application_name=$1 AND l.locktype='advisory' AND NOT l.granted)")
                .bind(&name).fetch_one(pool).await.unwrap();
            if waiting {break;}
            assert!(!claim.is_finished(),"claim bypassed the production admission lock");
            tokio::task::yield_now().await;
        }
    }).await.expect("claim must observably wait on the admission lock");
    claim
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn claimant_ban_and_timeout_commit_before_queued_claim_without_consuming_a_use() {
    for timeout in [false, true] {
        let (pool, community, issuer, claimant) = fixture().await;
        let invite = mint_relay_invite(&pool, community, &issuer, 3600, Some(1))
            .await
            .unwrap();
        let mut restriction = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,banned,muted_until,actor_pubkey) VALUES($1,$2,$3,CASE WHEN $3 THEN NULL ELSE clock_timestamp()+interval '1 hour' END,$4)")
            .bind(community.as_uuid()).bind(vec![2u8;32]).bind(!timeout).bind(vec![3u8;32])
            .execute(&mut *restriction).await.unwrap();
        let claim = queued_claim(&pool, community, &invite.code, claimant.clone()).await;
        restriction.commit().await.unwrap();
        assert_eq!(claim.await.unwrap().unwrap(), ClaimOutcome::Restricted);
        let uses: i32 = sqlx::query_scalar(
            "SELECT use_count FROM relay_invites WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(invite.invite_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(uses, 0);
        assert!(
            !crate::relay_members::is_relay_member(&pool, community, &claimant)
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_materialization_commits_before_queued_agent_claim() {
    let (pool, community, issuer, agent) = fixture().await;
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2),($1,$3)")
        .bind(community.as_uuid())
        .bind(vec![2u8; 32])
        .bind(vec![4u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    ban_member(&pool, community, &[4; 32], &[3; 32], None, None)
        .await
        .unwrap();
    let mut owner = pool.begin().await.unwrap();
    sqlx::query("UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2")
        .bind(community.as_uuid())
        .bind(vec![2u8; 32])
        .bind(vec![4u8; 32])
        .execute(&mut *owner)
        .await
        .unwrap();
    let claim = queued_claim(&pool, community, &invite.code, agent).await;
    owner.commit().await.unwrap();
    assert_eq!(claim.await.unwrap().unwrap(), ClaimOutcome::Restricted);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn admission_first_then_ban_preserves_the_committed_use_and_denies_retries() {
    let (pool, community, issuer, claimant) = fixture().await;
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, Some(2))
        .await
        .unwrap();
    assert!(matches!(
        claim_relay_invite(
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
    ban_member(&pool, community, &[2; 32], &[3; 32], None, None)
        .await
        .unwrap();
    assert_eq!(
        claim_relay_invite(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Restricted
    );
    let uses: i32 =
        sqlx::query_scalar("SELECT use_count FROM relay_invites WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(invite.invite_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(uses, 1);
}
