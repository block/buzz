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
    // BuzzTestClient::recv_event maps WsClientError::UnexpectedMessage to
    // TestClientError::UnexpectedMessage (via the From impl in lib.rs).
    let result = foreign.recv_event(Duration::from_secs(5)).await;
    let count: u64 = match result {
        Err(buzz_test_client::TestClientError::UnexpectedMessage(raw)) => {
            // Parse ["COUNT", sub_id, {"count": N}]
            let arr: serde_json::Value = serde_json::from_str(&raw).expect("parse COUNT response");
            arr.get(2)
                .and_then(|o| o.get("count"))
                .and_then(|c| c.as_u64())
                .expect("count field in COUNT response")
        }
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
/// AC-4c: NIP-33 replace shared→unshared makes subsequent foreign REQs return nothing.
#[tokio::test]
#[ignore]
async fn test_persona_live_fanout_shared_gate() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = format!("gate-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Foreign subscribes to all kind:30175 events BEFORE the author publishes.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("fanout-gate");
    let filter = Filter::new().kind(Kind::Custom(PERSONA_KIND));
    foreign
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    // Drain historical EOSE.
    let _ = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("drain eose");

    // Author publishes an UNSHARED persona — foreign must NOT receive it.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ev_unshared = persona_event_with_shared(&author_keys, &d_tag, false);
    let ok = author.send_event(ev_unshared).await.expect("send unshared");
    assert!(ok.accepted, "unshared rejected: {}", ok.message);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // No event should arrive for foreign.
    let result = foreign.recv_event(Duration::from_millis(500)).await;
    match result {
        Err(buzz_test_client::TestClientError::Timeout) => {}
        Ok(RelayMessage::Event { event, .. }) if event.kind == Kind::Custom(PERSONA_KIND) => {
            panic!("foreign MUST NOT receive unshared persona via live fan-out")
        }
        _ => {}
    }

    // Author republishes with ["shared","true"] — foreign MUST now receive it.
    let ev_shared = persona_event_with_shared(&author_keys, &d_tag, true);
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

    // AC-4c: Author republishes WITHOUT shared tag — NIP-33 replaces the head.
    // A subsequent foreign REQ for this author's personas should return nothing.
    let ev_unshared2 = persona_event_with_shared(&author_keys, &d_tag, false);
    let ok = author
        .send_event(ev_unshared2)
        .await
        .expect("send unshared2");
    assert!(ok.accepted, "unshared2 rejected: {}", ok.message);

    tokio::time::sleep(Duration::from_millis(300)).await;

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

/// AC-6: Mixed-kind filter {kinds:[30175, 9]} does not leak foreign unshared personas.
///        The kind:9 events from other authors pass through normally.
#[tokio::test]
#[ignore]
async fn test_persona_mixed_kind_filter_does_not_leak() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = format!("mixed-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Author publishes an unshared persona.
    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ev = persona_event_with_shared(&author_keys, &d_tag, false);
    let unshared_id = ev.id;
    let ok = author.send_event(ev).await.expect("send unshared");
    assert!(ok.accepted, "unshared persona rejected: {}", ok.message);
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

    assert!(
        !events.iter().any(|e| e.id == unshared_id),
        "mixed-kind filter must NOT leak foreign unshared persona (id {})",
        unshared_id
    );

    foreign.disconnect().await.expect("disconnect foreign");
}
