//! Workflow callers must use the attribution saved with the ordinary inventory.
use super::scrub_postgres_tests::{message, options, row, setup};
use crate::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use crate::user_deletion::{ExecutionMode, UserDeletionInventory};
use nostr::Keys;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn leased_content_uses_frozen_inventory_attribution_even_without_batches() {
    let (db, community, channel, alice, _) = setup().await;
    let human = alice.public_key().to_hex();
    let relay = Keys::generate().public_key().to_hex();
    let other = Keys::generate().public_key().to_hex();
    let users = db.user_deletion_store();
    let request = users
        .submit_with_relay(&human, "operator", None, Some(&relay))
        .await
        .unwrap();
    users
        .approve(
            request.id,
            "operator",
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
    let inputs = UserDeletionInventory::read(&request)
        .unwrap()
        .inputs(&human, Some(*community.as_uuid()))
        .unwrap();
    let NextStage::Stage(stage) = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_content".into(),
                community: Some(*community.as_uuid()),
                inputs,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("expected content stage")
    };
    store.start_stage(&lease, stage.id).await.unwrap();
    let mut opts = options(community, &alice);
    for with_content in [false, true] {
        let event = if with_content {
            Some(message(&db, community, channel, &alice, "retain on mismatch", None).await)
        } else {
            None
        };
        for supplied in [None, Some(other.clone())] {
            opts.relay_pubkey = supplied;
            let error = users
                .run_message_batches(
                    &opts,
                    ExecutionMode::Delete,
                    Some((&lease, stage.id)),
                    &mut Vec::new(),
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("relay key"), "{error}");
            if let Some(event) = &event {
                assert_eq!(
                    row(&db, community, event).await["content"],
                    "retain on mismatch"
                );
            }
        }
    }
    opts.relay_pubkey = Some(relay);
    let done = users
        .run_message_batches(
            &opts,
            ExecutionMode::Delete,
            Some((&lease, stage.id)),
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert!(done.items > 0);
}
