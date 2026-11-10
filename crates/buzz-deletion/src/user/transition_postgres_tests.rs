use super::*;
use buzz_db::Db;
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;

// This fixture exposes the real stores and transition component. It contains
// no routing rules; each helper delegates to the production API.
struct TestStore {
    store: EntityDeletionStore,
    pool: PgPool,
}
impl std::ops::Deref for TestStore {
    type Target = EntityDeletionStore;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}
impl TestStore {
    async fn submit(
        &self,
        target: &str,
        operator: &str,
        reason: Option<&str>,
    ) -> Result<EntityRequest> {
        Db::from_pool(self.pool.clone())
            .user_deletion_store()
            .submit(target, operator, reason)
            .await
    }
    async fn get_next_stage(
        &self,
        lease: &EntityLease,
        previous: Option<Uuid>,
    ) -> Result<NextStage> {
        UserDeletionTransition::new(&self.store)
            .get_next_stage(lease, previous)
            .await
    }
}
fn inventory_digest(value: &Value) -> Result<String> {
    // Use the same public fingerprint operation as request preparation.
    buzz_db::entity_deletion::inventory_digest(value)
}

async fn fixture() -> (TestStore, Uuid) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    let store = TestStore {
        store: Db::from_pool(pool.clone()).entity_deletion_store(),
        pool,
    };
    let request = store
        .submit("demo:executor", "submitter", None)
        .await
        .unwrap();
    store
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    (store, request.id)
}

async fn claim(store: &TestStore, id: Uuid, owner: &str) -> EntityLease {
    store
        .claim(id, owner, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap()
}

async fn snapshot(store: &TestStore, id: Uuid) -> serde_json::Value {
    serde_json::to_value(store.inspect(id).await.unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_claimers_have_one_owner_and_unapproved_requests_are_not_claimed() {
    let (store, id) = fixture().await;
    let unapproved = store
        .submit("demo:unapproved", "submitter", None)
        .await
        .unwrap();
    assert!(store
        .claim(unapproved.id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
    let (a, b) = tokio::join!(
        store.claim(id, "worker-a", Duration::from_secs(60)),
        store.claim(id, "worker-b", Duration::from_secs(60)),
    );
    let claims: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(claims.len(), 1);
    let lease = &claims[0];
    let inspection = store.inspect(id).await.unwrap();
    assert_eq!(inspection.request.status, "approved");
    assert!(inspection.stage_history.is_empty());
    assert_eq!(
        inspection.execution.lease_owner.as_deref(),
        Some(lease.owner.as_str())
    );
    assert_eq!(inspection.execution.lease_generation, 1);
    assert!(store
        .claim(id, &lease.owner, Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
    let before = snapshot(&store, id).await;
    assert!(store.claim(id, "worker", Duration::ZERO).await.is_err());
    assert_eq!(snapshot(&store, id).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expired_and_superseded_workers_cannot_change_any_execution_state() {
    let (store, id) = fixture().await;
    let old = claim(&store, id, "worker-a").await;
    let stage = start(&store, &old).await.unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&store.pool).await.unwrap();
    // Expiration alone rejects renewal, even before another worker claims.
    assert!(buzz_db::deletion::is_stale_deletion_lease(
        &store
            .heartbeat(&old, Duration::from_secs(60))
            .await
            .unwrap_err()
    ));
    assert!(!store.release(&old).await.unwrap());
    // Reusing the same executor ID isolates the generation check from owner checks.
    let new = claim(&store, id, "worker-a").await;
    assert_eq!(new.owner, old.owner);
    assert_eq!(new.generation, old.generation + 1);
    let resumed = start(&store, &new).await.unwrap();
    assert_eq!(resumed.id, stage.id);
    assert_eq!(resumed.started_at, stage.started_at);
    let before = snapshot(&store, id).await;
    for error in [
        store.get_next_stage(&old, None).await.unwrap_err(),
        start(&store, &old).await.unwrap_err(),
        store
            .heartbeat(&old, Duration::from_secs(60))
            .await
            .unwrap_err(),
        store
            .fail_stage(&old, stage.id, "stale failure", Duration::from_secs(30))
            .await
            .unwrap_err(),
        store
            .save_stage_success(&old, stage.id, serde_json::json!({}))
            .await
            .unwrap_err(),
    ] {
        assert!(
            buzz_db::deletion::is_stale_deletion_lease(&error),
            "{error}"
        );
    }
    assert!(!store.release(&old).await.unwrap());
    assert_eq!(snapshot(&store, id).await, before);
    store
        .save_stage_success(&new, stage.id, serde_json::json!({"dummy": true}))
        .await
        .unwrap();
    let next = next(&store, &new, Some(stage.id)).await;
    assert_eq!(next.stage, "purge_db");
    assert_eq!(store.inspect(id).await.unwrap().stage_history.len(), 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn heartbeat_checks_authority_after_waiting_for_parent_lock() {
    let (store, id) = fixture().await;
    let lease = claim(&store, id, "worker").await;
    let before = store.inspect(id).await.unwrap().execution;
    store
        .heartbeat(&lease, Duration::from_secs(120))
        .await
        .unwrap();
    let renewed = store.inspect(id).await.unwrap().execution;
    assert!(renewed.lease_expires_at > before.lease_expires_at);
    assert!(renewed.heartbeat_at > before.heartbeat_at);

    let mut blocker = store.pool.begin().await.unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&mut *blocker).await.unwrap();
    let renewal = store.heartbeat(&lease, Duration::from_secs(60));
    tokio::pin!(renewal);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut renewal)
            .await
            .is_err()
    );
    blocker.commit().await.unwrap();
    assert!(buzz_db::deletion::is_stale_deletion_lease(
        &renewal.await.unwrap_err()
    ));
}

async fn next(store: &TestStore, lease: &EntityLease, previous: Option<Uuid>) -> EntityStage {
    match store.get_next_stage(lease, previous).await.unwrap() {
        NextStage::Stage(row) => row,
        other => panic!("expected a stage, got {other:?}"),
    }
}

async fn start(store: &TestStore, lease: &EntityLease) -> Result<EntityStage> {
    match store.get_next_stage(lease, None).await? {
        NextStage::Stage(row) => store.start_stage(lease, row.id).await,
        _ => panic!("expected current stage"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn transition_recovers_on_both_sides_of_commit_and_repeated_previous_id_is_read_only() {
    let (store, id) = fixture().await;
    let initial_claim = claim(&store, id, "worker-before-initialization").await;
    store.release(&initial_claim).await.unwrap();
    let first = claim(&store, id, "worker-a").await;
    sqlx::query("ALTER TABLE entity_deletion_stage_history ADD CONSTRAINT reject_initial CHECK (stage <> 'gate_user') NOT VALID")
        .execute(&store.pool).await.unwrap();
    let before = snapshot(&store, id).await;
    assert!(store.get_next_stage(&first, None).await.is_err());
    assert_eq!(snapshot(&store, id).await, before);
    assert_eq!(store.get(id).await.unwrap().status, "approved");
    sqlx::query("ALTER TABLE entity_deletion_stage_history DROP CONSTRAINT reject_initial")
        .execute(&store.pool)
        .await
        .unwrap();
    let gate = start(&store, &first).await.unwrap();
    store
        .save_stage_success(&first, gate.id, serde_json::json!({"dummy": true}))
        .await
        .unwrap();
    let saved = store.inspect(id).await.unwrap();
    assert_eq!(saved.stage_history.len(), 1);
    assert_eq!(saved.stage_history[0].status, "completed");
    assert!(saved.stage_history[0].is_current);
    let result_time = saved.stage_history[0].finished_at;
    store.release(&first).await.unwrap();

    // Restart before transition: None finds the completed current row and advances.
    let second = claim(&store, id, "worker-b").await;
    let db_stage = next(&store, &second, None).await;
    assert_eq!(db_stage.stage, "purge_db");
    assert_eq!(
        db_stage.metadata_json["previous_stage_id"],
        gate.id.to_string()
    );
    assert_eq!(db_stage.metadata_json["inputs"]["target"], "demo:executor");
    let before = snapshot(&store, id).await;
    assert_eq!(next(&store, &second, Some(gate.id)).await.id, db_stage.id);
    assert_eq!(snapshot(&store, id).await, before);
    store.release(&second).await.unwrap();

    // Restart after transition: None returns the saved successor, without replay.
    let third = claim(&store, id, "worker-c").await;
    assert_eq!(next(&store, &third, None).await.id, db_stage.id);
    let after = store.inspect(id).await.unwrap();
    assert_eq!(after.stage_history.len(), 2);
    assert_eq!(after.stage_history[0].finished_at, result_time);
    let before = snapshot(&store, id).await;
    assert!(store
        .get_next_stage(&third, Some(Uuid::new_v4()))
        .await
        .is_err());
    assert_eq!(snapshot(&store, id).await, before);
    assert!(store
        .save_stage_success(&third, gate.id, serde_json::json!({}))
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_transition_keeps_completed_current_row_and_retry_creates_only_one_successor() {
    let (store, id) = fixture().await;
    let lease = claim(&store, id, "worker").await;
    let gate = start(&store, &lease).await.unwrap();
    store
        .save_stage_success(&lease, gate.id, serde_json::json!({"dummy":true}))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE entity_deletion_stage_history ADD CONSTRAINT reject_successor CHECK (stage <> 'purge_db') NOT VALID")
        .execute(&store.pool).await.unwrap();
    let before = snapshot(&store, id).await;
    assert!(store.get_next_stage(&lease, Some(gate.id)).await.is_err());
    assert_eq!(snapshot(&store, id).await, before);
    sqlx::query("ALTER TABLE entity_deletion_stage_history DROP CONSTRAINT reject_successor")
        .execute(&store.pool)
        .await
        .unwrap();
    let successor = next(&store, &lease, Some(gate.id)).await;
    assert_eq!(next(&store, &lease, Some(gate.id)).await.id, successor.id);
    assert_eq!(store.inspect(id).await.unwrap().stage_history.len(), 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_stage_waits_then_resumes_same_row_without_advancing() {
    let (store, id) = fixture().await;
    let lease = claim(&store, id, "worker-a").await;
    let stage = start(&store, &lease).await.unwrap();
    store
        .fail_stage(&lease, stage.id, "injected", Duration::from_secs(30))
        .await
        .unwrap();
    assert!(matches!(
        store.get_next_stage(&lease, Some(stage.id)).await.unwrap(),
        NextStage::WaitUntil(_)
    ));
    let failed = store.inspect(id).await.unwrap();
    assert_eq!(failed.request.status, "retry_wait");
    assert_eq!(failed.execution.retry_count, 1);
    assert_eq!(failed.stage_history.len(), 1);
    assert!(failed.stage_history[0].is_current);
    assert_eq!(failed.stage_history[0].status, "failed");
    assert!(store.start_stage(&lease, stage.id).await.is_err());
    store.release(&lease).await.unwrap();
    assert!(store
        .claim(id, "worker-b", Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&store.pool).await.unwrap();
    let retry = claim(&store, id, "worker-b").await;
    let resumed = start(&store, &retry).await.unwrap();
    assert_eq!(resumed.id, stage.id);
    assert_eq!(resumed.started_at, stage.started_at);
    store
        .save_stage_success(&retry, stage.id, serde_json::json!({}))
        .await
        .unwrap();
    next(&store, &retry, Some(stage.id)).await;
    let after = store.inspect(id).await.unwrap();
    assert_eq!(after.execution.retry_count, 0);
    assert!(after.execution.last_error.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn routing_rebuilds_variable_community_progress_and_obeys_saved_branch_results() {
    let (store, _) = fixture().await;
    for count in [0, 1, 3] {
        let mut request = store
            .submit(&format!("demo:communities-{count}"), "submitter", None)
            .await
            .unwrap();
        let mut inventory = request.inventory_manifest.take().unwrap();
        let template = inventory["communities"][0].clone();
        inventory["communities"] = (0..count)
            .map(|i| {
                let mut row = template.clone();
                row["community_id"] = Uuid::from_u128(100 + i).to_string().into();
                row
            })
            .collect::<Vec<_>>()
            .into();
        let digest = inventory_digest(&inventory).unwrap();
        sqlx::query("UPDATE entity_deletion_requests SET inventory_manifest = $2, inventory_digest = $3 WHERE id = $1")
            .bind(request.id).bind(&inventory).bind(&digest).execute(&store.pool).await.unwrap();
        store
            .approve(request.id, "approver", &digest)
            .await
            .unwrap();
        let mut lease = claim(&store, request.id, "worker").await;
        let mut stages = Vec::new();
        let mut previous = None;
        for _ in 0..10 {
            let row = match store.get_next_stage(&lease, previous).await.unwrap() {
                NextStage::Finished => break,
                NextStage::Stage(row) => row,
                other => panic!("{other:?}"),
            };
            stages.push((row.stage.clone(), row.target_community_id));
            store.start_stage(&lease, row.id).await.unwrap();
            // Alternate paths: first community skips agents, the next executes them.
            let has_agents = row
                .target_community_id
                .is_some_and(|id| id.as_u128() % 2 == 1);
            store
                .save_stage_success(&lease, row.id, serde_json::json!({"has_agents":has_agents}))
                .await
                .unwrap();
            // Rebuild from PostgreSQL after every successful action, with no previous id.
            store.release(&lease).await.unwrap();
            lease = claim(&store, request.id, "replacement").await;
            previous = None;
        }
        assert_eq!(store.get(request.id).await.unwrap().status, "completed");
        assert_eq!(stages.len(), 1 + count as usize + (count / 2) as usize);
        assert_eq!(
            stages.iter().filter(|(s, _)| s == "purge_agents").count(),
            (count / 2) as usize
        );
        assert!(matches!(
            store.get_next_stage(&lease, None).await.unwrap(),
            NextStage::Finished
        ));
        assert!(!store
            .inspect(request.id)
            .await
            .unwrap()
            .stage_history
            .iter()
            .any(|s| s.is_current));
        store.release(&lease).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn corrupt_inventory_history_and_results_fail_without_inventing_progress() {
    let (store, id) = fixture().await;
    let lease = claim(&store, id, "worker").await;
    let gate = start(&store, &lease).await.unwrap();
    let manifest = store.get(id).await.unwrap().inventory_manifest.unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET inventory_manifest = '{}' WHERE id = $1")
        .bind(id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.get_next_stage(&lease, None).await.is_err());
    sqlx::query("UPDATE entity_deletion_requests SET inventory_manifest = $2 WHERE id = $1")
        .bind(id)
        .bind(manifest)
        .execute(&store.pool)
        .await
        .unwrap();
    store
        .save_stage_success(&lease, gate.id, serde_json::json!({}))
        .await
        .unwrap();
    let db = next(&store, &lease, Some(gate.id)).await;
    let saved_transition =
        store.inspect(id).await.unwrap().stage_history[0].metadata_json["transition"].clone();
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json = jsonb_set(metadata_json, '{transition}', $2) WHERE id = $1")
        .bind(gate.id).bind(serde_json::json!({"kind": "community_complete"}))
        .execute(&store.pool).await.unwrap();
    let corrupted = snapshot(&store, id).await;
    let error = store
        .get_next_stage(&lease, Some(gate.id))
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("saved transition disagrees with handler result"));
    assert_eq!(snapshot(&store, id).await, corrupted);
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json = jsonb_set(metadata_json, '{transition}', $2) WHERE id = $1")
        .bind(gate.id).bind(saved_transition).execute(&store.pool).await.unwrap();
    store.start_stage(&lease, db.id).await.unwrap();
    store
        .save_stage_success(&lease, db.id, serde_json::json!({}))
        .await
        .unwrap();
    let before = snapshot(&store, id).await;
    assert!(store.get_next_stage(&lease, Some(db.id)).await.is_err());
    assert_eq!(snapshot(&store, id).await, before); // missing has_agents cannot skip work
    sqlx::query("DELETE FROM entity_deletion_stage_history WHERE id = $1")
        .bind(db.id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.get_next_stage(&lease, None).await.is_err()); // never reinitialize gate
    assert_eq!(store.inspect(id).await.unwrap().stage_history.len(), 1);
    sqlx::query("DELETE FROM entity_deletion_approvals WHERE request_id = $1")
        .bind(id)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.get_next_stage(&lease, None).await.is_err());
}
