//! Start at an approved account stage; the preceding handlers are
//! fixture preconditions, while account dispatch, persistence and release are real.

use super::*;
use buzz_db::{entity_deletion::EntityInspection, Db};
use serde_json::json;
use sqlx::PgPool;

const TARGET: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

struct Fixture {
    pool: PgPool,
    store: EntityDeletionStore,
    request: Uuid,
    community: Uuid,
    stage: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap();
        let pool = PgPool::connect(&url).await.unwrap();
        let db = Db::from_pool(pool.clone());
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            db.migrate().await.unwrap();
        }
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities(id,host) VALUES ($1,$2)")
            .bind(community)
            .bind(format!("{community}.example.org"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(community_id,pubkey) VALUES ($1,decode($2,'hex'))")
            .bind(community)
            .bind(TARGET)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(community)
            .bind(TARGET)
            .execute(&pool)
            .await
            .unwrap();
        for n in 1..=3 {
            sqlx::query("INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes) VALUES ($1,$2,decode($3,'hex'),$4,'fixture','[]')")
                .bind(community).bind(Uuid::from_u128(n)).bind(TARGET)
                .bind(vec![n as u8; 32]).execute(&pool).await.unwrap();
        }
        let request = db
            .user_deletion_store()
            .submit(TARGET, "submitter", None)
            .await
            .unwrap();
        let store = db.entity_deletion_store();
        store
            .approve(
                request.id,
                "approver",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        let lease = store
            .claim(request.id, "fixture", DEFAULT_LEASE_DURATION)
            .await
            .unwrap()
            .unwrap();
        let transition = UserDeletionTransition::new(&store);
        // Save explicit prerequisites through the storage APIs. Do not run or
        // pretend to exercise the preceding handlers in this account retry test.
        for name in ["gate_user", "purge_media", "purge_content"] {
            let NextStage::Stage(row) = transition.get_next_stage(&lease, None).await.unwrap()
            else {
                panic!("expected prerequisite stage");
            };
            assert_eq!(row.stage, name);
            store.start_stage(&lease, row.id).await.unwrap();
            store
                .save_stage_success(
                    &lease,
                    row.id,
                    if name == "purge_content" {
                        json!({"content": {}, "account_erasure_complete": false})
                    } else {
                        json!({"fixture_prerequisite": true})
                    },
                )
                .await
                .unwrap();
        }
        let NextStage::Stage(row) = transition.get_next_stage(&lease, None).await.unwrap() else {
            panic!("expected account stage");
        };
        assert_eq!(row.stage, "purge_account");
        assert_eq!(row.status, "pending");
        assert_eq!(row.target_community_id, Some(community));
        store.release(&lease).await.unwrap();
        // This worker journey covers a former member's credentials. The real
        // CLI journey supplies a process-local signing key for roster cleanup.
        sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
            .bind(community)
            .bind(TARGET)
            .execute(&pool)
            .await
            .unwrap();
        Self {
            pool,
            store,
            request: request.id,
            community,
            stage: row.id,
        }
    }

    fn options(&self, max_items: u32) -> RunOptions {
        RunOptions {
            db: Some(Db::from_pool(self.pool.clone())),
            media: None,
            delete: true,
            relay_pubkey: None,
            batch_size: 2,
            max_items,
            next_step: true,
            fail_stage: None,
            stage_delay: Duration::ZERO,
            no_agents_in: Vec::new(),
        }
    }

    async fn tokens(&self) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM api_tokens WHERE community_id=$1 ORDER BY id")
            .bind(self.community)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    async fn inspect_released(&self) -> EntityInspection {
        let state = self.store.inspect(self.request).await.unwrap();
        assert!(state.execution.lease_owner.is_none());
        assert!(state.execution.lease_expires_at.is_none());
        assert!(state.execution.completed_at.is_none());
        state
    }

    async fn waiting_on(&self, blocker_pid: i32) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
            .bind(blocker_pid).fetch_one(&self.pool).await.unwrap()
    }

    async fn assert_account_saved(&self, items: u32) {
        assert!(self.tokens().await.is_empty());
        let memberships: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM relay_members WHERE community_id=$1 AND pubkey=$2",
        )
        .bind(self.community)
        .bind(TARGET)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        assert_eq!(memberships, 0);
        let state = self.inspect_released().await;
        // A successful account stage does not imply complete user erasure.
        assert_eq!(state.request.status, "running");
        assert_eq!(state.execution.retry_count, 0);
        assert!(state.execution.last_error.is_none());
        assert!(state.execution.next_attempt_at.is_none());
        assert_eq!(state.stage_history.len(), 5);
        let account = state
            .stage_history
            .iter()
            .find(|s| s.id == self.stage)
            .unwrap();
        assert_eq!(account.status, "completed");
        assert!(!account.is_current);
        assert!(account.finished_at.is_some());
        assert!(account.last_error.is_none());
        assert_eq!(account.metadata_json["result"]["items"], items);
        assert_eq!(
            account.metadata_json["result"]["supported_scope_exhausted"],
            true
        );
        assert_eq!(
            account.metadata_json["result"]["account_erasure_complete"],
            false
        );
        assert_eq!(
            account.metadata_json["transition"],
            json!({"kind":"stage","stage":"purge_agents"})
        );
        let next = state.stage_history.iter().find(|s| s.is_current).unwrap();
        assert_eq!(next.stage, "purge_agents");
        assert_eq!(next.status, "pending");
        assert!(next.started_at.is_none());
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn real_account_cap_saves_failure_then_retry_saves_success() {
    let f = Fixture::new().await;
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        run(
            &f.store,
            f.request,
            "capped-worker",
            &f.options(1),
            &CancellationToken::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("account batch limit reached"));
    assert_eq!(
        f.tokens().await,
        vec![Uuid::from_u128(2), Uuid::from_u128(3)]
    );
    let failed = f.inspect_released().await;
    assert_eq!(failed.request.status, "retry_wait");
    assert_eq!(failed.execution.retry_count, 1);
    assert!(failed.execution.next_attempt_at.is_some());
    assert!(failed
        .execution
        .last_error
        .as_deref()
        .unwrap()
        .contains("account batch limit reached"));
    let account = failed
        .stage_history
        .iter()
        .find(|s| s.id == f.stage)
        .unwrap();
    assert_eq!(account.status, "failed");
    assert!(account.is_current);
    assert!(account.metadata_json.get("result").is_none());
    assert!(account
        .last_error
        .as_deref()
        .unwrap()
        .contains("account batch limit reached"));
    // Ownership changed after approval and the first invocation. The real
    // workflow retry must discover this direct agent while retaining its source.
    let agent = "22".repeat(32);
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES ($1,decode($2,'hex'),decode($3,'hex'))")
        .bind(f.community).bind(&agent).bind(TARGET).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes) VALUES ($1,$2,decode($3,'hex'),$4,'agent retry','[]')")
        .bind(f.community).bind(Uuid::new_v4()).bind(&agent).bind(vec![9u8;32]).execute(&f.pool).await.unwrap();
    let started = account.started_at;
    // Retry timing is a storage precondition, not a sleep in this regression.
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1")
        .bind(f.request).execute(&f.pool).await.unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(10),
            run(
                &f.store,
                f.request,
                "retry-worker",
                &f.options(100),
                &CancellationToken::new()
            ),
        )
        .await
        .unwrap()
        .unwrap(),
        0
    );
    f.assert_account_saved(3).await;
    let resumed = f.inspect_released().await;
    assert_eq!(
        resumed.execution.lease_generation,
        failed.execution.lease_generation + 1
    );
    assert_eq!(
        resumed
            .stage_history
            .iter()
            .find(|s| s.id == f.stage)
            .unwrap()
            .started_at,
        started
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn real_account_cancellation_releases_lease_and_resumes_committed_batches() {
    let f = Fixture::new().await;
    let mut blocker = f.pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    // Batch one deletes tokens 1 and 2. Hold token 3 so further progress must
    // wait; cancellation must finish even while this barrier stays locked.
    sqlx::query("SELECT id FROM api_tokens WHERE community_id=$1 AND id=$2 FOR UPDATE")
        .bind(f.community)
        .bind(Uuid::from_u128(3))
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let shutdown = CancellationToken::new();
    let options = f.options(100);
    let work = run(&f.store, f.request, "cancelled-worker", &options, &shutdown);
    tokio::pin!(work);
    let committed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut work => panic!("worker finished before cancellation: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    if f.tokens().await == vec![Uuid::from_u128(3)]
                        && f.waiting_on(blocker_pid).await { break; }
                }
            }
        }
    })
    .await;
    shutdown.cancel();
    let stopped = tokio::time::timeout(Duration::from_secs(5), &mut work).await;
    // Always release the barrier before asserting, including timeout paths.
    blocker.rollback().await.unwrap();
    committed.unwrap();
    assert_eq!(stopped.unwrap().unwrap(), 0);
    assert_eq!(f.tokens().await, vec![Uuid::from_u128(3)]);
    let paused = f.inspect_released().await;
    assert_eq!(paused.request.status, "running");
    let account = paused
        .stage_history
        .iter()
        .find(|s| s.id == f.stage)
        .unwrap();
    assert_eq!(account.status, "running");
    assert!(account.is_current);
    assert!(account.metadata_json.get("result").is_none());
    assert!(account.finished_at.is_none());
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(10),
            run(
                &f.store,
                f.request,
                "replacement-worker",
                &options,
                &CancellationToken::new()
            ),
        )
        .await
        .unwrap()
        .unwrap(),
        0
    );
    f.assert_account_saved(1).await;
    assert_eq!(
        f.inspect_released().await.execution.lease_generation,
        paused.execution.lease_generation + 1
    );
}
