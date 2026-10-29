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
