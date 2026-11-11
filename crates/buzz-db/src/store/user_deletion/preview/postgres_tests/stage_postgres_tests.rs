use super::*;
use crate::entity_deletion::{
    EntityLease, EntityStage, NewEntityStage, NextStage, StageTransition,
};
use std::time::Duration;

async fn content_stage(db: &Db, options: &PreviewOptions) -> (EntityLease, EntityStage, Uuid) {
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(options.community)
        .bind(HUMAN)
        .execute(db.pool())
        .await
        .unwrap();
    let users = db.user_deletion_store();
    let request = users.submit(HUMAN, "operator", None).await.unwrap();
    users
        .approve(
            request.id,
            "reviewer",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    let lease = store
        .claim(request.id, "worker", Duration::from_secs(120))
        .await
        .unwrap()
        .unwrap();
    let inventory = crate::user_deletion::UserDeletionInventory::read(&request).unwrap();
    let NextStage::Stage(media) = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_media".into(),
                community: None,
                inputs: inventory.inputs(HUMAN, None)?,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("missing media stage")
    };
    store.start_stage(&lease, media.id).await.unwrap();
    let media_store = db.media_deletion_store();
    let run = media_store
        .execution_run(
            &lease,
            media.id,
            json!({
                "mode":"delete", "human":HUMAN, "storage_namespace":"fixture", "media_bases":[]
            }),
        )
        .await
        .unwrap();
    let scan = db
        .scan_user_media(HUMAN, false, "fixture", &[])
        .await
        .unwrap();
    media_store
        .publish(
            &run,
            &scan.files,
            json!({"scope":scan.scope,"tables":scan.scanned}),
        )
        .await
        .unwrap();
    // The storage boundary retains every fixture file. The content adapter must
    // still consume the real scan's exact fingerprints and terminal decisions.
    for mut file in scan.files {
        file.status = "dont_delete".into();
        file.reason = Some("fixture storage retention".into());
        media_store
            .save_execution_decision(&run, &file)
            .await
            .unwrap();
    }
    store
        .save_stage_success(&lease, media.id, json!({"media_complete":true}))
        .await
        .unwrap();
    let NextStage::Stage(stage) = store
        .transition_stage(&lease, Some(media.id), |_, _| {
            Ok(StageTransition::Advance {
                previous: media.id,
                decision: json!({"kind":"stage","stage":"purge_content"}),
                successor: Some(NewEntityStage {
                    stage: "purge_content".into(),
                    community: Some(options.community),
                    inputs: inventory.inputs(HUMAN, Some(options.community))?,
                }),
            })
        })
        .await
        .unwrap()
    else {
        panic!("missing content stage")
    };
    let stage = store.start_stage(&lease, stage.id).await.unwrap();
    (lease, stage, media.id)
}

async fn set_media(pool: &PgPool, community: Uuid, event: u32, caption: &str) {
    sqlx::query("UPDATE events SET content=$3 WHERE community_id=$1 AND id=decode($2,'hex')")
        .bind(community)
        .bind(id(event))
        .bind(format!("{caption} /media/{}.png", "a".repeat(64)))
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn content_stage_requires_exact_media_evidence_and_resumes_atomic_progress() {
    let (pool, mut options) = setup().await;
    options.batch_size = 1;
    let (_, other) = setup().await;
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    event(&pool, options.community, 2, HUMAN, 9, json!([])).await;
    event(&pool, options.community, 3, OTHER, 9, json!([])).await;
    event(&pool, other.community, 1, HUMAN, 9, json!([])).await;
    set_media(&pool, options.community, 1, "before").await;
    set_media(&pool, options.community, 3, "survivor").await;
    let db = Db::from_pool(pool.clone());
    let (lease, stage, media) = content_stage(&db, &options).await;
    let users = db.user_deletion_store();
    // Standalone deletion cannot borrow workflow evidence implicitly.
    assert!(users
        .delete_message_batches(&options, &mut Vec::new())
        .await
        .is_err());
    let before = serde_json::to_value(
        db.entity_deletion_store()
            .inspect(lease.request_id)
            .await
            .unwrap(),
    )
    .unwrap();
    users
        .preview_message_batches(&options, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(
        before,
        serde_json::to_value(
            db.entity_deletion_store()
                .inspect(lease.request_id)
                .await
                .unwrap()
        )
        .unwrap()
    );
    // A changed caption with the same media hash must not reuse old evidence.
    set_media(&pool, options.community, 1, "changed").await;
    let error = users
        .delete_message_stage(&lease, stage.id, &options, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("source changed"), "{error}");
    assert_eq!(preview(&pool, &options).await.0.items, 2);
    set_media(&pool, options.community, 1, "before").await;
    // A missing journal row cannot masquerade as an empty completed inventory.
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{media_discovery,candidate_count}','2') WHERE id=$1")
        .bind(media).execute(&pool).await.unwrap();
    assert!(users
        .delete_message_stage(&lease, stage.id, &options, &mut Vec::new())
        .await
        .unwrap_err()
        .to_string()
        .contains("row count"));
    sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{media_discovery,candidate_count}','1') WHERE id=$1")
        .bind(media).execute(&pool).await.unwrap();
    struct StopAfterCommit {
        bytes: Vec<u8>,
        stopped: bool,
    }
    impl Write for StopAfterCommit {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.stopped {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if String::from_utf8_lossy(&self.bytes).contains("batch_committed") {
                self.stopped = true;
            }
            Ok(())
        }
    }
    let mut output = StopAfterCommit {
        bytes: Vec::new(),
        stopped: false,
    };
    assert!(users
        .delete_message_stage(&lease, stage.id, &options, &mut output)
        .await
        .is_err());
    let progress: Value = sqlx::query_scalar(
        "SELECT metadata_json->'content_batches' FROM entity_deletion_stage_history WHERE id=$1",
    )
    .bind(stage.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(progress["items"], 1);
    assert_eq!(progress["batches"], 1);
    options.relay_pubkey = Some(RELAY.into());
    assert!(users
        .delete_message_stage(&lease, stage.id, &options, &mut Vec::new())
        .await
        .unwrap_err()
        .to_string()
        .contains("semantic options changed"));
    options.relay_pubkey = None;
    // Resource limits may change on retry; the semantic selection may not.
    options.batch_size = 2;
    let store = db.entity_deletion_store();
    assert!(store.release(&lease).await.unwrap());
    let next = store
        .claim(lease.request_id, "next-worker", Duration::from_secs(120))
        .await
        .unwrap()
        .unwrap();
    assert!(users
        .delete_message_stage(&lease, stage.id, &options, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(
        users
            .delete_message_stage(&next, stage.id, &options, &mut Vec::new())
            .await
            .unwrap()
            .items,
        1
    );
    assert_eq!(
        users
            .delete_message_stage(&next, stage.id, &options, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    let progress: (Value, String) = sqlx::query_as("SELECT metadata_json->'content_batches',status FROM entity_deletion_stage_history WHERE id=$1")
        .bind(stage.id).fetch_one(&pool).await.unwrap();
    assert_eq!(progress.0["items"], 2);
    assert_eq!(progress.0["batches"], 2);
    assert_eq!(progress.0["account_erasure_complete"], false);
    assert_eq!(progress.1, "running");
    let surviving: Vec<String> =
        sqlx::query_scalar("SELECT encode(id,'hex') FROM events WHERE community_id=$1 ORDER BY id")
            .bind(options.community)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(surviving, [id(3)]);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(other.community)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn content_stage_expiry_after_print_rolls_back_rows_and_progress() {
    let (pool, options) = setup().await;
    event(&pool, options.community, 1, HUMAN, 9, json!([])).await;
    let db = Db::from_pool(pool.clone());
    let (lease, stage, _) = content_stage(&db, &options).await;
    // Leave enough time for preflight, then expire while the first proposed
    // batch is being printed. No takeover is needed to revoke this lease.
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()+interval '2 seconds' WHERE id=$1")
        .bind(lease.request_id).execute(&pool).await.unwrap();
    struct SlowBatch {
        bytes: Vec<u8>,
        delayed: bool,
    }
    impl Write for SlowBatch {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if !self.delayed && String::from_utf8_lossy(&self.bytes).contains("would_delete") {
                self.delayed = true;
                std::thread::sleep(Duration::from_secs(3));
            }
            Ok(())
        }
    }
    let error = db
        .user_deletion_store()
        .delete_message_stage(
            &lease,
            stage.id,
            &options,
            &mut SlowBatch {
                bytes: Vec::new(),
                delayed: false,
            },
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("stale deletion lease"),
        "{error}"
    );
    assert_eq!(preview(&pool, &options).await.0.items, 1);
    let progress: Value = sqlx::query_scalar("SELECT COALESCE(metadata_json->'content_batches','null'::jsonb) FROM entity_deletion_stage_history WHERE id=$1")
        .bind(stage.id).fetch_one(&pool).await.unwrap();
    assert!(progress.is_null());
}
