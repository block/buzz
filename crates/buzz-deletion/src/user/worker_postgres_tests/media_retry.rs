use super::media_fixture::fixture;
use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn gate_then_media_requires_delete_and_retries_storage_failure_before_communities() {
    let fixture = fixture().await;
    let (store, id) = (&fixture.store, fixture.id);
    let read_only = RunOptions {
        delete: false,
        ..fixture.options()
    };
    let error = run(store, id, "worker-a", &read_only, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("real workflow execution requires --delete"));
    let paused = store.inspect(id).await.unwrap();
    assert_eq!(paused.stage_history.len(), 2);
    let gate = serde_json::to_value(&paused.stage_history[0]).unwrap();
    assert_eq!(paused.stage_history[0].stage, "gate_user");
    assert_eq!(paused.stage_history[0].status, "completed");
    assert_eq!(
        paused.stage_history[0].metadata_json["result"]["fixture"],
        true
    );
    let media_id = paused.stage_history[1].id;
    assert_eq!(paused.stage_history[1].stage, "purge_media");
    assert_eq!(paused.stage_history[1].status, "pending");
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.uploader_count().await, 1);

    fixture.fail_listing.store(true, Ordering::SeqCst);
    run(
        store,
        id,
        "worker-b",
        &fixture.options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    let failed = store.inspect(id).await.unwrap();
    assert_eq!(failed.request.status, "retry_wait");
    assert_eq!(failed.execution.retry_count, 1);
    assert!(failed.execution.lease_owner.is_none());
    assert_eq!(failed.stage_history.len(), 2);
    assert_eq!(failed.stage_history[1].id, media_id);
    assert_eq!(failed.stage_history[1].status, "failed");
    assert!(!fixture.deleted.load(Ordering::SeqCst));
    assert_eq!(fixture.uploader_count().await, 1);
    let calls = fixture.requests.load(Ordering::SeqCst);
    assert!(run(
        store,
        id,
        "worker-c",
        &fixture.options(),
        &CancellationToken::new()
    )
    .await
    .is_err());
    assert_eq!(fixture.requests.load(Ordering::SeqCst), calls);

    fixture.fail_listing.store(false, Ordering::SeqCst);
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&fixture.pool).await.unwrap();
    let one = RunOptions {
        next_step: true,
        ..fixture.options()
    };
    run(store, id, "worker-d", &one, &CancellationToken::new())
        .await
        .unwrap();
    let after = store.inspect(id).await.unwrap();
    assert_eq!(after.stage_history.len(), 3);
    assert_eq!(after.stage_history[1].id, media_id);
    assert_eq!(after.stage_history[1].status, "completed");
    assert_eq!(
        after.stage_history[1].started_at,
        failed.stage_history[1].started_at
    );
    assert_eq!(
        after.stage_history[1].metadata_json["result"]["deleted_versions"],
        1
    );
    assert_eq!(after.stage_history[2].stage, "purge_content");
    assert_eq!(after.stage_history[2].status, "pending");
    assert!(fixture.deleted.load(Ordering::SeqCst));
    assert_eq!(fixture.uploader_count().await, 0);
    assert_eq!(serde_json::to_value(&after.stage_history[0]).unwrap(), gate);
    let calls = fixture.requests.load(Ordering::SeqCst);
    // Restarting transition lookup returns content without replaying media.
    let lease = store
        .claim(id, "worker-e", DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(resumed) = UserDeletionTransition::new(store)
        .get_next_stage(&lease, None)
        .await
        .unwrap()
    else {
        panic!("expected content")
    };
    assert_eq!(resumed.id, after.stage_history[2].id);
    assert_eq!(fixture.requests.load(Ordering::SeqCst), calls);
    store.release(&lease).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_media_retry_refetches_agent_ownership_before_deleting() {
    let fixture = fixture().await;
    let human: String =
        sqlx::query_scalar("SELECT encode(pubkey, 'hex') FROM media_uploaders WHERE sha256 = $1")
            .bind("a".repeat(64))
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let bob = nostr::Keys::generate().public_key().to_hex();
    for key in [&human, &bob] {
        sqlx::query("INSERT INTO users(community_id, pubkey) VALUES ($1, decode($2, 'hex'))")
            .bind(Uuid::from_u128(11))
            .bind(key)
            .execute(&fixture.pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO users(community_id, pubkey, agent_owner_pubkey) VALUES ($1, decode($2, 'hex'), decode($3, 'hex'))")
        .bind(Uuid::from_u128(11)).bind(&agent).bind(&human).execute(&fixture.pool).await.unwrap();
    // Retain ownership elsewhere on retry: flattening agent keys would still
    // select the upload in community 11 after that community changes owner.
    sqlx::query("INSERT INTO users(community_id, pubkey) VALUES ($1, decode($2, 'hex'))")
        .bind(Uuid::from_u128(12))
        .bind(&human)
        .execute(&fixture.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(community_id, pubkey, agent_owner_pubkey) VALUES ($1, decode($2, 'hex'), decode($3, 'hex'))")
        .bind(Uuid::from_u128(12)).bind(&agent).bind(&human).execute(&fixture.pool).await.unwrap();
    sqlx::query("UPDATE media_uploaders SET pubkey = decode($1, 'hex') WHERE sha256 = $2")
        .bind(&agent)
        .bind("a".repeat(64))
        .execute(&fixture.pool)
        .await
        .unwrap();
    fixture.fail_listing.store(true, Ordering::SeqCst);
    run(
        &fixture.store,
        fixture.id,
        "worker-a",
        &fixture.options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    let failed = fixture.store.inspect(fixture.id).await.unwrap();
    assert_eq!(failed.stage_history[1].status, "failed");
    assert!(!fixture.deleted.load(Ordering::SeqCst));
    sqlx::query(
        "UPDATE users SET agent_owner_pubkey = decode($1, 'hex') WHERE pubkey = decode($2, 'hex') AND community_id = $3",
    )
    .bind(&bob)
    .bind(&agent)
    .bind(Uuid::from_u128(11))
    .execute(&fixture.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(fixture.id).execute(&fixture.pool).await.unwrap();
    fixture.fail_listing.store(false, Ordering::SeqCst);
    run(
        &fixture.store,
        fixture.id,
        "worker-b",
        &RunOptions {
            next_step: true,
            ..fixture.options()
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let after = fixture.store.inspect(fixture.id).await.unwrap();
    assert_eq!(after.stage_history[1].id, failed.stage_history[1].id);
    assert_eq!(after.stage_history[1].status, "completed");
    assert_eq!(
        after.stage_history[1].metadata_json["result"]["selected_files"],
        0
    );
    assert_eq!(after.stage_history[2].stage, "purge_content");
    assert!(!fixture.deleted.load(Ordering::SeqCst));
    assert_eq!(fixture.uploader_count().await, 1);
}
