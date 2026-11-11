use super::*;
use crate::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use crate::{user_deletion::UserDeletionInventory, Db};
use std::time::Duration;

struct OutputBoundary {
    expire_at_proposal: bool,
    fail_receipt: bool,
    reached_proposal: bool,
}
impl Write for OutputBoundary {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(bytes);
        if text.contains("proposed_batch") {
            self.reached_proposal = true;
            if self.expire_at_proposal {
                // Deliberately outlive the one-second database lease while the
                // real batch owns the parent lock. This is an expiry test, not
                // a scheduling assumption about a competing worker.
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        if self.fail_receipt && text.contains("batch_committed") {
            return Err(std::io::Error::other("lost receipt"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn account_workflow_fences_scope_expiry_and_resumes_atomic_progress() {
    let (pool, store, options) = super::postgres_tests::fixture().await;
    let db = Db::from_pool(pool.clone());
    let entity = db.entity_deletion_store();
    let request = store
        .submit(&options.pubkey, "fixture", None)
        .await
        .unwrap();
    store
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let lease = entity
        .claim(request.id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(row) = entity
        .transition_stage(&lease, None, |request, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_account".into(),
                community: Some(options.community),
                inputs: UserDeletionInventory::read(request)?
                    .inputs(&request.target, Some(options.community))?,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("expected account stage")
    };
    entity.start_stage(&lease, row.id).await.unwrap();
    let before = super::postgres_tests::snapshot(&pool).await;
    store
        .preview_account_batches(&options, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(before, super::postgres_tests::snapshot(&pool).await);

    let mut stale = lease.clone();
    stale.generation -= 1;
    assert!(store
        .delete_account_stage(&stale, row.id, &options, &mut Vec::new())
        .await
        .is_err());
    assert!(store
        .delete_account_stage(&lease, Uuid::new_v4(), &options, &mut Vec::new())
        .await
        .is_err());
    let wrong = AccountBatchOptions {
        community: options.community,
        pubkey: "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5".into(),
        batch_size: 100,
        max_items: 100_000,
    };
    assert!(store
        .delete_account_stage(&lease, row.id, &wrong, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before, super::postgres_tests::snapshot(&pool).await);

    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()+interval '1 second' WHERE id=$1")
        .bind(request.id).execute(&pool).await.unwrap();
    let before_expiry = super::postgres_tests::snapshot(&pool).await;
    let mut expiry = OutputBoundary {
        expire_at_proposal: true,
        fail_receipt: false,
        reached_proposal: false,
    };
    let error = store
        .delete_account_stage(&lease, row.id, &options, &mut expiry)
        .await
        .unwrap_err();
    assert!(
        expiry.reached_proposal,
        "test must expire inside the actual batch"
    );
    assert!(
        error.to_string().contains("stale deletion lease"),
        "{error}"
    );
    assert_eq!(before_expiry, super::postgres_tests::snapshot(&pool).await);

    let next = entity
        .claim(request.id, "next-worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    assert!(next.generation > lease.generation);
    let mut lost = OutputBoundary {
        expire_at_proposal: false,
        fail_receipt: true,
        reached_proposal: false,
    };
    assert!(store
        .delete_account_stage(&next, row.id, &options, &mut lost)
        .await
        .is_err());
    let inspection = entity.inspect(request.id).await.unwrap();
    assert_eq!(
        inspection.stage_history[0].metadata_json["account_batches"]["items"],
        1
    );
    assert_eq!(inspection.stage_history[0].status, "running");
    let remaining = store
        .preview_account_batches(&options, &mut Vec::new())
        .await
        .unwrap()
        .items;
    let result = store
        .delete_account_stage(&next, row.id, &options, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(result.items, remaining);
    assert!(result.supported_scope_exhausted && !result.account_erasure_complete);
    let after = entity.inspect(request.id).await.unwrap();
    assert_eq!(
        after.stage_history[0].metadata_json["account_batches"]["items"],
        remaining + 1
    );
    assert_eq!(after.stage_history[0].status, "running");
    assert!(after.stage_history[0].metadata_json.get("result").is_none());
    assert_eq!(
        store
            .delete_account_stage(&next, row.id, &options, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(entity.inspect(request.id).await.unwrap()).unwrap()
    );
}
