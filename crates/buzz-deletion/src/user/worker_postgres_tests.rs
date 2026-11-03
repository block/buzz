use super::*;
use buzz_db::Db;

async fn fixture() -> (EntityDeletionStore, Uuid, sqlx::PgPool) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let store = db.entity_deletion_store();
    let request = db
        .user_deletion_store()
        .submit("demo:runner", "submitter", None)
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
    (
        store,
        request.id,
        sqlx::PgPool::connect(&url).await.unwrap(),
    )
}

fn options() -> RunOptions {
    RunOptions {
        next_step: false,
        fail_stage: None,
        stage_delay: Duration::ZERO,
        no_agents_in: Vec::new(),
    }
}

async fn assert_completed(store: &EntityDeletionStore, id: Uuid) {
    let inspection = store.inspect(id).await.unwrap();
    assert_eq!(inspection.request.status, "completed");
    assert!(inspection.execution.completed_at.is_some());
    assert!(inspection.execution.lease_owner.is_none());
    assert_eq!(inspection.approvals.len(), 1);
    let actual: Vec<_> = inspection
        .stage_history
        .iter()
        .map(|s| {
            (
                s.stage.as_str(),
                s.target_community_id.map(|id| id.to_string()),
            )
        })
        .collect();
    let b = Some("00000000-0000-0000-0000-00000000000b".to_owned());
    let c = Some("00000000-0000-0000-0000-00000000000c".to_owned());
    assert_eq!(
        actual,
        vec![
            ("gate_user", None),
            ("purge_db", b.clone()),
            ("purge_agents", b),
            ("purge_db", c.clone()),
            ("purge_agents", c)
        ]
    );
    for stage in &inspection.stage_history {
        assert_eq!(stage.status, "completed");
        assert!(!stage.is_current);
        assert!(stage.started_at.is_some());
        assert!(stage.finished_at >= stage.started_at);
        assert_eq!(stage.metadata_json["result"]["dummy"], true);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dummy_executor_completes_two_communities_and_completed_rerun_is_read_only() {
    let (store, id, _) = fixture().await;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(15),
            run(&store, id, "worker", &options(), &CancellationToken::new())
        )
        .await
        .unwrap()
        .unwrap(),
        0
    );
    assert_completed(&store, id).await;
    let before = serde_json::to_value(store.inspect(id).await.unwrap()).unwrap();
    assert_eq!(
        run(&store, id, "worker", &options(), &CancellationToken::new())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        serde_json::to_value(store.inspect(id).await.unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn next_step_advances_one_handler_at_a_time_through_completion() {
    let (store, id, _) = fixture().await;
    let one = RunOptions {
        next_step: true,
        ..options()
    };
    run(&store, id, "worker-a", &one, &CancellationToken::new())
        .await
        .unwrap();
    let paused = store.inspect(id).await.unwrap();
    assert_eq!(paused.request.status, "running");
    assert!(paused.execution.lease_owner.is_none());
    assert_eq!(paused.stage_history.len(), 2);
    assert_eq!(paused.stage_history[1].stage, "purge_db");
    assert_eq!(paused.stage_history[1].status, "pending");
    let gate = serde_json::to_value(&paused.stage_history[0]).unwrap();
    for completed_handlers in 2..=5 {
        run(&store, id, "worker-b", &one, &CancellationToken::new())
            .await
            .unwrap();
        let after = store.inspect(id).await.unwrap();
        assert!(after.execution.lease_owner.is_none());
        assert_eq!(
            after
                .stage_history
                .iter()
                .filter(|s| s.status == "completed")
                .count(),
            completed_handlers
        );
        if completed_handlers < 5 {
            assert_eq!(after.stage_history.len(), completed_handlers + 1);
            let current = after.stage_history.iter().find(|s| s.is_current).unwrap();
            assert_eq!(current.status, "pending");
            assert!(current.started_at.is_none());
        } else {
            assert_eq!(after.stage_history.len(), 5);
            assert!(!after.stage_history.iter().any(|s| s.is_current));
        }
    }
    assert_completed(&store, id).await;
    assert_eq!(
        serde_json::to_value(&store.inspect(id).await.unwrap().stage_history[0]).unwrap(),
        gate
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dummy_executor_failure_retries_same_community_stage() {
    let (store, id, pool) = fixture().await;
    let fail = RunOptions {
        fail_stage: Some("purge_db".to_owned()),
        ..options()
    };
    assert_eq!(
        run(&store, id, "worker-a", &fail, &CancellationToken::new())
            .await
            .unwrap(),
        1
    );
    let failed = store.inspect(id).await.unwrap();
    assert_eq!(failed.request.status, "retry_wait");
    assert_eq!(failed.execution.retry_count, 1);
    assert!(failed
        .execution
        .last_error
        .as_deref()
        .unwrap()
        .contains("injected dummy failure"));
    assert_eq!(failed.stage_history.len(), 2);
    let stage_id = failed.stage_history[1].id;
    let started = failed.stage_history[1].started_at;
    assert_eq!(failed.stage_history[1].status, "failed");
    assert!(run(
        &store,
        id,
        "worker-b",
        &options(),
        &CancellationToken::new()
    )
    .await
    .is_err());
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(id).execute(&pool).await.unwrap();
    run(
        &store,
        id,
        "worker-b",
        &options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_completed(&store, id).await;
    let after = store.inspect(id).await.unwrap();
    assert_eq!(after.stage_history[1].id, stage_id);
    assert_eq!(after.stage_history[1].started_at, started);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dummy_executor_shutdown_keeps_running_stage_for_next_worker() {
    let (store, id, _) = fixture().await;
    let shutdown = CancellationToken::new();
    let delayed = RunOptions {
        stage_delay: Duration::from_secs(60),
        ..options()
    };
    let work = run(&store, id, "worker-a", &delayed, &shutdown);
    tokio::pin!(work);
    // Poll the production executor until its durable running row is visible.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut work => panic!("run finished before cancellation: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    if store.inspect(id).await.unwrap().stage_history.first().is_some_and(|s| s.status == "running") { break; }
                }
            }
        }
    }).await.unwrap();
    shutdown.cancel();
    assert_eq!(work.await.unwrap(), 0);
    let paused = store.inspect(id).await.unwrap();
    assert_eq!(paused.stage_history[0].status, "running");
    assert!(paused.execution.lease_owner.is_none());
    let stage_id = paused.stage_history[0].id;
    run(
        &store,
        id,
        "worker-b",
        &options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_completed(&store, id).await;
    assert_eq!(
        store.inspect(id).await.unwrap().stage_history[0].id,
        stage_id
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dummy_handler_is_heartbeated_and_lease_loss_prevents_completion() {
    let (store, id, pool) = fixture().await;
    let lease = store
        .claim(id, "worker", Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    let stage = match UserDeletionTransition::new(&store)
        .get_next_stage(&lease, None)
        .await
        .unwrap()
    {
        NextStage::Stage(row) => row,
        other => panic!("{other:?}"),
    };
    let before = store.inspect(id).await.unwrap().execution;
    let delayed = RunOptions {
        stage_delay: Duration::from_millis(1100),
        ..options()
    };
    assert!(matches!(
        run_stage_with_heartbeat(
            &store,
            &lease,
            &stage,
            &delayed,
            &CancellationToken::new(),
            Duration::from_millis(10)
        )
        .await,
        StageOutcome::Completed
    ));
    let after = store.inspect(id).await.unwrap();
    assert!(
        after.execution.heartbeat_at.unwrap()
            > before.heartbeat_at.unwrap() + chrono::Duration::seconds(1)
    );
    assert!(after.execution.lease_expires_at > before.lease_expires_at);
    assert_eq!(after.stage_history.len(), 1);
    assert!(after.stage_history[0].is_current);
    assert_eq!(after.stage_history[0].status, "completed");

    let stage = match UserDeletionTransition::new(&store)
        .get_next_stage(&lease, None)
        .await
        .unwrap()
    {
        NextStage::Stage(row) => row,
        other => panic!("{other:?}"),
    };
    let delayed = RunOptions {
        stage_delay: Duration::from_secs(60),
        ..options()
    };
    let shutdown = CancellationToken::new();
    let work = run_stage_with_heartbeat(
        &store,
        &lease,
        &stage,
        &delayed,
        &shutdown,
        Duration::from_millis(10),
    );
    tokio::pin!(work);
    // Lose the lease while the handler is already running, not before start_stage.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                _ = &mut work => panic!("handler finished before takeover"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    if store.inspect(id).await.unwrap().stage_history[1].status == "running" {
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    let takeover = async {
        sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
            .bind(id).execute(&pool).await.unwrap();
        // Claim uses SKIP LOCKED; an in-flight heartbeat may still own the row.
        // The enclosing timeout bounds this retry, while join keeps work polled.
        loop {
            if let Some(replacement) = store
                .claim(id, "worker", DEFAULT_LEASE_DURATION)
                .await
                .unwrap()
            {
                break replacement;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    // Keep polling renewal while takeover waits for the parent lock; an
    // in-flight heartbeat must be allowed to commit and release that lock.
    let (outcome, replacement) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(&mut work, takeover)
    })
    .await
    .unwrap();
    assert_eq!(replacement.generation, lease.generation + 1);
    assert!(matches!(outcome, StageOutcome::Failed(_)));
    let after = store.inspect(id).await.unwrap();
    assert_eq!(after.stage_history.len(), 2);
    assert_eq!(after.stage_history[1].status, "running");
    assert!(after.stage_history[1].finished_at.is_none());
    assert_eq!(after.execution.lease_generation, replacement.generation);
    assert_eq!(after.execution.lease_owner.as_deref(), Some("worker"));
    assert!(store.release(&replacement).await.unwrap());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_dummy_executor_completes_two_communities() {
    let (store, id, _) = fixture().await;
    run(&store, id, "worker", &options(), &CancellationToken::new())
        .await
        .unwrap();
    assert_completed(&store, id).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn restart_after_handler_resumes_saved_branch_without_rerunning_action() {
    let (store, id, _) = fixture().await;
    assert!(store.inspect(id).await.unwrap().stage_history.is_empty());
    execute_without_transition(&store, id, &options()).await;
    let after_gate = store.inspect(id).await.unwrap();
    assert_eq!(after_gate.stage_history.len(), 1);
    assert_eq!(after_gate.stage_history[0].status, "completed");
    assert!(after_gate.stage_history[0].is_current);
    let gate_finished = after_gate.stage_history[0].finished_at;

    let branch = RunOptions {
        no_agents_in: vec![Uuid::from_u128(11)],
        ..options()
    };
    execute_without_transition(&store, id, &branch).await;
    let after_db = store.inspect(id).await.unwrap();
    assert_eq!(after_db.stage_history.len(), 2);
    assert_eq!(
        after_db.stage_history[1].metadata_json["result"]["has_agents"],
        false
    );
    let db_finished = after_db.stage_history[1].finished_at;
    // No skip flag on restart: routing must use the persisted result, not new options.
    run(
        &store,
        id,
        "worker-c",
        &options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let completed = store.inspect(id).await.unwrap();
    assert_eq!(completed.request.status, "completed");
    assert_eq!(completed.stage_history.len(), 4);
    assert_eq!(completed.stage_history[0].finished_at, gate_finished);
    assert_eq!(completed.stage_history[1].finished_at, db_finished);
    assert_eq!(
        completed.stage_history[1].metadata_json["transition"]["kind"],
        "community_complete"
    );
    assert!(!completed
        .stage_history
        .iter()
        .any(|row| row.target_community_id == Some(Uuid::from_u128(11))
            && row.stage == "purge_agents"));
}

// Simulate process exit after the real handler commits but before the worker
// calls get_next_stage again. No operator-facing pause flag is needed.
async fn execute_without_transition(store: &EntityDeletionStore, id: Uuid, options: &RunOptions) {
    let lease = store
        .claim(id, "interrupted-worker", DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(row) = UserDeletionTransition::new(store)
        .get_next_stage(&lease, None)
        .await
        .unwrap()
    else {
        panic!("expected an executable stage");
    };
    handlers::dispatch(store, &lease, &row, options)
        .await
        .unwrap();
    store.release(&lease).await.unwrap();
}
