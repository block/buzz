use super::*;
use crate::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use std::{collections::BTreeMap, time::Duration};

fn candidate(hash: &str) -> MediaFile {
    MediaFile {
        storage_namespace: "fixture".into(),
        file_hash: hash.repeat(64),
        status: "unknown".into(),
        evidence_json: json!({"sources":[]}),
        delete_targets_json: json!([]),
        reason: None,
        last_error: None,
    }
}
async fn db() -> Db {
    Db::from_pool(
        PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn atomic_discovery_zero_inventory_and_stale_worker() {
    let db = db().await;
    let store = db.media_deletion_store();
    let id = Uuid::new_v4();
    let run = store
        .open_preview(id, json!({"mode":"print-only","subject":"fixture"}))
        .await
        .unwrap();
    assert!(store.load(&run).await.unwrap().is_none());
    let file = candidate("a");
    // Second insert violates uniqueness AFTER the first insert. Neither the row
    // nor the marker may survive that transaction's failure.
    assert!(store
        .publish(&run, &[file.clone(), file], json!({}))
        .await
        .is_err());
    assert!(store.load(&run).await.unwrap().is_none());
    store.publish(&run, &[], json!({"events":0})).await.unwrap();
    assert!(store.load(&run).await.unwrap().unwrap().is_empty());
    let replacement = store.open_preview(id, run.binding.clone()).await.unwrap();
    assert!(store.load(&run).await.is_err());
    store
        .publish(&replacement, &[candidate("b")], json!({}))
        .await
        .unwrap();
    assert!(store.load(&replacement).await.unwrap().unwrap().is_empty());
    assert!(store
        .open_preview(id, json!({"mode":"print"}))
        .await
        .is_err());
    assert!(db
        .entity_deletion_store()
        .claim(id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_cannot_authorize_deletion_and_preserves_control_evidence() {
    let db = db().await;
    let store = db.media_deletion_store();
    let run = store
        .open_preview(Uuid::new_v4(), json!({"mode":"print-only"}))
        .await
        .unwrap();
    let mut file = candidate("a");
    store
        .publish(&run, &[file.clone()], json!({}))
        .await
        .unwrap();
    file.status = "to_be_deleted".into();
    file.delete_targets_json = json!([{"key":"a.jpg","version_id":"v1"}]);
    assert!(store.save_preview_decision(&run, &file).await.is_err());
    assert!(store.save_execution_decision(&run, &file).await.is_err());
    assert!(store
        .save_execution_outcome(&run, &file, None)
        .await
        .is_err());
    let rows = store.load(&run).await.unwrap().unwrap();
    assert_eq!(rows[0].status, "unknown");
    file = rows[0].clone();
    file.status = "dont_delete".into();
    file.reason = Some("surviving private reference".into());
    store.save_preview_decision(&run, &file).await.unwrap();
    assert_eq!(
        store.load(&run).await.unwrap().unwrap()[0].status,
        "dont_delete"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn exact_execution_plan_survives_failure_and_cannot_be_replaced() {
    let db = db().await;
    let requests = db.entity_deletion_store();
    let request = db
        .user_deletion_store()
        .submit(
            &format!("demo:media-{}", Uuid::new_v4().simple()),
            "operator",
            None,
        )
        .await
        .unwrap();
    requests
        .approve(
            request.id,
            "reviewer",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let lease = requests
        .claim(request.id, "worker", Duration::from_secs(120))
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(stage) = requests
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_media".into(),
                community: None,
                inputs: json!({}),
            }))
        })
        .await
        .unwrap()
    else {
        panic!("missing stage")
    };
    requests.start_stage(&lease, stage.id).await.unwrap();
    let store = db.media_deletion_store();
    let run = store
        .execution_run(&lease, stage.id, json!({"scope":"fixture"}))
        .await
        .unwrap();
    let mut file = candidate("a");
    store
        .publish(&run, &[file.clone()], json!({}))
        .await
        .unwrap();
    file.status = "to_be_deleted".into();
    file.delete_targets_json = json!([{"key":format!("{}.jpg",file.file_hash),"version_id":"v1"}]);
    store.save_execution_decision(&run, &file).await.unwrap();
    let mut changed = file.clone();
    changed.delete_targets_json = json!([{"key":"replacement","version_id":"v2"}]);
    assert!(store.save_execution_decision(&run, &changed).await.is_err());
    assert!(store
        .save_execution_outcome(&run, &changed, None)
        .await
        .is_err());
    store
        .save_execution_outcome(&run, &file, Some("timeout after external delete"))
        .await
        .unwrap();
    let row = store.load(&run).await.unwrap().unwrap().remove(0);
    assert_eq!(row.status, "to_be_deleted");
    assert_eq!(row.delete_targets_json, file.delete_targets_json);
    store
        .save_execution_outcome(&run, &file, None)
        .await
        .unwrap();
    assert_eq!(
        store.load(&run).await.unwrap().unwrap()[0].status,
        "deleted"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn source_scan_finds_unposted_uploads_private_survivors_and_scoped_agents() {
    let db = db().await;
    let pool = db.pool();
    let human = nostr::Keys::generate().public_key().to_hex();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let other = nostr::Keys::generate().public_key().to_hex();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    for c in [a, b] {
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(c)
            .bind(format!("{c}.example.org"))
            .execute(pool)
            .await
            .unwrap();
        for key in [&human, &agent, &other] {
            sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,decode($2,'hex'))")
                .bind(c)
                .bind(key)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    sqlx::query("UPDATE users SET agent_owner_pubkey=decode($2,'hex') WHERE community_id=$1 AND pubkey=decode($3,'hex')").bind(a).bind(&human).bind(&agent).execute(pool).await.unwrap();
    // No membership rows: historical source discovery must still find these.
    for (c, key, hash) in [
        (a, &human, "a"),
        (a, &agent, "b"),
        (b, &agent, "b"),
        (a, &human, "c"),
    ] {
        sqlx::query("INSERT INTO media_uploaders (community_id,pubkey,sha256) VALUES ($1,decode($2,'hex'),$3)").bind(c).bind(key).bind(hash.repeat(64)).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE users SET avatar_url=$3 WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(b)
        .bind(&other)
        .bind(format!("/media/{}.jpg", "a".repeat(64)))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET about=$3 WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(a)
        .bind(&human)
        .bind(format!("only a reference /media/{}.bin", "d".repeat(64)))
        .execute(pool)
        .await
        .unwrap();
    let scan = db
        .scan_user_media(&human, true, "fixture", &[])
        .await
        .unwrap();
    let files: BTreeMap<_, _> = scan
        .files
        .iter()
        .map(|f| (f.file_hash.clone(), f))
        .collect();
    for hash in ["a", "b", "c", "d"] {
        assert!(files.contains_key(&hash.repeat(64)));
    }
    assert_eq!(scan.scope.agents.len(), 1);
    for hash in ["a", "b"] {
        assert!(files[&hash.repeat(64)].evidence_json["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["surviving"] == true));
    }
    assert!(!files[&"c".repeat(64)].evidence_json["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["surviving"] == true));
    assert!(scan.scanned.contains_key("events"));
}
