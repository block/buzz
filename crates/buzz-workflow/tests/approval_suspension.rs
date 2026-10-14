use buzz_core::CommunityId;
use buzz_db::{workflow::RunStatus, Db, DbConfig};
use buzz_workflow::{
    executor::{execute_run, TriggerContext},
    WorkflowConfig, WorkflowEngine,
};
use uuid::Uuid;

#[tokio::test]
async fn approval_suspends_durably_before_next_step() {
    let db = Db::new(&DbConfig {
        database_url: std::env::var("DATABASE_URL").unwrap(),
        ..Default::default()
    })
    .await
    .unwrap();
    buzz_db::migration::run_migrations(db.pool()).await.unwrap();
    let owner = nostr::Keys::generate().public_key().to_bytes();
    let community = match db
        .create_community_with_owner(&format!("s37-{}.test", Uuid::new_v4()), &hex::encode(owner))
        .await
        .unwrap()
    {
        buzz_db::CreateCommunityWithOwnerResult::Created(row) => row.id,
        other => panic!("{other:?}"),
    };
    db.ensure_user(community, &owner).await.unwrap();
    let (def, json) = WorkflowEngine::parse_yaml(
        "name: approval\ntrigger:\n  on: message_posted\nsteps:\n  - id: gate\n    action: request_approval\n    from: '{{trigger.author}}'\n    message: confirm\n    timeout: 1h\n  - id: forbidden\n    action: send_dm\n    to: nobody\n    text: must-not-run\n",
    ).unwrap();
    let workflow = db
        .create_workflow(community, None, &owner, "approval", &json, &[0; 32])
        .await
        .unwrap();
    let run = db
        .create_workflow_run(community, workflow, None, None)
        .await
        .unwrap();
    let engine = WorkflowEngine::new(db.clone(), WorkflowConfig::default());
    let result = execute_run(
        &engine,
        community,
        run,
        &def,
        &TriggerContext {
            author: hex::encode(owner),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.step_index, 0, "subsequent step must not execute");
    let token = result.approval_token.clone().unwrap();
    engine.finalize_run(community, run, Ok(result), None).await;
    let stored = db.get_workflow_run(community, run).await.unwrap();
    assert_eq!(
        stored.status,
        RunStatus::WaitingApproval,
        "approval must suspend, not fail"
    );
    let approval = db.get_approval(community, &token).await.unwrap();
    assert_eq!(approval.run_id, run);
    assert_eq!(approval.approver_spec, hex::encode(owner));
    assert_eq!(approval.token.len(), 32);
    assert_ne!(approval.token, token.as_bytes());
    assert!(db
        .get_approval(CommunityId::from_uuid(Uuid::new_v4()), &token)
        .await
        .is_err());
    assert_eq!(stored.execution_trace, serde_json::json!([]));
    // Force the approval insert to fail after the status update. Both must roll back.
    db.update_workflow_run(
        community,
        run,
        RunStatus::Running,
        0,
        &serde_json::json!([]),
        None,
    )
    .await
    .unwrap();
    let failed = buzz_db::workflow::suspend_for_approval(
        db.pool(),
        buzz_db::workflow::CreateApprovalParams {
            community_id: community,
            token: &token,
            workflow_id: workflow,
            run_id: run,
            step_id: "gate",
            step_index: 0,
            approver_spec: &hex::encode(owner),
            expires_at: approval.expires_at,
        },
        &serde_json::json!([{"must": "rollback"}]),
    )
    .await;
    assert!(failed.is_err());
    let unchanged = db.get_workflow_run(community, run).await.unwrap();
    assert_eq!(unchanged.status, RunStatus::Running);
    assert_eq!(unchanged.execution_trace, serde_json::json!([]));
}
