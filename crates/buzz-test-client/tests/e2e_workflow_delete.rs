//! End-to-end regression test for #4864: `buzz workflows delete` (a kind:5
//! NIP-09 a-tag deletion) must tombstone the kind:30620 event rows so the
//! CLI/desktop list/get paths stop returning the workflow, and re-publishing
//! the coordinate (`workflows update`) must be rejected instead of silently
//! resurrecting the workflow — and re-arming its webhook trigger with a brand
//! new secret.
//!
//! # Running
//!
//! Start the relay against a test DB (dev auth, e.g.
//! `BUZZ_REQUIRE_AUTH_TOKEN=false`), then run:
//!
//! ```text
//! cargo test --test e2e_workflow_delete -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable (default
//! `ws://localhost:3000`).

use nostr::{Event, EventBuilder, Keys, Kind, Tag};

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

/// Post an event to the HTTP bridge using the dev X-Pubkey auth path, exactly
/// like the CLI's `submit_event`.
async fn post_event(keys: &Keys, event: &Event) -> serde_json::Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("POST /events");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("parse event response");
    serde_json::json!({
        "status": status.as_u16(),
        "accepted": body["accepted"].as_bool().unwrap_or(false),
        "error": body["error"].as_str().unwrap_or("").to_string(),
        "message": body["message"].as_str().unwrap_or("").to_string(),
        "raw": body,
    })
}

/// Query kind:30620 events in a channel via the HTTP bridge (`POST /query`),
/// the exact path the CLI's `workflows list` and desktop use.
async fn query_channel_workflows(keys: &Keys, channel_id: &str) -> Vec<serde_json::Value> {
    let client = reqwest::Client::new();
    let filter = serde_json::json!([{ "kinds": [30620], "#h": [channel_id] }]);
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(filter.to_string())
        .send()
        .await
        .expect("POST /query");
    assert!(resp.status().is_success(), "query failed: {}", resp.status());
    resp.json().await.expect("parse query response")
}

/// Create a real channel via a signed kind:9007 event (the creator becomes an
/// owner member), mirroring `e2e_relay.rs::create_test_channel`.
async fn create_test_channel(keys: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid]).unwrap(),
            Tag::parse(["name", &format!("wf-e2e-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let resp = post_event(keys, &event).await;
    assert!(
        resp["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {}",
        resp["raw"]
    );
    channel_uuid
}

/// Build a valid kind:30620 workflow definition event (webhook trigger so the
/// upsert would try to inject an ever-changing `webhook_secret` — the
/// resurrection regression re-arms the webhook, which this test detects by
/// asserting the update is rejected outright).
fn build_workflow_def_event(keys: &Keys, channel_id: &str, workflow_id: &str) -> Event {
    let yaml = "name: t\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: send_message\n    text: hi";
    let ch: uuid::Uuid = channel_id.parse().expect("channel uuid");
    let wf: uuid::Uuid = workflow_id.parse().expect("workflow uuid");
    buzz_sdk::builders::build_workflow_def(ch, wf, yaml)
        .expect("build workflow def")
        .sign_with_keys(keys)
        .expect("sign workflow def")
}

#[tokio::test]
#[ignore]
async fn test_workflow_delete_tombstones_event_and_blocks_resurrection() {
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // 1. Channel (creator = owner member) + workflow definition.
    let channel_id = create_test_channel(&keys).await;
    let wf_id = uuid::Uuid::new_v4().to_string();

    let create = build_workflow_def_event(&keys, &channel_id, &wf_id);
    let create_resp = post_event(&keys, &create).await;
    assert!(
        create_resp["accepted"].as_bool().unwrap_or(false),
        "workflow create should be accepted: {}",
        create_resp["raw"]
    );

    // 2. List — the workflow is queryable before deletion.
    let before = query_channel_workflows(&keys, &channel_id).await;
    assert_eq!(before.len(), 1, "exactly one workflow before deletion");

    // 3. NIP-09 a-tag deletion (what `buzz workflows delete` sends).
    let a_coord = format!("30620:{pubkey_hex}:{wf_id}");
    let del = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let del_resp = post_event(&keys, &del).await;
    assert!(
        del_resp["accepted"].as_bool().unwrap_or(false),
        "deletion should be accepted: {}",
        del_resp["raw"]
    );

    // 4. List — the workflow must be GONE (the event row is tombstoned).
    // The side effect runs synchronously with ingest, so a single query is
    // authoritative — no polling needed.
    let after = query_channel_workflows(&keys, &channel_id).await;
    assert_eq!(
        after.len(),
        0,
        "deleted workflow must not appear in list/get (#4864)"
    );

    // 5. Re-publishing the coordinate (what `buzz workflows update` sends)
    //    must be REJECTED, not silently resurrect the workflow.
    let update = build_workflow_def_event(&keys, &channel_id, &wf_id);
    let update_resp = post_event(&keys, &update).await;
    assert!(
        !update_resp["accepted"].as_bool().unwrap_or(false),
        "update after delete must be rejected: {}",
        update_resp["raw"]
    );
    let msg = update_resp["error"].as_str().unwrap_or("");
    assert!(
        msg.contains("was deleted"),
        "rejection should mention the deletion, got: {msg}"
    );

    // 6. A fresh workflow in the same channel still works — the tombstone
    //    guard is coordinate-scoped, never channel-scoped.
    let wf_id2 = uuid::Uuid::new_v4().to_string();
    let create2 = build_workflow_def_event(&keys, &channel_id, &wf_id2);
    let create2_resp = post_event(&keys, &create2).await;
    assert!(
        create2_resp["accepted"].as_bool().unwrap_or(false),
        "fresh create after delete should still be accepted: {}",
        create2_resp["raw"]
    );
    let after2 = query_channel_workflows(&keys, &channel_id).await;
    assert_eq!(after2.len(), 1, "the fresh workflow is the only one left");
}
