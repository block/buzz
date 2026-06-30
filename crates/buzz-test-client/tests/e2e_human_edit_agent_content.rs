//! End-to-end tests for human owners editing/managing content authored by
//! their agents — all four authorization predicate sites:
//!
//! - kind:40003 message edit (`validate_edit_ownership`)
//! - kind:9005 DELETE_EVENT (`validate_admin_event` 9005 branch)
//! - kind:9002 EDIT_METADATA privileged-tag branch
//! - kind:9008 DELETE_GROUP
//!
//! The owner→agent relationship is established via NIP-OA: the agent
//! connects and authenticates with an `auth` tag signed by the owner.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_human_edit_agent_content -- --ignored
//! ```


use buzz_sdk::nip_oa;
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};

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


/// Create a fresh channel owned by `owner_keys`, return the channel UUID string.
async fn create_agent_owned_channel(agent_keys: &Keys) -> String {
    let http = reqwest::Client::new();
    let channel_uuid = uuid::Uuid::new_v4();

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("haec-test-{}", channel_uuid.simple())]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(agent_keys)
        .unwrap();

    let resp = http
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &agent_keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-channel event");
    assert!(
        resp.status().is_success(),
        "channel creation failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {body}"
    );
    channel_uuid.to_string()
}

/// Build a NIP-OA auth tag for `agent_keys` signed by `owner_keys`.
fn make_nip_oa_auth_tag(owner_keys: &Keys, agent_keys: &Keys) -> Tag {
    let tag_json = nip_oa::compute_auth_tag(owner_keys, &agent_keys.public_key(), "kind=9")
        .expect("compute_auth_tag");
    nip_oa::parse_auth_tag(&tag_json).expect("parse_auth_tag")
}

/// Connect `agent_keys` to the relay with NIP-OA, establishing owner→agent in the DB.
/// Returns the connected (authenticated) client for the agent.
async fn connect_agent_with_owner(agent_keys: &Keys, owner_keys: &Keys) -> BuzzTestClient {
    let url = relay_url();
    let auth_tag = make_nip_oa_auth_tag(owner_keys, agent_keys);
    let mut client = BuzzTestClient::connect_unauthenticated(&url)
        .await
        .expect("connect agent unauthenticated");
    client
        .authenticate_with_nip_oa(agent_keys, &auth_tag)
        .await
        .expect("NIP-OA auth");
    client
}

// ─── kind:40003 message edit ───────────────────────────────────────────────

/// Owner can edit a message authored by their agent via kind:40003.
#[tokio::test]
#[ignore]
async fn test_owner_can_edit_agent_message() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    // Establish NIP-OA ownership in DB.
    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    // Agent sends a message.
    let content = format!("agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message");
    assert!(ok.accepted, "agent message rejected: {}", ok.message);
    let msg_event_id = ok.event_id;

    // Owner sends a kind:40003 edit event targeting the agent's message.
    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");

    let edit_event = EventBuilder::new(Kind::Custom(40003), "corrected content")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    let ok = owner_client
        .send_event(edit_event)
        .await
        .expect("send edit");
    assert!(
        ok.accepted,
        "owner edit of agent message rejected: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
    owner_client.disconnect().await.ok();
}

/// An unrelated third party cannot edit an agent's message.
#[tokio::test]
#[ignore]
async fn test_third_party_cannot_edit_agent_message() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let third_party_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    let content = format!("agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message");
    assert!(ok.accepted, "agent message rejected: {}", ok.message);
    let msg_event_id = ok.event_id;

    let mut third_party_client = BuzzTestClient::connect(&relay_url(), &third_party_keys)
        .await
        .expect("connect third party");

    let edit_event = EventBuilder::new(Kind::Custom(40003), "malicious edit")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&third_party_keys)
        .unwrap();

    let ok = third_party_client
        .send_event(edit_event)
        .await
        .expect("send edit attempt");
    assert!(
        !ok.accepted,
        "third party should NOT be able to edit agent message, but was accepted"
    );

    agent_client.disconnect().await.ok();
    third_party_client.disconnect().await.ok();
}

/// The agent itself can still edit its own message (self-edit unchanged).
#[tokio::test]
#[ignore]
async fn test_agent_can_self_edit_message() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    let content = format!("agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message");
    assert!(ok.accepted, "agent message rejected: {}", ok.message);
    let msg_event_id = ok.event_id;

    let edit_event = EventBuilder::new(Kind::Custom(40003), "agent self-edit")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&agent_keys)
        .unwrap();

    let ok = agent_client.send_event(edit_event).await.expect("send edit");
    assert!(
        ok.accepted,
        "agent self-edit rejected: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
}

// ─── kind:9005 DELETE_EVENT ─────────────────────────────────────────────────

/// Owner can delete a message authored by their agent via kind:9005.
#[tokio::test]
#[ignore]
async fn test_owner_can_delete_agent_message() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    let content = format!("agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message");
    assert!(ok.accepted, "agent message rejected: {}", ok.message);
    let msg_event_id = ok.event_id;

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");

    let delete_event = EventBuilder::new(Kind::Custom(9005), "")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    let ok = owner_client
        .send_event(delete_event)
        .await
        .expect("send delete");
    assert!(
        ok.accepted,
        "owner delete of agent message rejected: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
    owner_client.disconnect().await.ok();
}

/// An unrelated third party cannot delete an agent's message.
#[tokio::test]
#[ignore]
async fn test_third_party_cannot_delete_agent_message() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let third_party_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    let content = format!("agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message");
    assert!(ok.accepted, "agent message rejected: {}", ok.message);
    let msg_event_id = ok.event_id;

    let mut third_party_client = BuzzTestClient::connect(&relay_url(), &third_party_keys)
        .await
        .expect("connect third party");

    let delete_event = EventBuilder::new(Kind::Custom(9005), "")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&third_party_keys)
        .unwrap();

    let ok = third_party_client
        .send_event(delete_event)
        .await
        .expect("send delete attempt");
    assert!(
        !ok.accepted,
        "third party should NOT be able to delete agent message, but was accepted"
    );

    agent_client.disconnect().await.ok();
    third_party_client.disconnect().await.ok();
}

// ─── kind:9002 EDIT_METADATA ────────────────────────────────────────────────

/// Owner can edit metadata (name/archived) of a channel owned by their agent,
/// even when the owner is not a channel member.
#[tokio::test]
#[ignore]
async fn test_owner_can_edit_agent_channel_metadata() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();

    // Agent creates the channel (agent is the channel owner-member).
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    // Establish NIP-OA ownership.
    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    // Owner sends kind:9002 to rename the channel — owner is NOT a member.
    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");

    let edit_event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", "owner-renamed-channel"]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    let ok = owner_client
        .send_event(edit_event)
        .await
        .expect("send edit metadata");
    assert!(
        ok.accepted,
        "owner edit of agent channel metadata rejected: {}",
        ok.message
    );

    owner_client.disconnect().await.ok();
}

/// Owner can archive an agent's channel via kind:9002.
#[tokio::test]
#[ignore]
async fn test_owner_can_archive_agent_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");

    let archive_event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["archived", "true"]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();

    let ok = owner_client
        .send_event(archive_event)
        .await
        .expect("send archive");
    assert!(
        ok.accepted,
        "owner archive of agent channel rejected: {}",
        ok.message
    );

    owner_client.disconnect().await.ok();
}

/// Unrelated third party cannot edit metadata of an agent's channel.
#[tokio::test]
#[ignore]
async fn test_third_party_cannot_edit_agent_channel_metadata() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let third_party_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    let mut third_party_client = BuzzTestClient::connect(&relay_url(), &third_party_keys)
        .await
        .expect("connect third party");

    let edit_event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", "hijacked-name"]).unwrap(),
        ])
        .sign_with_keys(&third_party_keys)
        .unwrap();

    let ok = third_party_client
        .send_event(edit_event)
        .await
        .expect("send edit attempt");
    assert!(
        !ok.accepted,
        "third party should NOT be able to edit agent channel metadata, but was accepted"
    );

    third_party_client.disconnect().await.ok();
}

// ─── kind:9008 DELETE_GROUP ─────────────────────────────────────────────────

/// Owner can delete a channel owned by their agent via kind:9008,
/// even when the owner is not a channel member.
#[tokio::test]
#[ignore]
async fn test_owner_can_delete_agent_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    // Establish NIP-OA ownership.
    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    // Owner sends kind:9008 to delete the channel — owner is NOT a member.
    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");

    let delete_event = EventBuilder::new(Kind::Custom(9008), "")
        .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
        .sign_with_keys(&owner_keys)
        .unwrap();

    let ok = owner_client
        .send_event(delete_event)
        .await
        .expect("send delete group");
    assert!(
        ok.accepted,
        "owner delete of agent channel rejected: {}",
        ok.message
    );

    owner_client.disconnect().await.ok();
}

/// Unrelated third party cannot delete an agent's channel.
#[tokio::test]
#[ignore]
async fn test_third_party_cannot_delete_agent_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let third_party_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    let mut third_party_client = BuzzTestClient::connect(&relay_url(), &third_party_keys)
        .await
        .expect("connect third party");

    let delete_event = EventBuilder::new(Kind::Custom(9008), "")
        .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
        .sign_with_keys(&third_party_keys)
        .unwrap();

    let ok = third_party_client
        .send_event(delete_event)
        .await
        .expect("send delete attempt");
    assert!(
        !ok.accepted,
        "third party should NOT be able to delete agent channel, but was accepted"
    );

    third_party_client.disconnect().await.ok();
}

/// Create a fresh **private** channel owned by `agent_keys`, return the channel UUID string.
async fn create_private_agent_owned_channel(agent_keys: &Keys) -> String {
    let http = reqwest::Client::new();
    let channel_uuid = uuid::Uuid::new_v4();

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("haec-private-test-{}", channel_uuid.simple())]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(agent_keys)
        .unwrap();

    let resp = http
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &agent_keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-private-channel event");
    assert!(
        resp.status().is_success(),
        "private channel creation failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "private channel creation not accepted: {body}"
    );
    channel_uuid.to_string()
}

// ─── Private-channel coverage (Fix 1 regression guard) ──────────────────────
//
// These tests verify that the membership-gate bypass works on private channels —
// i.e., a non-member owning human can act on private agent-owned content.
// Previously all four predicates were unreachable for private channels because
// check_channel_membership rejected non-members before the per-kind validators ran.

/// Owner can edit a message in a private agent-owned channel (non-member owner allowed).
#[tokio::test]
#[ignore]
async fn test_owner_can_edit_agent_message_in_private_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_private_agent_owned_channel(&agent_keys).await;

    // Establish NIP-OA ownership; agent sends a message.
    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    let content = format!("private-agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message to private channel");
    assert!(ok.accepted, "agent message to private channel rejected: {}", ok.message);
    let msg_event_id = ok.event_id;
    agent_client.disconnect().await.ok();

    // Owner (not a channel member) edits the agent's message.
    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");
    let edit_event = EventBuilder::new(Kind::Custom(40003), "edited content")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let ok = owner_client.send_event(edit_event).await.expect("send edit");
    assert!(
        ok.accepted,
        "owner edit of agent message in private channel rejected (membership gate not bypassed): {}",
        ok.message
    );
    owner_client.disconnect().await.ok();
}

/// Owner can delete a message in a private agent-owned channel (non-member owner allowed).
#[tokio::test]
#[ignore]
async fn test_owner_can_delete_agent_message_in_private_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_private_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    let content = format!("private-agent-msg-{}", uuid::Uuid::new_v4());
    let ok = agent_client
        .send_text_message(&agent_keys, &channel_id, &content, 9)
        .await
        .expect("agent send message to private channel");
    assert!(ok.accepted, "agent message to private channel rejected: {}", ok.message);
    let msg_event_id = ok.event_id;
    agent_client.disconnect().await.ok();

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");
    let delete_event = EventBuilder::new(Kind::Custom(9005), "")
        .tags(vec![
            Tag::parse(["e", &msg_event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let ok = owner_client.send_event(delete_event).await.expect("send delete");
    assert!(
        ok.accepted,
        "owner delete of agent message in private channel rejected (membership gate not bypassed): {}",
        ok.message
    );
    owner_client.disconnect().await.ok();
}

/// Owner can edit metadata of a private agent-owned channel (non-member owner allowed).
#[tokio::test]
#[ignore]
async fn test_owner_can_edit_metadata_of_private_agent_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_private_agent_owned_channel(&agent_keys).await;

    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");
    let edit_event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", "owner-renamed-private-channel"]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let ok = owner_client.send_event(edit_event).await.expect("send edit metadata");
    assert!(
        ok.accepted,
        "owner edit of private agent channel metadata rejected (membership gate not bypassed): {}",
        ok.message
    );
    owner_client.disconnect().await.ok();
}

/// Owner can delete a private agent-owned channel (non-member owner allowed).
#[tokio::test]
#[ignore]
async fn test_owner_can_delete_private_agent_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_private_agent_owned_channel(&agent_keys).await;

    let agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;
    agent_client.disconnect().await.ok();

    let mut owner_client = BuzzTestClient::connect(&relay_url(), &owner_keys)
        .await
        .expect("connect owner");
    let delete_event = EventBuilder::new(Kind::Custom(9008), "")
        .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let ok = owner_client.send_event(delete_event).await.expect("send delete group");
    assert!(
        ok.accepted,
        "owner delete of private agent channel rejected (membership gate not bypassed): {}",
        ok.message
    );
    owner_client.disconnect().await.ok();
}

/// Agent itself can still delete its own channel (self-delete unchanged).
#[tokio::test]
#[ignore]
async fn test_agent_can_self_delete_channel() {
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let channel_id = create_agent_owned_channel(&agent_keys).await;

    let mut agent_client = connect_agent_with_owner(&agent_keys, &owner_keys).await;

    let delete_event = EventBuilder::new(Kind::Custom(9008), "")
        .tags(vec![Tag::parse(["h", &channel_id]).unwrap()])
        .sign_with_keys(&agent_keys)
        .unwrap();

    let ok = agent_client
        .send_event(delete_event)
        .await
        .expect("send self-delete group");
    assert!(
        ok.accepted,
        "agent self-delete of own channel rejected: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
}
