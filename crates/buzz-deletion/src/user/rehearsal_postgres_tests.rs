//! PostgreSQL coverage of the real print adapters and persisted worker recovery.
//! Media's S3 scan is outside this database fixture: its completed journal
//! boundary is simulated, never claiming to have tested storage.
use super::*;
use buzz_db::{user_deletion::UserDeletionInventory, Db, DbConfig};
use serde_json::{json, Value};

const TARGET: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

async fn source_snapshot(pool: &sqlx::PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('members',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,pubkey) FROM relay_members t),'allowlist',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,pubkey) FROM pubkey_allowlist t),'events',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,created_at,id) FROM events t),'fences',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM user_deletion_fences t))")
        .fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rehearsal_worker_recovers_completed_stage_retries_limit_and_rejects_mode_switch() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let db = Db::from_pool(pool.clone());
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community)
        .bind(format!("{community}.example.org"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(community)
        .bind(TARGET)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO pubkey_allowlist(community_id,pubkey,added_by) VALUES($1,decode($2,'hex'),decode($2,'hex'))").bind(community).bind(TARGET).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES($1,decode($2,'hex'),decode($2,'hex'),now(),9,'[]','rehearsal message',decode(repeat('00',64),'hex'))").bind(community).bind(TARGET).execute(&pool).await.unwrap();
    let users = db.user_deletion_store();
    let rehearsal = users.submit_rehearsal(TARGET, "test", None).await.unwrap();
    let deletion = users.submit(TARGET, "test", None).await.unwrap();
    for request in [&rehearsal, &deletion] {
        users
            .approve(
                request.id,
                "test",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
    }
    let store = db.entity_deletion_store();
    let source = Db::connect_cold_start(
        DbConfig {
            database_url: url,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: true,
            ..DbConfig::default()
        },
        "rehearsal_test_source",
    )
    .await
    .unwrap();
    // The source connection rejects writes; the journal uses the writer pool.
    assert!(sqlx::query("UPDATE relay_members SET role='admin'")
        .execute(source.pool())
        .await
        .is_err());
    let mut options = RunOptions {
        next_step: true,
        real: Some(super::super::real::Execution {
            db: source,
            args: super::super::real::Args {
                print_only: false,
                delete: false,
                rehearse: true,
                offline: false,
                stage: None,
                community: None,
                relay_pubkey: None,
                batch_size: 100,
                max_items: 1_000_000,
            },
        }),
        fail_stage: None,
        stage_delay: Duration::ZERO,
        no_agents_in: vec![],
    };
    let cancel = CancellationToken::new();
    let deletion_before = serde_json::to_value(store.inspect(deletion.id).await.unwrap()).unwrap();
    assert!(run(&store, deletion.id, "wrong-mode", &options, &cancel)
        .await
        .is_err());
    let rehearsal_before =
        serde_json::to_value(store.inspect(rehearsal.id).await.unwrap()).unwrap();
    let args = &mut options.real.as_mut().unwrap().args;
    args.delete = true;
    args.rehearse = false;
    args.offline = true;
    assert!(run(&store, rehearsal.id, "wrong-mode", &options, &cancel)
        .await
        .is_err());
    assert_eq!(
        rehearsal_before,
        serde_json::to_value(store.inspect(rehearsal.id).await.unwrap()).unwrap()
    );
    let args = &mut options.real.as_mut().unwrap().args;
    args.delete = false;
    args.rehearse = true;
    args.offline = false;

    // Exercise the real read-only gate through the worker. It advances to
    // media without requiring S3, installing fences, or changing source rows.
    let before_gate = source_snapshot(&pool).await;
    assert_eq!(
        run(&store, rehearsal.id, "gate-preview", &options, &cancel)
            .await
            .unwrap(),
        0
    );
    assert_eq!(before_gate, source_snapshot(&pool).await);
    let saved = store.inspect(rehearsal.id).await.unwrap();
    let gate = saved
        .stage_history
        .iter()
        .find(|s| s.stage == "gate_user")
        .unwrap();
    assert_eq!(gate.status, "completed");
    assert!(!gate.is_current);
    assert_eq!(
        gate.metadata_json["result"]["proposal"]["record"],
        "user_gate_preview"
    );

    // Establish only the media-completed boundary for this database-only test.
    let lease = store
        .claim(rehearsal.id, "fixture", DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    let transition = UserDeletionTransition::new(&store);
    let NextStage::Stage(media) = transition.get_next_stage(&lease, None).await.unwrap() else {
        panic!("media stage")
    };
    store.start_stage(&lease, media.id).await.unwrap();
    store
        .save_stage_success(&lease, media.id, json!({"execution_mode":"rehearsal"}))
        .await
        .unwrap();
    let NextStage::Stage(content) = transition
        .get_next_stage(&lease, Some(media.id))
        .await
        .unwrap()
    else {
        panic!("content stage")
    };
    assert_eq!(content.stage, "purge_content");
    let before = source_snapshot(&pool).await;
    store.release(&lease).await.unwrap();
    assert!(handlers::dispatch(&store, &lease, &content, &options)
        .await
        .is_err());
    let stopped = CancellationToken::new();
    stopped.cancel();
    assert_eq!(
        run(&store, rehearsal.id, "cancelled", &options, &stopped)
            .await
            .unwrap(),
        0
    );
    let saved = store.inspect(rehearsal.id).await.unwrap();
    assert!(saved.execution.lease_owner.is_none());
    assert_eq!(
        saved
            .stage_history
            .iter()
            .find(|s| s.is_current)
            .unwrap()
            .status,
        "pending"
    );
    assert_eq!(before, source_snapshot(&pool).await);
    let lease = store
        .claim(rehearsal.id, "content", DEFAULT_LEASE_DURATION)
        .await
        .unwrap()
        .unwrap();
    // Finish the real content preview but simulate a crash before transition.
    handlers::dispatch(&store, &lease, &content, &options)
        .await
        .unwrap();
    let saved = store.inspect(rehearsal.id).await.unwrap();
    let completed = saved
        .stage_history
        .iter()
        .find(|s| s.id == content.id)
        .unwrap();
    assert_eq!(completed.status, "completed");
    assert!(completed.is_current);
    assert!(
        completed.metadata_json["result"]["proposal"]["items"]
            .as_u64()
            .unwrap()
            > 0
    );
    let finished_at = completed.finished_at;
    store.release(&lease).await.unwrap();

    // Recovery must advance that completed stage without invoking content again.
    // The real account preview then stops at its limit, without advancing it.
    options.real.as_mut().unwrap().args.max_items = 1;
    assert!(run(&store, rehearsal.id, "limited", &options, &cancel)
        .await
        .is_err());
    let saved = store.inspect(rehearsal.id).await.unwrap();
    assert!(saved.execution.lease_owner.is_none());
    assert_eq!(
        saved
            .stage_history
            .iter()
            .find(|s| s.id == content.id)
            .unwrap()
            .finished_at,
        finished_at
    );
    let account = saved.stage_history.iter().find(|s| s.is_current).unwrap();
    assert_eq!(account.stage, "purge_account");
    assert_eq!(account.status, "failed");
    assert!(account.metadata_json.get("result").is_none());
    assert_eq!(before, source_snapshot(&pool).await);
    // Make only the fixture's retry deadline due, then resume the same stage.
    sqlx::query(
        "UPDATE entity_deletion_requests SET next_attempt_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(rehearsal.id)
    .execute(&pool)
    .await
    .unwrap();
    options.real.as_mut().unwrap().args.max_items = 1_000_000;
    assert_eq!(
        run(&store, rehearsal.id, "retry", &options, &cancel)
            .await
            .unwrap(),
        0
    );
    let saved = store.inspect(rehearsal.id).await.unwrap();
    assert_eq!(
        saved
            .stage_history
            .iter()
            .find(|s| s.is_current)
            .unwrap()
            .stage,
        "remaining_cleanup"
    );
    assert_eq!(
        run(&store, rehearsal.id, "boundary", &options, &cancel)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        run(&store, rehearsal.id, "boundary-again", &options, &cancel)
            .await
            .unwrap(),
        2
    );
    assert_eq!(before, source_snapshot(&pool).await);
    assert_eq!(
        deletion_before,
        serde_json::to_value(store.inspect(deletion.id).await.unwrap()).unwrap()
    );
    assert!(UserDeletionInventory::is_rehearsal(&store.get(rehearsal.id).await.unwrap()).unwrap());

    // The destructive worker must execute the real gate before it can select
    // media, without touching messages, memberships or uploader data.
    let execution = options.real.as_mut().unwrap();
    execution.db = Db::from_pool(pool.clone());
    execution.args.rehearse = false;
    execution.args.delete = true;
    execution.args.offline = true;
    assert_eq!(
        run(&store, deletion.id, "delete-gate", &options, &cancel)
            .await
            .unwrap(),
        0
    );
    let gated = store.inspect(deletion.id).await.unwrap();
    let gate = gated
        .stage_history
        .iter()
        .find(|s| s.stage == "gate_user")
        .unwrap();
    assert_eq!(gate.status, "completed");
    assert!(!gate.is_current);
    assert_eq!(gate.metadata_json["result"]["record"], "user_gate_complete");
    assert_eq!(
        gated
            .stage_history
            .iter()
            .find(|s| s.is_current)
            .unwrap()
            .stage,
        "purge_media"
    );
    let after_gate = source_snapshot(&pool).await;
    for key in ["members", "allowlist", "events"] {
        assert_eq!(after_gate[key], before[key]);
    }
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM user_deletion_fences WHERE request_id=$1 AND pubkey=$2 AND scope_community_id IS NULL")
        .bind(deletion.id).bind(TARGET).fetch_one(&pool).await.unwrap();
    assert_eq!(fences, 1);
}
