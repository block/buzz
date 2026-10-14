//! Tombstone-aware ownership recovery through the authenticated Nostr bridge.
//! Run against a local development relay with `--ignored`.

use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};

fn url() -> String {
    std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "ws://localhost:3000".into())
        .replace("ws://", "http://")
        .replace("wss://", "https://")
}

async fn post(keys: &Keys, event: &Event) {
    let response = reqwest::Client::new()
        .post(format!("{}/events", url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(event)
        .send()
        .await
        .expect("event request");
    assert!(
        response.status().is_success(),
        "event status {}",
        response.status()
    );
    let body: Value = response.json().await.expect("event response");
    assert_eq!(body["accepted"], true, "event rejected: {body}");
}

async fn query(keys: &Keys, filter: Value) -> Vec<Event> {
    let response = reqwest::Client::new()
        .post(format!("{}/query", url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(&vec![filter])
        .send()
        .await
        .expect("query request");
    assert!(
        response.status().is_success(),
        "query status {}",
        response.status()
    );
    response.json().await.expect("signed event array")
}

async fn channel(keys: &Keys, private: bool) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", &id]).unwrap(),
            Tag::parse(["name", &format!("thread-roots-{id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", if private { "private" } else { "open" }]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    post(keys, &event).await;
    id
}

fn request(channel: &str, target: &Event) -> Value {
    json!({"kinds": [39005], "ids": [target.id.to_hex()], "#h": [channel], "resolve_thread_roots": true})
}

#[tokio::test]
#[ignore]
async fn deleted_reply_roots_are_recovered_without_content_or_scope_leaks() {
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let other_channel = channel(&owner, false).await;
    let bounded_filter = json!({
        "kinds": [39005], "#h": [&other_channel], "resolve_thread_roots": true,
        "ids": (0..60).map(|id| format!("{id:064x}")).collect::<Vec<_>>()
    });
    let oversized = reqwest::Client::new()
        .post(format!("{}/query", url()))
        .header("X-Pubkey", owner.public_key().to_hex())
        .json(&vec![bounded_filter.clone(), bounded_filter])
        .send()
        .await
        .expect("bounded batch");
    assert_eq!(
        oversized.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "the target cap applies across filters, not just within each filter"
    );
    for deletion_kind in [5, 9005] {
        let channel_id = channel(&owner, true).await;
        let root = EventBuilder::new(Kind::Custom(9), "root")
            .tags([Tag::parse(["h", &channel_id]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();
        post(&owner, &root).await;
        let reply = EventBuilder::new(Kind::Custom(9), "deleted secret payload")
            .tags([
                Tag::parse(["h", &channel_id]).unwrap(),
                Tag::parse(["e", &root.id.to_hex(), "", "reply"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        post(&owner, &reply).await;
        let deletion = EventBuilder::new(Kind::Custom(deletion_kind), "")
            .tags([
                Tag::parse(["h", &channel_id]).unwrap(),
                Tag::parse(["e", &reply.id.to_hex()]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        post(&owner, &deletion).await;

        let ordinary = query(
            &owner,
            json!({"kinds":[9],"ids":[reply.id.to_hex()],"#h":[channel_id]}),
        )
        .await;
        assert!(
            ordinary.is_empty(),
            "ordinary history must exclude the deleted payload"
        );
        let summaries = query(&owner, request(&channel_id, &reply)).await;
        assert_eq!(summaries.len(), 1, "metadata must survive deletion");
        let summary = &summaries[0];
        summary.verify().expect("relay signature");
        assert_eq!(summary.kind, Kind::Custom(39005));
        assert!(summary
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["e", &root.id.to_hex()]));
        let counts: Value = serde_json::from_str(&summary.content).unwrap();
        assert_eq!(counts["reply_count"], 0);
        assert_eq!(counts["descendant_count"], 0);
        assert!(!summary.content.contains("deleted secret payload"));

        assert!(
            query(&owner, request(&other_channel, &reply))
                .await
                .is_empty(),
            "wrong channel must not leak ownership"
        );
        assert!(
            query(&outsider, request(&channel_id, &reply))
                .await
                .is_empty(),
            "private channel membership is required"
        );
    }
}
