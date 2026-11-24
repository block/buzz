use super::*;
use crate::Db;

async fn fixture() -> (Db, CommunityId) {
    let db = Db::from_pool(
        sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap(),
    );
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("{}.example.org", community.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    (db, community)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn upload_activity_tracks_owners_expiry_and_legacy_without_blocking_other_users() {
    let (db, community) = fixture().await;
    let alice = nostr::Keys::generate().public_key();
    let bob = nostr::Keys::generate().public_key();
    let agent = nostr::Keys::generate().public_key();
    let store = db.deletion_store();
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES($1,$2,$3)")
        .bind(community.as_uuid())
        .bind(agent.as_bytes().as_slice())
        .bind(alice.as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    let mut a = store
        .acquire_media_upload_lease(
            community,
            agent.as_bytes(),
            None,
            "replica-a",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    let b = store
        .acquire_media_upload_lease(
            community,
            bob.as_bytes(),
            None,
            "replica-b",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .user_media_upload_activity(&alice.to_hex())
            .await
            .unwrap()[0]
            .active,
        1
    );
    assert!(store.release_serving_write_lease(&b).await.unwrap());
    assert!(store
        .user_media_upload_activity(&bob.to_hex())
        .await
        .unwrap()
        .is_empty());
    // Ownership transfer does not erase the original owner's outstanding work.
    sqlx::query("UPDATE users SET agent_owner_pubkey=$1 WHERE community_id=$2 AND pubkey=$3")
        .bind(bob.as_bytes().as_slice())
        .bind(community.as_uuid())
        .bind(agent.as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .user_media_upload_activity(&alice.to_hex())
            .await
            .unwrap()[0]
            .active,
        1
    );
    assert_eq!(
        store
            .user_media_upload_activity(&bob.to_hex())
            .await
            .unwrap()[0]
            .active,
        1
    );
    store.begin_media_upload_write(&a).await.unwrap();
    store
        .renew_serving_write_lease(&mut a, Duration::from_secs(60))
        .await
        .unwrap();
    store.verify_serving_write_lease(&a).await.unwrap();
    let retained: bool = sqlx::query_scalar("SELECT lease_until='infinity'::timestamptz FROM community_serving_write_leases WHERE id=$1")
        .bind(a.id).fetch_one(db.pool()).await.unwrap();
    assert!(
        retained,
        "renewal must preserve compatibility with legacy reapers"
    );
    sqlx::query("UPDATE community_serving_write_leases SET media_lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(a.id).execute(db.pool()).await.unwrap();
    assert!(store.begin_media_upload_write(&a).await.is_err());
    assert!(store.verify_serving_write_lease(&a).await.is_err());
    assert!(store
        .renew_serving_write_lease(&mut a, Duration::from_secs(60))
        .await
        .is_err());
    // Execute the previous release's reaper verbatim: it must retain armed uploads too.
    sqlx::query("DELETE FROM community_serving_write_leases WHERE lease_until < now()")
        .execute(db.pool())
        .await
        .unwrap();
    store.reap_expired_serving_write_leases(100).await.unwrap();
    assert_eq!(
        store
            .user_media_upload_activity(&alice.to_hex())
            .await
            .unwrap()[0]
            .unresolved,
        1
    );
    assert!(!store.serving_writes_drained(community).await.unwrap());
    assert!(store.release_serving_write_lease(&a).await.unwrap());
    assert!(store
        .user_media_upload_activity(&alice.to_hex())
        .await
        .unwrap()
        .is_empty());
    let legacy = store
        .acquire_serving_write_lease(
            community,
            "media_upload",
            "old-replica",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .user_media_upload_activity(&alice.to_hex())
            .await
            .unwrap()[0]
            .unattributed,
        1
    );
    store.release_serving_write_lease(&legacy).await.unwrap();
    // Other operation types are not media drain blockers.
    store
        .acquire_serving_write_lease(community, "git_publish", "replica", Duration::from_secs(60))
        .await
        .unwrap();
    assert!(store
        .user_media_upload_activity(&alice.to_hex())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn upload_owner_is_community_scoped_and_verified_assertion_owner_survives_missing_profile() {
    let (db, a) = fixture().await;
    let b = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(b.as_uuid())
        .bind(format!("{}.example.org", b.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    let alice = nostr::Keys::generate().public_key();
    let bob = nostr::Keys::generate().public_key();
    let agent = nostr::Keys::generate().public_key();
    for (community, owner) in [(a, alice), (b, bob)] {
        sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES($1,$2,$3)")
            .bind(community.as_uuid())
            .bind(agent.as_bytes().as_slice())
            .bind(owner.as_bytes().as_slice())
            .execute(db.pool())
            .await
            .unwrap();
    }
    let store = db.deletion_store();
    let lease = store
        .acquire_media_upload_lease(
            b,
            agent.as_bytes(),
            None,
            "replica",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(store
        .user_media_upload_activity(&alice.to_hex())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .user_media_upload_activity(&bob.to_hex())
            .await
            .unwrap()[0]
            .active,
        1
    );
    store.release_serving_write_lease(&lease).await.unwrap();
    sqlx::query("DELETE FROM users WHERE pubkey=$1")
        .bind(agent.as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    store
        .acquire_media_upload_lease(
            a,
            agent.as_bytes(),
            Some(alice.as_bytes()),
            "replica",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .user_media_upload_activity(&alice.to_hex())
            .await
            .unwrap()[0]
            .active,
        1
    );
    sqlx::query("UPDATE communities SET deletion_state='quiescing' WHERE id=$1")
        .bind(a.as_uuid())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(store
        .acquire_media_upload_lease(
            a,
            alice.as_bytes(),
            None,
            "replica",
            Duration::from_secs(60)
        )
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_upload_attribution_preserves_live_legacy_leases() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    crate::migration::run_migrations_through(&pool, 105)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO community_serving_write_leases(id,community_id,operation,owner,fence_generation,lease_until) VALUES($1,$2,'media_upload','old-replica',0,now()+interval '1 minute')")
        .bind(id).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    crate::migration::run_migrations(&pool).await.unwrap();
    let actor: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT actor_pubkey FROM community_serving_write_leases WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(actor.is_none());
}
