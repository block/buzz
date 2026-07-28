//! End-to-end regression test for NIP-09 a-tag deletion of workflow
//! definitions (kind:30620).
//!
//! Requires a running relay. Marked `#[ignore]` like the other e2e suites.
//!
//! ```text
//! cargo test --test e2e_workflow_deletion -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};

const KIND_WORKFLOW_DEF: u16 = 30620;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Create a real channel via REST so the relay accepts kind:30620
async fn create_test_channel(keys: &Keys) -> String {
    let client = reqwest::Client::new();
    let channel_uuid = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("wf-del-e2e-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-channel event");
    assert!(
        resp.status().is_success(),
        "channel creation failed: {}",
        resp.status()
    );
    channel_uuid.to_string()
}

/// Deleting a workflow must also retire its kind:30620 definition event.
///
/// Regression guard for #2879: the relay used to drop the DB row and invalidate
/// the engine cache while leaving the definition event live, so `workflows list`
/// and `workflows get`kept returning a workflow ` workflows trigger` rejected.
#[tokio::test]
#[ignore]
async fn test_workflow_delete_retires_definition_event() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish a workflow
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let yaml = concat!(
        "name: E2E Delete Probe\n",
        "trigger:\n on: webhook\n",
        "steps:\n",
        "  - id: noop\n    action: send_message\n    text: probe\n    channel: general\n",
    );
    let def = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF), yaml)
        .tags(vec![
            Tag::parse(["d", &workflow_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let def_id = def.id;
    let ok = client.send_event(def).await.expect("send workflow def");
    assert!(
        ok.accepted,
        "workflow def should be accepted:{}",
        ok.message
    );

    // Sanity check it is queryable before deletion
    let sid_pre = sub_id("wf-del-pre");
    let filter_pre = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEF))
        .author(keys.public_key())
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            workflow_id.as_str(),
        );
    client
        .subscribe(&sid_pre, vec![filter_pre])
        .await
        .expect("subscribe pre");
    let pre = client
        .collect_until_eose(&sid_pre, Duration::from_secs(5))
        .await
        .expect("collect pre");
    assert!(
        pre.iter().any(|e| e.id == def_id),
        "workflow definition should be queryable before deletion"
    );

    // Delete via the same a-tag coordinate `buzz workflows delete` emits.
    let a_coord = format!(
        "{}:{}:{}",
        KIND_WORKFLOW_DEF,
        keys.public_key().to_hex(),
        workflow_id
    );
    let del = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let ok_del = client.send_event(del).await.expect("send deletion");
    assert!(
        ok_del.accepted,
        "a-tag deletion should be accepted: {}",
        ok_del.message
    );

    // The definition must no longer surface - this is what `workflows list`
    // and `workflows get` read.
    let sid_post = sub_id("wf-del-post");
    let filter_post = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEF))
        .author(keys.public_key())
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            workflow_id.as_str(),
        );
    client
        .subscribe(&sid_post, vec![filter_post])
        .await
        .expect("subscribe post");
    let post = client
        .collect_until_eose(&sid_post, Duration::from_secs(5))
        .await
        .expect("collect post");
    assert!(
        post.is_empty(),
        "workflow deletion should retire the definition event (got {} events)",
        post.len()
    );

    client.disconnect().await.expect("disconnect");
}
