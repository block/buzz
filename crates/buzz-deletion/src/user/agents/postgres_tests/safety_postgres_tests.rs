use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn private_and_job_media_references_are_preserved_in_the_failed_phase() {
    for (kind, phase) in [
        (30174, AgentDbPhase::PrivateState),
        (43004, AgentDbPhase::JobRecords),
    ] {
        let f = fixture(1).await;
        let event = f
            .event(
                &f.agents[0],
                kind,
                "https://example.test/shared-file",
                vec![],
            )
            .await;
        let mut failed = false;
        for _ in 0..20 {
            if f.batch().await.is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert!(f.exists(&event).await);
        let progress = f.progress().await;
        let item = &progress.agents[&f.agents[0].public_key().to_hex()];
        assert_eq!(item.status, AgentDbStatus::Failed);
        assert_eq!(item.phase, phase);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn oversized_agent_discovery_fails_without_returning_partial_inventory() {
    let f = fixture(0).await;
    sqlx::query(
        "INSERT INTO users (community_id,pubkey,agent_owner_pubkey)
        SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2 FROM generate_series(1,1025) n",
    )
    .bind(f.community.as_uuid())
    .bind(f.owner.public_key().to_bytes().as_slice())
    .execute(f.db.pool())
    .await
    .unwrap();
    let error =
        f.db.user_deletion_store()
            .generate_inventory(&f.owner.public_key().to_hex())
            .await
            .unwrap_err();
    assert!(error.to_string().contains("bounded scope"));
    // This is preview, not permission to rewrite the already approved scope.
    assert!(UserDeletionInventory::read(
        &f.db
            .entity_deletion_store()
            .get(f.lease.request_id)
            .await
            .unwrap()
    )
    .unwrap()
    .agents(*f.community.as_uuid(), &f.owner.public_key().to_hex())
    .unwrap()
    .is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn missing_or_wrong_agent_fence_and_live_owner_mismatch_refuse_cleanup() {
    let f = fixture(1).await;
    let event = f.event(&f.agents[0], 9, "keep", vec![]).await;
    let agent = f.agents[0].public_key().to_hex();
    sqlx::query("UPDATE user_deletion_fences SET source_owner_pubkey=$3 WHERE pubkey=$1 AND scope_community_id=$2")
        .bind(&agent).bind(f.community.as_uuid()).bind(f.other.public_key().to_hex()).execute(f.db.pool()).await.unwrap();
    assert!(f
        .batch()
        .await
        .unwrap_err()
        .to_string()
        .contains("provenance"));
    assert!(f.exists(&event).await);
    sqlx::query("UPDATE user_deletion_fences SET source_owner_pubkey=$3 WHERE pubkey=$1 AND scope_community_id=$2")
        .bind(&agent).bind(f.community.as_uuid()).bind(f.owner.public_key().to_hex()).execute(f.db.pool()).await.unwrap();
    sqlx::query(
        "UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=decode($2,'hex')",
    )
    .bind(f.community.as_uuid())
    .bind(&agent)
    .bind(f.other.public_key().to_bytes().as_slice())
    .execute(f.db.pool())
    .await
    .unwrap();
    f.due().await;
    assert!(f
        .batch()
        .await
        .unwrap_err()
        .to_string()
        .contains("ownership differs"));
    assert!(f.exists(&event).await);
    sqlx::query("DELETE FROM user_deletion_fences WHERE pubkey=$1 AND scope_community_id=$2")
        .bind(&agent)
        .bind(f.community.as_uuid())
        .execute(f.db.pool())
        .await
        .unwrap();
    f.due().await;
    assert!(f.batch().await.unwrap_err().to_string().contains("fence"));
    assert!(f.exists(&event).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unexpected_new_agent_and_corrupt_progress_are_not_silently_skipped() {
    let f = fixture(1).await;
    let event = f.event(&f.agents[0], 9, "keep", vec![]).await;
    sqlx::query("UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2")
        .bind(f.community.as_uuid())
        .bind(f.other.public_key().to_bytes().as_slice())
        .bind(f.owner.public_key().to_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    assert!(f
        .batch()
        .await
        .unwrap_err()
        .to_string()
        .contains("changed since approval"));
    assert!(f.exists(&event).await);
    sqlx::query("UPDATE users SET agent_owner_pubkey=NULL WHERE community_id=$1 AND pubkey=$2")
        .bind(f.community.as_uuid())
        .bind(f.other.public_key().to_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{progress,agents}','{}') WHERE id=$1")
        .bind(f.stage.id).execute(f.db.pool()).await.unwrap();
    f.due().await;
    assert!(f
        .batch()
        .await
        .unwrap_err()
        .to_string()
        .contains("saved agent progress differs"));
    assert!(f.exists(&event).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn batch_and_progress_rollback_together_on_database_error() {
    let f = fixture(1).await;
    let event = f.event(&f.agents[0], 9, "atomic deletion", vec![]).await;
    // Force only successful progress writes to fail. Failure recording remains available.
    // No test-only production hook: the constraint exercises the real final UPDATE.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE entity_deletion_stage_history ADD CONSTRAINT fail_agent_progress CHECK
         (id <> '{}' OR status <> 'running' OR metadata_json->'progress' IS NULL)",
        f.stage.id
    )))
    .execute(f.db.pool())
    .await
    .unwrap();
    assert!(f.batch().await.is_err());
    assert!(f.exists(&event).await);
    assert_eq!(
        f.progress().await.agents[&f.agents[0].public_key().to_hex()].status,
        AgentDbStatus::Failed
    );
    sqlx::query("ALTER TABLE entity_deletion_stage_history DROP CONSTRAINT fail_agent_progress")
        .execute(f.db.pool())
        .await
        .unwrap();
    f.due().await;
    f.finish().await;
    assert!(!f.exists(&event).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_workflow_and_recipient_delivery_block_their_phases() {
    let f = fixture(1).await;
    let agent = f.agents[0].public_key().to_bytes();
    let workflow:Uuid=sqlx::query_scalar("INSERT INTO workflows (community_id,name,owner_pubkey,definition,definition_hash) VALUES ($1,'owned',$2,'{}','\\x01') RETURNING id")
        .bind(f.community.as_uuid()).bind(agent.as_slice()).fetch_one(f.db.pool()).await.unwrap();
    let run:Uuid=sqlx::query_scalar("INSERT INTO workflow_runs (community_id,workflow_id,status) VALUES ($1,$2,'running') RETURNING id")
        .bind(f.community.as_uuid()).bind(workflow).fetch_one(f.db.pool()).await.unwrap();
    for _ in 0..20 {
        if f.batch().await.is_err() {
            break;
        }
    }
    assert_eq!(
        f.progress().await.agents[&hex::encode(agent)].phase,
        AgentDbPhase::OwnedWorkflows
    );
    sqlx::query("UPDATE workflow_runs SET status='completed' WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(run)
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO workflow_approvals (community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,expires_at) VALUES ($1,'\\x01',$2,$3,'step',0,'owner',now())")
        .bind(f.community.as_uuid()).bind(workflow).bind(run).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO scheduled_workflow_fires (community_id,workflow_id,scheduled_for,workflow_run_id) VALUES ($1,$2,now(),$3)")
        .bind(f.community.as_uuid()).bind(workflow).bind(run).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO operator_listener_outbox (listener_pubkey,target_pubkey,community_id,event_id,event_kind,event_created_at,state,lease_until) VALUES ($1,$2,$3,$4,9,now(),'sending',now()+interval '1 hour')")
        .bind(f.other.public_key().to_bytes().as_slice()).bind(agent.as_slice()).bind(f.community.as_uuid())
        .bind([78u8;32].as_slice()).execute(f.db.pool()).await.unwrap();
    f.due().await;
    for _ in 0..30 {
        if f.batch().await.is_err() {
            break;
        }
    }
    let progress = f.progress().await;
    assert_eq!(
        progress.agents[&hex::encode(agent)].phase,
        AgentDbPhase::DeliveryState
    );
    assert!(progress.agents[&hex::encode(agent)]
        .last_error
        .as_deref()
        .unwrap()
        .contains("drain"));
    sqlx::query("UPDATE operator_listener_outbox SET lease_until=NULL WHERE community_id=$1")
        .bind(f.community.as_uuid())
        .execute(f.db.pool())
        .await
        .unwrap();
    f.due().await;
    f.finish().await;
    for table in [
        "workflows",
        "workflow_runs",
        "workflow_approvals",
        "scheduled_workflow_fires",
        "operator_listener_outbox",
    ] {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE community_id=$1"
        )))
        .bind(f.community.as_uuid())
        .fetch_one(f.db.pool())
        .await
        .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}
