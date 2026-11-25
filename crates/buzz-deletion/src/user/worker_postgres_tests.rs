use super::*;
use buzz_db::Db;

#[path = "worker_postgres_tests/agent_owners.rs"]
mod agent_owners_postgres_tests;
#[path = "worker_postgres_tests/fixture.rs"]
mod media_fixture;
#[path = "worker_postgres_tests/media_retry.rs"]
mod media_retry_postgres_tests;
#[path = "worker_postgres_tests/owned_review.rs"]
mod owned_review_postgres_tests;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deletion_visits_only_memberships_saved_at_submission() {
    let fixture = media_fixture::fixture().await;
    let users = Db::from_pool(fixture.pool.clone()).user_deletion_store();
    let subject = nostr::Keys::generate().public_key().to_hex();
    let current = Uuid::from_u128(11);
    let former = Uuid::from_u128(12);
    for community in [current, former] {
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'member')",
        )
        .bind(community)
        .bind(&subject)
        .execute(&fixture.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig)
            VALUES ($1,decode(repeat('aa',32),'hex'),decode($2,'hex'),now(),9,'[]','hello',decode(repeat('00',64),'hex'))")
            .bind(community).bind(&subject).execute(&fixture.pool).await.unwrap();
    }
    sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
        .bind(former)
        .bind(&subject)
        .execute(&fixture.pool)
        .await
        .unwrap();
    let preview = users.generate_inventory(&subject).await.unwrap();
    let listed = preview["communities"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["community_id"], current.to_string());

    let read_only = users
        .submit_for_mode(&subject, "reader", None, true)
        .await
        .unwrap();
    let deletion = users.submit(&subject, "operator", None).await.unwrap();
    for request in [&read_only, &deletion] {
        assert_eq!(request.inventory_manifest.as_ref(), Some(&preview));
        users
            .approve(
                request.id,
                "reviewer",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
    }
    // Losing the remaining membership after approval must not change the saved list.
    sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
        .bind(current)
        .bind(&subject)
        .execute(&fixture.pool)
        .await
        .unwrap();
    for (id, delete) in [(read_only.id, false), (deletion.id, true)] {
        let options = RunOptions {
            delete,
            ..fixture.options()
        };
        run(
            &fixture.store,
            id,
            "worker",
            &options,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let result = fixture.store.inspect(id).await.unwrap();
        assert_eq!(result.request.status, "completed");
        assert!(result
            .stage_history
            .iter()
            .any(|row| row.target_community_id == Some(current)));
        assert!(result
            .stage_history
            .iter()
            .all(|row| row.target_community_id != Some(former)));
        let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT community_id FROM events WHERE pubkey=decode($1,'hex') AND content='hello' ORDER BY community_id")
            .bind(&subject).fetch_all(&fixture.pool).await.unwrap();
        assert_eq!(
            remaining,
            if delete {
                vec![former]
            } else {
                vec![current, former]
            }
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn executor_completes_two_communities_and_completed_rerun_is_read_only() {
    let f = media_fixture::fixture().await;
    let store = &f.store;
    let id = f.id;
    sqlx::query("DELETE FROM relay_members WHERE pubkey = $1")
        .bind(store.get(id).await.unwrap().target)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(15),
            run(store, id, "worker", &f.options(), &CancellationToken::new())
        )
        .await
        .unwrap()
        .unwrap(),
        0
    );
    assert_eq!(store.get(id).await.unwrap().status, "completed");
    let before = serde_json::to_value(store.inspect(id).await.unwrap()).unwrap();
    assert_eq!(
        run(store, id, "worker", &f.options(), &CancellationToken::new())
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
    let f = media_fixture::fixture().await;
    let store = &f.store;
    let id = f.id;
    sqlx::query("DELETE FROM relay_members WHERE pubkey = $1")
        .bind(store.get(id).await.unwrap().target)
        .execute(&f.pool)
        .await
        .unwrap();
    let one = RunOptions {
        next_step: true,
        ..f.options()
    };
    let paused = store.inspect(id).await.unwrap();
    assert_eq!(paused.request.status, "running");
    assert!(paused.execution.lease_owner.is_none());
    assert_eq!(paused.stage_history.len(), 2);
    assert_eq!(paused.stage_history[1].stage, "purge_media");
    assert_eq!(paused.stage_history[1].status, "pending");
    let gate = serde_json::to_value(&paused.stage_history[0]).unwrap();
    for completed_handlers in 2..=9 {
        run(store, id, "worker-b", &one, &CancellationToken::new())
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
        if completed_handlers < 9 {
            assert_eq!(after.stage_history.len(), completed_handlers + 1);
            let current = after.stage_history.iter().find(|s| s.is_current).unwrap();
            assert_eq!(current.status, "pending");
            assert!(current.started_at.is_none());
        } else {
            assert_eq!(after.stage_history.len(), 9);
            assert!(!after.stage_history.iter().any(|s| s.is_current));
        }
    }
    assert_eq!(store.get(id).await.unwrap().status, "completed");
    assert_eq!(
        serde_json::to_value(&store.inspect(id).await.unwrap().stage_history[0]).unwrap(),
        gate
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn read_only_run_writes_selected_keys_and_preserves_source_rows() {
    let f = media_fixture::fixture_with_roles(&["member", "member", "owner"]).await;
    let ordinary = f.store.get(f.id).await.unwrap();
    let db = Db::from_pool(f.pool.clone());
    let preview = db
        .user_deletion_store()
        .submit_for_mode(&ordinary.target, "reader", None, true)
        .await
        .unwrap();
    assert_ne!(preview.id, ordinary.id);
    assert_eq!(preview.inventory_manifest, ordinary.inventory_manifest);
    f.store
        .approve(
            preview.id,
            "reviewer",
            preview.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let path = std::env::temp_dir().join(format!("buzz-rows-{}.ndjson", Uuid::new_v4()));
    let mut options = f.options();
    options.delete = false;
    options.row_ids = Some(super::super::row_ids::create(&path).unwrap());
    run(
        &f.store,
        preview.id,
        "reader",
        &options,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let done = f.store.inspect(preview.id).await.unwrap();
    assert_eq!(done.request.status, "completed");
    let rows = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let rows: Vec<serde_json::Value> = rows
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(rows
        .iter()
        .all(|r| r["request_id"] == preview.id.to_string() && r["mode"] == "read-only"));
    assert!(rows
        .iter()
        .any(|r| r["stage"] == "purge_media" && r["version"]["version_id"] == "v1"));
    assert!(rows.iter().any(|r| r["stage"] == "purge_account"
        && r["table"] == "relay_members"
        && r["key"]["pubkey"] == ordinary.target));
    let accounts: Vec<_> = done
        .stage_history
        .iter()
        .filter(|s| s.stage == "purge_account")
        .collect();
    assert_eq!(accounts.len(), 2);
    let owner = Uuid::from_u128(13);
    let stage = done
        .stage_history
        .iter()
        .find(|s| s.stage == "delete_owned_community")
        .unwrap();
    assert_eq!(
        stage.metadata_json["result"],
        serde_json::json!({"community_id":owner, "action":"would_submit"})
    );
    let proposals: Vec<_> = rows
        .iter()
        .filter(|r| r["stage"] == "delete_owned_community")
        .collect();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["community"], owner.to_string());
    assert_eq!(proposals[0]["action"], "would_submit");
    assert!(proposals[0].get("child_request_id").is_none());
    let children: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM community_deletion_requests WHERE community_id = $1",
    )
    .bind(owner)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(children, 0);
    assert!(accounts
        .iter()
        .all(|s| s.metadata_json["result"]["items"].as_u64().unwrap() > 0));
    assert_eq!(f.uploader_count().await, 1);
    assert!(!f.deleted.load(std::sync::atomic::Ordering::SeqCst));
    options.delete = true;
    assert!(run(
        &f.store,
        preview.id,
        "reader",
        &options,
        &CancellationToken::new()
    )
    .await
    .is_err());
}
