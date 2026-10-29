use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn actor_leases_exclude_fencing_until_release_and_do_not_block_other_users() {
    let f = Fixture::new().await;
    let user = Keys::generate();
    let other = Keys::generate();
    let store = f.db.deletion_store();
    let mut write = store
        .acquire_actor_serving_write_lease(
            f.community,
            "upload-test",
            "writer",
            Duration::from_secs(60),
            user.public_key(),
        )
        .await
        .unwrap();
    let mut gate = f.pool.begin().await.unwrap();
    assert!(
        matches!(fences::insert(&mut gate, FenceIdentity::user(user.public_key()), &f.lease, None).await,
        Err(DbError::AccessDenied(reason)) if reason.contains("still in flight"))
    );
    gate.rollback().await.unwrap();
    store
        .renew_serving_write_lease(&mut write, Duration::from_secs(60))
        .await
        .unwrap();
    store.verify_serving_write_lease(&write).await.unwrap();
    assert!(store.release_serving_write_lease(&write).await.unwrap());
    f.fence(&user, None).await;
    assert!(matches!(
        store
            .acquire_actor_serving_write_lease(
                f.community,
                "upload-test",
                "writer",
                Duration::from_secs(60),
                user.public_key(),
            )
            .await,
        Err(DbError::UserDeletionRestricted)
    ));
    let control = store
        .acquire_actor_serving_write_lease(
            f.community,
            "upload-test",
            "writer",
            Duration::from_secs(60),
            other.public_key(),
        )
        .await
        .unwrap();
    store.release_serving_write_lease(&control).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expired_actor_lease_cannot_renew_or_finalize_after_fencing() {
    let f = Fixture::new().await;
    let user = Keys::generate();
    let store = f.db.deletion_store();
    let mut write = store
        .acquire_actor_serving_write_lease(
            f.community,
            "git-test",
            "writer",
            Duration::from_secs(60),
            user.public_key(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE community_serving_write_leases SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1")
        .bind(write.id).execute(&f.pool).await.unwrap();
    f.fence(&user, None).await;
    assert!(store
        .renew_serving_write_lease(&mut write, Duration::from_secs(60))
        .await
        .is_err());
    assert!(store.verify_serving_write_lease(&write).await.is_err());
    let mut tx = f.pool.begin().await.unwrap();
    assert!(store
        .guard_transaction_with_serving_lease(&mut tx, &write)
        .await
        .is_err());
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scoped_agent_lease_and_fence_leave_another_community_writable() {
    let f = Fixture::new().await;
    let owner = Keys::generate();
    let agent = f.agent(&owner).await;
    let other =
        f.db.ensure_configured_community(&format!("actor-lease-{}.test", Uuid::new_v4()))
            .await
            .unwrap()
            .id;
    let store = f.db.deletion_store();
    let write = store
        .acquire_actor_serving_write_lease(
            other,
            "upload-test",
            "writer",
            Duration::from_secs(60),
            agent.public_key(),
        )
        .await
        .unwrap();
    f.fence(&agent, Some(&owner)).await;
    store.verify_serving_write_lease(&write).await.unwrap();
    assert!(matches!(
        store
            .acquire_actor_serving_write_lease(
                f.community,
                "upload-test",
                "writer",
                Duration::from_secs(60),
                agent.public_key(),
            )
            .await,
        Err(DbError::UserDeletionRestricted)
    ));
    store.release_serving_write_lease(&write).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_claims_cannot_create_agents_for_a_fenced_owner() {
    let f = Fixture::new().await;
    let owner = Keys::generate();
    let relay = Keys::generate();
    let invite = crate::relay_invite::mint_relay_invite(
        &f.pool,
        f.community,
        &relay.public_key().to_hex(),
        3600,
        Some(10),
    )
    .await
    .unwrap();
    let hash = buzz_core::invite::hash_v2_code(&invite.code);
    for fenced in [false, true] {
        if fenced {
            f.fence(&owner, None).await;
        }
        for legacy in [false, true] {
            let agent = Keys::generate();
            let result = if legacy {
                crate::relay_invite::claim_relay_membership_with_owner(
                    &f.pool,
                    f.community,
                    &agent.public_key().to_hex(),
                    "member",
                    None,
                    Some(&owner.public_key().to_hex()),
                    &relay,
                )
                .await
                .map(|write| write.outcome.owner_link_created)
            } else {
                crate::relay_invite::claim_relay_invite_with_owner(
                    &f.pool,
                    f.community,
                    &hash,
                    &agent.public_key().to_hex(),
                    None,
                    Some(&owner.public_key().to_hex()),
                    &relay,
                )
                .await
                .map(|write| write.outcome.owner_link_created)
            };
            if fenced {
                assert!(
                    matches!(result, Err(DbError::UserDeletionRestricted)),
                    "{result:?}"
                );
                let count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM users WHERE community_id=$1 AND pubkey=$2",
                )
                .bind(f.community.as_uuid())
                .bind(agent.public_key().as_bytes())
                .fetch_one(&f.pool)
                .await
                .unwrap();
                assert_eq!(count, 0, "refused claim must not create the agent");
            } else {
                assert!(result.unwrap(), "baseline must actually link ownership");
            }
            assert_eq!(
                crate::relay_members::is_relay_member(
                    &f.pool,
                    f.community,
                    &agent.public_key().to_hex(),
                )
                .await
                .unwrap(),
                !fenced,
            );
        }
    }
    let used: i32 =
        sqlx::query_scalar("SELECT use_count FROM relay_invites WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(invite.invite_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        used, 1,
        "refused claim must not consume another invitation use"
    );
}
