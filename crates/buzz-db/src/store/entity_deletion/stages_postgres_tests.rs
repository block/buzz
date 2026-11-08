use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stage_storage_accepts_caller_policy_and_rolls_back_invalid_decisions() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let request = db
        .user_deletion_store()
        .submit("demo:generic-storage", "operator", None)
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    store
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let lease = store
        .claim(request.id, "worker", Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let before = serde_json::to_value(store.inspect(request.id).await.unwrap()).unwrap();
    assert!(store
        .transition_stage(&lease, None, |_, _| Err(invalid("policy refused")))
        .await
        .is_err());
    assert_eq!(
        serde_json::to_value(store.inspect(request.id).await.unwrap()).unwrap(),
        before
    );
    let NextStage::Stage(row) = store
        .transition_stage(&lease, None, |request, history| {
            assert!(history.is_empty());
            assert_eq!(request.entity_type, "user");
            assert_eq!(request.status, "approved");
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "arbitrary_policy_stage".into(),
                community: None,
                inputs: serde_json::json!({"example": true}),
            }))
        })
        .await
        .unwrap()
    else {
        panic!("expected stage");
    };
    assert_eq!(row.stage, "arbitrary_policy_stage");
    assert!(store
        .transition_stage(&lease, Some(row.id), |_, _| Ok(StageTransition::Advance {
            previous: row.id,
            decision: serde_json::json!({"kind": "done"}),
            successor: None,
        }))
        .await
        .is_err()); // pending work cannot be skipped by an invalid decision
    store.start_stage(&lease, row.id).await.unwrap();
    store
        .save_stage_success(&lease, row.id, serde_json::json!({"done": true}))
        .await
        .unwrap();
    assert!(matches!(
        store
            .transition_stage(&lease, Some(row.id), |_, history| {
                assert_eq!(history[0].metadata_json["result"]["done"], true);
                Ok(StageTransition::Advance {
                    previous: row.id,
                    decision: serde_json::json!({"kind": "done"}),
                    successor: None,
                })
            })
            .await
            .unwrap(),
        NextStage::Finished
    ));
    assert_eq!(store.get(request.id).await.unwrap().status, "completed");
    assert!(store.release(&lease).await.unwrap());
}
