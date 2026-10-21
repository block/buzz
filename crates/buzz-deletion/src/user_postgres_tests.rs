use super::*;
use user::{Command, DemoControls};

async fn fixture() -> (Db, buzz_db::entity_deletion::EntityRequest) {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("isolated PostgreSQL URL");
    let pool = sqlx::PgPool::connect(&url).await.expect("connect");
    let db = Db::from_pool(pool);
    let store = db.entity_deletion_store();
    let target = format!("demo:{}", Uuid::new_v4().simple());
    user::run_with_store(
        Command::Submit {
            target: target.clone(),
            requested_by: "submitter".into(),
            reason: None,
        },
        store.clone(),
    )
    .await
    .expect("submit command");
    let request = store
        .list(100)
        .await
        .expect("list")
        .into_iter()
        .find(|r| r.target == target)
        .expect("submitted request");
    user::run_with_store(Command::Inventory { id: request.id }, store.clone())
        .await
        .expect("inventory command");
    assert!(user::run_with_store(
        Command::Run {
            id: request.id,
            executor_id: Some("unapproved".into()),
            controls: DemoControls::default()
        },
        store.clone(),
    )
    .await
    .is_err());
    user::run_with_store(
        Command::Approve {
            id: request.id,
            approved_by: "approver".into(),
        },
        store.clone(),
    )
    .await
    .expect("approve command");
    (db, store.get(request.id).await.expect("request"))
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn shared_executor_runs_three_communities_and_resumes_b() {
    let (db, request) = fixture().await;
    let store = db.entity_deletion_store();
    // Canary data at the synthetic community IDs makes accidental real cleanup
    // observable, including updates that would preserve row counts.
    for n in 1..=3 {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(Uuid::from_u128(n))
            .bind(format!("canary-{n}.invalid"))
            .execute(db.pool())
            .await
            .expect("community canary");
        sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2)")
            .bind(Uuid::from_u128(n))
            .bind(vec![7u8; 32])
            .execute(db.pool())
            .await
            .expect("user canary");
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')",
        )
        .bind(Uuid::from_u128(n))
        .bind(hex::encode([7u8; 32]))
        .execute(db.pool())
        .await
        .expect("membership canary");
    }
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
             'communities', (SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM communities c),
             'users', (SELECT jsonb_agg(to_jsonb(u) ORDER BY community_id, pubkey) FROM users u),
             'members', (SELECT jsonb_agg(to_jsonb(m) ORDER BY community_id, pubkey) FROM relay_members m),
             'events', (SELECT count(*) FROM events),
             'community_requests', (SELECT count(*) FROM community_deletion_requests))",
    )
    .fetch_one(db.pool())
    .await
    .expect("domain snapshot");
    let result = user::run_with_store(
        Command::Run {
            id: request.id,
            executor_id: Some("first-process".into()),
            controls: DemoControls {
                fail_at: Some(9),
                ..Default::default()
            },
        },
        store.clone(),
    )
    .await
    .expect("failed demonstration");
    assert_eq!(result, 1);
    let snapshot = store.inspect(request.id).await.expect("history");
    assert_eq!(snapshot.stage_history.len(), 9);
    assert_eq!(snapshot.stage_history[8].stage, "delete_data");
    assert_eq!(
        snapshot.stage_history[8].target_community_id,
        Some(Uuid::from_u128(2))
    );
    assert_eq!(snapshot.stage_history[8].status, "failed");
    assert!(snapshot.request.lease_owner.is_none());
    assert_eq!(
        snapshot.stage_history[..4]
            .iter()
            .map(|s| s.stage.as_str())
            .collect::<Vec<_>>(),
        [
            "gate_user",
            "gate_community",
            "gate_community",
            "gate_community"
        ]
    );
    assert!(snapshot.stage_history[4..7]
        .iter()
        .all(|s| s.target_community_id == Some(Uuid::from_u128(1)) && s.status == "completed"));
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at = now() WHERE id = $1")
        .bind(request.id)
        .execute(db.pool())
        .await
        .expect("make retry due");
    assert_eq!(
        user::run_with_store(
            Command::Drain {
                executor_id: Some("replacement-process".into()),
            },
            store.clone()
        )
        .await
        .expect("drain retry"),
        0
    );
    let finished = store.inspect(request.id).await.expect("finished");
    assert_eq!(finished.request.status, "demo_completed");
    assert_eq!(finished.request.lease_generation, 2);
    assert_eq!(finished.stage_history.len(), 15);
    assert!(finished
        .stage_history
        .iter()
        .all(|s| !s.is_current && s.metadata_json["simulated"] == true));
    for (a, b) in snapshot.stage_history[..8]
        .iter()
        .zip(&finished.stage_history[..8])
    {
        assert_eq!(
            serde_json::to_value(a).expect("before"),
            serde_json::to_value(b).expect("after")
        );
    }
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
             'communities', (SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM communities c),
             'users', (SELECT jsonb_agg(to_jsonb(u) ORDER BY community_id, pubkey) FROM users u),
             'members', (SELECT jsonb_agg(to_jsonb(m) ORDER BY community_id, pubkey) FROM relay_members m),
             'events', (SELECT count(*) FROM events),
             'community_requests', (SELECT count(*) FROM community_deletion_requests))",
    )
    .fetch_one(db.pool())
    .await
    .expect("domain snapshot");
    assert_eq!(before, after);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn voluntary_stop_retains_one_pending_stage_for_next_executor() {
    let (db, request) = fixture().await;
    let store = db.entity_deletion_store();
    user::run_with_store(
        Command::Run {
            id: request.id,
            executor_id: Some("stopping-process".into()),
            controls: DemoControls {
                stop_after: Some(7),
                ..Default::default()
            },
        },
        store.clone(),
    )
    .await
    .expect("stop after A");
    let stopped = store.inspect(request.id).await.expect("stopped");
    assert_eq!(stopped.request.status, "running");
    assert!(stopped.request.lease_owner.is_none());
    assert_eq!(stopped.stage_history.len(), 8);
    assert_eq!(stopped.stage_history[7].status, "pending");
    assert!(stopped.stage_history[7].is_current);
    user::run_with_store(
        Command::Run {
            id: request.id,
            executor_id: Some("next-process".into()),
            controls: DemoControls::default(),
        },
        store.clone(),
    )
    .await
    .expect("resume");
    let finished = store.inspect(request.id).await.expect("finished");
    assert_eq!(finished.request.status, "demo_completed");
    assert_eq!(finished.stage_history[7].id, stopped.stage_history[7].id);
}
