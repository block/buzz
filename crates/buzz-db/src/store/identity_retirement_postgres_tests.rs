//! Identity retirement through the real store functions: archive, replay,
//! inventory binding, the execution hold on every workflow and run writer,
//! cancelled-run fencing, a signed NIP-IA unarchive leaving the hold, and
//! provenance-checked release. The race
//! tests hold the other side's lock in an open transaction and assert the
//! production writer waits for it.

use std::time::Duration;

use buzz_core::CommunityId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::DbError;
use crate::identity_retirement::{
    RetirementError, RetirementOperation, RetirementOutcome, RetirementPayload, RetirementRequest,
};
use crate::workflow::RunStatus;
use crate::Db;

const ROOT: [u8; 32] = [0x11; 32];
const AGENT: [u8; 32] = [0x22; 32];
const OTHER: [u8; 32] = [0x33; 32];
const OPERATOR: [u8; 32] = [0x44; 32];

struct Fixture {
    pool: PgPool,
    db: Db,
    c: CommunityId,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
            crate::migration::run_migrations(&pool)
                .await
                .expect("migrate test DB");
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("retire-{}.example", id.simple()))
            .execute(&pool)
            .await
            .unwrap();
        let f = Self {
            db: Db::from_pool(pool.clone()),
            pool,
            c: CommunityId::from_uuid(id),
        };
        f.user(&ROOT, None).await;
        f.user(&AGENT, Some(&ROOT)).await;
        f.user(&OTHER, None).await;
        f
    }

    async fn user(&self, key: &[u8], owner: Option<&[u8]>) {
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
        )
        .bind(self.c.as_uuid())
        .bind(key)
        .bind(owner)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn workflow(&self, owner: &[u8], status: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash, status) \
             VALUES ($1, $2, 'example', '{}', $2, $3::workflow_status) RETURNING id",
        )
        .bind(self.c.as_uuid())
        .bind(owner)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn run(&self, workflow: Uuid, status: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO workflow_runs (community_id, workflow_id, status) \
             VALUES ($1, $2, $3::run_status) RETURNING id",
        )
        .bind(self.c.as_uuid())
        .bind(workflow)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn version(&self) -> String {
        self.db
            .admin_identity_inventory(self.c, &ROOT)
            .await
            .unwrap()
            .inventory_version
            .expect("untruncated inventory has a version")
    }

    fn request(&self, operation: RetirementOperation, target: [u8; 32]) -> RetirementRequest {
        RetirementRequest {
            community: self.c,
            request_id: Uuid::new_v4(),
            operation,
            target,
            actor: OPERATOR,
            payload: RetirementPayload {
                reason: None,
                replaced_by: None,
                inventory_version: None,
            },
        }
    }

    async fn archive_request(&self) -> RetirementRequest {
        let mut req = self.request(RetirementOperation::Archive, ROOT);
        req.payload.reason = Some("left-company".into());
        req.payload.inventory_version = Some(self.version().await);
        req
    }

    async fn workflow_status(&self, id: Uuid) -> String {
        self.db
            .get_workflow(self.c, id)
            .await
            .unwrap()
            .status
            .to_string()
    }

    async fn owner_of(&self, key: &[u8]) -> Option<Vec<u8>> {
        sqlx::query_scalar(
            "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(self.c.as_uuid())
        .bind(key)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn held(&self, key: &[u8]) -> bool {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM identity_retirement_holds WHERE community_id = $1 AND pubkey = $2)",
        )
        .bind(self.c.as_uuid())
        .bind(key)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

fn applied(
    outcome: RetirementOutcome,
) -> (crate::identity_retirement::RetirementRecord, Vec<String>) {
    match outcome {
        RetirementOutcome::Applied { record, archived } => (record, archived),
        RetirementOutcome::Replayed(_) => panic!("expected a fresh action"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_retires_the_exact_key_and_agents_and_replays_without_reapplying() {
    let f = Fixture::new().await;
    let root_wf = f.workflow(&ROOT, "active").await;
    let agent_wf = f.workflow(&AGENT, "active").await;
    let disabled_wf = f.workflow(&AGENT, "disabled").await;
    let other_wf = f.workflow(&OTHER, "active").await;
    let pending = f.run(agent_wf, "pending").await;
    let running = f.run(agent_wf, "running").await;
    let waiting = f.run(agent_wf, "waiting_approval").await;
    let done = f.run(agent_wf, "completed").await;
    let other_run = f.run(other_wf, "running").await;
    sqlx::query(
        "INSERT INTO workflow_approvals (community_id, token, workflow_id, run_id, step_id, step_index, approver_spec, expires_at) \
         VALUES ($1, $2, $3, $4, 's', 0, 'x', now() + interval '1 day')",
    )
    .bind(f.c.as_uuid())
    .bind([7u8; 32].as_slice())
    .bind(agent_wf)
    .bind(waiting)
    .execute(&f.pool)
    .await
    .unwrap();

    let req = f.archive_request().await;
    let (record, archived) = applied(f.db.apply_identity_retirement(&req).await.unwrap());
    assert_eq!(archived, vec![hex::encode(ROOT), hex::encode(AGENT)]);
    assert!(!record.published);
    let r = &record.result;
    assert_eq!(r.identities.len(), 2);
    assert!(r
        .identities
        .iter()
        .all(|i| i.archived && i.execution_held && i.changed));
    assert_eq!(
        (r.runs.cancelled, r.runs.in_flight, r.runs.approvals_expired),
        (3, 1, 1)
    );
    assert_eq!(r.stages.new_work_admission, "blocked");
    assert_eq!(r.stages.workflows, "confirmed");
    assert_eq!(r.stages.in_flight_runs, "draining");
    assert_eq!(r.stages.agent_runtime, "unsupported");
    assert_eq!(r.workflows.len(), 3);

    assert_eq!(f.workflow_status(root_wf).await, "archived");
    assert_eq!(f.workflow_status(agent_wf).await, "archived");
    assert_eq!(f.workflow_status(disabled_wf).await, "disabled");
    assert_eq!(f.workflow_status(other_wf).await, "active");
    for (run, status, completed) in [
        (pending, RunStatus::Cancelled, true),
        (running, RunStatus::Cancelled, false),
        (waiting, RunStatus::Cancelled, true),
        (done, RunStatus::Completed, true),
        (other_run, RunStatus::Running, false),
    ] {
        let row = f.db.get_workflow_run(f.c, run).await.unwrap();
        assert_eq!(row.status, status);
        if status == RunStatus::Cancelled {
            assert_eq!(row.completed_at.is_some(), completed, "run {run}");
            assert_eq!(row.error_code.as_deref(), Some("identity_retired"));
        }
    }
    let approval: String = sqlx::query_scalar(
        "SELECT status::text FROM workflow_approvals WHERE community_id = $1 AND run_id = $2",
    )
    .bind(f.c.as_uuid())
    .bind(waiting)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(approval, "expired");

    // Relay-consent rows stay readable by the existing (and older) reader.
    let rows = f.db.list_archived(f.c).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|a| a.consent_path == "relay" && a.request_event_id.is_empty()));
    assert_eq!(
        rows.iter()
            .find(|a| a.pubkey == hex::encode(ROOT))
            .unwrap()
            .reason
            .as_deref(),
        Some("left-company")
    );

    // The inventory reports the hold and the draining run.
    let inventory = f.db.admin_identity_inventory(f.c, &ROOT).await.unwrap();
    assert!(inventory.identities.iter().all(|i| i.execution_held));
    let row = inventory
        .workflows
        .iter()
        .find(|w| w.id == agent_wf)
        .unwrap();
    assert_eq!(row.draining_runs, 1);

    // Same request: replayed, nothing re-applied.
    sqlx::query("UPDATE workflows SET status = 'active' WHERE community_id = $1 AND id = $2")
        .bind(f.c.as_uuid())
        .bind(other_wf)
        .execute(&f.pool)
        .await
        .unwrap();
    match f.db.apply_identity_retirement(&req).await.unwrap() {
        RetirementOutcome::Replayed(replayed) => {
            assert_eq!(replayed.action_id, record.action_id);
            assert_eq!(replayed.result, record.result);
        }
        RetirementOutcome::Applied { .. } => panic!("replay re-applied"),
    }
    // Same id with any other payload, operation, target or signer: conflict.
    let mut variants = Vec::new();
    for change in 0..4 {
        let mut other = req.clone();
        match change {
            0 => other.payload.reason = Some("other".into()),
            1 => other.operation = RetirementOperation::Release,
            2 => other.target = AGENT,
            _ => other.actor = [0x55; 32],
        }
        variants.push(other);
    }
    for other in variants {
        assert!(matches!(
            f.db.apply_identity_retirement(&other).await,
            Err(RetirementError::RequestIdConflict)
        ));
    }

    // A row recorded under an operation this release no longer accepts (an
    // earlier build's operator unarchive) stays as written, and reusing its
    // request id is a conflict, not a decode failure.
    let historical = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity_retirement_actions \
         (community_id, id, request_id, operation, target_pubkey, actor_pubkey, request, result) \
         VALUES ($1, $2, $3, 'unarchive', $4, $5, '{}', '{}')",
    )
    .bind(f.c.as_uuid())
    .bind(Uuid::new_v4())
    .bind(historical)
    .bind(ROOT.as_slice())
    .bind(req.actor.as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    let mut reuse = f.request(RetirementOperation::Release, ROOT);
    reuse.request_id = historical;
    assert!(matches!(
        f.db.apply_identity_retirement(&reuse).await,
        Err(RetirementError::RequestIdConflict)
    ));
    let (operation, result): (String, serde_json::Value) = sqlx::query_as(
        "SELECT operation, result FROM identity_retirement_actions \
         WHERE community_id = $1 AND request_id = $2",
    )
    .bind(f.c.as_uuid())
    .bind(historical)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        (operation.as_str(), result),
        ("unarchive", serde_json::json!({}))
    );
    assert!(
        f.held(&ROOT).await,
        "the conflicting reuse released nothing"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_binds_to_the_confirmed_inventory_and_fails_closed_on_overflow() {
    let f = Fixture::new().await;
    let req = f.archive_request().await;
    f.workflow(&AGENT, "active").await;
    assert!(matches!(
        f.db.apply_identity_retirement(&req).await,
        Err(RetirementError::InventoryChanged)
    ));
    assert!(!f.held(&ROOT).await);

    let mut missing = f.request(RetirementOperation::Archive, ROOT);
    missing.payload.inventory_version = None;
    assert!(matches!(
        f.db.apply_identity_retirement(&missing).await,
        Err(RetirementError::MissingInventoryVersion)
    ));

    // 200 agents plus the root is one over the limit.
    sqlx::query(
        "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) \
         SELECT $1, decode(lpad(to_hex(i), 64, '0'), 'hex'), $2 FROM generate_series(1, 200) i",
    )
    .bind(f.c.as_uuid())
    .bind(ROOT.as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    let inventory = f.db.admin_identity_inventory(f.c, &ROOT).await.unwrap();
    assert!(inventory.identities_truncated && inventory.inventory_version.is_none());
    let mut req = f.request(RetirementOperation::Archive, ROOT);
    req.payload.inventory_version = Some("0".repeat(64));
    assert!(matches!(
        f.db.apply_identity_retirement(&req).await,
        Err(RetirementError::InventoryTooLarge)
    ));
    assert!(!f.held(&ROOT).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn hold_blocks_every_workflow_writer_and_survives_a_signed_unarchive() {
    let f = Fixture::new().await;
    let root_wf = f.workflow(&ROOT, "active").await;
    let other_wf = f.workflow(&OTHER, "active").await;
    f.db.apply_identity_retirement(&f.archive_request().await)
        .await
        .unwrap();

    // An agent linked after retirement is held through its owner.
    let late = [0x66; 32];
    f.user(&late, Some(&ROOT)).await;
    for owner in [ROOT, AGENT, late] {
        assert!(matches!(
            f.db.create_workflow(f.c, None, &owner, "n", "{}", &[0; 32])
                .await,
            Err(DbError::ExecutionHeld)
        ));
        assert!(matches!(
            f.db.upsert_workflow(f.c, Uuid::new_v4(), None, &owner, "n", "{}", &[0; 32])
                .await,
            Err(DbError::ExecutionHeld)
        ));
    }
    assert!(matches!(
        f.db.create_workflow_run(
            f.c,
            root_wf,
            None,
            None,
            &crate::workflow::RunCause::Webhook
        )
        .await,
        Err(DbError::ExecutionHeld)
    ));
    f.db.create_workflow_run(
        f.c,
        other_wf,
        None,
        None,
        &crate::workflow::RunCause::Webhook,
    )
    .await
    .unwrap();

    // A signed NIP-IA unarchive (the store write its handler makes) restores
    // visibility only.
    for key in [ROOT, AGENT] {
        assert!(f.db.unarchive(f.c, &hex::encode(key)).await.unwrap());
    }
    assert!(f.db.list_archived(f.c).await.unwrap().is_empty());
    assert!(f.held(&ROOT).await && f.held(&AGENT).await);
    assert!(matches!(
        f.db.create_workflow_run(
            f.c,
            root_wf,
            None,
            None,
            &crate::workflow::RunCause::Webhook
        )
        .await,
        Err(DbError::ExecutionHeld)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cancelled_run_never_leaves_cancelled_and_records_stop_evidence() {
    let f = Fixture::new().await;
    let wf = f.workflow(&AGENT, "active").await;
    let running = f.run(wf, "running").await;
    f.db.apply_identity_retirement(&f.archive_request().await)
        .await
        .unwrap();
    assert!(f
        .db
        .get_workflow_run(f.c, running)
        .await
        .unwrap()
        .completed_at
        .is_none());
    for status in [RunStatus::Running, RunStatus::Completed, RunStatus::Failed] {
        assert!(matches!(
            f.db.update_workflow_run(f.c, running, status, 3, &serde_json::json!([]), None).await,
            Err(DbError::RunCancelled(id)) if id == running
        ));
    }
    let row = f.db.get_workflow_run(f.c, running).await.unwrap();
    assert_eq!(row.status, RunStatus::Cancelled);
    assert_eq!(row.error_code.as_deref(), Some("identity_retired"));
    assert!(row.completed_at.is_some());
    let inventory = f.db.admin_identity_inventory(f.c, &ROOT).await.unwrap();
    assert_eq!(inventory.workflows[0].draining_runs, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn release_restores_only_unmodified_workflows_of_the_exact_key() {
    let f = Fixture::new().await;
    let root_wf = f.workflow(&ROOT, "active").await;
    let kept = f.workflow(&AGENT, "active").await;
    let edited = f.workflow(&AGENT, "active").await;
    let deleted = f.workflow(&AGENT, "active").await;
    let req = f.archive_request().await;
    f.db.apply_identity_retirement(&req).await.unwrap();
    sqlx::query("UPDATE workflows SET updated_at = updated_at + interval '1 second' WHERE community_id = $1 AND id = $2")
        .bind(f.c.as_uuid())
        .bind(edited)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM workflows WHERE community_id = $1 AND id = $2")
        .bind(f.c.as_uuid())
        .bind(deleted)
        .execute(&f.pool)
        .await
        .unwrap();

    let (record, archived) = applied(
        f.db.apply_identity_retirement(&f.request(RetirementOperation::Release, AGENT))
            .await
            .unwrap(),
    );
    assert!(archived.is_empty());
    let r = &record.result;
    assert_eq!(r.stages.workflows, "partial");
    assert_eq!(
        r.stages.new_work_admission, "blocked",
        "owner is still held"
    );
    assert!(r.identities[0].changed && r.identities[0].archived);
    let by_id = |id| r.workflows.iter().find(|w| w.id == id).unwrap();
    assert!(by_id(kept).changed && by_id(kept).status == "active");
    assert_eq!(
        by_id(edited).skipped.as_deref(),
        Some("modified_since_retirement")
    );
    assert_eq!(by_id(edited).status, "archived");
    assert_eq!(by_id(deleted).status, "deleted");
    assert_eq!(f.workflow_status(kept).await, "active");
    assert_eq!(f.workflow_status(edited).await, "archived");
    assert_eq!(f.workflow_status(root_wf).await, "archived");
    // The agent is still held through its owner, and the preview says so
    // even when the agent is inspected as the root.
    assert!(!f.held(&AGENT).await);
    let preview = f.db.admin_identity_inventory(f.c, &AGENT).await.unwrap();
    assert!(!preview.identities[0].execution_held);
    assert!(preview.identities[0].owner_execution_held);
    let preview = f.db.admin_identity_inventory(f.c, &ROOT).await.unwrap();
    assert!(preview.identities[0].execution_held && !preview.identities[0].owner_execution_held);
    assert!(preview.identities[1].owner_execution_held);
    assert!(matches!(
        f.db.create_workflow_run(f.c, kept, None, None, &crate::workflow::RunCause::Webhook)
            .await,
        Err(DbError::ExecutionHeld)
    ));

    let (record, _) = applied(
        f.db.apply_identity_retirement(&f.request(RetirementOperation::Release, ROOT))
            .await
            .unwrap(),
    );
    assert_eq!(record.result.stages.new_work_admission, "released");
    assert_eq!(record.result.stages.workflows, "confirmed");
    assert_eq!(f.workflow_status(root_wf).await, "active");
    let preview = f.db.admin_identity_inventory(f.c, &AGENT).await.unwrap();
    assert!(!preview.identities[0].owner_execution_held);
    f.db.create_workflow_run(f.c, kept, None, None, &crate::workflow::RunCause::Webhook)
        .await
        .unwrap();

    // Releasing again changes nothing.
    let (record, _) = applied(
        f.db.apply_identity_retirement(&f.request(RetirementOperation::Release, ROOT))
            .await
            .unwrap(),
    );
    assert_eq!(record.result.stages.new_work_admission, "unchanged");
    assert_eq!(record.result.stages.workflows, "unchanged");
}

async fn lock_tx(
    pool: &PgPool,
    sql: &'static str,
    c: CommunityId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(sql)
        .bind(c.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

async fn assert_waits<T>(handle: &tokio::task::JoinHandle<T>) {
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!handle.is_finished(), "writer did not wait for the lock");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn retirement_waits_for_in_flight_link_and_workflow_writers() {
    for (lock, write) in [
        // Agent-owner link writers hold the invite-admission lock.
        (
            "SELECT pg_advisory_xact_lock(hashtextextended('invite-admission:' || $1::text, 0))",
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, '\\x7777777777777777777777777777777777777777777777777777777777777777', $2)",
        ),
        // Workflow and run writers hold the retirement lock shared.
        (
            "SELECT pg_advisory_xact_lock_shared(hashtextextended('identity-retirement:' || $1::text, 0))",
            "INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) \
             VALUES ($1, $2, 'late', '{}', $2)",
        ),
    ] {
        let f = Fixture::new().await;
        let req = f.archive_request().await;
        let mut writer = lock_tx(&f.pool, lock, f.c).await;
        let db = f.db.clone();
        let retire = tokio::spawn(async move { db.apply_identity_retirement(&req).await });
        assert_waits(&retire).await;
        sqlx::query(write)
        .bind(f.c.as_uuid())
        .bind(if write.contains("users") { ROOT.as_slice() } else { AGENT.as_slice() })
        .execute(&mut *writer)
        .await
        .unwrap();
        writer.commit().await.unwrap();
        assert!(matches!(
            retire.await.unwrap(),
            Err(RetirementError::InventoryChanged)
        ));
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_and_run_writers_wait_for_an_in_flight_retirement() {
    let f = Fixture::new().await;
    let wf = f.workflow(&ROOT, "active").await;
    // Take the same two locks retirement takes, through the same functions.
    let mut retirement = f.pool.begin().await.unwrap();
    crate::relay_invite::lock_admission(&mut retirement, f.c)
        .await
        .unwrap();
    crate::identity_retirement::lock_exclusive(&mut retirement, f.c)
        .await
        .unwrap();
    // Each writer runs on its own task so each must take the lock itself.
    let (c, db) = (f.c, f.db.clone());
    let create = tokio::spawn(async move {
        db.create_workflow(c, None, &ROOT, "n", "{}", &[0; 32])
            .await
    });
    let db = f.db.clone();
    let upsert = tokio::spawn(async move {
        db.upsert_workflow(c, Uuid::new_v4(), None, &AGENT, "n", "{}", &[0; 32])
            .await
    });
    let db = f.db.clone();
    let run = tokio::spawn(async move {
        db.create_workflow_run(c, wf, None, None, &crate::workflow::RunCause::Webhook)
            .await
            .map(|_| ())
    });
    assert_waits(&create).await;
    assert!(
        !upsert.is_finished() && !run.is_finished(),
        "writer did not wait for the lock"
    );
    sqlx::query(
        "INSERT INTO identity_retirement_holds (community_id, pubkey, action_id) VALUES ($1, $2, $3)",
    )
    .bind(f.c.as_uuid())
    .bind(ROOT.as_slice())
    .bind(Uuid::new_v4())
    .execute(&mut *retirement)
    .await
    .unwrap();
    retirement.commit().await.unwrap();
    assert!(matches!(create.await.unwrap(), Err(DbError::ExecutionHeld)));
    assert!(matches!(upsert.await.unwrap(), Err(DbError::ExecutionHeld)));
    assert!(matches!(run.await.unwrap(), Err(DbError::ExecutionHeld)));

    // A competing agent-owner link waits (alone, so no other waiter holds a
    // writer slot) and, once the hold commits, is refused.
    let mut retirement = f.pool.begin().await.unwrap();
    crate::relay_invite::lock_admission(&mut retirement, f.c)
        .await
        .unwrap();
    let db = f.db.clone();
    let link = tokio::spawn(async move { db.set_agent_owner(c, &OTHER, &ROOT).await });
    assert_waits(&link).await;
    retirement.commit().await.unwrap();
    assert!(matches!(link.await.unwrap(), Err(DbError::ExecutionHeld)));
    assert_eq!(f.owner_of(&OTHER).await, None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn held_owner_refuses_new_agent_links_and_keeps_existing_ones() {
    let f = Fixture::new().await;
    let req = f.archive_request().await;
    f.db.apply_identity_retirement(&req).await.unwrap();
    let (root, other) = (hex::encode(ROOT), hex::encode(OTHER));
    let relay = nostr::Keys::generate();

    // Direct NIP-OA link writers.
    assert!(matches!(
        f.db.set_agent_owner(f.c, &OTHER, &ROOT).await,
        Err(DbError::ExecutionHeld)
    ));
    assert!(matches!(
        f.db.set_agent_owner_for_authorization(f.c, &OTHER, &ROOT)
            .await,
        Err(DbError::ExecutionHeld)
    ));
    // Invitation claims that would record the link.
    assert!(matches!(
        f.db.claim_relay_membership_with_owner(f.c, &other, "member", None, Some(&root), &relay)
            .await,
        Err(DbError::InviteRestricted)
    ));
    let fresh = hex::encode([0x55; 32]);
    assert!(matches!(
        f.db.claim_relay_membership_with_owner(f.c, &fresh, "member", None, Some(&root), &relay)
            .await,
        Err(DbError::InviteRestricted)
    ));
    assert_eq!(f.owner_of(&OTHER).await, None);

    // An existing link to the held owner replays unchanged.
    assert!(!f.db.set_agent_owner(f.c, &AGENT, &ROOT).await.unwrap());
    let replay =
        f.db.claim_relay_membership_with_owner(
            f.c,
            &hex::encode(AGENT),
            "member",
            None,
            Some(&root),
            &relay,
        )
        .await
        .unwrap();
    assert!(!replay.outcome.owner_link_created);

    // After release, linking works again.
    f.db.apply_identity_retirement(&f.request(RetirementOperation::Release, ROOT))
        .await
        .unwrap();
    assert!(f.db.set_agent_owner(f.c, &OTHER, &ROOT).await.unwrap());
}
