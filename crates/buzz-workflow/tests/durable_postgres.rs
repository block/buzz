//! PostgreSQL-backed acceptance test for durable tribunal invariants.

use buzz_core::CommunityId;
use buzz_db::agent_approval::EnsureAgentApproval;
use buzz_db::agent_workflow::{AgentTaskStatus, CreateAgentArtifact, CreateAgentTask, EnsureAgentRunState};
use buzz_db::workflow::{ApprovalStatus, RunStatus};
use buzz_db::{CreateCommunityWithOwnerResult, Db, DbConfig};
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

async fn community(db: &Db, owner: &[u8; 32]) -> CommunityId {
    let host = format!("durable-e2e-{}.example", Uuid::new_v4().simple());
    let community = match db
        .create_community_with_owner(&host, &hex::encode(owner))
        .await
        .expect("create isolated test community")
    {
        CreateCommunityWithOwnerResult::Created(record) => record.id,
        other => panic!("unexpected community result: {other:?}"),
    };
    db.ensure_user(community, owner)
        .await
        .expect("create workflow owner user");
    community
}

async fn running_run(db: &Db, community: CommunityId, owner: &[u8; 32]) -> Uuid {
    let definition = json!({
        "name": "durable-postgres-e2e",
        "trigger": {"on": "webhook"},
        "steps": [{"id": "ingest", "action": "barrier"}]
    });
    let workflow_id = db
        .create_workflow(
            community,
            None,
            owner,
            &format!("durable-e2e-{}", Uuid::new_v4().simple()),
            &definition.to_string(),
            &[7_u8; 32],
        )
        .await
        .expect("create workflow");
    let run_id = db
        .create_workflow_run(community, workflow_id, None, Some(&json!({"e2e": true})))
        .await
        .expect("create workflow run");
    db.update_workflow_run(community, run_id, RunStatus::Running, 0, &json!([]), None)
        .await
        .expect("start workflow run");
    run_id
}

async fn task(
    db: &Db,
    community: CommunityId,
    run_id: Uuid,
    key: &str,
    agent: &[u8; 32],
    dependencies: &Value,
    max_attempts: i32,
) -> buzz_db::agent_workflow::AgentTask {
    db.agent_workflow_store()
        .create_task(
            community,
            CreateAgentTask {
                run_id,
                task_key: key,
                phase: key,
                agent_pubkey: Some(agent),
                max_attempts,
                input: &json!({"action": {"type": "run_agent"}, "timeout_secs": 300}),
                output_schema: Some(&json!({"type": "object"})),
                idempotency_key: &format!("{run_id}:{key}"),
                parent_task_id: None,
                depends_on: dependencies,
            },
        )
        .await
        .expect("create durable task")
}

#[tokio::test]
#[ignore = "requires an isolated migrated PostgreSQL via BUZZ_TEST_DATABASE_URL"]
async fn durable_tribunal_transactions_survive_replay_races_and_tenant_boundaries() {
    let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .expect("BUZZ_TEST_DATABASE_URL must name a disposable PostgreSQL database");
    let db = Db::new(&DbConfig {
        database_url: database_url.clone(),
        max_connections: 8,
        min_connections: 1,
        ..DbConfig::default()
    })
    .await
    .expect("connect isolated PostgreSQL");
    db.migrate().await.expect("apply all embedded migrations");

    let owner_a = [0x11_u8; 32];
    let owner_b = [0x22_u8; 32];
    let agent = [0x33_u8; 32];
    let community_a = community(&db, &owner_a).await;
    let community_b = community(&db, &owner_b).await;
    let run_a = running_run(&db, community_a, &owner_a).await;
    let run_b = running_run(&db, community_b, &owner_b).await;
    let store = db.agent_workflow_store();

    for (community, run_id) in [(community_a, run_a), (community_b, run_b)] {
        store
            .ensure_run_state(
                community,
                EnsureAgentRunState {
                    run_id,
                    phase: "materialized",
                    manifest_hash: None,
                    thread_root_event_id: None,
                    deadline: None,
                    metadata: &json!({"e2e": true}),
                },
            )
            .await
            .expect("ensure durable run state");
    }

    let prerequisite = task(&db, community_a, run_a, "analysis", &agent, &json!([]), 2).await;
    let dependent = task(
        &db,
        community_a,
        run_a,
        "barrier",
        &agent,
        &json!(["analysis"]),
        2,
    )
    .await;
    let tenant_b_task = task(&db, community_b, run_b, "analysis", &agent, &json!([]), 2).await;

    assert!(store
        .get_task(community_b, prerequisite.id)
        .await
        .expect("cross-tenant task lookup")
        .is_none());
    assert!(store
        .get_run_state(community_b, run_a)
        .await
        .expect("cross-tenant state lookup")
        .is_none());
    assert!(store
        .claim_task(community_a, dependent.id, dependent.version, &agent)
        .await
        .expect("dependency refusal")
        .is_none());

    let (first_claim, second_claim) = tokio::join!(
        store.claim_task(community_a, prerequisite.id, prerequisite.version, &agent),
        store.claim_task(community_a, prerequisite.id, prerequisite.version, &agent)
    );
    let claims = [first_claim.expect("first claim"), second_claim.expect("second claim")];
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    let claimed = claims.into_iter().flatten().next().expect("one claim wins");
    assert!(store
        .claim_task(community_a, prerequisite.id, prerequisite.version, &agent)
        .await
        .expect("stale claim")
        .is_none());

    let checkpoint = store
        .append_checkpoint(
            community_a,
            run_a,
            prerequisite.id,
            1,
            &json!({"cursor": 17}),
            None,
        )
        .await
        .expect("append checkpoint");
    assert_eq!(checkpoint.sequence, 1);
    assert_eq!(
        store
            .latest_checkpoint(community_a, run_a, prerequisite.id)
            .await
            .expect("latest checkpoint")
            .expect("checkpoint exists")
            .state,
        json!({"cursor": 17})
    );

    let body = json!({"decision": "validated"});
    let metadata = json!({"schema_valid": true});
    let digest = [0x44_u8; 32];
    let artifact = || CreateAgentArtifact {
        run_id: run_a,
        task_id: Some(prerequisite.id),
        kind: "analysis",
        version: 1,
        content_type: "application/json",
        uri: None,
        sha256: &digest,
        inline_content: Some(&body),
        metadata: &metadata,
        created_by: Some(&agent),
        idempotency_key: "analysis-v1",
    };
    let completed = store
        .persist_artifact_and_complete(community_a, prerequisite.id, claimed.version, artifact())
        .await
        .expect("persist artifact atomically")
        .expect("active task completes");
    assert_eq!(completed.0.status, AgentTaskStatus::Completed);
    assert!(store
        .persist_artifact_and_complete(community_a, prerequisite.id, completed.0.version, artifact())
        .await
        .expect("exact artifact replay")
        .is_none());
    let divergent = CreateAgentArtifact {
        sha256: &[0x55_u8; 32],
        ..artifact()
    };
    assert!(store
        .persist_artifact_and_complete(
            community_a,
            prerequisite.id,
            completed.0.version,
            divergent,
        )
        .await
        .is_err());

    let dependent_claim = store
        .claim_task(community_a, dependent.id, dependent.version, &agent)
        .await
        .expect("claim after dependency")
        .expect("dependency completion opens barrier");
    let retry = store
        .recover_timed_out_task(
            community_a,
            dependent.id,
            dependent_claim.version,
            0,
            Utc::now() - Duration::seconds(1),
        )
        .await
        .expect("recover first timeout")
        .expect("first timeout schedules retry");
    assert_eq!(retry.status, AgentTaskStatus::RetryScheduled);
    let second_attempt = store
        .claim_task(community_a, dependent.id, retry.version, &agent)
        .await
        .expect("claim retry")
        .expect("retry becomes eligible");
    let exhausted = store
        .recover_timed_out_task(
            community_a,
            dependent.id,
            second_attempt.version,
            0,
            Utc::now(),
        )
        .await
        .expect("recover final timeout")
        .expect("final timeout fails task");
    assert_eq!(exhausted.status, AgentTaskStatus::Failed);
    assert_eq!(exhausted.error_code.as_deref(), Some("agent_timeout_exhausted"));

    let approval_task = task(
        &db,
        community_a,
        run_a,
        "human_approval",
        &agent,
        &json!(["analysis"]),
        1,
    )
    .await;
    let workflow_id = db
        .get_workflow_run(community_a, run_a)
        .await
        .expect("load run")
        .workflow_id;
    let first_expiry = Utc::now() + Duration::hours(24);
    let approval = store
        .ensure_approval(
            community_a,
            EnsureAgentApproval {
                workflow_id,
                run_id: run_a,
                task_id: approval_task.id,
                step_id: "human_approval",
                request_message: "Review the verified decision",
                step_index: 9,
                approver_spec: &hex::encode(owner_a),
                expires_at: first_expiry,
            },
        )
        .await
        .expect("arm approval")
        .expect("approval is eligible");
    let replay = store
        .ensure_approval(
            community_a,
            EnsureAgentApproval {
                workflow_id,
                run_id: run_a,
                task_id: approval_task.id,
                step_id: "human_approval",
                request_message: "Review the verified decision",
                step_index: 9,
                approver_spec: &hex::encode(owner_a),
                expires_at: first_expiry + Duration::hours(1),
            },
        )
        .await
        .expect("approval replay")
        .expect("approval persists");
    assert_eq!(replay.token, approval.token);
    assert_eq!(replay.expires_at, approval.expires_at);
    assert!(store
        .ensure_approval(
            community_a,
            EnsureAgentApproval {
                workflow_id,
                run_id: run_a,
                task_id: approval_task.id,
                step_id: "human_approval",
                request_message: "Divergent message",
                step_index: 9,
                approver_spec: &hex::encode(owner_a),
                expires_at: first_expiry,
            },
        )
        .await
        .is_err());
    assert!(store
        .mark_run_waiting_approval(community_a, run_a, 9)
        .await
        .expect("suspend lifecycle for approval"));

    drop(store);
    drop(db);
    let restarted_db = Db::new(&DbConfig {
        database_url,
        max_connections: 8,
        min_connections: 1,
        ..DbConfig::default()
    })
    .await
    .expect("reconnect after simulated relay restart");
    let store = restarted_db.agent_workflow_store();
    let persisted = store
        .get_approval(community_a, run_a, approval_task.id)
        .await
        .expect("read approval after restart")
        .expect("approval survives restart");
    assert_eq!(persisted.token, approval.token);
    assert_eq!(persisted.expires_at, approval.expires_at);
    assert!(restarted_db
        .update_approval_by_stored_hash(
            community_a,
            &persisted.token,
            ApprovalStatus::Granted,
            Some(&owner_a),
            Some("approved by durable E2E"),
        )
        .await
        .expect("grant approval once"));
    assert!(!restarted_db
        .update_approval_by_stored_hash(
            community_a,
            &persisted.token,
            ApprovalStatus::Granted,
            Some(&owner_a),
            Some("duplicate approval"),
        )
        .await
        .expect("duplicate grant is a CAS miss"));
    let approved_task = store
        .complete_ready_task(community_a, approval_task.id, approval_task.version)
        .await
        .expect("complete approved task")
        .expect("approval task completes exactly once");
    assert_eq!(approved_task.status, AgentTaskStatus::Completed);
    assert!(store
        .complete_ready_task(community_a, approval_task.id, approval_task.version)
        .await
        .expect("duplicate task completion")
        .is_none());
    assert!(store
        .mark_run_running_after_approval(community_a, run_a)
        .await
        .expect("resume lifecycle once"));
    assert!(!store
        .mark_run_running_after_approval(community_a, run_a)
        .await
        .expect("duplicate resume is a CAS miss"));

    let state = store
        .get_run_state(community_a, run_a)
        .await
        .expect("read state")
        .expect("state exists");
    assert!(store
        .cas_run_state(
            community_a,
            run_a,
            state.state_version,
            "settling",
            &json!({"e2e": true}),
            None,
        )
        .await
        .expect("winning state CAS")
        .is_some());
    assert!(store
        .cas_run_state(
            community_a,
            run_a,
            state.state_version,
            "stale",
            &json!({}),
            None,
        )
        .await
        .expect("stale state CAS")
        .is_none());

    let candidates = store
        .list_reconcilable_runs(2)
        .await
        .expect("fair reconciliation scan");
    assert!(candidates.iter().any(|candidate| candidate.run_id == run_a));
    assert!(candidates.iter().any(|candidate| candidate.run_id == run_b));
    assert!(store
        .get_task(community_b, tenant_b_task.id)
        .await
        .expect("tenant B task")
        .is_some());

    let terminal = store
        .fail_active_run(
            community_a,
            run_a,
            11,
            "agent_timeout_exhausted",
            "durable E2E terminal failure",
            &json!({"e2e": true}),
        )
        .await
        .expect("atomic settlement")
        .expect("active run settles once");
    assert_eq!(terminal.to_status, "failed");
    assert!(store
        .fail_active_run(
            community_a,
            run_a,
            11,
            "agent_timeout_exhausted",
            "duplicate settlement",
            &json!({}),
        )
        .await
        .expect("settlement replay")
        .is_none());
    let transitions = store
        .list_transitions(community_a, run_a, Some(10))
        .await
        .expect("terminal ledger");
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].id, terminal.id);
    let run = restarted_db
        .get_workflow_run(community_a, run_a)
        .await
        .expect("terminal run");
    assert_eq!(run.status, RunStatus::Failed);
}
