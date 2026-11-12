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
    store.save_preview_decision(&run, &file).await.unwrap();
    assert!(store.save_execution_decision(&run, &file).await.is_err());
    assert!(store
        .save_execution_outcome(&run, &file, None)
        .await
        .is_err());
    let rows = store.load(&run).await.unwrap().unwrap();
    assert_eq!(rows[0].status, "to_be_deleted");
    assert_eq!(rows[0].delete_targets_json, file.delete_targets_json);
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
    store.verify_execution(&run).await.unwrap();
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
    requests.release(&lease).await.unwrap();
    assert!(store.verify_execution(&run).await.is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn source_scan_uses_uploaders_and_audit_ignores_links_and_scopes_agents() {
    let db = Db::from_pool(
        sqlx::postgres::PgPoolOptions::new()
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET TIME ZONE 'UTC'")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&crate::test_support::database_url())
            .await
            .unwrap(),
    );
    let pool = db.pool();
    let human_keys = nostr::Keys::generate();
    let human = human_keys.public_key().to_hex();
    let agent = nostr::Keys::generate().public_key().to_hex();
    let other = nostr::Keys::generate().public_key().to_hex();
    let hashes: Vec<_> = (0..5)
        .map(|_| Uuid::new_v4().simple().to_string().repeat(2))
        .collect();
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
        (a, &human, &hashes[0]),
        (a, &agent, &hashes[1]),
        (b, &agent, &hashes[1]),
        (a, &human, &hashes[2]),
    ] {
        sqlx::query("INSERT INTO media_uploaders (community_id,pubkey,sha256) VALUES ($1,decode($2,'hex'),$3)").bind(c).bind(key).bind(hash).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE users SET avatar_url=$3 WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(b)
        .bind(&other)
        .bind(format!("/media/{}.jpg", hashes[0]))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET about=$3 WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(a)
        .bind(&human)
        .bind(format!("only a reference /media/{}.bin", hashes[3]))
        .execute(pool)
        .await
        .unwrap();
    let event = nostr::EventBuilder::new(
        nostr::Kind::TextNote,
        format!("attachment /media/{}.jpg", hashes[2]),
    )
    .sign_with_keys(&human_keys)
    .unwrap();
    db.insert_event(buzz_core::CommunityId::from_uuid(a), &event, None)
        .await
        .unwrap();
    sqlx::query("INSERT INTO audit_log (community_id,seq,hash,action,actor_pubkey,object_id,detail) VALUES ($1,1,decode($2,'hex'),'media_uploaded',decode($3,'hex'),$2,jsonb_build_object('sha256',$2::text))")
        .bind(a).bind(&hashes[4]).bind(&human).execute(pool).await.unwrap();
    let scan = db.scan_user_media(&human, true, "fixture").await.unwrap();
    let alternate_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET TIME ZONE 'America/New_York'")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let alternate = crate::Db::from_pool(alternate_pool)
        .scan_user_media(&human, true, "fixture")
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&scan.files).unwrap(),
        serde_json::to_value(&alternate.files).unwrap(),
        "event timestamp keys and fingerprints must not depend on the session timezone"
    );
    let files: BTreeMap<_, _> = scan
        .files
        .iter()
        .map(|f| (f.file_hash.clone(), f))
        .collect();
    for hash in [&hashes[0], &hashes[1], &hashes[2], &hashes[4]] {
        assert!(files.contains_key(hash));
    }
    assert!(
        !files.contains_key(&hashes[3]),
        "a copied link is not upload attribution"
    );
    assert_eq!(scan.scope.agents.len(), 1);
    for (hash, kept) in [(&hashes[0], false), (&hashes[1], true), (&hashes[2], false)] {
        assert_eq!(
            files[hash].evidence_json["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surviving"] == true),
            kept
        );
    }
    assert!(!scan.scanned.contains_key("events"));
    assert!(!scan.scanned.contains_key("users"));
    let candidates = vec![hashes[0].clone(), hashes[4].clone()];
    let targeted = db
        .scan_media_uploads(
            &scan.scope,
            "fixture",
            Some(&candidates),
            MediaScanBudget::default(),
        )
        .await
        .unwrap();
    assert_eq!(targeted.files.len(), 2);
    assert!(targeted
        .files
        .iter()
        .all(|f| candidates.contains(&f.file_hash)));
    // A second actual upload changes the evidence; a copied avatar above did not.
    sqlx::query(
        "INSERT INTO media_uploaders (community_id,pubkey,sha256) VALUES ($1,decode($2,'hex'),$3)",
    )
    .bind(b)
    .bind(&other)
    .bind(&hashes[0])
    .execute(pool)
    .await
    .unwrap();
    let targeted = db
        .scan_media_uploads(
            &scan.scope,
            "fixture",
            Some(&candidates),
            MediaScanBudget::default(),
        )
        .await
        .unwrap();
    assert!(targeted
        .files
        .iter()
        .find(|f| f.file_hash == hashes[0])
        .unwrap()
        .evidence_json["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["surviving"] == true));
}
