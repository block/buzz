use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rehearsal_submits_child_without_approval_then_adopts_it_in_another_run() {
    let f = fixture_mode(true).await;
    let (media, server) = submission_media().await;
    let outcome = handoff(&f.store, &f.lease, &f.row, &f.db.deletion_store(), &media)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(outcome, OwnedCommunityOutcome::HandedOff);
    let child = child(&f).await;
    assert_eq!(child.stage, DeletionStage::Inventoried);
    assert!(f
        .db
        .deletion_store()
        .inspect(child.id)
        .await
        .unwrap()
        .approval
        .is_none());
    let parent = f.store.inspect(f.row.request_id).await.unwrap();
    assert_eq!(parent.stage_history[0].status, "completed");
    assert_eq!(
        parent.stage_history[0].metadata_json["result"]["delegated"],
        true
    );
    assert_eq!(
        parent.stage_history[0].metadata_json["result"]["child_request_id"],
        child.id.to_string()
    );
    // Another run has independent stage state and reuses the same child. No
    // listing is allowed here: adoption must not refresh a child's inventory.
    let second = fixture_mode(true).await;
    assert_eq!(
        handoff(
            &second.store,
            &second.lease,
            &second.row,
            &second.db.deletion_store(),
            &lookup_only_media()
        )
        .await
        .unwrap(),
        OwnedCommunityOutcome::HandedOff
    );
    assert_eq!(
        second.db.deletion_store().get(child.id).await.unwrap(),
        child
    );
    // Existing intervention state is observable and remains a failure.
    sqlx::query(
        "UPDATE community_deletion_requests SET blocked_reason='fixture intervention' WHERE id=$1",
    )
    .bind(child.id)
    .execute(f.db.pool())
    .await
    .unwrap();
    let third = fixture_mode(true).await;
    assert!(handoff(
        &third.store,
        &third.lease,
        &third.row,
        &third.db.deletion_store(),
        &lookup_only_media()
    )
    .await
    .is_err());
    let failed = third.store.inspect(third.row.request_id).await.unwrap();
    assert_eq!(failed.stage_history[0].status, "failed");
    assert_eq!(
        failed.stage_history[0].metadata_json["waiting"]["blocked_reason"],
        "fixture intervention"
    );
}
