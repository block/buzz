use super::*;
use crate::moderation::{ban_member, unban_member};
use uuid::Uuid;

pub(super) async fn claim_relay_invite_outcome(
    pool: &PgPool,
    community: CommunityId,
    token_hash: &[u8; 32],
    claimer: &str,
    policy_version: Option<&str>,
) -> Result<ClaimOutcome> {
    super::claim_relay_invite(
        pool,
        community,
        token_hash,
        claimer,
        policy_version,
        &nostr::Keys::generate(),
    )
    .await
    .map(|write| write.outcome)
}

pub(super) async fn fixture() -> (PgPool, CommunityId, String, String) {
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
        claim_relay_invite_outcome(
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
        claim_relay_invite_outcome(
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
        claim_relay_invite_outcome(
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
        claim_relay_invite_outcome(
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
        crate::relay_members::claim_relay_membership(
            &pool,
            community,
            &claimant,
            "member",
            None,
            &nostr::Keys::generate()
        )
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
        claim_relay_invite_outcome(&pool, community, &hash_v2_code(&invite.code), &agent, None)
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
        claim_relay_invite_outcome(
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stalled_production_mint_ignores_lifted_history_and_later_audit_touches() {
    let (pool, community, issuer, claimant) = fixture().await;
    let issuer_bytes = [1u8; 32];
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, false, $3)",
    )
    .bind(community.as_uuid())
    .bind(issuer_bytes.as_slice())
    .bind([3u8; 32].as_slice())
    .execute(&pool)
    .await
    .unwrap();

    let old_action = Uuid::new_v4();
    let old_action_time = Utc::now() - chrono::Duration::days(1);
    sqlx::query(
        "INSERT INTO relay_admin_actions \
         (id, report_community_id, request_id, actor_pubkey, actor_role, action, \
          state, step_marker, enforcement_target_pubkey, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'operator', 'ban', 'succeeded', \
                 'mutation_committed', $5, $6, $6)",
    )
    .bind(old_action)
    .bind(community.as_uuid())
    .bind(Uuid::new_v4())
    .bind([3u8; 32].as_slice())
    .bind(issuer_bytes.as_slice())
    .bind(old_action_time)
    .execute(&pool)
    .await
    .unwrap();

    // Hold the production admission seam so mint starts its transaction and
    // stalls before insert. Touching the old action while it waits creates the
    // timestamp ordering that the retired audit predicate misread as proof.
    let mut admission = pool.begin().await.unwrap();
    lock_admission(&mut admission, community).await.unwrap();
    let (mint_pool, mint_name) = named_pool().await;
    let mint_issuer = issuer.clone();
    let mint = tokio::spawn(async move {
        mint_relay_invite(&mint_pool, community, &mint_issuer, 3600, None).await
    });
    wait_for_lock(&pool, &mint_name, "advisory", &mint).await;

    sqlx::query(
        "UPDATE relay_admin_actions \
         SET updated_at=clock_timestamp()+interval '1 minute' WHERE id=$1",
    )
    .bind(old_action)
    .execute(&pool)
    .await
    .unwrap();
    admission.commit().await.unwrap();

    let invite = tokio::time::timeout(std::time::Duration::from_secs(5), mint)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let marker: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(invite.invite_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !marker,
        "the production mint seam marks protocol-native rows"
    );

    let old_action_touched_after_mint_started: bool = sqlx::query_scalar(
        "SELECT a.updated_at >= i.created_at \
         FROM relay_admin_actions a CROSS JOIN relay_invites i \
         WHERE a.id=$1 AND i.community_id=$2 AND i.id=$3",
    )
    .bind(old_action)
    .bind(community.as_uuid())
    .bind(invite.invite_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        old_action_touched_after_mint_started,
        "the mint transaction began before the old action timestamp was touched"
    );
    for action in ["ban", "resolve:ban"] {
        sqlx::query(
            "INSERT INTO moderation_actions \
             (community_id, actor_pubkey, action, target_pubkey, created_at) \
             VALUES ($1, $2, $3, $4, clock_timestamp()+interval '1 minute')",
        )
        .bind(community.as_uuid())
        .bind([2u8; 32].as_slice())
        .bind(action)
        .bind(issuer_bytes.as_slice())
        .execute(&pool)
        .await
        .unwrap();
    }

    let revoked: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM relay_invites WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(invite.invite_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        revoked.is_none(),
        "lifted history and late audits spare new rows"
    );
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
        ClaimOutcome::Joined { .. }
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_ban_lazily_revokes_new_protocol_rows() {
    let (pool, community, issuer, claimant) = fixture().await;
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    let legacy_admission: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(invite.invite_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!legacy_admission);
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, true, $3)",
    )
    .bind(community.as_uuid())
    .bind([1u8; 32].as_slice())
    .bind([3u8; 32].as_slice())
    .execute(&pool)
    .await
    .unwrap();

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
        ClaimOutcome::Revoked
    );
    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM relay_invites WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(invite.invite_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoked, "active bans durably revoke new-protocol rows");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn malformed_legacy_issuers_return_a_typed_revoked_claim() {
    let (pool, community, _, claimant) = fixture().await;
    let issuers = [
        "not-a-pubkey".to_owned(),
        hex::encode([0xabu8; 32]).to_ascii_uppercase(),
    ];
    for (index, issuer) in issuers.iter().enumerate() {
        let token_hash = [index as u8 + 10; 32];
        sqlx::query(
            "INSERT INTO relay_invites (community_id, token_hash, created_by, expires_at) \
             VALUES ($1, $2, $3, clock_timestamp()+interval '1 day')",
        )
        .bind(community.as_uuid())
        .bind(token_hash.as_slice())
        .bind(issuer)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            claim_relay_invite_outcome(&pool, community, &token_hash, &claimant, None)
                .await
                .unwrap(),
            ClaimOutcome::Revoked,
            "issuer {issuer} must fail closed without a database error"
        );
    }

    let token_hash = [99u8; 32];
    sqlx::query(
        "INSERT INTO relay_invites \
         (community_id, token_hash, created_by, expires_at, legacy_admission) \
         VALUES ($1, $2, 'not-a-pubkey', clock_timestamp()+interval '1 day', false)",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        claim_relay_invite_outcome(&pool, community, &token_hash, &claimant, None)
            .await
            .unwrap(),
        ClaimOutcome::Invalid,
        "a malformed non-legacy issuer returns a clean invalid outcome"
    );
}

pub(super) async fn named_pool() -> (PgPool, String) {
    let name = format!("invite-race-{}", Uuid::new_v4());
    let connection_name = name.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
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
    (pool, name)
}

pub(super) async fn wait_for_lock<T>(
    pool: &PgPool,
    name: &str,
    kind: &str,
    task: &tokio::task::JoinHandle<T>,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid \
                 WHERE a.application_name=$1 AND l.locktype=$2 AND NOT l.granted)",
            )
            .bind(name)
            .bind(kind)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            assert!(!task.is_finished(), "writer bypassed expected {kind} lock");
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer must observably wait on the expected lock");
}

async fn queued_claim(
    pool: &PgPool,
    community: CommunityId,
    code: &str,
    claimant: String,
) -> tokio::task::JoinHandle<Result<ClaimOutcome>> {
    let (claim_pool, name) = named_pool().await;
    let hash = hash_v2_code(code);
    let claim = tokio::spawn(async move {
        claim_relay_invite_outcome(&claim_pool, community, &hash, &claimant, None).await
    });
    wait_for_lock(pool, &name, "advisory", &claim).await;
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
        // Hold the existing restriction row so the production writer stops only
        // after acquiring admission. The claim must queue behind that writer.
        ban_member(&pool, community, &[2; 32], &[3; 32], None, None)
            .await
            .unwrap();
        unban_member(&pool, community, &[2; 32], &[3; 32])
            .await
            .unwrap();
        let mut row_lock = pool.begin().await.unwrap();
        sqlx::query("SELECT pubkey FROM community_bans WHERE community_id=$1 FOR UPDATE")
            .bind(community.as_uuid())
            .fetch_one(&mut *row_lock)
            .await
            .unwrap();
        let (writer_pool, name) = named_pool().await;
        let restriction = tokio::spawn(async move {
            if timeout {
                crate::moderation::timeout_member(
                    &writer_pool,
                    community,
                    &[2; 32],
                    &[3; 32],
                    Utc::now() + chrono::Duration::hours(1),
                    None,
                )
                .await
            } else {
                ban_member(&writer_pool, community, &[2; 32], &[3; 32], None, None).await
            }
        });
        wait_for_lock(&pool, &name, "transactionid", &restriction).await;
        let claim = queued_claim(&pool, community, &invite.code, claimant.clone()).await;
        row_lock.commit().await.unwrap();
        restriction.await.unwrap().unwrap();
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
    let mut row_lock = pool.begin().await.unwrap();
    sqlx::query("SELECT pubkey FROM users WHERE community_id=$1 AND pubkey=$2 FOR UPDATE")
        .bind(community.as_uuid())
        .bind(vec![2u8; 32])
        .fetch_one(&mut *row_lock)
        .await
        .unwrap();
    let (writer_pool, name) = named_pool().await;
    let owner = tokio::spawn(async move {
        crate::user::set_agent_owner(&writer_pool, community, &[2; 32], &[4; 32]).await
    });
    wait_for_lock(&pool, &name, "transactionid", &owner).await;
    let claim = queued_claim(&pool, community, &invite.code, agent).await;
    row_lock.commit().await.unwrap();
    assert!(owner.await.unwrap().unwrap());
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
    ban_member(&pool, community, &[2; 32], &[3; 32], None, None)
        .await
        .unwrap();
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
