use super::*;
use crate::entity_deletion::{EntityLease, NewEntityStage, NextStage, StageTransition};
use std::time::Duration;

async fn prepare(f: &Fixture) -> (EntityLease, Uuid) {
    let db = Db::from_pool(f.pool.clone());
    let request = f.store().submit(TARGET, "submitter", None).await.unwrap();
    f.store()
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    let lease = store
        .claim(request.id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let inputs = crate::user_deletion::UserDeletionInventory::read(&request)
        .unwrap()
        .inputs(TARGET, Some(f.options.community))
        .unwrap();
    // Gate/content are outside this handler's scope; initialize an approved
    // account stage through the real transition/start APIs to exercise its guard.
    let next = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_account".into(),
                community: Some(f.options.community),
                inputs,
            }))
        })
        .await
        .unwrap();
    let NextStage::Stage(row) = next else {
        panic!("expected stage")
    };
    store.start_stage(&lease, row.id).await.unwrap();
    (lease, row.id)
}

async fn refused(f: &Fixture, lease: &EntityLease, id: Uuid) {
    let before = f.snapshot().await;
    assert!(f
        .store()
        .run_account_batches(
            &f.options,
            ExecutionMode::Delete,
            &mut Vec::new(),
            Some((lease, id))
        )
        .await
        .is_err());
    assert_eq!(f.snapshot().await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn approved_account_stage_runs_same_operations_without_advancing_workflow() {
    let f = Fixture::new().await;
    let (lease, id) = prepare(&f).await;
    let before = f.snapshot().await;
    let summary = f
        .store()
        .run_account_batches(
            &f.options,
            ExecutionMode::Delete,
            &mut Vec::new(),
            Some((&lease, id)),
        )
        .await
        .unwrap();
    assert_eq!(summary.items, 35);
    assert!(summary.supported_scope_exhausted && !summary.account_erasure_complete);
    f.assert_cleaned(&before, &[f.options.community]).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stale_expired_wrong_scope_or_unapproved_stages_cannot_mutate() {
    let mut f = Fixture::new().await;
    let (lease, id) = prepare(&f).await;
    let stale = EntityLease {
        generation: lease.generation - 1,
        ..lease.clone()
    };
    refused(&f, &stale, id).await;
    refused(&f, &lease, Uuid::new_v4()).await;
    let community = f.options.community;
    f.options.community = f.other;
    refused(&f, &lease, id).await;
    f.options.community = community;
    f.options.pubkey = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5".into();
    refused(&f, &lease, id).await;
    f.options.pubkey = TARGET.into();
    for assignment in [
        "status='pending'",
        "is_current=false",
        "stage='purge_content'",
        "metadata_json='{}'::jsonb",
    ] {
        let original: Value = sqlx::query_scalar(
            "SELECT to_jsonb(r) FROM entity_deletion_stage_history r WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE entity_deletion_stage_history SET {assignment} WHERE id=$1"
        )))
        .bind(id)
        .execute(&f.pool)
        .await
        .unwrap();
        refused(&f, &lease, id).await;
        sqlx::query("UPDATE entity_deletion_stage_history SET status=$2,is_current=$3,stage=$4,metadata_json=$5 WHERE id=$1")
            .bind(id).bind(original["status"].as_str().unwrap()).bind(original["is_current"].as_bool().unwrap())
            .bind(original["stage"].as_str().unwrap()).bind(&original["metadata_json"]).execute(&f.pool).await.unwrap();
    }
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1")
        .bind(lease.request_id).execute(&f.pool).await.unwrap();
    refused(&f, &lease, id).await;
    let renewed = Db::from_pool(f.pool.clone())
        .entity_deletion_store()
        .claim(lease.request_id, "replacement", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    refused(&f, &lease, id).await;
    sqlx::query("DELETE FROM entity_deletion_approvals WHERE request_id=$1")
        .bind(lease.request_id)
        .execute(&f.pool)
        .await
        .unwrap();
    refused(&f, &renewed, id).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn lease_expiring_after_selection_rolls_back_current_batch() {
    let f = Fixture::new().await;
    let (lease, id) = prepare(&f).await;
    sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()+interval '2 seconds' WHERE id=$1")
        .bind(lease.request_id).execute(&f.pool).await.unwrap();
    let before = f.snapshot().await;
    let (selected_tx, selected_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let store = f.store();
    let community = f.options.community;
    let request_id = lease.request_id;
    let task = tokio::spawn(async move {
        let mut selected = Some(selected_tx);
        let mut writer = failures_postgres_tests::RecordWriter::new(move |r: &Value| {
            if r["record"] == "proposed_batch" {
                if let Some(tx) = selected.take() {
                    let _ = tx.send(());
                    // Write is synchronous; let Tokio keep driving database
                    // I/O and the expiry observer while this barrier waits.
                    tokio::task::block_in_place(|| {
                        release_rx.recv_timeout(Duration::from_secs(10))
                    })
                    .map_err(std::io::Error::other)?;
                }
            }
            Ok(())
        });
        let options = AccountBatchOptions {
            community,
            pubkey: TARGET.into(),
            batch_size: 2,
            max_items: 1000,
        };
        store
            .run_account_batches(
                &options,
                ExecutionMode::Delete,
                &mut writer,
                Some((&lease, id)),
            )
            .await
    });
    let selected = tokio::time::timeout(Duration::from_secs(5), selected_rx).await;
    // Observe the actual DB-clock deadline, rather than assuming a sleep won a race.
    let expired = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let expired: bool = sqlx::query_scalar("SELECT lease_expires_at <= clock_timestamp() FROM entity_deletion_requests WHERE id=$1")
                .bind(request_id).fetch_one(&f.pool).await.unwrap();
            if expired { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    let _ = release_tx.send(());
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    selected.unwrap().unwrap();
    expired.unwrap();
    assert!(result.unwrap_err().to_string().contains("lease"));
    assert_eq!(f.snapshot().await, before);
}
