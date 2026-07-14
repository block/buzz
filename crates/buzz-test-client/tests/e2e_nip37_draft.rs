//! End-to-end integration tests for NIP-37 draft wraps (kind:31234),
//! channel-bound contract.
//!
//! Every kind:31234 must carry exactly one `h` UUID binding it to a Buzz
//! channel (or DM).  The relay enforces:
//!
//! - Structural: valid `d`/`k`/`h` tags, `p` forbidden
//! - Channel existence: `h` UUID must resolve to a live channel
//! - Membership: author must be a member of that channel
//! - Immutable binding: once written, the `h` tag is frozen per (author, d_tag)
//! - Author-only reads: REQ, WS COUNT, WS subscription, HTTP /query, /count
//! - FTS exclusion: search_tsv = NULL, never surfaces in NIP-50 results
//! - Workflow exclusion: draft events must not appear in workflow triggers
//! - NIP-11 advertisement: relay claims NIP-37
//!
//! # Running
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_nip37_draft -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{EventBuilder, Filter, Keys, Kind, Tag, Timestamp};
use reqwest::Client;
use serde_json::{json, Value};

const KIND_DRAFT: u16 = 31234;
const KIND_CREATE_CHANNEL: u16 = 9007;
const KIND_PUT_USER: u16 = 9000;
const KIND_REMOVE_USER: u16 = 9001;

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
    format!("e2e-nip37-{name}-{}", uuid::Uuid::new_v4())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Minimal syntactically-plausible NIP-44 v2 payload.
/// base64(b"\x02" + b"\x00" * 98) — 132 chars, decoded 99 bytes, first byte 0x02.
fn fake_nip44_v2() -> String {
    let mut s = String::from("Ag");
    s.push_str(&"A".repeat(130));
    s
}

/// Create an open channel as `owner`; returns the channel UUID string.
async fn create_open_channel(owner: &Keys) -> String {
    let client = http_client();
    let ch_id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_CHANNEL), "")
        .tags([
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["name", &format!("nip37-test-{ch_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("create channel");
    let body: Value = resp.json().await.expect("parse channel response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {body}"
    );
    ch_id
}

/// Create a private channel as `owner`; returns the channel UUID string.
async fn create_private_channel(owner: &Keys) -> String {
    let client = http_client();
    let ch_id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_CHANNEL), "")
        .tags([
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["name", &format!("nip37-priv-{ch_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("create private channel");
    let body: Value = resp.json().await.expect("parse channel response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "private channel creation not accepted: {body}"
    );
    ch_id
}

/// Add `member` to a channel via kind:9000 submitted by `owner` over HTTP.
async fn add_member_http(client: &Client, owner: &Keys, channel_id: &str, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(KIND_PUT_USER), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("add member");
    let body: Value = resp.json().await.expect("parse add-member response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "add member not accepted: {body}"
    );
}

/// Remove `member` from a channel via kind:9001 submitted by `owner` over HTTP.
async fn remove_member_http(client: &Client, owner: &Keys, channel_id: &str, member: &Keys) {
    let event = EventBuilder::new(Kind::Custom(KIND_REMOVE_USER), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("remove member");
    let body: Value = resp.json().await.expect("parse remove-member response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "remove member not accepted: {body}"
    );
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
        let message = body["error"].as_str().unwrap_or("").to_string();
        (false, message)
    }
}

/// Query events via HTTP bridge as `as_pubkey_hex`. Returns events array.
async fn query_events_http(
    client: &Client,
    as_pubkey_hex: &str,
    filters: Vec<Filter>,
) -> Vec<Value> {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", as_pubkey_hex)
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

/// Build a valid kind:31234 draft wrap event bound to `channel_id`.
fn build_draft(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    content: &str,
) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, channel_id, content, Timestamp::now())
}

/// Build a valid kind:31234 draft wrap event bound to `channel_id` at `ts`.
fn build_draft_at(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    content: &str,
    ts: Timestamp,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_DRAFT), content)
        .tags([
            Tag::parse(["d", d_tag]).unwrap(),
            Tag::parse(["k", k_val]).unwrap(),
            Tag::parse(["h", channel_id]).unwrap(),
        ])
        .custom_created_at(ts)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a kind:31234 draft with a short-lived `expiration` tag.
///
/// `secs_from_now` controls how far in the future the expiration is set.
/// Ingest requires `expiration` strictly in the future, so callers should use
/// a margin wide enough to survive CI scheduler pauses (~10s recommended).
fn build_expiring_draft(
    keys: &Keys,
    d_tag: &str,
    channel_id: &str,
    secs_from_now: u64,
) -> nostr::Event {
    let exp_ts = Timestamp::now().as_secs() + secs_from_now;
    EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", d_tag]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["expiration", &exp_ts.to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a blank-content tombstone (NIP-37 deletion) bound to `channel_id`.
fn build_tombstone(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    channel_id: &str,
    ts: Timestamp,
) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, channel_id, "", ts)
}

// ─── h-tag validation ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            // no h tag
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "missing h tag should be rejected");
    assert!(
        msg.contains("h` tag") || msg.contains("channel-bound"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let ch = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch]).unwrap(),
            Tag::parse(["h", &ch]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "duplicate h tag should be rejected");
    assert!(
        msg.contains("h` tag") || msg.contains("channel-bound"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_non_uuid_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", "not-a-uuid"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "non-UUID h tag should be rejected");
    assert!(
        msg.contains("UUID") || msg.contains("h` tag"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_nonexistent_channel_h_tag() {
    // h tag is a syntactically valid UUID, but no channel exists for it.
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let nonexistent_ch = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&keys, &d, "9", &nonexistent_ch, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "draft to nonexistent channel should be rejected");
    assert!(
        msg.contains("channel") || msg.contains("not found") || msg.contains("member"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_non_member_author() {
    // Channel exists but author is not a member.
    let client = http_client();
    let owner = Keys::generate();
    let non_member = Keys::generate();

    let ch_id = create_private_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&non_member, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &non_member, &event).await;
    assert!(
        !accepted,
        "non-member should be unable to post draft: {msg}"
    );
    assert!(
        msg.contains("member") || msg.contains("restricted"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_by_channel_member() {
    // Channel owner is always a member — their draft must be accepted.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "owner draft must be accepted: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_after_member_removed() {
    // Member writes a draft; gets removed; attempts a replacement — must be rejected.
    let client = http_client();
    let owner = Keys::generate();
    let member = Keys::generate();
    let ch_id = create_private_channel(&owner).await;
    add_member_http(&client, &owner, &ch_id, &member).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &member,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &member, &v1).await;
    assert!(ok1, "member draft v1 must be accepted: {msg1}");

    remove_member_http(&client, &owner, &ch_id, &member).await;

    let v2 = build_draft(&member, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &member, &v2).await;
    assert!(
        !accepted,
        "removed member should not be able to update draft: {msg}"
    );
    assert!(
        msg.contains("member") || msg.contains("restricted"),
        "unexpected message: {msg}"
    );
}

// ─── Immutable channel binding ────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_channel_binding_is_immutable() {
    // Once a draft is bound to channel A, updating it with h=B must be rejected.
    let client = http_client();
    let owner = Keys::generate();
    let ch_a = create_open_channel(&owner).await;
    let ch_b = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_a,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "initial draft to ch_a must be accepted: {msg1}");

    // Attempt to update the same d to a different channel.
    let v2 = build_draft(&owner, &d, "9", &ch_b, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &v2).await;
    assert!(
        !accepted,
        "rebinding draft to a different channel must be rejected"
    );
    assert!(
        msg.contains("immutable") || msg.contains("channel"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_same_channel_replacement_accepted() {
    // Updating a draft on the same channel must succeed (normal NIP-33 replacement).
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;

    let d = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        Timestamp::from(now - 1),
    );
    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "v1 must be accepted: {msg1}");

    let v2 = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let v2_id = v2.id;
    let (ok2, msg2) = submit_event_http(&client, &owner, &v2).await;
    assert!(ok2, "v2 same-channel replacement must be accepted: {msg2}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "replacement must leave exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "v2 must be the current head"
    );
}

// ─── Ingest validation (structural) ──────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_accepted_with_ciphertext_content() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "valid draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_blank_tombstone() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = build_tombstone(&owner, &d, "9", &ch_id, Timestamp::now());
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "blank tombstone rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_future_expiration() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["expiration", "4102444800"]).unwrap(), // year 2100
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(accepted, "future expiration draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "missing d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_empty_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", ""]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "empty d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_oversized_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    // D_TAG_MAX_LEN is 1024 bytes in buzz-db. Use 1025 'a' chars.
    let d_tag = "a".repeat(1025);
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d_tag]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "oversized d tag should be rejected");
    assert!(
        msg.contains("d` tag") || msg.contains("too long"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_d_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "duplicate d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_k_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "missing k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_k_tag() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "duplicate k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_k_tag_non_decimal() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "0x9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "non-decimal k tag should be rejected");
    assert!(
        msg.contains("canonical decimal"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_k_tag_leading_zero() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "09"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "k tag with leading zero should be rejected");
    assert!(msg.contains("leading zero"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_k_tag_out_of_range() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "65536"]).unwrap(), // u16::MAX + 1
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "k=65536 should be rejected (out of u16 range)");
    assert!(msg.contains("range"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_p_tag() {
    let client = http_client();
    let owner = Keys::generate();
    // Use a different pubkey for the `p` tag — EventBuilder silently strips
    // `p` tags that match the signer's own key (NIP self-tagging rule), so
    // testing with owner.public_key() would produce an event with NO `p` tag
    // and the rejection would never be exercised.
    let other = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["p", &other.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "p tag on draft should be rejected");
    assert!(msg.contains("p` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_outer_e_tag() {
    // Smoke test: outer `e` tag on a kind:31234 event must be rejected.
    // Thread-reference context (reply-to, root) belongs inside the NIP-44
    // encrypted payload — plain outer `e` tags would expose the fact that
    // this draft is a reply/edit of a specific event to any relay reader.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let target_id = "0".repeat(64); // 64-char hex string representing a nonexistent event id
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            nostr::Tag::parse(["d", &d]).unwrap(),
            nostr::Tag::parse(["k", "9"]).unwrap(),
            nostr::Tag::parse(["h", &ch_id]).unwrap(),
            nostr::Tag::parse(["e", &target_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "outer `e` tag on draft should be rejected");
    assert!(
        msg.contains("e` tag") || msg.contains("e tag"),
        "unexpected rejection message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_ciphertext() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), "not-a-ciphertext")
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "malformed ciphertext should be rejected");
    assert!(
        msg.contains("base64") || msg.contains("NIP-44"),
        "unexpected message: {msg}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_expiration_in_past() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["expiration", "1000000000"]).unwrap(), // long past
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &event).await;
    assert!(!accepted, "past expiration should be rejected");
    assert!(msg.contains("expiration"), "unexpected message: {msg}");
}

// ─── NIP-01 replacement / tombstone ordering ─────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_replaced_by_newer_event() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t0 = Timestamp::from(now - 2);
    let t1 = Timestamp::from(now - 1);

    let v1 = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t0);
    let v2 = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t1);
    let v2_id = v2.id;

    let (ok1, msg1) = submit_event_http(&client, &owner, &v1).await;
    assert!(ok1, "v1 must be accepted: {msg1}");
    let (ok2, msg2) = submit_event_http(&client, &owner, &v2).await;
    assert!(ok2, "v2 must be accepted: {msg2}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "should return exactly the latest draft");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "latest event must be the returned head"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_stale_write_cannot_supersede_current_head() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_old = Timestamp::from(now - 2);
    let t_new = Timestamp::from(now - 1);

    let v_new = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_new);
    let v_old = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_old);

    let (ok_n, msg_n) = submit_event_http(&client, &owner, &v_new).await;
    assert!(ok_n, "newer draft must be accepted: {msg_n}");
    // The stale write must be accepted (relay returns `accepted: true` with
    // `duplicate:` or silently deduplicated) but MUST NOT become the new head.
    // The relay's stale-ordering protection keeps the newer event as head.
    let (stale_accepted, _stale_msg) = submit_event_http(&client, &owner, &v_old).await;
    assert!(
        stale_accepted,
        "stale write must be accepted (no-op), not hard-rejected; got: {_stale_msg}"
    );

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "should have exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v_new.id.to_hex(),
        "stale write must not replace current head"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_same_second_tie_break_lower_id_wins() {
    // Two events at identical timestamps: NIP-01 tie-break retains the one
    // with the lexically lower event ID, regardless of submission order.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let ts = Timestamp::now();

    // Sign candidates until we have at least two with distinct IDs. Add a
    // per-candidate unknown tag so each signing call produces a unique event
    // (different tag payload → different event hash → distinct IDs even if the
    // Schnorr nonce were deterministic).
    let mut candidates: Vec<nostr::Event> = Vec::new();
    for i in 0u32..20 {
        let e = EventBuilder::new(Kind::Custom(KIND_DRAFT), fake_nip44_v2())
            .tags([
                Tag::parse(["d", &d]).unwrap(),
                Tag::parse(["k", "9"]).unwrap(),
                Tag::parse(["h", &ch_id]).unwrap(),
                // Unique-per-candidate sentinel tag — forces distinct event hashes.
                Tag::parse(["_tiebreak", &i.to_string()]).unwrap(),
            ])
            .custom_created_at(ts)
            .sign_with_keys(&owner)
            .unwrap();
        candidates.push(e);
    }
    // Deduplicate by ID (should never trigger, but kept for safety).
    candidates.dedup_by_key(|e| e.id.to_hex());
    // The unique _tiebreak tag guarantees distinct event hashes — this must
    // always produce at least 2 distinct IDs.  A silent return here would
    // allow the test to pass without ever exercising the tie-break logic.
    assert!(
        candidates.len() >= 2,
        "expected at least 2 distinct candidate IDs with unique _tiebreak tags; got {}",
        candidates.len()
    );
    candidates.sort_by_key(|a| a.id.to_hex());
    let lowest = candidates.first().unwrap().clone();
    let highest = candidates.last().unwrap().clone();

    // Submit highest first, then lowest.
    let (ok_h, msg_h) = submit_event_http(&client, &owner, &highest).await;
    assert!(ok_h, "highest-id draft must be accepted: {msg_h}");
    let (ok_l, msg_l) = submit_event_http(&client, &owner, &lowest).await;
    assert!(ok_l, "lowest-id draft must be accepted: {msg_l}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "tie-break must leave exactly one head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        lowest.id.to_hex(),
        "lower event ID must win same-second tie"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_tombstone_head_queryable_by_author() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_draft = Timestamp::from(now - 1);
    let t_tomb = Timestamp::now();

    let draft = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), t_draft);
    let tombstone = build_tombstone(&owner, &d, "9", &ch_id, t_tomb);
    let tomb_id = tombstone.id;

    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");
    let (ok_t, msg_t) = submit_event_http(&client, &owner, &tombstone).await;
    assert!(ok_t, "tombstone must be accepted: {msg_t}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(results.len(), 1, "tombstone must be the queryable head");
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        tomb_id.to_hex(),
        "tombstone is the current head"
    );
    assert_eq!(
        results[0]["content"].as_str().unwrap(),
        "",
        "tombstone content must be empty"
    );
}

// ─── Author-only read gates ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_author_can_req_own_drafts_ws() {
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let mut c = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect author");
    let sid = sub_id("author-req");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());
    c.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = c
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        results.iter().any(|e| e.id == draft_id),
        "author must receive own draft"
    );
    c.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_req_victims_drafts_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("attacker-excl");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
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
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker received victim's draft via exclusive filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for exclusive draft filter, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_see_draft_in_mixed_kinds_filter_ws() {
    // A filter with kinds=[0,31234] must return the victim's public profile (kind:0)
    // but MUST NOT return their draft (kind:31234).
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Victim publishes a public profile (kind:0).
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "victim profile must be accepted: {msg_p}");

    // Victim publishes a draft.
    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "victim draft must be accepted: {msg_d}");

    // Attacker subscribes with an explicit kinds=[0,31234] filter.
    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("mixed-kinds-0-31234");
    let filter = Filter::new()
        .kinds(vec![Kind::Metadata, Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        results.iter().any(|e| e.id == profile_id),
        "attacker must receive victim's public profile (positive control)"
    );
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kinds=[0,31234] filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_see_draft_in_mixed_longform_kinds_filter_ws() {
    // A filter with kinds=[30023,31234] must return the victim's long-form note
    // but MUST NOT return their draft (kind:31234).
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d_draft = uuid::Uuid::new_v4().to_string();

    // Victim publishes a long-form note (kind:30023 — global replaceable, public).
    let d_article = uuid::Uuid::new_v4().to_string();
    let article = EventBuilder::new(Kind::Custom(30023), "article content")
        .tags([Tag::parse(["d", &d_article]).unwrap()])
        .sign_with_keys(&victim)
        .unwrap();
    let article_id = article.id;
    let (ok_a, msg_a) = submit_event_http(&client, &victim, &article).await;
    assert!(ok_a, "victim article must be accepted: {msg_a}");

    // Victim publishes a draft.
    let draft = build_draft(&victim, &d_draft, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "victim draft must be accepted: {msg_d}");

    // Attacker subscribes with kinds=[30023,31234].
    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("mixed-kinds-30023-31234");
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(30023), Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key());
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        results.iter().any(|e| e.id == article_id),
        "attacker must receive victim's public article (positive control)"
    );
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kinds=[30023,31234] filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_by_known_event_id_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("attacker-ids");
    let filter = Filter::new().id(draft_id);
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "knowing a draft's event id must not expose it to another user"
    );
    ac.disconnect().await.expect("disconnect");
}

// ─── known-#d privacy tripwires ───────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_by_known_d_tag_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("d-excl");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for #d exclusive filter, got: {message}"
            );
        }
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker retrieved victim's draft via exclusive #d filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for #d exclusive filter, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_draft_by_d_tag_in_mixed_kinds_ws() {
    // An attacker who knows the victim's d-tag value submits
    // kinds=[31234] + #d=[d_value] + author=[victim].  Must get CLOSED, not the event.
    //
    // Positive control: a public kind:30023 (long-form article) published under
    // the SAME `d` must be returned by kinds=[30023,31234]+author+#d — proving
    // the filter itself is not broken, only the draft is gated.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    // Use the same `d` value for both the draft AND the kind:30023 control.
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    // Publish a public kind:30023 with the same d-tag — this is the positive
    // control that proves kinds=[30023,31234]+#d is a live filter, not a no-op.
    let article = EventBuilder::new(Kind::Custom(30023), "long-form article content")
        .tags([Tag::parse(["d", &d]).unwrap()])
        .sign_with_keys(&victim)
        .unwrap();
    let article_id = article.id;
    let (ok_a, msg_a) = submit_event_http(&client, &victim, &article).await;
    assert!(ok_a, "victim article must be accepted: {msg_a}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("d-mixed-31234");
    // kinds=[31234] + #d=[d] is the sharpest possible known-address query.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for #d+kind:31234 filter, got: {message}"
            );
        }
        RelayMessage::Event { event, .. } => {
            panic!(
                "attacker retrieved victim's draft via #d+kind:31234 filter: event {}",
                event.id
            );
        }
        other => panic!("expected CLOSED for #d+kind:31234 filter, got: {other:?}"),
    }

    // Mixed-kinds positive control: kinds=[30023,31234] + author + #d=[same d].
    // The kind:30023 article must appear; the kind:31234 draft must NOT.
    // Using explicit kinds avoids the p-gated wildcard guard.
    let sid2 = sub_id("d-mixed-control");
    let filter2 = Filter::new()
        .kinds(vec![Kind::Custom(30023), Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.subscribe(&sid2, vec![filter2])
        .await
        .expect("subscribe mixed kinds");
    let mixed_results = ac
        .collect_until_eose(&sid2, Duration::from_secs(5))
        .await
        .expect("collect mixed kinds");
    assert!(
        mixed_results.iter().any(|e| e.id == article_id),
        "kind:30023 article under same d must appear in [30023,31234]+#d filter (positive control)"
    );
    assert!(
        !mixed_results.iter().any(|e| e.id == draft_id),
        "kind:31234 draft must not appear in [30023,31234]+#d filter for attacker"
    );

    ac.disconnect().await.expect("disconnect");
}

// ─── COUNT privacy gates ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_exclusive_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("count-ws");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    ac.send_raw(&json!(["COUNT", sid, filter]))
        .await
        .expect("send COUNT");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, sid);
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted message for COUNT on another author's drafts, got: {message}"
            );
        }
        other => {
            panic!("expected CLOSED for WS COUNT on another author's drafts, got: {other:?}")
        }
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_via_known_d_ws() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("count-ws-d");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    ac.send_raw(&json!(["COUNT", sid, filter]))
        .await
        .expect("send COUNT");

    let relay_msg = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv response");
    match relay_msg {
        RelayMessage::Closed { message, .. } => {
            assert!(
                message.contains("restricted:") || message.contains("author-only"),
                "expected restricted for #d COUNT, got: {message}"
            );
        }
        other => panic!("expected CLOSED for #d COUNT, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_exclusive_http() {
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "HTTP exclusive COUNT for another author's drafts must return 403"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_author_can_count_own_drafts_http() {
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &owner.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    assert!(
        resp.status().is_success(),
        "author's own count must succeed, got: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("parse count response");
    let count = body["count"].as_u64().unwrap_or(0);
    assert!(count >= 1, "author must count at least 1 own draft");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_mixed_kinds_count_excludes_drafts() {
    // A mixed-kinds COUNT filter (e.g. kinds=[9,31234]) from a non-author must
    // count only the public events — the kind:31234 draft must not be included.
    // This exercises the per-event fallback path in handle_count where the
    // reader_can_receive_event gate runs for each row in a mixed-kinds result.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Victim submits one kind:9 channel message and one kind:31234 draft.
    let msg = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&victim)
        .unwrap();
    let (ok_m, err_m) = submit_event_http(&client, &victim, &msg).await;
    assert!(ok_m, "kind:9 message must be accepted: {err_m}");

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok_d, err_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    // Attacker COUNTs with a mixed kinds=[9,31234] filter via HTTP /count.
    // Should be allowed (kindless-mixed filter is not exclusively author-only)
    // but the kind:31234 must be excluded from the count.
    let url = relay_url();
    let filter = serde_json::json!({
        "kinds": [9, KIND_DRAFT],
        "authors": [victim.public_key().to_hex()],
    });
    let resp = client
        .post(format!(
            "{}/count",
            url.replace("ws://", "http://")
                .replace("wss://", "https://")
        ))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    assert!(
        resp.status().is_success(),
        "mixed-kinds /count must succeed for attacker, got: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("parse count response");
    let count = body["count"].as_u64().unwrap_or(0);
    // The attacker sees at most 1 (the kind:9 message). The kind:31234 draft
    // must NOT be included.
    assert!(
        count <= 1,
        "mixed-kinds count for non-author must not include kind:31234 drafts; got count={count}"
    );
}

// ─── HTTP /query exclusive-author privacy ────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_query_exclusive_http() {
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(victim.public_key());
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("query request");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "exclusive other-author HTTP /query for kind:31234 must return 403"
    );
}

// ─── Live fan-out privacy ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_live_fanout_only_reaches_author() {
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid_fanout = sub_id("fanout-attacker");
    let filter = Filter::new()
        .kinds(vec![Kind::Metadata, Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key())
        .limit(0);
    ac.subscribe(&sid_fanout, vec![filter])
        .await
        .expect("subscribe to mixed filter");
    let _ = ac
        .collect_until_eose(&sid_fanout, Duration::from_secs(3))
        .await;

    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "profile must be accepted: {msg_p}");

    let mut received_draft = false;
    let mut received_profile = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            break;
        }
        match ac.recv_event(remaining).await {
            Ok(RelayMessage::Event { event, .. }) => {
                if event.id == draft_id {
                    received_draft = true;
                }
                if event.id == profile_id {
                    received_profile = true;
                }
            }
            _ => break,
        }
    }

    assert!(
        !received_draft,
        "attacker must NOT receive victim's draft via live fan-out"
    );
    assert!(
        received_profile,
        "attacker MUST receive victim's public profile (positive control)"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_live_fanout_reaches_author_own_subscription() {
    // Positive-control companion to test_draft_live_fanout_only_reaches_author:
    // the AUTHOR themselves MUST receive their own draft via live fan-out when
    // they have an active subscription for it.  This proves the author-only
    // gate is a filter (not a kill-switch) and doesn't silently break delivery
    // to the author.
    let url = relay_url();
    let client = http_client();
    let author = Keys::generate();
    let ch_id = create_open_channel(&author).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Author opens a subscription with limit:0 (skip historical, live only).
    // The subscription MUST be channel-scoped (#h ch_id): drafts are channel-bound
    // events and fan_out_scoped's symmetric scoping invariant means a global
    // (no #h) sub never sees channel-scoped events, regardless of the author-only
    // gate.
    let mut ac = BuzzTestClient::connect(&url, &author)
        .await
        .expect("connect author");
    let sid = sub_id("fanout-author-self");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_DRAFT))
        .author(author.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            ch_id.as_str(),
        )
        .limit(0);
    ac.subscribe(&sid, vec![filter])
        .await
        .expect("subscribe author draft filter");
    let _ = ac.collect_until_eose(&sid, Duration::from_secs(3)).await;

    // Author submits their own draft.
    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &author, &draft).await;
    assert!(ok, "author's draft must be accepted: {msg}");

    // Author's own subscription must deliver the draft via fan-out.
    let received = ac
        .recv_event(Duration::from_secs(5))
        .await
        .expect("author must receive their own draft via fan-out");
    let got_draft = matches!(&received, RelayMessage::Event { event, .. } if event.id == draft_id);
    assert!(
        got_draft,
        "author's own subscription must receive their draft via live fan-out; got: {received:?}"
    );
    ac.disconnect().await.expect("disconnect");
}

// ─── Nonexistent / alien channel rejection ────────────────────────────────────

// NOTE: test_draft_rejected_nonexistent_channel_h_tag (at the top of this file)
// already covers this: a draft with a valid-UUID h-tag pointing to no live
// channel is rejected.  That test is the single authoritative nonexistent-channel
// guard.  True cross-community tenant confinement is covered at the DB layer by
// `draft_is_confined_to_its_community` (requires Postgres, wired to CI).

// ─── Kindless channel query — draft privacy ───────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_not_returned_in_kindless_channel_query_by_attacker() {
    // A kindless channel h-tag filter submitted by a non-author must never
    // return the author's draft.  The attacker has channel membership (the
    // channel is open) and subscribes to all events in the channel — they
    // must receive the owner's public messages but not their drafts.
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    // Also publish a public channel message as a positive control.
    let msg_event = EventBuilder::new(Kind::Custom(9), "hello channel")
        .tags([Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let msg_id = msg_event.id;
    let (ok_m, msg_m) = submit_event_http(&client, &owner, &msg_event).await;
    assert!(ok_m, "channel message must be accepted: {msg_m}");

    // Attacker queries by channel h-tag, no kind filter.
    let mut c = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("ch-kindless-attacker");
    let filter = Filter::new().custom_tag(
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
        ch_id.as_str(),
    );
    c.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = c
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // Channel message must appear (positive control — attacker can see public messages).
    assert!(
        results.iter().any(|e| e.id == msg_id),
        "channel message must appear in attacker's h-tag query (positive control)"
    );
    // Draft must be absent — author-only gate must strip it before delivery.
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "draft must not be returned by a kindless channel h-tag filter to a non-author"
    );
    c.disconnect().await.expect("disconnect");
}

// ─── FTS / NIP-50 exclusion ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_not_returned_in_kindless_channel_http_query() {
    // HTTP bridge /query tests for draft privacy.  The bridge runs sensitive-kind
    // gates unconditionally, so a kindless h-tag filter (wildcard) triggers
    // p_gated_filters_authorized and returns 403 for any caller that is not the
    // p-tagged owner.  This is fail-closed, pre-existing main behavior — NOT a
    // production defect.
    //
    // Two sub-cases:
    //   (i)  Kindless h-tag /query as non-author → 403.  The fail-closed
    //        outcome IS the oracle; the route simply does not exist over HTTP.
    //  (ii)  Mixed kinds:[9, KIND_DRAFT] h-tag /query as attacker (passes all
    //        three HTTP gates — neither kind is p-gated, not exclusively
    //        author-only).  Kind:9 message must appear; kind:31234 draft must
    //        be absent, proving reader_can_receive_event is live on the bridge
    //        per-event path.
    let client = http_client();
    let owner = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    // Publish a public channel message for use as a positive control.
    let msg_event = EventBuilder::new(Kind::Custom(9), "hello http channel")
        .tags([Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let msg_id = msg_event.id;
    let (ok_m, msg_m) = submit_event_http(&client, &owner, &msg_event).await;
    assert!(ok_m, "channel message must be accepted: {msg_m}");

    // (i) Kindless h-tag /query as non-author must return 403 (fail-closed).
    let kindless_filter = Filter::new().custom_tag(
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
        ch_id.as_str(),
    );
    let kindless_resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![kindless_filter])
        .send()
        .await
        .expect("kindless query request");
    assert_eq!(
        kindless_resp.status().as_u16(),
        403,
        "kindless h-tag /query as non-author must be rejected with 403 (bridge fail-closed)"
    );

    // (ii) Mixed kinds:[9, KIND_DRAFT] h-tag /query as attacker.
    //      Both kinds pass the p-gated and exclusively-author-only HTTP gates,
    //      so the query reaches the per-event reader_can_receive_event guard.
    let mixed_filter = Filter::new()
        .kinds([Kind::Custom(9), Kind::Custom(KIND_DRAFT)])
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            ch_id.as_str(),
        );
    let results =
        query_events_http(&client, &attacker.public_key().to_hex(), vec![mixed_filter]).await;

    // Kind:9 message must appear — proves the filter reached the data path.
    assert!(
        results
            .iter()
            .any(|e| e["id"].as_str() == Some(&msg_id.to_hex())),
        "kind:9 channel message must appear in mixed-kinds HTTP query (positive control)"
    );
    // Draft must be absent — reader_can_receive_event must strip it for non-author.
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "kind:31234 draft must not appear in mixed-kinds HTTP query for non-author"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_not_indexed_in_fts_search() {
    // Proves the search surface returns no drafts: the HTTP /query search path
    // is exercised with a mixed kinds=[1,31234] filter, and the kind:31234 is
    // absent from results.
    //
    // This test CANNOT prove the NULL-tsvector storage guarantee by itself:
    // draft content is a NIP-44 ciphertext, so the search token could never
    // appear in stored content regardless of the tsvector column.  The
    // storage-layer guarantee (search_tsv = NULL for all kind:31234 rows) is
    // owned by `crates/buzz-search/tests/fts_integration.rs`.  This e2e test
    // only proves the search read-path returns no draft rows — a necessary
    // complement that exercises the HTTP bridge search branch end-to-end.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&victim).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Unique word token — used as kind:1 content (FTS-indexed) and as the
    // search query for both kinds.
    let token = format!("nip37probe{}", uuid::Uuid::new_v4().simple());

    // Kind:1 control note — MUST appear in FTS results.
    let note = EventBuilder::new(Kind::TextNote, &token)
        .sign_with_keys(&victim)
        .unwrap();
    let note_id = note.id;
    let (ok_note, msg_note) = submit_event_http(&client, &victim, &note).await;
    assert!(ok_note, "control note must be accepted: {msg_note}");

    // Kind:31234 draft — NIP-44 v2 content (relay validates). The draft
    // content is a ciphertext and cannot contain the plaintext token; the
    // search_tsv is also NULL at the storage layer (fts_integration.rs owns
    // that guarantee).  Either property would prevent this draft from surfacing.
    let draft = build_draft(&victim, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    // Search as the author with explicit kinds=[1,31234].  The kind:1 note IS
    // found (positive control); the kind:31234 draft is absent.
    let search_filter = Filter::new()
        .kinds(vec![Kind::TextNote, Kind::Custom(KIND_DRAFT)])
        .search(&token)
        .limit(50);
    let results =
        query_events_http(&client, &victim.public_key().to_hex(), vec![search_filter]).await;

    assert!(
        results
            .iter()
            .any(|e| e["id"].as_str() == Some(&note_id.to_hex())),
        "FTS must index the control kind:1 note (positive control)"
    );
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "kind:31234 must have NULL search_tsv — draft must not appear in NIP-50 search"
    );

    // Attacker-side check: search with kinds=[1,31234] as attacker.
    let attacker_filter = Filter::new()
        .kinds(vec![Kind::TextNote, Kind::Custom(KIND_DRAFT)])
        .search(&token)
        .limit(50);
    let attacker_results = query_events_http(
        &client,
        &attacker.public_key().to_hex(),
        vec![attacker_filter],
    )
    .await;
    assert!(
        !attacker_results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "draft must not appear in attacker's NIP-50 search either"
    );
}

// ─── NIP-11 advertisement ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_nip11_advertises_nip37_not_nip40() {
    let client = http_client();
    let resp = client
        .get(relay_http_url())
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .expect("NIP-11 request");
    assert!(resp.status().is_success());
    let info: Value = resp.json().await.expect("parse NIP-11 response");
    let nips = info["supported_nips"]
        .as_array()
        .expect("supported_nips must be an array");
    let nip_numbers: Vec<u64> = nips.iter().filter_map(|v| v.as_u64()).collect();
    assert!(
        nip_numbers.contains(&37),
        "NIP-11 must advertise NIP-37 (draft wraps); got {nip_numbers:?}"
    );
    assert!(
        !nip_numbers.contains(&40),
        "NIP-11 must NOT advertise NIP-40 (expiry suppression not implemented); got {nip_numbers:?}"
    );
}

// ─── NIP-09 deletion guard (a-tag and e-tag) ──────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_nip09_a_tag_deletion_of_draft_is_rejected() {
    // kind:5 with a single `a` tag targeting `31234:<pubkey>:<d>` must be
    // rejected at ingest.  The relay must never let a kind:5 event act as an
    // escape hatch to clear a draft's immutable channel binding.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();

    // First publish a draft so there is something to attempt to delete.
    let draft = build_draft(&owner, &d, "9", &ch_id, &fake_nip44_v2());
    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(
        ok_d,
        "draft must be accepted before the deletion attempt: {msg_d}"
    );

    // Build kind:5 with a single a-tag targeting the draft's NIP-33 address.
    let a_coord = format!("31234:{}:{}", owner.public_key().to_hex(), d);
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::parse(["a", &a_coord]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();

    let (accepted, msg) = submit_event_http(&client, &owner, &deletion).await;
    assert!(
        !accepted,
        "kind:5 a-tag deletion targeting kind:31234 must be rejected; relay said: {msg}"
    );
    assert!(
        msg.contains("31234")
            || msg.contains("draft")
            || msg.contains("not supported")
            || msg.contains("invalid"),
        "rejection message must explain why; got: {msg}"
    );

    // The draft must still exist as a live head — the rejected kind:5 must not
    // have modified anything.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "draft must still have one live head after rejected kind:5 deletion"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        draft.id.to_hex(),
        "live head must still be the original draft — kind:5 must not have altered it"
    );
}

#[tokio::test]
#[ignore]
async fn test_nip09_e_tag_deletion_of_draft_is_rejected_and_binding_holds() {
    // Full bypass sequence proving the e-tag deletion path cannot circumvent
    // the immutable channel binding.
    //
    // 1. Publish draft at address `d` on channel h=A (timestamp = base).
    // 2. Submit kind:5 with a single `e` tag pointing at the draft event id.
    //    The relay must reject this pre-storage.
    // 3. Query as the author — the original draft head must still be live.
    // 4. Attempt to rebind the same `d` address to h=B (timestamp = base+1,
    //    guaranteed newer) — must be rejected because the ch_a binding is intact.
    let client = http_client();
    let owner = Keys::generate();

    // Two distinct channels — owner is a member of both.
    let ch_a = create_open_channel(&owner).await;
    let ch_b = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let base = Timestamp::now().as_secs();

    // Step 1: publish draft bound to ch_a.
    let draft = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_a,
        &fake_nip44_v2(),
        Timestamp::from(base),
    );
    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(
        ok_d,
        "draft must be accepted before the deletion attempt: {msg_d}"
    );

    // Step 2: submit kind:5 with e-tag pointing at the draft event id.
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::parse(["e", &draft.id.to_hex()]).unwrap()])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &deletion).await;
    assert!(
        !accepted,
        "kind:5 e-tag deletion targeting kind:31234 must be rejected; relay said: {msg}"
    );
    assert!(
        msg.contains("31234")
            || msg.contains("draft")
            || msg.contains("not supported")
            || msg.contains("invalid"),
        "rejection message must explain why; got: {msg}"
    );

    // Step 3: author queries by address — draft head must still be live.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "draft must still have one live head after rejected kind:5 e-tag deletion"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        draft.id.to_hex(),
        "live head must be the original draft — e-tag kind:5 must not have altered it"
    );

    // Step 4: attempt rebind to ch_b with a strictly newer timestamp — must be
    // rejected because the ch_a binding is intact (the e-tag delete did not
    // erase the head row).
    let rebind = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_b,
        &fake_nip44_v2(),
        Timestamp::from(base + 1),
    );
    let (rebind_accepted, rebind_msg) = submit_event_http(&client, &owner, &rebind).await;
    assert!(
        !rebind_accepted,
        "rebind to ch_b must be rejected; relay said: {rebind_msg}"
    );
    assert!(
        rebind_msg.contains("channel")
            || rebind_msg.contains("mismatch")
            || rebind_msg.contains("immutable")
            || rebind_msg.contains("invalid"),
        "rebind rejection must name the binding invariant; got: {rebind_msg}"
    );
}

#[tokio::test]
#[ignore]
/// kind:9005 deletion of a kind:31234 draft must be rejected pre-storage and
/// the immutable channel binding must remain intact.
///
/// 1. Publish draft at address `d` on channel h=A (timestamp = base).
/// 2. Draft author submits kind:9005 with e=<draft event id> and h=A — must
///    be rejected pre-storage (tombstone-guidance error).
/// 3. Query as the author — the original draft head must still be live.
/// 4. Attempt to rebind the same `d` address to h=B (timestamp = base+1) —
///    must be rejected because the ch_a binding is intact.
async fn test_nip09_kind9005_deletion_of_draft_is_rejected_and_binding_holds() {
    let client = http_client();
    let owner = Keys::generate();

    // Two distinct channels — owner is a member of both.
    let ch_a = create_open_channel(&owner).await;
    let ch_b = create_open_channel(&owner).await;
    let d = uuid::Uuid::new_v4().to_string();
    let base = Timestamp::now().as_secs();

    // Step 1: publish draft bound to ch_a.
    let draft = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_a,
        &fake_nip44_v2(),
        Timestamp::from(base),
    );
    let (ok_d, msg_d) = submit_event_http(&client, &owner, &draft).await;
    assert!(
        ok_d,
        "draft must be accepted before the deletion attempt: {msg_d}"
    );

    // Step 2: author submits kind:9005 targeting the draft event id.
    let del_9005 = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["e", &draft.id.to_hex()]).unwrap(),
            Tag::parse(["h", &ch_a]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &owner, &del_9005).await;
    assert!(
        !accepted,
        "kind:9005 deletion targeting kind:31234 must be rejected; relay said: {msg}"
    );
    assert!(
        msg.contains("31234")
            || msg.contains("draft")
            || msg.contains("not supported")
            || msg.contains("invalid"),
        "rejection message must explain why; got: {msg}"
    );

    // Step 3: author queries by address — draft head must still be live.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &owner.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "draft must still have one live head after rejected kind:9005 deletion"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        draft.id.to_hex(),
        "live head must be the original draft — kind:9005 must not have altered it"
    );

    // Step 4: attempt rebind to ch_b with a strictly newer timestamp — must be
    // rejected because the ch_a binding is intact.
    let rebind = build_draft_at(
        &owner,
        &d,
        "9",
        &ch_b,
        &fake_nip44_v2(),
        Timestamp::from(base + 1),
    );
    let (rebind_accepted, rebind_msg) = submit_event_http(&client, &owner, &rebind).await;
    assert!(
        !rebind_accepted,
        "rebind to ch_b must be rejected; relay said: {rebind_msg}"
    );
    assert!(
        rebind_msg.contains("channel")
            || rebind_msg.contains("mismatch")
            || rebind_msg.contains("immutable")
            || rebind_msg.contains("invalid"),
        "rebind rejection must name the binding invariant; got: {rebind_msg}"
    );
}

#[tokio::test]
#[ignore]
/// A channel admin who does not own the draft must receive "target event not
/// found" when attempting kind:9005 deletion — indistinguishable from a
/// missing-target response (oracle-masking tripwire).
async fn test_nip09_kind9005_admin_deletion_of_draft_is_masked_as_not_found() {
    let client = http_client();
    let owner = Keys::generate();
    let admin = Keys::generate();

    // Create an open channel, add admin as a channel admin.
    let ch_id = create_open_channel(&owner).await;
    let add_admin_event = EventBuilder::new(Kind::Custom(KIND_PUT_USER), "")
        .tags([
            Tag::parse(["h", &ch_id]).unwrap(),
            Tag::parse(["p", &admin.public_key().to_hex()]).unwrap(),
            Tag::parse(["role", "admin"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let (ok_add, msg_add) = submit_event_http(&client, &owner, &add_admin_event).await;
    assert!(ok_add, "add admin must succeed: {msg_add}");

    // Publish a draft as the channel owner.
    let d = uuid::Uuid::new_v4().to_string();
    let draft = build_draft_at(&owner, &d, "9", &ch_id, &fake_nip44_v2(), Timestamp::now());
    let (ok_draft, msg_draft) = submit_event_http(&client, &owner, &draft).await;
    assert!(ok_draft, "draft must be accepted: {msg_draft}");

    // Admin (non-owner of draft) submits kind:9005 targeting the draft event id.
    // Must be rejected with "target event not found" — not a draft-specific error.
    let del_9005 = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["e", &draft.id.to_hex()]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&admin)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &admin, &del_9005).await;
    assert!(
        !accepted,
        "admin kind:9005 on draft must be rejected; relay said: {msg}"
    );

    // Control probe: same admin submits kind:9005 with a random nonexistent
    // event id. This produces the genuine "target event not found" path.
    // The oracle-masking invariant requires that `msg` (draft target) is
    // byte-identical to `control_msg` (nonexistent target) and contains no
    // draft-specific terms.
    let fake_id = "de".repeat(32);
    let control_9005 = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["e", &fake_id]).unwrap(),
            Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .sign_with_keys(&admin)
        .unwrap();
    let (control_accepted, control_msg) = submit_event_http(&client, &admin, &control_9005).await;
    assert!(
        !control_accepted,
        "control probe (nonexistent target) must also be rejected; relay said: {control_msg}"
    );
    assert_eq!(
        msg, control_msg,
        "draft-target and nonexistent-target must produce byte-identical rejection messages \
         (oracle mask); draft msg: {msg:?}, control msg: {control_msg:?}"
    );
    assert!(
        !msg.contains("31234") && !msg.contains("draft") && !msg.contains("tombstone"),
        "rejection must not leak draft-specific terms; got: {msg:?}"
    );
}

// ─── DM channel path ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_accepted_and_replaced_in_dm_channel() {
    // Draft wraps are channel-bound. A DM channel UUID (returned from
    // kind:41010) is a valid `h` target — the relay treats it identically
    // to a stream/broadcast channel for draft storage purposes.
    let client = http_client();
    let alice = Keys::generate();
    let bob = Keys::generate();

    // Alice opens a DM with Bob — relay creates and returns the channel UUID.
    let dm_event = EventBuilder::new(Kind::Custom(41010), "")
        .tags([Tag::parse(["p", &bob.public_key().to_hex()]).unwrap()])
        .sign_with_keys(&alice)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &alice.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&dm_event).unwrap())
        .send()
        .await
        .expect("open DM");
    let body: Value = resp.json().await.expect("parse DM response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "DM open must be accepted: {body}"
    );
    // The relay embeds the DM channel UUID in the message payload.
    let msg = body["message"].as_str().unwrap_or("");
    let dm_channel_id = if let Some(stripped) = msg.strip_prefix("response:") {
        let parsed: Value = serde_json::from_str(stripped).expect("response JSON");
        parsed["channel_id"]
            .as_str()
            .expect("channel_id in DM response")
            .to_string()
    } else {
        panic!(
            "DM open response must contain a `response:{{...}}` payload with channel_id; \
             got message: {msg:?} (full body: {body})"
        );
    };

    // Alice submits a draft bound to the DM channel UUID.
    // Timestamps are strictly increasing to guarantee deterministic ordering.
    let d = uuid::Uuid::new_v4().to_string();
    let base = nostr::Timestamp::now().as_secs();
    let v1 = build_draft_at(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(base - 2),
    );
    let (ok1, msg1) = submit_event_http(&client, &alice, &v1).await;
    assert!(ok1, "draft v1 to DM channel must be accepted: {msg1}");

    // Replace with a strictly newer version (base - 1 > base - 2).
    let v2 = build_draft_at(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(base - 1),
    );
    let v2_id = v2.id;
    let (ok2, msg2) = submit_event_http(&client, &alice, &v2).await;
    assert!(
        ok2,
        "draft v2 replacement in DM channel must be accepted: {msg2}"
    );

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &alice.public_key().to_hex(), vec![filter]).await;
    assert_eq!(
        results.len(),
        1,
        "DM-channel draft must have exactly one head"
    );
    assert_eq!(
        results[0]["id"].as_str().unwrap(),
        v2_id.to_hex(),
        "v2 must be the head after replacement"
    );

    // Tombstone the draft (base > base - 1, so this supersedes v2).
    let tomb = build_tombstone(
        &alice,
        &d,
        "9",
        &dm_channel_id,
        nostr::Timestamp::from(base),
    );
    let tomb_id = tomb.id;
    let (ok_t, msg_t) = submit_event_http(&client, &alice, &tomb).await;
    assert!(ok_t, "tombstone in DM channel must be accepted: {msg_t}");

    let filter2 = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results2 = query_events_http(&client, &alice.public_key().to_hex(), vec![filter2]).await;
    assert_eq!(results2.len(), 1, "tombstone must be the only head");
    assert_eq!(
        results2[0]["id"].as_str().unwrap(),
        tomb_id.to_hex(),
        "tombstone must be the current head after DM-channel draft is closed"
    );
}

#[tokio::test]
#[ignore]
async fn test_dm_draft_not_readable_by_dm_recipient() {
    // Regression guard (Q13): Alice opens a DM with Bob and writes a draft to
    // that DM channel.  Bob (the DM recipient and channel member) must NOT be
    // able to read Alice's draft — drafts are author-only regardless of
    // channel membership.  The test queries from Bob's perspective to prove
    // the author-only guard fires on the recipient-side, not just on arbitrary
    // third parties.
    let client = http_client();
    let alice = Keys::generate();
    let bob = Keys::generate();

    // Alice opens a DM with Bob — relay creates the DM channel UUID.
    let dm_event = EventBuilder::new(Kind::Custom(41010), "")
        .tags([Tag::parse(["p", &bob.public_key().to_hex()]).unwrap()])
        .sign_with_keys(&alice)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &alice.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&dm_event).unwrap())
        .send()
        .await
        .expect("open DM");
    let body: Value = resp.json().await.expect("parse DM response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "DM open must be accepted: {body}"
    );
    let msg = body["message"].as_str().unwrap_or("");
    let dm_channel_id = if let Some(stripped) = msg.strip_prefix("response:") {
        let parsed: Value = serde_json::from_str(stripped).expect("response JSON");
        parsed["channel_id"]
            .as_str()
            .expect("channel_id in DM response")
            .to_string()
    } else {
        panic!("DM open response must contain channel_id; got: {msg:?}");
    };

    // Alice writes a draft to the DM channel.
    let d = uuid::Uuid::new_v4().to_string();
    let draft = build_draft(&alice, &d, "9", &dm_channel_id, &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, err) = submit_event_http(&client, &alice, &draft).await;
    assert!(ok, "Alice's draft must be accepted: {err}");

    // Bob queries the DM channel as a member (recipient-side query).
    // He should NOT see Alice's draft — drafts are author-only.
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let resp2 = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &bob.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("Bob query");
    assert_eq!(
        resp2.status().as_u16(),
        403,
        "Bob (DM recipient) must get 403 querying Alice's draft; got: {}",
        resp2.status()
    );

    // Verify Alice herself can still read it (positive control).
    let filter2 = Filter::new()
        .kind(Kind::Custom(KIND_DRAFT))
        .author(alice.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let alice_results =
        query_events_http(&client, &alice.public_key().to_hex(), vec![filter2]).await;
    assert_eq!(
        alice_results.len(),
        1,
        "Alice must be able to read her own DM draft (positive control)"
    );
    assert_eq!(
        alice_results[0]["id"].as_str().unwrap(),
        draft_id.to_hex(),
        "Alice's query must return her draft"
    );
}

// ─── Removed-member read denial ──────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_removed_member_cannot_read_drafts_after_removal() {
    // A member who wrote a draft is later removed from the channel.
    // After removal they must not be able to REQ or COUNT their own draft
    // (channel membership is required for draft reads, not just writes).
    //
    // Note: draft reads are author-only, so this also tests that the
    // author-only gate stacks correctly with the channel-membership gate.
    let url = relay_url();
    let client = http_client();
    let owner = Keys::generate();
    let member = Keys::generate();
    let ch_id = create_private_channel(&owner).await;
    add_member_http(&client, &owner, &ch_id, &member).await;

    let d = uuid::Uuid::new_v4().to_string();
    let v1 = build_draft_at(
        &member,
        &d,
        "9",
        &ch_id,
        &fake_nip44_v2(),
        nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 2),
    );
    let (ok1, msg1) = submit_event_http(&client, &member, &v1).await;
    assert!(ok1, "member draft must be accepted: {msg1}");
    let v1_id = v1.id;

    // Remove member.
    remove_member_http(&client, &owner, &ch_id, &member).await;

    // Historical REQ: removed member must not retrieve their old draft.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(member.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results =
        query_events_http(&client, &member.public_key().to_hex(), vec![filter.clone()]).await;
    // The relay may return CLOSED or 0 results; the draft must not appear.
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&v1_id.to_hex())),
        "removed member must not retrieve their draft via HTTP /query"
    );

    // HTTP COUNT: removed member must get 0 count (not 403 here, since it's
    // their own draft address, but the channel-membership check should exclude it).
    let count_resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &member.public_key().to_hex())
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("count request");
    let count_status = count_resp.status().as_u16();
    if count_status == 200 {
        let body: Value = count_resp.json().await.expect("parse count");
        let count = body["count"].as_u64().unwrap_or(0);
        assert_eq!(
            count, 0,
            "removed member's draft count must be 0 after removal"
        );
    } else {
        // 403 is also acceptable — relay may gate based on membership entirely.
        assert!(
            count_status == 403,
            "expected 200 with count=0 or 403, got {count_status}"
        );
    }

    // Live fan-out: owner posts a new draft to the channel.  The removed
    // member must not receive it via a pre-existing WS subscription, even
    // if they filter on author(owner) — only current channel members may
    // receive owner's drafts.
    let mut removed_client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("connect removed member");
    let sid = sub_id("removed-fanout");
    // Subscribe to the owner's drafts — if channel membership is properly
    // enforced, the relay must CLOSE or simply not deliver owner's new draft
    // to the removed member.
    let live_filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key())
        .limit(0); // skip historical; only live
    let _ = removed_client.subscribe(&sid, vec![live_filter]).await;
    let _ = removed_client
        .collect_until_eose(&sid, Duration::from_secs(2))
        .await;

    // Owner submits a new draft — this is the live probe event.
    let owner_draft = build_draft(
        &owner,
        &uuid::Uuid::new_v4().to_string(),
        "9",
        &ch_id,
        &fake_nip44_v2(),
    );
    let owner_draft_id = owner_draft.id;
    let (ok_od, _) = submit_event_http(&client, &owner, &owner_draft).await;
    // Owner's own draft must be accepted; verify it doesn't reach removed member.
    if ok_od {
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while let Ok(RelayMessage::Event { event, .. }) =
                removed_client.recv_event(Duration::from_secs(1)).await
            {
                if event.id == owner_draft_id {
                    panic!("removed member received owner's draft via live fan-out after removal");
                }
            }
        })
        .await;
    }
    removed_client.disconnect().await.expect("disconnect");
}

// ─── Channel-window (top_level) draft privacy ────────────────────────────────

/// POST /query with `top_level: true`, mixed `kinds:[9,31234]`, as `as_keys`.
/// Returns the raw event array (rows + overlays + aux).
async fn query_channel_window_mixed(
    as_keys: &Keys,
    channel_id: &str,
    include_aux: bool,
    include_summaries: bool,
) -> Vec<Value> {
    let client = http_client();
    let filter = serde_json::json!({
        "kinds": [9, KIND_DRAFT],
        "#h": [channel_id],
        "top_level": true,
        "include_aux": include_aux,
        "include_summaries": include_summaries,
    });
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &as_keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&serde_json::json!([filter])).unwrap())
        .send()
        .await
        .expect("channel window mixed query");
    assert!(
        resp.status().is_success(),
        "window query failed: {}",
        resp.status()
    );
    resp.json::<Vec<Value>>()
        .await
        .expect("parse window response")
}

/// `top_level: true` channel-window path with mixed `kinds:[9,31234]`:
/// a non-author channel member must receive zero kind:31234 draft rows and
/// zero draft event-ids anywhere in the response (rows, aux, overlays).
///
/// Contract: the channel-window path applies the same author-only visibility
/// rule as every other read path — kind:31234 drafts are excluded for any
/// requester who is not their author. The SQL-level guard ensures that
/// `next_cursor` and `has_more` are also computed from the draft-excluded row
/// set, so no draft id can leak via the 39006 bounds overlay or aux closure.
#[tokio::test]
#[ignore]
async fn test_channel_window_draft_excluded_for_non_author() {
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&author).await;

    // Attacker joins the channel so they can issue a valid top_level query.
    add_member_http(&client, &author, &ch_id, &attacker).await;

    // Author posts a visible kind:9 message — provides a positive control row.
    let msg = EventBuilder::new(nostr::Kind::Custom(9), "hello channel")
        .tags([nostr::Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&author)
        .unwrap();
    let msg_id = msg.id.to_hex();
    let (ok_m, reason_m) = submit_event_http(&client, &author, &msg).await;
    assert!(ok_m, "kind:9 message must be accepted: {reason_m}");

    // Author posts a kind:31234 draft in the same channel.
    let d = uuid::Uuid::new_v4().to_string();
    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id.to_hex();
    let (ok_d, reason_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {reason_d}");

    // Attacker issues a top_level window query with kinds:[9,31234] — must
    // not receive the draft in rows, aux, or overlays.
    let events = query_channel_window_mixed(&attacker, &ch_id, true, true).await;

    // Collect all event-ids that appear anywhere in the response (own id +
    // any id referenced in tags — covers bounds `d` tag, summary `e`/`d` tags).
    let all_ids: Vec<String> = events
        .iter()
        .flat_map(|e| {
            let own_id = e["id"].as_str().map(|s| s.to_string());
            let tag_values: Vec<String> = e["tags"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .flat_map(|t| {
                    t.as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                })
                .collect();
            own_id.into_iter().chain(tag_values)
        })
        .collect();

    assert!(
        !all_ids.iter().any(|id| id == &draft_id),
        "draft id must not appear anywhere in the channel-window response for a non-author: \
        draft_id={draft_id}, response={events:?}"
    );

    // Positive control: the kind:9 message MUST be present as a row.
    let row_kinds: Vec<u64> = events.iter().filter_map(|e| e["kind"].as_u64()).collect();
    assert!(
        row_kinds.contains(&9),
        "kind:9 row must appear in window response for attacker: {events:?}"
    );
    // msg_id must specifically be in the rows.
    let ids_in_response: Vec<String> = events
        .iter()
        .filter_map(|e| e["id"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        ids_in_response.contains(&msg_id),
        "kind:9 message id must appear in window rows: msg_id={msg_id}, response={events:?}"
    );

    assert!(
        !row_kinds.contains(&(KIND_DRAFT as u64)),
        "no kind:31234 event may appear in window response for non-author: {events:?}"
    );

    // Exactly one 39006 bounds overlay must be present (window invariant).
    let bounds_count = events
        .iter()
        .filter(|e| e["kind"].as_u64() == Some(39006))
        .count();
    assert_eq!(
        bounds_count, 1,
        "exactly one 39006 bounds overlay required: {events:?}"
    );
}

/// `top_level: true` channel-window with kinds:[9,31234]: the author themselves
/// CAN see their own draft in the window — consistent with all other read paths
/// which allow authors to retrieve their own author-only events.
#[tokio::test]
#[ignore]
async fn test_channel_window_draft_visible_to_author() {
    let client = http_client();
    let author = Keys::generate();
    let ch_id = create_open_channel(&author).await;

    // Post a kind:9 message and a draft in the same channel.
    let msg = EventBuilder::new(nostr::Kind::Custom(9), "public message")
        .tags([nostr::Tag::parse(["h", &ch_id]).unwrap()])
        .sign_with_keys(&author)
        .unwrap();
    let (ok_m, reason_m) = submit_event_http(&client, &author, &msg).await;
    assert!(ok_m, "kind:9 message must be accepted: {reason_m}");

    let d = uuid::Uuid::new_v4().to_string();
    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id = draft.id.to_hex();
    let (ok_d, reason_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {reason_d}");

    // Author queries the window — their own draft must appear.
    let events = query_channel_window_mixed(&author, &ch_id, false, false).await;

    let row_kinds: Vec<u64> = events.iter().filter_map(|e| e["kind"].as_u64()).collect();

    // Both kind:9 and kind:31234 (own draft) must be present.
    assert!(
        row_kinds.contains(&9),
        "kind:9 row must appear for author: {events:?}"
    );
    assert!(
        row_kinds.contains(&(KIND_DRAFT as u64)),
        "author must see their own kind:31234 draft in the window: {events:?}"
    );

    // Verify the specific draft id is present.
    let ids_in_response: Vec<String> = events
        .iter()
        .filter_map(|e| e["id"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        ids_in_response.contains(&draft_id),
        "author's draft id must appear in window rows: draft_id={draft_id}, response={events:?}"
    );

    // 39006 bounds overlay invariant.
    let bounds_count = events
        .iter()
        .filter(|e| e["kind"].as_u64() == Some(39006))
        .count();
    assert_eq!(
        bounds_count, 1,
        "exactly one 39006 bounds overlay required: {events:?}"
    );
}

/// Q16: channel-window cursor-boundary variant.
///
/// Posts more messages than the `limit` so the window response includes a
/// `next_cursor`, then verifies:
/// 1. `next_cursor` ids never contain a draft id (draft-excluded row set).
/// 2. Paginating with the cursor returns only public messages and remains gapless.
/// 3. Drafts are excluded from both pages and from every id in the overlay.
///
/// This is the exact mutate-bite scenario the whole review run hunted:
/// a future regression that moves the author-only filter after pagination
/// would either leak a draft id in `next_cursor` or create a gap on page 2.
#[tokio::test]
#[ignore]
async fn test_channel_window_cursor_boundary_excludes_draft() {
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&author).await;
    add_member_http(&client, &author, &ch_id, &attacker).await;

    // Post 3 public kind:9 messages with strictly increasing timestamps.
    // The draft is placed at base+3 — inside the raw first page of limit=2
    // (raw DESC order: msg2@+4, draft@+3, msg1@+2, msg0@+0).  A regression
    // that filters drafts AFTER computing the cursor would pick draft as the
    // cursor candidate and leak it.  Correct SQL-layer exclusion computes
    // the cursor from the filtered set, giving msg1@+2 as the page-1 cursor.
    let base_ts = nostr::Timestamp::now().as_secs() - 10;
    let mut msg_ids = Vec::new();
    for i in 0..3u64 {
        let ts = nostr::Timestamp::from(base_ts + i * 2); // spread 0, 2, 4 seconds
        let msg = nostr::EventBuilder::new(nostr::Kind::Custom(9), format!("msg-{i}"))
            .tags([nostr::Tag::parse(["h", &ch_id]).unwrap()])
            .custom_created_at(ts)
            .sign_with_keys(&author)
            .unwrap();
        msg_ids.push(msg.id.to_hex());
        let (ok, err) = submit_event_http(&client, &author, &msg).await;
        assert!(ok, "kind:9 message {i} must be accepted: {err}");
    }

    // Draft at base+3: sits between msg2@+4 and msg1@+2, inside the raw
    // first page.  A filter-after-pagination regression would use it as
    // the cursor (leaking the draft id via 39006 next_cursor).
    let d = uuid::Uuid::new_v4().to_string();
    let draft_ts = nostr::Timestamp::from(base_ts + 3);
    let draft = nostr::EventBuilder::new(nostr::Kind::Custom(KIND_DRAFT), fake_nip44_v2())
        .tags([
            nostr::Tag::parse(["d", &d]).unwrap(),
            nostr::Tag::parse(["k", "9"]).unwrap(),
            nostr::Tag::parse(["h", &ch_id]).unwrap(),
        ])
        .custom_created_at(draft_ts)
        .sign_with_keys(&author)
        .unwrap();
    let draft_id = draft.id.to_hex();
    let (ok_d, err_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    // Page 1: limit=2 as the attacker.  Response must have has_more=true and a
    // next_cursor.  No draft id must appear anywhere (rows, overlay, aux).
    let page1_filter = serde_json::json!({
        "kinds": [9, KIND_DRAFT],
        "#h": [ch_id],
        "top_level": true,
        "limit": 2,
        "include_aux": true,
        "include_summaries": false,
    });
    let resp1 = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&serde_json::json!([page1_filter])).unwrap())
        .send()
        .await
        .expect("page 1 window query");
    assert!(resp1.status().is_success(), "page 1 must succeed");
    let page1_events: Vec<Value> = resp1.json().await.expect("page 1 parse");

    // Collect all ids present in page 1 (rows + overlays + aux).
    let page1_ids: Vec<String> = page1_events
        .iter()
        .flat_map(|e| {
            let mut ids = vec![];
            if let Some(id) = e["id"].as_str() {
                ids.push(id.to_string());
            }
            // 39006 overlay content may embed next_cursor.id — check content too.
            if e["kind"].as_u64() == Some(39006) {
                if let Some(content_str) = e["content"].as_str() {
                    if let Ok(content) = serde_json::from_str::<Value>(content_str) {
                        if let Some(cursor_id) = content
                            .get("next_cursor")
                            .and_then(|c| c.get("id"))
                            .and_then(|v| v.as_str())
                        {
                            ids.push(cursor_id.to_string());
                        }
                    }
                }
            }
            ids
        })
        .collect();

    assert!(
        !page1_ids.iter().any(|id| id == &draft_id),
        "draft id must not appear anywhere in page 1: draft_id={draft_id}, page1={page1_events:?}"
    );

    // Extract next_cursor from the 39006 overlay.
    let overlay1 = page1_events
        .iter()
        .find(|e| e["kind"].as_u64() == Some(39006));
    let cursor = overlay1.and_then(|o| {
        let content: Value = serde_json::from_str(o["content"].as_str()?).ok()?;
        let has_more = content["has_more"].as_bool().unwrap_or(false);
        if !has_more {
            return None;
        }
        let cursor_ts = content["next_cursor"]["created_at"].as_i64()?;
        let cursor_id = content["next_cursor"]["id"].as_str()?.to_string();
        Some((cursor_ts, cursor_id))
    });

    // With 3 public messages and limit=2, has_more MUST be true and a cursor
    // MUST be present — the test is structured to force pagination.  If the
    // relay returned has_more=false, the SQL-layer exclusion likely mis-counted.
    let (cursor_ts, cursor_id) = cursor.expect(
        "page 1 must have has_more=true and a next_cursor — 3 public messages + limit=2 \
         guarantees pagination; if this fails the draft may have been counted or the \
         cursor may have been computed from the unfiltered row set",
    );

    // The cursor must not be the draft id.
    assert_ne!(
        cursor_id, draft_id,
        "next_cursor.id must not be the draft id — cursor must be computed on the \
         draft-excluded row set"
    );

    let page2_filter = serde_json::json!({
        "kinds": [9, KIND_DRAFT],
        "#h": [ch_id],
        "top_level": true,
        "limit": 2,
        "until": cursor_ts,
        "before_id": cursor_id,
        "include_aux": true,
        "include_summaries": false,
    });
    let resp2 = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &attacker.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&serde_json::json!([page2_filter])).unwrap())
        .send()
        .await
        .expect("page 2 window query");
    assert!(resp2.status().is_success(), "page 2 must succeed");
    let page2_events: Vec<Value> = resp2.json().await.expect("page 2 parse");

    let page2_ids: Vec<String> = page2_events
        .iter()
        .filter_map(|e| e["id"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        !page2_ids.iter().any(|id| id == &draft_id),
        "draft id must not appear in page 2: draft_id={draft_id}, page2={page2_events:?}"
    );

    // Gapless invariant: the two pages together cover all 3 public messages.
    let all_msg_ids: std::collections::HashSet<String> = page1_ids
        .iter()
        .chain(page2_ids.iter())
        .filter(|id| msg_ids.contains(id))
        .cloned()
        .collect();
    assert_eq!(
        all_msg_ids.len(),
        3,
        "all 3 public messages must appear across both pages with no gaps; \
         got: page1={page1_ids:?}, page2={page2_ids:?}"
    );
}

// ─── Q15 oracle-closure e2e — write-path id-oracle guards ────────────────────
//
// These four tests prove the author-only target masking actually fires in the
// live relay, not just that the constant AUTHOR_ONLY_KINDS contains KIND_DRAFT.
// For each write path, we submit against a REAL draft id and against a RANDOM
// (guaranteed-nonexistent) 64-hex id, and assert:
//   - the error strings are BYTE-IDENTICAL between the two submissions (no oracle)
//   - after each rejected attempt, no public rows referencing the draft id exist
//
// The tests are intentionally verbose so a reviewer can follow every assertion.

#[tokio::test]
#[ignore]
async fn test_draft_target_reaction_oracle_closed() {
    // Attacker reacts (kind:7) to:
    //   (A) a real draft event id authored by `author`
    //   (B) a random 64-hex id that definitely does not exist
    //
    // Both must return the SAME byte-identical error string so an attacker
    // cannot distinguish "this id is a real draft" from "this id doesn't exist".
    // After each rejected submission, the attacker queries for kind:7 events
    // that e-tag the target id and asserts zero are stored.
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&author).await;
    let d = uuid::Uuid::new_v4().to_string();

    // Author publishes a draft — this is the real id we'll probe with.
    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id_hex = draft.id.to_hex();
    let (ok_d, err_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    // A guaranteed-nonexistent id: all zeros is an invalid SHA-256 preimage
    // for any real event on any relay.
    let random_id_hex = "0".repeat(64);

    // Helper: build a kind:7 reaction with e-tag pointing to `target_id`.
    // Reactions targeting non-channel events don't carry an h tag — the relay
    // derives the channel from the target.  We don't include one here to stay
    // minimal and match the ingest path under test.
    let build_reaction = |target_hex: &str| {
        EventBuilder::new(nostr::Kind::Custom(7), "+")
            .tags([nostr::Tag::parse(["e", target_hex]).unwrap()])
            .sign_with_keys(&attacker)
            .unwrap()
    };

    // (A) Attacker reacts to the real draft id.
    let reaction_a = build_reaction(&draft_id_hex);
    let (accepted_a, msg_a) = submit_event_http(&client, &attacker, &reaction_a).await;
    assert!(
        !accepted_a,
        "reaction to a real draft id must be rejected; relay said: {msg_a}"
    );

    // (B) Attacker reacts to the random (nonexistent) id.
    let reaction_b = build_reaction(&random_id_hex);
    let (accepted_b, msg_b) = submit_event_http(&client, &attacker, &reaction_b).await;
    assert!(
        !accepted_b,
        "reaction to a nonexistent id must be rejected; relay said: {msg_b}"
    );

    // Oracle-closure: error strings must be byte-identical.
    assert_eq!(
        msg_a, msg_b,
        "reaction rejection for real-draft-id vs random-id must be BYTE-IDENTICAL \
         to prevent an id-oracle attack; \
         real_draft='{msg_a}', random='{msg_b}'"
    );
    assert_eq!(
        msg_a, "invalid: reaction target event not found",
        "expected byte-exact masking error; got: '{msg_a}'"
    );

    // Post-rejection storage check: no kind:7 events e-tagging the draft id
    // must be stored — neither the attacker's attempt nor any fan-out.
    let kind7_filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(7))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
            draft_id_hex.as_str(),
        );
    // Query as attacker (their own events) and as author (would see any fan-out)
    let kind7_as_attacker = query_events_http(
        &client,
        &attacker.public_key().to_hex(),
        vec![kind7_filter.clone()],
    )
    .await;
    assert!(
        kind7_as_attacker.is_empty(),
        "zero kind:7 events referencing the draft id must be stored after attacker's \
         rejected reaction attempt; found: {kind7_as_attacker:?}"
    );
    let kind7_as_author =
        query_events_http(&client, &author.public_key().to_hex(), vec![kind7_filter]).await;
    assert!(
        kind7_as_author.is_empty(),
        "author-side query must also return zero kind:7 events referencing the draft id; \
         found: {kind7_as_author:?}"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_target_thread_parent_oracle_closed() {
    // Attacker posts a kind:9 reply (NIP-10 e-tag) whose parent is:
    //   (A) a real draft event id authored by `author`
    //   (B) a random 64-hex id that definitely does not exist
    //
    // Both must return the SAME byte-identical error string.  After each
    // rejected submission, we verify no kind:9 thread-children of the draft
    // id exist, and that no 39005 thread-summary event was fan-out fired.
    let url = relay_url();
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&author).await;

    // Attacker must be a channel member to get past membership check and reach
    // the thread-parent guard.  Add them to the open channel.
    add_member_http(&client, &author, &ch_id, &attacker).await;

    let d = uuid::Uuid::new_v4().to_string();
    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id_hex = draft.id.to_hex();
    let (ok_d, err_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    let random_id_hex = "1".repeat(64);

    // Subscribe as a channel watcher to detect any 39005 fan-out.
    let mut watcher = BuzzTestClient::connect(&url, &author)
        .await
        .expect("connect watcher");
    let sid_39005 = sub_id("thread-39005-watch");
    let filter_39005 = nostr::Filter::new()
        .kind(nostr::Kind::Custom(39005))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            ch_id.as_str(),
        )
        .limit(0);
    watcher
        .subscribe(&sid_39005, vec![filter_39005])
        .await
        .expect("subscribe 39005");
    let _ = watcher
        .collect_until_eose(&sid_39005, Duration::from_secs(3))
        .await;

    // Helper: build a kind:9 reply whose parent (and root) is `parent_hex`.
    // Using a single e-tag with marker "reply" makes it both root and parent
    // per the NIP-10 two-marker fallback in resolve_nip10_thread_meta.
    let build_reply = |parent_hex: &str| {
        EventBuilder::new(nostr::Kind::Custom(9), "reply text")
            .tags([
                nostr::Tag::parse(["h", &ch_id]).unwrap(),
                nostr::Tag::parse(["e", parent_hex, "", "reply"]).unwrap(),
            ])
            .sign_with_keys(&attacker)
            .unwrap()
    };

    // (A) Attacker replies with real draft id as parent.
    let reply_a = build_reply(&draft_id_hex);
    let (accepted_a, msg_a) = submit_event_http(&client, &attacker, &reply_a).await;
    assert!(
        !accepted_a,
        "reply with real draft id as parent must be rejected; relay said: {msg_a}"
    );

    // (B) Attacker replies with random (nonexistent) id as parent.
    let reply_b = build_reply(&random_id_hex);
    let (accepted_b, msg_b) = submit_event_http(&client, &attacker, &reply_b).await;
    assert!(
        !accepted_b,
        "reply with nonexistent id as parent must be rejected; relay said: {msg_b}"
    );

    // Oracle-closure: error strings must be byte-identical.
    assert_eq!(
        msg_a, msg_b,
        "thread-parent rejection for real-draft-id vs random-id must be BYTE-IDENTICAL; \
         real_draft='{msg_a}', random='{msg_b}'"
    );
    assert_eq!(
        msg_a, "invalid: reply parent not found",
        "expected byte-exact masking error; got: '{msg_a}'"
    );

    // Post-rejection storage check: no kind:9 events e-tagging the draft id
    // as a parent must be stored.
    let reply_filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(9))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
            draft_id_hex.as_str(),
        );
    let replies_stored =
        query_events_http(&client, &author.public_key().to_hex(), vec![reply_filter]).await;
    assert!(
        replies_stored.is_empty(),
        "zero kind:9 thread-children referencing the draft id must be stored; \
         found: {replies_stored:?}"
    );

    // No 39005 fan-out must have fired for this channel during the above
    // rejected attempts.  Give the relay a short window to deliver anything.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut received_39005 = false;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            break;
        }
        match watcher.recv_event(remaining).await {
            Ok(RelayMessage::Event { event, .. }) => {
                if event.kind == nostr::Kind::Custom(39005) {
                    received_39005 = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        !received_39005,
        "no 39005 thread-summary fan-out must fire when a reply is rejected due to \
         author-only parent masking"
    );
    watcher.disconnect().await.expect("disconnect watcher");
}

#[tokio::test]
#[ignore]
async fn test_draft_target_public_reference_author_also_rejected() {
    // The public-reference paths (reaction, thread-reply) reject drafts as
    // targets for EVERYONE — including the draft's own author.  This is because
    // a successful reaction/reply would write the draft id into a public row
    // (kind:7 or thread metadata), making the draft's existence public state.
    //
    // This test uses the author themselves as the submitter to prove the guard
    // is unconditional on public-reference paths.
    let client = http_client();
    let author = Keys::generate();
    let ch_id = create_open_channel(&author).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id_hex = draft.id.to_hex();
    let (ok_d, err_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    // Author reacts to their own draft.
    let self_reaction = EventBuilder::new(nostr::Kind::Custom(7), "+")
        .tags([nostr::Tag::parse(["e", &draft_id_hex]).unwrap()])
        .sign_with_keys(&author)
        .unwrap();
    let (accepted_rxn, msg_rxn) = submit_event_http(&client, &author, &self_reaction).await;
    assert!(
        !accepted_rxn,
        "author reacting to their own draft must be rejected (public-reference path \
         writes draft id into public kind:7 row); relay said: {msg_rxn}"
    );
    assert_eq!(
        msg_rxn, "invalid: reaction target event not found",
        "author's self-reaction rejection must use the masking not-found error; got: '{msg_rxn}'"
    );

    // Author replies with their own draft as parent.
    let self_reply = EventBuilder::new(nostr::Kind::Custom(9), "author reply")
        .tags([
            nostr::Tag::parse(["h", &ch_id]).unwrap(),
            nostr::Tag::parse(["e", &draft_id_hex, "", "reply"]).unwrap(),
        ])
        .sign_with_keys(&author)
        .unwrap();
    let (accepted_reply, msg_reply) = submit_event_http(&client, &author, &self_reply).await;
    assert!(
        !accepted_reply,
        "author replying to their own draft as parent must be rejected (public-reference \
         path writes draft id into thread metadata); relay said: {msg_reply}"
    );
    assert_eq!(
        msg_reply, "invalid: reply parent not found",
        "author's self-reply rejection must use the masking not-found error; got: '{msg_reply}'"
    );

    // Verify the draft is still readable by the author (the rejections must
    // not have altered it).
    let draft_filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(author.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let drafts =
        query_events_http(&client, &author.public_key().to_hex(), vec![draft_filter]).await;
    assert_eq!(
        drafts.len(),
        1,
        "draft must still be readable by author after rejected self-reaction and self-reply"
    );
    assert_eq!(
        drafts[0]["id"].as_str().unwrap(),
        draft_id_hex,
        "draft head must be unchanged"
    );
}

#[tokio::test]
#[ignore]
async fn test_draft_target_kind5_oracle_closed() {
    // Non-author submits a kind:5 (NIP-09 e-tag deletion) targeting:
    //   (A) a real draft event id authored by `author`
    //   (B) a random 64-hex id that definitely does not exist
    //
    // Both must return the SAME byte-identical error string so the attacker
    // cannot distinguish "this id is a real draft" from "this id doesn't exist".
    // After each rejected submission, the draft must still be readable by the
    // author (it was not deleted).
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();
    let ch_id = create_open_channel(&author).await;
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&author, &d, "9", &ch_id, &fake_nip44_v2());
    let draft_id_hex = draft.id.to_hex();
    let (ok_d, err_d) = submit_event_http(&client, &author, &draft).await;
    assert!(ok_d, "draft must be accepted: {err_d}");

    let random_id_hex = "2".repeat(64);

    let build_deletion = |target_hex: &str, signer: &Keys| {
        EventBuilder::new(nostr::Kind::EventDeletion, "")
            .tags([nostr::Tag::parse(["e", target_hex]).unwrap()])
            .sign_with_keys(signer)
            .unwrap()
    };

    // (A) Attacker submits kind:5 e-tagging the real draft id.
    let del_a = build_deletion(&draft_id_hex, &attacker);
    let (accepted_a, msg_a) = submit_event_http(&client, &attacker, &del_a).await;
    assert!(
        !accepted_a,
        "kind:5 targeting a real draft id must be rejected; relay said: {msg_a}"
    );

    // (B) Attacker submits kind:5 e-tagging the random (nonexistent) id.
    let del_b = build_deletion(&random_id_hex, &attacker);
    let (accepted_b, msg_b) = submit_event_http(&client, &attacker, &del_b).await;
    assert!(
        !accepted_b,
        "kind:5 targeting a nonexistent id must be rejected; relay said: {msg_b}"
    );

    // Oracle-closure: error strings must be byte-identical.
    assert_eq!(
        msg_a, msg_b,
        "kind:5 rejection for real-draft-id vs random-id must be BYTE-IDENTICAL; \
         real_draft='{msg_a}', random='{msg_b}'"
    );
    assert_eq!(
        msg_a, "invalid: target event not found",
        "expected byte-exact masking error; got: '{msg_a}'"
    );

    // Draft must still be readable by the author — not deleted.
    let draft_filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(author.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let drafts =
        query_events_http(&client, &author.public_key().to_hex(), vec![draft_filter]).await;
    assert_eq!(
        drafts.len(),
        1,
        "draft must still be readable by author after attacker's rejected kind:5 deletion"
    );
    assert_eq!(
        drafts[0]["id"].as_str().unwrap(),
        draft_id_hex,
        "draft head must be the original draft — attacker's kind:5 must not have altered it"
    );
}

#[tokio::test]
#[ignore]
async fn test_reminder_target_reaction_oracle_closed() {
    // Behavioral companion to test_draft_target_reaction_oracle_closed for
    // kind:30300 (NIP-ER reminders).  This proves the author-only mask in
    // derive_reaction_channel generalizes over ALL AUTHOR_ONLY_KINDS, not
    // just kind:31234 drafts.
    //
    // Attacker reacts (kind:7) to:
    //   (A) a real kind:30300 reminder event id authored by `author`
    //   (B) a random 64-hex id that definitely does not exist
    //
    // Both must return the SAME byte-identical error string.  After each
    // rejected submission, the attacker queries for kind:7 events that
    // e-tag the target id and asserts zero are stored.
    //
    // Reminders are global (no channel association), so the test does not
    // need a channel and the relay must fire the author-only guard before
    // any channel-derivation logic.
    let client = http_client();
    let author = Keys::generate();
    let attacker = Keys::generate();

    // Author publishes a kind:30300 reminder.  Minimal valid shape: d tag + alt tag.
    let d = uuid::Uuid::new_v4().to_string();
    let reminder =
        nostr::EventBuilder::new(nostr::Kind::Custom(30300), "nip44-ciphertext-placeholder")
            .tags([
                nostr::Tag::parse(["d", &d]).unwrap(),
                nostr::Tag::parse(["alt", "Encrypted reminder"]).unwrap(),
            ])
            .sign_with_keys(&author)
            .unwrap();
    let reminder_id_hex = reminder.id.to_hex();
    let (ok_r, err_r) = submit_event_http(&client, &author, &reminder).await;
    assert!(ok_r, "reminder must be accepted: {err_r}");

    let random_id_hex = "3".repeat(64);

    let build_reaction = |target_hex: &str| {
        nostr::EventBuilder::new(nostr::Kind::Custom(7), "+")
            .tags([nostr::Tag::parse(["e", target_hex]).unwrap()])
            .sign_with_keys(&attacker)
            .unwrap()
    };

    // (A) Attacker reacts to the real reminder id.
    let reaction_a = build_reaction(&reminder_id_hex);
    let (accepted_a, msg_a) = submit_event_http(&client, &attacker, &reaction_a).await;
    assert!(
        !accepted_a,
        "reaction to a real reminder id must be rejected; relay said: {msg_a}"
    );

    // (B) Attacker reacts to the random (nonexistent) id.
    let reaction_b = build_reaction(&random_id_hex);
    let (accepted_b, msg_b) = submit_event_http(&client, &attacker, &reaction_b).await;
    assert!(
        !accepted_b,
        "reaction to a nonexistent id must be rejected; relay said: {msg_b}"
    );

    // Oracle-closure: error strings must be byte-identical.
    assert_eq!(
        msg_a, msg_b,
        "reaction rejection for real-reminder-id vs random-id must be BYTE-IDENTICAL; \
         real_reminder='{msg_a}', random='{msg_b}'"
    );
    assert_eq!(
        msg_a, "invalid: reaction target event not found",
        "expected byte-exact masking error for reminder target; got: '{msg_a}'"
    );

    // Post-rejection storage check: no kind:7 events e-tagging the reminder id
    // must be stored.
    let kind7_filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(7))
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
            reminder_id_hex.as_str(),
        );
    let kind7_as_attacker = query_events_http(
        &client,
        &attacker.public_key().to_hex(),
        vec![kind7_filter.clone()],
    )
    .await;
    assert!(
        kind7_as_attacker.is_empty(),
        "zero kind:7 events referencing the reminder id must be stored after attacker's \
         rejected reaction attempt; found: {kind7_as_attacker:?}"
    );
    let kind7_as_author =
        query_events_http(&client, &author.public_key().to_hex(), vec![kind7_filter]).await;
    assert!(
        kind7_as_author.is_empty(),
        "author-side query must also return zero kind:7 events referencing the reminder id; \
         found: {kind7_as_author:?}"
    );
}

// ─── Read-time expiry suppression (short-lived expiration tag) ────────────────

#[tokio::test]
#[ignore]
async fn test_draft_expired_by_client_tag_suppressed_on_http_query() {
    // A draft with a short future `expiration` tag must be served immediately
    // after ingest, then suppressed on /query once the tag lapses. A fresh
    // draft (no expiration) must remain visible throughout.
    //
    // Bite check: if `draft_expired` were removed from `reader_can_receive_event`,
    // the expired draft would persist alongside the fresh one — the final `ids`
    // assertion would fail.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d_expiring = uuid::Uuid::new_v4().to_string();
    let d_fresh = uuid::Uuid::new_v4().to_string();

    let expiring = build_expiring_draft(&owner, &d_expiring, &ch_id, 10);
    let expiring_id = expiring.id;
    let (ok_e, msg_e) = submit_event_http(&client, &owner, &expiring).await;
    assert!(ok_e, "expiring draft must be accepted at ingest: {msg_e}");

    let fresh = build_draft(&owner, &d_fresh, "9", &ch_id, &fake_nip44_v2());
    let fresh_id = fresh.id;
    let (ok_f, msg_f) = submit_event_http(&client, &owner, &fresh).await;
    assert!(ok_f, "fresh draft must be accepted: {msg_f}");

    // Poll until the expiring draft drops from /query (timeout 20s).
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let events =
            query_events_http(&client, &owner.public_key().to_hex(), vec![filter.clone()]).await;
        let ids: Vec<String> = events
            .iter()
            .filter_map(|e| e["id"].as_str().map(String::from))
            .collect();
        if !ids.contains(&expiring_id.to_hex()) {
            // Suppressed — verify the fresh draft is still present.
            assert!(
                ids.contains(&fresh_id.to_hex()),
                "fresh draft must appear in self-authored /query; got: {ids:?}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for expiring draft to be suppressed on /query"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
#[ignore]
async fn test_draft_expired_not_counted_on_count_surface() {
    // COUNT of self-authored drafts must exclude expired drafts.
    //
    // This validates the COUNT fast-path bypass: `filter_can_match_draft` forces
    // the per-event fallback path (which runs `reader_can_receive_event` including
    // `draft_expired`) instead of the raw SQL `count_events()` that cannot see
    // per-event expiry. Without the bypass, `count_events()` would return 2 (both
    // drafts in storage) — the `count == 1` assertion proves the test bites.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d_expiring = uuid::Uuid::new_v4().to_string();
    let d_fresh = uuid::Uuid::new_v4().to_string();

    let expiring = build_expiring_draft(&owner, &d_expiring, &ch_id, 10);
    let (ok_e, msg_e) = submit_event_http(&client, &owner, &expiring).await;
    assert!(ok_e, "expiring draft must be accepted at ingest: {msg_e}");

    let fresh = build_draft(&owner, &d_fresh, "9", &ch_id, &fake_nip44_v2());
    let (ok_f, msg_f) = submit_event_http(&client, &owner, &fresh).await;
    assert!(ok_f, "fresh draft must be accepted: {msg_f}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(owner.public_key());

    // Poll until COUNT drops to 1 (timeout 20s).
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let resp = client
            .post(format!("{}/count", relay_http_url()))
            .header("X-Pubkey", &owner.public_key().to_hex())
            .header("Content-Type", "application/json")
            .json(&vec![filter.clone()])
            .send()
            .await
            .expect("count request");
        assert!(
            resp.status().is_success(),
            "author COUNT must succeed, got: {}",
            resp.status()
        );
        let body: Value = resp.json().await.expect("parse count response");
        let count = body["count"].as_u64().unwrap_or(u64::MAX);
        if count == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for COUNT to exclude expired draft; last count: {count}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
#[ignore]
async fn test_draft_expired_suppressed_on_channel_window() {
    // An expired draft must also be absent from the channel-window (`top_level:true`)
    // bridge path. This surface previously bypassed `reader_can_receive_event` —
    // this test proves the fix bites.
    //
    // Bite check: if the `reader_can_receive_event` gate in `handle_channel_window_filter`
    // were removed, the expired draft would appear in the window response and this
    // test would fail.
    let client = http_client();
    let owner = Keys::generate();
    let ch_id = create_open_channel(&owner).await;
    let d_expiring = uuid::Uuid::new_v4().to_string();
    let d_fresh = uuid::Uuid::new_v4().to_string();

    let expiring = build_expiring_draft(&owner, &d_expiring, &ch_id, 10);
    let expiring_id = expiring.id;
    let (ok_e, msg_e) = submit_event_http(&client, &owner, &expiring).await;
    assert!(ok_e, "expiring draft must be accepted at ingest: {msg_e}");

    let fresh = build_draft(&owner, &d_fresh, "9", &ch_id, &fake_nip44_v2());
    let fresh_id = fresh.id;
    let (ok_f, msg_f) = submit_event_http(&client, &owner, &fresh).await;
    assert!(ok_f, "fresh draft must be accepted: {msg_f}");

    // Poll channel-window until the expiring draft drops (timeout 20s).
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let events = query_channel_window_mixed(&owner, &ch_id, false, false).await;
        let ids: Vec<String> = events
            .iter()
            .filter_map(|e| e["id"].as_str().map(String::from))
            .collect();
        if !ids.contains(&expiring_id.to_hex()) {
            // Suppressed — verify the fresh draft is still present.
            assert!(
                ids.contains(&fresh_id.to_hex()),
                "fresh draft must appear in channel-window; got: {ids:?}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for expiring draft to be suppressed on channel-window"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
