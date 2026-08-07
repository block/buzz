//! End-to-end integration tests for the workflow approval gate lifecycle.
//!
//! These tests require a running relay instance with Postgres and Redis.
//! By default they are marked `#[ignore]` so that `cargo test` does not
//! fail in CI when infrastructure is not available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_workflow_approval -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test --test e2e_workflow_approval -- --ignored
//! ```

use std::sync::OnceLock;
use std::time::Duration;

use buzz_core::kind::{
    KIND_APPROVAL_DENY, KIND_APPROVAL_GRANT, KIND_WORKFLOW_APPROVAL_REQUESTED,
    KIND_WORKFLOW_CANCELLED, KIND_WORKFLOW_COMPLETED, KIND_WORKFLOW_DEF, KIND_WORKFLOW_TRIGGER,
};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use uuid::Uuid;

fn relay_http_url() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        std::env::var("RELAY_URL")
            .unwrap_or_else(|_| "ws://localhost:3000".to_string())
            .replace("wss://", "https://")
            .replace("ws://", "http://")
            .trim_end_matches('/')
            .to_string()
    })
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Submit a signed event to the relay via the HTTP bridge (`POST /events`).
/// Returns the parsed JSON response `{accepted, message, ...}`.
async fn submit_event(keys: &Keys, event: nostr::Event) -> serde_json::Value {
    let http_url = relay_http_url();
    let resp = http_client()
        .post(format!("{http_url}/events"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).expect("serialize event"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST /events failed: {e}"));
    let status = resp.status();
    let body = resp.text().await.expect("read /events body");
    serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("parse /events JSON: {e} (status={status}, body={body})"))
}

/// Create a channel via kind:9007 and return the channel UUID string.
async fn create_test_channel(keys: &Keys) -> String {
    let channel_uuid = Uuid::new_v4();
    let channel_name = format!("approval-e2e-{channel_uuid}");

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &channel_name]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let body = submit_event(keys, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {body}"
    );

    channel_uuid.to_string()
}

/// YAML for a workflow with a `request_approval` step followed by `send_message`.
fn approval_workflow_yaml(name: &str) -> String {
    format!(
        "name: {name}\n\
         description: approval gate e2e test\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: approve\n\
         \x20   name: Request approval\n\
         \x20   action: request_approval\n\
         \x20   from: 'any'\n\
         \x20   message: Please approve this workflow\n\
         \x20   timeout: 1h\n\
         \x20 - id: notify\n\
         \x20   name: Send notification\n\
         \x20   action: send_message\n\
         \x20   text: Workflow approved and completed\n"
    )
}

/// YAML for a simple workflow with only a `send_message` step (no approval).
fn simple_workflow_yaml(name: &str) -> String {
    format!(
        "name: {name}\n\
         description: simple workflow e2e test\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: step1\n\
         \x20   name: Notify\n\
         \x20   action: send_message\n\
         \x20   text: Hello from workflow\n"
    )
}

/// Define a workflow and return the server-generated workflow ID.
async fn define_workflow(keys: &Keys, channel_id: &str, yaml: &str, name: &str) -> String {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16), yaml)
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["name", name]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let body = submit_event(keys, event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "workflow def not accepted: {body}"
    );

    // The command executor returns `message: "response:{json}"` where json
    // carries `workflow_id`.
    let msg = body["message"].as_str().unwrap_or_default();
    let json_part = msg
        .strip_prefix("response:")
        .unwrap_or_else(|| panic!("workflow def OK message missing `response:` prefix: {msg:?}"));
    let resp: serde_json::Value = serde_json::from_str(json_part)
        .unwrap_or_else(|e| panic!("parse workflow def response json: {e} ({json_part:?})"));
    resp["workflow_id"]
        .as_str()
        .unwrap_or_else(|| panic!("workflow def response missing workflow_id: {resp}"))
        .to_string()
}

/// Trigger a workflow by ID. Returns the response body.
async fn trigger_workflow(keys: &Keys, workflow_id: &str) -> serde_json::Value {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER as u16), "")
        .tags(vec![Tag::parse(["d", workflow_id]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();

    submit_event(keys, event).await
}

/// Query the relay via `POST /query` for events matching the given filter.
/// Returns the parsed event array.
async fn query_events(keys: &Keys, filter: Filter) -> Vec<serde_json::Value> {
    let http_url = relay_http_url();
    let filter_json = serde_json::to_value(&filter).expect("serialize filter");
    let body = serde_json::json!([filter_json]);

    let resp = http_client()
        .post(format!("{http_url}/query"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).unwrap())
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST /query failed: {e}"));

    let status = resp.status();
    let resp_body = resp.text().await.expect("read /query body");
    assert!(
        status.is_success(),
        "POST /query returned HTTP {status}: {resp_body}"
    );

    serde_json::from_str(&resp_body)
        .unwrap_or_else(|e| panic!("parse /query JSON: {e} (body={resp_body})"))
}

/// Poll for events of a given kind in a channel until at least one appears,
/// or until a timeout is reached. Returns the matching events.
async fn poll_for_events(
    keys: &Keys,
    channel_id: &str,
    kind: u32,
    timeout: Duration,
) -> Vec<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let filter = Filter::new()
            .kind(Kind::Custom(kind as u16))
            .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel_id]);
        let events = query_events(keys, filter).await;
        if !events.is_empty() {
            return events;
        }
        if tokio::time::Instant::now() >= deadline {
            return events;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Extract the `d` tag value from a JSON event object.
fn extract_d_tag_from_json(event: &serde_json::Value) -> Option<String> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|t| t[0].as_str() == Some("d"))
        .and_then(|t| t[1].as_str().map(String::from))
}

// ---------------------------------------------------------------------------
// Test: approval_grant_resumes_workflow
// ---------------------------------------------------------------------------

/// Create a workflow with `request_approval` then `send_message`. Trigger it.
/// Verify the run transitions to WaitingApproval. Query for the pending
/// approval kind:46010 event and extract the token hash from its `d` tag.
/// Submit a kind:46030 grant event with the token hash. Verify the run
/// completes (kind:46005 emitted and send_message executed).
#[tokio::test]
#[ignore]
async fn approval_grant_resumes_workflow() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define workflow with approval gate.
    let name = format!("approval_grant_{}", Uuid::new_v4().simple());
    let yaml = approval_workflow_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;
    assert!(
        Uuid::parse_str(&workflow_id).is_ok(),
        "workflow_id must be a UUID, got {workflow_id:?}"
    );

    // 2. Trigger the workflow.
    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 3. Poll for kind:46010 (approval requested) event in the channel.
    //    This confirms the run reached WaitingApproval and emitted the notification.
    let approval_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected at least one kind:46010 approval-requested event in channel {channel_id}"
    );

    // 4. Extract the token hash from the `d` tag of the approval-requested event.
    let approval_event = &approval_events[0];
    let token_hash_hex = extract_d_tag_from_json(approval_event)
        .expect("kind:46010 event must have a `d` tag with the token hash");
    assert!(
        !token_hash_hex.is_empty(),
        "token hash in `d` tag must not be empty"
    );

    // 5. Submit a kind:46030 grant event referencing the token hash.
    let grant_event = EventBuilder::new(Kind::Custom(KIND_APPROVAL_GRANT as u16), "")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let grant_resp = submit_event(&keys, grant_event).await;
    assert!(
        grant_resp["accepted"].as_bool().unwrap_or(false),
        "approval grant not accepted: {grant_resp}"
    );
    let grant_msg = grant_resp["message"].as_str().unwrap_or_default();
    assert!(
        grant_msg.contains("granted"),
        "grant response should contain 'granted', got: {grant_msg}"
    );

    // 6. Poll for kind:46005 (workflow completed) to verify the run resumed
    //    and completed after the approval was granted.
    let completed_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_COMPLETED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !completed_events.is_empty(),
        "expected kind:46005 workflow-completed event after grant; workflow did not resume"
    );
}

// ---------------------------------------------------------------------------
// Test: approval_deny_cancels_workflow
// ---------------------------------------------------------------------------

/// Same setup as grant test, but submit kind:46031 deny instead.
/// Verify the run transitions to Cancelled (kind:46007).
#[tokio::test]
#[ignore]
async fn approval_deny_cancels_workflow() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define workflow with approval gate.
    let name = format!("approval_deny_{}", Uuid::new_v4().simple());
    let yaml = approval_workflow_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;

    // 2. Trigger the workflow.
    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 3. Poll for kind:46010 (approval requested) event.
    let approval_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected kind:46010 approval-requested event"
    );

    // 4. Extract token hash from `d` tag.
    let token_hash_hex =
        extract_d_tag_from_json(&approval_events[0]).expect("kind:46010 event must have a `d` tag");

    // 5. Submit a kind:46031 deny event.
    let deny_event = EventBuilder::new(Kind::Custom(KIND_APPROVAL_DENY as u16), "Not approved")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let deny_resp = submit_event(&keys, deny_event).await;
    assert!(
        deny_resp["accepted"].as_bool().unwrap_or(false),
        "approval deny not accepted: {deny_resp}"
    );
    let deny_msg = deny_resp["message"].as_str().unwrap_or_default();
    assert!(
        deny_msg.contains("denied"),
        "deny response should contain 'denied', got: {deny_msg}"
    );

    // 6. Poll for kind:46007 (workflow cancelled) to verify the run was cancelled.
    let cancelled_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_CANCELLED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !cancelled_events.is_empty(),
        "expected kind:46007 workflow-cancelled event after deny; run was not cancelled"
    );
}

// ---------------------------------------------------------------------------
// Test: approval_emits_kind_46010
// ---------------------------------------------------------------------------

/// After triggering a workflow with `request_approval`, query for kind:46010
/// events in the channel. Verify one exists with the correct `d` tag containing
/// the token hash (SHA-256 hex of the approval token).
#[tokio::test]
#[ignore]
async fn approval_emits_kind_46010() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define and trigger workflow with approval gate.
    let name = format!("approval_46010_{}", Uuid::new_v4().simple());
    let yaml = approval_workflow_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;

    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 2. Poll for kind:46010 events.
    let approval_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected at least one kind:46010 event in channel {channel_id}"
    );

    // 3. Verify the event has a `d` tag with a 64-char hex token hash.
    let event = &approval_events[0];
    let d_tag = extract_d_tag_from_json(event).expect("kind:46010 must have a `d` tag");
    assert_eq!(
        d_tag.len(),
        64,
        "token hash in `d` tag should be 64-char SHA-256 hex, got {} chars: {d_tag}",
        d_tag.len()
    );
    assert!(
        d_tag.chars().all(|c| c.is_ascii_hexdigit()),
        "token hash must be hex, got: {d_tag}"
    );

    // 4. Verify the event kind is correct.
    let event_kind = event["kind"].as_u64().unwrap_or(0);
    assert_eq!(
        event_kind, KIND_WORKFLOW_APPROVAL_REQUESTED as u64,
        "event kind must be 46010"
    );

    // 5. Verify the event has an `h` tag matching the channel.
    let has_h_tag = event["tags"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .any(|t| t[0].as_str() == Some("h") && t[1].as_str() == Some(&channel_id))
        })
        .unwrap_or(false);
    assert!(
        has_h_tag,
        "kind:46010 event must have an h tag for channel {channel_id}"
    );
}

// ---------------------------------------------------------------------------
// Test: workflow_without_approval_completes_normally
// ---------------------------------------------------------------------------

/// A simple `send_message` workflow with no approval step. Verify it completes
/// normally without any WaitingApproval transition. Regression test for R5:
/// ensures the approval gate mechanism does not interfere with workflows that
/// have no approval steps.
#[tokio::test]
#[ignore]
async fn workflow_without_approval_completes_normally() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define a simple workflow (no approval step).
    let name = format!("no_approval_{}", Uuid::new_v4().simple());
    let yaml = simple_workflow_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;
    assert!(
        Uuid::parse_str(&workflow_id).is_ok(),
        "workflow_id must be a UUID, got {workflow_id:?}"
    );

    // 2. Trigger the workflow.
    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 3. Poll for kind:46005 (workflow completed). A simple workflow with only
    //    send_message should complete without any approval gate.
    let completed_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_COMPLETED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !completed_events.is_empty(),
        "expected kind:46005 workflow-completed event for a simple workflow; \
         workflow did not complete normally"
    );

    // 4. Verify no kind:46010 (approval requested) events were emitted — this
    //    workflow has no approval step, so none should appear.
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_APPROVAL_REQUESTED as u16))
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::H),
            [channel_id.as_str()],
        );
    let approval_events = query_events(&keys, filter).await;
    assert!(
        approval_events.is_empty(),
        "expected NO kind:46010 events for a workflow without approval steps, \
         but found {}: approval gate incorrectly triggered",
        approval_events.len()
    );
}

// ---------------------------------------------------------------------------
// YAML helpers for edge-case tests
// ---------------------------------------------------------------------------

/// YAML for a workflow with a very short approval timeout (1 second).
fn short_timeout_approval_yaml(name: &str) -> String {
    format!(
        "name: {name}\n\
         description: short timeout approval e2e test\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: approve\n\
         \x20   name: Request approval\n\
         \x20   action: request_approval\n\
         \x20   from: 'any'\n\
         \x20   message: Quick approval\n\
         \x20   timeout: 1s\n\
         \x20 - id: notify\n\
         \x20   name: Send notification\n\
         \x20   action: send_message\n\
         \x20   text: Approved\n"
    )
}

/// YAML for a workflow with a specific hex pubkey as the approver.
fn pubkey_approver_yaml(name: &str, approver_hex: &str) -> String {
    format!(
        "name: {name}\n\
         description: pubkey-restricted approval e2e test\n\
         trigger:\n\
         \x20 on: webhook\n\
         steps:\n\
         \x20 - id: approve\n\
         \x20   name: Request approval\n\
         \x20   action: request_approval\n\
         \x20   from: '{approver_hex}'\n\
         \x20   message: Only the designated approver may grant\n\
         \x20   timeout: 1h\n\
         \x20 - id: notify\n\
         \x20   name: Send notification\n\
         \x20   action: send_message\n\
         \x20   text: Approved\n"
    )
}

// ---------------------------------------------------------------------------
// Test: approval_expired_token_rejected
// ---------------------------------------------------------------------------

/// Trigger a workflow with a 1-second timeout. Wait for expiry, then submit
/// a grant. The grant should be rejected because the approval has expired.
#[tokio::test]
#[ignore]
async fn approval_expired_token_rejected() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define workflow with short timeout.
    let name = format!("approval_expired_{}", Uuid::new_v4().simple());
    let yaml = short_timeout_approval_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;

    // 2. Trigger the workflow.
    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 3. Poll for kind:46010 (approval requested).
    let approval_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected kind:46010 event for expired-token test"
    );

    let token_hash_hex =
        extract_d_tag_from_json(&approval_events[0]).expect("kind:46010 must have a `d` tag");

    // 4. Wait for the 1-second timeout to expire.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 5. Submit a grant — should be rejected as expired.
    let grant_event = EventBuilder::new(Kind::Custom(KIND_APPROVAL_GRANT as u16), "")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let grant_resp = submit_event(&keys, grant_event).await;

    // The relay should reject the expired approval.
    let accepted = grant_resp["accepted"].as_bool().unwrap_or(true);
    let msg = grant_resp["message"].as_str().unwrap_or_default();
    assert!(
        !accepted || msg.contains("expired"),
        "expected expired approval rejection, got: accepted={accepted}, message={msg}"
    );
}

// ---------------------------------------------------------------------------
// Test: approval_wrong_approver_rejected
// ---------------------------------------------------------------------------

/// Define a workflow with a specific hex pubkey as the required approver.
/// Submit a grant from a different keypair. The grant should be rejected.
#[tokio::test]
#[ignore]
async fn approval_wrong_approver_rejected() {
    let designated_approver = Keys::generate();
    let workflow_owner = Keys::generate();
    let unauthorized_granter = Keys::generate();

    let channel_id = create_test_channel(&workflow_owner).await;

    // 1. Define workflow restricted to the designated approver's pubkey.
    let name = format!("approval_wrong_approver_{}", Uuid::new_v4().simple());
    let yaml = pubkey_approver_yaml(&name, &designated_approver.public_key().to_hex());
    let workflow_id = define_workflow(&workflow_owner, &channel_id, &yaml, &name).await;

    // 2. Trigger the workflow.
    let trigger_resp = trigger_workflow(&workflow_owner, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 3. Poll for kind:46010 (approval requested).
    let approval_events = poll_for_events(
        &workflow_owner,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected kind:46010 event for wrong-approver test"
    );

    let token_hash_hex =
        extract_d_tag_from_json(&approval_events[0]).expect("kind:46010 must have a `d` tag");

    // 4. Submit a grant from the WRONG keypair (not the designated approver).
    let grant_event = EventBuilder::new(Kind::Custom(KIND_APPROVAL_GRANT as u16), "")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&unauthorized_granter)
        .unwrap();
    let grant_resp = submit_event(&unauthorized_granter, grant_event).await;

    // The relay should reject the unauthorized approver.
    let accepted = grant_resp["accepted"].as_bool().unwrap_or(true);
    let msg = grant_resp["message"].as_str().unwrap_or_default();
    assert!(
        !accepted || msg.contains("forbidden"),
        "expected unauthorized approver rejection, got: accepted={accepted}, message={msg}"
    );
}

// ---------------------------------------------------------------------------
// Test: approval_double_grant_idempotent
// ---------------------------------------------------------------------------

/// Grant an approval successfully, then submit a second grant with the same
/// token hash. The second grant should be rejected because the approval
/// status is no longer `pending`.
#[tokio::test]
#[ignore]
async fn approval_double_grant_idempotent() {
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    // 1. Define and trigger workflow with approval gate.
    let name = format!("approval_double_grant_{}", Uuid::new_v4().simple());
    let yaml = approval_workflow_yaml(&name);
    let workflow_id = define_workflow(&keys, &channel_id, &yaml, &name).await;

    let trigger_resp = trigger_workflow(&keys, &workflow_id).await;
    assert!(
        trigger_resp["accepted"].as_bool().unwrap_or(false),
        "workflow trigger not accepted: {trigger_resp}"
    );

    // 2. Poll for kind:46010 (approval requested).
    let approval_events = poll_for_events(
        &keys,
        &channel_id,
        KIND_WORKFLOW_APPROVAL_REQUESTED,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        !approval_events.is_empty(),
        "expected kind:46010 event for double-grant test"
    );

    let token_hash_hex =
        extract_d_tag_from_json(&approval_events[0]).expect("kind:46010 must have a `d` tag");

    // 3. First grant — should succeed.
    let grant_event_1 = EventBuilder::new(Kind::Custom(KIND_APPROVAL_GRANT as u16), "")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let grant_resp_1 = submit_event(&keys, grant_event_1).await;
    assert!(
        grant_resp_1["accepted"].as_bool().unwrap_or(false),
        "first grant should be accepted: {grant_resp_1}"
    );

    // 4. Second grant with same token hash — should be rejected.
    let grant_event_2 = EventBuilder::new(Kind::Custom(KIND_APPROVAL_GRANT as u16), "")
        .tags(vec![Tag::parse(["d", &token_hash_hex]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let grant_resp_2 = submit_event(&keys, grant_event_2).await;

    let accepted_2 = grant_resp_2["accepted"].as_bool().unwrap_or(true);
    let msg_2 = grant_resp_2["message"].as_str().unwrap_or_default();
    assert!(
        !accepted_2 || msg_2.contains("already") || msg_2.contains("not pending"),
        "second grant should be rejected (approval no longer pending), \
         got: accepted={accepted_2}, message={msg_2}"
    );
}
