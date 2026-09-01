//! End-to-end tests for agent media sessions (kind:48200 / kind:48201).
//!
//! These bind the relay's *stored-state* authorization for the two kinds —
//! the half that `validate_agent_media_session_links` decides and that the
//! in-crate shape tests cannot reach, because every predicate here needs a
//! real community, a real channel, and a real owner→agent registration:
//!
//! - only a registered agent may announce a session;
//! - an end must name a start that exists and is a start;
//! - an end must be published in the channel its start was announced in;
//! - only the session's owner (or the relay) may end it.
//!
//! Each refusal is paired with the acceptance it is the refusal *of*. A guard
//! that tightened until it rejected everything would satisfy the refusals
//! alone, so on their own they prove nothing about the boundary they claim to
//! describe.
//!
//! Ownership is established the way production establishes it: the agent
//! authenticates over NIP-42 carrying a NIP-OA `auth` tag signed by its owner,
//! and the relay materializes the relationship. Seeding `agent_owner_pubkey`
//! directly would test a row the feature does not read.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_agent_media_session -- --ignored
//! ```

use buzz_core::kind::{KIND_AGENT_MEDIA_SESSION_ENDED, KIND_AGENT_MEDIA_SESSION_STARTED};
use buzz_sdk::nip_oa;
use buzz_test_client::{BuzzTestClient, OkResponse};
use nostr::{EventBuilder, Keys, Kind, Tag};

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

/// Create a fresh open channel, signed by `creator` over its own authenticated
/// connection, and return its UUID string.
///
/// Published over the WebSocket rather than `POST /events` on purpose. The HTTP
/// bridge is a second authorization surface — a relay running with
/// `BUZZ_REQUIRE_AUTH_TOKEN` refuses a bare `X-Pubkey` submission — and setup
/// failing for a reason unrelated to media sessions makes every test in this
/// file unrunnable against a relay that is otherwise perfectly able to answer
/// the question being asked. NIP-42 is the path these tests already depend on.
async fn create_channel(client: &mut BuzzTestClient, creator: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4();

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("ams-e2e-{}", channel_uuid.simple())]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(creator)
        .unwrap();

    let ok = client
        .send_event(event)
        .await
        .expect("send create-channel event");
    assert!(ok.accepted, "channel creation not accepted: {}", ok.message);
    channel_uuid.to_string()
}

/// Connect `agent` with a NIP-OA `auth` tag signed by `owner`.
///
/// This is what makes the pubkey a *registered agent* as far as the media
/// session gate is concerned: the relay records the owner during auth, and
/// the gate resolves it back out.
async fn connect_registered_agent(agent: &Keys, owner: &Keys) -> BuzzTestClient {
    let tag_json = nip_oa::compute_auth_tag(owner, &agent.public_key(), "kind=9")
        .expect("compute NIP-OA auth tag");
    let auth_tag = nip_oa::parse_auth_tag(&tag_json).expect("parse NIP-OA auth tag");
    let mut client = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("connect agent unauthenticated");
    client
        .authenticate_with_nip_oa(agent, &auth_tag)
        .await
        .expect("NIP-OA auth");
    client
}

/// Add `member` to `channel_id` via kind:9000 (PUT_USER), signed by `actor`.
async fn add_channel_member(
    client: &mut BuzzTestClient,
    actor: &Keys,
    channel_id: &str,
    member: &Keys,
) {
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(actor)
        .unwrap();
    let ok = client.send_event(event).await.expect("send PUT_USER");
    assert!(
        ok.accepted,
        "adding a channel member failed: {}",
        ok.message
    );
}

/// A valid 48200 body expiring `offset_secs` from now.
fn session_body(offset_secs: i64) -> String {
    let expires_at = nostr::Timestamp::now().as_secs() as i64 + offset_secs;
    format!(
        r#"{{"v":1,"provider":"livekit","connect":{{"url":"wss://media.example","room":"r-{}"}},"expires_at":{expires_at}}}"#,
        uuid::Uuid::new_v4().simple()
    )
}

async fn announce_session(
    client: &mut BuzzTestClient,
    signer: &Keys,
    channel_id: &str,
) -> OkResponse {
    let event = EventBuilder::new(
        Kind::Custom(KIND_AGENT_MEDIA_SESSION_STARTED as u16),
        session_body(600),
    )
    .tags(vec![Tag::parse(["h", channel_id]).unwrap()])
    .sign_with_keys(signer)
    .unwrap();
    client.send_event(event).await.expect("send 48200")
}

async fn end_session(
    client: &mut BuzzTestClient,
    signer: &Keys,
    channel_id: &str,
    start_event_id: &str,
) -> OkResponse {
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_MEDIA_SESSION_ENDED as u16), "")
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["e", start_event_id]).unwrap(),
        ])
        .sign_with_keys(signer)
        .unwrap();
    client.send_event(event).await.expect("send 48201")
}

// ─── who may announce ──────────────────────────────────────────────────────

/// The acceptance the refusal below is the refusal *of*.
#[tokio::test]
#[ignore]
async fn registered_agent_can_announce_a_session() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut client, &agent).await;

    let ok = announce_session(&mut client, &agent, &channel_id).await;
    assert!(
        ok.accepted,
        "a registered agent's own announcement was rejected: {}",
        ok.message
    );

    client.disconnect().await.ok();
}

/// The gate itself: a member of the channel who is not a registered agent
/// cannot announce a session there.
///
/// This is the whole security boundary of the feature. Without it the contract
/// is "any member may announce a media session under an agent's identity",
/// which is a different feature wearing this one's name — and the card renders
/// as an agent either way.
#[tokio::test]
#[ignore]
async fn an_unregistered_member_cannot_announce_a_session() {
    let human = Keys::generate();
    let mut client = BuzzTestClient::connect(&relay_url(), &human)
        .await
        .expect("connect human");
    let channel_id = create_channel(&mut client, &human).await;

    // The channel's own creator, so a refusal here cannot be membership.
    let ok = announce_session(&mut client, &human, &channel_id).await;
    assert!(
        !ok.accepted,
        "an unregistered member announced a media session"
    );
    assert!(
        ok.message.contains("registered agent"),
        "expected the registered-agent refusal, got: {}",
        ok.message
    );

    client.disconnect().await.ok();
}

// ─── who may end, and what an end may name ─────────────────────────────────

#[tokio::test]
#[ignore]
async fn a_session_owner_can_end_its_own_session() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut client, &agent).await;

    let start = announce_session(&mut client, &agent, &channel_id).await;
    assert!(start.accepted, "announcement rejected: {}", start.message);

    let ok = end_session(&mut client, &agent, &channel_id, &start.event_id).await;
    assert!(
        ok.accepted,
        "an agent could not end its own session: {}",
        ok.message
    );

    client.disconnect().await.ok();
}

/// A third party cannot retire somebody else's live session.
///
/// The third party is itself a registered agent and a member of the channel,
/// and its own announcement is accepted first. Without that, a refusal here
/// would be ambiguous: it could just as well be the membership gate or the
/// registered-agent gate, and the end-authorization rule would be untested
/// while appearing covered.
#[tokio::test]
#[ignore]
async fn a_third_party_cannot_end_another_agents_session() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let intruder_owner = Keys::generate();
    let intruder = Keys::generate();

    let mut agent_client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut agent_client, &agent).await;
    add_channel_member(&mut agent_client, &agent, &channel_id, &intruder).await;

    let start = announce_session(&mut agent_client, &agent, &channel_id).await;
    assert!(start.accepted, "announcement rejected: {}", start.message);

    let mut intruder_client = connect_registered_agent(&intruder, &intruder_owner).await;
    let own = announce_session(&mut intruder_client, &intruder, &channel_id).await;
    assert!(
        own.accepted,
        "the intruder must be able to publish here, or the refusal below proves nothing: {}",
        own.message
    );

    let ok = end_session(
        &mut intruder_client,
        &intruder,
        &channel_id,
        &start.event_id,
    )
    .await;
    assert!(!ok.accepted, "a third party ended another agent's session");
    assert!(
        ok.message.contains("session owner"),
        "expected the end-authorization refusal, got: {}",
        ok.message
    );

    agent_client.disconnect().await.ok();
    intruder_client.disconnect().await.ok();
}

#[tokio::test]
#[ignore]
async fn an_end_must_name_a_start_that_exists() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut client, &agent).await;

    let unknown = "b".repeat(64);
    let ok = end_session(&mut client, &agent, &channel_id, &unknown).await;
    assert!(!ok.accepted, "an end named a start that does not exist");
    assert!(
        ok.message.contains("unknown start"),
        "expected the unknown-start refusal, got: {}",
        ok.message
    );

    client.disconnect().await.ok();
}

/// An `e` tag to some *other* stored event is not a session start.
#[tokio::test]
#[ignore]
async fn an_end_must_reference_a_session_start() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut client, &agent).await;

    let content = format!("not-a-session-{}", uuid::Uuid::new_v4());
    let message = client
        .send_text_message(&agent, &channel_id, &content, 9)
        .await
        .expect("send message");
    assert!(message.accepted, "message rejected: {}", message.message);

    let ok = end_session(&mut client, &agent, &channel_id, &message.event_id).await;
    assert!(!ok.accepted, "an end referenced an ordinary message");
    assert!(
        ok.message.contains("session start"),
        "expected the wrong-kind refusal, got: {}",
        ok.message
    );

    client.disconnect().await.ok();
}

/// An end published in a different channel would retire a card its own readers
/// never saw.
#[tokio::test]
#[ignore]
async fn an_end_must_be_published_in_the_starts_channel() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut client = connect_registered_agent(&agent, &owner).await;
    let channel_id = create_channel(&mut client, &agent).await;
    let elsewhere = create_channel(&mut client, &agent).await;

    let start = announce_session(&mut client, &agent, &channel_id).await;
    assert!(start.accepted, "announcement rejected: {}", start.message);

    let ok = end_session(&mut client, &agent, &elsewhere, &start.event_id).await;
    assert!(!ok.accepted, "an end retired a start from another channel");
    assert!(
        ok.message.contains("start's channel"),
        "expected the channel-scoping refusal, got: {}",
        ok.message
    );

    client.disconnect().await.ok();
}
