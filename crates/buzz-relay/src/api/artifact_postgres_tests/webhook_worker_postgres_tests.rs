//! NIP-WH subscription worker, from a gateway delivery through the production
//! router to records read back from the channel.
use super::webhook_delivery_postgres_tests::{delivery, with_gateway};
use super::*;

const INTEGRATION: &str = "0f4c1a52-7d3e-4b8a-9c61-2e5f8d7a1b30";
const OTHER_INTEGRATION: &str = "5b0a8e7c-2d41-4f3a-8e6b-9c1d2f3a4b5c";

impl Fixture {
    async fn outcomes(
        &self,
        delivery: &Event,
    ) -> Vec<(Uuid, String, Option<Vec<u8>>, Option<String>)> {
        sqlx::query_as(
            "SELECT subscription_id, outcome, record_event_id, last_error FROM webhook_records \
             WHERE community_id = $1 AND delivery_event_id = $2 ORDER BY subscription_id",
        )
        .bind(self.community.as_uuid())
        .bind(delivery.id.as_bytes().as_slice())
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn delivery_state(&self, delivery: &Event) -> Option<String> {
        sqlx::query_scalar(
            "SELECT state FROM webhook_deliveries WHERE community_id = $1 AND delivery_event_id = $2",
        )
        .bind(self.community.as_uuid())
        .bind(delivery.id.as_bytes().as_slice())
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }

    async fn records(&self, channel: Uuid) -> Vec<Value> {
        let (status, body) = self
            .request(
                "/query",
                json!([{"kinds": [40701], "#h": [channel.to_string()]}]),
            )
            .await;
        assert!(status.is_success(), "{status}: {body}");
        body.as_array().unwrap().clone()
    }
}

fn subscription(filter: &str, transform: Option<&str>) -> Value {
    match transform {
        Some(transform) => json!({"version": 1, "filter": filter, "transform": transform}),
        None => json!({"version": 1, "filter": filter}),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deliveries_become_records_once_per_subscription() {
    let (f, gateway) = with_gateway().await;
    let ids: Vec<Uuid> = {
        let mut ids: Vec<Uuid> = (0..6).map(|_| Uuid::new_v4()).collect();
        ids.sort();
        ids
    };
    let [matched, no_match, demoted, broken, archived, elsewhere] = ids[..] else {
        unreachable!()
    };
    let archived_channel = Uuid::new_v4();
    sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by) VALUES($1,$2,$2::text,'open','stream',$3)")
        .bind(f.community.as_uuid()).bind(archived_channel).bind(f.owner.public_key().as_bytes().as_slice()).execute(&f.pool).await.unwrap();
    f.set_role(archived_channel, &f.owner, "owner").await;
    f.set_role(f.home, &f.peer, "admin").await;

    let opened = r#"event.type == "pull_request.opened""#;
    for (key, channel, id, content) in [
        (
            &f.owner,
            f.home,
            matched,
            subscription(opened, Some(r#"{"action": payload.action}"#)),
        ),
        (
            &f.owner,
            f.private,
            no_match,
            subscription(r#"payload.action == "closed""#, None),
        ),
        (&f.peer, f.home, demoted, subscription("true", None)),
        (
            &f.owner,
            f.home,
            broken,
            subscription("payload.missing == 1", None),
        ),
        (
            &f.owner,
            archived_channel,
            archived,
            subscription("true", None),
        ),
    ] {
        let revision = f.subscription_with(key, channel, id, "create", None, content);
        f.accepted(key, &revision).await;
    }
    // Another integration's subscription never sees this integration's deliveries.
    let other = f.subscription_with(
        &f.owner,
        f.home,
        elsewhere,
        "create",
        None,
        subscription("true", None),
    );
    let other = EventBuilder::new(Kind::Custom(45010), other.content.clone())
        .tags(other.tags.iter().map(|tag| match tag.as_slice() {
            [name, _] if name == "integration" => {
                Tag::parse(["integration", OTHER_INTEGRATION]).unwrap()
            }
            _ => tag.clone(),
        }))
        .sign_with_keys(&f.owner)
        .unwrap();
    f.accepted(&f.owner, &other).await;
    // The peer loses the role after saving its subscription, and the channel
    // is archived after its subscription was saved.
    f.set_role(f.home, &f.peer, "member").await;
    sqlx::query("UPDATE channels SET archived_at = now() WHERE community_id = $1 AND id = $2")
        .bind(f.community.as_uuid())
        .bind(archived_channel)
        .execute(&f.pool)
        .await
        .unwrap();

    let queued = delivery(&gateway, "d-1", nostr::Timestamp::now().as_secs());
    let (status, body) = f.request_as(&gateway, "/events", json!(queued)).await;
    assert!(status.is_success(), "{status}: {body}");

    assert_eq!(crate::webhook_worker::sweep(&f.state).await, 1);
    assert_eq!(f.delivery_state(&queued).await.as_deref(), Some("done"));
    let outcomes = f.outcomes(&queued).await;
    let summary: Vec<(Uuid, &str)> = outcomes
        .iter()
        .map(|(id, o, ..)| (*id, o.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (matched, "matched"),
            (demoted, "skipped_signer"),
            (broken, "error"),
            (archived, "skipped_channel"),
        ]
    );
    assert!(
        outcomes[2]
            .3
            .as_deref()
            .is_some_and(|e| e.starts_with("filter: ")),
        "{outcomes:?}"
    );

    // The match is a relay-signed channel record.
    let records = f.records(f.home).await;
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(
        record["pubkey"],
        f.state.relay_keypair.public_key().to_hex()
    );
    assert_eq!(
        outcomes[0].2.as_deref(),
        Some(
            hex::decode(record["id"].as_str().unwrap())
                .unwrap()
                .as_slice()
        )
    );
    assert_eq!(
        record["tags"],
        json!([
            ["h", f.home.to_string()],
            ["integration", INTEGRATION],
            ["t", "provider:github"],
            ["t", format!("subscription:{matched}")],
        ])
    );
    let head = f.head(matched).await.unwrap().0;
    assert_eq!(
        serde_json::from_str::<Value>(record["content"].as_str().unwrap()).unwrap(),
        json!({
            "version": 1, "type": "pull_request.opened",
            "delivery_event_id": queued.id.to_hex(),
            "subscription_id": matched.to_string(),
            "subscription_revision": hex::encode(head),
            "payload": {"action": "opened"},
        })
    );

    // A retry after the lease ran out finishes what is missing and publishes
    // nothing twice.
    assert_eq!(crate::webhook_worker::sweep(&f.state).await, 0);
    sqlx::query(
        "UPDATE webhook_deliveries SET state = 'evaluating', next_attempt_at = now() \
         WHERE community_id = $1 AND delivery_event_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(queued.id.as_bytes().as_slice())
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM webhook_records WHERE community_id = $1 AND subscription_id = $2")
        .bind(f.community.as_uuid())
        .bind(broken)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(crate::webhook_worker::sweep(&f.state).await, 1);
    assert_eq!(f.outcomes(&queued).await, outcomes);
    assert_eq!(f.records(f.home).await.len(), 1);

    // Finished deliveries are deleted after retention; records stay.
    sqlx::query("UPDATE webhook_deliveries SET received_at = now() - interval '8 days' WHERE community_id = $1")
        .bind(f.community.as_uuid()).execute(&f.pool).await.unwrap();
    crate::webhook_worker::reap(&f.state).await;
    assert_eq!(f.delivery_state(&queued).await, None);
    assert!(f.outcomes(&queued).await.is_empty());
    assert_eq!(f.records(f.home).await.len(), 1);
}
