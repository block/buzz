use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn final_review_rechecks_only_outstanding_communities() {
    // A participant between the owners must still be processed in the first run.
    let f = media_fixture::fixture_with_roles(&["owner", "member", "owner"]).await;
    // The participant left after approval. Their token still requires cleanup
    // while the owned communities await their own approvals.
    let participant = Uuid::from_u128(12);
    let target = f.store.get(f.id).await.unwrap().target;
    sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(participant)
        .bind(&target)
        .execute(&f.pool)
        .await
        .unwrap();
    let token = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes)
        VALUES ($1,$2,decode($3,'hex'),$4,'fixture','[]')",
    )
    .bind(participant)
    .bind(token)
    .bind(&target)
    .bind(vec![1u8; 32])
    .execute(&f.pool)
    .await
    .unwrap();
    let options = f.options();
    let shutdown = CancellationToken::new();
    let mut completed_rows = std::collections::HashMap::new();
    let mut child_ids = Vec::new();
    for pending in [2, 2, 1, 0] {
        run(&f.store, f.id, "reviewer", &options, &shutdown)
            .await
            .unwrap();
        let saved = f.store.inspect(f.id).await.unwrap();
        let still_present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM api_tokens WHERE community_id = $1 AND id = $2)",
        )
        .bind(participant)
        .bind(token)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(!still_present);
        assert!(saved.execution.lease_owner.is_none());
        assert_eq!(saved.execution.retry_count, 0);
        assert_eq!(
            saved.request.status,
            if pending == 0 { "completed" } else { "running" }
        );
        let final_stage = saved
            .stage_history
            .iter()
            .find(|s| s.stage == "global_cleanup")
            .unwrap();
        assert_eq!(final_stage.is_current, pending != 0);
        let key = if pending == 0 { "result" } else { "waiting" };
        let left = final_stage.metadata_json[key]["summary"]["remaining"]
            .as_array()
            .unwrap();
        assert_eq!(left.len(), pending);
        for item in left {
            assert_eq!(item["reason"], "approval_required");
        }
        let mut submitted = 0;
        for row in &saved.stage_history {
            if let Some(previous) = completed_rows.get(&row.id) {
                assert_eq!(&serde_json::to_value(row).unwrap(), previous);
            }
            if row.status == "completed" {
                completed_rows.insert(row.id, serde_json::to_value(row).unwrap());
            } else if row.stage == "delete_owned_community" {
                assert_eq!(row.status, "submitted");
                assert!(row.finished_at.is_none());
                submitted += 1;
            }
        }
        assert_eq!(submitted, pending);
        let children: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM community_deletion_requests ORDER BY community_id")
                .fetch_all(&f.pool)
                .await
                .unwrap();
        assert_eq!(children.len(), 2);
        if child_ids.is_empty() {
            child_ids = children;
        } else {
            assert_eq!(children, child_ids);
            if pending > 0 {
                let child = child_ids[2 - pending];
                sqlx::query("UPDATE community_deletion_requests SET stage = 'retention_pending', completed_at = now() WHERE id = $1")
                    .bind(child).execute(&f.pool).await.unwrap();
            }
        }
    }
    let before = serde_json::to_value(f.store.inspect(f.id).await.unwrap()).unwrap();
    run(&f.store, f.id, "reviewer", &options, &shutdown)
        .await
        .unwrap();
    assert_eq!(
        before,
        serde_json::to_value(f.store.inspect(f.id).await.unwrap()).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn old_premature_completion_reopens_at_final_review() {
    let f = media_fixture::fixture_with_roles(&["owner", "owner"]).await;
    let options = f.options();
    let shutdown = CancellationToken::new();
    run(&f.store, f.id, "submit", &options, &shutdown)
        .await
        .unwrap();
    let before = f.store.inspect(f.id).await.unwrap();

    // Reproduce the previous release's saved terminal checkpoint. Leave both
    // child requests waiting for approval, as they were after submission.
    sqlx::query("UPDATE entity_deletion_stage_history SET status = 'completed',
        is_current = false, finished_at = now() WHERE request_id = $1 AND stage = 'delete_owned_community'")
        .bind(f.id).execute(&f.pool).await.unwrap();
    sqlx::query(
        "UPDATE entity_deletion_stage_history SET status = 'completed', is_current = false,
        finished_at = now(), metadata_json = (metadata_json - 'waiting') ||
        jsonb_build_object('result', (metadata_json->'waiting') - 'summary' - 'observed_at',
                           'transition', jsonb_build_object('kind', 'request_complete'))
        WHERE request_id = $1 AND stage = 'global_cleanup'",
    )
    .bind(f.id)
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET status = 'completed', completed_at = now() WHERE id = $1")
        .bind(f.id).execute(&f.pool).await.unwrap();

    run(&f.store, f.id, "recover", &options, &shutdown)
        .await
        .unwrap();
    let recovered = f.store.inspect(f.id).await.unwrap();
    assert_eq!(recovered.request.status, "running");
    assert!(recovered.execution.completed_at.is_none());
    assert_eq!(recovered.stage_history.len(), before.stage_history.len());
    for (old, new) in before.stage_history.iter().zip(&recovered.stage_history) {
        assert_eq!(old.id, new.id);
        assert_eq!(old.status, new.status);
        if old.status == "completed" {
            assert_eq!(old.updated_at, new.updated_at);
        }
    }
    let final_stage = recovered.stage_history.last().unwrap();
    assert_eq!(
        final_stage.metadata_json["waiting"]["summary"]["remaining"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn final_review_reports_missing_and_blocked_children_without_resubmission() {
    let f = media_fixture::fixture_with_roles(&["owner", "owner"]).await;
    let options = f.options();
    let shutdown = CancellationToken::new();
    run(&f.store, f.id, "submit", &options, &shutdown)
        .await
        .unwrap();

    sqlx::query("DELETE FROM community_deletion_requests WHERE community_id = $1")
        .bind(Uuid::from_u128(11))
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE community_deletion_requests SET blocked_at = now(), blocked_reason = 'requires intervention' WHERE community_id = $1")
        .bind(Uuid::from_u128(12)).execute(&f.pool).await.unwrap();

    run(&f.store, f.id, "review", &options, &shutdown)
        .await
        .unwrap();
    let saved = f.store.inspect(f.id).await.unwrap();
    assert_eq!(saved.request.status, "running");
    let final_stage = saved.stage_history.last().unwrap();
    let remaining = final_stage.metadata_json["waiting"]["summary"]["remaining"]
        .as_array()
        .unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining
        .iter()
        .any(|row| row["reason"] == "operator_intervention"));
    let missing = remaining
        .iter()
        .find(|row| row["reason"] == "submitted_request_missing")
        .unwrap();
    assert!(missing["child_request_id"].is_string());
    assert!(missing["child_stage"].is_null());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM community_deletion_requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
