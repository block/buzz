//! End-to-end tests for NIP-AO observer frames (kind 24200) between an agent
//! and its owner:
//!
//! - an owner's connection may subscribe to the control frames it sends to
//!   agents it owns; another user's connection may not;
//! - an agent may publish telemetry over HTTP `POST /events`, with the same
//!   rules as over WebSocket, and the relay never stores it.
//!
//! The owner→agent link is made with NIP-OA: the agent authenticates with an
//! `auth` tag signed by the owner.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_agent_observer -- --ignored
//! ```

use std::time::Duration;

use buzz_core::kind::KIND_AGENT_OBSERVER_FRAME;
use buzz_core::observer::{
    encrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG,
    OBSERVER_FRAME_TELEMETRY,
};
use buzz_sdk::nip_oa;
use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{
    Alphabet, Event, EventBuilder, Filter, Keys, Kind, PublicKey, SingleLetterTag, Tag, Timestamp,
};
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(5);

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

/// Authenticate `agent` with a NIP-OA tag signed by `owner`, which records the
/// owner link in the relay's database.
async fn connect_agent_with_owner(agent: &Keys, owner: &Keys) -> BuzzTestClient {
    let tag_json =
        nip_oa::compute_auth_tag(owner, &agent.public_key(), "kind=9").expect("compute_auth_tag");
    let auth_tag = nip_oa::parse_auth_tag(&tag_json).expect("parse_auth_tag");
    let mut client = BuzzTestClient::connect_unauthenticated(&relay_url())
        .await
        .expect("connect agent");
    client
        .authenticate_with_nip_oa(agent, &auth_tag)
        .await
        .expect("NIP-OA auth");
    client
}

async fn connect(keys: &Keys) -> BuzzTestClient {
    BuzzTestClient::connect(&relay_url(), keys)
        .await
        .expect("connect")
}

/// An observer frame from `signer` to `recipient` about `agent`, created at
/// `created_at`.
fn observer_frame(
    signer: &Keys,
    recipient: &PublicKey,
    agent: &PublicKey,
    frame: &str,
    payload: &Value,
    created_at: Timestamp,
) -> Event {
    let content = encrypt_observer_payload(signer, recipient, payload).expect("encrypt");
    EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), content)
        .tags([
            Tag::parse(["p", &recipient.to_hex()]).unwrap(),
            Tag::parse([OBSERVER_AGENT_TAG, &agent.to_hex()]).unwrap(),
            Tag::parse([OBSERVER_FRAME_TAG, frame]).unwrap(),
        ])
        .custom_created_at(created_at)
        .sign_with_keys(signer)
        .unwrap()
}

fn observer_filter(p: &PublicKey) -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::P), [p.to_hex()])
        .since(Timestamp::now())
}

/// Open `filter` as `sub_id` and return the relay's first answer for it:
/// `Ok(())` on EOSE, `Err(reason)` on CLOSED.
async fn open(client: &mut BuzzTestClient, sub_id: &str, filter: Filter) -> Result<(), String> {
    client
        .subscribe(sub_id, vec![filter])
        .await
        .expect("send REQ");
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match client.recv_event(remaining).await.expect("relay answer") {
            RelayMessage::Eose { subscription_id } if subscription_id == sub_id => return Ok(()),
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sub_id => return Err(message),
            _ => {}
        }
    }
}

/// Wait for `event_id` on `sub_id`.
async fn expect_event(client: &mut BuzzTestClient, sub_id: &str, event_id: &nostr::EventId) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match client.recv_event(remaining).await {
            Ok(RelayMessage::Event {
                subscription_id,
                event,
            }) if subscription_id == sub_id && event.id == *event_id => return,
            Ok(_) => {}
            Err(e) => panic!("frame {event_id} not delivered on {sub_id}: {e}"),
        }
    }
}

/// POST `event` to `/events` as `caller` and return (HTTP status, body).
async fn post_event(caller: &Keys, event: &Event) -> (reqwest::StatusCode, Value) {
    let resp = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", caller.public_key().to_hex())
        .json(event)
        .send()
        .await
        .expect("POST /events");
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// Device A and device B are two connections of one owner. B receives the
/// control frame A sends to their agent; another user cannot subscribe to it.
#[tokio::test]
#[ignore]
async fn owner_connection_receives_control_frames_for_its_agent() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let stranger = Keys::generate();
    let agent_client = connect_agent_with_owner(&agent, &owner).await;

    let mut device_b = connect(&owner).await;
    open(
        &mut device_b,
        "control-b",
        observer_filter(&agent.public_key()),
    )
    .await
    .expect("owner may subscribe to its agent's control frames");

    let mut other = connect(&stranger).await;
    let refused = open(
        &mut other,
        "control-x",
        observer_filter(&agent.public_key()),
    )
    .await
    .expect_err("another user may not subscribe to the agent's control frames");
    assert!(refused.starts_with("restricted:"), "{refused}");

    let mut device_a = connect(&owner).await;
    let cancel = observer_frame(
        &owner,
        &agent.public_key(),
        &agent.public_key(),
        OBSERVER_FRAME_CONTROL,
        &json!({ "type": "cancel_turn", "channelId": uuid::Uuid::new_v4().to_string() }),
        Timestamp::now(),
    );
    let ok = device_a.send_event(cancel.clone()).await.expect("send");
    assert!(ok.accepted, "control frame rejected: {}", ok.message);
    expect_event(&mut device_b, "control-b", &cancel.id).await;

    for client in [agent_client, device_a, device_b, other] {
        client.disconnect().await.ok();
    }
}

/// An agent that publishes only over HTTP sends telemetry to its owner's live
/// REQ. An unowned recipient and a stale frame are refused, and nothing is
/// stored.
#[tokio::test]
#[ignore]
async fn agent_publishes_telemetry_over_http() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let stranger = Keys::generate();
    let agent_client = connect_agent_with_owner(&agent, &owner).await;

    let mut owner_client = connect(&owner).await;
    open(
        &mut owner_client,
        "telemetry",
        observer_filter(&owner.public_key()),
    )
    .await
    .expect("owner subscribes to its telemetry");

    let result = json!({
        "kind": "control_result",
        "payload": { "type": "cancel_turn", "status": "sent" },
    });
    let telemetry = observer_frame(
        &agent,
        &owner.public_key(),
        &agent.public_key(),
        OBSERVER_FRAME_TELEMETRY,
        &result,
        Timestamp::now(),
    );
    let (status, body) = post_event(&agent, &telemetry).await;
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(body["accepted"], true, "{body}");
    expect_event(&mut owner_client, "telemetry", &telemetry.id).await;

    let unowned = observer_frame(
        &agent,
        &stranger.public_key(),
        &agent.public_key(),
        OBSERVER_FRAME_TELEMETRY,
        &result,
        Timestamp::now(),
    );
    let (status, body) = post_event(&agent, &unowned).await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN, "{body}");

    let stale = observer_frame(
        &agent,
        &owner.public_key(),
        &agent.public_key(),
        OBSERVER_FRAME_TELEMETRY,
        &result,
        Timestamp::from(Timestamp::now().as_secs() - 600),
    );
    let (status, body) = post_event(&agent, &stale).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{body}");

    // The NIP-98 caller must be the frame's author.
    let (status, body) = post_event(&owner, &telemetry).await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN, "{body}");

    // Not stored: an id lookup finds nothing.
    let stored = reqwest::Client::new()
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", owner.public_key().to_hex())
        .json(&json!([{ "ids": [telemetry.id.to_hex()] }]))
        .send()
        .await
        .expect("POST /query");
    assert!(stored.status().is_success(), "{}", stored.status());
    let stored: Value = stored.json().await.expect("query body");
    assert_eq!(stored, json!([]), "observer frames are never stored");

    agent_client.disconnect().await.ok();
    owner_client.disconnect().await.ok();
}
