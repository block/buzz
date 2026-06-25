//! End-to-end tests for kind:30176 team events (NIP-AP).
//!
//! These tests verify the relay accepts and addresses team events the same way
//! it does personas:
//! - Accepts a valid team event and queries it back by NIP-33 coordinate.
//! - Enforces NIP-33 replacement semantics (same d-tag, newer timestamp wins).
//! - Honors a NIP-09 a-tag tombstone, removing the team coordinate.
//!
//! The team `d`-tag is the team's stable id (a slug-like string), not a 64-hex
//! pubkey — it exercises the generic parameterized-replaceable path that
//! enforces only `D_TAG_MAX_LEN`, distinct from the persona slug envelope.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test --test e2e_team -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const TEAM_KIND: u16 = 30176;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-team-{name}-{}", uuid::Uuid::new_v4())
}

/// Build a team event whose `d`-tag is the team id, mirroring the desktop
/// `build_team_event` shape.
fn team_event(keys: &Keys, d_tag: &str, content: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(TEAM_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

fn team_event_at(keys: &Keys, d_tag: &str, content: &str, created_at: u64) -> nostr::Event {
    EventBuilder::new(Kind::Custom(TEAM_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a NIP-09 a-tag-only deletion at the team's NIP-33 coordinate,
/// mirroring the desktop `build_team_delete` shape (no `e`-tag, so the relay
/// takes the coordinate-delete path rather than the event-id path).
fn team_delete_event(keys: &Keys, d_tag: &str) -> nostr::Event {
    let coord = format!("{TEAM_KIND}:{}:{d_tag}", keys.public_key().to_hex());
    EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![Tag::parse(["a", coord.as_str()]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
#[ignore]
async fn test_team_publish_and_query() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("team-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let content = serde_json::json!({
        "name": "Test Team",
        "description": "A test team for E2E validation",
        "persona_ids": ["p1", "p2"]
    })
    .to_string();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = team_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send team");
    assert!(ok.accepted, "relay rejected team event: {}", ok.message);

    let sid = sub_id("query");
    let filter = Filter::new()
        .kind(Kind::Custom(TEAM_KIND))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");

    assert_eq!(events.len(), 1, "expected exactly one team event");
    let ev = &events[0];
    assert_eq!(ev.content, content);
    assert_eq!(ev.pubkey, keys.public_key());
    assert_eq!(ev.kind, Kind::Custom(TEAM_KIND));

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_team_nip33_replacement_newer_wins() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("team-replace-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let now = Timestamp::now().as_secs();
    let old_content = r#"{"name":"Old Team","persona_ids":["p1"]}"#;
    let old_event = team_event_at(&keys, &d_tag, old_content, now - 100);
    let ok = client.send_event(old_event).await.expect("send old");
    assert!(ok.accepted, "relay rejected old event: {}", ok.message);

    let new_content = r#"{"name":"New Team","persona_ids":["p1","p2"]}"#;
    let new_event = team_event_at(&keys, &d_tag, new_content, now);
    let ok = client.send_event(new_event).await.expect("send new");
    assert!(ok.accepted, "relay rejected new event: {}", ok.message);

    let sid = sub_id("replace");
    let filter = Filter::new()
        .kind(Kind::Custom(TEAM_KIND))
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
    assert_eq!(
        events[0].content, new_content,
        "should be the newer version"
    );

    client.disconnect().await.expect("disconnect");
}

/// The a-tag tombstone is the only state-destroying op in the team flow: it
/// removes the team for every client and across reboots. This proves the relay
/// acts on it — publish a team, confirm it is live, publish the a-tag-only
/// tombstone at its coordinate, then assert the query returns it gone.
#[tokio::test]
#[ignore]
async fn test_team_tombstone_deletes_coordinate() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("team-tombstone-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let content = r#"{"name":"Doomed Team","persona_ids":["p1"]}"#;

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = team_event(&keys, &d_tag, content);
    let ok = client.send_event(event).await.expect("send team");
    assert!(ok.accepted, "relay rejected team event: {}", ok.message);

    let filter = || {
        Filter::new()
            .kind(Kind::Custom(TEAM_KIND))
            .author(keys.public_key())
            .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()])
    };

    let sid = sub_id("tombstone-pre");
    client
        .subscribe(&sid, vec![filter()])
        .await
        .expect("subscribe pre");
    let before = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect pre");
    assert_eq!(before.len(), 1, "team should be live before deletion");

    let tombstone = team_delete_event(&keys, &d_tag);
    let ok = client.send_event(tombstone).await.expect("send tombstone");
    assert!(ok.accepted, "relay rejected tombstone: {}", ok.message);

    let sid = sub_id("tombstone-post");
    client
        .subscribe(&sid, vec![filter()])
        .await
        .expect("subscribe post");
    let after = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect post");
    assert_eq!(
        after.len(),
        0,
        "tombstone should remove the team coordinate, got {} event(s)",
        after.len()
    );

    client.disconnect().await.expect("disconnect");
}
