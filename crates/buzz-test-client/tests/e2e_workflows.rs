//! E2E tests for the Sprout workflow engine.
//!
//! These tests require a running relay instance with `require_auth_token=false`
//! (dev mode). By default they are marked `#[ignore]` so that `cargo test`
//! does not fail in CI when the relay is not available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p sprout-test-client --test e2e_workflows -- --ignored
//! ```
//!
//! # Auth
//!
//! In dev mode (`require_auth_token=false`) the relay accepts an
//! `X-Pubkey: <hex>` header as authentication. Tests generate fresh
//! [`nostr::Keys`] per test and pass the hex-encoded public key.

use std::time::Duration;

use nostr::Keys;
use reqwest::Client;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// WebSocket relay URL (e.g. `ws://localhost:3001`).
fn relay_ws_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

/// HTTP base URL derived from the WebSocket URL.
fn relay_http_url() -> String {
    relay_ws_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
}

/// Build a `reqwest::Client` with a short timeout.
fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to build HTTP client")
}

/// Known open channel IDs seeded in the dev database.
///
/// These are UUID5-derived from the channel name and are stable across relay
/// restarts as long as the seed data uses the same namespace + name inputs.
const CHANNEL_GENERAL: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

/// A seeded user pubkey that exists in the `users` table.
///
/// Workflow creation requires the owner pubkey to exist in `users` (FK constraint).
/// The relay does not auto-create users on first auth — users are created via
/// `sprout-admin mint-token` or WebSocket metadata events. This pubkey is present
/// in the dev database after the initial seed.
///
/// If tests fail with 500 "FK constraint fails", run:
/// ```
/// DATABASE_URL=postgres://sprout:sprout_dev@localhost:5432/sprout \
///   cargo run -p sprout-admin -- mint-token --name e2e-test --scopes messages:read \
///   --pubkey 0b5c83782cf123e698131ac976179f8366224e03db932c9da0074512aed2388d
/// ```
const SEEDED_PUBKEY: &str = "0b5c83782cf123e698131ac976179f8366224e03db932c9da0074512aed2388d";

/// A minimal webhook-triggered workflow YAML definition.
///
/// Uses `send_message` action (the simplest valid action type).
fn webhook_workflow_yaml(name: &str) -> String {
    format!(
        r#"name: {name}
description: Test workflow
trigger:
  on: webhook
steps:
  - id: step1
    name: Notify channel
    action: send_message
    text: "Workflow triggered by webhook"
"#
    )
}

// ── Shared HTTP helpers ───────────────────────────────────────────────────────

/// POST to create a workflow in a channel. Returns the parsed JSON response body.
async fn create_workflow(
    client: &Client,
    base: &str,
    pubkey_hex: &str,
    channel_id: &str,
    yaml: &str,
) -> serde_json::Value {
    let url = format!("{base}/api/channels/{channel_id}/workflows");
    let resp = client
        .post(&url)
        .header("X-Pubkey", pubkey_hex)
        .json(&serde_json::json!({ "yaml_definition": yaml }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url} failed: {e}"));

    assert_eq!(
        resp.status(),
        200,
        "expected 200 from POST /api/channels/:id/workflows"
    );
    resp.json()
        .await
        .expect("create workflow response must be JSON")
}

/// DELETE a workflow by ID. Returns the HTTP status code.
async fn delete_workflow(client: &Client, base: &str, pubkey_hex: &str, workflow_id: &str) -> u16 {
    let url = format!("{base}/api/workflows/{workflow_id}");
    client
        .delete(&url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .unwrap_or_else(|e| panic!("DELETE {url} failed: {e}"))
        .status()
        .as_u16()
}

// ── Test 1: List workflows (empty) ────────────────────────────────────────────

/// GET /api/channels/:id/workflows returns 200 OK with a valid JSON array.
/// The channel may have workflows from other tests, but the response must be
/// a well-formed array where every element has at least `id` and `name`.
#[tokio::test]
#[ignore]
async fn test_list_workflows_empty_channel() {
    let client = http_client();
    // Any authenticated user can list workflows in an open channel.
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let base = relay_http_url();

    let url = format!("{base}/api/channels/{CHANNEL_GENERAL}/workflows");
    let resp = client
        .get(&url)
        .header("X-Pubkey", &pubkey_hex)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url} failed: {e}"));

    assert_eq!(
        resp.status(),
        200,
        "expected 200 OK from GET /api/channels/:id/workflows"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert!(body.is_array(), "expected JSON array, got: {body}");

    let arr = body.as_array().unwrap();
    for wf in arr {
        assert!(wf.get("id").is_some(), "workflow missing 'id' field");
        assert!(wf.get("name").is_some(), "workflow missing 'name' field");
    }
}

// ── Test 2: Create + list workflow ────────────────────────────────────────────

/// POST /api/channels/:id/workflows creates a workflow, and it appears in the
/// subsequent GET list. Cleans up after itself by deleting the created workflow.
#[tokio::test]
#[ignore]
async fn test_create_and_list_workflow() {
    let client = http_client();
    // Must use a pubkey that exists in `users` table (FK constraint on workflows.owner_pubkey).
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    let yaml = webhook_workflow_yaml("e2e-create-list-test");
    let created = create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, &yaml).await;

    let workflow_id = created["id"]
        .as_str()
        .expect("created workflow must have 'id'");
    assert_eq!(
        created["name"].as_str().unwrap_or(""),
        "e2e-create-list-test",
        "workflow name must match"
    );
    assert!(
        created.get("channel_id").is_some(),
        "created workflow must have 'channel_id'"
    );
    // Webhook workflows get a secret on creation.
    assert!(
        created.get("webhook_secret").is_some(),
        "webhook workflow must return 'webhook_secret' on creation"
    );

    let list_url = format!("{base}/api/channels/{CHANNEL_GENERAL}/workflows");
    let list_resp = client
        .get(&list_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET workflows list failed");
    assert_eq!(list_resp.status(), 200);

    let list: Vec<serde_json::Value> = list_resp.json().await.expect("list must be JSON array");
    let found = list.iter().any(|wf| wf["id"].as_str() == Some(workflow_id));
    assert!(
        found,
        "newly created workflow {workflow_id} not found in list"
    );

    let status = delete_workflow(&client, &base, pubkey_hex, workflow_id).await;
    assert_eq!(status, 204, "cleanup DELETE should return 204");
}

// ── Test 3: Trigger workflow + check run ──────────────────────────────────────

/// Create a webhook-triggered workflow, POST to its trigger endpoint, then
/// verify a run record appears in GET /api/workflows/:id/runs.
///
/// The trigger endpoint returns 202 Accepted and spawns execution asynchronously.
/// We poll briefly for the run to appear (up to ~1 second).
#[tokio::test]
#[ignore]
async fn test_trigger_workflow_and_check_run() {
    let client = http_client();
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    let yaml = webhook_workflow_yaml("e2e-trigger-test");
    let created = create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, &yaml).await;
    let workflow_id = created["id"]
        .as_str()
        .expect("workflow must have 'id'")
        .to_string();

    let trigger_url = format!("{base}/api/workflows/{workflow_id}/trigger");
    let trigger_resp = client
        .post(&trigger_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {trigger_url} failed: {e}"));

    assert_eq!(
        trigger_resp.status(),
        202,
        "trigger endpoint must return 202 Accepted"
    );

    let trigger_body: serde_json::Value = trigger_resp
        .json()
        .await
        .expect("trigger response must be JSON");
    let run_id = trigger_body["run_id"]
        .as_str()
        .expect("trigger response must include 'run_id'");
    assert_eq!(
        trigger_body["workflow_id"].as_str().unwrap_or(""),
        workflow_id,
        "trigger response workflow_id must match"
    );
    assert_eq!(
        trigger_body["status"].as_str().unwrap_or(""),
        "pending",
        "trigger response initial status must be 'pending'"
    );

    // Poll GET /api/workflows/:id/runs until the run appears (max ~1 s).
    let runs_url = format!("{base}/api/workflows/{workflow_id}/runs");
    let mut found_run: Option<serde_json::Value> = None;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let runs_resp = client
            .get(&runs_url)
            .header("X-Pubkey", pubkey_hex)
            .send()
            .await
            .expect("GET runs failed");
        assert_eq!(runs_resp.status(), 200, "GET runs must return 200");
        let runs: Vec<serde_json::Value> = runs_resp.json().await.expect("runs must be JSON array");
        if let Some(run) = runs.iter().find(|r| r["id"].as_str() == Some(run_id)) {
            found_run = Some(run.clone());
            break;
        }
    }

    let run = found_run.expect("run must appear in GET /api/workflows/:id/runs within 1 second");

    assert!(run.get("id").is_some(), "run missing 'id'");
    assert!(
        run.get("workflow_id").is_some(),
        "run missing 'workflow_id'"
    );
    assert!(run.get("status").is_some(), "run missing 'status'");

    let status = run["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "pending" | "running" | "completed" | "failed"),
        "run status '{status}' is not a recognized value"
    );

    let del_status = delete_workflow(&client, &base, pubkey_hex, &workflow_id).await;
    assert_eq!(del_status, 204, "cleanup DELETE should return 204");
}

// ── Test 5: Event-driven workflow execution ───────────────────────────────────

/// Send a kind:9 message to a channel that has a `message_posted` workflow.
/// Verify that the workflow engine creates a run record.
///
/// NOTE: Uses `SEEDED_PUBKEY` for workflow ownership due to the FK constraint
/// on `workflows.owner_pubkey`. The WebSocket sender uses fresh keys.
#[tokio::test]
#[ignore = "requires running relay"]
async fn test_event_driven_workflow_execution() {
    use nostr::{Kind, Tag};
    use sprout_test_client::SproutTestClient;

    let client = http_client();
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    // ── Step 1: Create a message_posted workflow in the general channel ───────
    let workflow_yaml = r#"name: event-driven-e2e-test
description: E2E test for message_posted trigger
trigger:
  on: message_posted
steps:
  - id: step1
    name: Acknowledge
    action: send_message
    text: "Workflow fired by event"
"#;
    let created = create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, workflow_yaml).await;
    let workflow_id = created["id"]
        .as_str()
        .expect("created workflow must have 'id'")
        .to_string();

    // ── Step 2: Connect via WebSocket and send a kind:9 message ───────────
    // Use fresh keys for the sender (channel is open, no auth required to post).
    let sender_keys = Keys::generate();
    let mut ws_client = SproutTestClient::connect(&relay_ws_url(), &sender_keys)
        .await
        .expect("ws connect failed");

    let h_tag = Tag::parse(["h", CHANNEL_GENERAL]).expect("tag parse failed");
    let event = nostr::EventBuilder::new(Kind::Custom(9), "trigger this workflow please")
        .tags([h_tag])
        .sign_with_keys(&sender_keys)
        .expect("sign event");

    ws_client
        .send_event(event)
        .await
        .expect("send event failed");

    // ── Step 3: Wait for the workflow engine to process the event ─────────────
    tokio::time::sleep(Duration::from_secs(3)).await;

    // ── Step 4: Check that a run was created ──────────────────────────────────
    let runs_url = format!("{base}/api/workflows/{workflow_id}/runs");
    let runs_resp = client
        .get(&runs_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET runs failed");
    assert_eq!(runs_resp.status(), 200, "GET runs must return 200");

    let runs: Vec<serde_json::Value> = runs_resp.json().await.expect("runs must be JSON array");
    assert!(
        !runs.is_empty(),
        "expected at least one workflow run after sending kind:9 event"
    );

    let run = &runs[0];
    assert!(run.get("id").is_some(), "run missing 'id'");
    assert!(
        run.get("workflow_id").is_some(),
        "run missing 'workflow_id'"
    );
    assert!(run.get("status").is_some(), "run missing 'status'");

    let status = run["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "pending" | "running" | "completed" | "failed"),
        "run status '{status}' is not a recognized value"
    );

    let _ = ws_client.disconnect().await;
    let del_status = delete_workflow(&client, &base, pubkey_hex, &workflow_id).await;
    assert_eq!(del_status, 204, "cleanup DELETE should return 204");
}

// ── Test 6: Event-driven workflow with filter ─────────────────────────────────

/// Verify that a `message_posted` workflow with a filter expression only fires
/// when the filter matches.
///
/// 1. Create a workflow with `filter: "str_contains(trigger_text, \"P1\")"`.
/// 2. Send a message that does NOT contain "P1" — expect zero runs.
/// 3. Send a message that DOES contain "P1" — expect one run.
///
/// NOTE: Filter evaluation is wired in WF-07. Until then, all matched-kind
/// events fire the workflow regardless of filter. This test documents the
/// intended behaviour so it can be un-skipped once WF-07 lands.
#[tokio::test]
#[ignore = "requires running relay with WF-07 filter evaluation"]
async fn test_event_driven_workflow_with_filter() {
    use nostr::{Kind, Tag};
    use sprout_test_client::SproutTestClient;

    let client = http_client();
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    // ── Step 1: Create a filtered message_posted workflow ─────────────────────
    let workflow_yaml = r#"name: filtered-event-e2e-test
description: E2E test for message_posted trigger with filter
trigger:
  on: message_posted
  filter: "str_contains(trigger_text, \"P1\")"
steps:
  - id: step1
    name: Notify
    action: send_message
    text: "P1 incident detected"
"#;
    let created = create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, workflow_yaml).await;
    let workflow_id = created["id"]
        .as_str()
        .expect("created workflow must have 'id'")
        .to_string();

    let sender_keys = Keys::generate();
    let mut ws_client = SproutTestClient::connect(&relay_ws_url(), &sender_keys)
        .await
        .expect("ws connect failed");

    // ── Step 2: Send a message that does NOT match the filter ─────────────────
    let h_tag = Tag::parse(["h", CHANNEL_GENERAL]).expect("tag parse failed");
    let non_matching =
        nostr::EventBuilder::new(Kind::Custom(9), "this is a routine update, nothing urgent")
            .tags([h_tag.clone()])
            .sign_with_keys(&sender_keys)
            .expect("sign event");

    ws_client
        .send_event(non_matching)
        .await
        .expect("send non-matching event failed");

    tokio::time::sleep(Duration::from_secs(2)).await;

    let runs_url = format!("{base}/api/workflows/{workflow_id}/runs");
    let runs_resp = client
        .get(&runs_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET runs (non-matching) failed");
    assert_eq!(runs_resp.status(), 200);
    let runs_after_non_match: Vec<serde_json::Value> =
        runs_resp.json().await.expect("runs must be JSON array");
    assert!(
        runs_after_non_match.is_empty(),
        "non-matching message must NOT trigger a workflow run, but got {} run(s)",
        runs_after_non_match.len()
    );

    // ── Step 3: Send a message that DOES match the filter ─────────────────────
    let matching = nostr::EventBuilder::new(Kind::Custom(9), "P1 alert: database is down")
        .tags([h_tag])
        .sign_with_keys(&sender_keys)
        .expect("sign event");

    ws_client
        .send_event(matching)
        .await
        .expect("send matching event failed");

    tokio::time::sleep(Duration::from_secs(3)).await;

    let runs_resp2 = client
        .get(&runs_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET runs (matching) failed");
    assert_eq!(runs_resp2.status(), 200);
    let runs_after_match: Vec<serde_json::Value> =
        runs_resp2.json().await.expect("runs must be JSON array");
    assert!(
        !runs_after_match.is_empty(),
        "matching message must trigger a workflow run"
    );

    let run = &runs_after_match[0];
    let status = run["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "pending" | "running" | "completed" | "failed"),
        "run status '{status}' is not a recognized value"
    );

    let _ = ws_client.disconnect().await;
    let del_status = delete_workflow(&client, &base, pubkey_hex, &workflow_id).await;
    assert_eq!(del_status, 204, "cleanup DELETE should return 204");
}

// ── Test 4: Workflow CRUD (update + delete) ───────────────────────────────────

/// Full CRUD lifecycle:
///   1. Create a workflow
///   2. GET it by ID — verify fields
///   3. PUT to update the name
///   4. GET again — verify updated name
///   5. DELETE it
///   6. GET — verify 404
#[tokio::test]
#[ignore]
async fn test_workflow_update_and_delete() {
    let client = http_client();
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    // ── Step 1: Create ────────────────────────────────────────────────────────
    let yaml_v1 = webhook_workflow_yaml("e2e-crud-original");
    let created = create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, &yaml_v1).await;
    let workflow_id = created["id"]
        .as_str()
        .expect("workflow must have 'id'")
        .to_string();

    // ── Step 2: GET by ID ─────────────────────────────────────────────────────
    let get_url = format!("{base}/api/workflows/{workflow_id}");
    let get_resp = client
        .get(&get_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET workflow failed");
    assert_eq!(get_resp.status(), 200, "GET workflow must return 200");
    let fetched: serde_json::Value = get_resp.json().await.expect("GET response must be JSON");
    assert_eq!(
        fetched["name"].as_str().unwrap_or(""),
        "e2e-crud-original",
        "fetched workflow name must match original"
    );
    assert_eq!(
        fetched["id"].as_str().unwrap_or(""),
        workflow_id,
        "fetched workflow id must match"
    );

    // ── Step 3: PUT to update ─────────────────────────────────────────────────
    let yaml_v2 = webhook_workflow_yaml("e2e-crud-updated");
    let put_url = format!("{base}/api/workflows/{workflow_id}");
    let put_resp = client
        .put(&put_url)
        .header("X-Pubkey", pubkey_hex)
        .json(&serde_json::json!({ "yaml_definition": yaml_v2 }))
        .send()
        .await
        .expect("PUT workflow failed");
    assert_eq!(put_resp.status(), 200, "PUT workflow must return 200");
    let updated: serde_json::Value = put_resp.json().await.expect("PUT response must be JSON");
    assert_eq!(
        updated["name"].as_str().unwrap_or(""),
        "e2e-crud-updated",
        "updated workflow name must reflect new YAML"
    );
    assert_eq!(
        updated["id"].as_str().unwrap_or(""),
        workflow_id,
        "PUT must return the same workflow id"
    );

    // ── Step 4: GET again — verify update persisted ───────────────────────────
    let get_resp2 = client
        .get(&get_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("second GET workflow failed");
    assert_eq!(get_resp2.status(), 200);
    let refetched: serde_json::Value = get_resp2.json().await.expect("second GET must be JSON");
    assert_eq!(
        refetched["name"].as_str().unwrap_or(""),
        "e2e-crud-updated",
        "re-fetched workflow must have updated name"
    );

    // ── Step 5: DELETE ────────────────────────────────────────────────────────
    let del_status = delete_workflow(&client, &base, pubkey_hex, &workflow_id).await;
    assert_eq!(del_status, 204, "DELETE must return 204 No Content");

    // ── Step 6: GET after delete — expect 404 ────────────────────────────────
    let get_after_del = client
        .get(&get_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .expect("GET after DELETE failed");
    assert_eq!(
        get_after_del.status(),
        404,
        "GET after DELETE must return 404"
    );
}

// ── Test 7: Approval gate (WF-08 stub) ────────────────────────────────────────

/// Create a workflow with a `request_approval` step, trigger it, and verify
/// the run fails with the "approval gates not yet implemented" message.
///
/// This test documents the current stub behavior. When WF-08 is implemented,
/// this test should be updated to verify the full approval round-trip:
/// create → trigger → poll for waiting_approval → grant → verify completed.
#[tokio::test]
#[ignore]
async fn test_approval_gate_stub_fails_gracefully() {
    let client = http_client();
    let pubkey_hex: &str = SEEDED_PUBKEY;
    let base = relay_http_url();

    // ── Step 1: Create a workflow with a request_approval step ────────────────
    let workflow_yaml = format!(
        r#"name: approval-test
description: Test approval gate
trigger:
  on: webhook
steps:
  - id: step1
    name: Pre-approval step
    action: send_message
    channel: "{CHANNEL_GENERAL}"
    text: "Before approval"
  - id: approve
    action: request_approval
    from: "any"
    message: "Please approve this workflow"
  - id: step3
    name: Post-approval step
    action: send_message
    channel: "{CHANNEL_GENERAL}"
    text: "After approval"
"#
    );
    let created =
        create_workflow(&client, &base, pubkey_hex, CHANNEL_GENERAL, &workflow_yaml).await;
    let workflow_id = created["id"]
        .as_str()
        .expect("created workflow must have 'id'")
        .to_string();

    // ── Step 2: Trigger the workflow ──────────────────────────────────────────
    let trigger_url = format!("{base}/api/workflows/{workflow_id}/trigger");
    let trigger_resp = client
        .post(&trigger_url)
        .header("X-Pubkey", pubkey_hex)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {trigger_url} failed: {e}"));

    assert_eq!(
        trigger_resp.status(),
        202,
        "trigger endpoint must return 202 Accepted"
    );

    let trigger_body: serde_json::Value = trigger_resp
        .json()
        .await
        .expect("trigger response must be JSON");
    let run_id = trigger_body["run_id"]
        .as_str()
        .expect("trigger response must include 'run_id'")
        .to_string();

    // ── Step 3: Poll until the run reaches a terminal status ──────────────────
    let runs_url = format!("{base}/api/workflows/{workflow_id}/runs");
    let mut final_run: Option<serde_json::Value> = None;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let runs_resp = client
            .get(&runs_url)
            .header("X-Pubkey", pubkey_hex)
            .send()
            .await
            .expect("GET runs failed");
        assert_eq!(runs_resp.status(), 200, "GET runs must return 200");
        let runs: Vec<serde_json::Value> = runs_resp.json().await.expect("runs must be JSON array");
        if let Some(run) = runs.iter().find(|r| r["id"].as_str() == Some(&run_id)) {
            let status = run["status"].as_str().unwrap_or("");
            if matches!(status, "completed" | "failed" | "cancelled") {
                final_run = Some(run.clone());
                break;
            }
        }
    }

    // ── Step 4: Assert the run failed with the expected stub error ────────────
    let run = final_run.expect("run must reach a terminal status within 1 second");

    assert_eq!(
        run["status"].as_str().unwrap_or(""),
        "failed",
        "approval gate stub must cause the run to fail"
    );

    let error_msg = run["error_message"].as_str().unwrap_or("");
    assert!(
        error_msg.contains("approval gates not yet implemented"),
        "run error must contain 'approval gates not yet implemented', got: {error_msg:?}"
    );

    // ── Step 5: Clean up ──────────────────────────────────────────────────────
    let del_status = delete_workflow(&client, &base, pubkey_hex, &workflow_id).await;
    assert_eq!(del_status, 204, "cleanup DELETE should return 204");
}
