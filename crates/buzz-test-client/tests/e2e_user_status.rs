//! End-to-end tests for NIP-38 user statuses (kind:30315).
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
//! cargo test --test e2e_user_status -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_user_status -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const KIND_USER_STATUS: u16 = 30315;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Build a kind:30315 event with a d-tag and content.
fn build_user_status_event(
    keys: &Keys,
    d_tag: &str,
    content: &str,
    extra_tags: Vec<Tag>,
) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", d_tag]).unwrap()];
    tags.extend(extra_tags);
    EventBuilder::new(Kind::Custom(KIND_USER_STATUS), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

/// kind:30315 events are accepted by the relay.
#[tokio::test]
#[ignore]
async fn test_user_status_accepted() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = build_user_status_event(&keys, "general", "Working on NIP-38 support", vec![]);

    let ok = client.send_event(event).await.expect("send event");
    assert!(
        ok.accepted,
        "relay should accept kind:30315: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// kind:30315 events are retrievable via REQ with kinds filter.
#[tokio::test]
#[ignore]
async fn test_user_status_retrievable() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let d_tag = format!("retrieve-{}", uuid::Uuid::new_v4().simple());
    let event = build_user_status_event(&keys, &d_tag, "Currently online", vec![]);
    let event_id = event.id;

    let ok = client.send_event(event).await.expect("send event");
    assert!(ok.accepted, "relay should accept: {}", ok.message);

    // Query back by kind + author
    let sid = sub_id("retrieve");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_USER_STATUS))
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
        events.iter().any(|e| e.id == event_id),
        "should find the published user status in query results"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-33 replacement: publishing a newer kind:30315 with the same d-tag replaces the old one.
#[tokio::test]
#[ignore]
async fn test_user_status_nip33_replacement() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let d_tag = format!("replace-{}", uuid::Uuid::new_v4().simple());

    // Publish v1
    let v1 = build_user_status_event(&keys, &d_tag, "Status v1", vec![]);
    let ok1 = client.send_event(v1).await.expect("send v1");
    assert!(ok1.accepted, "v1 should be accepted: {}", ok1.message);

    // Small delay to ensure different created_at timestamps
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Publish v2 with the same d-tag
    let v2 = build_user_status_event(&keys, &d_tag, "Status v2 — updated", vec![]);
    let v2_id = v2.id;
    let ok2 = client.send_event(v2).await.expect("send v2");
    assert!(ok2.accepted, "v2 should be accepted: {}", ok2.message);

    // Query — should only get v2 (v1 replaced)
    let sid = sub_id("replace");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_USER_STATUS))
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

    assert_eq!(
        events.len(),
        1,
        "should have exactly one event after replacement"
    );
    assert_eq!(events[0].id, v2_id, "surviving event should be v2");
    assert!(events[0].content.contains("v2"), "content should be v2");

    client.disconnect().await.expect("disconnect");
}

/// NIP-38: multiple d-tags coexist — "general" and "music" are independent status slots.
#[tokio::test]
#[ignore]
async fn test_user_status_multiple_d_tags_coexist() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let general_d = format!("general-{}", uuid::Uuid::new_v4().simple());
    let music_d = format!("music-{}", uuid::Uuid::new_v4().simple());

    // Publish general status
    let general = build_user_status_event(&keys, &general_d, "Working on code", vec![]);
    let general_id = general.id;
    let ok1 = client.send_event(general).await.expect("send general");
    assert!(ok1.accepted, "general should be accepted: {}", ok1.message);

    // Publish music status
    let music = build_user_status_event(&keys, &music_d, "Listening to jazz", vec![]);
    let music_id = music.id;
    let ok2 = client.send_event(music).await.expect("send music");
    assert!(ok2.accepted, "music should be accepted: {}", ok2.message);

    // Query by kind + author — both should be returned
    let sid = sub_id("multi-dtag");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_USER_STATUS))
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
        events.iter().any(|e| e.id == general_id),
        "general status should be present"
    );
    assert!(
        events.iter().any(|e| e.id == music_id),
        "music status should be present"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-33 stale-write protection: an older event cannot replace a newer one.
#[tokio::test]
#[ignore]
async fn test_user_status_stale_write_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let d_tag = format!("stale-{}", uuid::Uuid::new_v4().simple());

    // Publish the "newer" event first (with a future-ish timestamp)
    let newer = {
        let tags = vec![Tag::parse(["d", &d_tag]).unwrap()];
        EventBuilder::new(Kind::Custom(KIND_USER_STATUS), "Newer status")
            .tags(tags)
            .custom_created_at(Timestamp::from(nostr::Timestamp::now().as_secs() + 100))
            .sign_with_keys(&keys)
            .unwrap()
    };
    let newer_id = newer.id;
    let ok1 = client.send_event(newer).await.expect("send newer");
    assert!(ok1.accepted, "newer should be accepted: {}", ok1.message);

    // Now try to publish an "older" event with the same d-tag but earlier timestamp
    let older = {
        let tags = vec![Tag::parse(["d", &d_tag]).unwrap()];
        EventBuilder::new(Kind::Custom(KIND_USER_STATUS), "Older status")
            .tags(tags)
            .custom_created_at(Timestamp::from(nostr::Timestamp::now().as_secs() - 100))
            .sign_with_keys(&keys)
            .unwrap()
    };
    let _ok2 = client.send_event(older).await.expect("send older");
    // Stale write may be rejected or accepted-as-duplicate — either way,
    // the older event must NOT replace the newer one.

    // Query — should still have the newer event
    let sid = sub_id("stale");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_USER_STATUS))
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
    assert_eq!(
        events[0].id, newer_id,
        "surviving event should be the newer one"
    );
    assert!(
        events[0].content.contains("Newer"),
        "content should be from the newer event"
    );

    client.disconnect().await.expect("disconnect");
}
