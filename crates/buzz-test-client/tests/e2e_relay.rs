//! End-to-end integration tests for the Buzz relay.
//!
//! These tests require a running relay instance.  By default they are marked
//! `#[ignore]` so that `cargo test` does not fail in CI when the relay is not
//! available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_relay -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_relay -- --ignored
//! ```

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn test_owner_keys() -> Keys {
    std::env::var("BUZZ_TEST_OWNER_PRIVATE_KEY")
        .ok()
        .and_then(|secret| Keys::parse(&secret).ok())
        .unwrap_or_else(Keys::generate)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn nip98_post_header(keys: &Keys, url: &str, body: &str) -> String {
    let event = EventBuilder::new(Kind::Custom(27_235), "")
        .tags(vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &sha256_hex(body.as_bytes())]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .expect("sign NIP-98 event");
    format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_string(&event).expect("serialize NIP-98 event"))
    )
}

async fn e2e_db_pool() -> sqlx::Pool<sqlx::Postgres> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string() // sadscan:disable np.postgres.1
    });
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect to e2e Postgres")
}

async fn ensure_test_community(host: &str) -> uuid::Uuid {
    let pool = e2e_db_pool().await;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO communities (id, host) \
         VALUES ($1, $2) \
         ON CONFLICT (lower(host)) DO NOTHING",
    )
    .bind(id)
    .bind(host)
    .execute(&pool)
    .await
    .unwrap_or_else(|e| panic!("seed community {host}: {e}"));

    sqlx::query_scalar("SELECT id FROM communities WHERE lower(host) = lower($1)")
        .bind(host)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("lookup community {host}: {e}"))
}

async fn seed_relay_member(host: &str, keys: &Keys, role: &str) {
    let pool = e2e_db_pool().await;
    let community_id = ensure_test_community(host).await;
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET role = $3, updated_at = now()",
    )
    .bind(community_id)
    .bind(keys.public_key().to_hex())
    .bind(role)
    .execute(&pool)
    .await
    .unwrap_or_else(|e| panic!("seed relay member {role}: {e}"));
}

async fn seed_relay_owner(keys: &Keys) {
    seed_relay_member(&relay_authority(), keys, "owner").await;
}

fn relay_authority() -> String {
    let url = url::Url::parse(&relay_http_url()).expect("relay HTTP URL");
    url[url::Position::BeforeHost..url::Position::AfterPort].to_string()
}

fn http_origin_for_host(host: &str) -> String {
    let scheme = if relay_http_url().starts_with("https://") {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

async fn invite_post(keys: &Keys, path: &str, body: &str) -> reqwest::Response {
    invite_post_with_host(keys, None, path, body).await
}

async fn invite_post_with_host(
    keys: &Keys,
    host: Option<&str>,
    path: &str,
    body: &str,
) -> reqwest::Response {
    let client = reqwest::Client::new();
    let connection_url = format!("{}{}", relay_http_url(), path);
    let signed_url = host
        .map(|host| format!("{}{}", http_origin_for_host(host), path))
        .unwrap_or_else(|| connection_url.clone());
    let mut request = client
        .post(&connection_url)
        .header("Authorization", nip98_post_header(keys, &signed_url, body))
        .header("Content-Type", "application/json");
    if let Some(host) = host {
        request = request.header(reqwest::header::HOST, host);
    }
    request
        .body(body.to_string())
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {path} failed: {e}"))
}

/// Create a real channel via a signed kind:9007 event submitted to POST /events.
async fn create_test_channel(keys: &Keys) -> String {
    let client = reqwest::Client::new();
    let pubkey_hex = keys.public_key().to_hex();
    let channel_uuid = uuid::Uuid::new_v4();
    let channel_name = format!("relay-e2e-{}", channel_uuid);

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &channel_name]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit create-channel event");
    assert!(
        resp.status().is_success(),
        "channel creation event failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse event response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "channel creation not accepted: {}",
        body
    );

    channel_uuid.to_string()
}

#[tokio::test]
#[ignore]
async fn test_connect_and_authenticate() {
    let url = relay_url();
    let keys = Keys::generate();

    let client = BuzzTestClient::connect(&url, &keys)
        .await
        .expect("should connect and authenticate");

    client.disconnect().await.expect("clean disconnect");
}

#[tokio::test]
#[ignore]
async fn test_client_submitted_nip43_membership_snapshots_are_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    // Prove this actor can submit a normal event so the rejection below is
    // specifically the relay-only invariant, not a broader authorization failure.
    create_test_channel(&keys).await;
    let forged = EventBuilder::new(Kind::Custom(13_534), "")
        .tags([Tag::parse(["member", &keys.public_key().to_hex(), "owner"]).unwrap()])
        .sign_with_keys(&keys)
        .expect("sign forged membership snapshot");

    let mut ws = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let ok = ws
        .send_event(forged.clone())
        .await
        .expect("submit forged snapshot via websocket");
    assert!(!ok.accepted, "forged WebSocket snapshot must be rejected");
    assert_eq!(ok.message, "restricted: relay-only kind");
    ws.disconnect().await.expect("disconnect");

    let response = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&forged).unwrap())
        .send()
        .await
        .expect("submit forged snapshot via HTTP");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = response.text().await.expect("read HTTP rejection");
    assert!(
        body.contains("restricted: relay-only kind"),
        "unexpected HTTP rejection: {body}"
    );
}

#[tokio::test]
#[ignore]
async fn test_invite_mint_and_claim_admits_new_pubkey() {
    let owner = test_owner_keys();
    let joiner = Keys::generate();
    seed_relay_owner(&owner).await;

    let mint_response = invite_post(&owner, "/api/invites", "{}").await;
    assert_eq!(mint_response.status(), reqwest::StatusCode::OK);
    let mint_json: serde_json::Value = mint_response.json().await.expect("mint JSON");
    let code = mint_json
        .get("code")
        .and_then(serde_json::Value::as_str)
        .expect("mint response includes code");
    assert_eq!(
        mint_json.get("url").and_then(serde_json::Value::as_str),
        Some(format!("{}/invite/{code}", relay_http_url()).as_str()),
        "minted URL should be the shareable HTTPS/HTTP invite URL"
    );

    let claim_body = serde_json::json!({ "code": code }).to_string();
    let claim_response = invite_post(&joiner, "/api/invites/claim", &claim_body).await;
    assert_eq!(claim_response.status(), reqwest::StatusCode::OK);
    let claim_json: serde_json::Value = claim_response.json().await.expect("claim JSON");
    assert_eq!(
        claim_json.get("status").and_then(serde_json::Value::as_str),
        Some("joined")
    );
    assert_eq!(
        claim_json.get("role").and_then(serde_json::Value::as_str),
        Some("member")
    );

    let repeat_response = invite_post(&joiner, "/api/invites/claim", &claim_body).await;
    assert_eq!(repeat_response.status(), reqwest::StatusCode::OK);
    let repeat_json: serde_json::Value = repeat_response.json().await.expect("repeat claim JSON");
    assert_eq!(
        repeat_json
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("already_member")
    );
}

#[tokio::test]
#[ignore]
async fn test_invite_claim_rejects_invalid_code() {
    let joiner = Keys::generate();
    let body = serde_json::json!({ "code": "garbage.code" }).to_string();

    let response = invite_post(&joiner, "/api/invites/claim", &body).await;
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let json: serde_json::Value = response.json().await.expect("error JSON");
    assert_eq!(
        json.get("error").and_then(serde_json::Value::as_str),
        Some("invite_invalid")
    );
}

#[tokio::test]
#[ignore]
async fn test_invite_mint_requires_owner_or_admin() {
    let member = Keys::generate();
    seed_relay_member(&relay_authority(), &member, "member").await;

    let response = invite_post(&member, "/api/invites", "{}").await;
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    let outsider = Keys::generate();
    let response = invite_post(&outsider, "/api/invites", "{}").await;
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore]
async fn test_invite_code_minted_for_one_host_fails_on_another() {
    let host_a = format!("invites-a-{}.example", Uuid::new_v4().simple());
    let host_b = format!("invites-b-{}.example", Uuid::new_v4().simple());
    let owner = Keys::generate();
    let joiner = Keys::generate();
    ensure_test_community(&host_b).await;
    seed_relay_member(&host_a, &owner, "owner").await;

    let mint_response = invite_post_with_host(&owner, Some(&host_a), "/api/invites", "{}").await;
    assert_eq!(mint_response.status(), reqwest::StatusCode::OK);
    let mint_json: serde_json::Value = mint_response.json().await.expect("mint JSON");
    let code = mint_json
        .get("code")
        .and_then(serde_json::Value::as_str)
        .expect("mint response includes code");

    let claim_body = serde_json::json!({ "code": code }).to_string();
    let claim_response =
        invite_post_with_host(&joiner, Some(&host_b), "/api/invites/claim", &claim_body).await;
    assert_eq!(claim_response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore]
async fn test_send_event_and_receive_via_subscription() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let channel = create_test_channel(&keys_a).await;

    let mut client_a = BuzzTestClient::connect(&url, &keys_a)
        .await
        .expect("client A connect");

    let sid = sub_id("send-recv");
    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client_a
        .subscribe(&sid, vec![filter])
        .await
        .expect("client A subscribe");

    // Drain EOSE so we're ready for live events.
    client_a
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("client A EOSE");

    let mut client_b = BuzzTestClient::connect(&url, &keys_b)
        .await
        .expect("client B connect");

    let content = format!("hello from B at {}", uuid::Uuid::new_v4());
    let ok = client_b
        .send_text_message(&keys_b, &channel, &content, kind)
        .await
        .expect("client B send");

    assert!(ok.accepted, "relay rejected event: {}", ok.message);

    let msg = client_a
        .recv_event(Duration::from_secs(5))
        .await
        .expect("client A recv");

    match msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(event.content, content);
            assert_eq!(event.pubkey, keys_b.public_key());
        }
        other => panic!("Expected Event, got {other:?}"),
    }

    client_a.disconnect().await.expect("disconnect A");
    client_b.disconnect().await.expect("disconnect B");
}

#[tokio::test]
#[ignore]
async fn test_large_event_frame_below_configured_limit_is_accepted() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let h_tag = Tag::parse(["h", channel.as_str()]).expect("h tag");
    let content = "x".repeat(70_000);
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags([h_tag])
        .sign_with_keys(&keys)
        .expect("sign large event");

    let frame = serde_json::to_string(&serde_json::json!(["EVENT", &event])).expect("frame JSON");
    assert!(
        frame.len() > 65_536,
        "test frame must exceed the old 64 KiB cap; got {} bytes",
        frame.len()
    );
    assert!(
        frame.len() < 512 * 1024,
        "test frame should fit under the new default cap; got {} bytes",
        frame.len()
    );

    let ok = client.send_event(event).await.expect("send large event");
    assert!(ok.accepted, "large event rejected: {}", ok.message);

    let ok_after = client
        .send_text_message(
            &keys,
            &channel,
            "socket still usable after large frame",
            kind,
        )
        .await
        .expect("send follow-up event");
    assert!(
        ok_after.accepted,
        "follow-up event rejected: {}",
        ok_after.message
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_subscription_filters_by_kind() {
    let url = relay_url();
    let target_kind: u16 = 9;
    let other_kind: u16 = 40002;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("filter-kind");
    let filter = Filter::new()
        .kind(Kind::Custom(target_kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");

    // Send one matching event and one non-matching event.
    let ok_match = client
        .send_text_message(&keys, &channel, "should arrive", target_kind)
        .await
        .expect("send matching");
    assert!(ok_match.accepted, "matching event rejected");

    let ok_other = client
        .send_text_message(&keys, &channel, "should not arrive", other_kind)
        .await
        .expect("send non-matching");
    assert!(ok_other.accepted, "non-matching event rejected");

    // We should receive exactly the matching event.
    let msg = client
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv event");

    match msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(event.content, "should arrive");
            assert_eq!(event.kind, Kind::Custom(target_kind));
        }
        other => panic!("Expected Event, got {other:?}"),
    }

    // No second event should arrive within a short timeout.
    let result = client.recv_event(Duration::from_millis(500)).await;
    match result {
        Err(TestClientError::Timeout) => { /* expected */ }
        Ok(RelayMessage::Event { event, .. }) => {
            panic!("Received unexpected event: kind={}", event.kind.as_u16());
        }
        Ok(other) => {
            // EOSE or NOTICE are fine to receive here.
            let _ = other;
        }
        Err(e) => panic!("Unexpected error: {e}"),
    }

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_close_subscription_stops_delivery() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("close-sub");
    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");

    client
        .close_subscription(&sid)
        .await
        .expect("close subscription");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let ok = client
        .send_text_message(&keys, &channel, "after close", kind)
        .await
        .expect("send");
    assert!(ok.accepted, "event rejected: {}", ok.message);

    let result = client.recv_event(Duration::from_millis(500)).await;
    match result {
        Err(TestClientError::Timeout) => { /* expected — no delivery */ }
        Ok(RelayMessage::Event { event, .. }) => {
            panic!(
                "Received event after subscription closed: {}",
                event.content
            );
        }
        Ok(_) => { /* NOTICE etc. are fine */ }
        Err(e) => panic!("Unexpected error: {e}"),
    }

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_unauthenticated_rejected() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect_unauthenticated(&url)
        .await
        .expect("connect unauthenticated");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = client
        .send_text_message(&keys, "some-channel", "unauthenticated message", 9)
        .await;

    match result {
        Ok(ok) => {
            // Relay may accept the send but reject with OK false.
            assert!(
                !ok.accepted,
                "Relay accepted unauthenticated event — expected rejection"
            );
        }
        Err(TestClientError::ConnectionClosed) => {
            // Relay closed the connection — also acceptable.
        }
        Err(TestClientError::Timeout) => {
            // Relay may not respond at all to unauthenticated clients.
            // This is acceptable behaviour.
        }
        Err(e) => panic!("Unexpected error: {e}"),
    }

    let _ = client.disconnect().await;
}

#[tokio::test]
#[ignore]
async fn test_multiple_concurrent_clients() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys: Vec<Keys> = (0..3).map(|_| Keys::generate()).collect();
    let channel = create_test_channel(&keys[0]).await;

    let mut clients: Vec<BuzzTestClient> =
        futures_util::future::try_join_all(keys.iter().map(|k| BuzzTestClient::connect(&url, k)))
            .await
            .expect("all clients connect");

    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    for (i, client) in clients.iter_mut().enumerate() {
        let sid = format!("multi-{i}");
        client
            .subscribe(&sid, vec![filter.clone()])
            .await
            .expect("subscribe");
        client
            .collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("EOSE");
    }

    let content = format!("broadcast-{}", uuid::Uuid::new_v4());
    let ok = clients[0]
        .send_text_message(&keys[0], &channel, &content, kind)
        .await
        .expect("send");
    assert!(ok.accepted, "event rejected: {}", ok.message);

    for (i, client) in clients.iter_mut().enumerate() {
        let msg = client
            .recv_event(Duration::from_secs(5))
            .await
            .unwrap_or_else(|e| panic!("client {i} recv failed: {e}"));

        match msg {
            RelayMessage::Event { event, .. } => {
                assert_eq!(event.content, content, "client {i} received wrong content");
            }
            other => panic!("client {i}: expected Event, got {other:?}"),
        }
    }

    for client in clients {
        client.disconnect().await.expect("disconnect");
    }
}

/// Historical events must be returned before EOSE.
#[tokio::test]
#[ignore]
async fn test_stored_events_returned_before_eose() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let content = format!("stored-{}", uuid::Uuid::new_v4());
    let ok = client
        .send_text_message(&keys, &channel, &content, kind)
        .await
        .expect("send");
    assert!(ok.accepted, "event rejected: {}", ok.message);

    let sid = sub_id("stored");
    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect until EOSE");

    let found = events.iter().any(|e| e.content == content);
    assert!(
        found,
        "Stored event not returned before EOSE. Got: {events:?}"
    );

    client.disconnect().await.expect("disconnect");
}

/// An explicit `#h` branch that cannot match must not cancel a valid OR sibling.
/// The valid channel remains usable for historical delivery and live fan-out;
/// malformed-only requests still close because no authorized UUID survives.
#[tokio::test]
#[ignore]
async fn test_valid_channel_survives_malformed_or_empty_h_sibling() {
    let url = relay_url();
    let kind: u16 = 9;
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    for (label, sibling) in [
        (
            "malformed",
            serde_json::json!({"kinds": [kind], "#h": ["not-a-uuid"]}),
        ),
        ("empty", serde_json::json!({"kinds": [kind], "#h": []})),
    ] {
        let historical = format!("{label}-historical-{}", Uuid::new_v4());
        let ok = client
            .send_text_message(&keys, &channel, &historical, kind)
            .await
            .expect("send historical event");
        assert!(ok.accepted, "historical event rejected: {}", ok.message);

        let valid = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);
        let sibling: Filter = serde_json::from_value(sibling).expect("parse sibling filter");
        let sid = sub_id(label);
        client
            .subscribe(&sid, vec![valid, sibling])
            .await
            .expect("subscribe");

        let events = client
            .collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("valid sibling history followed by EOSE");
        assert!(
            events.iter().any(|event| event.content == historical),
            "valid sibling history missing for {label} #h branch: {events:?}",
        );

        let live = format!("{label}-live-{}", Uuid::new_v4());
        let ok = client
            .send_text_message(&keys, &channel, &live, kind)
            .await
            .expect("send live event");
        assert!(ok.accepted, "live event rejected: {}", ok.message);
        let message = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("receive post-EOSE live event");
        match message {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                assert_eq!(subscription_id, sid);
                assert_eq!(event.content, live);
            }
            other => panic!("expected live EVENT for {label} sibling, got {other:?}"),
        }

        client
            .close_subscription(&sid)
            .await
            .expect("close subscription");
    }

    client.disconnect().await.expect("disconnect");
}

/// Ephemeral events (kind 20000–29999) must be accepted but not persisted.
#[tokio::test]
#[ignore]
async fn test_ephemeral_event_not_stored() {
    let url = relay_url();
    let ephemeral_kind: u16 = 20001;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let ok = client
        .send_text_message(&keys, &channel, "ephemeral content", ephemeral_kind)
        .await
        .expect("send ephemeral");
    assert!(
        ok.accepted,
        "relay rejected ephemeral event: {}",
        ok.message
    );

    let sid = sub_id("ephemeral");
    let filter = Filter::new()
        .kind(Kind::Custom(ephemeral_kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect until EOSE");

    assert!(
        events.is_empty(),
        "Ephemeral event must not be stored. Got: {events:?}"
    );

    client.disconnect().await.expect("disconnect");
}

/// Kind-22242 AUTH events submitted via EVENT must be rejected.
#[tokio::test]
#[ignore]
async fn test_auth_event_kind_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let relay_url_parsed: nostr::RelayUrl = url.parse().unwrap();
    let auth_event = nostr::EventBuilder::auth("fake-challenge", relay_url_parsed)
        .sign_with_keys(&keys)
        .expect("sign");

    let ok = client.send_event(auth_event).await.expect("send");

    assert!(
        !ok.accepted,
        "Relay must reject kind-22242 submitted as EVENT"
    );
    let msg_lower = ok.message.to_lowercase();
    assert!(
        msg_lower.contains("invalid") || msg_lower.contains("auth"),
        "Rejection message should mention 'invalid' or 'auth', got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-11 max_subscriptions must be enforced; (limit+1)th REQ gets CLOSED.
///
/// This is a protocol-cap test, not an admission-throughput test. Open one REQ
/// at a time and wait out any shared fixed-window quota before retrying a REQ
/// rejected specifically as `rate-limited`, so production admission remains
/// enabled while the test deterministically reaches the independent 1024 cap.
#[tokio::test]
#[ignore]
async fn test_subscription_limit_enforced() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    for i in 0..1024 {
        let sid = format!("limit-sub-{i}");
        let filter = Filter::new().kind(Kind::Custom(49_999));
        subscribe_until_eose(&mut client, &sid, filter).await;
    }

    let overflow_sid = sub_id("overflow");
    let filter = Filter::new().kind(Kind::Custom(49_999));
    loop {
        client
            .subscribe(&overflow_sid, vec![filter.clone()])
            .await
            .expect("send overflow REQ");

        match client
            .recv_event(Duration::from_secs(6))
            .await
            .expect("recv overflow CLOSED")
        {
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == overflow_sid && message.starts_with("rate-limited:") => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, overflow_sid);
                assert!(
                    message.to_lowercase().contains("too many"),
                    "Expected 'too many' in CLOSED message, got: {message}"
                );
                break;
            }
            other => panic!("Expected CLOSED for overflow subscription, got {other:?}"),
        }
    }

    client.disconnect().await.expect("disconnect");
}

async fn subscribe_until_eose(client: &mut BuzzTestClient, sid: &str, filter: Filter) {
    loop {
        client
            .subscribe(sid, vec![filter.clone()])
            .await
            .expect("subscribe");
        match client
            .recv_event(Duration::from_secs(6))
            .await
            .expect("EOSE or rate-limit CLOSED")
        {
            RelayMessage::Eose { subscription_id } => {
                assert_eq!(subscription_id, sid);
                return;
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sid && message.starts_with("rate-limited:") => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            other => panic!("unexpected response while opening {sid}: {other:?}"),
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_nip11_relay_info() {
    let ws_url = relay_url();
    let http_url = ws_url
        .replace("ws://", "http://")
        .replace("wss://", "https://");
    let info_url = format!("{http_url}/info");

    let client = reqwest::Client::new();
    let resp = client
        .get(&info_url)
        .send()
        .await
        .expect("HTTP GET /info failed");

    assert!(
        resp.status().is_success(),
        "GET /info returned {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("response is not valid JSON");

    assert!(body.get("name").is_some(), "Missing 'name' field");
    assert!(
        body.get("description").is_some(),
        "Missing 'description' field"
    );
    assert!(
        body.get("supported_nips").is_some(),
        "Missing 'supported_nips' field"
    );
    assert!(body.get("version").is_some(), "Missing 'version' field");

    let limitation = body.get("limitation").expect("Missing 'limitation' field");
    assert_eq!(
        limitation.get("max_subscriptions").and_then(|v| v.as_u64()),
        Some(1024),
        "limitation.max_subscriptions must be 1024"
    );
    // The REQ, EVENT, and COUNT handlers unconditionally require an
    // authenticated connection, so the NIP-11 doc must advertise that.
    assert_eq!(
        limitation.get("auth_required").and_then(|v| v.as_bool()),
        Some(true),
        "limitation.auth_required must be true — REQ/EVENT/COUNT require NIP-42 auth"
    );
}

/// Events signed by a key other than the authenticated pubkey must be rejected.
#[tokio::test]
#[ignore]
async fn test_pubkey_mismatch_rejected() {
    let url = relay_url();

    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let channel = create_test_channel(&keys_a).await;

    let mut client = BuzzTestClient::connect(&url, &keys_a)
        .await
        .expect("connect as keys_a");

    let ok = client
        .send_text_message(&keys_b, &channel, "impersonation attempt", 9)
        .await
        .expect("send");

    assert!(
        !ok.accepted,
        "Relay must reject event signed by a different key than the authenticated pubkey"
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_eose_sent_for_empty_subscription() {
    let url = relay_url();
    let kind: u16 = 9;

    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("empty-eose");
    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()])
        .since(nostr::Timestamp::now());

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect until EOSE");

    // There should be no stored events (we just created this channel).
    assert!(
        events.is_empty(),
        "Expected no stored events, got: {events:?}"
    );

    client.disconnect().await.expect("disconnect");
}

/// Kind:0 NIP-05 sync regression test.
///
/// Verifies:
/// 1. A valid `nip05` in kind:0 content is synced to the profile and resolvable via NIP-05 endpoint.
/// 2. An off-domain `nip05` in kind:0 content is NOT synced (handle is cleared).
#[tokio::test]
#[ignore]
async fn test_kind0_nip05_sync() {
    let url = relay_url();
    let http = relay_http_url();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // Extract the relay domain from the relay URL for building a valid NIP-05 handle.
    // e.g. "ws://localhost:3000" → "localhost"
    let relay_domain = url
        .trim_start_matches("wss://")
        .trim_start_matches("ws://")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .split('/')
        .next()
        .unwrap_or("localhost")
        .to_lowercase();

    let unique_name = format!("kind0test{}", &pubkey_hex[..8]);
    let valid_handle = format!("{}@{}", unique_name, relay_domain);

    // Step 1: Connect and publish kind:0 with a valid nip05 handle.
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let kind0_content = serde_json::json!({
        "display_name": "Kind0 Test User",
        "nip05": valid_handle,
    })
    .to_string();

    let event = nostr::EventBuilder::new(Kind::Custom(0), kind0_content)
        .tags([])
        .sign_with_keys(&keys)
        .expect("sign kind:0");

    let ok = client.send_event(event).await.expect("send kind:0");
    assert!(
        ok.accepted,
        "kind:0 event should be accepted: {:?}",
        ok.message
    );

    // Give the relay a moment to process the side effect.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Step 2: Verify the kind:0 content was stored via POST /query.
    let http_client = reqwest::Client::new();
    let filters = serde_json::json!([{
        "kinds": [0],
        "authors": [&pubkey_hex],
        "limit": 1,
    }]);
    let profile_resp = http_client
        .post(format!("{}/query", http))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&filters).unwrap())
        .send()
        .await
        .expect("query kind:0");
    assert!(
        profile_resp.status().is_success(),
        "kind:0 query failed: {}",
        profile_resp.status()
    );
    let events: Vec<serde_json::Value> = profile_resp.json().await.expect("kind:0 json");
    assert!(
        !events.is_empty(),
        "kind:0 event should exist after publishing"
    );
    let kind0_stored: serde_json::Value =
        serde_json::from_str(events[0]["content"].as_str().unwrap_or("{}"))
            .expect("parse kind:0 content");
    assert_eq!(
        kind0_stored["nip05"].as_str(),
        Some(valid_handle.as_str()),
        "nip05 should be stored in kind:0 content"
    );

    // Step 3: Verify NIP-05 resolves via /.well-known/nostr.json.
    let nip05_resp = http_client
        .get(format!(
            "{}/.well-known/nostr.json?name={}",
            http, unique_name
        ))
        .send()
        .await
        .expect("GET nostr.json");
    assert_eq!(nip05_resp.status(), 200);
    let nip05_body: serde_json::Value = nip05_resp.json().await.expect("nip05 json");
    let resolved_pubkey = nip05_body["names"][&unique_name].as_str();
    assert_eq!(
        resolved_pubkey,
        Some(pubkey_hex.as_str()),
        "NIP-05 should resolve the pubkey after kind:0 sync"
    );

    // Step 4: Publish another kind:0 with an off-domain nip05 (should be cleared).
    // Sleep to ensure a strictly newer created_at (second-level granularity).
    tokio::time::sleep(Duration::from_secs(1)).await;
    let off_domain_content = serde_json::json!({
        "display_name": "Kind0 Test User",
        "nip05": format!("{}@evil.com", unique_name),
    })
    .to_string();

    let event2 = nostr::EventBuilder::new(Kind::Custom(0), off_domain_content)
        .tags([])
        .sign_with_keys(&keys)
        .expect("sign kind:0 off-domain");

    let ok2 = client
        .send_event(event2)
        .await
        .expect("send kind:0 off-domain");
    assert!(
        ok2.accepted,
        "off-domain kind:0 should still be accepted (stored but handle cleared)"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Step 5: Verify the handle was CLEARED — NIP-05 should no longer resolve
    // after the off-domain kind:0 was accepted. The relay's side effect clears
    // the nip05_handle in the users table when the domain doesn't match.
    let nip05_resp2 = http_client
        .get(format!(
            "{}/.well-known/nostr.json?name={}",
            http, unique_name
        ))
        .send()
        .await
        .expect("GET nostr.json after clear");
    let nip05_body2: serde_json::Value = nip05_resp2.json().await.expect("nip05 json");
    assert!(
        nip05_body2["names"][&unique_name].is_null(),
        "NIP-05 should not resolve after handle was cleared"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-29 kind 9000 (PUT_USER): default policy ("anyone") allows a third party to add an agent.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_default_policy_allows() {
    let url = relay_url();

    let channel_owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Create a channel owned by channel_owner.
    let channel_id = create_test_channel(&channel_owner_keys).await;

    // Connect as channel_owner.
    let mut ws = BuzzTestClient::connect(&url, &channel_owner_keys)
        .await
        .expect("connect as channel_owner");

    // Build kind 9000 PUT_USER event: h = channel_id, p = agent pubkey.
    let h_tag = nostr::Tag::parse(["h", &channel_id]).expect("h tag");
    let p_tag = nostr::Tag::parse(["p", &agent_pubkey_hex]).expect("p tag");
    let event = nostr::EventBuilder::new(Kind::Custom(9000), "")
        .tags([h_tag, p_tag])
        .sign_with_keys(&channel_owner_keys)
        .expect("sign kind 9000");

    let ok = ws.send_event(event).await.expect("send kind 9000");

    assert!(
        ok.accepted,
        "default policy should allow PUT_USER, got: {}",
        ok.message
    );

    ws.disconnect().await.expect("disconnect");
}

/// Restoring an archived channel must re-signal connected members so their
/// agents resubscribe. Archive evicts live channel subscriptions; unarchive
/// emits a kind:44100 member_added notification per member on the always-live
/// global membership feed, which is the resubscribe trigger remove/re-add uses.
#[tokio::test]
#[ignore]
async fn test_unarchive_emits_member_added_notification() {
    let url = relay_url();

    let owner_keys = Keys::generate();
    let owner_pubkey_hex = owner_keys.public_key().to_hex();

    // Creating the channel makes the owner its sole member.
    let channel_id = create_test_channel(&owner_keys).await;

    let mut ws = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");

    // Subscribe to the global membership feed (kind:44100 addressed to the owner).
    // This is a global, non-channel-scoped subscription, so archive's
    // channel-scoped eviction leaves it intact across the archive→unarchive cycle.
    let sid = sub_id("membership-feed");
    let membership_filter = Filter::new().kind(Kind::Custom(44100)).custom_tags(
        SingleLetterTag::lowercase(Alphabet::P),
        [owner_pubkey_hex.as_str()],
    );
    ws.subscribe(&sid, vec![membership_filter])
        .await
        .expect("subscribe to membership feed");
    ws.collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("membership feed EOSE");

    // Archive, then unarchive, the channel via kind:9002 edit-metadata.
    for archived in ["true", "false"] {
        let event = EventBuilder::new(Kind::Custom(9002), "")
            .tags([
                Tag::parse(["h", &channel_id]).unwrap(),
                Tag::parse(["archived", archived]).unwrap(),
            ])
            .sign_with_keys(&owner_keys)
            .unwrap();
        let ok = ws.send_event(event).await.expect("send kind 9002");
        assert!(ok.accepted, "edit-metadata rejected: {}", ok.message);
    }

    // The unarchive must fan out a 44100 to the owner. Loop past any other
    // events delivered on the connection until we see it (or time out).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .filter(|d| !d.is_zero())
            .expect("timed out waiting for member_added notification");
        if let RelayMessage::Event { event, .. } = ws
            .recv_event(remaining)
            .await
            .expect("recv membership notification")
        {
            if event.kind == Kind::Custom(44100) {
                let content: serde_json::Value =
                    serde_json::from_str(&event.content).expect("parse notification content");
                assert_eq!(content["type"], "member_added");
                assert_eq!(content["channel_id"], channel_id);
                break;
            }
        }
    }

    ws.disconnect().await.expect("disconnect");
}

/// NIP-29 kind 9000 (PUT_USER): "nobody" policy blocks a third party from adding the agent.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_nobody_blocks() {
    let url = relay_url();

    let channel_owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Set agent's channel_add_policy to "nobody" via kind:10100 event.
    let http_client = reqwest::Client::new();
    let policy_event = EventBuilder::new(
        Kind::Custom(10100),
        serde_json::json!({ "channel_add_policy": "nobody" }).to_string(),
    )
    .sign_with_keys(&agent_keys)
    .expect("sign kind:10100");
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &agent_pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&policy_event).unwrap())
        .send()
        .await
        .expect("set policy request");
    assert!(
        resp.status().is_success(),
        "set policy failed: {}",
        resp.status()
    );

    // Create a channel owned by channel_owner (not the agent).
    let channel_id = create_test_channel(&channel_owner_keys).await;

    // Connect as channel_owner.
    let mut ws = BuzzTestClient::connect(&url, &channel_owner_keys)
        .await
        .expect("connect as channel_owner");

    // Build kind 9000 PUT_USER event targeting the agent.
    let h_tag = nostr::Tag::parse(["h", &channel_id]).expect("h tag");
    let p_tag = nostr::Tag::parse(["p", &agent_pubkey_hex]).expect("p tag");
    let event = nostr::EventBuilder::new(Kind::Custom(9000), "")
        .tags([h_tag, p_tag])
        .sign_with_keys(&channel_owner_keys)
        .expect("sign kind 9000");

    let ok = ws.send_event(event).await.expect("send kind 9000");

    assert!(
        !ok.accepted,
        "nobody policy should block PUT_USER, but relay accepted it"
    );
    assert!(
        ok.message.contains("policy:nobody"),
        "rejection message should contain 'policy:nobody', got: {}",
        ok.message
    );

    ws.disconnect().await.expect("disconnect");
}

/// NIP-29 kind 9000 (PUT_USER) naming several people over the wire: each is
/// added on its own, a refused agent does not refuse the others, and the OK
/// message reports every result.
#[tokio::test]
#[ignore]
async fn test_nip29_batch_put_user_wire() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let (person, bot, refusing) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (person_hex, bot_hex, refusing_hex) = (
        person.public_key().to_hex(),
        bot.public_key().to_hex(),
        refusing.public_key().to_hex(),
    );

    let policy_event = EventBuilder::new(
        Kind::Custom(10100),
        serde_json::json!({ "channel_add_policy": "nobody" }).to_string(),
    )
    .sign_with_keys(&refusing)
    .expect("sign kind:10100");
    let resp = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &refusing_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&policy_event).unwrap())
        .send()
        .await
        .expect("set policy request");
    assert!(
        resp.status().is_success(),
        "set policy failed: {}",
        resp.status()
    );

    let channel_id = create_test_channel(&owner_keys).await;
    let mut ws = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let put_user = |p_tags: &[&[&str]]| {
        let mut tags = vec![Tag::parse(["h", &channel_id]).unwrap()];
        for tag in p_tags {
            tags.push(Tag::parse(["p"].iter().chain(tag.iter()).copied()).unwrap());
        }
        EventBuilder::new(Kind::Custom(9000), "")
            .tags(tags)
            .sign_with_keys(&owner_keys)
            .expect("sign kind 9000")
    };
    let report = |message: &str| -> serde_json::Value {
        serde_json::from_str(message.strip_prefix("members: ").expect("members report"))
            .expect("members report JSON")
    };

    let ok = ws
        .send_event(put_user(&[
            &[&person_hex],
            &[&refusing_hex],
            &[&bot_hex, "bot"],
        ]))
        .await
        .expect("send batch kind 9000");
    assert!(ok.accepted, "batch put-user refused: {}", ok.message);
    assert_eq!(
        report(&ok.message),
        serde_json::json!({
            "added": [person_hex, bot_hex],
            "rejected": [{
                "reason": "policy:nobody — this agent has disabled external channel additions",
                "pubkeys": [refusing_hex],
            }],
        })
    );

    let ok = ws
        .send_event(put_user(&[&[&person_hex], &[&bot_hex, "bot"]]))
        .await
        .expect("resend names in a new kind 9000");
    assert!(ok.accepted, "repeat put-user refused: {}", ok.message);
    assert_eq!(
        report(&ok.message),
        serde_json::json!({ "already_member": [person_hex, bot_hex] })
    );

    ws.disconnect().await.expect("disconnect");
}

/// NIP-29 kind 9000 (PUT_USER): self-add bypasses "nobody" policy — an agent can always add itself.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_self_add_bypasses_policy() {
    let url = relay_url();

    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Set agent's channel_add_policy to "nobody" via kind:10100 event.
    let http_client = reqwest::Client::new();
    let policy_event = EventBuilder::new(
        Kind::Custom(10100),
        serde_json::json!({ "channel_add_policy": "nobody" }).to_string(),
    )
    .sign_with_keys(&agent_keys)
    .expect("sign kind:10100");
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &agent_pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&policy_event).unwrap())
        .send()
        .await
        .expect("set policy request");
    assert!(
        resp.status().is_success(),
        "set policy failed: {}",
        resp.status()
    );

    // Create a channel where the agent is the owner.
    let channel_id = create_test_channel(&agent_keys).await;

    // Connect as agent.
    let mut ws = BuzzTestClient::connect(&url, &agent_keys)
        .await
        .expect("connect as agent");

    // Build kind 9000 PUT_USER event where agent targets ITSELF.
    let h_tag = nostr::Tag::parse(["h", &channel_id]).expect("h tag");
    let p_tag = nostr::Tag::parse(["p", &agent_pubkey_hex]).expect("p tag");
    let event = nostr::EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([h_tag, p_tag])
        .sign_with_keys(&agent_keys)
        .expect("sign kind 9000");

    let ok = ws.send_event(event).await.expect("send kind 9000");

    assert!(
        ok.accepted,
        "self-add should bypass nobody policy, got: {}",
        ok.message
    );

    ws.disconnect().await.expect("disconnect");
}

/// NIP-29 kind 9000: `owner_only` policy blocks third-party PUT_USER.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_owner_only_blocks() {
    let url = relay_url();

    let channel_owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Set agent's channel_add_policy to "owner_only" via kind:10100 event.
    let http_client = reqwest::Client::new();
    let policy_event = EventBuilder::new(
        Kind::Custom(10100),
        serde_json::json!({ "channel_add_policy": "owner_only" }).to_string(),
    )
    .sign_with_keys(&agent_keys)
    .expect("sign kind:10100");
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &agent_pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&policy_event).unwrap())
        .send()
        .await
        .expect("set policy request");
    assert!(
        resp.status().is_success(),
        "set policy failed: {}",
        resp.status()
    );

    // Create a channel owned by channel_owner (not the agent).
    let channel_id = create_test_channel(&channel_owner_keys).await;

    // Connect as channel_owner.
    let mut ws = BuzzTestClient::connect(&url, &channel_owner_keys)
        .await
        .expect("connect as channel_owner");

    // Build kind 9000 PUT_USER event targeting the agent.
    let h_tag = nostr::Tag::parse(["h", &channel_id]).expect("h tag");
    let p_tag = nostr::Tag::parse(["p", &agent_pubkey_hex]).expect("p tag");
    let event = nostr::EventBuilder::new(Kind::Custom(9000), "")
        .tags([h_tag, p_tag])
        .sign_with_keys(&channel_owner_keys)
        .expect("sign kind 9000");

    let ok = ws.send_event(event).await.expect("send kind 9000");

    assert!(
        !ok.accepted,
        "owner_only policy should block third-party PUT_USER, but relay accepted it"
    );
    assert!(
        ok.message.contains("policy:owner_only"),
        "rejection message should contain 'policy:owner_only', got: {}",
        ok.message
    );

    ws.disconnect().await.expect("disconnect");
}

/// End-to-end test of the standard NIP-29 client flow:
/// connect, authenticate, discover groups, subscribe, send/receive messages,
/// react, and delete.
#[tokio::test]
#[ignore]
async fn test_nip29_standard_client_flow() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    let mut client = BuzzTestClient::connect(&url, &keys)
        .await
        .expect("connect and authenticate via NIP-42");

    // 1. Query group discovery events (kind:39000)
    //    The channel was just created, so the relay should have emitted a 39000 event.
    let discovery_sid = sub_id("discovery");
    let discovery_filter = Filter::new().kind(Kind::Custom(39000));
    client
        .subscribe(&discovery_sid, vec![discovery_filter])
        .await
        .expect("subscribe to group discovery");
    let discovery_events = client
        .collect_until_eose(&discovery_sid, Duration::from_secs(5))
        .await
        .expect("collect discovery events");

    // Find our channel's 39000 event by checking d tags.
    let our_group = discovery_events.iter().find(|e| {
        e.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "d" && s[1] == channel_id
        })
    });
    assert!(
        our_group.is_some(),
        "should find kind:39000 for our channel among {} events",
        discovery_events.len()
    );

    let group_meta = our_group.unwrap();
    // Verify it has a name tag.
    let has_name = group_meta.tags.iter().any(|t| {
        let s = t.as_slice();
        s.len() >= 2 && s[0] == "name"
    });
    assert!(has_name, "39000 event should have a name tag");

    // 1b. Verify kind:39001 (group admins) was also emitted.
    let admins_sid = sub_id("admins");
    let admins_filter = Filter::new().kind(Kind::Custom(39001));
    client
        .subscribe(&admins_sid, vec![admins_filter])
        .await
        .expect("subscribe to group admins");
    let admins_events = client
        .collect_until_eose(&admins_sid, Duration::from_secs(5))
        .await
        .expect("collect admins events");
    let our_admins = admins_events.iter().find(|e| {
        e.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "d" && s[1] == channel_id
        })
    });
    assert!(
        our_admins.is_some(),
        "should find kind:39001 for our channel among {} events",
        admins_events.len()
    );

    // 1c. Verify kind:39002 (group members) was also emitted.
    let members_sid = sub_id("members");
    let members_filter = Filter::new().kind(Kind::Custom(39002));
    client
        .subscribe(&members_sid, vec![members_filter])
        .await
        .expect("subscribe to group members");
    let members_events = client
        .collect_until_eose(&members_sid, Duration::from_secs(5))
        .await
        .expect("collect members events");
    let our_members = members_events.iter().find(|e| {
        e.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "d" && s[1] == channel_id
        })
    });
    assert!(
        our_members.is_some(),
        "should find kind:39002 for our channel among {} events",
        members_events.len()
    );

    // 2. Subscribe to channel messages (kind:9 + h tag).
    let msg_sid = sub_id("messages");
    let msg_filter = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel_id.as_str());
    client
        .subscribe(&msg_sid, vec![msg_filter])
        .await
        .expect("subscribe to channel messages");
    let _historical = client
        .collect_until_eose(&msg_sid, Duration::from_secs(5))
        .await
        .expect("collect historical messages");

    // 3. Send a kind:9 message with h tag.
    let content = format!("nip29-test-{}", uuid::Uuid::new_v4());
    let ok = client
        .send_text_message(&keys, &channel_id, &content, 9)
        .await
        .expect("send kind:9 message");
    assert!(
        ok.accepted,
        "relay should accept kind:9 with h tag: {}",
        ok.message
    );

    // 4. Receive the message on the subscription and capture the event ID.
    let msg = client
        .recv_event(Duration::from_secs(5))
        .await
        .expect("receive kind:9 event");
    let message_event_id = match msg {
        RelayMessage::Event { ref event, .. } => {
            assert_eq!(event.kind, Kind::Custom(9));
            assert_eq!(event.content, content);
            event.id.to_hex()
        }
        other => panic!("expected EVENT, got: {:?}", other),
    };

    // 5. Send a kind:7 reaction targeting the message.
    let h_tag = Tag::parse(["h", &channel_id]).expect("h tag");
    let e_tag = Tag::parse(["e", &message_event_id]).expect("e tag");
    let reaction_event = EventBuilder::new(Kind::Custom(7), "+")
        .tags([h_tag, e_tag])
        .sign_with_keys(&keys)
        .expect("sign reaction");
    let ok = client
        .send_event(reaction_event)
        .await
        .expect("send reaction");
    assert!(
        ok.accepted,
        "relay should accept kind:7 reaction: {}",
        ok.message
    );

    // 6. Send a kind:5 deletion targeting the message.
    let h_tag2 = Tag::parse(["h", &channel_id]).expect("h tag");
    let e_tag2 = Tag::parse(["e", &message_event_id]).expect("e tag");
    let delete_event = EventBuilder::new(Kind::Custom(5), "test delete")
        .tags([h_tag2, e_tag2])
        .sign_with_keys(&keys)
        .expect("sign deletion");
    let ok = client
        .send_event(delete_event)
        .await
        .expect("send deletion");
    assert!(
        ok.accepted,
        "relay should accept kind:5 deletion: {}",
        ok.message
    );

    // 7. Verify kind:9 without h tag is rejected.
    let no_h_event = EventBuilder::new(Kind::Custom(9), "no h tag")
        .tags([])
        .sign_with_keys(&keys)
        .expect("sign no-h event");
    let ok = client
        .send_event(no_h_event)
        .await
        .expect("send no-h event");
    assert!(!ok.accepted, "relay should reject kind:9 without h tag");

    client.disconnect().await.expect("clean disconnect");
}

/// Client-submitted kind:44100 (member-added notification) must be rejected.
/// Only the relay keypair may sign these events.
#[tokio::test]
#[ignore]
async fn test_membership_notification_kind_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let p_tag = Tag::parse(["p", &keys.public_key().to_hex()]).expect("p tag");
    let h_tag = Tag::parse(["h", &channel_id]).expect("h tag");
    let event = EventBuilder::new(Kind::Custom(44100), "")
        .tags([p_tag, h_tag])
        .sign_with_keys(&keys)
        .expect("sign kind:44100");

    let ok = client.send_event(event).await.expect("send");

    assert!(
        !ok.accepted,
        "relay must reject client-submitted kind:44100, but accepted it"
    );
    let msg_lower = ok.message.to_lowercase();
    assert!(
        msg_lower.contains("relay-signed only")
            || msg_lower.contains("relay signed only")
            || msg_lower.contains("relay"),
        "rejection message should mention relay-signed restriction, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// When a member is added via REST, the relay must emit a kind:44100 notification
/// to any subscriber filtering on `#p` = that member's pubkey.
#[tokio::test]
#[ignore]
async fn test_membership_notification_emitted_on_add() {
    let url = relay_url();

    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Connect as agent — NIP-42 auth establishes the authenticated pubkey.
    let mut agent_client = BuzzTestClient::connect(&url, &agent_keys)
        .await
        .expect("connect as agent");

    // Create a channel owned by owner (not agent).
    let channel_id = create_test_channel(&owner_keys).await;

    // Subscribe to membership notifications for agent's own pubkey.
    let sid = sub_id("membership-notif");
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(44100), Kind::Custom(44101)])
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            agent_pubkey_hex.as_str(),
        )
        .since(nostr::Timestamp::now() - 5u64);

    agent_client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe to membership notifications");

    // Drain EOSE — no historical events expected.
    agent_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE for membership sub");

    // Add agent to the channel via signed kind:9000 event.
    let http_client = reqwest::Client::new();
    let add_event = EventBuilder::new(Kind::Custom(9000), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &agent_pubkey_hex]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner_keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&add_event).unwrap())
        .send()
        .await
        .expect("submit add-member event");
    assert!(
        resp.status().is_success(),
        "add member failed: {}",
        resp.status()
    );

    // Wait for the kind:44100 notification.
    let msg = agent_client
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv kind:44100 notification");

    match msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(
                event.kind,
                Kind::Custom(44100),
                "expected kind:44100, got {}",
                event.kind.as_u16()
            );

            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
                .collect();

            let has_p = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "p" && t[1] == agent_pubkey_hex);
            assert!(
                has_p,
                "kind:44100 missing p tag = agent pubkey. tags: {tags:?}"
            );

            let has_h = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "h" && t[1] == channel_id);
            assert!(
                has_h,
                "kind:44100 missing h tag = channel uuid. tags: {tags:?}"
            );
        }
        other => panic!("expected EVENT kind:44100, got {other:?}"),
    }

    agent_client.disconnect().await.expect("disconnect");
}

/// Subscribing to kind:44100/44101 without a `#p` filter must be rejected with CLOSED.
#[tokio::test]
#[ignore]
async fn test_membership_notification_requires_p_filter() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("no-p-filter");
    let filter = Filter::new().kinds(vec![Kind::Custom(44100), Kind::Custom(44101)]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain until we get the CLOSED for our subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue,
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(
                subscription_id, sid,
                "CLOSED for wrong subscription: {subscription_id}"
            );
            assert!(
                message.to_lowercase().contains("restricted"),
                "expected 'restricted' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// A subscription with NO kinds filter and NO #p filter (wildcard) must be rejected with CLOSED
/// because it can match kind:44100/44101.
#[tokio::test]
#[ignore]
async fn test_membership_notification_wildcard_filter_rejected() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("wildcard-filter");
    // Empty filter — no kinds, no #p — can match kind:44100/44101.
    let filter = Filter::new();

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain until we get the CLOSED for our subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue,
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(
                subscription_id, sid,
                "CLOSED for wrong subscription: {subscription_id}"
            );
            assert!(
                message.to_lowercase().contains("restricted"),
                "expected 'restricted' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// Subscribing to kind:44100/44101 with someone else's `#p` must be rejected with CLOSED.
#[tokio::test]
#[ignore]
async fn test_membership_notification_requires_own_p_filter() {
    let url = relay_url();

    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let keys_b_pubkey_hex = keys_b.public_key().to_hex();

    // Connect as keys_a.
    let mut client = BuzzTestClient::connect(&url, &keys_a)
        .await
        .expect("connect as keys_a");

    let sid = sub_id("wrong-p-filter");
    // Filter uses keys_b's pubkey — not the authenticated pubkey (keys_a).
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(44100), Kind::Custom(44101)])
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            keys_b_pubkey_hex.as_str(),
        );

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain until we get the CLOSED for our subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue,
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(
                subscription_id, sid,
                "CLOSED for wrong subscription: {subscription_id}"
            );
            assert!(
                message.to_lowercase().contains("restricted"),
                "expected 'restricted' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// When a member is removed via REST, the relay must emit a kind:44101 notification
/// to any subscriber filtering on `#p` = that member's pubkey.
#[tokio::test]
#[ignore]
async fn test_membership_notification_emitted_on_remove() {
    let url = relay_url();

    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Connect as agent — NIP-42 auth establishes the authenticated pubkey.
    let mut agent_client = BuzzTestClient::connect(&url, &agent_keys)
        .await
        .expect("connect as agent");

    // Create a channel owned by owner (not agent).
    let channel_id = create_test_channel(&owner_keys).await;

    // Subscribe to membership notifications for agent's own pubkey.
    let sid = sub_id("membership-remove-notif");
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(44100), Kind::Custom(44101)])
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            agent_pubkey_hex.as_str(),
        )
        .since(nostr::Timestamp::now() - 5u64);

    agent_client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe to membership notifications");

    // Drain EOSE — no historical events expected.
    agent_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE for membership sub");

    let http_client = reqwest::Client::new();
    let owner_pubkey_hex = owner_keys.public_key().to_hex();

    // Add agent to the channel via signed kind:9000 event.
    let add_event = EventBuilder::new(Kind::Custom(9000), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &agent_pubkey_hex]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner_pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&add_event).unwrap())
        .send()
        .await
        .expect("submit add-member event");
    assert!(
        resp.status().is_success(),
        "add member failed: {}",
        resp.status()
    );

    // Consume the kind:44100 add notification before waiting for the remove.
    let add_msg = agent_client
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv kind:44100 notification");
    match add_msg {
        RelayMessage::Event { ref event, .. } => {
            assert_eq!(
                event.kind,
                Kind::Custom(44100),
                "expected kind:44100 add notification, got {}",
                event.kind.as_u16()
            );
        }
        other => panic!("expected EVENT kind:44100, got {other:?}"),
    }

    // Remove agent from the channel via signed kind:9001 event.
    let remove_event = EventBuilder::new(Kind::Custom(9001), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &agent_pubkey_hex]).unwrap(),
        ])
        .sign_with_keys(&owner_keys)
        .unwrap();
    let resp = http_client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &owner_pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&remove_event).unwrap())
        .send()
        .await
        .expect("submit remove-member event");
    assert!(
        resp.status().is_success(),
        "remove member failed: {}",
        resp.status()
    );

    // Wait for the kind:44101 remove notification.
    let msg = agent_client
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv kind:44101 notification");

    match msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(
                event.kind,
                Kind::Custom(44101),
                "expected kind:44101, got {}",
                event.kind.as_u16()
            );

            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
                .collect();

            let has_p = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "p" && t[1] == agent_pubkey_hex);
            assert!(
                has_p,
                "kind:44101 missing p tag = agent pubkey. tags: {tags:?}"
            );

            let has_h = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "h" && t[1] == channel_id);
            assert!(
                has_h,
                "kind:44101 missing h tag = channel uuid. tags: {tags:?}"
            );
        }
        other => panic!("expected EVENT kind:44101, got {other:?}"),
    }

    agent_client.disconnect().await.expect("disconnect");
}

/// Subscribing to kind:44100/44101 with `#p` containing BOTH the client's own pubkey AND
/// a victim's pubkey must be rejected with CLOSED. All #p values must match the authenticated
/// pubkey — including the victim's key is not allowed.
#[tokio::test]
#[ignore]
async fn test_membership_notification_multi_p_rejected() {
    let url = relay_url();

    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let keys_a_pubkey_hex = keys_a.public_key().to_hex();
    let keys_b_pubkey_hex = keys_b.public_key().to_hex();

    // Connect as keys_a.
    let mut client = BuzzTestClient::connect(&url, &keys_a)
        .await
        .expect("connect as keys_a");

    let sid = sub_id("multi-p-filter");
    // Filter includes keys_a's own pubkey AND keys_b's (victim) pubkey.
    // The relay must reject this because not all #p values match the authenticated pubkey.
    let filter = Filter::new()
        .kinds(vec![Kind::Custom(44100), Kind::Custom(44101)])
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::P),
            [keys_a_pubkey_hex.as_str(), keys_b_pubkey_hex.as_str()],
        );

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain until we get the CLOSED for our subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue,
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(
                subscription_id, sid,
                "CLOSED for wrong subscription: {subscription_id}"
            );
            assert!(
                message.to_lowercase().contains("restricted"),
                "expected 'restricted' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// A mixed-filter subscription where one filter has `#h` + membership kinds and another
/// filter makes the subscription globally scoped must be rejected with CLOSED.
/// This prevents bypassing the #p requirement via mixed filters.
#[tokio::test]
#[ignore]
async fn test_membership_notification_mixed_filter_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel_id = create_test_channel(&keys).await;

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let sid = sub_id("mixed-filter");
    // Filter 1: has #h + membership kinds (would skip per-filter #h check)
    let filter1 = Filter::new()
        .kinds(vec![Kind::Custom(44100)])
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel_id.as_str());
    // Filter 2: global filter (no #h) — makes the subscription globally scoped.
    // No kinds = wildcard, no #p = should trigger rejection.
    let filter2 = Filter::new().authors(vec![keys.public_key()]);

    client
        .subscribe(&sid, vec![filter1, filter2])
        .await
        .expect("send REQ");

    // Drain until we get the CLOSED for our subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue,
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(
                subscription_id, sid,
                "CLOSED for wrong subscription: {subscription_id}"
            );
            assert!(
                message.to_lowercase().contains("restricted"),
                "expected 'restricted' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// Create a private channel over WebSocket and return the channel UUID.
async fn create_private_channel_ws(client: &mut BuzzTestClient, keys: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4().to_string();
    let channel_name = format!("relay-e2e-private-{}", channel_uuid);

    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid]).unwrap(),
            Tag::parse(["name", &channel_name]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "private"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();

    let ok = client
        .send_event(event)
        .await
        .expect("create private channel");
    assert!(
        ok.accepted,
        "private channel creation failed: {}",
        ok.message
    );
    channel_uuid
}

/// Submit a kind:9000 PUT_USER event over WebSocket.
///
/// `allow_self_tagging` keeps self-targeted adds working: EventBuilder otherwise
/// drops a `p` tag matching the signer (nostr-0.44.3 builder.rs:435-449) and the
/// event fails as "missing p tag" instead of exercising the authority check.
async fn add_member_ws(
    client: &mut BuzzTestClient,
    channel_id: &str,
    target_pubkey_hex: &str,
    signer: &Keys,
) -> (bool, String) {
    let h_tag = Tag::parse(["h", channel_id]).unwrap();
    let p_tag = Tag::parse(["p", target_pubkey_hex]).unwrap();
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([h_tag, p_tag])
        .sign_with_keys(signer)
        .unwrap();

    let ok = client.send_event(event).await.expect("send PUT_USER event");
    (ok.accepted, ok.message)
}

/// Submit a kind:9000 PUT_USER event with a role tag over WebSocket.
///
/// See [`add_member_ws`] for why `allow_self_tagging` is required.
async fn add_member_with_role_ws(
    client: &mut BuzzTestClient,
    channel_id: &str,
    target_pubkey_hex: &str,
    role: &str,
    signer: &Keys,
) -> (bool, String) {
    let h_tag = Tag::parse(["h", channel_id]).unwrap();
    let p_tag = Tag::parse(["p", target_pubkey_hex]).unwrap();
    let role_tag = Tag::parse(["role", role]).unwrap();
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([h_tag, p_tag, role_tag])
        .sign_with_keys(signer)
        .unwrap();

    let ok = client
        .send_event(event)
        .await
        .expect("send PUT_USER event with role");
    (ok.accepted, ok.message)
}

/// Submit a self-targeted NIP-29 departure and return the relay's exact OK
/// frame payload. kind:9001 carries a self `p` tag; kind:9022 does not.
async fn self_departure_ws(url: &str, channel_id: &str, actor: &Keys, kind: u16) -> (bool, String) {
    let h_tag = Tag::parse(["h", channel_id]).unwrap();
    let event = match kind {
        9001 => EventBuilder::new(Kind::Custom(kind), "")
            .allow_self_tagging()
            .tags([
                h_tag,
                Tag::parse(["p", &actor.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(actor)
            .expect("sign kind:9001 self-removal"),
        9022 => EventBuilder::new(Kind::Custom(kind), "")
            .tags([h_tag])
            .sign_with_keys(actor)
            .expect("sign kind:9022 leave request"),
        _ => panic!("unsupported self-departure kind: {kind}"),
    };

    let mut client = BuzzTestClient::connect(url, actor)
        .await
        .expect("connect departure actor");
    let ok = client.send_event(event).await.expect("send self-departure");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

async fn promote_co_owner(url: &str, channel_id: &str, owner: &Keys, co_owner: &Keys) {
    let mut client = BuzzTestClient::connect(url, owner)
        .await
        .expect("connect channel owner");
    let result = add_member_with_role_ws(
        &mut client,
        channel_id,
        &co_owner.public_key().to_hex(),
        "owner",
        owner,
    )
    .await;
    client.disconnect().await.ok();
    assert_eq!(result, (true, String::new()), "promote co-owner OK frame");
    assert_eq!(
        member_role(url, owner, channel_id, &co_owner.public_key().to_hex())
            .await
            .as_deref(),
        Some("owner"),
        "the setup must leave a second active owner"
    );
}

/// Binds kind:9001's production `validate_admin_event` call to the WebSocket
/// OK frame. The DB applier has a different rejection message, so this exact
/// historical result can only come from the pre-storage relay validator.
#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9001_sole_owner_rejected() {
    let url = relay_url();
    let owner = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let result = self_departure_ws(&url, &channel_id, &owner, 9001).await;

    assert_eq!(
        result,
        (false, "invalid: cannot remove the last owner".to_string())
    );
}

/// An open channel lets the nonmember event reach the per-kind validator; a
/// private channel would be rejected earlier by the generic membership gate.
#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9001_nonmember_rejected() {
    let url = relay_url();
    let owner = Keys::generate();
    let nonmember = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let result = self_departure_ws(&url, &channel_id, &nonmember, 9001).await;

    assert_eq!(
        result,
        (false, "invalid: actor is not an active member".to_string())
    );
}

#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9001_co_owner_allowed() {
    let url = relay_url();
    let owner = Keys::generate();
    let co_owner = Keys::generate();
    let channel_id = create_test_channel(&owner).await;
    promote_co_owner(&url, &channel_id, &owner, &co_owner).await;

    let result = self_departure_ws(&url, &channel_id, &co_owner, 9001).await;

    assert_eq!(result, (true, String::new()));
}

/// Binds kind:9022's distinct production `validate_admin_event` call to the
/// same historical WebSocket rejection contract as self-removal.
#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9022_sole_owner_rejected() {
    let url = relay_url();
    let owner = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let result = self_departure_ws(&url, &channel_id, &owner, 9022).await;

    assert_eq!(
        result,
        (false, "invalid: cannot remove the last owner".to_string())
    );
}

#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9022_nonmember_rejected() {
    let url = relay_url();
    let owner = Keys::generate();
    let nonmember = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let result = self_departure_ws(&url, &channel_id, &nonmember, 9022).await;

    assert_eq!(
        result,
        (false, "invalid: actor is not an active member".to_string())
    );
}

#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_kind_9022_co_owner_allowed() {
    let url = relay_url();
    let owner = Keys::generate();
    let co_owner = Keys::generate();
    let channel_id = create_test_channel(&owner).await;
    promote_co_owner(&url, &channel_id, &owner, &co_owner).await;

    let result = self_departure_ws(&url, &channel_id, &co_owner, 9022).await;

    assert_eq!(result, (true, String::new()));
}

/// An archived channel refuses changes, but a member may still leave it by
/// themselves, with either departure kind. The last-owner rule still applies,
/// and removing someone else is still refused.
#[tokio::test]
#[ignore]
async fn test_nip29_departure_wire_archived_channel_allows_self_leave() {
    let url = relay_url();
    let owner = Keys::generate();
    let guest = Keys::generate();
    let member = Keys::generate();
    let other = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect channel owner");
    for (keys, role) in [(&guest, "guest"), (&member, "member"), (&other, "member")] {
        let (accepted, msg) = add_member_with_role_ws(
            &mut owner_client,
            &channel_id,
            &keys.public_key().to_hex(),
            role,
            &owner,
        )
        .await;
        assert!(accepted, "owner adds a {role}: {msg}");
    }
    let archive = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["archived", "true"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let ok = owner_client.send_event(archive).await.expect("archive");
    assert!(ok.accepted, "archive: {}", ok.message);

    // Removing someone else is still a change to an archived channel.
    let remove_other = EventBuilder::new(Kind::Custom(9001), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &other.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let ok = owner_client
        .send_event(remove_other)
        .await
        .expect("remove other");
    assert!(!ok.accepted, "removing another member must stay refused");
    assert!(ok.message.contains("archived"), "{}", ok.message);
    owner_client.disconnect().await.ok();

    assert_eq!(
        self_departure_ws(&url, &channel_id, &guest, 9022).await,
        (true, String::new()),
        "a guest leaves an archived channel"
    );
    assert_eq!(
        self_departure_ws(&url, &channel_id, &member, 9001).await,
        (true, String::new()),
        "a member removes themselves from an archived channel"
    );
    for (keys, who) in [(&guest, "guest"), (&member, "member")] {
        assert_eq!(
            member_role(&url, &owner, &channel_id, &keys.public_key().to_hex()).await,
            None,
            "the departed {who} is off the roster"
        );
    }

    let (accepted, msg) = self_departure_ws(&url, &channel_id, &owner, 9022).await;
    assert!(!accepted, "the last owner must not leave");
    assert!(msg.contains("last owner"), "{msg}");
}

/// Any active member who may write can add any ordinary role to a private
/// channel. A guest cannot add anyone.
#[tokio::test]
#[ignore]
async fn test_private_channel_any_member_can_invite() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let actors = [
        ("member", Keys::generate()),
        ("guest", Keys::generate()),
        ("bot", Keys::generate()),
    ];

    // Connect as owner and create a private channel.
    let mut owner_client = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let channel_id = create_private_channel_ws(&mut owner_client, &owner_keys).await;

    // Seed one actor for each ordinary active role.
    for (role, keys) in &actors {
        let (accepted, msg) = add_member_with_role_ws(
            &mut owner_client,
            &channel_id,
            &keys.public_key().to_hex(),
            role,
            &owner_keys,
        )
        .await;
        assert!(accepted, "owner should add {role} actor, got: {msg}");
    }

    // Exercise the full ordinary-role target matrix. Relay and DB authorization
    // both run here, unlike the Desktop/mobile policy-unit-test mirrors. Any
    // member who may write can add people; a guest cannot.
    for (actor_role, actor_keys) in &actors {
        let mut actor_client = BuzzTestClient::connect(&url, actor_keys)
            .await
            .unwrap_or_else(|err| panic!("connect as {actor_role}: {err}"));

        for target_role in ["member", "guest", "bot"] {
            let target_keys = Keys::generate();
            let target_pubkey_hex = target_keys.public_key().to_hex();
            let (accepted, msg) = add_member_with_role_ws(
                &mut actor_client,
                &channel_id,
                &target_pubkey_hex,
                target_role,
                actor_keys,
            )
            .await;
            // Adding people is a write: a guest may not, in any channel.
            if *actor_role == "guest" {
                assert!(
                    !accepted,
                    "private-channel guest must not add {target_role}"
                );
                assert!(
                    msg.contains("only members can add people"),
                    "private-channel guest add refusal: {msg}"
                );
                assert_eq!(
                    member_role(&url, &owner_keys, &channel_id, &target_pubkey_hex).await,
                    None,
                    "a refused guest add must not store a row"
                );
                continue;
            }
            assert!(
                accepted,
                "private-channel {actor_role} should add {target_role}, got: {msg}"
            );
            assert_eq!(
                member_role(&url, &owner_keys, &channel_id, &target_pubkey_hex).await,
                Some(target_role.to_string()),
                "private-channel {actor_role} add must persist the {target_role} role"
            );
        }

        // Re-adding oneself stays idempotent — the huddle bot-add and kind:9021
        // paths depend on a self-targeted PUT_USER working.
        let (accepted, msg) = add_member_with_role_ws(
            &mut actor_client,
            &channel_id,
            &actor_keys.public_key().to_hex(),
            actor_role,
            actor_keys,
        )
        .await;
        assert!(
            accepted,
            "self-targeted {actor_role} re-add must stay idempotent, got: {msg}"
        );

        actor_client
            .disconnect()
            .await
            .unwrap_or_else(|err| panic!("disconnect {actor_role}: {err}"));
    }

    owner_client.disconnect().await.expect("disconnect owner");
}

/// An admin — not just the owner — can still add to a private channel.
#[tokio::test]
#[ignore]
async fn test_private_channel_admin_can_invite() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let admin_keys = Keys::generate();
    let invitee_keys = Keys::generate();

    let mut owner_client = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let channel_id = create_private_channel_ws(&mut owner_client, &owner_keys).await;

    let (accepted, msg) = add_member_with_role_ws(
        &mut owner_client,
        &channel_id,
        &admin_keys.public_key().to_hex(),
        "admin",
        &owner_keys,
    )
    .await;
    assert!(accepted, "owner should add an admin, got: {msg}");

    let mut admin_client = BuzzTestClient::connect(&url, &admin_keys)
        .await
        .expect("connect as admin");

    let (accepted, msg) = add_member_ws(
        &mut admin_client,
        &channel_id,
        &invitee_keys.public_key().to_hex(),
        &admin_keys,
    )
    .await;
    assert!(
        accepted,
        "admin should be able to add to a private channel, got: {msg}"
    );

    owner_client.disconnect().await.expect("disconnect owner");
    admin_client.disconnect().await.expect("disconnect admin");
}

/// A non-member cannot invite someone to a private channel.
#[tokio::test]
#[ignore]
async fn test_private_channel_non_member_cannot_invite() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let outsider_keys = Keys::generate();
    let target_keys = Keys::generate();

    // Owner creates a private channel.
    let mut owner_client = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let channel_id = create_private_channel_ws(&mut owner_client, &owner_keys).await;

    // Connect as outsider (not a member of the channel).
    let mut outsider_client = BuzzTestClient::connect(&url, &outsider_keys)
        .await
        .expect("connect as outsider");

    // Outsider tries to add someone — should be rejected.
    let (accepted, msg) = add_member_ws(
        &mut outsider_client,
        &channel_id,
        &target_keys.public_key().to_hex(),
        &outsider_keys,
    )
    .await;
    assert!(
        !accepted,
        "non-member should NOT be able to invite to private channel, but it was accepted"
    );
    assert!(
        msg.contains("not authorized") || msg.contains("not a channel member"),
        "rejection should mention authorization or membership, got: {msg}"
    );

    owner_client.disconnect().await.expect("disconnect owner");
    outsider_client
        .disconnect()
        .await
        .expect("disconnect outsider");
}

/// Regular members cannot grant elevated roles (owner/admin) in private channels.
#[tokio::test]
#[ignore]
async fn test_private_channel_member_cannot_grant_admin() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let member_keys = Keys::generate();
    let target_keys = Keys::generate();

    // Owner creates a private channel and adds a regular member.
    let mut owner_client = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let channel_id = create_private_channel_ws(&mut owner_client, &owner_keys).await;

    let (accepted, msg) = add_member_ws(
        &mut owner_client,
        &channel_id,
        &member_keys.public_key().to_hex(),
        &owner_keys,
    )
    .await;
    assert!(accepted, "owner should add member, got: {msg}");

    // Connect as the regular member.
    let mut member_client = BuzzTestClient::connect(&url, &member_keys)
        .await
        .expect("connect as member");

    // Regular member tries to add someone with admin role — should fail.
    let (accepted, msg) = add_member_with_role_ws(
        &mut member_client,
        &channel_id,
        &target_keys.public_key().to_hex(),
        "admin",
        &member_keys,
    )
    .await;
    assert!(
        !accepted,
        "regular member should NOT grant admin role, but it was accepted"
    );
    assert!(
        msg.contains("elevated")
            || msg.contains("owner")
            || msg.contains("admin")
            || msg.contains("grant"),
        "rejection should mention elevated roles, got: {msg}"
    );

    owner_client.disconnect().await.expect("disconnect owner");
    member_client.disconnect().await.expect("disconnect member");
}

/// Live badge counts: every thread mutation pushes a fresh relay-signed
/// kind:39005 recount to channel subscribers — a reply counts up, deleting
/// that reply counts back down — without any window refetch.
#[tokio::test]
#[ignore]
async fn test_reply_ingest_pushes_live_thread_summary() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Root message for the thread — built locally so we keep its id.
    let root = EventBuilder::new(Kind::Custom(9), "thread root")
        .tags([Tag::parse(["h", channel.as_str()]).unwrap()])
        .sign_with_keys(&keys)
        .expect("sign root");
    let root_id = root.id;
    let ok = client.send_event(root).await.expect("send root");
    assert!(ok.accepted, "root rejected: {}", ok.message);

    // Live subscription shaped like the desktop window-store one: channel
    // scope with 39005 in kinds.
    let sid = sub_id("live-summary");
    let filter = Filter::new()
        .kind(Kind::Custom(39005))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");

    async fn recv_summary(client: &mut BuzzTestClient) -> nostr::Event {
        loop {
            match client
                .recv_event(Duration::from_secs(5))
                .await
                .expect("recv 39005")
            {
                RelayMessage::Event { event, .. } if event.kind == Kind::Custom(39005) => {
                    return *event;
                }
                _ => continue,
            }
        }
    }

    // Reply → pushed summary counts up.
    let h_tag = Tag::parse(["h", channel.as_str()]).unwrap();
    let root_tag = Tag::parse(["e", &root_id.to_hex(), "", "reply"]).unwrap();
    let reply = EventBuilder::new(Kind::Custom(9), "first reply")
        .tags([h_tag, root_tag])
        .sign_with_keys(&keys)
        .expect("sign reply");
    let reply_id = reply.id;
    let ok = client.send_event(reply).await.expect("send reply");
    assert!(ok.accepted, "reply rejected: {}", ok.message);

    let summary = recv_summary(&mut client).await;
    let root_tag_val = summary
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some("e"))
        .and_then(|t| t.content().map(str::to_string))
        .expect("summary carries root e-tag");
    assert_eq!(root_tag_val, root_id.to_hex(), "summary targets the root");
    let content: serde_json::Value = serde_json::from_str(&summary.content).expect("JSON");
    assert_eq!(content["reply_count"], 1, "reply counted up: {content}");

    // Delete the reply → pushed summary counts back down.
    let delete = EventBuilder::new(Kind::Custom(5), "")
        .tags([
            Tag::parse(["e", &reply_id.to_hex()]).unwrap(),
            Tag::parse(["h", channel.as_str()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .expect("sign delete");
    let ok = client.send_event(delete).await.expect("send delete");
    assert!(ok.accepted, "delete rejected: {}", ok.message);

    let summary = recv_summary(&mut client).await;
    let content: serde_json::Value = serde_json::from_str(&summary.content).expect("JSON");
    assert_eq!(content["reply_count"], 0, "reply counted down: {content}");

    client.disconnect().await.expect("disconnect");
}

/// F3 (workflow path): a `message_posted` workflow whose `send_message` action
/// has `reply_in_thread: true` posts a threaded reply to the triggering
/// top-level message — and that relay-built reply must push the same live
/// kind:39005 thread-summary overlay the human ingest path does, so desktops
/// update the root's badge without refetching. Also exercises F2's semantics:
/// the `trigger_is_reply == false` filter must fire on the top-level message.
#[tokio::test]
#[ignore]
async fn test_workflow_reply_in_thread_pushes_live_thread_summary() {
    let url = relay_url();
    let http = relay_http_url();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();
    let channel = create_test_channel(&keys).await;

    // A message_posted workflow that replies in-thread, but only to NEW
    // top-level messages (`trigger_is_reply == false`) — so it cannot recurse
    // on the reply it just posted.
    let yaml = "name: reply-bot\n\
         description: F3 live probe\n\
         trigger:\n\
         \x20 on: message_posted\n\
         \x20 filter: \"trigger_is_reply == false\"\n\
         steps:\n\
         \x20 - id: step1\n\
         \x20   name: Reply\n\
         \x20   action: send_message\n\
         \x20   text: \"auto-reply\"\n\
         \x20   reply_in_thread: true\n"
        .to_string();
    let def = EventBuilder::new(Kind::Custom(30620), yaml)
        .tags([
            Tag::parse(["d", &Uuid::new_v4().to_string()]).unwrap(),
            Tag::parse(["h", channel.as_str()]).unwrap(),
            Tag::parse(["name", "reply-bot"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .expect("sign workflow def");
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{http}/events"))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&def).unwrap())
        .send()
        .await
        .expect("submit workflow def");
    let body: serde_json::Value = resp.json().await.expect("parse def response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "workflow def not accepted: {body}"
    );

    // Live 39005 subscription for the channel, shaped like the desktop window
    // store's.
    let mut ws = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let sid = sub_id("wf-live-summary");
    let filter = Filter::new()
        .kind(Kind::Custom(39005))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");
    ws.collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");

    // Post a top-level message — the workflow fires and posts a threaded reply.
    let root = EventBuilder::new(Kind::Custom(9), "trigger me")
        .tags([Tag::parse(["h", channel.as_str()]).unwrap()])
        .sign_with_keys(&keys)
        .expect("sign root");
    let root_id = root.id;
    let ok = ws.send_event(root).await.expect("send root");
    assert!(ok.accepted, "root rejected: {}", ok.message);

    // The workflow reply's 39005 overlay must arrive and target the root with a
    // reply_count of 1 — proving the relay-built reply pushed the live summary.
    let summary = loop {
        match ws
            .recv_event(Duration::from_secs(10))
            .await
            .expect("recv 39005 for workflow reply")
        {
            RelayMessage::Event { event, .. } if event.kind == Kind::Custom(39005) => break *event,
            _ => continue,
        }
    };
    let root_tag_val = summary
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some("e"))
        .and_then(|t| t.content().map(str::to_string))
        .expect("summary carries root e-tag");
    assert_eq!(
        root_tag_val,
        root_id.to_hex(),
        "workflow-reply summary targets the triggering top-level message as root"
    );
    let content: serde_json::Value = serde_json::from_str(&summary.content).expect("JSON");
    assert_eq!(
        content["reply_count"], 1,
        "workflow threaded reply counted up: {content}"
    );

    ws.disconnect().await.expect("disconnect");
}

/// Read a member's authoritative role from the relay-signed kind:39002 member
/// list. The relay's own view of membership, not the client's — a kind:9000 can
/// be `accepted` (stored) while its membership side effect fails, so asserting
/// on the OK alone cannot see a broken write.
async fn member_role(url: &str, keys: &Keys, channel_id: &str, pubkey_hex: &str) -> Option<String> {
    let mut ws = BuzzTestClient::connect(url, keys).await.expect("connect");
    let sid = sub_id("members");
    let filter = Filter::new()
        .kind(Kind::Custom(39002))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [channel_id]);
    ws.subscribe(&sid, vec![filter])
        .await
        .expect("subscribe 39002");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("39002 EOSE");
    ws.disconnect().await.ok();
    // Latest 39002 wins; find this pubkey's tag. Shape is
    // ["p", pubkey, relay_url, role] (side_effects.rs:1058-1062) — role is
    // index 3, not 2.
    events.iter().max_by_key(|e| e.created_at).and_then(|e| {
        e.tags.iter().find_map(|t| {
            let p = t.as_slice();
            (p.len() >= 4 && p[0] == "p" && p[1] == pubkey_hex).then(|| p[3].clone())
        })
    })
}

/// Submit a kind:9002 `join_role` change and return the OK frame.
async fn set_join_role_ws(
    url: &str,
    channel_id: &str,
    signer: &Keys,
    join_role: &str,
) -> (bool, String) {
    // The nonce keeps a repeated setting within one second from signing the
    // same event id as the earlier one, which the relay drops as a duplicate.
    let event = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["join_role", join_role]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(signer)
        .unwrap();
    let mut client = BuzzTestClient::connect(url, signer)
        .await
        .expect("connect join_role actor");
    let ok = client.send_event(event).await.expect("send kind:9002");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

/// The latest kind:39000 tags for a channel.
async fn group_metadata_tags(url: &str, keys: &Keys, channel_id: &str) -> Vec<Vec<String>> {
    let mut ws = BuzzTestClient::connect(url, keys).await.expect("connect");
    let sid = sub_id("metadata");
    let filter = Filter::new()
        .kind(Kind::Custom(39000))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [channel_id]);
    ws.subscribe(&sid, vec![filter])
        .await
        .expect("subscribe 39000");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("39000 EOSE");
    ws.disconnect().await.ok();
    events
        .iter()
        .max_by_key(|e| e.created_at)
        .map(|e| e.tags.iter().map(|t| t.as_slice().to_vec()).collect())
        .unwrap_or_default()
}

/// Send a kind:9021 join and return the OK frame.
async fn join_ws(url: &str, channel_id: &str, signer: &Keys) -> (bool, String) {
    let join = EventBuilder::new(Kind::Custom(9021), "")
        .tags([Tag::parse(["h", channel_id]).unwrap()])
        .sign_with_keys(signer)
        .unwrap();
    let mut client = BuzzTestClient::connect(url, signer)
        .await
        .expect("connect joiner");
    let ok = client.send_event(join).await.expect("send join");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

/// Send a kind:9 post and return the OK frame.
async fn post_ws(url: &str, channel_id: &str, signer: &Keys, text: &str) -> (bool, String) {
    let mut client = BuzzTestClient::connect(url, signer)
        .await
        .expect("connect poster");
    let ok = client
        .send_text_message(signer, channel_id, text, 9)
        .await
        .expect("send post");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

/// Submit a kind:9002 `unlisted` change and return the OK frame.
async fn set_unlisted_ws(
    url: &str,
    channel_id: &str,
    signer: &Keys,
    value: &str,
) -> (bool, String) {
    let event = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["unlisted", value]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(signer)
        .unwrap();
    let mut client = BuzzTestClient::connect(url, signer)
        .await
        .expect("connect unlisted actor");
    let ok = client.send_event(event).await.expect("send kind:9002");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

/// The events one REQ returns for `keys`.
async fn req_events(url: &str, keys: &Keys, filter: Filter) -> Vec<nostr::Event> {
    let mut ws = BuzzTestClient::connect(url, keys).await.expect("connect");
    let sid = sub_id("unlisted");
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");
    ws.disconnect().await.ok();
    events
}

/// The channel ids of the kind:39000 events `keys` sees without naming any
/// channel, since `since`.
async fn listed_channel_ids(url: &str, keys: &Keys, since: nostr::Timestamp) -> Vec<String> {
    req_events(
        url,
        keys,
        Filter::new().kind(Kind::Custom(39000)).since(since),
    )
    .await
    .iter()
    .filter_map(|e| {
        e.tags
            .iter()
            .find(|t| t.kind().to_string() == "d")
            .and_then(|t| t.content().map(str::to_owned))
    })
    .collect()
}

/// An unlisted open channel through the real relay: only owners/admins set
/// it; non-members do not see it in kind:39000 discovery, history or search
/// unless they name it, and then read it; anyone can join it by id and then
/// lists it; listing it again shows it to everyone. A reader whose channel set
/// was cached before a change sees the change at once.
#[tokio::test]
#[ignore]
async fn test_nip29_unlisted_channel_wire() {
    let url = relay_url();
    let owner = Keys::generate();
    let outsider = Keys::generate();
    // 39000 timestamps are whole seconds; start one second early.
    let since = nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 1);
    let listed = create_test_channel(&owner).await;
    let channel_id = create_test_channel(&owner).await;
    // A non-member whose accessible channels are cached while it is listed.
    let stranger = Keys::generate();
    assert!(listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&channel_id));

    // Only owners/admins may change the setting; only true/false are valid.
    let (accepted, msg) = set_unlisted_ws(&url, &channel_id, &outsider, "true").await;
    assert!(!accepted, "a non-member must not change unlisted");
    assert!(msg.contains("not authorized"), "{msg}");
    let (accepted, msg) = set_unlisted_ws(&url, &channel_id, &owner, "yes").await;
    assert!(!accepted, "an unknown unlisted value must be refused");
    assert!(msg.contains("invalid unlisted value"), "{msg}");
    assert_eq!(
        set_unlisted_ws(&url, &channel_id, &owner, "true").await,
        (true, String::new())
    );

    // Named by `#d`, the metadata is readable and carries the flag.
    let tags = group_metadata_tags(&url, &outsider, &channel_id).await;
    assert!(tags.contains(&vec!["unlisted".to_string()]), "{tags:?}");
    assert!(tags.contains(&vec!["public".to_string()]), "{tags:?}");
    assert!(!tags.contains(&vec!["hidden".to_string()]), "{tags:?}");

    // Discovery without `#d`: members list it, non-members do not, including
    // one whose channel set was cached before the change.
    assert!(!listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&channel_id));
    let seen = listed_channel_ids(&url, &outsider, since).await;
    assert!(
        seen.contains(&listed),
        "a listed channel is listed: {seen:?}"
    );
    assert!(!seen.contains(&channel_id), "unlisted to a non-member");
    assert!(listed_channel_ids(&url, &owner, since)
        .await
        .contains(&channel_id));

    // History and search: a non-member reads it only by naming it.
    let token = format!("unlisted{}", uuid::Uuid::new_v4().simple());
    let (accepted, msg) = post_ws(&url, &channel_id, &owner, &format!("hello {token}")).await;
    assert!(accepted, "owner post: {msg}");
    let h = SingleLetterTag::lowercase(Alphabet::H);
    let messages = Filter::new().kind(Kind::Custom(9)).since(since);
    let search = Filter::new().kind(Kind::Custom(9)).search(&token);
    for (filter, what) in [(&messages, "history"), (&search, "search")] {
        let in_channel = |events: &[nostr::Event]| {
            events.iter().any(|e| {
                e.tags.iter().any(|t| {
                    t.kind().to_string() == "h" && t.content() == Some(channel_id.as_str())
                })
            })
        };
        let unnamed = req_events(&url, &outsider, filter.clone()).await;
        assert!(!in_channel(&unnamed), "{what}: unnamed, non-member");
        let named = req_events(
            &url,
            &outsider,
            filter.clone().custom_tags(h, [channel_id.as_str()]),
        )
        .await;
        assert!(in_channel(&named), "{what}: named, non-member");
        let member = req_events(&url, &owner, filter.clone()).await;
        assert!(in_channel(&member), "{what}: unnamed, member");
    }

    // Anyone can join by id, and then lists it like any member.
    let (accepted, msg) = join_ws(&url, &channel_id, &outsider).await;
    assert!(accepted, "join an unlisted channel: {msg}");
    assert!(listed_channel_ids(&url, &outsider, since)
        .await
        .contains(&channel_id));

    // Listing it again shows it to everyone, at once.
    assert!(!listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&channel_id));
    assert_eq!(
        set_unlisted_ws(&url, &channel_id, &owner, "false").await,
        (true, String::new())
    );
    let tags = group_metadata_tags(&url, &stranger, &channel_id).await;
    assert!(!tags.contains(&vec!["unlisted".to_string()]), "{tags:?}");
    assert!(listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&channel_id));
}

/// Submit one signed event over WebSocket and return the OK frame.
async fn submit_ws(url: &str, signer: &Keys, kind: u16, tags: &[&[&str]]) -> (bool, String) {
    let mut tags: Vec<Tag> = tags
        .iter()
        .map(|t| Tag::parse(t.iter().copied()).unwrap())
        .collect();
    tags.push(Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap());
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(signer)
        .unwrap();
    let mut client = BuzzTestClient::connect(url, signer)
        .await
        .expect("connect submitter");
    let ok = client.send_event(event).await.expect("send event");
    client.disconnect().await.ok();
    (ok.accepted, ok.message)
}

/// Create a private Solo-style chat (a private stream labelled `buzz.solo`)
/// with kind:9007, adding `extra` tags, and return its id.
async fn create_solo_chat_ws(url: &str, owner: &Keys, extra: &[&[&str]]) -> String {
    let channel_id = uuid::Uuid::new_v4().to_string();
    let name = format!("solo-e2e-{channel_id}");
    let base: [&[&str]; 5] = [
        &["h", &channel_id],
        &["name", &name],
        &["channel_type", "stream"],
        &["visibility", "private"],
        &["t", "buzz.solo"],
    ];
    let tags: Vec<&[&str]> = base.into_iter().chain(extra.iter().copied()).collect();
    let (accepted, msg) = submit_ws(url, owner, 9007, &tags).await;
    assert!(accepted, "create Solo chat: {msg}");
    channel_id
}

/// Whether any event in `events` belongs to `channel_id`.
fn any_in_channel(events: &[nostr::Event], channel_id: &str) -> bool {
    events.iter().any(|e| {
        e.tags
            .iter()
            .any(|t| t.kind().to_string() == "h" && t.content() == Some(channel_id))
    })
}

/// The Solo sharing model through the real relay, one event per user action:
///
/// - **Create unlisted:** kind:9007 `["unlisted", "true"]` starts a private
///   chat unlisted, and a later plain `["visibility", "open"]` keeps it
///   unlisted.
/// - **Share:** one kind:9002 with `visibility open` + `unlisted true` +
///   `join_role guest`. A non-member does not list it, reads it by `#h`
///   (history and live), and cannot post. A kind:9021 join (pin) lists it for
///   them, still as a read-only guest; a kind:9022 leave (unpin) unlists it
///   again and it stays readable by `#h`.
/// - **Unshare:** kind:9002 `visibility private` closes the non-member's live
///   subscription and refuses a new `#h` read.
/// - **Convert to a channel:** one kind:9002 with `["t", ""]` +
///   `unlisted false`. A non-member then finds it in discovery.
#[tokio::test]
#[ignore]
async fn test_nip29_solo_share_and_convert_wire() {
    let url = relay_url();
    let owner = Keys::generate();
    let viewer = Keys::generate();
    // A non-member whose accessible channels are cached before each change.
    let stranger = Keys::generate();
    let since = nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 1);
    let h = SingleLetterTag::lowercase(Alphabet::H);
    let messages = Filter::new().kind(Kind::Custom(9)).since(since);

    // ── Create unlisted (kind:9007) ─────────────────────────────────────────
    let (accepted, msg) = submit_ws(
        &url,
        &owner,
        9007,
        &[
            &["h", &uuid::Uuid::new_v4().to_string()],
            &["name", "solo-bad-unlisted"],
            &["visibility", "private"],
            &["unlisted", "yes"],
        ],
    )
    .await;
    assert!(!accepted, "an unknown unlisted value must be refused");
    assert!(msg.contains("invalid unlisted value"), "{msg}");

    let born_unlisted = create_solo_chat_ws(&url, &owner, &[&["unlisted", "true"]]).await;
    let tags = group_metadata_tags(&url, &owner, &born_unlisted).await;
    assert!(tags.contains(&vec!["unlisted".to_string()]), "{tags:?}");
    assert!(tags.contains(&vec!["private".to_string()]), "{tags:?}");
    assert!(!listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&born_unlisted));
    let (accepted, msg) = submit_ws(
        &url,
        &owner,
        9002,
        &[&["h", &born_unlisted], &["visibility", "open"]],
    )
    .await;
    assert!(accepted, "open a chat created unlisted: {msg}");
    assert!(
        !listed_channel_ids(&url, &stranger, since)
            .await
            .contains(&born_unlisted),
        "opening a chat created unlisted keeps it unlisted"
    );
    let tags = group_metadata_tags(&url, &stranger, &born_unlisted).await;
    assert!(tags.contains(&vec!["unlisted".to_string()]), "{tags:?}");
    assert!(tags.contains(&vec!["public".to_string()]), "{tags:?}");

    // ── Share: one kind:9002 on an existing (listed) private chat ───────────
    let chat = create_solo_chat_ws(&url, &owner, &[]).await;
    let (accepted, msg) = post_ws(&url, &chat, &owner, "before sharing").await;
    assert!(accepted, "owner post: {msg}");
    assert!(!listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&chat));
    let (accepted, msg) = submit_ws(
        &url,
        &owner,
        9002,
        &[
            &["h", &chat],
            &["visibility", "open"],
            &["unlisted", "true"],
            &["join_role", "guest"],
        ],
    )
    .await;
    assert!(accepted, "share in one event: {msg}");
    let tags = group_metadata_tags(&url, &viewer, &chat).await;
    for flag in ["public", "unlisted", "restricted"] {
        assert!(tags.contains(&vec![flag.to_string()]), "{flag}: {tags:?}");
    }
    assert!(
        tags.contains(&vec!["join_role".to_string(), "guest".to_string()]),
        "{tags:?}"
    );
    assert!(
        !listed_channel_ids(&url, &stranger, since)
            .await
            .contains(&chat),
        "a shared chat is not listed to a non-member"
    );

    // A non-member viewer reads it by `#h`, history and live, but can't post.
    let named = messages.clone().custom_tags(h, [chat.as_str()]);
    assert!(any_in_channel(
        &req_events(&url, &viewer, named.clone()).await,
        &chat
    ));
    assert!(!any_in_channel(
        &req_events(&url, &viewer, messages.clone()).await,
        &chat
    ));
    let mut live = BuzzTestClient::connect(&url, &viewer)
        .await
        .expect("connect viewer");
    let live_sub = sub_id("solo-live");
    live.subscribe(&live_sub, vec![named.clone()])
        .await
        .expect("subscribe by #h");
    live.collect_until_eose(&live_sub, Duration::from_secs(5))
        .await
        .expect("EOSE");
    let token = format!("live-{}", uuid::Uuid::new_v4().simple());
    let (accepted, msg) = post_ws(&url, &chat, &owner, &token).await;
    assert!(accepted, "owner post: {msg}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match live.recv_event(remaining).await.expect("live event") {
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == live_sub && event.content == token => break,
            RelayMessage::Closed { message, .. } => panic!("live sub closed: {message}"),
            _ => {}
        }
    }
    let (accepted, msg) = post_ws(&url, &chat, &viewer, "viewer post").await;
    assert!(!accepted, "a guest-join chat is read-only to a non-member");
    assert!(msg.contains("only members can post"), "{msg}");

    // Pin (kind:9021): listed for the viewer, still a read-only guest.
    let (accepted, msg) = join_ws(&url, &chat, &viewer).await;
    assert!(accepted, "pin: {msg}");
    assert!(listed_channel_ids(&url, &viewer, since)
        .await
        .contains(&chat));
    assert!(any_in_channel(
        &req_events(&url, &viewer, messages.clone()).await,
        &chat
    ));
    let (accepted, msg) = post_ws(&url, &chat, &viewer, "guest post").await;
    assert!(!accepted, "a pinned guest still cannot post");
    assert!(msg.contains("only members can post"), "{msg}");
    assert!(!listed_channel_ids(&url, &stranger, since)
        .await
        .contains(&chat));

    // Unpin (kind:9022): unlisted for the viewer again, still readable by id.
    let (accepted, msg) = self_departure_ws(&url, &chat, &viewer, 9022).await;
    assert!(accepted, "unpin: {msg}");
    assert!(
        !listed_channel_ids(&url, &viewer, since)
            .await
            .contains(&chat),
        "an unpinned chat leaves the viewer's list"
    );
    assert!(any_in_channel(
        &req_events(&url, &viewer, named.clone()).await,
        &chat
    ));

    // Unshare: the viewer's live subscription closes and a new read is refused.
    let (accepted, msg) = submit_ws(
        &url,
        &owner,
        9002,
        &[&["h", &chat], &["visibility", "private"]],
    )
    .await;
    assert!(accepted, "unshare: {msg}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match live.recv_event(remaining).await.expect("CLOSED on unshare") {
            RelayMessage::Closed {
                subscription_id, ..
            } if subscription_id == live_sub => break,
            RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == live_sub && event.kind == Kind::Custom(9) => {
                panic!("a message reached the viewer after unsharing: {event:?}")
            }
            _ => {}
        }
    }
    live.disconnect().await.ok();
    let mut ws = BuzzTestClient::connect(&url, &viewer)
        .await
        .expect("connect");
    let sid = sub_id("solo-unshared");
    ws.subscribe(&sid, vec![named]).await.expect("subscribe");
    match ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("reply to #h read")
    {
        RelayMessage::Closed { message, .. } => {
            assert!(message.contains("restricted"), "{message}")
        }
        other => panic!("an unshared chat must refuse a non-member: {other:?}"),
    }
    ws.disconnect().await.ok();

    // ── Convert to a channel: one kind:9002 ─────────────────────────────────
    let (accepted, msg) = submit_ws(
        &url,
        &owner,
        9002,
        &[
            &["h", &born_unlisted],
            &["t", ""],
            &["unlisted", "false"],
            &["visibility", "open"],
        ],
    )
    .await;
    assert!(accepted, "convert in one event: {msg}");
    let tags = group_metadata_tags(&url, &stranger, &born_unlisted).await;
    assert!(!tags.contains(&vec!["unlisted".to_string()]), "{tags:?}");
    assert!(
        !tags.contains(&vec!["t".to_string(), "buzz.solo".to_string()]),
        "converting clears the Solo label: {tags:?}"
    );
    assert!(
        listed_channel_ids(&url, &stranger, since)
            .await
            .contains(&born_unlisted),
        "a converted chat is listed to non-members"
    );
}

/// Record `owner` as the NIP-OA owner of `agent`, by authenticating once.
async fn record_agent_owner(url: &str, agent: &Keys, owner: &Keys) {
    let auth_tag = buzz_sdk::nip_oa::parse_auth_tag(
        &buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "kind=9")
            .expect("compute auth tag"),
    )
    .expect("parse auth tag");
    let mut agent_client = BuzzTestClient::connect_unauthenticated(url)
        .await
        .expect("connect agent");
    agent_client
        .authenticate_with_nip_oa(agent, &auth_tag)
        .await
        .expect("NIP-OA auth");
    agent_client.disconnect().await.ok();
}

/// Announce channel (`join_role = guest` on an open channel) through the real
/// relay: joining gives guest, guests and non-members cannot write or add
/// people, members can, only owners/admins change the setting, and changing
/// it back to `member` gives new joiners member without changing any
/// existing role.
#[tokio::test]
#[ignore]
async fn test_nip29_announce_channel_wire() {
    let url = relay_url();
    let owner = Keys::generate();
    let member = Keys::generate();
    let outsider = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let (accepted, msg) = add_member_with_role_ws(
        &mut owner_client,
        &channel_id,
        &member.public_key().to_hex(),
        "member",
        &owner,
    )
    .await;
    assert!(accepted, "owner adds member: {msg}");
    owner_client.disconnect().await.ok();

    // Only owners/admins may change the setting; unknown values are refused.
    let (accepted, msg) = set_join_role_ws(&url, &channel_id, &member, "guest").await;
    assert!(!accepted, "member must not change join_role");
    assert!(msg.contains("not authorized"), "{msg}");
    let (accepted, msg) = set_join_role_ws(&url, &channel_id, &owner, "members").await;
    assert!(!accepted, "unknown join_role value must be refused");
    assert!(msg.contains("invalid join_role value"), "{msg}");
    assert_eq!(
        set_join_role_ws(&url, &channel_id, &owner, "guest").await,
        (true, String::new())
    );

    let tags = group_metadata_tags(&url, &owner, &channel_id).await;
    assert!(tags.contains(&vec!["restricted".to_string()]), "{tags:?}");
    assert!(tags.contains(&vec!["join_role".to_string(), "guest".to_string()]));
    assert!(tags.contains(&vec!["public".to_string()]));
    assert!(
        !tags.contains(&vec!["closed".to_string()]),
        "open channels accept kind:9021 joins: {tags:?}"
    );

    // A non-member of an open announce channel cannot write, and cannot
    // self-add as a member.
    let (accepted, msg) = post_ws(&url, &channel_id, &outsider, "outsider post").await;
    assert!(!accepted, "non-member must not post");
    assert!(msg.contains("only members can post"), "{msg}");
    let mut outsider_client = BuzzTestClient::connect(&url, &outsider)
        .await
        .expect("connect outsider");
    let (accepted, msg) = add_member_with_role_ws(
        &mut outsider_client,
        &channel_id,
        &outsider.public_key().to_hex(),
        "member",
        &outsider,
    )
    .await;
    assert!(!accepted, "non-member must not self-add as member");
    assert!(msg.contains("gives the guest role"), "{msg}");
    outsider_client.disconnect().await.ok();

    // Join makes a guest. A guest cannot post or add people.
    let (accepted, msg) = join_ws(&url, &channel_id, &outsider).await;
    assert!(accepted, "join: {msg}");
    assert_eq!(
        member_role(&url, &owner, &channel_id, &outsider.public_key().to_hex())
            .await
            .as_deref(),
        Some("guest")
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &outsider, "guest post").await;
    assert!(!accepted, "guest must not post");
    assert!(msg.contains("only members can post"), "{msg}");
    let mut outsider_client = BuzzTestClient::connect(&url, &outsider)
        .await
        .expect("connect guest");
    let (accepted, msg) = add_member_with_role_ws(
        &mut outsider_client,
        &channel_id,
        &Keys::generate().public_key().to_hex(),
        "member",
        &outsider,
    )
    .await;
    assert!(!accepted, "guest must not add a member");
    assert!(msg.contains("only members can add people"), "{msg}");
    outsider_client.disconnect().await.ok();

    // A member can still post.
    let mut member_client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("connect member");
    let ok = member_client
        .send_text_message(&member, &channel_id, "member post", 9)
        .await
        .expect("send member post");
    assert!(ok.accepted, "member post: {}", ok.message);
    member_client.disconnect().await.ok();
    let member_post_id = ok.event_id;

    // A reaction is a write: a guest cannot react to the member's post. A
    // reaction derives its channel from its target, so the relay reports
    // every channel-gate refusal as a missing target.
    let reaction = EventBuilder::new(Kind::Custom(7), "+")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["e", &member_post_id]).unwrap(),
        ])
        .sign_with_keys(&outsider)
        .unwrap();
    let mut guest_client = BuzzTestClient::connect(&url, &outsider)
        .await
        .expect("connect guest");
    let ok = guest_client
        .send_event(reaction)
        .await
        .expect("send guest reaction");
    assert!(!ok.accepted, "guest must not react");
    assert!(
        ok.message.contains("reaction target event not found"),
        "{}",
        ok.message
    );
    guest_client.disconnect().await.ok();

    // Back to `member`: roles do not change with the setting, so the guest
    // stays read-only, but a new joiner becomes a member and can post.
    assert_eq!(
        set_join_role_ws(&url, &channel_id, &owner, "member").await,
        (true, String::new())
    );
    let tags = group_metadata_tags(&url, &owner, &channel_id).await;
    assert!(!tags.contains(&vec!["restricted".to_string()]), "{tags:?}");
    assert!(tags.contains(&vec!["join_role".to_string(), "member".to_string()]));
    let (accepted, msg) = post_ws(&url, &channel_id, &outsider, "guest post after").await;
    assert!(!accepted, "a guest stays read-only after join_role=member");
    assert!(msg.contains("only members can post"), "{msg}");
    let late = Keys::generate();
    let (accepted, msg) = join_ws(&url, &channel_id, &late).await;
    assert!(accepted, "late join: {msg}");
    assert_eq!(
        member_role(&url, &owner, &channel_id, &late.public_key().to_hex())
            .await
            .as_deref(),
        Some("member")
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &late, "late post").await;
    assert!(accepted, "late joiner post: {msg}");

    // A member (not only an admin) can make a guest person a member.
    let mut member_client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("reconnect member");
    let (accepted, msg) = add_member_with_role_ws(
        &mut member_client,
        &channel_id,
        &outsider.public_key().to_hex(),
        "member",
        &member,
    )
    .await;
    assert!(accepted, "member promotes the guest: {msg}");
    member_client.disconnect().await.ok();
    assert_eq!(
        member_role(&url, &owner, &channel_id, &outsider.public_key().to_hex())
            .await
            .as_deref(),
        Some("member")
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &outsider, "promoted post").await;
    assert!(accepted, "promoted member post: {msg}");

    // Anyone can always leave.
    assert_eq!(
        self_departure_ws(&url, &channel_id, &outsider, 9022).await,
        (true, String::new())
    );
}

/// Roles mean the same for agents and people: an agent that joins an
/// announce channel by itself is a guest, and any member can make it a
/// member, as for a person. Owner-only raising belongs to the later
/// restricted-agent level (Channels for IFC), not to this rule.
#[tokio::test]
#[ignore]
async fn test_nip29_member_raises_guest_agent() {
    let url = relay_url();
    let owner = Keys::generate();
    let member = Keys::generate();
    let human = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&owner).await;
    record_agent_owner(&url, &agent, &human).await;

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    for pk in [&member, &human] {
        let (accepted, msg) = add_member_with_role_ws(
            &mut owner_client,
            &channel_id,
            &pk.public_key().to_hex(),
            "member",
            &owner,
        )
        .await;
        assert!(accepted, "owner adds member: {msg}");
    }
    owner_client.disconnect().await.ok();
    assert_eq!(
        set_join_role_ws(&url, &channel_id, &owner, "guest").await,
        (true, String::new())
    );

    let (accepted, msg) = join_ws(&url, &channel_id, &agent).await;
    assert!(accepted, "agent join: {msg}");
    let agent_hex = agent.public_key().to_hex();
    assert_eq!(
        member_role(&url, &owner, &channel_id, &agent_hex)
            .await
            .as_deref(),
        Some("guest"),
        "an agent that joins by itself gets join_role"
    );

    let mut member_client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("connect member");
    let (accepted, msg) = add_member_with_role_ws(
        &mut member_client,
        &channel_id,
        &agent_hex,
        "member",
        &member,
    )
    .await;
    assert!(accepted, "a member raises a guest agent: {msg}");
    member_client.disconnect().await.ok();
    assert_eq!(
        member_role(&url, &owner, &channel_id, &agent_hex)
            .await
            .as_deref(),
        Some("member")
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &agent, "agent post").await;
    assert!(accepted, "raised agent post: {msg}");
}

/// In an announce channel, the human owner of an agent keeps the existing
/// owner authority over that agent's messages: they can edit (kind:40003) and
/// delete (kind:9005) them while not a member and while a guest, although
/// they cannot post themselves. An ordinary guest cannot edit them.
#[tokio::test]
#[ignore]
async fn test_nip29_announce_channel_wire_agent_owner_exception() {
    let url = relay_url();
    let agent = Keys::generate();
    let human = Keys::generate();
    let stranger = Keys::generate();
    let channel_id = create_test_channel(&agent).await;
    record_agent_owner(&url, &agent, &human).await;

    assert_eq!(
        set_join_role_ws(&url, &channel_id, &agent, "guest").await,
        (true, String::new())
    );
    let mut agent_client = BuzzTestClient::connect(&url, &agent)
        .await
        .expect("reconnect agent");
    let first = agent_client
        .send_text_message(&agent, &channel_id, "agent post one", 9)
        .await
        .expect("send agent post");
    assert!(first.accepted, "agent post: {}", first.message);
    let second = agent_client
        .send_text_message(&agent, &channel_id, "agent post two", 9)
        .await
        .expect("send agent post");
    assert!(second.accepted, "agent post: {}", second.message);
    agent_client.disconnect().await.ok();

    let edit = |signer: &Keys, target: &str, content: &str| {
        EventBuilder::new(Kind::Custom(40003), content)
            .tags([
                Tag::parse(["e", target]).unwrap(),
                Tag::parse(["h", &channel_id]).unwrap(),
            ])
            .sign_with_keys(signer)
            .unwrap()
    };

    // An ordinary guest cannot edit the agent's message.
    let (accepted, msg) = join_ws(&url, &channel_id, &stranger).await;
    assert!(accepted, "stranger join: {msg}");
    let mut stranger_client = BuzzTestClient::connect(&url, &stranger)
        .await
        .expect("connect stranger");
    let ok = stranger_client
        .send_event(edit(&stranger, &first.event_id, "stranger edit"))
        .await
        .expect("send stranger edit");
    assert!(
        !ok.accepted,
        "an ordinary guest must not edit an agent message"
    );
    stranger_client.disconnect().await.ok();

    // The owner edits while not a member, then joins as a guest. As a guest
    // they still cannot post, but can edit and delete the agent's messages.
    let mut human_client = BuzzTestClient::connect(&url, &human)
        .await
        .expect("connect owner");
    let ok = human_client
        .send_event(edit(&human, &first.event_id, "edited by owner"))
        .await
        .expect("send owner edit");
    assert!(ok.accepted, "non-member owner edit: {}", ok.message);
    human_client.disconnect().await.ok();
    let (accepted, msg) = join_ws(&url, &channel_id, &human).await;
    assert!(accepted, "owner join: {msg}");
    assert_eq!(
        member_role(&url, &agent, &channel_id, &human.public_key().to_hex())
            .await
            .as_deref(),
        Some("guest")
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &human, "owner post").await;
    assert!(!accepted, "a guest owner must not post");
    assert!(msg.contains("only members can post"), "{msg}");
    let mut human_client = BuzzTestClient::connect(&url, &human)
        .await
        .expect("reconnect owner");
    let ok = human_client
        .send_event(edit(&human, &first.event_id, "edited again by owner"))
        .await
        .expect("send guest owner edit");
    assert!(ok.accepted, "guest owner edit: {}", ok.message);
    let delete = EventBuilder::new(Kind::Custom(9005), "")
        .tags([
            Tag::parse(["e", &second.event_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    let ok = human_client.send_event(delete).await.expect("send delete");
    assert!(ok.accepted, "guest owner delete: {}", ok.message);
    human_client.disconnect().await.ok();
}

/// Roles mean the same thing in every channel. In a normal channel
/// (`join_role = member`) a self-join gives member, as today, and a bare
/// self-add does too; but a guest that an owner adds still cannot post.
#[tokio::test]
#[ignore]
async fn test_nip29_guest_is_read_only_in_normal_channel() {
    let url = relay_url();
    let owner = Keys::generate();
    let joiner = Keys::generate();
    let self_adder = Keys::generate();
    let guest = Keys::generate();
    let channel_id = create_test_channel(&owner).await;

    let (accepted, msg) = join_ws(&url, &channel_id, &joiner).await;
    assert!(accepted, "join: {msg}");
    assert_eq!(
        member_role(&url, &owner, &channel_id, &joiner.public_key().to_hex())
            .await
            .as_deref(),
        Some("member"),
        "kind:9021 join must give join_role (member)"
    );
    let (accepted, msg) = post_ws(&url, &channel_id, &joiner, "joiner post").await;
    assert!(accepted, "a joiner can post in a normal channel: {msg}");

    let self_hex = self_adder.public_key().to_hex();
    let mut self_client = BuzzTestClient::connect(&url, &self_adder)
        .await
        .expect("connect self-adder");
    let (accepted, msg) = add_member_with_role_ws(
        &mut self_client,
        &channel_id,
        &self_hex,
        "admin",
        &self_adder,
    )
    .await;
    assert!(!accepted, "non-member must not self-add as admin: {msg}");
    let bare = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &self_hex]).unwrap(),
        ])
        .sign_with_keys(&self_adder)
        .unwrap();
    let ok = self_client
        .send_event(bare)
        .await
        .expect("send bare self-add");
    assert!(ok.accepted, "bare self-add: {}", ok.message);
    self_client.disconnect().await.ok();
    assert_eq!(
        member_role(&url, &owner, &channel_id, &self_hex)
            .await
            .as_deref(),
        Some("member"),
        "a self-add with no role tag must give join_role (member)"
    );

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let (accepted, msg) = add_member_with_role_ws(
        &mut owner_client,
        &channel_id,
        &guest.public_key().to_hex(),
        "guest",
        &owner,
    )
    .await;
    assert!(accepted, "owner adds a guest: {msg}");
    owner_client.disconnect().await.ok();
    let (accepted, msg) = post_ws(&url, &channel_id, &guest, "guest post").await;
    assert!(!accepted, "a guest must not post, also in a normal channel");
    assert!(msg.contains("only members can post"), "{msg}");
}

/// SECURITY REPRO (Dawn): can an unprivileged NON-MEMBER demote the owner of an
/// OPEN channel to `member` with a single kind:9000? Asserts the reported
/// vulnerability is FIXED; it fails on vulnerable code.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_cannot_demote_owner() {
    let url = relay_url();

    let victim_keys = Keys::generate();
    let victim_hex = victim_keys.public_key().to_hex();
    let attacker_keys = Keys::generate();

    // Victim creates an open channel -> victim is seeded as its owner.
    let channel_id = create_test_channel(&victim_keys).await;

    let before = member_role(&url, &victim_keys, &channel_id, &victim_hex).await;
    assert_eq!(
        before.as_deref(),
        Some("owner"),
        "victim must start as owner (got {before:?})"
    );

    // The attack: attacker (not a member, not the creator) publishes
    // kind:9000 { h=channel, p=victim, role=member }.
    let mut ws = BuzzTestClient::connect(&url, &attacker_keys)
        .await
        .expect("connect as attacker");
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &victim_hex]).unwrap(),
            Tag::parse(["role", "member"]).unwrap(),
        ])
        .sign_with_keys(&attacker_keys)
        .expect("sign kind 9000");
    let ok = ws.send_event(event).await.expect("send kind 9000");
    ws.disconnect().await.ok();

    let after = member_role(&url, &victim_keys, &channel_id, &victim_hex).await;
    println!(
        "attacker kind:9000 accepted = {} ({})",
        ok.accepted, ok.message
    );
    println!("victim role before = {before:?}, after = {after:?}");

    assert_eq!(
        after.as_deref(),
        Some("owner"),
        "PRIVESC: unprivileged non-member demoted the channel owner to {after:?}"
    );
    assert!(
        !ok.accepted,
        "unprivileged non-member's role-demotion kind:9000 must be rejected"
    );
}

/// SECURITY REPRO (Dawn), follow-on questions the report asserts but does not test:
/// after the owner is demoted, (a) can the attacker promote itself to owner, and
/// (b) can the ex-owner restore its own role? Both must be rejected — which is
/// exactly what makes the demotion unrecoverable over the relay.
#[tokio::test]
#[ignore]
async fn test_nip29_owner_demotion_recovery_paths() {
    let url = relay_url();

    let victim_keys = Keys::generate();
    let victim_hex = victim_keys.public_key().to_hex();
    let attacker_keys = Keys::generate();
    let attacker_hex = attacker_keys.public_key().to_hex();

    let channel_id = create_test_channel(&victim_keys).await;

    let put_user = |signer: Keys, target_hex: String, role: &'static str| {
        let channel_id = channel_id.clone();
        let url = url.clone();
        async move {
            let mut ws = BuzzTestClient::connect(&url, &signer)
                .await
                .expect("connect");
            // `allow_self_tagging` is REQUIRED: EventBuilder otherwise silently
            // drops any `p` tag matching the signer (nostr-0.44.3
            // builder.rs:435-449), which would make self-targeted PUT_USER
            // events fail as "missing p tag" and mask the real verdict.
            let event = EventBuilder::new(Kind::Custom(9000), "")
                .allow_self_tagging()
                .tags([
                    Tag::parse(["h", &channel_id]).unwrap(),
                    Tag::parse(["p", &target_hex]).unwrap(),
                    Tag::parse(["role", role]).unwrap(),
                ])
                .sign_with_keys(&signer)
                .expect("sign kind 9000");
            let ok = ws.send_event(event).await.expect("send kind 9000");
            ws.disconnect().await.ok();
            ok
        }
    };

    // Step 1: strip the owner.
    let demote = put_user(attacker_keys.clone(), victim_hex.clone(), "member").await;
    println!(
        "1. attacker demotes owner   -> accepted={} {}",
        demote.accepted, demote.message
    );

    // Step 2: attacker tries to make itself owner.
    let self_promote = put_user(attacker_keys.clone(), attacker_hex.clone(), "owner").await;
    println!(
        "2. attacker self->owner     -> accepted={} {}",
        self_promote.accepted, self_promote.message
    );

    // Step 3: ex-owner tries to restore itself.
    let restore = put_user(victim_keys.clone(), victim_hex.clone(), "owner").await;
    println!(
        "3. ex-owner restores self   -> accepted={} {}",
        restore.accepted, restore.message
    );

    // `accepted` only means the event was stored — the membership side effect can
    // still fail. Read the authoritative roles back from the relay-signed 39002.
    let mut ws = BuzzTestClient::connect(&url, &victim_keys)
        .await
        .expect("connect");
    let sid = sub_id("members-final");
    let filter = Filter::new().kind(Kind::Custom(39002)).custom_tags(
        SingleLetterTag::lowercase(Alphabet::D),
        [channel_id.as_str()],
    );
    ws.subscribe(&sid, vec![filter])
        .await
        .expect("subscribe 39002");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("39002 EOSE");
    ws.disconnect().await.ok();
    let latest = events.iter().max_by_key(|e| e.created_at).expect("a 39002");
    let roles: Vec<(String, String)> = latest
        .tags
        .iter()
        .filter_map(|t| {
            let p = t.as_slice();
            (p.len() >= 4 && p[0] == "p").then(|| (p[1].clone(), p[3].clone()))
        })
        .collect();
    let role_of = |hex: &str| {
        roles
            .iter()
            .find(|(pk, _)| pk == hex)
            .map(|(_, r)| r.clone())
    };
    println!("FINAL victim   role = {:?}", role_of(&victim_hex));
    println!("FINAL attacker role = {:?}", role_of(&attacker_hex));
    assert_eq!(
        role_of(&victim_hex).as_deref(),
        Some("owner"),
        "owner must retain its role through all three attempts"
    );
    assert_eq!(
        role_of(&attacker_hex),
        None,
        "attacker must never gain a role"
    );
}

/// SECURITY REPRO (Dawn), finding #2: a kind:9000 PUT_USER with NO `role` tag.
///
/// `handle_put_user` used to default an absent role tag to `member`, so a bare
/// self-targeted PUT_USER silently demoted the sender — no attacker required.
/// `test_nip29_put_user_self_add_bypasses_policy` sends exactly this event but
/// asserts only `ok.accepted`, and side-effect failures are logged rather than
/// surfaced (ingest.rs:2460-2467), so an `accepted` assertion structurally
/// cannot observe the demotion. This asserts the resulting STATE instead.
///
/// TWO owners on purpose. With a sole owner the last-owner guard rejects the
/// demotion anyway, so the test would pass even with the role-preservation
/// removed — it would be measuring a different defect's fix. A second owner
/// disarms that guard, leaving the absent-role-tag handling as the only thing
/// that can decide the outcome.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_without_role_tag_preserves_role() {
    let url = relay_url();

    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let b_hex = owner_b.public_key().to_hex();
    let channel_id = create_test_channel(&owner_a).await;

    // owner_a promotes owner_b, so the channel has two owners.
    let mut ws = BuzzTestClient::connect(&url, &owner_a)
        .await
        .expect("connect as owner_a");
    let promote = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &b_hex]).unwrap(),
            Tag::parse(["role", "owner"]).unwrap(),
        ])
        .sign_with_keys(&owner_a)
        .expect("sign promote");
    let ok = ws.send_event(promote).await.expect("send promote");
    ws.disconnect().await.ok();
    assert!(ok.accepted, "promote rejected: {}", ok.message);
    assert_eq!(
        member_role(&url, &owner_a, &channel_id, &b_hex)
            .await
            .as_deref(),
        Some("owner"),
        "owner_b must be a second owner before the probe"
    );

    // The probe: owner_b sends a bare self-targeted PUT_USER — h + p, no `role`.
    // `allow_self_tagging` is required or EventBuilder drops the self `p` tag
    // (nostr-0.44.3 builder.rs:435-449) and the event fails as "missing p tag".
    let mut ws = BuzzTestClient::connect(&url, &owner_b)
        .await
        .expect("connect as owner_b");
    let bare = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &b_hex]).unwrap(),
        ])
        .sign_with_keys(&owner_b)
        .expect("sign bare put_user");
    let ok = ws.send_event(bare).await.expect("send bare put_user");
    ws.disconnect().await.ok();
    assert!(
        ok.accepted,
        "self-targeted PUT_USER must stay accepted: {}",
        ok.message
    );

    assert_eq!(
        member_role(&url, &owner_a, &channel_id, &b_hex)
            .await
            .as_deref(),
        Some("owner"),
        "an absent role tag means no role change — it must not demote an owner"
    );
}

/// SECURITY (Dawn), relay-layer guard isolation — the *validator*, not the DB.
///
/// The DB guards in `add_member` are what actually stop the privesc, and every
/// other test here asserts the resulting STATE ("the role didn't change").
/// That makes them structurally blind to `validate_admin_event`: stubbing out
/// either relay-side check leaves the whole nip29 suite green, because the DB
/// still refuses the write and the role is still correct. Verified by mutation
/// — the relay returns `accepted:true` and logs `Side effect failed: access
/// denied: ...` while the state assertion happily passes.
///
/// The relay guards earn their keep by giving the client an honest
/// `accepted:false` instead of an OK for an event that silently fails after the
/// fact. So these two tests assert `accepted == false` — the one observable
/// only the validator controls — and each is shaped so exactly one guard can
/// fire.
///
/// Guard under test: "only owners/admins may change an active member's role".
/// TWO owners on purpose, so the last-owner guard cannot fire and take the
/// credit; a plain member targeting a co-owner leaves the actor check as the
/// only thing that can reject.
#[tokio::test]
#[ignore]
async fn test_nip29_relay_rejects_role_change_by_unprivileged_actor() {
    let url = relay_url();

    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let b_hex = owner_b.public_key().to_hex();
    let attacker = Keys::generate();
    let attacker_hex = attacker.public_key().to_hex();
    let channel_id = create_test_channel(&owner_a).await;

    // owner_a promotes owner_b -> the channel has two owners.
    let mut ws = BuzzTestClient::connect(&url, &owner_a)
        .await
        .expect("connect as owner_a");
    let promote = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &b_hex]).unwrap(),
            Tag::parse(["role", "owner"]).unwrap(),
        ])
        .sign_with_keys(&owner_a)
        .expect("sign promote");
    let ok = ws.send_event(promote).await.expect("send promote");
    ws.disconnect().await.ok();
    assert!(ok.accepted, "promote rejected: {}", ok.message);

    // The attacker joins the open channel as a plain member.
    let mut ws = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect as attacker");
    let join = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &attacker_hex]).unwrap(),
            Tag::parse(["role", "member"]).unwrap(),
        ])
        .sign_with_keys(&attacker)
        .expect("sign self-join");
    let ok = ws.send_event(join).await.expect("send self-join");
    ws.disconnect().await.ok();
    assert!(ok.accepted, "self-join rejected: {}", ok.message);
    assert_eq!(
        member_role(&url, &owner_a, &channel_id, &attacker_hex)
            .await
            .as_deref(),
        Some("member"),
        "attacker must be an active plain member before the probe"
    );

    // The probe: a plain member demotes a co-owner. Two owners remain, so only
    // the actor-authorization guard can reject this.
    let mut ws = BuzzTestClient::connect(&url, &attacker)
        .await
        .expect("connect as attacker");
    let attack = EventBuilder::new(Kind::Custom(9000), "")
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &b_hex]).unwrap(),
            Tag::parse(["role", "member"]).unwrap(),
        ])
        .sign_with_keys(&attacker)
        .expect("sign attack");
    let ok = ws.send_event(attack).await.expect("send attack");
    ws.disconnect().await.ok();
    println!(
        "unprivileged co-owner demotion -> accepted={} {}",
        ok.accepted, ok.message
    );

    assert!(
        !ok.accepted,
        "the relay validator must reject an unprivileged actor's role change, \
         not accept it and let the side effect fail silently"
    );
    assert_eq!(
        member_role(&url, &owner_a, &channel_id, &b_hex)
            .await
            .as_deref(),
        Some("owner"),
        "co-owner must keep their role"
    );
}

/// SECURITY (Dawn), relay-layer guard isolation — see the test above for why
/// `accepted` rather than state is the assertion that matters here.
///
/// Guard under test: the relay-side last-owner check. The actor is the SOLE
/// owner demoting themselves, so the actor-authorization guard is satisfied
/// (an owner is elevated) and cannot mask the result — only the last-owner
/// check can reject.
#[tokio::test]
#[ignore]
async fn test_nip29_relay_rejects_last_owner_self_demotion() {
    let url = relay_url();

    let owner = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let channel_id = create_test_channel(&owner).await;

    assert_eq!(
        member_role(&url, &owner, &channel_id, &owner_hex)
            .await
            .as_deref(),
        Some("owner"),
        "creator must be the sole owner before the probe"
    );

    // The probe: the sole owner demotes themselves. Elevated actor, so the
    // actor check passes; the last-owner guard is the only thing left.
    let mut ws = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect as owner");
    let demote = EventBuilder::new(Kind::Custom(9000), "")
        .allow_self_tagging()
        .tags([
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["p", &owner_hex]).unwrap(),
            Tag::parse(["role", "member"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .expect("sign self-demote");
    let ok = ws.send_event(demote).await.expect("send self-demote");
    ws.disconnect().await.ok();
    println!(
        "sole-owner self-demotion -> accepted={} {}",
        ok.accepted, ok.message
    );

    assert!(
        !ok.accepted,
        "the relay validator must reject demoting the last owner, not accept \
         it and let the side effect fail silently"
    );
    assert_eq!(
        member_role(&url, &owner, &channel_id, &owner_hex)
            .await
            .as_deref(),
        Some("owner"),
        "the last owner must keep their role"
    );
}

/// A workflow posts as its owner, so in an announce channel its `send_message`
/// needs the same write role a post does: it replies while the owner is a
/// member, and stops once the owner is made a guest.
#[tokio::test]
#[ignore]
async fn test_nip29_announce_channel_wire_workflow_send_needs_write_role() {
    let url = relay_url();
    let http = relay_http_url();
    let owner = Keys::generate();
    let author = Keys::generate();
    let author_hex = author.public_key().to_hex();
    let channel_id = create_test_channel(&owner).await;

    let mut owner_client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let (accepted, msg) = add_member_with_role_ws(
        &mut owner_client,
        &channel_id,
        &author_hex,
        "member",
        &owner,
    )
    .await;
    assert!(accepted, "owner adds the workflow author: {msg}");
    assert_eq!(
        set_join_role_ws(&url, &channel_id, &owner, "guest").await,
        (true, String::new())
    );

    // Replies only to top-level messages, so it cannot trigger itself.
    let yaml = "name: announce-reply\n\
         description: announce write-role probe\n\
         trigger:\n\
         \x20 on: message_posted\n\
         \x20 filter: \"trigger_is_reply == false\"\n\
         steps:\n\
         \x20 - id: step1\n\
         \x20   name: Reply\n\
         \x20   action: send_message\n\
         \x20   text: \"auto-reply\"\n\
         \x20   reply_in_thread: true\n"
        .to_string();
    let def = EventBuilder::new(Kind::Custom(30620), yaml)
        .tags([
            Tag::parse(["d", &Uuid::new_v4().to_string()]).unwrap(),
            Tag::parse(["h", channel_id.as_str()]).unwrap(),
            Tag::parse(["name", "announce-reply"]).unwrap(),
        ])
        .sign_with_keys(&author)
        .expect("sign workflow def");
    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{http}/events"))
        .header("X-Pubkey", &author_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&def).unwrap())
        .send()
        .await
        .expect("submit workflow def")
        .json()
        .await
        .expect("parse def response");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "workflow def not accepted: {body}"
    );

    let sid = sub_id("announce-wf");
    let filter = Filter::new().kind(Kind::Custom(9)).custom_tags(
        SingleLetterTag::lowercase(Alphabet::H),
        [channel_id.as_str()],
    );
    owner_client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    owner_client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");

    // Post a top-level message, then report whether a workflow reply to it
    // arrives within `wait`.
    async fn trigger_gets_reply(
        client: &mut BuzzTestClient,
        owner: &Keys,
        channel_id: &str,
        wait: Duration,
    ) -> bool {
        let root = EventBuilder::new(Kind::Custom(9), "trigger me")
            .tags([Tag::parse(["h", channel_id]).unwrap()])
            .sign_with_keys(owner)
            .expect("sign root");
        let root_hex = root.id.to_hex();
        let ok = client.send_event(root).await.expect("send root");
        assert!(ok.accepted, "owner's post rejected: {}", ok.message);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let Some(remaining) = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .filter(|d| !d.is_zero())
            else {
                return false;
            };
            match client.recv_event(remaining).await {
                Ok(RelayMessage::Event { event, .. })
                    if event.content == "auto-reply"
                        && event.tags.iter().any(|t| {
                            let p = t.as_slice();
                            p.len() >= 2 && p[0] == "e" && p[1] == root_hex
                        }) =>
                {
                    return true
                }
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    }

    assert!(
        trigger_gets_reply(
            &mut owner_client,
            &owner,
            &channel_id,
            Duration::from_secs(10)
        )
        .await,
        "control: the member's workflow replies"
    );

    let (accepted, msg) =
        add_member_with_role_ws(&mut owner_client, &channel_id, &author_hex, "guest", &owner).await;
    assert!(accepted, "owner makes the workflow author a guest: {msg}");
    assert!(
        !trigger_gets_reply(
            &mut owner_client,
            &owner,
            &channel_id,
            Duration::from_secs(5)
        )
        .await,
        "a guest's workflow must not post in the announce channel"
    );

    owner_client.disconnect().await.ok();
}

/// A kind:9002 is stored before its updates run, so a value the database cannot
/// hold must be refused first. Otherwise a name longer than the column fails
/// after the event is acknowledged, and the `join_role` in the same event is
/// never applied.
#[tokio::test]
#[ignore]
async fn test_nip29_announce_channel_wire_overlong_name_is_refused_before_storage() {
    let url = relay_url();
    let owner = Keys::generate();
    let channel_id = create_test_channel(&owner).await;
    let long_name = "n".repeat(256);
    let event = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", channel_id.as_str()]).unwrap(),
            Tag::parse(["name", long_name.as_str()]).unwrap(),
            Tag::parse(["join_role", "guest"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let ok = client.send_event(event).await.expect("send kind:9002");
    client.disconnect().await.ok();
    assert!(
        !ok.accepted && ok.message.starts_with("invalid:"),
        "an overlong name must be refused, got: {} {}",
        ok.accepted,
        ok.message
    );
    let tags = group_metadata_tags(&url, &owner, &channel_id).await;
    assert!(
        tags.iter()
            .any(|t| t.len() >= 2 && t[0] == "join_role" && t[1] == "member"),
        "the refused event must not change join_role: {tags:?}"
    );

    // The longest storable name, with join_role, applies in full.
    let max_name = "m".repeat(255);
    let event = EventBuilder::new(Kind::Custom(9002), "")
        .tags([
            Tag::parse(["h", channel_id.as_str()]).unwrap(),
            Tag::parse(["name", max_name.as_str()]).unwrap(),
            Tag::parse(["join_role", "guest"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect owner");
    let ok = client.send_event(event).await.expect("send kind:9002");
    client.disconnect().await.ok();
    assert!(ok.accepted, "a 255-character name is valid: {}", ok.message);
    let tags = group_metadata_tags(&url, &owner, &channel_id).await;
    assert!(
        tags.iter()
            .any(|t| t.len() >= 2 && t[0] == "join_role" && t[1] == "guest"),
        "join_role applies with a valid name: {tags:?}"
    );
}

/// Workflow commands skip the generic write gate, so they check the role
/// themselves. A writing member can save and manually trigger a workflow. A
/// guest can do neither, and a workflow saved while its owner could write
/// cannot be triggered after the owner is made a guest. Roles mean the same in
/// every channel, so this holds in a normal channel too.
#[tokio::test]
#[ignore]
async fn test_nip29_announce_channel_wire_workflow_commands_need_write_role() {
    let url = relay_url();
    let http = relay_http_url();

    async fn submit(http: &str, keys: &Keys, event: nostr::Event) -> (bool, String) {
        let resp = reqwest::Client::new()
            .post(format!("{http}/events"))
            .header("X-Pubkey", keys.public_key().to_hex())
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).unwrap())
            .send()
            .await
            .expect("submit event");
        let body: serde_json::Value = resp.json().await.expect("parse response");
        match body["accepted"].as_bool() {
            Some(accepted) => (
                accepted,
                body["message"].as_str().unwrap_or_default().to_string(),
            ),
            None => (
                false,
                body["error"].as_str().unwrap_or_default().to_string(),
            ),
        }
    }

    async fn define(
        http: &str,
        keys: &Keys,
        channel_id: &str,
        workflow_id: &str,
    ) -> (bool, String) {
        let yaml = "name: announce-manual\n\
             description: announce write-role probe\n\
             trigger:\n\
             \x20 on: webhook\n\
             steps:\n\
             \x20 - id: step1\n\
             \x20   name: Notify\n\
             \x20   action: send_message\n\
             \x20   text: \"manual\"\n";
        let def = EventBuilder::new(Kind::Custom(30620), yaml)
            .tags([
                Tag::parse(["h", channel_id]).unwrap(),
                Tag::parse(["d", workflow_id]).unwrap(),
                Tag::parse(["name", "announce-manual"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .expect("sign workflow def");
        submit(http, keys, def).await
    }

    async fn trigger(http: &str, keys: &Keys, workflow_id: &str) -> (bool, String) {
        let event = EventBuilder::new(Kind::Custom(46020), "")
            .tags([Tag::parse(["d", workflow_id]).unwrap()])
            .sign_with_keys(keys)
            .expect("sign trigger");
        submit(http, keys, event).await
    }

    for join_role in ["guest", "member"] {
        let owner = Keys::generate();
        let author = Keys::generate();
        let author_hex = author.public_key().to_hex();
        let channel_id = create_test_channel(&owner).await;
        let mut owner_client = BuzzTestClient::connect(&url, &owner)
            .await
            .expect("connect owner");
        let (accepted, msg) = add_member_with_role_ws(
            &mut owner_client,
            &channel_id,
            &author_hex,
            "member",
            &owner,
        )
        .await;
        assert!(accepted, "owner adds the workflow author: {msg}");
        assert_eq!(
            set_join_role_ws(&url, &channel_id, &owner, join_role).await,
            (true, String::new())
        );

        let workflow_id = uuid::Uuid::new_v4().to_string();
        let (accepted, msg) = define(&http, &author, &channel_id, &workflow_id).await;
        assert!(
            accepted,
            "join_role={join_role}: a member saves a workflow: {msg}"
        );
        let (accepted, msg) = trigger(&http, &author, &workflow_id).await;
        assert!(
            accepted,
            "join_role={join_role}: a member triggers its workflow: {msg}"
        );

        let (accepted, msg) =
            add_member_with_role_ws(&mut owner_client, &channel_id, &author_hex, "guest", &owner)
                .await;
        assert!(accepted, "owner makes the workflow author a guest: {msg}");

        let (accepted, msg) = trigger(&http, &author, &workflow_id).await;
        assert!(
            !accepted,
            "join_role={join_role}: a guest must not trigger its saved workflow: {msg}"
        );
        let other_workflow_id = uuid::Uuid::new_v4().to_string();
        let (accepted, msg) = define(&http, &author, &channel_id, &other_workflow_id).await;
        assert!(
            !accepted && msg.starts_with("restricted:"),
            "join_role={join_role}: a guest must not save a workflow, got: {msg}"
        );

        owner_client.disconnect().await.ok();
    }
}
