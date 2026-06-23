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

use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};

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

/// Create a real channel via a signed kind:9007 event submitted to POST /api/events.
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
        .post(format!("{}/api/events", relay_http_url()))
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
/// The relay's MAX_SUBSCRIPTIONS is 1024. Opening 1024 subs in a test is slow,
/// so we open a smaller batch and verify the NIP-11 advertised limit matches
/// the actual enforcement constant. The full-limit test is covered by the
/// NIP-11 assertion below (which verifies the advertised value is 1024).
#[tokio::test]
#[ignore]
async fn test_subscription_limit_enforced() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    // Open 1024 subscriptions (the relay's MAX_SUBSCRIPTIONS).
    for i in 0..1024 {
        let sid = format!("limit-sub-{i}");
        let filter = Filter::new().kind(Kind::Custom(9));
        client
            .subscribe(&sid, vec![filter])
            .await
            .expect("subscribe");
        // Drain EOSE to avoid buffer buildup.
        client
            .collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("EOSE");
    }

    let overflow_sid = sub_id("overflow");
    // Use a kind that no other test writes, so we don't receive stale events.
    let filter = Filter::new().kind(Kind::Custom(49999));
    client
        .subscribe(&overflow_sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain EOSE and stale events from the 100 earlier subscriptions
    // until we receive the CLOSED for the overflow subscription.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv CLOSED (or timeout)");
        match &m {
            RelayMessage::Eose { .. } => continue,
            RelayMessage::Event { .. } => continue, // stale event from earlier subs
            _ => break m,
        }
    };

    match msg {
        RelayMessage::Closed {
            subscription_id,
            message,
        } => {
            assert_eq!(subscription_id, overflow_sid);
            assert!(
                message.to_lowercase().contains("too many"),
                "Expected 'too many' in CLOSED message, got: {message}"
            );
        }
        other => panic!("Expected CLOSED for overflow subscription, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
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

    // Step 2: Verify the profile has the NIP-05 handle via REST GET.
    let http_client = reqwest::Client::new();
    let profile_resp = http_client
        .get(format!("{}/api/users/{}/profile", http, pubkey_hex))
        .header("X-Pubkey", &pubkey_hex)
        .send()
        .await
        .expect("GET profile");
    assert_eq!(
        profile_resp.status(),
        200,
        "profile should exist after kind:0"
    );
    let profile: serde_json::Value = profile_resp.json().await.expect("profile json");
    assert_eq!(
        profile["nip05_handle"].as_str(),
        Some(valid_handle.as_str()),
        "nip05_handle should be synced from kind:0"
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

    // Step 5: Verify the handle was CLEARED (not set to the off-domain value).
    let profile_resp2 = http_client
        .get(format!("{}/api/users/{}/profile", http, pubkey_hex))
        .header("X-Pubkey", &pubkey_hex)
        .send()
        .await
        .expect("GET profile after off-domain kind:0");
    assert_eq!(profile_resp2.status(), 200);
    let profile2: serde_json::Value = profile_resp2.json().await.expect("profile json");
    let handle_after = profile2["nip05_handle"].as_str().unwrap_or("");
    assert!(
        handle_after.is_empty() || handle_after == "null",
        "nip05_handle should be cleared after off-domain kind:0, got: {:?}",
        profile2["nip05_handle"]
    );

    // Step 6: Confirm NIP-05 no longer resolves.
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

    // Set agent's channel_add_policy to "nobody" via REST.
    let http_client = reqwest::Client::new();
    let resp = http_client
        .put(format!(
            "{}/api/users/me/channel-add-policy",
            relay_http_url()
        ))
        .header("X-Pubkey", &agent_pubkey_hex)
        .json(&serde_json::json!({ "channel_add_policy": "nobody" }))
        .send()
        .await
        .expect("set policy request");
    assert_eq!(resp.status(), 200, "set policy failed");

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

/// NIP-29 kind 9000 (PUT_USER): self-add bypasses "nobody" policy — an agent can always add itself.
#[tokio::test]
#[ignore]
async fn test_nip29_put_user_self_add_bypasses_policy() {
    let url = relay_url();

    let agent_keys = Keys::generate();
    let agent_pubkey_hex = agent_keys.public_key().to_hex();

    // Set agent's channel_add_policy to "nobody" via REST.
    let http_client = reqwest::Client::new();
    let resp = http_client
        .put(format!(
            "{}/api/users/me/channel-add-policy",
            relay_http_url()
        ))
        .header("X-Pubkey", &agent_pubkey_hex)
        .json(&serde_json::json!({ "channel_add_policy": "nobody" }))
        .send()
        .await
        .expect("set policy request");
    assert_eq!(resp.status(), 200, "set policy failed");

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

    // Set agent's channel_add_policy to "owner_only" via REST.
    let http_client = reqwest::Client::new();
    let resp = http_client
        .put(format!(
            "{}/api/users/me/channel-add-policy",
            relay_http_url()
        ))
        .header("X-Pubkey", &agent_pubkey_hex)
        .json(&serde_json::json!({ "channel_add_policy": "owner_only" }))
        .send()
        .await
        .expect("set policy request");
    assert_eq!(resp.status(), 200, "set policy failed");

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
        .post(format!("{}/api/events", relay_http_url()))
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
        .post(format!("{}/api/events", relay_http_url()))
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
        .post(format!("{}/api/events", relay_http_url()))
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

// ─── Private channel membership permission tests ───────────────────────────────

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
async fn add_member_ws(
    client: &mut BuzzTestClient,
    channel_id: &str,
    target_pubkey_hex: &str,
    signer: &Keys,
) -> (bool, String) {
    let h_tag = Tag::parse(["h", channel_id]).unwrap();
    let p_tag = Tag::parse(["p", target_pubkey_hex]).unwrap();
    let event = EventBuilder::new(Kind::Custom(9000), "")
        .tags([h_tag, p_tag])
        .sign_with_keys(signer)
        .unwrap();

    let ok = client.send_event(event).await.expect("send PUT_USER event");
    (ok.accepted, ok.message)
}

/// Submit a kind:9000 PUT_USER event with a role tag over WebSocket.
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
        .tags([h_tag, p_tag, role_tag])
        .sign_with_keys(signer)
        .unwrap();

    let ok = client
        .send_event(event)
        .await
        .expect("send PUT_USER event with role");
    (ok.accepted, ok.message)
}

/// Any member of a private channel can invite another user (Slack model).
#[tokio::test]
#[ignore]
async fn test_private_channel_any_member_can_invite() {
    let url = relay_url();
    let owner_keys = Keys::generate();
    let member_keys = Keys::generate();
    let invitee_keys = Keys::generate();

    // Connect as owner and create a private channel.
    let mut owner_client = BuzzTestClient::connect(&url, &owner_keys)
        .await
        .expect("connect as owner");
    let channel_id = create_private_channel_ws(&mut owner_client, &owner_keys).await;

    // Owner adds member_keys as a regular member.
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

    // Regular member invites a third user — this should succeed.
    let (accepted, msg) = add_member_ws(
        &mut member_client,
        &channel_id,
        &invitee_keys.public_key().to_hex(),
        &member_keys,
    )
    .await;
    assert!(
        accepted,
        "regular member should be able to invite to private channel, got: {msg}"
    );

    owner_client.disconnect().await.expect("disconnect owner");
    member_client.disconnect().await.expect("disconnect member");
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
