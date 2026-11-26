use super::revocation_postgres_tests::{
    claim_relay_invite_outcome, fixture, named_pool, wait_for_lock,
};
use super::*;
use crate::moderation::ban_member;
use crate::relay_members::{add_relay_member, remove_relay_member, update_relay_member_role};
use crate::test_support::ScratchDb;

async fn add_member(pool: &PgPool, community: CommunityId, pubkey: &str, role: &str) {
    add_relay_member(
        pool,
        community,
        pubkey,
        role,
        None,
        &nostr::Keys::generate(),
    )
    .await
    .map(|write| write.outcome)
    .unwrap();
}

async fn revoke(
    pool: &PgPool,
    community: CommunityId,
    actor: &str,
    selector: InviteSelector<'_>,
) -> Result<Option<RevokedInvite>> {
    revoke_relay_invite(pool, community, actor, selector).await
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revoke_by_code_targets_exactly_that_invite_and_is_idempotent() {
    let (pool, community, admin, claimant) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    // Two invites that differ only in their secret.
    let first = mint_relay_invite(&pool, community, &admin, 3600, Some(5))
        .await
        .unwrap();
    let second = mint_relay_invite(&pool, community, &admin, 3600, Some(5))
        .await
        .unwrap();

    let hash = hash_v2_code(&first.code);
    let revoked = revoke(&pool, community, &admin, InviteSelector::TokenHash(&hash))
        .await
        .unwrap()
        .expect("revoked");
    assert_eq!(revoked.id, first.invite_id);
    let again = revoke(
        &pool,
        community,
        &admin,
        InviteSelector::Id(first.invite_id),
    )
    .await
    .unwrap()
    .expect("still revoked");
    assert_eq!(again, revoked, "a second revoke keeps the first time");

    assert_eq!(
        claim_relay_invite_outcome(&pool, community, &hash, &claimant, None)
            .await
            .unwrap(),
        ClaimOutcome::Revoked
    );
    assert!(matches!(
        claim_relay_invite_outcome(
            &pool,
            community,
            &hash_v2_code(&second.code),
            &claimant,
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Joined { .. }
    ));
    // Revoking never removes the member a claim already added.
    revoke(
        &pool,
        community,
        &admin,
        InviteSelector::Id(second.invite_id),
    )
    .await
    .unwrap()
    .expect("revoked");
    assert!(
        crate::relay_members::is_relay_member(&pool, community, &claimant)
            .await
            .unwrap()
    );

    let listed = list_relay_invites(&pool, community, 50, None)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|invite| invite.status(Utc::now()) == InviteStatus::Revoked));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revoke_is_scoped_to_the_community() {
    let (pool, community, admin, _) = fixture().await;
    let (_, other, other_admin, _) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    add_member(&pool, other, &other_admin, "admin").await;
    let invite = mint_relay_invite(&pool, community, &admin, 3600, None)
        .await
        .unwrap();
    let hash = hash_v2_code(&invite.code);

    for selector in [
        InviteSelector::Id(invite.invite_id),
        InviteSelector::TokenHash(&hash),
    ] {
        assert_eq!(
            revoke(&pool, other, &other_admin, selector).await.unwrap(),
            None
        );
    }
    assert_eq!(
        revoke(&pool, community, &admin, InviteSelector::Id(Uuid::new_v4()))
            .await
            .unwrap(),
        None
    );
    assert!(list_relay_invites(&pool, other, 50, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        list_relay_invites(&pool, community, 50, None)
            .await
            .unwrap()[0]
            .revoked_at,
        None
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn creator_or_admin_may_revoke_and_other_members_may_not() {
    let (pool, community, creator, other_member) = fixture().await;
    let admin = hex::encode([4; 32]);
    add_member(&pool, community, &creator, "admin").await;
    add_member(&pool, community, &other_member, "member").await;
    add_member(&pool, community, &admin, "admin").await;
    let mine = mint_relay_invite(&pool, community, &creator, 3600, None)
        .await
        .unwrap();
    let also_mine = mint_relay_invite(&pool, community, &creator, 3600, None)
        .await
        .unwrap();

    // The creator is demoted but stays a member: they can still revoke theirs.
    update_relay_member_role(
        &pool,
        community,
        &creator,
        "member",
        &nostr::Keys::generate(),
    )
    .await
    .map(|write| write.outcome)
    .unwrap();
    assert!(revoke(
        &pool,
        community,
        &creator,
        InviteSelector::Id(mine.invite_id)
    )
    .await
    .unwrap()
    .is_some());

    // A member who did not create it is refused and the row is untouched.
    assert!(matches!(
        revoke(
            &pool,
            community,
            &other_member,
            InviteSelector::Id(also_mine.invite_id)
        )
        .await,
        Err(crate::DbError::InviteRevokeForbidden)
    ));

    // A creator who left the community is refused.
    remove_relay_member(&pool, community, &creator, &nostr::Keys::generate())
        .await
        .map(|write| write.outcome)
        .unwrap();
    assert!(matches!(
        revoke(
            &pool,
            community,
            &creator,
            InviteSelector::Id(also_mine.invite_id)
        )
        .await,
        Err(crate::DbError::InviteRevokeForbidden)
    ));
    let listed = list_relay_invites(&pool, community, 50, None)
        .await
        .unwrap();
    let untouched = listed
        .iter()
        .find(|invite| invite.id == also_mine.invite_id)
        .unwrap();
    assert_eq!(untouched.revoked_at, None);

    // Any admin can revoke an invite someone else made.
    assert!(revoke(
        &pool,
        community,
        &admin,
        InviteSelector::Id(also_mine.invite_id)
    )
    .await
    .unwrap()
    .is_some());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn restricted_admin_cannot_revoke() {
    let (pool, community, admin, _) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    ban_member(&pool, community, &[1; 32], &[3; 32], None, None)
        .await
        .unwrap();
    // Another admin's invite, so only the actor's restriction can refuse.
    let other_admin = hex::encode([5; 32]);
    add_member(&pool, community, &other_admin, "admin").await;
    let other = mint_relay_invite(&pool, community, &other_admin, 3600, None)
        .await
        .unwrap();
    assert!(matches!(
        revoke(
            &pool,
            community,
            &admin,
            InviteSelector::Id(other.invite_id)
        )
        .await,
        Err(crate::DbError::InviteRestricted)
    ));
}

async fn revoked_at(pool: &PgPool, community: CommunityId, id: Uuid) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT revoked_at FROM relay_invites WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Start a revoke on its own named connection.
async fn queued_revoke(
    community: CommunityId,
    actor: &str,
    id: Uuid,
) -> (
    tokio::task::JoinHandle<Result<Option<RevokedInvite>>>,
    String,
) {
    let (pool, name) = named_pool().await;
    let actor = actor.to_owned();
    let task = tokio::spawn(async move {
        revoke_relay_invite(&pool, community, &actor, InviteSelector::Id(id)).await
    });
    (task, name)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn demotion_that_wins_the_role_row_refuses_the_revoke() {
    let (pool, community, admin, other_admin) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    add_member(&pool, community, &other_admin, "admin").await;
    // Minted by someone else, so a demoted actor loses the right to revoke it.
    let invite = mint_relay_invite(&pool, community, &other_admin, 3600, None)
        .await
        .unwrap();

    let mut demotion = pool.begin().await.unwrap();
    sqlx::query("UPDATE relay_members SET role = 'member' WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(&admin)
        .execute(&mut *demotion)
        .await
        .unwrap();
    let (revoking, name) = queued_revoke(community, &admin, invite.invite_id).await;
    // The revoke's role read `FOR SHARE` waits on the demotion's row lock.
    wait_for_lock(&pool, &name, "transactionid", &revoking).await;
    demotion.commit().await.unwrap();
    assert!(matches!(
        revoking.await.unwrap(),
        Err(crate::DbError::InviteRevokeForbidden)
    ));
    assert_eq!(revoked_at(&pool, community, invite.invite_id).await, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn restriction_that_wins_admission_refuses_the_revoke() {
    let (pool, community, admin, other_admin) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    add_member(&pool, community, &other_admin, "admin").await;
    let invite = mint_relay_invite(&pool, community, &other_admin, 3600, None)
        .await
        .unwrap();

    // A restriction writer holds the admission lock and bans the actor.
    let mut restriction = pool.begin().await.unwrap();
    lock_admission(&mut restriction, community).await.unwrap();
    let (revoking, name) = queued_revoke(community, &admin, invite.invite_id).await;
    wait_for_lock(&pool, &name, "advisory", &revoking).await;
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, true, $3)",
    )
    .bind(community.as_uuid())
    .bind(hex::decode(&admin).unwrap())
    .bind([3u8; 32].as_slice())
    .execute(&mut *restriction)
    .await
    .unwrap();
    restriction.commit().await.unwrap();
    assert!(matches!(
        revoking.await.unwrap(),
        Err(crate::DbError::InviteRestricted)
    ));
    assert_eq!(revoked_at(&pool, community, invite.invite_id).await, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn claim_and_revoke_serialize_and_the_claim_keeps_its_member() {
    let (pool, community, admin, claimant) = fixture().await;
    add_member(&pool, community, &admin, "admin").await;
    let invite = mint_relay_invite(&pool, community, &admin, 3600, Some(2))
        .await
        .unwrap();
    let hash = hash_v2_code(&invite.code);

    // The claim pauses holding the admission lock and the invite row.
    let (arrived, release) = super::test_hook::arm_before_membership_insert(community);
    let claim = tokio::spawn({
        let pool = pool.clone();
        let claimant = claimant.clone();
        async move { claim_relay_invite_outcome(&pool, community, &hash, &claimant, None).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), arrived)
        .await
        .expect("claim reaches the hook")
        .expect("hook fires");
    let (revoking, name) = queued_revoke(community, &admin, invite.invite_id).await;
    wait_for_lock(&pool, &name, "advisory", &revoking).await;
    release.notify_one();

    assert!(matches!(
        claim.await.unwrap().unwrap(),
        ClaimOutcome::Joined { use_count: 1, .. }
    ));
    assert!(revoking.await.unwrap().unwrap().is_some());
    assert!(
        crate::relay_members::is_relay_member(&pool, community, &claimant)
            .await
            .unwrap()
    );
    assert_eq!(
        claim_relay_invite_outcome(&pool, community, &hash, &hex::encode([9; 32]), None)
            .await
            .unwrap(),
        ClaimOutcome::Revoked
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revoke_is_refused_while_the_community_is_quiescing() {
    let scratch =
        ScratchDb::create(&crate::test_support::database_url(), "invite_revoke_fence").await;
    let db = crate::Db::new(&crate::DbConfig {
        database_url: scratch.url().to_owned(),
        max_connections: 5,
        min_connections: 0,
        ..crate::DbConfig::default()
    })
    .await
    .unwrap();
    db.migrate().await.unwrap();
    let pool = db.pool.clone();
    let host = format!("invite-revoke-fence-{}.example", Uuid::new_v4().simple());
    let community = db.ensure_configured_community(&host).await.unwrap().id;
    let admin = hex::encode([6; 32]);
    add_member(&pool, community, &admin, "admin").await;
    let invite = mint_relay_invite(&pool, community, &admin, 3600, None)
        .await
        .unwrap();
    super::postgres_tests::quiesce(&db, &host, community).await;

    assert!(matches!(
        revoke(
            &pool,
            community,
            &admin,
            InviteSelector::Id(invite.invite_id)
        )
        .await,
        Err(crate::DbError::AccessDenied(_))
    ));
    assert_eq!(revoked_at(&pool, community, invite.invite_id).await, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn list_pages_newest_first_even_when_the_cursor_row_is_gone() {
    let (pool, community, admin, _) = fixture().await;
    let mut minted = Vec::new();
    for _ in 0..3 {
        minted.push(
            mint_relay_invite(&pool, community, &admin, 3600, None)
                .await
                .unwrap()
                .invite_id,
        );
    }
    let first = list_relay_invites(&pool, community, 2, None).await.unwrap();
    assert_eq!(
        first.iter().map(|i| i.id).collect::<Vec<_>>(),
        [minted[2], minted[1]]
    );
    let cursor = (first[1].created_at, first[1].id);
    sqlx::query("DELETE FROM relay_invites WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(cursor.1)
        .execute(&pool)
        .await
        .unwrap();
    let rest = list_relay_invites(&pool, community, 2, Some(cursor))
        .await
        .unwrap();
    assert_eq!(rest.iter().map(|i| i.id).collect::<Vec<_>>(), [minted[0]]);
}

/// The next release writes invites with no expiry. Readers in this release
/// must already accept them, so this test drops `NOT NULL` on its own
/// database and reads a row with no expiry.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn readers_accept_an_invite_without_expiry() {
    let (pool, _scratch) =
        ScratchDb::migrated(&crate::test_support::database_url(), "invite_null_expiry").await;
    let host = format!("null-expiry-{}.example", Uuid::new_v4().simple());
    let community = crate::Db::from_pool(pool.clone())
        .ensure_configured_community(&host)
        .await
        .unwrap()
        .id;
    let admin = hex::encode([6; 32]);
    let claimant = hex::encode([7; 32]);
    add_member(&pool, community, &admin, "admin").await;
    sqlx::query("ALTER TABLE relay_invites ALTER COLUMN expires_at DROP NOT NULL")
        .execute(&pool)
        .await
        .unwrap();
    let invite = mint_relay_invite(&pool, community, &admin, 3600, None)
        .await
        .unwrap();
    sqlx::query("UPDATE relay_invites SET expires_at = NULL WHERE community_id = $1 AND id = $2")
        .bind(community.as_uuid())
        .bind(invite.invite_id)
        .execute(&pool)
        .await
        .unwrap();

    let listed = list_relay_invites(&pool, community, 50, None)
        .await
        .unwrap();
    assert_eq!(listed[0].expires_at, None);
    assert_eq!(listed[0].status(Utc::now()), InviteStatus::Active);
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
    assert_eq!(
        reap_expired_relay_invites(&pool, Utc::now()).await.unwrap(),
        0
    );
    revoke(
        &pool,
        community,
        &admin,
        InviteSelector::Id(invite.invite_id),
    )
    .await
    .unwrap()
    .expect("revoked");
    assert_eq!(
        claim_relay_invite_outcome(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &hex::encode([8; 32]),
            None
        )
        .await
        .unwrap(),
        ClaimOutcome::Revoked
    );
}
