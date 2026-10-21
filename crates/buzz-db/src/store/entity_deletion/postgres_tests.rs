use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;

use super::*;

const LEASE: Duration = Duration::from_secs(120);

async fn store() -> EntityDeletionStore {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect isolated DB");
    EntityDeletionStore { pool }
}

async fn approved(store: &EntityDeletionStore) -> EntityRequest {
    let target = format!("demo:{}", Uuid::new_v4().simple());
    let request = store
        .submit(&target, "operator-a", None)
        .await
        .expect("submit");
    store
        .prepare_inventory(request.id)
        .await
        .expect("inventory");
    store
        .approve(request.id, "operator-b")
        .await
        .expect("approve")
}

async fn claim(store: &EntityDeletionStore, id: Uuid, owner: &str) -> EntityClaim {
    store
        .claim(Some(id), owner, LEASE)
        .await
        .expect("claim")
        .expect("eligible")
}

async fn due(store: &EntityDeletionStore, id: Uuid) {
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = now() WHERE id = $1")
        .bind(id)
        .execute(&store.pool)
        .await
        .expect("make retry due");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_approval_and_completed_history_are_immutable() {
    let store = store().await;
    assert!(store.submit("real-user", "operator-a", None).await.is_err());
    let request = store
        .submit("demo:alice", "operator-a", None)
        .await
        .expect("submit");
    assert_eq!(
        store
            .submit("demo:alice", "operator-a", None)
            .await
            .expect("repeat")
            .id,
        request.id
    );
    assert!(store
        .submit("demo:alice", "someone-else", None)
        .await
        .is_err());
    assert!(store
        .claim(Some(request.id), "worker", LEASE)
        .await
        .expect("unapproved")
        .is_none());
    assert!(store.approve(request.id, "operator-b").await.is_err());
    store
        .prepare_inventory(request.id)
        .await
        .expect("inventory");
    assert!(sqlx::query(
        "INSERT INTO entity_deletion_approvals (request_id, inventory_digest, approved_by)
         SELECT id, inventory_digest, requested_by FROM entity_deletion_requests WHERE id = $1",
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "UPDATE entity_deletion_requests SET target = 'demo:retargeted' WHERE id = $1"
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "UPDATE entity_deletion_requests SET requested_by = 'replacement' WHERE id = $1"
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(store.approve(request.id, "operator-a").await.is_err());
    store
        .approve(request.id, "operator-b")
        .await
        .expect("approve");
    store
        .approve(request.id, "operator-b")
        .await
        .expect("idempotent approve");
    assert_eq!(
        store
            .inspect(request.id)
            .await
            .expect("inspect")
            .stage_history
            .len(),
        1
    );
    assert!(sqlx::query(
        "UPDATE entity_deletion_requests SET inventory_manifest = '{}' WHERE id = $1"
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(
        sqlx::query("DELETE FROM entity_deletion_approvals WHERE request_id = $1")
            .bind(request.id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO entity_deletion_stage_history (request_id, sequence, stage) VALUES ($1, 2, 'drain')")
        .bind(request.id).execute(&store.pool).await.is_err());
    let claim = claim(&store, request.id, "worker").await;
    let stage = store.start_stage(&claim.lease).await.expect("start");
    assert!(sqlx::query(
        "UPDATE entity_deletion_stage_history SET stage = 'delete_data' WHERE id = $1"
    )
    .bind(stage.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(sqlx::query("UPDATE entity_deletion_stage_history SET started_at = now() + interval '1 second' WHERE id = $1")
        .bind(stage.id).execute(&store.pool).await.is_err());
    store
        .complete_stage(&claim.lease, stage.id, &json!({"simulated": true}))
        .await
        .expect("complete");
    assert!(sqlx::query(
        "UPDATE entity_deletion_stage_history SET metadata_json = '{}' WHERE id = $1"
    )
    .bind(stage.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(
        sqlx::query("DELETE FROM entity_deletion_stage_history WHERE id = $1")
            .bind(stage.id)
            .execute(&store.pool)
            .await
            .is_err()
    );
    let mut inventory = claim.request.inventory().expect("fixture");
    inventory.communities[0].relationship = "owner".into();
    assert!(
        inventory.validate().is_err(),
        "owned-community fixtures are unsupported"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn claim_generation_guards_children_heartbeat_and_release() {
    let store = store().await;
    let request = approved(&store).await;
    let (a, b) = tokio::join!(
        store.claim(Some(request.id), "a", LEASE),
        store.claim(Some(request.id), "b", LEASE),
    );
    let a = a.expect("claim a");
    let b = b.expect("claim b");
    assert_ne!(a.is_some(), b.is_some());
    let first = a.or(b).expect("one claim");
    let stage = store.start_stage(&first.lease).await.expect("start");
    store
        .heartbeat(&first.lease, "run", LEASE)
        .await
        .expect("heartbeat");
    let heartbeat: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entity_deletion_executor_heartbeats WHERE executor_id = $1",
    )
    .bind(&first.lease.owner)
    .fetch_one(&store.pool)
    .await
    .expect("heartbeat row");
    assert_eq!(heartbeat, 1);
    assert_eq!(
        store
            .get(request.id)
            .await
            .expect("request")
            .lease_generation,
        first.lease.generation
    );
    sqlx::query("UPDATE entity_deletion_requests SET lease_until = now() - interval '1 second' WHERE id = $1")
        .bind(request.id).execute(&store.pool).await.expect("expire");
    assert!(is_stale_lease(
        &store
            .heartbeat(&first.lease, "run", LEASE)
            .await
            .expect_err("expired")
    ));
    let second = claim(&store, request.id, "successor").await;
    assert_eq!(second.lease.generation, first.lease.generation + 1);
    assert!(is_stale_lease(
        &store
            .complete_stage(&first.lease, stage.id, &json!({}))
            .await
            .expect_err("stale child write")
    ));
    assert!(is_stale_lease(
        &store
            .start_stage(&first.lease)
            .await
            .expect_err("stale start")
    ));
    store
        .stop_executor(Some(&first.lease), &first.lease.owner)
        .await
        .expect("stale release");
    assert_eq!(
        store
            .get(request.id)
            .await
            .expect("request")
            .lease_owner
            .as_deref(),
        Some("successor")
    );
    let resumed = store.start_stage(&second.lease).await.expect("resume");
    assert_eq!(resumed.id, stage.id);
    assert_eq!(resumed.started_at, stage.started_at);
    store
        .complete_stage(&second.lease, stage.id, &json!({}))
        .await
        .expect("successor advances");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stage_and_successor_creation_roll_back_together() {
    let store = store().await;
    let request = approved(&store).await;
    let claim = claim(&store, request.id, "worker").await;
    let stage = store.start_stage(&claim.lease).await.expect("start");
    sqlx::raw_sql(
        "CREATE FUNCTION reject_demo_successor() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected successor insert failure'; END $$;
         CREATE TRIGGER reject_demo_successor BEFORE INSERT ON entity_deletion_stage_history
         FOR EACH ROW EXECUTE FUNCTION reject_demo_successor();",
    )
    .execute(&store.pool)
    .await
    .expect("inject next-row failure");
    assert!(store
        .complete_stage(&claim.lease, stage.id, &json!({"result": "done"}))
        .await
        .is_err());
    let inspection = store.inspect(request.id).await.expect("inspect rollback");
    assert_eq!(inspection.stage_history.len(), 1);
    let current = &inspection.stage_history[0];
    assert_eq!(current.status, "running");
    assert!(current.is_current);
    assert!(current.finished_at.is_none());
    assert_eq!(current.metadata_json, json!({}));
    sqlx::raw_sql("DROP TRIGGER reject_demo_successor ON entity_deletion_stage_history; DROP FUNCTION reject_demo_successor();")
        .execute(&store.pool).await.expect("remove fault");
    store
        .complete_stage(&claim.lease, stage.id, &json!({"result": "done"}))
        .await
        .expect("advance");
    let inspection = store.inspect(request.id).await.expect("inspect");
    assert_eq!(inspection.stage_history.len(), 2);
    assert_eq!(
        inspection
            .stage_history
            .iter()
            .filter(|s| s.is_current)
            .count(),
        1
    );
    assert_eq!(inspection.stage_history[1].sequence, 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_b_failure_resumes_without_repeating_a_and_retries_are_bounded() {
    let store = store().await;
    let request = approved(&store).await;
    let first = claim(&store, request.id, "worker-1").await;
    for sequence in 1..9 {
        let stage = store.start_stage(&first.lease).await.expect("start");
        assert_eq!(stage.sequence, sequence);
        store
            .complete_stage(&first.lease, stage.id, &json!({"sequence": sequence}))
            .await
            .expect("advance");
    }
    let failed = store.start_stage(&first.lease).await.expect("B delete");
    assert_eq!(failed.target_community_id, Some(Uuid::from_u128(2)));
    assert_eq!(failed.stage, "delete_data");
    store
        .record_failure(
            &first.lease,
            failed.id,
            "temporary",
            false,
            Duration::from_secs(30),
        )
        .await
        .expect("fail");
    assert!(store
        .claim(Some(request.id), "too-early", LEASE)
        .await
        .expect("claim delayed")
        .is_none());
    let snapshot = store.inspect(request.id).await.expect("inspect failed");
    assert_eq!(
        snapshot
            .stage_history
            .iter()
            .filter(|s| s.status == "completed")
            .count(),
        8
    );
    assert_eq!(
        snapshot.stage_history.last().expect("B stage").status,
        "failed"
    );
    assert!(snapshot.request.lease_owner.is_none());
    for attempt in 2..=5 {
        due(&store, request.id).await;
        let retry = claim(&store, request.id, &format!("worker-{attempt}")).await;
        let stage = store
            .start_stage(&retry.lease)
            .await
            .expect("retry same step");
        assert_eq!(stage.id, failed.id);
        assert_eq!(stage.started_at, failed.started_at);
        store
            .record_failure(
                &retry.lease,
                stage.id,
                "temporary",
                false,
                Duration::from_secs(30),
            )
            .await
            .expect("fail retry");
    }
    assert!(store
        .get(request.id)
        .await
        .expect("blocked")
        .blocked_at
        .is_some());
    due(&store, request.id).await;
    assert!(store
        .claim(Some(request.id), "blocked", LEASE)
        .await
        .expect("blocked claim")
        .is_none());
    store
        .unblock(request.id, "operator-c", "dependency repaired")
        .await
        .expect("unblock");
    let retry = claim(&store, request.id, "recovered").await;
    for sequence in 9..=15 {
        let stage = store.start_stage(&retry.lease).await.expect("resume");
        assert_eq!(stage.sequence, sequence);
        store
            .complete_stage(&retry.lease, stage.id, &json!({"simulated": true}))
            .await
            .expect("complete");
    }
    let finished = store.inspect(request.id).await.expect("finished");
    assert_eq!(finished.request.status, "demo_completed");
    assert_eq!(finished.request.retry_count, 0);
    assert!(finished.request.lease_owner.is_none());
    assert!(finished
        .stage_history
        .iter()
        .all(|s| !s.is_current && s.status == "completed"));
    assert_eq!(finished.stage_history.len(), 15);
    for (before, after) in snapshot.stage_history[..8]
        .iter()
        .zip(&finished.stage_history[..8])
    {
        assert_eq!(
            serde_json::to_value(before).expect("before"),
            serde_json::to_value(after).expect("after")
        );
    }
    assert_eq!(
        finished.stage_history[8].last_error.as_deref(),
        Some("temporary")
    );
    assert_eq!(
        finished.stage_history[8].metadata_json["last_unblock"]["by"],
        "operator-c"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_schema_classifies_workflow_tables_without_community_fences() {
    let store = store().await;
    let names = [
        "entity_deletion_requests",
        "entity_deletion_stage_history",
        "entity_deletion_approvals",
        "entity_deletion_executor_heartbeats",
    ];
    let classified: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _operator_global_tables WHERE table_name = ANY($1)",
    )
    .bind(names.as_slice())
    .fetch_one(&store.pool)
    .await
    .expect("classifications");
    assert_eq!(classified, 4);
    let fences: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
         WHERE c.relname = ANY($1) AND t.tgname LIKE 'community_write_fence_%'",
    )
    .bind(names.as_slice())
    .fetch_one(&store.pool)
    .await
    .expect("fences");
    assert_eq!(fences, 0);
    let checks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE conname IN
         ('entity_deletion_request_shape', 'entity_deletion_stage_shape') AND convalidated",
    )
    .fetch_one(&store.pool)
    .await
    .expect("shape checks");
    assert_eq!(checks, 2);
    let request = approved(&store).await;
    assert!(sqlx::query(
        "UPDATE entity_deletion_requests SET lease_owner = 'partial-lease' WHERE id = $1"
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "UPDATE entity_deletion_stage_history SET status = 'completed' WHERE request_id = $1"
    )
    .bind(request.id)
    .execute(&store.pool)
    .await
    .is_err());
    // Adding operator state must not break the existing destructive catalog.
    crate::Db::from_pool(store.pool.clone())
        .validate_deletion_catalog()
        .await
        .expect("community catalog");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_entity_workflow_upgrade_preserves_existing_data() {
    let store = store().await;
    crate::migration::run_migrations_through(&store.pool, 66)
        .await
        .expect("previous schema");
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'upgrade-fixture.invalid')")
        .bind(id)
        .execute(&store.pool)
        .await
        .expect("existing community");
    crate::migration::run_migrations(&store.pool)
        .await
        .expect("upgrade");
    let request = approved(&store).await;
    crate::migration::run_migrations(&store.pool)
        .await
        .expect("migration rerun");
    assert_eq!(
        store
            .inspect(request.id)
            .await
            .expect("preserved request")
            .stage_history
            .len(),
        1
    );
    let host: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .expect("preserved community");
    assert_eq!(host, "upgrade-fixture.invalid");
    let claimed = claim(&store, request.id, "migration-worker").await;
    let stage = store.start_stage(&claimed.lease).await.expect("start");
    store
        .complete_stage(&claimed.lease, stage.id, &json!({}))
        .await
        .expect("advance");
}
