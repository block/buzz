use super::*;
use crate::store::{DeliveryCursor, Mutation, NewMessage, Provider};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

fn put(integration_id: Uuid, revision: i64) -> Operation {
    Operation {
        operation_id: Uuid::new_v4(),
        integration_id,
        revision,
        mutation: Mutation::PutIntegration {
            community_host: "acme.communities.example.org".into(),
            provider: Provider::Github,
        },
    }
}

fn secret(integration_id: Uuid, revision: i64, grace_secs: i64) -> Operation {
    Operation {
        operation_id: Uuid::new_v4(),
        integration_id,
        revision,
        mutation: Mutation::PutSecret {
            key_id: "k1".into(),
            ciphertext: vec![revision as u8; 8],
            grace: Duration::seconds(grace_secs),
        },
    }
}

#[tokio::test]
async fn operations_apply_once_and_replay_the_stored_response() {
    let store = MemoryStore::default();
    let id = Uuid::new_v4();
    let op = put(id, 1);
    let first = store.apply(op.clone(), at(0)).await.unwrap();
    let ApplyOutcome::Applied(applied) = first else {
        panic!("{first:?}");
    };
    assert_eq!(applied.response["revision"], 1);
    assert_eq!(applied.response["has_secret"], false);
    let again = store.apply(op.clone(), at(5)).await.unwrap();
    assert_eq!(again, ApplyOutcome::Replayed(applied.clone()));
    let mut reused = put(id, 2);
    reused.operation_id = op.operation_id;
    assert_eq!(
        store.apply(reused, at(6)).await.unwrap(),
        ApplyOutcome::Conflict
    );
    let mut other_integration = put(Uuid::new_v4(), 1);
    other_integration.operation_id = op.operation_id;
    assert_eq!(
        store.apply(other_integration, at(7)).await.unwrap(),
        ApplyOutcome::Conflict
    );
    assert_eq!(
        store.applied_operation(applied.operation_id).await.unwrap(),
        Some(applied)
    );
}

#[tokio::test]
async fn stale_revisions_and_missing_integrations_are_reported() {
    let store = MemoryStore::default();
    let id = Uuid::new_v4();
    assert_eq!(
        store.apply(secret(id, 1, 0), at(0)).await.unwrap(),
        ApplyOutcome::NotFound
    );
    store.apply(put(id, 3), at(0)).await.unwrap();
    assert_eq!(
        store.apply(put(id, 3), at(1)).await.unwrap(),
        ApplyOutcome::StaleRevision { current: 3 }
    );
    assert_eq!(
        store.apply(secret(id, 2, 0), at(1)).await.unwrap(),
        ApplyOutcome::StaleRevision { current: 3 }
    );
    assert!(store.integration(id).await.unwrap().unwrap().enabled);
}

#[tokio::test]
async fn secret_rotation_keeps_the_old_secret_through_grace_only() {
    let store = MemoryStore::default();
    let id = Uuid::new_v4();
    store.apply(put(id, 1), at(0)).await.unwrap();
    store.apply(secret(id, 2, 0), at(10)).await.unwrap();
    let ApplyOutcome::Applied(rotated) = store.apply(secret(id, 3, 60), at(20)).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(rotated.response["has_secret"], true);
    assert_eq!(
        rotated.response["rotating_until"],
        serde_json::json!(at(80))
    );
    let during = store.verifying_secrets(id, at(50)).await.unwrap();
    assert_eq!(during.len(), 2);
    assert_eq!(during[0].ciphertext, vec![3u8; 8]);
    let after = store.verifying_secrets(id, at(80)).await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].ciphertext, vec![3u8; 8]);

    let ApplyOutcome::Applied(revoked) = store.apply(secret(id, 4, 0), at(90)).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(revoked.response["rotating_until"], serde_json::Value::Null);
    assert_eq!(store.verifying_secrets(id, at(90)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn claims_lease_due_rows_oldest_first_and_hide_them_until_the_lease_ends() {
    let store = MemoryStore::default();
    let integration_id = Uuid::new_v4();
    store
        .apply(
            Operation {
                operation_id: Uuid::new_v4(),
                integration_id,
                revision: 1,
                mutation: Mutation::PutIntegration {
                    community_host: "acme.communities.example.org".into(),
                    provider: Provider::Github,
                },
            },
            at(0),
        )
        .await
        .unwrap();
    for (id, secs) in [("late", 20), ("early", 10)] {
        store
            .insert_delivery(NewDelivery {
                integration_id,
                delivery_id: id.into(),
                event_type: "ping".into(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: b"{}".to_vec(),
                received_at: at(secs),
            })
            .await
            .unwrap();
    }
    let lease = Duration::seconds(60);
    let first = store.claim_due(at(30), lease, 1).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].delivery_id, "early");
    assert_eq!(first[0].attempts, 1);
    assert_eq!(first[0].provider, Provider::Github);
    let second = store.claim_due(at(31), lease, 10).await.unwrap();
    assert_eq!(second.len(), 1, "the leased row is hidden");
    assert_eq!(second[0].delivery_id, "late");
    assert!(store.claim_due(at(32), lease, 10).await.unwrap().is_empty());
    let reclaimed = store.claim_due(at(91), lease, 10).await.unwrap();
    assert_eq!(
        reclaimed.len(),
        2,
        "leases expired without completion return"
    );
    assert!(reclaimed.iter().all(|d| d.attempts == 2));
    store
        .complete_fanout(
            integration_id,
            "early",
            2,
            FanoutOutcome::Fanned {
                messages: vec![],
                evaluated: vec![],
            },
            at(92),
        )
        .await
        .unwrap();
    assert!(store
        .delivery(integration_id, "early")
        .unwrap()
        .delivery
        .body
        .is_empty());
    let still_due = store.claim_due(at(200), lease, 10).await.unwrap();
    assert_eq!(still_due.len(), 1);
    assert_eq!(still_due[0].delivery_id, "late");
}

#[tokio::test]
async fn deliveries_dedupe_per_integration_and_page_newest_first() {
    let store = MemoryStore::default();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let new = |integration_id, delivery_id: &str, secs| NewDelivery {
        integration_id,
        delivery_id: delivery_id.into(),
        event_type: "ping".into(),
        provider_timestamp: None,
        headers: serde_json::json!({}),
        body: b"{}".to_vec(),
        received_at: at(secs),
    };
    assert_eq!(
        store.insert_delivery(new(a, "d1", 0)).await.unwrap(),
        InsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_delivery(new(a, "d1", 5)).await.unwrap(),
        InsertOutcome::Duplicate
    );
    assert_eq!(
        store.insert_delivery(new(b, "d1", 5)).await.unwrap(),
        InsertOutcome::Inserted
    );
    store.insert_delivery(new(a, "d2", 10)).await.unwrap();
    store.insert_delivery(new(a, "d3", 20)).await.unwrap();

    let page = DeliveryPage {
        limit: 2,
        ..Default::default()
    };
    let first = store.list_deliveries(a, page.clone()).await.unwrap();
    let ids: Vec<_> = first.iter().map(|d| d.delivery_id.as_str()).collect();
    assert_eq!(ids, ["d3", "d2"]);
    assert!(first.iter().all(|d| d.state == DeliveryState::Queued));
    let older = store
        .list_deliveries(
            a,
            DeliveryPage {
                before: first.last().map(|d| DeliveryCursor {
                    received_at: d.received_at,
                    delivery_id: d.delivery_id.clone(),
                }),
                ..page.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].delivery_id, "d1");
    let recent = store
        .list_deliveries(
            a,
            DeliveryPage {
                since: Some(at(10)),
                ..page.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].delivery_id, "d3");
    assert!(store
        .list_deliveries(
            a,
            DeliveryPage {
                state: Some(DeliveryState::Failed),
                ..page
            }
        )
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn disable_and_enable_toggle_without_losing_configuration() {
    let store = MemoryStore::default();
    let id = Uuid::new_v4();
    store.apply(put(id, 1), at(0)).await.unwrap();
    let disable = Operation {
        operation_id: Uuid::new_v4(),
        integration_id: id,
        revision: 2,
        mutation: Mutation::Disable,
    };
    store.apply(disable, at(1)).await.unwrap();
    let disabled = store.integration(id).await.unwrap().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.disabled_at, Some(at(1)));
    assert_eq!(disabled.community_host, "acme.communities.example.org");
    let enable = Operation {
        operation_id: Uuid::new_v4(),
        integration_id: id,
        revision: 3,
        mutation: Mutation::Enable,
    };
    store.apply(enable, at(2)).await.unwrap();
    let enabled = store.integration(id).await.unwrap().unwrap();
    assert!(enabled.enabled && enabled.disabled_at.is_none());
    assert_eq!(enabled.revision, 3);
}

#[tokio::test]
async fn fanout_queues_messages_once_and_delete_cancels_unsent_ones() {
    let store = MemoryStore::default();
    let integration_id = Uuid::new_v4();
    store.apply(put(integration_id, 1), at(0)).await.unwrap();
    let (sub_a, sub_b, channel) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    for sub in [sub_a, sub_b] {
        let write = SubscriptionWrite {
            id: sub,
            integration_id,
            revision: 1,
            change: SubscriptionChange::Put {
                channel_id: channel,
                filter: None,
                message_template: None,
            },
        };
        assert!(matches!(
            store.write_subscription(write, at(1)).await.unwrap(),
            WriteOutcome::Applied(_)
        ));
    }
    assert_eq!(
        store
            .write_subscription(
                SubscriptionWrite {
                    id: Uuid::new_v4(),
                    integration_id: Uuid::new_v4(),
                    revision: 1,
                    change: SubscriptionChange::Put {
                        channel_id: channel,
                        filter: None,
                        message_template: None,
                    },
                },
                at(1),
            )
            .await
            .unwrap(),
        WriteOutcome::NotFound,
        "a subscription needs an existing integration"
    );
    store
        .insert_delivery(NewDelivery {
            integration_id,
            delivery_id: "d1".into(),
            event_type: "push".into(),
            provider_timestamp: None,
            headers: serde_json::json!({}),
            body: b"{}".to_vec(),
            received_at: at(10),
        })
        .await
        .unwrap();
    let messages = vec![
        NewMessage {
            subscription_id: sub_a,
            channel_id: channel,
            content: Ok("hello".into()),
        },
        NewMessage {
            subscription_id: sub_b,
            channel_id: channel,
            content: Err("filter failed".into()),
        },
    ];
    let claim = store
        .claim_due(at(10), Duration::seconds(60), 1)
        .await
        .unwrap()
        .remove(0);
    let live = Evaluated::of(&store.live_subscriptions(integration_id).await.unwrap());
    let fanned = |evaluated: Vec<Evaluated>| FanoutOutcome::Fanned {
        messages: messages.clone(),
        evaluated,
    };
    // (evaluated set, claimed attempt, commit, why)
    let commits = [
        (
            vec![live[0]],
            claim.attempts,
            FanoutCommit::Stale,
            "a subscription was added",
        ),
        (
            live.clone(),
            0,
            FanoutCommit::Lost,
            "an older claim cannot settle",
        ),
        (
            live.clone(),
            claim.attempts,
            FanoutCommit::Settled,
            "current",
        ),
        (
            live.clone(),
            claim.attempts,
            FanoutCommit::Lost,
            "settled once",
        ),
    ];
    for (evaluated, attempts, commit, why) in commits {
        let outcome = fanned(evaluated);
        assert_eq!(
            store
                .complete_fanout(integration_id, "d1", attempts, outcome, at(11))
                .await
                .unwrap(),
            commit,
            "{why}"
        );
        if commit == FanoutCommit::Stale {
            let row = store.delivery(integration_id, "d1").unwrap();
            assert_eq!(row.state, DeliveryState::Queued, "{why}");
            assert_eq!(row.next_attempt_at, Some(at(11)), "{why}: due again now");
            assert!(!row.delivery.body.is_empty(), "{why}");
            assert!(store
                .message(&MessageKey {
                    integration_id,
                    delivery_id: "d1".into(),
                    subscription_id: sub_a,
                })
                .is_none());
        }
    }
    assert_eq!(
        store
            .complete_fanout(
                integration_id,
                "d1",
                claim.attempts,
                FanoutOutcome::Failed("late".into()),
                at(12),
            )
            .await
            .unwrap(),
        FanoutCommit::Lost
    );
    let delivery = store.delivery(integration_id, "d1").unwrap();
    assert_eq!(delivery.state, DeliveryState::Accepted, "settled once");
    assert!(delivery.delivery.body.is_empty());
    let key = |subscription_id| MessageKey {
        integration_id,
        delivery_id: "d1".into(),
        subscription_id,
    };
    let failed = store.message(&key(sub_b)).unwrap();
    assert_eq!(failed.state, DeliveryState::Failed);
    assert_eq!(failed.last_error.as_deref(), Some("filter failed"));
    let timeline = store
        .list_deliveries(
            integration_id,
            DeliveryPage {
                limit: 10,
                ..DeliveryPage::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        timeline[0].messages,
        MessageCounts {
            queued: 1,
            accepted: 0,
            failed: 1
        }
    );

    let lease = Duration::seconds(60);
    let claimed = store.claim_due_messages(at(20), lease, 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].key, key(sub_a));
    assert_eq!(claimed[0].content, "hello");
    assert_eq!(claimed[0].received_at, at(10));
    assert_eq!(claimed[0].signed_at, at(20), "stamped by its first claim");
    assert_eq!(claimed[0].community_host, "acme.communities.example.org");
    assert!(store
        .claim_due_messages(at(21), lease, 10)
        .await
        .unwrap()
        .is_empty());

    let delete = SubscriptionWrite {
        id: sub_a,
        integration_id,
        revision: 2,
        change: SubscriptionChange::Delete,
    };
    store.write_subscription(delete, at(22)).await.unwrap();
    let cancelled = store.message(&key(sub_a)).unwrap();
    assert_eq!(cancelled.state, DeliveryState::Failed);
    assert_eq!(
        cancelled.last_error.as_deref(),
        Some("subscription deleted")
    );
    store
        .complete_message(
            &claimed[0],
            ForwardOutcome::Accepted {
                event_id: "ab".repeat(32),
            },
            at(23),
        )
        .await
        .unwrap();
    assert_eq!(
        store.message(&key(sub_a)).unwrap(),
        cancelled,
        "an in-flight send cannot revive a cancelled message"
    );
    assert!(store
        .claim_due_messages(at(200), lease, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .live_subscriptions(integration_id)
            .await
            .unwrap()
            .len(),
        1
    );
}
