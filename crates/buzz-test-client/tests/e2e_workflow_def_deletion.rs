//! End-to-end tests for NIP-09 a-tag deletion of workflow definitions
//! (kind:30620).
//!
//! Regression coverage for issue #717 — before the fix,
//! `handle_a_tag_deletion`'s workflow branch deleted the `workflows`
//! projection row but never tombstoned the underlying kind:30620 event row,
//! so REQ subscribers kept receiving the deleted definition.
//!
//! These tests require a running relay instance. By default they are marked
//! `#[ignore]` so that `cargo test` does not fail in CI when the relay is not
//! available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_workflow_def_deletion -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable.

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

/// Minimal valid workflow YAML. `handle_workflow_def` parses this before
/// persisting, and injects a webhook secret for webhook triggers.
fn workflow_yaml(name: &str) -> String {
    format!(
        "name: {name}\n\
         description: e2e deletion probe\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: step1\n\
         \x20   name: Notify\n\
         \x20   action: send_message\n\
         \x20   text: \"e2e\"\n"
    )
}

/// Create an `open` channel (kind:9007) via POST /events. The h-tag UUID
/// takes the `create_channel_with_id` path, which bootstraps the creator as
/// owner-member — the membership `handle_workflow_def` requires. Returns the
/// channel UUID string.
async fn create_test_channel(keys: &Keys) -> String {
    let client = reqwest::Client::new();
    let channel_uuid = uuid::Uuid::new_v4();

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("e2e-wf-del-{channel_uuid}")]).unwrap(),
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
        "channel creation event failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse event response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {body}"
    );

    channel_uuid.to_string()
}

/// Publish a workflow definition (kind:30620, `d` = workflow UUID, `h` =
/// channel) and assert acceptance. Returns the workflow UUID string — which
/// is also the event row's NIP-33 d-tag (create-path invariant).
async fn define_workflow(
    client: &mut BuzzTestClient,
    keys: &Keys,
    channel_id: &str,
) -> (String, nostr::EventId) {
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let name = format!("e2e-doomed-wf-{}", uuid::Uuid::new_v4().simple());
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF), workflow_yaml(&name))
        .tags(vec![
            Tag::parse(["d", &workflow_id]).unwrap(),
            Tag::parse(["h", channel_id]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let event_id = event.id;

    let ok = client.send_event(event).await.expect("send workflow def");
    assert!(
        ok.accepted,
        "workflow def should be accepted: {}",
        ok.message
    );

    (workflow_id, event_id)
}

/// REQ the live kind:30620 row for `(author, d-tag)`.
async fn query_def_events(
    client: &mut BuzzTestClient,
    keys: &Keys,
    workflow_id: &str,
    label: &str,
) -> Vec<nostr::Event> {
    let sid = sub_id(label);
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEF))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), workflow_id);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect")
}

/// A kind:5 deletion carrying `["a", "30620:<pk>:<workflow_id>"]` removes the
/// workflow AND tombstones the underlying kind:30620 event row, so REQs stop
/// returning the deleted definition.
///
/// Regression test for issue #717.
#[tokio::test]
#[ignore]
async fn test_workflow_def_a_tag_deletion_by_uuid_tombstones_event() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let (workflow_id, def_event_id) = define_workflow(&mut client, &keys, &channel_id).await;

    // Sanity: the def event is queryable before deletion — this is the exact
    // surface `buzz workflows list` reads.
    let pre = query_def_events(&mut client, &keys, &workflow_id, "wf-del-pre").await;
    assert!(
        pre.iter().any(|e| e.id == def_event_id),
        "workflow def event should be queryable before deletion"
    );

    // NIP-09 a-tag deletion of the workflow coordinate.
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

    // The kind:30620 event row must be gone from REQ results.
    let post = query_def_events(&mut client, &keys, &workflow_id, "wf-del-post").await;
    assert!(
        post.is_empty(),
        "a-tag deletion must tombstone the workflow def event (got {} events)",
        post.len()
    );

    client.disconnect().await.expect("disconnect");
}

/// The name-fallback deletion path (`d` = workflow name, not UUID) resolves
/// the workflow row and tombstones the event row by the resolved id.
#[tokio::test]
#[ignore]
async fn test_workflow_def_a_tag_deletion_by_name_tombstones_event() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish with a `name` tag so the row name is human-stable.
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let name = format!("e2e-named-wf-{}", uuid::Uuid::new_v4().simple());
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF), workflow_yaml(&name))
        .tags(vec![
            Tag::parse(["d", &workflow_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", &name]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let def_event_id = event.id;
    let ok = client.send_event(event).await.expect("send workflow def");
    assert!(
        ok.accepted,
        "workflow def should be accepted: {}",
        ok.message
    );

    let pre = query_def_events(&mut client, &keys, &workflow_id, "wf-name-pre").await;
    assert!(
        pre.iter().any(|e| e.id == def_event_id),
        "workflow def event should be queryable before deletion"
    );

    // a-tag d-value is the workflow *name* — not a UUID, not the event d-tag.
    let a_coord = format!(
        "{}:{}:{}",
        KIND_WORKFLOW_DEF,
        keys.public_key().to_hex(),
        name
    );
    let del = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let ok_del = client.send_event(del).await.expect("send name deletion");
    assert!(
        ok_del.accepted,
        "name-based a-tag deletion should be accepted: {}",
        ok_del.message
    );

    let post = query_def_events(&mut client, &keys, &workflow_id, "wf-name-post").await;
    assert!(
        post.is_empty(),
        "name-based deletion must tombstone the workflow def event (got {} events)",
        post.len()
    );

    client.disconnect().await.expect("disconnect");
}
