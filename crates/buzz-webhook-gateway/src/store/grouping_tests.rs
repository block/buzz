//! Shared contract for the memory and Postgres stores, exercised through fan-out
//! and the same event builder the forwarder uses.
use super::*;
use crate::fanout::Fanout;
use crate::forwarder::build_message_event;
use chrono::{TimeDelta, Timelike};
use nostr::{Keys, NostrSigner};
use std::sync::Arc;

#[tokio::test]
async fn grouped_delivery_memory() {
    exercise_grouped_delivery(Arc::new(MemoryStore::default())).await;
}

pub(crate) async fn exercise_grouped_delivery(store: Arc<dyn Store>) {
    let actions = [
        (Uuid::new_v4(), GroupKeyAction::ReplyInThread),
        (Uuid::new_v4(), GroupKeyAction::UpdateMessage),
    ];
    let channels = [Uuid::new_v4(), Uuid::new_v4()];
    let now = Utc::now().with_nanosecond(0).unwrap();
    for (id, action) in actions {
        let config = IntegrationConfig {
            group_key_expr: Some("payload.incident".into()),
            group_key_action: action,
            ..Default::default()
        };
        let op = Operation {
            operation_id: Uuid::new_v4(),
            integration_id: id,
            revision: 1,
            mutation: Mutation::PutIntegration {
                community_host: "relay.example.org".into(),
                provider: Provider::Pagerduty,
                config,
            },
        };
        store.apply(op, now).await.unwrap();
        for channel_id in channels {
            store
                .write_subscription(
                    SubscriptionWrite {
                        id: Uuid::new_v4(),
                        integration_id: id,
                        revision: 1,
                        change: SubscriptionChange::Put {
                            channel_id,
                            filter: None,
                        },
                    },
                    now,
                )
                .await
                .unwrap();
        }
        for (index, stage) in ["triggered", "resolved"].into_iter().enumerate() {
            store
                .insert_delivery(NewDelivery {
                    integration_id: id,
                    delivery_id: stage.into(),
                    event_type: stage.into(),
                    provider_timestamp: None,
                    headers: serde_json::json!({}),
                    body: br#"{"incident":"shared-key"}"#.to_vec(),
                    received_at: now - TimeDelta::seconds(2 - index as i64),
                })
                .await
                .unwrap();
        }
    }

    let fanout = Fanout {
        store: store.clone(),
        concurrency: 32,
        retry_max_age: TimeDelta::hours(1),
        now: Utc::now,
    };
    while fanout.run_once().await.unwrap() > 0 {}
    let lease = TimeDelta::milliseconds(100);
    let claim = |at| store.claim_due_messages(at, lease, 32);
    let (a, b) = tokio::join!(claim(now), claim(now));
    let roots: Vec<_> = a.unwrap().into_iter().chain(b.unwrap()).collect();
    assert_eq!(roots.len(), 4, "one root per integration and channel");
    assert!(roots
        .iter()
        .all(|root| root.target.is_none() && root.key.delivery_id == "triggered"));
    assert!(
        claim(now).await.unwrap().is_empty(),
        "followers must wait for acceptance"
    );

    // The first worker loses its lease. It cannot settle the group with a stale id.
    let later = now + lease;
    let reclaimed = claim(later).await.unwrap();
    assert_eq!(reclaimed.len(), roots.len());
    let accepted = |id| ForwardOutcome::Accepted { event_id: id };
    for old in &roots {
        store
            .complete_message(old, accepted("stale".into()), later)
            .await
            .unwrap();
    }
    assert!(claim(later).await.unwrap().is_empty());
    let signer: Arc<dyn NostrSigner> = Arc::new(Keys::generate());
    let mut expected = std::collections::HashMap::new();
    for root in reclaimed {
        let event = build_message_event(&signer, &root).await.unwrap();
        let original = roots.iter().find(|r| r.key == root.key).unwrap();
        assert_eq!(
            event.id,
            build_message_event(&signer, original).await.unwrap().id
        );
        expected.insert(
            (root.key.integration_id, root.channel_id),
            event.id.to_hex(),
        );
        store
            .complete_message(&root, accepted(event.id.to_hex()), later)
            .await
            .unwrap();
    }
    let replies = store
        .claim_due_messages(later, TimeDelta::seconds(2), 32)
        .await
        .unwrap();
    assert_eq!(
        replies.len(),
        2,
        "only thread replies can share the root's timestamp"
    );
    for reply in &replies {
        assert_eq!(
            reply.target.as_ref().unwrap().0,
            GroupKeyAction::ReplyInThread
        );
    }
    // Keep the replies leased while advancing to the next whole second.
    let next = now + TimeDelta::seconds(1);
    let edits = claim(next).await.unwrap();
    assert_eq!(edits.len(), 2);
    let replies = replies.into_iter().chain(edits);
    for reply in replies {
        let id = &expected[&(reply.key.integration_id, reply.channel_id)];
        let action = actions
            .iter()
            .find(|a| a.0 == reply.key.integration_id)
            .unwrap()
            .1;
        assert_eq!(reply.key.delivery_id, "resolved");
        assert_eq!(reply.target, Some((action, id.clone())));
        let event = build_message_event(&signer, &reply).await.unwrap();
        let tag = event.tags.iter().find(|t| t.as_slice()[0] == "e").unwrap();
        match action {
            GroupKeyAction::ReplyInThread => {
                assert_eq!(event.kind.as_u16(), 9);
                assert_eq!(tag.as_slice(), ["e", id.as_str(), "", "reply"]);
            }
            GroupKeyAction::UpdateMessage => {
                assert_eq!(event.kind.as_u16(), 40003);
                assert_eq!(tag.as_slice(), ["e", id.as_str()]);
                assert!(event.created_at.as_secs() > now.timestamp() as u64);
            }
        }
        store
            .complete_message(&reply, accepted(event.id.to_hex()), next)
            .await
            .unwrap();
    }
}
