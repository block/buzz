use super::*;
use crate::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use crate::media_deletion::SubjectScope;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn user_gate_drains_all_work_and_verifies_completed_scope_without_mutation() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let users = db.user_deletion_store();
    let subject = nostr::Keys::generate().public_key().to_hex();
    let scope = SubjectScope {
        human: subject.clone(),
        agents: Default::default(),
    };
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community)
        .bind(format!("{community}.example.org"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(community)
        .bind(&subject)
        .execute(&pool)
        .await
        .unwrap();
    let request = users.submit(&subject, "operator", None).await.unwrap();
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
        .claim(request.id, "worker", Duration::from_secs(300))
        .await
        .unwrap()
        .unwrap();
    let inventory = UserDeletionInventory::read(&request).unwrap();
    let NextStage::Stage(gate) = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "gate_user".into(),
                community: None,
                inputs: inventory.inputs(&subject, None)?,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("gate stage")
    };
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
    assert!(users.gate_user(&lease, gate.id, &scope).await.is_err());
    store.start_stage(&lease, gate.id).await.unwrap();

    // Non-upload work must prevent the gate from completing as well.
    let active = Uuid::new_v4();
    sqlx::query("INSERT INTO community_serving_write_leases(id,community_id,operation,owner,fence_generation,lease_until) VALUES($1,$2,'workflow','test',0,clock_timestamp()+interval '5 minutes')")
        .bind(active).bind(community).execute(&pool).await.unwrap();
    assert!(users.gate_user(&lease, gate.id, &scope).await.is_err());
    let first_fence: Uuid =
        sqlx::query_scalar("SELECT id FROM user_deletion_fences WHERE request_id=$1")
            .bind(request.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let metadata: serde_json::Value =
        sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE id=$1")
            .bind(gate.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(metadata.get("gate_ready").is_none());
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
    let mut changed = scope.clone();
    changed
        .agents
        .insert((community, nostr::Keys::generate().public_key().to_hex()));
    assert!(users.gate_user(&lease, gate.id, &changed).await.is_err());
    sqlx::query("DELETE FROM community_serving_write_leases WHERE id=$1")
        .bind(active)
        .execute(&pool)
        .await
        .unwrap();
    let result = users.gate_user(&lease, gate.id, &scope).await.unwrap();
    assert_eq!(result["drain_complete"], true);
    assert_eq!(result["online_write_fencing_complete"], false);
    assert_eq!(
        users.gate_user(&lease, gate.id, &scope).await.unwrap(),
        result
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM user_deletion_fences WHERE request_id=$1")
            .bind(request.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        first_fence
    );
    // A ready receipt is insufficient until the worker saves success and moves.
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
    store
        .save_stage_success(&lease, gate.id, result)
        .await
        .unwrap();
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
    let NextStage::Stage(media) = store
        .transition_stage(&lease, Some(gate.id), |_, _| {
            Ok(StageTransition::Advance {
                previous: gate.id,
                decision: json!({"kind":"stage","stage":"purge_media"}),
                successor: Some(NewEntityStage {
                    stage: "purge_media".into(),
                    community: None,
                    inputs: inventory.inputs(&subject, None)?,
                }),
            })
        })
        .await
        .unwrap()
    else {
        panic!("media stage")
    };
    let before = serde_json::to_value(store.inspect(request.id).await.unwrap()).unwrap();
    users.verify_user_gate(&lease, &scope).await.unwrap();
    assert_eq!(
        before,
        serde_json::to_value(store.inspect(request.id).await.unwrap()).unwrap()
    );
    assert!(users.verify_user_gate(&lease, &changed).await.is_err());
    // Media cannot install or rewrite the gate.
    store.start_stage(&lease, media.id).await.unwrap();
    assert!(users.gate_user(&lease, media.id, &scope).await.is_err());
    sqlx::query("DELETE FROM user_deletion_fences WHERE request_id=$1")
        .bind(request.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM user_deletion_fences WHERE request_id=$1"
        )
        .bind(request.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    store.release(&lease).await.unwrap();
    assert!(users.verify_user_gate(&lease, &scope).await.is_err());
}
