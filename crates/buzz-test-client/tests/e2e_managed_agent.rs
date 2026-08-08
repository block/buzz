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

/// NIP-33 author-coordinate isolation probe (relay-level, two keypairs on one relay).
///
/// This test verifies relay-level NIP-33 author scoping. It does NOT cover
/// desktop workspace activation, `apply_workspace`, the scoped file store,
/// inbound event routing, or runtime fan-out — those are verified by desktop
/// unit tests and the live two-workspace probe run after Thufir's clear.
///
/// Two distinct owner keypairs share one relay. The test verifies:
///
/// 1. Owner A's events are author-scoped: a subscription filtered by
///    `author: owner_a` returns only owner_a's events, not owner_b's.
/// 2. Symmetrically, owner B's subscription returns only owner_b's events.
/// 3. NIP-33 coordinates are scoped by `(kind, author, d-tag)`. A subscription
///    for `(kind=30177, author=owner_b, d=shared_d_tag)` returns B's event —
///    not A's — confirming the (kind, author, d-tag) tuple is unique per owner.
///
/// The filesystem isolation proof (different `(relay_url, owner_pubkey)` pairs
/// always produce distinct scope_id directories) is covered separately by the
/// scope_id unit tests.
#[tokio::test]
#[ignore]
async fn test_two_workspace_relay_partition() {
    let url = relay_url();

    // Workspace A and workspace B: two distinct owner keypairs (same relay)
    let owner_a_keys = Keys::generate();
    let owner_b_keys = Keys::generate();

    // Use the same d-tag value (simulating same agent slug) in both workspaces.
    // Relay NIP-33 addressing is (kind, author, d-tag) — so same d-tag but
    // different authors are distinct coordinates that cannot collide.
    let shared_d_tag = "workspace-leak-probe-agent";

    // Publish agent definition as owner A
    let mut client_a = BuzzTestClient::connect(&url, &owner_a_keys)
        .await
        .expect("owner_a connect");
    let content_a = agent_projection_content("WorkspaceA-ExclusiveAgent");
    let event_a = EventBuilder::new(Kind::Custom(AGENT_KIND), content_a.clone())
        .tag(Tag::identifier(shared_d_tag))
        .sign_with_keys(&owner_a_keys)
        .expect("owner_a sign");
    let ok_a = client_a
        .send_event(event_a)
        .await
        .expect("send owner_a event");
    assert!(
        ok_a.accepted,
        "relay rejected owner_a's event: {}",
        ok_a.message
    );

    // Publish agent definition as owner B (same relay, different owner)
    let mut client_b = BuzzTestClient::connect(&url, &owner_b_keys)
        .await
        .expect("owner_b connect");
    let content_b = agent_projection_content("WorkspaceB-ExclusiveAgent");
    let event_b = EventBuilder::new(Kind::Custom(AGENT_KIND), content_b.clone())
        .tag(Tag::identifier(shared_d_tag))
        .sign_with_keys(&owner_b_keys)
        .expect("owner_b sign");
    let ok_b = client_b
        .send_event(event_b)
        .await
        .expect("send owner_b event");
    assert!(
        ok_b.accepted,
        "relay rejected owner_b's event: {}",
        ok_b.message
    );

    // ── Direction 1: owner_a's author-scoped subscription ──────────────────
    // Owner A subscribes to their own agent coordinate.
    // Must see exactly their definition, not owner_b's.
    let sid_a = sub_id("probe-workspace-a");
    let filter_a = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
        .author(owner_a_keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [shared_d_tag]);
    client_a
        .subscribe(&sid_a, vec![filter_a])
        .await
        .expect("owner_a subscribe");
    let events_a = client_a
        .collect_until_eose(&sid_a, Duration::from_secs(5))
        .await
        .expect("owner_a collect");

    assert_eq!(
        events_a.len(),
        1,
        "owner_a's NIP-33 subscription must return exactly 1 event (their own), got {}",
        events_a.len()
    );
    assert!(
        events_a[0].content.contains("WorkspaceA-ExclusiveAgent"),
        "owner_a's event must contain workspace-A content, got: {}",
        events_a[0].content
    );
    assert_eq!(
        events_a[0].pubkey,
        owner_a_keys.public_key(),
        "owner_a's subscription must not return events from owner_b"
    );
    assert!(
        !events_a[0].content.contains("WorkspaceB-ExclusiveAgent"),
        "workspace A's subscription must NOT return workspace B's content"
    );

    // ── Direction 2: owner_b's author-scoped subscription ──────────────────
    // Symmetric: owner B must see only their definition.
    let sid_b = sub_id("probe-workspace-b");
    let filter_b = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
        .author(owner_b_keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [shared_d_tag]);
    client_b
        .subscribe(&sid_b, vec![filter_b])
        .await
        .expect("owner_b subscribe");
    let events_b = client_b
        .collect_until_eose(&sid_b, Duration::from_secs(5))
        .await
        .expect("owner_b collect");

    assert_eq!(
        events_b.len(),
        1,
        "owner_b's NIP-33 subscription must return exactly 1 event (their own), got {}",
        events_b.len()
    );
    assert!(
        events_b[0].content.contains("WorkspaceB-ExclusiveAgent"),
        "owner_b's event must contain workspace-B content, got: {}",
        events_b[0].content
    );
    assert_eq!(
        events_b[0].pubkey,
        owner_b_keys.public_key(),
        "owner_b's subscription must not return events from owner_a"
    );
    assert!(
        !events_b[0].content.contains("WorkspaceA-ExclusiveAgent"),
        "workspace B's subscription must NOT return workspace A's content"
    );

    // ── Direction 3: NIP-33 coordinate ownership — B's coord returns B's event ──
    // Owner A subscribes to the same d-tag but filtered by owner_b's pubkey.
    // This proves that NIP-33 coordinates are scoped by (kind, author, d-tag):
    // A's coordinate and B's coordinate are distinct even though they share
    // the same d-tag value, because they are authored by different pubkeys.
    // The query returns B's event — not A's — confirming per-author isolation.
    let sid_cross = sub_id("probe-cross-scope");
    let filter_cross = Filter::new()
        .kind(Kind::Custom(AGENT_KIND))
        .author(owner_b_keys.public_key()) // owner_b's pubkey
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [shared_d_tag]);
    client_a
        .subscribe(&sid_cross, vec![filter_cross])
        .await
        .expect("cross-scope subscribe");
    let events_cross = client_a
        .collect_until_eose(&sid_cross, Duration::from_secs(5))
        .await
        .expect("cross-scope collect");

    // The cross-scope query must return B's event (by B's pubkey), not A's.
    // This confirms NIP-33 coordinates are scoped by (kind, author, d-tag).
    assert_eq!(
        events_cross.len(),
        1,
        "cross-scope query must return exactly 1 event (B's own), got {}",
        events_cross.len()
    );
    assert_eq!(
        events_cross[0].pubkey,
        owner_b_keys.public_key(),
        "cross-scope query must return B's event, not A's"
    );
    assert!(
        !events_cross[0]
            .content
            .contains("WorkspaceA-ExclusiveAgent"),
        "cross-scope query must NOT return workspace A's definitions"
    );

    client_a.disconnect().await.expect("owner_a disconnect");
    client_b.disconnect().await.expect("owner_b disconnect");
}
