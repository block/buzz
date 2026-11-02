use super::*;
use chrono::{DateTime, Utc};

async fn fixture() -> (Db, CommunityId) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        crate::migration::run_migrations(&pool).await.unwrap();
    }
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community.as_uuid())
        .bind(format!("certification-{}.test", community.as_uuid()))
        .execute(&pool)
        .await
        .unwrap();
    (Db::from_pool(pool), community)
}

async fn begin(db: &Db, community: CommunityId) -> crate::AdmittedTx {
    db.begin_event_write_transaction(community).await.unwrap()
}

async fn state(db: &Db, community: CommunityId) -> (DateTime<Utc>, i64) {
    sqlx::query_as("SELECT artifact_policy_modified_at,(SELECT count(*) FROM artifact_sync_pending WHERE community_id=$1) FROM communities WHERE id=$1")
        .bind(community.as_uuid()).fetch_one(db.pool()).await.unwrap()
}

async fn age(tx: &mut crate::AdmittedTx, seconds: i32) {
    let community = tx.community();
    sqlx::query("UPDATE artifact_sync_pending SET earliest_at=clock_timestamp()-make_interval(secs=>$2) WHERE community_id=$1")
        .bind(community.as_uuid()).bind(seconds as f64).execute(tx.conn()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rollback_and_lost_commit_reply_leave_correct_recovery_work() {
    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    assert_eq!(state(&db, community).await, initial);
    drop(tx);
    assert_eq!(state(&db, community).await, initial);

    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, 121).await;
    assert_eq!(db.recover_artifact_certifications().await.unwrap(), 0);
    // The writer can lose its commit reply and never return to retry.
    tx.commit().await.unwrap();
    assert_eq!(state(&db, community).await, (initial.0, 1));
    assert_eq!(db.recover_artifact_certifications().await.unwrap(), 1);
    let completed = state(&db, community).await;
    assert!(completed.0 > initial.0);
    assert_eq!(completed.1, 0);
    assert_eq!(db.recover_artifact_certifications().await.unwrap(), 0);
    assert_eq!(state(&db, community).await, completed);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn visibility_within_overlap_preserves_the_policy_stamp() {
    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(certify(db.pool(), community, None).await.unwrap(), 1);
    assert_eq!(state(&db, community).await, initial);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn certification_crash_rolls_back_reset_and_completion_together() {
    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, 121).await;
    tx.commit().await.unwrap();

    let mut recovery = begin(&db, community).await;
    assert_eq!(certify_pending(&mut recovery, None).await.unwrap(), 1);
    assert_eq!(state(&db, community).await, (initial.0, 1));
    drop(recovery);
    let (a, b) = tokio::join!(
        certify(db.pool(), community, None),
        certify(db.pool(), community, None),
    );
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    let completed = state(&db, community).await;
    assert!(completed.0 > initial.0);
    assert_eq!(completed.1, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_reset_keeps_pending_work_and_retry_is_idempotent() {
    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, 121).await;
    tx.commit().await.unwrap();
    let operation: Uuid =
        sqlx::query_scalar("SELECT operation_id FROM artifact_sync_pending WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
    let mut blocker = db.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM communities WHERE id=$1 FOR NO KEY UPDATE")
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    assert!(certify(db.pool(), community, Some(operation))
        .await
        .is_err());
    assert_eq!(state(&db, community).await, (initial.0, 1));
    blocker.rollback().await.unwrap();
    assert_eq!(
        certify(db.pool(), community, Some(operation))
            .await
            .unwrap(),
        1
    );
    let completed = state(&db, community).await;
    assert_eq!(
        certify(db.pool(), community, Some(operation))
            .await
            .unwrap(),
        0
    );
    assert_eq!(state(&db, community).await, completed);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn recovery_handles_clock_reversal_and_leaves_deleting_communities() {
    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, -121).await;
    tx.commit().await.unwrap();
    assert_eq!(db.recover_artifact_certifications().await.unwrap(), 1);
    assert!(state(&db, community).await.0 > initial.0);

    let mut pending = begin(&db, community).await;
    pending.record_artifact_change().await.unwrap();
    pending.commit().await.unwrap();
    let mut deletion = db.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('buzz.deletion_executor_community',$1,true),set_config('buzz.deletion_fence_generation','0',true)")
        .bind(community.as_uuid().to_string()).execute(&mut *deletion).await.unwrap();
    sqlx::query("UPDATE communities SET deletion_state='quiescing' WHERE id=$1")
        .bind(community.as_uuid())
        .execute(&mut *deletion)
        .await
        .unwrap();
    deletion.commit().await.unwrap();
    assert!(db.begin_event_write_transaction(community).await.is_err());
    assert_eq!(db.recover_artifact_certifications().await.unwrap(), 0);
    assert_eq!(state(&db, community).await.1, 1);
}
