//! Real coordinator and handler journey. Only the storage transport is faked.
use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rehearsal_resumes_real_previews_delegates_owner_and_finishes_without_deleting() {
    use nostr::{EventBuilder, Kind};
    let f = media_fixture::fixture().await;
    let db = Db::from_pool(f.pool.clone());
    let key = f.human.public_key().to_hex();
    let member = Uuid::from_u128(11);
    let owner = Uuid::from_u128(12);
    sqlx::query("UPDATE relay_members SET role='owner' WHERE community_id=$1 AND pubkey=$2")
        .bind(owner)
        .bind(&key)
        .execute(&f.pool)
        .await
        .unwrap();
    for community in [member, owner] {
        sqlx::query("INSERT INTO users(community_id,pubkey,display_name) VALUES($1,decode($2,'hex'),'rehearsal subject')")
            .bind(community).bind(&key).execute(&f.pool).await.unwrap();
    }
    for kind in [0, 1] {
        let event = EventBuilder::new(Kind::Custom(kind), "rehearsal fixture")
            .sign_with_keys(&f.human)
            .unwrap();
        sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(member).bind(event.id.to_bytes().as_slice()).bind(event.pubkey.to_bytes().as_slice())
            .bind(chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64,0).unwrap())
            .bind(i32::from(kind)).bind(serde_json::to_value(&event.tags).unwrap())
            .bind(&event.content).bind(event.sig.serialize().as_slice()).execute(&f.pool).await.unwrap();
    }
    // Use the actual submission API; parent adopts this unapproved child.
    let child = db
        .deletion_store()
        .submit(
            None,
            "operator",
            None,
            Some(buzz_core::CommunityId::from_uuid(owner)),
        )
        .await
        .unwrap();
    let users = db.user_deletion_store();
    let request = users
        .submit_rehearsal(&key, "operator", None, Uuid::new_v4())
        .await
        .unwrap();
    users
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let source = Db::connect_cold_start(
        buzz_db::DbConfig {
            database_url: url,
            default_transaction_read_only: true,
            max_connections: 1,
            min_connections: 0,
            ..Default::default()
        },
        "rehearsal_test",
    )
    .await
    .unwrap();
    let mut options = f.options();
    options.db = Some(source);
    options.delete = false;
    options.read_only = true;
    options.community_store = Some(db.deletion_store());
    options.next_step = true;
    let before = snapshot(&f.pool).await;
    let shutdown = CancellationToken::new();
    run(&f.store, request.id, "rehearsal", &options, &shutdown)
        .await
        .unwrap(); // actual dummy gate
    f.fail_listing.store(true, Ordering::SeqCst);
    assert!(run(&f.store, request.id, "rehearsal", &options, &shutdown)
        .await
        .is_err());
    let failed = f.store.inspect(request.id).await.unwrap();
    let failed_media = failed
        .stage_history
        .iter()
        .find(|s| s.stage == "purge_media")
        .unwrap();
    assert_eq!(failed_media.status, "failed");
    assert!(failed_media.metadata_json.get("result").is_none());
    let media_id = failed_media.id;
    f.fail_listing.store(false, Ordering::SeqCst);
    sqlx::query("UPDATE entity_deletion_requests SET next_attempt_at=now() WHERE id=$1")
        .bind(request.id)
        .execute(&f.pool)
        .await
        .unwrap();
    run(
        &f.store,
        request.id,
        "restarted-worker",
        &options,
        &shutdown,
    )
    .await
    .unwrap();
    let inspection = f.store.inspect(request.id).await.unwrap();
    let saved = inspection
        .stage_history
        .iter()
        .find(|s| s.id == media_id)
        .unwrap();
    assert_eq!(saved.status, "completed");
    let result = &saved.metadata_json["result"];
    assert_eq!(result["mode"], "read-only");
    assert_eq!(result["selected_files"], 1);
    assert_eq!(result["deleted_versions"], 0);
    assert_eq!(result["target_pubkeys"], serde_json::json!([key]));
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let file = &files[0];
    assert_eq!(file["file_hash"], "a".repeat(64));
    assert_eq!(file["filename"], format!("{}.png", "a".repeat(64)));
    assert_eq!(file["decision"], "delete");
    assert_eq!(
        file["reason"],
        "All known uploaders belong to the target set"
    );
    assert_eq!(file["versions"][0]["version_id"], "v1");
    assert_eq!(file["deleted_versions"], 0);
    assert_eq!(file["removed_uploader_rows"], 0);
    for _ in 0..20 {
        run(
            &f.store,
            request.id,
            "restarted-worker",
            &options,
            &shutdown,
        )
        .await
        .unwrap();
        if f.store.get(request.id).await.unwrap().status == "completed" {
            break;
        }
    }
    let done = f.store.inspect(request.id).await.unwrap();
    assert_eq!(done.request.status, "completed");
    assert!(done.execution.completed_at.is_some());
    assert!(done
        .stage_history
        .iter()
        .all(|s| s.status == "completed" && !s.is_current));
    assert_eq!(
        done.stage_history
            .iter()
            .find(|s| s.stage == "purge_media")
            .unwrap()
            .id,
        media_id
    );
    let delegated = done
        .stage_history
        .iter()
        .find(|s| s.stage == "delete_owned_community")
        .unwrap();
    assert_eq!(
        delegated.metadata_json["result"]["child_request_id"],
        child.id.to_string()
    );
    assert!(done
        .stage_history
        .iter()
        .filter(|s| s.target_community_id == Some(owner))
        .all(|s| s.stage == "delete_owned_community"));
    let profile = done
        .stage_history
        .iter()
        .find(|s| s.stage == "purge_profile" && s.target_community_id == Some(member))
        .unwrap();
    assert!(!profile.metadata_json["result"]["prerequisites"]
        .as_array()
        .unwrap()
        .is_empty());
    let report = done
        .stage_history
        .iter()
        .find(|s| s.stage == "remaining_cleanup")
        .unwrap();
    assert_eq!(report.metadata_json["result"]["record"], "rehearsal_report");
    assert_eq!(
        report.metadata_json["result"]["account_erasure_complete"],
        false
    );
    let media_summary = report.metadata_json["result"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["stage"] == "purge_media")
        .unwrap();
    assert_eq!(media_summary["id"], media_id.to_string());
    assert_eq!(media_summary["result"]["selected_files"], 1);
    assert!(media_summary["result"].get("files").is_none());
    assert_eq!(snapshot(&f.pool).await, before);
    assert!(!f.deleted.load(Ordering::SeqCst));
    assert!(f.requests.load(Ordering::SeqCst) > 0);
    let calls = f.requests.load(Ordering::SeqCst);
    run(&f.store, request.id, "terminal-rerun", &options, &shutdown)
        .await
        .unwrap();
    assert_eq!(calls, f.requests.load(Ordering::SeqCst));
    options.delete = true;
    options.read_only = false;
    assert!(run(&f.store, request.id, "wrong-mode", &options, &shutdown)
        .await
        .is_err());
    let child_after = db.deletion_store().get(child.id).await.unwrap();
    assert_eq!(
        child_after.stage,
        buzz_db::deletion::DeletionStage::Submitted
    );
    assert_eq!(child_after.inventory_digest, None);
    // A fresh rehearsal starts from the gate with its own history, and adopts
    // the same child rather than duplicating community-deletion intent.
    let again = users
        .submit_rehearsal(&key, "operator", None, Uuid::new_v4())
        .await
        .unwrap();
    users
        .approve(
            again.id,
            "approver",
            again.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    options.delete = false;
    options.read_only = true;
    options.next_step = false;
    run(&f.store, again.id, "fresh-run", &options, &shutdown)
        .await
        .unwrap();
    assert_eq!(f.store.get(again.id).await.unwrap().status, "completed");
    assert_eq!(snapshot(&f.pool).await, before);
}

async fn snapshot(pool: &sqlx::PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
        'users',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,pubkey) FROM users t),
        'members',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,pubkey) FROM relay_members t),
        'events',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,id) FROM events t),
        'uploads',(SELECT jsonb_agg(to_jsonb(t) ORDER BY community_id,sha256,pubkey) FROM media_uploaders t))")
        .fetch_one(pool).await.unwrap()
}
