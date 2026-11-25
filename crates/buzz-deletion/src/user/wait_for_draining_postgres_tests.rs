use super::*;
use buzz_core::CommunityId;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn drain_refuses_active_and_abandoned_uploads_then_allows_completed_uploads() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let db = Db::from_pool(sqlx::PgPool::connect(&url).await.unwrap());
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("{}.example.org", community.as_uuid()))
        .execute(db.pool())
        .await
        .unwrap();
    let alice = nostr::Keys::generate().public_key();
    let bob = nostr::Keys::generate().public_key();
    assert!(handle(&db, &alice.to_hex(), false).await.is_err());
    let a = crate::acquire_media_upload(&db, community, &alice, None)
        .await
        .unwrap();
    let b = crate::acquire_media_upload(&db, community, &bob, None)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        handle(&db, &alice.to_hex(), true),
    )
    .await
    .unwrap()
    .unwrap_err()
    .to_string();
    assert!(error.contains("active=1"), "{error}");
    a.finish().await.unwrap();
    assert!(handle(&db, &alice.to_hex(), true).await.unwrap().drained);
    b.finish().await.unwrap();
    let abandoned = crate::acquire_media_upload(&db, community, &alice, None)
        .await
        .unwrap();
    let token = abandoned.lease().clone();
    db.deletion_store()
        .begin_media_upload_write(&token)
        .await
        .unwrap();
    drop(abandoned);
    sqlx::query("UPDATE community_serving_write_leases SET media_lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(token.id).execute(db.pool()).await.unwrap();
    db.deletion_store()
        .reap_expired_serving_write_leases(100)
        .await
        .unwrap();
    let report = inspect(&db, &alice.to_hex(), true).await.unwrap();
    assert!(!report.drained);
    assert_eq!(report.activity[0].unresolved, 1);
    // Simulate an independently confirmed completion, using the exact lease token.
    db.deletion_store()
        .release_serving_write_lease(&token)
        .await
        .unwrap();
    assert!(handle(&db, &alice.to_hex(), true).await.unwrap().drained);
}
