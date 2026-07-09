//! End-to-end tests for kind:30177 managed-agent events (NIP-AP).
//!
//! These tests verify the relay accepts and addresses managed-agent events the
//! same way it does personas, and that content published as a projection-shaped
//! body round-trips through the relay unchanged.
//!
//! - Accepts a valid managed-agent event and queries it back by NIP-33
//!   coordinate (the `d`-tag is the agent's 64-hex pubkey).
//! - Round-trips the published content through the relay, confirming it returns
//!   exactly the projected fields and nothing more. The body is a hand-built
//!   secret-free literal; the secret-exclusion regression guard lives in the
//!   `agent_events.rs` unit test, not here (the e2e crate can't reach the
//!   desktop projection function).
//! - Enforces NIP-33 replacement semantics (same d-tag, newer timestamp wins).
//! - Honors a NIP-09 a-tag tombstone, removing the agent coordinate.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test --test e2e_managed_agent -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const AGENT_KIND: u16 = 30177;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-managed-agent-{name}-{}", uuid::Uuid::new_v4())
}

/// The opt-IN allowlist projection a real `ManagedAgentRecord` produces (see
/// `desktop/src-tauri/src/managed_agents/agent_events.rs`). Built inline here
/// because the e2e crate does not depend on the desktop crate; the
/// projection-function exclusion contract is unit-tested in that module. This
/// is exactly the field set the desktop publishes for a definition-less
/// record (a definition-linked one omits the definition quad — the slimmed
/// NIP-AP shape; the relay accepts both, as the legacy-fat test below proves) —
/// secrets, the backend blob, env vars, and runtime fields are absent by
/// construction.
fn agent_projection_content(name: &str) -> String {
    serde_json::json!({
        "name": name,
        "system_prompt": "You are a test agent.",
        "model": "claude-opus-4",
        "provider": "anthropic",
        "mcp_toolsets": "default",
        "parallelism": 24,
        "respond_to": "allowlist",
        "respond_to_allowlist": ["79be667e"]
    })
    .to_string()
}

/// A legacy "fat" definition-linked projection (pre-slimming shape): carries
/// both `persona_id` and the definition quad. Old clients still publish this;
/// the relay must keep accepting it.
fn legacy_fat_agent_projection_content(name: &str) -> String {
    serde_json::json!({
        "name": name,
        "persona_id": "persona-1",
        "system_prompt": "You are a test agent.",
        "model": "claude-opus-4",
        "provider": "anthropic",
        "mcp_toolsets": "default",
        "persona_source_version": "abc123",
        "parallelism": 24,
        "respond_to": "allowlist",
        "respond_to_allowlist": ["79be667e"]
    })
    .to_string()
}

/// Build a managed-agent event whose `d`-tag is the agent's 64-hex pubkey,
/// mirroring the desktop `build_agent_event` shape.
fn agent_event(keys: &Keys, d_tag: &str, content: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(AGENT_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

fn agent_event_at(keys: &Keys, d_tag: &str, content: &str, created_at: u64) -> nostr::Event {
    EventBuilder::new(Kind::Custom(AGENT_KIND), content)
        .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a NIP-09 a-tag-only deletion at the agent's NIP-33 coordinate,
/// mirroring the desktop `build_agent_delete` shape (no `e`-tag).
fn agent_delete_event(keys: &Keys, d_tag: &str) -> nostr::Event {
    let coord = format!("{AGENT_KIND}:{}:{d_tag}", keys.public_key().to_hex());
    EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![Tag::parse(["a", coord.as_str()]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

/// A synthetic agent `d`-tag: 64 lowercase hex chars, the agent-pubkey grammar.
fn agent_d_tag() -> String {
    uuid::Uuid::new_v4().simple().to_string().repeat(2)
}

#[tokio::test]
#[ignore]
async fn test_managed_agent_publish_and_query() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = agent_d_tag();
    let content = agent_projection_content("Test Agent");

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = agent_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send agent");
    assert!(
        ok.accepted,
        "relay rejected managed-agent event: {}",
        ok.message
    );

    let sid = sub_id("query");
    let filter = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
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

    assert_eq!(events.len(), 1, "expected exactly one managed-agent event");
    let ev = &events[0];
    assert_eq!(ev.content, content);
    assert_eq!(ev.pubkey, keys.public_key());
    assert_eq!(ev.kind, Kind::Custom(AGENT_KIND));

    client.disconnect().await.expect("disconnect");
}

/// The relay keeps accepting the legacy "fat" definition-linked projection
/// (pre-slimming shape) — old clients publish it during the transition.
#[tokio::test]
#[ignore]
async fn test_legacy_fat_agent_event_still_accepted() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = agent_d_tag();
    let content = legacy_fat_agent_projection_content("Legacy Agent");

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let event = agent_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send legacy agent");
    assert!(
        ok.accepted,
        "relay rejected legacy fat managed-agent event: {}",
        ok.message
    );
    client.disconnect().await.expect("disconnect");
}

/// The relay round-trips published content byte-for-byte: a projection-shaped
/// body goes out, and the relay returns exactly those fields and nothing more.
/// The body here is a hand-built secret-free literal (`agent_projection_content`),
/// so this test does NOT exercise the desktop projection function — the e2e crate
/// can't reach it. The secret-exclusion regression guard lives in the
/// `agent_events.rs` unit test (`content_excludes_secrets_and_runtime_fields`),
/// which feeds a fully-populated secret-bearing record through the real
/// projection. This test confirms the relay neither adds nor drops fields.
#[tokio::test]
#[ignore]
async fn test_managed_agent_round_trips_only_projected_fields() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = agent_d_tag();
    let content = agent_projection_content("Secret-Free Agent");

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = agent_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send agent");
    assert!(
        ok.accepted,
        "relay rejected managed-agent event: {}",
        ok.message
    );

    let sid = sub_id("secrets");
    let filter = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
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

    assert_eq!(events.len(), 1, "expected exactly one managed-agent event");
    let published = &events[0].content;

    // The relay must not inject fields. These assertions confirm round-trip
    // fidelity for a projection-shaped body — they are NOT the secret guard
    // (the input literal is secret-free by construction; the real guard is the
    // `agent_events.rs` unit test over the projection function).
    for forbidden in [
        "private_key_nsec",
        "private_key",
        "nsec1",
        "auth_tag",
        "env_vars",
        "backend",
        "backend_agent_id",
        "provider_binary_path",
    ] {
        assert!(
            !published.contains(forbidden),
            "published managed-agent content leaked `{forbidden}`: {published}"
        );
    }

    // Runtime fields — must never appear.
    for forbidden in [
        "runtime_pid",
        "last_started_at",
        "last_stopped_at",
        "last_exit_code",
        "last_error",
        "relay_url",
    ] {
        assert!(
            !published.contains(forbidden),
            "published managed-agent content leaked runtime field `{forbidden}`: {published}"
        );
    }

    // Identity/config fields — must be present (proves we published real content).
    assert!(published.contains("Secret-Free Agent"), "missing name");
    assert!(published.contains("system_prompt"), "missing system_prompt");

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_managed_agent_nip33_replacement_newer_wins() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = agent_d_tag();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let now = Timestamp::now().as_secs();
    let old_content = agent_projection_content("Old Agent");
    let old_event = agent_event_at(&keys, &d_tag, &old_content, now - 100);
    let ok = client.send_event(old_event).await.expect("send old");
    assert!(ok.accepted, "relay rejected old event: {}", ok.message);

    let new_content = agent_projection_content("New Agent");
    let new_event = agent_event_at(&keys, &d_tag, &new_content, now);
    let ok = client.send_event(new_event).await.expect("send new");
    assert!(ok.accepted, "relay rejected new event: {}", ok.message);

    let sid = sub_id("replace");
    let filter = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
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

/// The a-tag tombstone is the only state-destroying op in the managed-agent
/// flow. Publish an agent, confirm it is live, publish the a-tag-only tombstone
/// at its coordinate, then assert the query returns it gone.
#[tokio::test]
#[ignore]
async fn test_managed_agent_tombstone_deletes_coordinate() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = agent_d_tag();
    let content = agent_projection_content("Doomed Agent");

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let event = agent_event(&keys, &d_tag, &content);
    let ok = client.send_event(event).await.expect("send agent");
    assert!(
        ok.accepted,
        "relay rejected managed-agent event: {}",
        ok.message
    );

    let filter = || {
        Filter::new()
            .kind(Kind::Custom(AGENT_KIND))
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
    assert_eq!(before.len(), 1, "agent should be live before deletion");

    let tombstone = agent_delete_event(&keys, &d_tag);
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
        "tombstone should remove the agent coordinate, got {} event(s)",
        after.len()
    );

    client.disconnect().await.expect("disconnect");
}
