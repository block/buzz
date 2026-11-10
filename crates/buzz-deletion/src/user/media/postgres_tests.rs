use super::*;
use buzz_db::{
    entity_deletion::{NewEntityStage, NextStage, StageTransition},
    Db,
};
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

struct Backend {
    present: Mutex<BTreeSet<String>>,
    fail_after_delete: AtomicBool,
    protected: AtomicBool,
    deletes: AtomicUsize,
    cleanups: AtomicUsize,
}
impl ProtectedMedia for Backend {
    async fn verify(&self, _: &MediaRun) -> Result<()> {
        if !self.protected.load(Ordering::SeqCst) {
            bail!("protection lost");
        }
        Ok(())
    }
    async fn decide(&self, _: &MediaRun, file: &MediaFile) -> Result<MediaFile> {
        let mut file = file.clone();
        file.status = "to_be_deleted".into();
        file.delete_targets_json = json!([{"key":format!("{}.jpg",file.file_hash),"version_id":"v1"},{"key":format!("{}.thumb.webp",file.file_hash),"version_id":"v2"}]);
        Ok(file)
    }
    async fn present(&self, t: &MediaTarget) -> Result<bool> {
        Ok(self.present.lock().unwrap().contains(&t.version_id))
    }
    async fn delete(&self, t: &MediaTarget) -> Result<()> {
        self.present.lock().unwrap().remove(&t.version_id);
        self.deletes.fetch_add(1, Ordering::SeqCst);
        if self.fail_after_delete.swap(false, Ordering::SeqCst) {
            bail!("lost acknowledgement");
        }
        Ok(())
    }
    async fn cleanup(&self, _: &MediaRun) -> Result<()> {
        self.cleanups.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn print_only_skips_deletion_and_crash_recovery_uses_saved_versions() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap();
    let db = Db::from_pool(sqlx::PgPool::connect(&url).await.unwrap());
    let request = db
        .user_deletion_store()
        .submit(
            &format!("demo:media-loop-{}", Uuid::new_v4().simple()),
            "operator",
            None,
        )
        .await
        .unwrap();
    let requests = db.entity_deletion_store();
    requests
        .approve(
            request.id,
            "approver",
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
    store
        .publish(
            &run,
            &[MediaFile {
                storage_namespace: "fixture".into(),
                file_hash: "a".repeat(64),
                status: "unknown".into(),
                evidence_json: json!({"sources":[]}),
                delete_targets_json: json!([]),
                reason: None,
                last_error: None,
            }],
            json!({}),
        )
        .await
        .unwrap();
    let backend = Backend {
        present: Mutex::new(BTreeSet::from(["v1".into(), "v2".into()])),
        fail_after_delete: AtomicBool::new(true),
        protected: AtomicBool::new(true),
        deletes: AtomicUsize::new(0),
        cleanups: AtomicUsize::new(0),
    };
    handle(&store, &run, Mode::PrintOnly, &backend)
        .await
        .unwrap();
    assert_eq!(backend.deletes.load(Ordering::SeqCst), 0);
    assert_eq!(backend.cleanups.load(Ordering::SeqCst), 0);
    backend.protected.store(false, Ordering::SeqCst);
    assert!(handle(&store, &run, Mode::NoPrint, &backend).await.is_err());
    assert_eq!(backend.deletes.load(Ordering::SeqCst), 0);
    backend.protected.store(true, Ordering::SeqCst);
    assert!(handle(&store, &run, Mode::NoPrint, &backend).await.is_err());
    let pending = store.load(&run).await.unwrap().unwrap().remove(0);
    assert_eq!(pending.status, "to_be_deleted");
    assert!(pending.last_error.is_some());
    assert_eq!(backend.cleanups.load(Ordering::SeqCst), 0);
    handle(&store, &run, Mode::NoPrint, &backend).await.unwrap();
    assert_eq!(
        store.load(&run).await.unwrap().unwrap()[0].status,
        "deleted"
    );
    assert_eq!(backend.deletes.load(Ordering::SeqCst), 2); // v1 was not deleted twice.
    assert_eq!(backend.cleanups.load(Ordering::SeqCst), 1);
}
