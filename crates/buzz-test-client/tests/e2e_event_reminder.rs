//! End-to-end integration tests for NIP-ER (Event Reminders, kind:30300).
//!
//! These tests verify:
//! - Write-path validation: `not_before` tag parsing, duplicate rejection,
//!   expiration ordering
//! - Read-path filtering: author-only enforcement on REQ, COUNT, and the
//!   HTTP bridge (/query, /count)
//! - Scheduler delivery: the due-reminder poll pushes a reminder to the
//!   author's live subscription when `not_before` passes
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p buzz-test-client --test e2e_event_reminder -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use reqwest::Client;
use serde_json::Value;

const KIND_EVENT_REMINDER: u16 = 30300;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("e2e-niper-{name}-{}", uuid::Uuid::new_v4())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Build a valid kind:30300 reminder event with the given tags.
fn build_reminder(keys: &Keys, d_tag: &str, extra_tags: Vec<Tag>) -> nostr::Event {
    let mut tags = vec![
        Tag::parse(["d", d_tag]).unwrap(),
        Tag::parse(["alt", "Encrypted reminder"]).unwrap(),
    ];
    tags.extend(extra_tags);
    EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(tags)
    .sign_with_keys(keys)
    .unwrap()
}

/// Build a reminder with an explicit `created_at`, so replacement-ordering
/// tests are deterministic instead of racing the whole-second timestamp the
/// default builder stamps.
fn build_reminder_at(
    keys: &Keys,
    d_tag: &str,
    created_at: Timestamp,
    extra_tags: Vec<Tag>,
) -> nostr::Event {
    let mut tags = vec![
        Tag::parse(["d", d_tag]).unwrap(),
        Tag::parse(["alt", "Encrypted reminder"]).unwrap(),
    ];
    tags.extend(extra_tags);
    EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(tags)
    .custom_created_at(created_at)
    .sign_with_keys(keys)
    .unwrap()
}

/// Submit an event via the HTTP bridge and return (accepted, message).
async fn submit_event_http(client: &Client, keys: &Keys, event: &nostr::Event) -> (bool, String) {
    let pubkey_hex = keys.public_key().to_hex();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    if status == 200 {
        let accepted = body["accepted"].as_bool().unwrap_or(false);
        let message = body["message"].as_str().unwrap_or("").to_string();
        (accepted, message)
    } else {
        // Rejections come back as `api_error` → `{"error": msg}` with no
        // `accepted`/`message` fields (see relay api/mod.rs). Mirror the
        // sibling `count_events_http`, which reads `error` on non-200.
        let message = body["error"].as_str().unwrap_or("").to_string();
        (false, message)
    }
}

/// Query events via the HTTP bridge. Returns the JSON array of events.
async fn query_events_http(client: &Client, pubkey_hex: &str, filters: Vec<Filter>) -> Vec<Value> {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("query events");
    assert!(
        resp.status().is_success(),
        "query failed: {}",
        resp.status()
    );
    resp.json::<Vec<Value>>()
        .await
        .expect("parse query response")
}

/// Count events via the HTTP bridge. Returns the count or an error status.
async fn count_events_http(
    client: &Client,
    pubkey_hex: &str,
    filters: Vec<Filter>,
) -> Result<u64, (u16, String)> {
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("count events");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse count response");
    if status == 200 {
        Ok(body["count"].as_u64().unwrap_or(0))
    } else {
        let msg = body["error"].as_str().unwrap_or("").to_string();
        Err((status, msg))
    }
}

// ── Write-path validation tests ──────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_reminder_accepted_with_valid_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "valid reminder rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_reminder_accepted_missing_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // No not_before tag — valid for terminal states (done/cancelled) and bookmarks
    let event = build_reminder(&keys, &d_tag, vec![]);
    let (accepted, _msg) = submit_event_http(&client, &keys, &event).await;
    assert!(
        accepted,
        "should accept missing not_before (bookmark/terminal)"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_duplicate_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![
            Tag::parse(["not_before", "1717000000"]).unwrap(),
            Tag::parse(["not_before", "1717000005"]).unwrap(),
        ],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject duplicate not_before");
    assert!(
        msg.contains("malformed not_before"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_malformed_not_before_leading_zero() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "007"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject leading-zero not_before");
    assert!(
        msg.contains("malformed not_before"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_malformed_not_before_non_digits() {
    let client = http_client();
    let keys = Keys::generate();

    for value in ["abc", "-1", "1.0", "1e3", " 1", ""] {
        let d_tag = uuid::Uuid::new_v4().to_string();
        let event = build_reminder(
            &keys,
            &d_tag,
            vec![Tag::parse(["not_before", value]).unwrap()],
        );
        let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
        assert!(
            !accepted,
            "should reject not_before={value:?}, got accepted"
        );
        assert!(
            msg.contains("malformed not_before"),
            "value={value:?}, unexpected message: {msg}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_not_before_above_max_safe_integer() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "9007199254740992"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject above MAX_SAFE_INTEGER");
    assert!(
        msg.contains("malformed not_before"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_expiration_before_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![
            Tag::parse(["not_before", "1717000000"]).unwrap(),
            Tag::parse(["expiration", "1716000000"]).unwrap(),
        ],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject expiration < not_before");
    assert!(
        msg.contains("expiration before not_before"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_expiration_equal_to_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![
            Tag::parse(["not_before", "1717000000"]).unwrap(),
            Tag::parse(["expiration", "1717000000"]).unwrap(),
        ],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject expiration == not_before");
    assert!(
        msg.contains("expiration before not_before"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_accepted_with_expiration_after_not_before() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![
            Tag::parse(["not_before", "1717000000"]).unwrap(),
            Tag::parse(["expiration", "1717000001"]).unwrap(),
        ],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(
        accepted,
        "valid expiration after not_before rejected: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_accepted_with_malformed_expiration() {
    // Malformed expiration is NIP-40's concern — should not block the reminder
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![
            Tag::parse(["not_before", "1717000000"]).unwrap(),
            Tag::parse(["expiration", "notanumber"]).unwrap(),
        ],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(
        accepted,
        "malformed expiration should not block reminder: {msg}"
    );
}

// ── d-tag validation tests ──────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_missing_d_tag() {
    let client = http_client();
    let keys = Keys::generate();

    // Build event without d tag
    let tags = vec![Tag::parse(["not_before", "1717000000"]).unwrap()];
    let event = EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(tags)
    .sign_with_keys(&keys)
    .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject missing d tag");
    assert!(msg.contains("missing d tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_empty_d_tag() {
    let client = http_client();
    let keys = Keys::generate();

    let tags = vec![
        Tag::parse(["d", ""]).unwrap(),
        Tag::parse(["not_before", "1717000000"]).unwrap(),
    ];
    let event = EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(tags)
    .sign_with_keys(&keys)
    .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject empty d tag");
    assert!(msg.contains("empty d tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_duplicate_d_tag() {
    let client = http_client();
    let keys = Keys::generate();

    let tags = vec![
        Tag::parse(["d", "abc"]).unwrap(),
        Tag::parse(["d", "def"]).unwrap(),
        Tag::parse(["not_before", "1717000000"]).unwrap(),
    ];
    let event = EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(tags)
    .sign_with_keys(&keys)
    .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject duplicate d tag");
    assert!(msg.contains("duplicate d tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_reminder_accepted_expiration_without_not_before() {
    // Terminal/bookmark with expiration but no not_before — no ordering check applies
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["expiration", "1777542730"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(
        accepted,
        "expiration without not_before should be accepted: {msg}"
    );
}

// ── Read-path filtering tests (HTTP bridge) ──────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_author_can_query_own_reminders_http() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder
    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Author queries their own reminders
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(keys.public_key());
    let results = query_events_http(&client, &pubkey_hex, vec![filter]).await;

    assert!(
        results.iter().any(|e| {
            e["tags"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .any(|t| t[0] == "d" && t[1] == d_tag)
        }),
        "author should see their own reminder"
    );
}

#[tokio::test]
#[ignore]
async fn test_other_user_cannot_query_reminders_http() {
    let client = http_client();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();
    let other_pubkey_hex = other_keys.public_key().to_hex();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder as author
    let event = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Other user tries to query author's reminders — should get 403
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(author_keys.public_key());
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &other_pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("query events");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "should get 403 for querying another author's reminders"
    );
}

#[tokio::test]
#[ignore]
async fn test_author_can_count_own_reminders_http() {
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder
    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Author counts their own reminders
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(keys.public_key());
    let count = count_events_http(&client, &pubkey_hex, vec![filter])
        .await
        .expect("count should succeed for author");
    assert!(count >= 1, "author should count at least 1 reminder");
}

#[tokio::test]
#[ignore]
async fn test_other_user_cannot_count_reminders_http() {
    let client = http_client();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();
    let other_pubkey_hex = other_keys.public_key().to_hex();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder as author
    let event = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Other user tries to count author's reminders — should get 403
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(author_keys.public_key());
    let result = count_events_http(&client, &other_pubkey_hex, vec![filter]).await;
    assert!(
        result.is_err(),
        "should get error counting another author's reminders"
    );
    let (status, _) = result.unwrap_err();
    assert_eq!(status, 403);
}

// ── Read-path filtering tests (WebSocket) ────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_author_can_subscribe_to_own_reminders_ws() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder via HTTP first
    let client = http_client();
    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Subscribe via WebSocket as the author
    let mut ws = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let sid = sub_id("author-read");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(keys.public_key());
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");

    assert!(
        events.iter().any(|e| {
            e.tags.iter().any(|t| {
                t.as_slice().len() >= 2 && t.as_slice()[0] == "d" && t.as_slice()[1] == d_tag
            })
        }),
        "author should receive their own reminder via WS subscription"
    );

    ws.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_other_user_subscription_closed_for_author_only_kind_ws() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Store a reminder as author
    let client = http_client();
    let event = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Other user tries to subscribe to author's reminders
    let mut ws = BuzzTestClient::connect(&url, &other_keys)
        .await
        .expect("connect");
    let sid = sub_id("other-read");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(author_keys.public_key());
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");

    // Should get CLOSED with "restricted:" message
    let msg = ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got: {other:?}"),
    }

    ws.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_mixed_kind_filter_omits_other_authors_reminders_ws() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let reader_keys = Keys::generate();

    // Create a channel so the reader can send a kind:9 message
    let channel = {
        let client = reqwest::Client::new();
        let pubkey_hex = reader_keys.public_key().to_hex();
        let channel_uuid = uuid::Uuid::new_v4();
        let channel_name = format!("niper-e2e-{}", channel_uuid);
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
                Tag::parse(["name", &channel_name]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(&reader_keys)
            .unwrap();
        let resp = client
            .post(format!("{}/events", relay_http_url()))
            .header("X-Pubkey", &pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).unwrap())
            .send()
            .await
            .expect("create channel");
        assert!(resp.status().is_success());
        channel_uuid.to_string()
    };

    // Author stores a reminder
    let client = http_client();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let reminder = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &reminder).await;
    assert!(accepted, "reminder setup failed: {msg}");

    // Reader sends a kind:9 message in the channel
    let mut ws_reader = BuzzTestClient::connect(&url, &reader_keys)
        .await
        .expect("connect reader");
    let unique_content = format!("mixed-filter-test-{}", uuid::Uuid::new_v4());
    let ok = ws_reader
        .send_text_message(&reader_keys, &channel, &unique_content, 9)
        .await
        .expect("send message");
    assert!(ok.accepted, "message rejected: {}", ok.message);

    // Reader subscribes with a mixed-kind filter (kind:9 + kind:30300)
    // The filter doesn't exclusively target author-only kinds, so it passes
    // the pre-filter gate. But per-event filtering should omit the reminder.
    let sid = sub_id("mixed-kind");
    let filter = Filter::new().kinds(vec![Kind::Custom(9), Kind::Custom(KIND_EVENT_REMINDER)]);
    ws_reader
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = ws_reader
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");

    // Should see the kind:9 message but NOT the author's reminder
    let has_message = events.iter().any(|e| e.content == unique_content);
    let has_reminder = events
        .iter()
        .any(|e| e.kind == Kind::Custom(KIND_EVENT_REMINDER));

    assert!(has_message, "reader should see their own kind:9 message");
    assert!(
        !has_reminder,
        "reader should NOT see another author's reminders in mixed-kind results"
    );

    ws_reader.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_reminder_not_before_zero_accepted() {
    // Edge case: not_before=0 is valid per spec
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "0"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "not_before=0 should be valid: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_reminder_not_before_max_safe_integer_rejected_too_far_in_future() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // MAX_SAFE_INTEGER is structurally valid (NIP-ER.md:60, range 0..=2^53-1),
    // but ~285M years out exceeds the relay's default max_not_before horizon
    // (NIP-ER.md:130), so the relay correctly rejects it.
    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", "9007199254740991"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(
        !accepted,
        "MAX_SAFE_INTEGER exceeds horizon, should reject: {msg}"
    );
    assert!(
        msg.contains("too far in future"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_replacement_semantics() {
    // Verify parameterized replaceable behavior: same (pubkey, kind, d) replaces
    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // First version. Explicit distinct created_at makes replacement ordering
    // deterministic: the default builder stamps whole-second timestamps, so v1
    // and v2 landing in the same second would let the stale-write tiebreak
    // (created_at == existing && incoming_id >= existing_id) keep v1 and fail
    // the assert. v2 two seconds later can never collide.
    let v1_ts = Timestamp::now();
    let v2_ts = v1_ts + 2u64;
    let event1 = build_reminder_at(
        &keys,
        &d_tag,
        v1_ts,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event1).await;
    assert!(accepted, "first version rejected: {msg}");

    // Second version (snooze — later not_before)
    let event2 = build_reminder_at(
        &keys,
        &d_tag,
        v2_ts,
        vec![Tag::parse(["not_before", "1718000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event2).await;
    assert!(accepted, "replacement rejected: {msg}");

    // Query should return only the latest version
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(keys.public_key());
    let results = query_events_http(&client, &pubkey_hex, vec![filter]).await;

    let matching: Vec<&Value> = results
        .iter()
        .filter(|e| {
            e["tags"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .any(|t| t[0] == "d" && t[1] == d_tag)
        })
        .collect();

    assert_eq!(
        matching.len(),
        1,
        "should have exactly one version after replacement, got {}",
        matching.len()
    );

    // Verify it's the newer one (has not_before=1718000000)
    let reminder = matching[0];
    let has_new_not_before = reminder["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t[0] == "not_before" && t[1] == "1718000000");
    assert!(
        has_new_not_before,
        "should be the replacement version with not_before=1718000000"
    );
}

// ── Fan-out isolation, WS search isolation, WS COUNT tests ───────────────────

#[tokio::test]
#[ignore]
async fn test_fanout_isolation_other_user_does_not_receive_reminder() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();

    // Connect user B and open a wildcard subscription BEFORE the reminder is published.
    let mut ws_other = BuzzTestClient::connect(&url, &other_keys)
        .await
        .expect("connect other");
    let sid = sub_id("fanout-isolation");
    // Subscribe with a mixed-kind filter authored by B itself. This passes both
    // the p-gate (neither kind:9 nor 30300 is p-gated) and the author-only
    // pre-filter (authors=[self=B]), so the subscription registers for fan-out.
    // Reminder isolation is then proven by the per-event `is_author_only_event`
    // omission: A's reminder is silently dropped from B's live delivery rather
    // than rejected up front. A kindless wildcard would instead trip the
    // anti-harvest p-gate guard (CLOSED), proving the wrong mechanism.
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(9), Kind::Custom(KIND_EVENT_REMINDER)])
        .author(other_keys.public_key());
    ws_other
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    // Drain historical events (EOSE)
    let _historical = ws_other
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("drain historical");

    // User A publishes a reminder
    let client = http_client();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let reminder = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &reminder).await;
    assert!(accepted, "reminder setup failed: {msg}");

    // Wait briefly for fan-out to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    // User B should NOT receive the reminder via fan-out.
    // Try to receive — should timeout (no event delivered).
    let result = ws_other.recv_event(Duration::from_secs(2)).await;
    match result {
        Err(buzz_test_client::TestClientError::Timeout) => {
            // Expected: no event delivered to non-author
        }
        Ok(RelayMessage::Event { event, .. }) => {
            assert_ne!(
                event.kind,
                nostr::Kind::Custom(KIND_EVENT_REMINDER),
                "user B should NOT receive author-only reminder via fan-out"
            );
        }
        Ok(_) => {
            // Other message types (NOTICE, etc.) are fine
        }
        Err(e) => panic!("unexpected error: {e}"),
    }

    ws_other.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_ws_search_isolation_other_user_cannot_find_reminder() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();

    // Store a reminder as author with a unique searchable content marker
    let client = http_client();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let reminder = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &reminder).await;
    assert!(accepted, "reminder setup failed: {msg}");

    // User B does a NIP-50 search with kinds including 30300
    let mut ws_other = BuzzTestClient::connect(&url, &other_keys)
        .await
        .expect("connect other");
    let sid = sub_id("search-isolation");

    // Build a search filter that includes kind:30300
    let filter = Filter::new()
        .kinds(vec![
            nostr::Kind::Custom(9),
            nostr::Kind::Custom(KIND_EVENT_REMINDER),
        ])
        .search("nip44-ciphertext-placeholder");
    ws_other
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = ws_other
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");

    // User B should NOT see any kind:30300 events from other authors
    let has_reminder = events
        .iter()
        .any(|e| e.kind == nostr::Kind::Custom(KIND_EVENT_REMINDER));
    assert!(
        !has_reminder,
        "user B should NOT see another author's reminders in NIP-50 search results"
    );

    ws_other.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_ws_count_returns_zero_for_other_users_reminders() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let other_keys = Keys::generate();

    // Store a reminder as author
    let client = http_client();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let reminder = build_reminder(
        &author_keys,
        &d_tag,
        vec![Tag::parse(["not_before", "1717000000"]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &author_keys, &reminder).await;
    assert!(accepted, "reminder setup failed: {msg}");

    // User B sends a COUNT for kind:30300 targeting author A — should get CLOSED
    // with "restricted:" because the filter exclusively targets author-only kinds
    // with another user's pubkey.
    let mut ws_other = BuzzTestClient::connect(&url, &other_keys)
        .await
        .expect("connect other");
    let sid = sub_id("ws-count");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_EVENT_REMINDER))
        .author(author_keys.public_key());

    // Send raw COUNT message
    let count_msg = serde_json::json!(["COUNT", sid, filter]);
    ws_other.send_raw(&count_msg).await.expect("send COUNT");

    // Should get CLOSED with "restricted:" message
    let msg = ws_other
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:"),
                "expected restricted message for COUNT on another author's reminders, got: {message}"
            );
        }
        other => panic!("expected CLOSED for COUNT on another author's reminders, got: {other:?}"),
    }

    ws_other.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_reminder_rejected_not_before_too_far_in_future() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Set not_before to 2 years from now (exceeds default 1-year max_not_before_delta)
    let two_years_from_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 63_072_000; // ~2 years

    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", &two_years_from_now.to_string()]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "should reject not_before too far in future");
    assert!(
        msg.contains("not_before too far in future"),
        "unexpected message: {msg}"
    );
}

// ── Scheduler delivery test ──────────────────────────────────────────────────

/// True if the event carries a `d` tag equal to `d_tag`.
fn has_d_tag(event: &nostr::Event, d_tag: &str) -> bool {
    event.tags.iter().any(|t| {
        let parts = t.as_slice();
        parts.len() >= 2 && parts[0] == "d" && parts[1] == d_tag
    })
}

/// Wait for a live `EVENT` on `sub_id` whose `d` tag is `d_tag`, draining the
/// initial EOSE and any historical matches first. The post-EOSE delivery is the
/// scheduler's push — with the scheduler disabled, no such frame ever arrives
/// and this returns `Timeout`.
async fn await_scheduler_push(
    ws: &mut BuzzTestClient,
    sub_id: &str,
    d_tag: &str,
    timeout_dur: Duration,
) -> Result<(), String> {
    // Drain the stored-events phase up to EOSE; the reminder may appear here as
    // history, which proves storage but not the scheduler.
    ws.collect_until_eose(sub_id, Duration::from_secs(5))
        .await
        .map_err(|e| format!("EOSE phase failed: {e:?}"))?;

    let deadline = tokio::time::Instant::now() + timeout_dur;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return Err("no live scheduler push received before timeout".to_string());
        }
        match ws.recv_event(remaining).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == sub_id && has_d_tag(&event, d_tag) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(format!("recv failed: {e:?}")),
        }
    }
}

/// The scheduler tick delivers a due reminder to the author's live subscription.
///
/// Submits a reminder whose `not_before` is a few seconds out *before* any
/// WebSocket is connected, so the ingest-time fan-out has no subscriber to
/// deliver to. The author then subscribes and drains the historical EOSE while
/// the reminder is still not due — so the scheduler cannot have fired yet. Once
/// `not_before` passes, the only path that reaches the live subscription is the
/// scheduler polling `query_due_reminders` and publishing the event for
/// fan-out.
///
/// Requires a low scheduler interval; run the relay with
/// `SPROUT_REMINDER_SCHEDULER_INTERVAL_SECS=1`.
#[tokio::test]
#[ignore]
async fn test_scheduler_delivers_due_reminder_to_author_subscription() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    // Submit a reminder due a few seconds out, with no WebSocket connected — the
    // ingest fan-out therefore has zero live recipients, and the reminder is not
    // yet due so the scheduler will not have claimed it before we subscribe.
    let due_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3;
    let client = http_client();
    let event = build_reminder(
        &keys,
        &d_tag,
        vec![Tag::parse(["not_before", &due_at.to_string()]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "setup failed: {msg}");

    // Now subscribe as the author and wait for the scheduler to push the due
    // reminder live (after the historical EOSE).
    let mut ws = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let sid = sub_id("scheduler-delivery");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EVENT_REMINDER))
        .author(keys.public_key());
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let result = await_scheduler_push(&mut ws, &sid, &d_tag, Duration::from_secs(15)).await;

    ws.disconnect().await.expect("disconnect");
    result.expect("scheduler should deliver the due reminder to the author");
}
