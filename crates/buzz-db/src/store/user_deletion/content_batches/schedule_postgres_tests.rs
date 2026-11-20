use super::scrub_postgres_tests::{options, setup, PauseAfterPlan};
use super::workflow_postgres_tests::definition;
use super::*;
use crate::user_deletion::ExecutionMode;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unused_definition_and_claims_commit_atomically_or_refuse_small_batch() {
    let (db, c, _, alice, _) = setup().await;
    let workflow = Uuid::new_v4();
    definition(&db, c, workflow, &alice).await;
    let instant = Utc::now();
    assert!(db
        .claim_scheduled_workflow_fire(c, workflow, instant)
        .await
        .unwrap()
        .is_some());
    let store = db.user_deletion_store();
    let mut opts = options(c, &alice);
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let error = store
            .run_message_batches(&opts, mode, None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceed --batch-size"), "{error}");
        assert!(
            db.claim_scheduled_workflow_fire(c, workflow, instant)
                .await
                .unwrap()
                .is_none(),
            "refusal must preserve the dedupe claim"
        );
    }
    opts.batch_size = 2;
    let mut output = Vec::new();
    let preview = store
        .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut output)
        .await
        .unwrap();
    assert_eq!((preview.items, preview.batches), (2, 1));
    let error = store
        .run_message_batches(
            &opts,
            ExecutionMode::Delete,
            None,
            &mut super::private_state_postgres_tests::InterruptAfterCommit(Vec::new()),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("simulated lost output"),
        "{error}"
    );
    // A lost commit receipt must not expose an enabled workflow without a claim.
    for table in ["workflows", "scheduled_workflow_fires"] {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE community_id=$1"
        )))
        .bind(c.as_uuid())
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
    assert!(db
        .claim_scheduled_workflow_fire(c, workflow, instant)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn late_schedule_claim_rolls_back_the_entire_unused_workflow_group() {
    let (db, c, _, alice, _) = setup().await;
    let workflow = Uuid::new_v4();
    definition(&db, c, workflow, &alice).await;
    let instant = Utc::now();
    db.claim_scheduled_workflow_fire(c, workflow, instant)
        .await
        .unwrap();
    let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let store = db.user_deletion_store();
    let mut opts = options(c, &alice);
    opts.batch_size = 3;
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            record: "delete_start",
            line: Vec::new(),
            planned: Some(planned_tx),
            resume: resume_rx,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    planned_rx.await.unwrap();
    let later = instant + chrono::Duration::seconds(1);
    db.claim_scheduled_workflow_fire(c, workflow, later)
        .await
        .unwrap();
    resume_tx.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert!(
        error.to_string().contains("new dependent records"),
        "{error}"
    );
    // The planned claim deletion must roll back along with the parent.
    for scheduled in [instant, later] {
        assert!(db
            .claim_scheduled_workflow_fire(c, workflow, scheduled)
            .await
            .unwrap()
            .is_none());
    }
    let mut opts = options(c, &alice);
    opts.batch_size = 3;
    assert_eq!(
        db.user_deletion_store()
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        3
    );
}
