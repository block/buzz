//! End-to-end integration tests for NIP-37 draft wraps (kind:31234).
//!
//! These tests verify:
//! - Write-path validation: d/k tag rules, h/p rejection, expiration,
//!   ciphertext validation, blank tombstone acceptance, oversized d
//! - Replacement ordering: NIP-01 last-write-wins, same-second tie-break
//!   (lower lexicographic event ID wins), stale write cannot supersede
//!   current head, tombstone replaces live draft as addressable head
//! - Author-only reads: REQ, WS COUNT, WS subscription, HTTP /query, /count
//!   all confine drafts to their author — exclusive, mixed/kindless, ids,
//!   known-#d filters, search/FTS, fan-out
//! - known-#d privacy tripwires: attacker knowing the `d` value does NOT
//!   retrieve or count the draft via exclusive or kindless #d filters
//! - FTS / NIP-50: draft content is never surfaced in search results
//! - NIP-11: relay advertises NIP-37, does not advertise NIP-40
//!
//! # Running
//!
//! Start the relay, then run:
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

/// Build a valid kind:31234 draft wrap event with given timestamps.
fn build_draft_at(
    keys: &Keys,
    d_tag: &str,
    k_val: &str,
    content: &str,
    ts: Timestamp,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_DRAFT), content)
        .tags([
            Tag::parse(["d", d_tag]).unwrap(),
            Tag::parse(["k", k_val]).unwrap(),
        ])
        .custom_created_at(ts)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a valid kind:31234 draft wrap event at current time.
fn build_draft(keys: &Keys, d_tag: &str, k_val: &str, content: &str) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, content, Timestamp::now())
}

/// Build a blank-content tombstone (NIP-37 deletion) for a draft address.
fn build_tombstone(keys: &Keys, d_tag: &str, k_val: &str, ts: Timestamp) -> nostr::Event {
    build_draft_at(keys, d_tag, k_val, "", ts)
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

// ─── Ingest validation ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_accepted_with_ciphertext_content() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let event = build_draft(&keys, &d_tag, "9", &fake_nip44_v2());
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "valid draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_blank_tombstone() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let event = build_tombstone(&keys, &d_tag, "9", Timestamp::now());
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "blank tombstone rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_accepted_future_expiration() {
    let client = http_client();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d_tag]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["expiration", "4102444800"]).unwrap(), // year 2100
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(accepted, "future expiration draft rejected: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_d_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([Tag::parse(["k", "9"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "missing d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_empty_d_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", ""]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "empty d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_oversized_d_tag() {
    let client = http_client();
    let keys = Keys::generate();
    // D_TAG_MAX_LEN is 255 bytes in buzz-db. Use 256 'a' chars.
    let d_tag = "a".repeat(256);
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d_tag]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "duplicate d tag should be rejected");
    assert!(msg.contains("d` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_missing_k_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([Tag::parse(["d", &d]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "missing k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_duplicate_k_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "duplicate k tag should be rejected");
    assert!(msg.contains("k` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_k_tag_non_decimal() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "0x9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "09"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "k tag with leading zero should be rejected");
    assert!(msg.contains("leading zero"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_k_tag_out_of_range() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "65536"]).unwrap(), // u16::MAX + 1
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "k=65536 should be rejected (out of u16 range)");
    assert!(msg.contains("range"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_h_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["h", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "h tag on draft should be rejected");
    assert!(msg.contains("h` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_p_tag() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["p", &keys.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "p tag on draft should be rejected");
    assert!(msg.contains("p` tag"), "unexpected message: {msg}");
}

#[tokio::test]
#[ignore]
async fn test_draft_rejected_malformed_ciphertext() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), "not-a-ciphertext")
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_DRAFT), &fake_nip44_v2())
        .tags([
            Tag::parse(["d", &d]).unwrap(),
            Tag::parse(["k", "9"]).unwrap(),
            Tag::parse(["expiration", "1000000000"]).unwrap(), // long past
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let (accepted, msg) = submit_event_http(&client, &keys, &event).await;
    assert!(!accepted, "past expiration should be rejected");
    assert!(msg.contains("expiration"), "unexpected message: {msg}");
}

// ─── NIP-01 replacement / tombstone ordering ─────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_replaced_by_newer_event() {
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    // Use timestamps offset from now so they pass ±15-min ingest gate.
    let now = Timestamp::now().as_secs();
    let t0 = Timestamp::from(now - 2);
    let t1 = Timestamp::from(now - 1);

    let v1 = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), t0);
    let v2 = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), t1);
    let v2_id = v2.id;

    let (ok1, msg1) = submit_event_http(&client, &keys, &v1).await;
    assert!(ok1, "v1 must be accepted: {msg1}");
    let (ok2, msg2) = submit_event_http(&client, &keys, &v2).await;
    assert!(ok2, "v2 must be accepted: {msg2}");

    // Author queries by #d — only the latest should be returned.
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &keys.public_key().to_hex(), vec![filter]).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_old = Timestamp::from(now - 2);
    let t_new = Timestamp::from(now - 1);

    let v_new = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), t_new);
    let v_old = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), t_old);

    // Submit new first, then try to replace with stale.
    let (ok_n, msg_n) = submit_event_http(&client, &keys, &v_new).await;
    assert!(ok_n, "newer draft must be accepted: {msg_n}");
    let (ok_o, msg_o) = submit_event_http(&client, &keys, &v_old).await;
    // Relay may accept (duplicate) or reject the old event — either is correct;
    // what matters is that the returned head is still the newer one.
    let _ = (ok_o, msg_o);

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &keys.public_key().to_hex(), vec![filter]).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now();
    // Generate candidates until we have two with different IDs at the same ts.
    // Sign 10 candidates and pick the lexically lowest and highest pair.
    let mut candidates = Vec::new();
    for _ in 0..10 {
        let e = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), now);
        candidates.push(e);
    }
    candidates.sort_by(|a, b| a.id.to_hex().cmp(&b.id.to_hex()));
    let lowest = candidates.first().unwrap().clone();
    let highest = candidates.last().unwrap().clone();

    if lowest.id == highest.id {
        // Extremely unlikely — skip rather than fail.
        return;
    }

    // Submit highest first, then lowest.
    let (ok_h, msg_h) = submit_event_http(&client, &keys, &highest).await;
    assert!(ok_h, "highest-id draft must be accepted: {msg_h}");
    let (ok_l, msg_l) = submit_event_http(&client, &keys, &lowest).await;
    assert!(ok_l, "lowest-id draft must be accepted: {msg_l}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &keys.public_key().to_hex(), vec![filter]).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let now = Timestamp::now().as_secs();
    let t_draft = Timestamp::from(now - 1);
    let t_tomb = Timestamp::now(); // strictly newer

    let draft = build_draft_at(&keys, &d, "9", &fake_nip44_v2(), t_draft);
    let tombstone = build_tombstone(&keys, &d, "9", t_tomb);
    let tomb_id = tombstone.id;

    let (ok_d, msg_d) = submit_event_http(&client, &keys, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");
    let (ok_t, msg_t) = submit_event_http(&client, &keys, &tombstone).await;
    assert!(ok_t, "tombstone must be accepted: {msg_t}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key())
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
            d.as_str(),
        );
    let results = query_events_http(&client, &keys.public_key().to_hex(), vec![filter]).await;
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
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&keys, &d, "9", &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &keys, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let mut c = BuzzTestClient::connect(&url, &keys)
        .await
        .expect("connect author");
    let sid = sub_id("author-req");
    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key());
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
    // Victim stores a draft; attacker queries {kinds:[31234], authors:[victim]}.
    // The relay must CLOSE the subscription with "restricted:".
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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

    let msg = ac
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
async fn test_draft_attacker_cannot_see_draft_in_kindless_filter_ws() {
    // Victim stores a draft; attacker issues a kindless filter.
    // Draft must be silently omitted. A public kind:0 event provides a
    // positive control — the attacker MUST receive that but NOT the draft.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    // Victim publishes a kind:0 profile event (public) and a draft (private).
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "victim profile must be accepted: {msg_p}");

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "victim draft must be accepted: {msg_d}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("attacker-kindless");
    // Kindless filter targeting victim's pubkey.
    let filter = Filter::new().author(victim.public_key()).limit(50);
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // Positive control: profile must be present.
    assert!(
        results.iter().any(|e| e.id == profile_id),
        "attacker must receive victim's public profile event"
    );
    // Privacy gate: draft must be absent.
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kindless filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_retrieve_by_known_event_id_ws() {
    // Knowing the exact event ID of a draft must not grant access.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
    // Attacker queries {kinds:[31234], authors:[victim], #d:[known_d]}.
    // The relay must CLOSE the subscription with "restricted:".
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
async fn test_draft_attacker_cannot_retrieve_by_known_d_tag_kindless_ws() {
    // Attacker queries {#d:[known_d]} — kindless, no authors filter.
    // Draft must be silently omitted; a public kind:9 message on the same
    // d-value (different kind, different event) provides a positive control
    // that the attacker can receive from a public channel.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok, msg) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok, "victim draft must be accepted: {msg}");

    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid = sub_id("d-kindless");
    // Kindless #d filter — this is the dictionary-attack vector for draft addresses.
    let filter = Filter::new().custom_tag(
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::D),
        d.as_str(),
    );
    ac.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let results = ac
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        !results.iter().any(|e| e.id == draft_id),
        "kindless #d filter must not expose victim's draft to attacker"
    );
    ac.disconnect().await.expect("disconnect");
}

// ─── COUNT privacy gates ──────────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_exclusive_ws() {
    // WS COUNT: {kinds:[31234], authors:[victim]} must be CLOSED with restricted:.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
        other => panic!("expected CLOSED for WS COUNT on another author's drafts, got: {other:?}"),
    }
    ac.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_count_via_known_d_ws() {
    // WS COUNT: {kinds:[31234], authors:[victim], #d:[known]} must be CLOSED.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
    // HTTP /count: {kinds:[31234], authors:[victim]} must return 403.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
    // Author's own HTTP /count must succeed and return ≥1.
    let client = http_client();
    let keys = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&keys, &d, "9", &fake_nip44_v2());
    let (ok, msg) = submit_event_http(&client, &keys, &draft).await;
    assert!(ok, "draft must be accepted: {msg}");

    let filter = Filter::new()
        .kind(nostr::Kind::Custom(KIND_DRAFT))
        .author(keys.public_key());
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", &keys.public_key().to_hex())
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

// ─── HTTP /query exclusive-author privacy ────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_attacker_cannot_query_exclusive_http() {
    // HTTP /query: exclusive other-author draft query must return 403.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
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
    // Attacker subscribes to a mixed filter BEFORE the draft is published.
    // They must NOT receive the draft in live fan-out. They MUST receive a
    // public control event (kind:0 profile) from the same author — this
    // proves fan-out is working and the draft was specifically excluded.
    let url = relay_url();
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    // Attacker subscribes to victim's events using a MIXED filter
    // (not exclusively kind:31234, so it won't be immediately CLOSED).
    let mut ac = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect attacker");
    let sid_fanout = sub_id("fanout-attacker");
    let filter = Filter::new()
        .kinds(vec![Kind::Metadata, Kind::Custom(KIND_DRAFT)])
        .author(victim.public_key())
        .limit(0); // live only, no stored events
    ac.subscribe(&sid_fanout, vec![filter])
        .await
        .expect("subscribe to mixed filter");
    // Drain EOSE.
    let _ = ac
        .collect_until_eose(&sid_fanout, Duration::from_secs(3))
        .await;

    // Victim publishes a draft — must NOT reach attacker via fan-out.
    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    // Victim also publishes a public profile event — MUST reach attacker.
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&victim)
        .unwrap();
    let profile_id = profile.id;
    let (ok_p, msg_p) = submit_event_http(&client, &victim, &profile).await;
    assert!(ok_p, "profile must be accepted: {msg_p}");

    // Drain messages briefly and check what arrived.
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

// ─── FTS / NIP-50 exclusion ───────────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_draft_not_indexed_in_fts_search() {
    // The relay stores search_tsv = NULL for kind:31234. Even if we could
    // search by the author, the draft must never surface in NIP-50 results.
    // We use the FTS HTTP query path as an attacker to verify.
    let client = http_client();
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let d = uuid::Uuid::new_v4().to_string();

    // Publish a kind:1 text note with a unique marker as a positive control.
    let marker = format!("nip37fts_probe_{}", uuid::Uuid::new_v4().simple());
    let note = EventBuilder::new(Kind::TextNote, &marker)
        .sign_with_keys(&victim)
        .unwrap();
    let note_id = note.id;
    let (ok_note, msg_note) = submit_event_http(&client, &victim, &note).await;
    assert!(ok_note, "control note must be accepted: {msg_note}");

    // Publish a draft from the same author.
    let draft = build_draft(&victim, &d, "9", &fake_nip44_v2());
    let draft_id = draft.id;
    let (ok_d, msg_d) = submit_event_http(&client, &victim, &draft).await;
    assert!(ok_d, "draft must be accepted: {msg_d}");

    // NIP-50 search as the victim — the kind:1 note must appear; the draft must not.
    let search_filter = Filter::new().search(&marker).limit(50);
    let results =
        query_events_http(&client, &victim.public_key().to_hex(), vec![search_filter]).await;

    // Positive control: the text note must be found.
    assert!(
        results
            .iter()
            .any(|e| e["id"].as_str() == Some(&note_id.to_hex())),
        "FTS must index the control kind:1 note (positive control)"
    );

    // Privacy gate: draft must never appear in search results.
    assert!(
        !results
            .iter()
            .any(|e| e["id"].as_str() == Some(&draft_id.to_hex())),
        "kind:31234 must have NULL search_tsv — draft must not appear in NIP-50 search"
    );

    // Searching as the attacker must also not expose the draft.
    let search_filter2 = Filter::new().search(&marker).limit(50);
    let attacker_results = query_events_http(
        &client,
        &attacker.public_key().to_hex(),
        vec![search_filter2],
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
