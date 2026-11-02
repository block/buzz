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
    age(&mut tx, 121).await;
    tx.record_artifact_change().await.unwrap();
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

#[tokio::test]
#[ignore = "requires Postgres"]
async fn blocked_community_does_not_exclude_other_pending_work() {
    let (db, a) = fixture().await;
    let (_, b) = fixture().await;
    let mut communities = [a, b];
    communities.sort_by_key(|c| *c.as_uuid());
    for community in communities {
        let mut tx = begin(&db, community).await;
        sqlx::query("INSERT INTO artifact_sync_pending(community_id,operation_id,earliest_at) SELECT $1,gen_random_uuid(),clock_timestamp()-interval '121 seconds' FROM generate_series(1,101)")
            .bind(community.as_uuid()).execute(tx.conn()).await.unwrap();
        tx.commit().await.unwrap();
    }
    let mut blocker = db.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM communities WHERE id=$1 FOR NO KEY UPDATE")
        .bind(communities[0].as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let (completed, after) = db
        .recover_artifact_certifications_after(None)
        .await
        .unwrap();
    assert_eq!(completed, 0);
    assert_eq!(after, Some(*communities[0].as_uuid()));
    let (completed, after) = db
        .recover_artifact_certifications_after(after)
        .await
        .unwrap();
    assert_eq!(completed, 100);
    assert_eq!(after, Some(*communities[1].as_uuid()));
    assert_eq!(state(&db, communities[0]).await.1, 101);
    assert_eq!(state(&db, communities[1]).await.1, 1);
    blocker.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn certification_closes_a_connection_after_a_blocked_reset() {
    let (db, community) = fixture().await;
    let mut tx = begin(&db, community).await;
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, 121).await;
    tx.commit().await.unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut blocker = db.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM communities WHERE id=$1 FOR NO KEY UPDATE")
        .bind(community.as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let start = tokio::time::Instant::now();
    assert!(certify(&pool, community, None).await.is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    let next: i32 = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&pool),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(pid, next);
    assert_eq!(state(&db, community).await.1, 1);
    blocker.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn lost_commit_acknowledgement_recovers_the_committed_operation() {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    let (db, community) = fixture().await;
    let initial = state(&db, community).await;
    let options: PgConnectOptions = crate::test_support::database_url().parse().unwrap();
    let upstream = (options.get_host().to_owned(), options.get_port());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let server = TcpStream::connect(upstream).await.unwrap();
        let (mut client_read, mut client_write) = client.into_split();
        let (mut server_read, mut server_write) = server.into_split();
        let forward =
            tokio::spawn(async move { tokio::io::copy(&mut client_read, &mut server_write).await });
        loop {
            let mut header = [0u8; 5];
            server_read.read_exact(&mut header).await.unwrap();
            let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
            assert!((4..=1_048_576).contains(&length));
            let mut body = vec![0; length - 4];
            server_read.read_exact(&mut body).await.unwrap();
            if header[0] == b'C' && body == b"COMMIT\0" {
                forward.abort();
                let _ = forward.await;
                return;
            }
            client_write.write_all(&header).await.unwrap();
            client_write.write_all(&body).await.unwrap();
        }
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            options
                .host("127.0.0.1")
                .port(port)
                .ssl_mode(PgSslMode::Disable),
        )
        .await
        .unwrap();
    let writer = Db::from_pool(pool);
    let mut tx = begin(&writer, community).await;
    tx.record_artifact_change().await.unwrap();
    age(&mut tx, 121).await;
    let operation: Uuid =
        sqlx::query_scalar("SELECT operation_id FROM artifact_sync_pending WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_one(tx.conn())
            .await
            .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), tx.commit())
            .await
            .unwrap()
            .is_err()
    );
    proxy.await.unwrap();
    assert_eq!(state(&db, community).await, (initial.0, 1));
    assert_eq!(
        certify(db.pool(), community, Some(operation))
            .await
            .unwrap(),
        1
    );
    let completed = state(&db, community).await;
    assert!(completed.0 > initial.0);
    assert_eq!(completed.1, 0);
    assert_eq!(
        certify(db.pool(), community, Some(operation))
            .await
            .unwrap(),
        0
    );
    assert_eq!(state(&db, community).await, completed);
}
