use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn lease_expiring_at_checkpoint_rolls_back_agent_deletion_and_progress() {
    let f = fixture(1).await;
    let event = f
        .event(&f.agents[0], 9, "must survive expiry", vec![])
        .await;
    // Test-only fault injection on the real final UPDATE; no production seam.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION expire_agent_checkpoint() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
         UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()-interval '1 second'
         WHERE id='{}'; RETURN NEW; END $$;
         CREATE TRIGGER expire_agent_checkpoint AFTER UPDATE ON entity_deletion_stage_history
         FOR EACH ROW WHEN (NEW.id='{}' AND NEW.metadata_json->'progress' IS NOT NULL AND NEW.status='running')
         EXECUTE FUNCTION expire_agent_checkpoint();", f.lease.request_id, f.stage.id
    ))).execute(f.db.pool()).await.unwrap();
    let outcome = f.batch().await;
    sqlx::raw_sql("DROP TRIGGER expire_agent_checkpoint ON entity_deletion_stage_history; DROP FUNCTION expire_agent_checkpoint();")
        .execute(f.db.pool()).await.unwrap();
    assert!(outcome
        .unwrap_err()
        .to_string()
        .contains("lease expired during cleanup batch"));
    assert!(f.exists(&event).await);
    let item = &f.progress().await.agents[&f.agents[0].public_key().to_hex()];
    assert_eq!(item.phase, AgentDbPhase::Messages);
    assert_eq!(item.status, AgentDbStatus::Failed);
    f.due().await;
    f.finish().await;
    assert!(!f.exists(&event).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn restart_within_agent_uses_saved_scope_even_after_profile_disappears() {
    let mut f = fixture(1).await;
    let key = f.agents[0].public_key().to_hex();
    let one = f.event(&f.agents[0], 9, "first", vec![]).await;
    let two = f.event(&f.agents[0], 9, "second", vec![]).await;
    let batch = f.batch().await.unwrap();
    assert!(!batch.exhausted);
    assert_eq!(batch.progress.current_agent.as_deref(), Some(key.as_str()));
    assert_eq!(batch.progress.agents[&key].phase, AgentDbPhase::Messages);
    assert_ne!(f.exists(&one).await, f.exists(&two).await);
    // The handler must recover from approved inventory and fence provenance,
    // not rediscover agents from a profile row which might already be absent.
    sqlx::query("DELETE FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(f.community.as_uuid())
        .bind(&key)
        .execute(f.db.pool())
        .await
        .unwrap();
    f.reclaim().await;
    f.db = Db::from_pool(PgPool::connect(&database_url()).await.unwrap());
    f.finish().await;
    assert!(!f.exists(&one).await);
    assert!(!f.exists(&two).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn completion_write_can_retry_after_all_agent_checkpoints_commit() {
    let mut f = fixture(1).await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE entity_deletion_stage_history ADD CONSTRAINT fail_agent_completion
         CHECK (id <> '{}' OR status <> 'completed')",
        f.stage.id
    )))
    .execute(f.db.pool())
    .await
    .unwrap();
    let mut failed = false;
    for _ in 0..20 {
        if f.batch().await.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    let before = f.progress().await;
    let key = f.agents[0].public_key().to_hex();
    assert_eq!(before.agents[&key].status, AgentDbStatus::Completed);
    sqlx::query("ALTER TABLE entity_deletion_stage_history DROP CONSTRAINT fail_agent_completion")
        .execute(f.db.pool())
        .await
        .unwrap();
    f.reclaim().await;
    let done = f.batch().await.unwrap();
    assert!(done.exhausted);
    assert_eq!(
        done.progress.agents[&key].completed_at,
        before.agents[&key].completed_at
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn verification_refuses_remaining_content_instead_of_marking_agent_complete() {
    let f = fixture(1).await;
    let key = f.agents[0].public_key().to_hex();
    for _ in 0..20 {
        let batch = f.batch().await.unwrap();
        if batch.progress.agents[&key].phase == AgentDbPhase::VerifyDbContent {
            break;
        }
        assert!(!batch.exhausted);
    }
    // Simulate an ungated writer after the private-state phase. This is not a
    // supported serving path, but completion must still refuse the residual data.
    let leftover = f.event(&f.agents[0], 30174, "late memory", vec![]).await;
    assert!(f
        .batch()
        .await
        .unwrap_err()
        .to_string()
        .contains("verification"));
    assert!(f.exists(&leftover).await);
    assert_eq!(
        f.progress().await.agents[&key].status,
        AgentDbStatus::Failed
    );
}
