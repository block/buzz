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

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const PERSONA_KIND: u16 = 30175;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-persona-{name}-{}", uuid::Uuid::new_v4())
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
