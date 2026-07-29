//! End-to-end tests for kind:30175 persona events (NIP-AP).
//!
//! These tests verify the relay correctly handles persona events:
//! - Accepts valid persona events with proper d-tag slugs
//! - Enforces NIP-33 replacement semantics (same d-tag, newer timestamp wins)
//! - Rejects invalid d-tag values (empty, too long, invalid characters)
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test --test e2e_persona -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use reqwest::Client;
use serde_json::Value;

const PERSONA_KIND: u16 = 30175;

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

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Submit an event via the NIP-98 HTTP bridge (`POST /events`).
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

/// Query events via the NIP-98 HTTP bridge (`POST /query`).
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

/// Count events via the NIP-98 HTTP bridge (`POST /count`).
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

fn sub_id(name: &str) -> String {
    format!("e2e-persona-{name}-{}", uuid::Uuid::new_v4())
}

/// Create an open-visibility channel so a kind:9 event can be read by any relay member.
///
/// Uses `visibility=open` so the foreign reader does not need an explicit membership
/// entry — the relay allows any authenticated member to read open-channel events.
async fn create_test_channel(keys: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4();
    let channel_name = format!("persona-e2e-{channel_uuid}");
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &channel_name]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let (ok, msg) = submit_event_http(&http_client(), keys, &event).await;
    assert!(ok, "channel creation rejected: {msg}");
    channel_uuid.to_string()
}

/// Build a minimal persona event with the given d-tag and content.
fn persona_event(keys: &Keys, d_tag: &str, content: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(PERSONA_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a persona event with an explicit created_at timestamp.
fn persona_event_at(keys: &Keys, d_tag: &str, content: &str, created_at: u64) -> nostr::Event {
    EventBuilder::new(Kind::Custom(PERSONA_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn test_persona_publish_and_query() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("test-persona-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let content = serde_json::json!({
        "name": &d_tag,
        "display_name": "Test Persona",
        "description": "A test persona for E2E validation"
    })
    .to_string();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish persona event
    let event = persona_event(&keys, &d_tag, &content);
    let event_id = event.id;
    let ok = client
        .send_event(event.clone())
        .await
        .expect("send persona");
    assert!(ok.accepted, "relay rejected persona event: {}", ok.message);

    // Query the exact event this test published. Replacement-address filters
    // are exercised below and may legitimately replay a racing duplicate.
    let sid = sub_id("query");
    let filter = Filter::new().id(event_id);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");

    let ev = events
        .iter()
        .find(|candidate| candidate.id == event_id)
        .expect("published persona event was not returned");
    assert_eq!(ev.content, content);
    assert_eq!(ev.pubkey, keys.public_key());
    assert_eq!(ev.kind, Kind::Custom(PERSONA_KIND));

    client.disconnect().await.expect("disconnect");
}

/// NIP-AP revision: a prompt-less definition (display_name only) must ingest
/// through the REAL relay path and round-trip byte-for-byte — not just pass
/// the envelope validator helper.
#[tokio::test]
#[ignore]
async fn test_promptless_persona_ingests_and_round_trips() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("promptless-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let content = serde_json::json!({ "display_name": "Config Only" }).to_string();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let event = persona_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send promptless");
    assert!(
        ok.accepted,
        "relay rejected prompt-less persona: {}",
        ok.message
    );

    let sid = sub_id("promptless");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content, content, "byte-for-byte round-trip");
    client.disconnect().await.expect("disconnect");
}

/// NIP-AP revision: the reserved behavioral fields ingest through the real
/// relay path as opaque content and round-trip byte-for-byte.
#[tokio::test]
#[ignore]
async fn test_behavioral_fields_persona_ingests_and_round_trips() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("behavioral-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let content = serde_json::json!({
        "display_name": "Behavioral",
        "system_prompt": "p",
        "respond_to": "owner-only",
        "respond_to_allowlist": [],
        "parallelism": 2
    })
    .to_string();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let event = persona_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send behavioral");
    assert!(
        ok.accepted,
        "relay rejected behavioral-fields persona: {}",
        ok.message
    );

    let sid = sub_id("behavioral");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content, content, "byte-for-byte round-trip");
    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_nip33_replacement_newer_wins() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("replace-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish older version
    let now = Timestamp::now().as_secs();
    let old_content = r#"{"name":"old","display_name":"Old","description":"Old version"}"#;
    let old_event = persona_event_at(&keys, &d_tag, old_content, now - 100);
    let ok = client.send_event(old_event).await.expect("send old");
    assert!(ok.accepted, "relay rejected old event: {}", ok.message);

    // Publish newer version with same d-tag
    let new_content = r#"{"name":"new","display_name":"New","description":"New version"}"#;
    let new_event = persona_event_at(&keys, &d_tag, new_content, now);
    let ok = client.send_event(new_event).await.expect("send new");
    assert!(ok.accepted, "relay rejected new event: {}", ok.message);

    // Query — should return only the newer event
    let sid = sub_id("replace");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(events.len(), 1, "NIP-33: only newest event should remain");
    let ev = &events[0];
    assert_eq!(ev.content, new_content, "should be the newer version");

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_nip33_older_does_not_replace_newer() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("no-replace-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish newer version first
    let now = Timestamp::now().as_secs();
    let new_content = r#"{"name":"new","display_name":"New","description":"Newer"}"#;
    let new_event = persona_event_at(&keys, &d_tag, new_content, now);
    let ok = client.send_event(new_event).await.expect("send new");
    assert!(ok.accepted, "relay rejected new event: {}", ok.message);

    // Publish older version — relay should accept but not replace
    let old_content = r#"{"name":"old","display_name":"Old","description":"Older"}"#;
    let old_event = persona_event_at(&keys, &d_tag, old_content, now - 100);
    let _ok = client.send_event(old_event).await.expect("send old");
    // Note: relay may accept or reject the older event depending on implementation.
    // The key assertion is that querying returns the newer one.

    // Query — should still return the newer event
    let sid = sub_id("no-replace");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(events.len(), 1, "should have exactly one event");
    let ev = &events[0];
    assert_eq!(ev.content, new_content, "newer event should persist");

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_empty_d_tag() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = EventBuilder::new(
        Kind::Custom(PERSONA_KIND),
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    )
    .tags(vec![Tag::parse(["d", ""]).unwrap()])
    .sign_with_keys(&keys)
    .unwrap();

    let ok = client.send_event(event).await.expect("send");
    assert!(!ok.accepted, "relay should reject persona with empty d-tag");
    assert!(
        ok.message.contains("empty") || ok.message.contains("d") || ok.message.contains("tag"),
        "rejection message should mention d-tag issue, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_missing_d_tag() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // No d-tag at all
    let event = EventBuilder::new(
        Kind::Custom(PERSONA_KIND),
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    )
    .sign_with_keys(&keys)
    .unwrap();

    let ok = client.send_event(event).await.expect("send");
    assert!(!ok.accepted, "relay should reject persona without d-tag");

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_d_tag_too_long() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // 65 characters — exceeds the 64-char limit
    let long_slug = "a".repeat(65);
    let event = persona_event(
        &keys,
        &long_slug,
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    );
    let ok = client.send_event(event).await.expect("send");
    assert!(
        !ok.accepted,
        "relay should reject persona with d-tag > 64 chars"
    );
    assert!(
        ok.message.contains("long") || ok.message.contains("64"),
        "rejection should mention length, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_d_tag_uppercase() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = persona_event(
        &keys,
        "My-Persona",
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    );
    let ok = client.send_event(event).await.expect("send");
    assert!(
        !ok.accepted,
        "relay should reject persona with uppercase d-tag"
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_d_tag_special_chars() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = persona_event(
        &keys,
        "my.persona!",
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    );
    let ok = client.send_event(event).await.expect("send");
    assert!(
        !ok.accepted,
        "relay should reject persona with special chars in d-tag"
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_rejects_d_tag_starting_with_underscore() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Slug must start with [a-z0-9], not underscore
    let event = persona_event(
        &keys,
        "_invalid",
        r#"{"name":"x","display_name":"X","description":"X"}"#,
    );
    let ok = client.send_event(event).await.expect("send");
    assert!(
        !ok.accepted,
        "relay should reject persona with d-tag starting with underscore"
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_accepts_valid_slugs() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Various valid slug patterns
    let valid_slugs = [
        "a",
        "my-persona",
        "persona_v2",
        "0-starts-with-digit",
        "a-b-c-d-e",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // exactly 64 chars
    ];

    for slug in valid_slugs {
        let content = format!(
            r#"{{"name":"{}","display_name":"Test","description":"Valid slug test"}}"#,
            slug
        );
        let event = persona_event(&keys, slug, &content);
        let ok = client.send_event(event).await.expect("send");
        assert!(
            ok.accepted,
            "relay should accept valid slug '{}', got rejection: {}",
            slug, ok.message
        );
    }

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_persona_multiple_per_author() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Publish two different personas (different d-tags)
    let slug_a = format!("persona-a-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let slug_b = format!("persona-b-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let event_a = persona_event(
        &keys,
        &slug_a,
        r#"{"name":"a","display_name":"Persona A","description":"First"}"#,
    );
    let event_b = persona_event(
        &keys,
        &slug_b,
        r#"{"name":"b","display_name":"Persona B","description":"Second"}"#,
    );

    let ok_a = client.send_event(event_a).await.expect("send A");
    assert!(ok_a.accepted, "persona A rejected: {}", ok_a.message);

    let ok_b = client.send_event(event_b).await.expect("send B");
    assert!(ok_b.accepted, "persona B rejected: {}", ok_b.message);

    // Query all personas by this author
    let sid = sub_id("multi");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(keys.public_key());

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.len() >= 2,
        "expected at least 2 persona events, got {}",
        events.len()
    );

    client.disconnect().await.expect("disconnect");
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared-read-gate tests (author-only-unless-shared semantics for kind:30175)
// ─────────────────────────────────────────────────────────────────────────────

/// Build a persona event with an optional `["shared","true"]` tag.
fn persona_event_with_shared(keys: &Keys, d_tag: &str, shared: bool) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", d_tag]).unwrap()];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"test"}"#)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a persona event with an optional `["shared","true"]` tag and an
/// explicit `created_at` — needed for NIP-33 replacement tests where two
/// events could otherwise land in the same second and be ordered by event-id
/// tie-break rather than timestamp.
fn persona_event_with_shared_at(
    keys: &Keys,
    d_tag: &str,
    shared: bool,
    created_at: u64,
) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", d_tag]).unwrap()];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"test"}"#)
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

/// AC-1: Foreign reader receives ONLY shared heads; author receives all own heads.
///
/// Gate changed: `test_persona_publish_and_query` queries by id (author, always
/// allowed), so it is unaffected. The "all personas by author" variant in
/// `test_persona_multiple_per_author` uses `authors:[self]`, also unaffected.
/// This test is the cross-author assertion.
#[tokio::test]
#[ignore]
async fn test_persona_shared_read_gate_foreign_sees_only_shared() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_unshared = format!("priv-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let d_shared = format!("pub-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Author publishes one unshared and one shared persona.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ev_unshared = persona_event_with_shared(&author_keys, &d_unshared, false);
    let ev_shared = persona_event_with_shared(&author_keys, &d_shared, true);
    let shared_id = ev_shared.id;

    let ok = author.send_event(ev_unshared).await.expect("send unshared");
    assert!(ok.accepted, "unshared ingest rejected: {}", ok.message);
    let ok = author.send_event(ev_shared).await.expect("send shared");
    assert!(ok.accepted, "shared ingest rejected: {}", ok.message);

    // Foreign reader queries all personas by the author.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("fg-all");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // Foreign should see ONLY the shared event.
    assert!(
        !events.iter().any(|e| e.tags.iter().any(|t| t
            .as_slice()
            .get(1)
            .is_some_and(|v| v.as_str() == d_unshared))),
        "foreign should NOT see unshared persona"
    );
    assert!(
        events.iter().any(|e| e.id == shared_id),
        "foreign should see the shared persona"
    );

    // Author queries all own personas — must see both.
    let sid_author = sub_id("auth-all");
    let filter_self = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    author
        .subscribe(&sid_author, vec![filter_self])
        .await
        .expect("subscribe author");
    let author_events = author
        .collect_until_eose(&sid_author, Duration::from_secs(5))
        .await
        .expect("collect author");
    assert!(
        author_events.len() >= 2,
        "author should see both own personas, got {}",
        author_events.len()
    );

    author.disconnect().await.expect("disconnect author");
    foreign.disconnect().await.expect("disconnect foreign");
}

/// AC-2: `{ids:[unshared-foreign-30175-id]}` returns nothing to a foreign reader.
#[tokio::test]
#[ignore]
async fn test_persona_ids_lookup_unshared_returns_nothing_to_foreign() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = format!("priv-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let ev = persona_event_with_shared(&author_keys, &d_tag, false);
    let event_id = ev.id;

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ok = author.send_event(ev).await.expect("send");
    assert!(ok.accepted, "ingest rejected: {}", ok.message);
    author.disconnect().await.expect("disconnect");

    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("ids-unshared");
    let filter = Filter::new().id(event_id);
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.is_empty(),
        "ids-lookup of unshared persona must return nothing to foreign reader, got {:?}",
        events.iter().map(|e| e.id).collect::<Vec<_>>()
    );

    foreign.disconnect().await.expect("disconnect");
}

/// AC-3: COUNT over 30175 uses the fallback path; foreign unshared events are excluded.
///
/// The relay does not pre-block COUNT on persona kinds (unlike AUTHOR_ONLY_KINDS).
/// We verify the per-event fallback fires correctly by cross-checking: the foreign
/// reader's REQ count equals the shared-persona count, not total-persona count.
#[tokio::test]
#[ignore]
async fn test_persona_count_excludes_foreign_unshared() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_unshared = format!("priv-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let d_shared = format!("pub-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Publish one unshared and one shared persona.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ok = author
        .send_event(persona_event_with_shared(&author_keys, &d_unshared, false))
        .await
        .expect("send unshared");
    assert!(ok.accepted, "unshared rejected: {}", ok.message);
    let ok = author
        .send_event(persona_event_with_shared(&author_keys, &d_shared, true))
        .await
        .expect("send shared");
    assert!(ok.accepted, "shared rejected: {}", ok.message);
    author.disconnect().await.expect("disconnect author");

    // Foreign sends COUNT for {kinds:[30175], authors:[author]} — uses fallback path
    // and must exclude the unshared event.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("count-persona");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let count_msg = serde_json::json!(["COUNT", sid, filter]);
    foreign.send_raw(&count_msg).await.expect("send COUNT");

    // The relay returns ["COUNT", sub_id, {"count": N}].
    // buzz-ws-client parses this into RelayMessage::Count.
    let result = foreign.recv_event(Duration::from_secs(5)).await;
    let count: u64 = match result {
        Ok(RelayMessage::Count { count, .. }) => count,
        Ok(RelayMessage::Closed { message, .. }) => {
            panic!("COUNT closed unexpectedly: {message}");
        }
        Ok(other) => panic!("unexpected relay message for COUNT: {other:?}"),
        Err(e) => panic!("unexpected error for COUNT: {e}"),
    };

    assert_eq!(
        count, 1,
        "foreign COUNT should see only the 1 shared persona, got {count}"
    );

    foreign.disconnect().await.expect("disconnect foreign");
}

/// AC-4a: Unshared persona publish is NOT delivered to foreign live subscription.
/// AC-4b: Shared persona publish IS delivered to foreign live subscription.
/// AC-4c: NIP-33 replace shared→unshared makes subsequent foreign REQs return nothing,
///        and the live subscription for the foreign reader receives no event.
///
/// Uses explicit monotonic `created_at` timestamps so NIP-33 head ordering is
/// deterministic — same-second events would otherwise be ordered by event-id
/// tie-break, making it possible for the "wrong" head to win.
#[tokio::test]
#[ignore]
async fn test_persona_live_fanout_shared_gate() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = format!("gate-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Use strictly increasing timestamps relative to now: now-2 < now-1 < now.
    // This satisfies the relay's timestamp-skew check while keeping NIP-33
    // ordering deterministic (t0 < t1 < t2 so same d-tag replacements always
    // promote the highest timestamp regardless of event-id ordering).
    let now = nostr::Timestamp::now().as_secs();
    let t0: u64 = now.saturating_sub(2);
    let t1: u64 = now.saturating_sub(1);
    let t2: u64 = now;

    // Foreign subscribes to the test author's kind:30175 events BEFORE the author
    // publishes. Scoped to this author so concurrent persona tests publishing their
    // own 30175s don't trip the leak-panic (parallel suite interference).
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("fanout-gate");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    // Drain historical EOSE.
    let _ = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("drain eose");

    // Author publishes an UNSHARED persona (t0) — foreign must NOT receive it.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ev_unshared = persona_event_with_shared_at(&author_keys, &d_tag, false, t0);
    let unshared_id = ev_unshared.id;
    let ok = author.send_event(ev_unshared).await.expect("send unshared");
    assert!(ok.accepted, "unshared rejected: {}", ok.message);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // No event should arrive for foreign.
    let result = foreign.recv_event(Duration::from_millis(500)).await;
    match result {
        Err(buzz_test_client::TestClientError::Timeout) => {}
        Ok(RelayMessage::Event { event, .. }) if event.kind == Kind::Custom(PERSONA_KIND) => {
            panic!(
                "foreign MUST NOT receive unshared persona via live fan-out \
                 (got event id={} author={})",
                event.id, event.pubkey
            )
        }
        _ => {}
    }

    // Verify the unshared event IS the NIP-33 head for the author (self-query).
    let sid_head_check = sub_id("head-unshared");
    let filter_self = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    author
        .subscribe(&sid_head_check, vec![filter_self])
        .await
        .expect("subscribe head-check");
    let author_events = author
        .collect_until_eose(&sid_head_check, Duration::from_secs(5))
        .await
        .expect("collect head-check");
    assert!(
        author_events.iter().any(|e| e.id == unshared_id),
        "unshared event must be the NIP-33 head visible to author"
    );

    // Author republishes with ["shared","true"] at t1 — t1 > t0 so it wins.
    // Foreign MUST receive it via live fan-out.
    let ev_shared = persona_event_with_shared_at(&author_keys, &d_tag, true, t1);
    let shared_id = ev_shared.id;
    let ok = author.send_event(ev_shared).await.expect("send shared");
    assert!(ok.accepted, "shared rejected: {}", ok.message);

    tokio::time::sleep(Duration::from_millis(300)).await;

    let msg = foreign
        .recv_event(Duration::from_secs(3))
        .await
        .expect("recv shared event");
    match msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(event.id, shared_id, "received wrong event via fanout");
        }
        other => panic!("expected shared persona event via fanout, got: {other:?}"),
    }

    // Verify shared event is now the NIP-33 head (self-query sees shared_id).
    let sid_head_shared = sub_id("head-shared");
    let filter_self2 = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    author
        .subscribe(&sid_head_shared, vec![filter_self2])
        .await
        .expect("subscribe head-shared");
    let author_events2 = author
        .collect_until_eose(&sid_head_shared, Duration::from_secs(5))
        .await
        .expect("collect head-shared");
    assert!(
        author_events2.iter().any(|e| e.id == shared_id),
        "shared event must be the NIP-33 head visible to author"
    );

    // AC-4c: Author republishes WITHOUT shared tag at t2 — NIP-33 replaces the head.
    // The foreign live subscription must NOT receive any event for this publish
    // (unshared fan-out is blocked), and a subsequent foreign REQ must return nothing.
    let ev_unshared2 = persona_event_with_shared_at(&author_keys, &d_tag, false, t2);
    let unshared2_id = ev_unshared2.id;
    let ok = author
        .send_event(ev_unshared2)
        .await
        .expect("send unshared2");
    assert!(ok.accepted, "unshared2 rejected: {}", ok.message);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Foreign live subscription must NOT receive the unshared replacement.
    let live_result = foreign.recv_event(Duration::from_millis(500)).await;
    match live_result {
        Err(buzz_test_client::TestClientError::Timeout) => {}
        Ok(RelayMessage::Event { event, .. }) if event.kind == Kind::Custom(PERSONA_KIND) => {
            panic!(
                "foreign MUST NOT receive unshared replacement via live fan-out \
                 (got event id={} author={})",
                event.id, event.pubkey
            )
        }
        _ => {}
    }

    // Verify unshared2 is now the NIP-33 head for the author.
    let sid_head_unshared2 = sub_id("head-unshared2");
    let filter_self3 = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    author
        .subscribe(&sid_head_unshared2, vec![filter_self3])
        .await
        .expect("subscribe head-unshared2");
    let author_events3 = author
        .collect_until_eose(&sid_head_unshared2, Duration::from_secs(5))
        .await
        .expect("collect head-unshared2");
    assert!(
        author_events3.iter().any(|e| e.id == unshared2_id),
        "unshared2 event must be the NIP-33 head visible to author"
    );

    // Foreign REQ post-unshare must see nothing for this author's personas.
    let sid2 = sub_id("fanout-gate-post-unshare");
    let filter2 = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    foreign
        .subscribe(&sid2, vec![filter2])
        .await
        .expect("subscribe2");
    let events = foreign
        .collect_until_eose(&sid2, Duration::from_secs(5))
        .await
        .expect("collect post-unshare");
    assert!(
        events.is_empty(),
        "after removing shared tag, foreign REQ must return nothing, got {} events",
        events.len()
    );

    author.disconnect().await.expect("disconnect author");
    foreign.disconnect().await.expect("disconnect foreign");
}

/// AC-5: Ingest rejects ["shared","false"], ["shared","x"], and duplicate shared tags.
///       Accepts ["shared","true"] and tag-absent.
#[tokio::test]
#[ignore]
async fn test_persona_ingest_shared_tag_validation() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Accept: no shared tag
    let ev = persona_event_with_shared(
        &keys,
        &format!("no-shared-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        false,
    );
    let ok = client.send_event(ev).await.expect("send no-shared");
    assert!(
        ok.accepted,
        "no-shared-tag persona must be accepted: {}",
        ok.message
    );

    // Accept: ["shared","true"]
    let ev = persona_event_with_shared(
        &keys,
        &format!("shared-true-{}", &uuid::Uuid::new_v4().to_string()[..8]),
        true,
    );
    let ok = client.send_event(ev).await.expect("send shared-true");
    assert!(
        ok.accepted,
        "shared=true persona must be accepted: {}",
        ok.message
    );

    // Reject: ["shared","false"]
    let ev = EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"x"}"#)
        .tags(vec![
            Tag::parse([
                "d",
                &format!("shared-false-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ])
            .unwrap(),
            Tag::parse(["shared", "false"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let ok = client.send_event(ev).await.expect("send shared-false");
    assert!(!ok.accepted, "shared=false persona must be rejected");
    assert!(
        ok.message.contains("shared") || ok.message.contains("invalid"),
        "unexpected rejection message: {}",
        ok.message
    );

    // Reject: ["shared","x"] — any value other than "true" is malformed.
    let ev = EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"x"}"#)
        .tags(vec![
            Tag::parse([
                "d",
                &format!("shared-x-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ])
            .unwrap(),
            Tag::parse(["shared", "x"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let ok = client.send_event(ev).await.expect("send shared-x");
    assert!(
        !ok.accepted,
        "shared=x persona must be rejected: {}",
        ok.message
    );

    // Reject: ["shared"] — value is missing (tag has only 1 element after the key).
    let ev = EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"x"}"#)
        .tags(vec![
            Tag::parse([
                "d",
                &format!("shared-no-value-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ])
            .unwrap(),
            Tag::parse(["shared"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let ok = client.send_event(ev).await.expect("send shared-no-value");
    assert!(
        !ok.accepted,
        "shared tag with missing value must be rejected: {}",
        ok.message
    );

    // Reject: duplicate shared tags
    let ev = EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"x"}"#)
        .tags(vec![
            Tag::parse([
                "d",
                &format!("dup-shared-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ])
            .unwrap(),
            Tag::parse(["shared", "true"]).unwrap(),
            Tag::parse(["shared", "true"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let ok = client.send_event(ev).await.expect("send dup-shared");
    assert!(!ok.accepted, "duplicate shared tags must be rejected");

    client.disconnect().await.expect("disconnect");
}

/// AC-6: Mixed-kind filter {kinds:[30175, 9]} does not leak foreign unshared personas,
///        but DOES pass through kind:9 events from the same author. This pins the
///        correctness property in both directions — an implementation that silently
///        drops the whole mixed-kind filter would satisfy only the absence assertion.
#[tokio::test]
#[ignore]
async fn test_persona_mixed_kind_filter_does_not_leak() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = format!("mixed-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Create an open-visibility channel so the kind:9 control event is accessible
    // to the foreign reader without an explicit membership entry.
    let channel_id = create_test_channel(&author_keys).await;

    // Author publishes an unshared persona AND a kind:9 message in the open channel.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ev = persona_event_with_shared(&author_keys, &d_tag, false);
    let unshared_id = ev.id;
    let ok = author.send_event(ev).await.expect("send unshared");
    assert!(ok.accepted, "unshared persona rejected: {}", ok.message);

    // Publish a kind:9 event in the open channel so the mixed-kind filter has
    // something to return and we can assert the persona gate is per-event, not
    // a wholesale filter drop.
    let ev9 = EventBuilder::new(Kind::Custom(9), "hello from author")
        .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
        .sign_with_keys(&author_keys)
        .unwrap();
    let msg9_id = ev9.id;
    let ok9 = author.send_event(ev9).await.expect("send kind:9");
    assert!(ok9.accepted, "kind:9 rejected: {}", ok9.message);

    author.disconnect().await.expect("disconnect author");

    // Foreign queries with mixed-kind filter.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("mixed-kind");
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(PERSONA_KIND), Kind::Custom(9)])
        .author(author_keys.public_key());
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // Foreign must NOT see the unshared persona.
    assert!(
        !events.iter().any(|e| e.id == unshared_id),
        "mixed-kind filter must NOT leak foreign unshared persona (id {})",
        unshared_id
    );
    // Foreign MUST see the kind:9 event — filter is not wholesale dropped.
    assert!(
        events.iter().any(|e| e.id == msg9_id),
        "mixed-kind filter must pass through kind:9 events (id {})",
        msg9_id
    );

    foreign.disconnect().await.expect("disconnect foreign");
}

// ─── NIP-98 HTTP bridge persona gate tests ───────────────────────────────────
//
// These tests verify that `POST /query` and `POST /count` enforce the same
// author-only-unless-shared gate as the WebSocket paths.  A foreign
// authenticated caller must not receive or count unshared kind:30175 events
// belonging to another author, even via the HTTP bridge.

/// NIP-98 bridge AC-query: `/query` cross-author gate.
///
/// - A foreign pubkey posting `{kinds:[30175],authors:[victim]}` receives only
///   the shared head, not the unshared one.
/// - A kindless `{ids:[unshared-id]}` returns nothing.
/// - A `{ids:[shared-id]}` returns the shared event.
#[tokio::test]
#[ignore]
async fn test_persona_http_query_cross_author_gate() {
    let client = http_client();
    let author_keys = Keys::generate();
    let foreign_pubkey_hex = Keys::generate().public_key().to_hex();
    let author_pubkey_hex = author_keys.public_key().to_hex();

    let d_unshared = format!("priv-http-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let d_shared = format!("pub-http-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Publish via HTTP bridge (ingest path is the same).
    let ev_unshared = persona_event_with_shared(&author_keys, &d_unshared, false);
    let unshared_id_hex = ev_unshared.id.to_hex();
    let ev_shared = persona_event_with_shared(&author_keys, &d_shared, true);
    let shared_id_hex = ev_shared.id.to_hex();

    let (ok, msg) = submit_event_http(&client, &author_keys, &ev_unshared).await;
    assert!(ok, "unshared ingest rejected: {msg}");
    let (ok, msg) = submit_event_http(&client, &author_keys, &ev_shared).await;
    assert!(ok, "shared ingest rejected: {msg}");

    // Foreign queries all author's kind:30175 — only shared must come back.
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let results = query_events_http(&client, &foreign_pubkey_hex, vec![filter]).await;

    let has_unshared = results.iter().any(|e| {
        e.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == unshared_id_hex)
    });
    let has_shared = results.iter().any(|e| {
        e.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == shared_id_hex)
    });

    assert!(
        !has_unshared,
        "/query (authors:[victim]) must NOT return unshared persona to foreign"
    );
    assert!(
        has_shared,
        "/query (authors:[victim]) must return shared persona to foreign"
    );

    // Author self-query must see both.
    let filter_self = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let self_results = query_events_http(&client, &author_pubkey_hex, vec![filter_self]).await;
    assert!(
        self_results.iter().any(|e| e
            .get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == unshared_id_hex)),
        "author self-query must see own unshared persona"
    );
    assert!(
        self_results.iter().any(|e| e
            .get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == shared_id_hex)),
        "author self-query must see own shared persona"
    );

    // Kindless ids lookup for unshared event: foreign must get nothing.
    let unshared_event_id = nostr::EventId::from_hex(&unshared_id_hex).unwrap();
    let filter_ids = Filter::new().id(unshared_event_id);
    let id_results = query_events_http(&client, &foreign_pubkey_hex, vec![filter_ids]).await;
    assert!(
        id_results.is_empty(),
        "/query {{ids:[unshared-id]}} must return nothing to foreign, got {}",
        id_results.len()
    );

    // Kindless ids lookup for shared event: foreign must get it.
    let shared_event_id = nostr::EventId::from_hex(&shared_id_hex).unwrap();
    let filter_shared_ids = Filter::new().id(shared_event_id);
    let shared_id_results =
        query_events_http(&client, &foreign_pubkey_hex, vec![filter_shared_ids]).await;
    assert!(
        shared_id_results.iter().any(|e| e
            .get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == shared_id_hex)),
        "/query {{ids:[shared-id]}} must return shared event to foreign"
    );
}

/// NIP-98 bridge AC-count: `/count` cross-author gate.
///
/// A foreign authenticated caller counting `{kinds:[30175],authors:[victim]}`
/// must count only shared heads — not unshared ones — on both the fast SQL
/// path (prevented by `needs_persona_filtering`) and the fallback path.
#[tokio::test]
#[ignore]
async fn test_persona_http_count_cross_author_gate() {
    let client = http_client();
    let author_keys = Keys::generate();
    let foreign_pubkey_hex = Keys::generate().public_key().to_hex();
    let author_pubkey_hex = author_keys.public_key().to_hex();

    // Publish two unshared + one shared persona for the author.
    let d1 = format!("priv1-cnt-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let d2 = format!("priv2-cnt-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let d_shared = format!("pub-cnt-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    for (d, shared) in [(&d1, false), (&d2, false), (&d_shared, true)] {
        let ev = persona_event_with_shared(&author_keys, d, shared);
        let (ok, msg) = submit_event_http(&client, &author_keys, &ev).await;
        assert!(ok, "ingest rejected for {d}: {msg}");
    }

    // Foreign counts author's personas — must see only 1 (the shared one).
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let foreign_count = count_events_http(&client, &foreign_pubkey_hex, vec![filter])
        .await
        .expect("count should succeed");
    assert_eq!(
        foreign_count, 1,
        "foreign /count should return 1 (only shared persona), got {foreign_count}"
    );

    // Author self-count must see all 3.
    let filter_self = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let author_count = count_events_http(&client, &author_pubkey_hex, vec![filter_self])
        .await
        .expect("author count should succeed");
    assert_eq!(
        author_count, 3,
        "author /count should return 3, got {author_count}"
    );

    // Wildcard count (no authors filter) for the foreign reader — must not
    // include foreign unshared personas in the total.
    let filter_wildcard = Filter::new().kind(Kind::Custom(PERSONA_KIND));
    let wildcard_count = count_events_http(&client, &foreign_pubkey_hex, vec![filter_wildcard])
        .await
        .expect("wildcard count should succeed");
    // We can't assert the exact number (other tests may have published shared
    // personas), but we can assert it's ≥ 1 (the shared one) and verify the
    // unshared ones aren't counted by checking author-scoped again.
    assert!(
        wildcard_count >= 1,
        "wildcard count must include the shared persona"
    );
    // The foreign author-scoped count must still be exactly 1.
    let filter_scoped = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key());
    let scoped_count2 = count_events_http(&client, &foreign_pubkey_hex, vec![filter_scoped])
        .await
        .expect("scoped count2 should succeed");
    assert_eq!(
        scoped_count2, 1,
        "foreign author-scoped /count must still return 1 after wildcard query, got {scoped_count2}"
    );
}

/// Task-1 regression: WS REQ visibility-before-LIMIT gate.
///
/// Publishes N unshared (newer) personas followed by 1 shared (older) persona,
/// then queries with `limit < N`.  Without the SQL-level visibility clause, the
/// page is filled with unshared events and the shared one never appears.  With
/// the clause, unshared events are excluded before ORDER/LIMIT so the shared
/// event is returned.
///
/// Verifies at `312014d5e`: this test fails there because `query_events` did
/// not have the `persona_reader` SQL clause and the private rows starved the
/// shared one off the page.
#[tokio::test]
#[ignore]
async fn test_persona_ws_req_shared_visible_with_newer_private_ahead() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    // Publish N=3 private personas with the highest timestamps, then 1 shared
    // with the lowest timestamp.  limit=2 means the first page has only 2
    // candidates in the old (no-SQL-clause) world — both private — and the
    // shared one is invisible.
    let now = nostr::Timestamp::now().as_secs();
    let shared_ts = now.saturating_sub(10);

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");

    // Publish 3 private personas at t=now, now-1, now-2 (all newer than shared).
    for i in 0..3u64 {
        let d = format!(
            "priv-limit-{}-{}",
            i,
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let ev = persona_event_with_shared_at(&author_keys, &d, false, now.saturating_sub(i));
        let ok = author.send_event(ev).await.expect("send private");
        assert!(ok.accepted, "private persona {i} rejected: {}", ok.message);
    }

    // Publish 1 shared persona at a lower timestamp (older than all private ones).
    let shared_d = format!("shared-limit-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let ev_shared = persona_event_with_shared_at(&author_keys, &shared_d, true, shared_ts);
    let shared_id = ev_shared.id;
    let ok = author.send_event(ev_shared).await.expect("send shared");
    assert!(ok.accepted, "shared persona rejected: {}", ok.message);
    author.disconnect().await.expect("disconnect author");

    // Foreign queries with limit=2: private rows are excluded at SQL level,
    // so the shared event must appear despite having a lower timestamp.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("limit-gate");
    let filter = Filter::new()
        .kind(Kind::Custom(PERSONA_KIND))
        .author(author_keys.public_key())
        .limit(2);
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.iter().any(|e| e.id == shared_id),
        "shared persona must appear even when newer private personas fill the LIMIT (got {} events)",
        events.len()
    );
    assert!(
        events.iter().all(|e| {
            e.tags
                .iter()
                .any(|t| t.as_slice().first().is_some_and(|v| v.as_str() == "shared"))
        }),
        "foreign must NOT see any unshared persona"
    );

    foreign.disconnect().await.expect("disconnect foreign");
}

/// Task-1 regression: HTTP `/query` visibility-before-LIMIT gate.
///
/// Same scenario as the WS variant above, over the NIP-98 HTTP bridge.
#[tokio::test]
#[ignore]
async fn test_persona_http_query_shared_visible_with_newer_private_ahead() {
    let client = http_client();
    let author_keys = Keys::generate();
    let foreign_pubkey_hex = Keys::generate().public_key().to_hex();

    let now = nostr::Timestamp::now().as_secs();
    let shared_ts = now.saturating_sub(10);

    // Publish 3 private (newer) + 1 shared (older).
    for i in 0..3u64 {
        let d = format!(
            "http-priv-limit-{}-{}",
            i,
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let ev = persona_event_with_shared_at(&author_keys, &d, false, now.saturating_sub(i));
        let (ok, msg) = submit_event_http(&client, &author_keys, &ev).await;
        assert!(ok, "private persona {i} rejected: {msg}");
    }

    let shared_d = format!(
        "http-shared-limit-{}",
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let ev_shared = persona_event_with_shared_at(&author_keys, &shared_d, true, shared_ts);
    let shared_id_hex = ev_shared.id.to_hex();
    let (ok, msg) = submit_event_http(&client, &author_keys, &ev_shared).await;
    assert!(ok, "shared persona rejected: {msg}");

    // Foreign /query with limit=2: shared event must appear.
    let filter = serde_json::json!({
        "kinds": [PERSONA_KIND],
        "authors": [author_keys.public_key().to_hex()],
        "limit": 2
    });
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", &foreign_pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&vec![filter])
        .send()
        .await
        .expect("query");
    assert!(
        resp.status().is_success(),
        "query failed: {}",
        resp.status()
    );
    let results: Vec<serde_json::Value> = resp.json().await.expect("parse");

    let has_shared = results.iter().any(|e| {
        e.get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == shared_id_hex)
    });
    assert!(
        has_shared,
        "shared persona must appear in /query result even with newer private ones ahead (got {} events)",
        results.len()
    );
}

/// Task-2 wire-level: ingest must reject ["shared","true","extra"] over the wire.
///
/// The unit tests verify the validator directly; this test confirms the rejection
/// propagates end-to-end through the WebSocket ingest path.
#[tokio::test]
#[ignore]
async fn test_persona_ingest_rejects_three_element_shared_tag() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Build a persona event with a three-element ["shared","true","extra"] tag.
    // nostr::Tag::parse accepts variable-length slices, so this is straightforward.
    let ev = EventBuilder::new(Kind::Custom(PERSONA_KIND), r#"{"display_name":"x"}"#)
        .tags(vec![
            Tag::parse([
                "d",
                &format!("extra-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            ])
            .unwrap(),
            Tag::parse(["shared", "true", "extra"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    let ok = client
        .send_event(ev)
        .await
        .expect("send three-element shared tag");
    assert!(
        !ok.accepted,
        "three-element [\"shared\",\"true\",\"extra\"] tag must be rejected at ingest: {}",
        ok.message
    );
    assert!(
        ok.message.contains("[\"shared\",\"true\"]") || ok.message.contains("shared"),
        "rejection message should reference the shared tag constraint, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}
