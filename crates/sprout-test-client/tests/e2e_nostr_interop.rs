//! End-to-end integration tests for Nostr interop features:
//! NIP-50 search, NIP-10 threads, NIP-17 gift wraps, and DM discovery.
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
//! cargo test --test e2e_nostr_interop -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_nostr_interop -- --ignored
//! ```

use std::time::Duration;

use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use sprout_test_client::{RelayMessage, SproutTestClient, TestClientError};

// ── Helpers ───────────────────────────────────────────────────────────────────

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

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Create a real channel in the DB via REST so the relay accepts events for it.
async fn create_test_channel(keys: &Keys) -> String {
    let client = reqwest::Client::new();
    let pubkey_hex = keys.public_key().to_hex();
    let channel_uuid = uuid::Uuid::new_v4();
    let channel_name = format!("interop-e2e-{}", channel_uuid);

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

/// Send a message via a signed kind:9 event and return the event_id hex.
async fn send_rest_message(keys: &Keys, channel_id: &str, content: &str) -> String {
    let client = reqwest::Client::new();
    let pubkey_hex = keys.public_key().to_hex();
    let event = EventBuilder::new(Kind::Custom(9), content)
        .tags(vec![Tag::parse(["h", channel_id]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    let resp = client
        .post(format!("{}/api/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .expect("submit send-message event");
    assert!(
        resp.status().is_success(),
        "send message failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse event response");
    body["event_id"].as_str().expect("event_id").to_string()
}

/// Create a DM via REST and return the channel_id UUID string.
async fn create_dm(requester_keys: &Keys, other_pubkey_hex: &str) -> String {
    let client = reqwest::Client::new();
    let url = format!("{}/api/dms", relay_http_url());
    let pubkey_hex = requester_keys.public_key().to_hex();
    let resp = client
        .post(&url)
        .header("X-Pubkey", &pubkey_hex)
        .json(&serde_json::json!({ "pubkeys": [other_pubkey_hex] }))
        .send()
        .await
        .expect("create DM request");
    assert!(
        resp.status().is_success(),
        "create DM failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse DM response");
    body["channel_id"].as_str().expect("channel_id").to_string()
}

// ── Phase 1: NIP-50 Search ────────────────────────────────────────────────────

/// Send a message with unique content, then search for it.
/// Verify: events returned before EOSE, content matches, EOSE received.
/// Verify: no live events delivered after EOSE (search is one-shot).
#[tokio::test]
#[ignore]
async fn test_nip50_search_returns_results_and_eose() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Send a message with a unique search token.
    let unique_token = format!("searchtoken_{}", uuid::Uuid::new_v4().simple());
    let content = format!("Hello world {unique_token}");

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let ok = client
        .send_text_message(&keys, &channel, &content, 9)
        .await
        .expect("send message");
    assert!(ok.accepted, "relay rejected message: {}", ok.message);

    // Small delay to allow indexing.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Subscribe with NIP-50 search filter.
    let sid = sub_id("nip50-search");
    let filter = Filter::new()
        .kind(Kind::Custom(9))
        .search(&unique_token)
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    // Collect until EOSE — should find our message.
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect until EOSE");

    assert!(
        !events.is_empty(),
        "expected at least one search result, got none"
    );
    assert!(
        events.iter().any(|e| e.content.contains(&unique_token)),
        "search result content does not contain unique token. events: {:?}",
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );

    // Search is one-shot: send another message and verify it does NOT arrive.
    let ok2 = client
        .send_text_message(&keys, &channel, "post-eose message", 9)
        .await
        .expect("send post-eose message");
    assert!(ok2.accepted, "relay rejected post-eose message");

    let result = client.recv_event(Duration::from_secs(2)).await;
    match result {
        Err(TestClientError::Timeout) => { /* expected — search is one-shot */ }
        Ok(RelayMessage::Event { event, .. }) => {
            panic!(
                "search subscription delivered live event after EOSE (kind={}): {}",
                event.kind.as_u16(),
                event.content
            );
        }
        Ok(_other) => {
            // NOTICE or other non-event messages are acceptable.
        }
        Err(_) => {
            // Any other error (e.g. connection closed) is also acceptable here.
        }
    }

    client.disconnect().await.expect("disconnect");
}

/// Subscribe with mixed search + non-search filters.
/// Verify: relay sends CLOSED with error message containing "mixed".
#[tokio::test]
#[ignore]
async fn test_nip50_search_mixed_filters_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let sid = sub_id("nip50-mixed");

    // Filter 1: has search
    let filter_search = Filter::new()
        .kind(Kind::Custom(9))
        .search("hello")
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    // Filter 2: no search
    let filter_plain = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter_search, filter_plain])
        .await
        .expect("send REQ");

    // Drain until CLOSED.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv message");
        match &m {
            RelayMessage::Eose { .. } | RelayMessage::Event { .. } => continue,
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
                message.to_lowercase().contains("mixed"),
                "expected 'mixed' in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// Subscribe with a search filter that matches nothing.
/// Verify: EOSE received with no events.
#[tokio::test]
#[ignore]
async fn test_nip50_search_empty_results() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let sid = sub_id("nip50-empty");
    // Must include kinds to avoid triggering P_GATED_KINDS check (wildcard
    // kinds match gift-wrap/membership kinds which require #p filter).
    let filter = Filter::new()
        .search("nonexistent_gibberish_xyz123_zzzzzz")
        .kind(Kind::Custom(9));

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect until EOSE");

    assert!(
        events.is_empty(),
        "expected no results for gibberish search, got {} events",
        events.len()
    );

    client.disconnect().await.expect("disconnect");
}

// ── Phase 2: NIP-10 Threads ───────────────────────────────────────────────────

/// Send a root message via REST, then send a WS reply with NIP-10 e-tags.
/// Verify: relay accepts the reply. Query thread via REST and verify reply appears.
#[tokio::test]
#[ignore]
async fn test_nip10_thread_reply_creates_metadata() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Send root message via REST.
    let root_event_id = send_rest_message(&keys, &channel, "root message for NIP-10 test").await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    // Build reply event with NIP-10 e-tag.
    let h_tag = Tag::parse(["h", &channel]).expect("h tag");
    let e_reply_tag = Tag::parse(["e", &root_event_id, "", "reply"]).expect("e reply tag");

    let reply_content = format!("reply to root {}", uuid::Uuid::new_v4());
    let reply_event = EventBuilder::new(Kind::Custom(9), &reply_content)
        .tags([h_tag, e_reply_tag])
        .sign_with_keys(&keys)
        .expect("sign reply");

    let ok = client.send_event(reply_event).await.expect("send reply");
    assert!(ok.accepted, "relay rejected reply: {}", ok.message);

    // Query thread via REST to verify reply is recorded.
    let http_client = reqwest::Client::new();
    let thread_url = format!(
        "{}/api/channels/{}/threads/{}",
        relay_http_url(),
        channel,
        root_event_id
    );
    let resp = http_client
        .get(&thread_url)
        .header("X-Pubkey", &keys.public_key().to_hex())
        .send()
        .await
        .expect("get thread request");
    assert!(
        resp.status().is_success(),
        "get thread failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse thread response");

    // The thread response should contain the reply somewhere in replies/events.
    let body_str = body.to_string();
    assert!(
        body_str.contains(&reply_content),
        "thread response does not contain reply content. body: {body_str}"
    );

    client.disconnect().await.expect("disconnect");
}

/// Send a reply via WS with e-tags pointing to a nonexistent parent.
/// Verify: relay rejects with OK false, message contains "parent not found".
#[tokio::test]
#[ignore]
async fn test_nip10_unknown_parent_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    // Use a random 32-byte hex as a nonexistent parent ID.
    let fake_parent_id = hex::encode([0xdeu8; 32]);

    let h_tag = Tag::parse(["h", &channel]).expect("h tag");
    let e_reply_tag = Tag::parse(["e", &fake_parent_id, "", "reply"]).expect("e reply tag");

    let event = EventBuilder::new(Kind::Custom(9), "orphan reply")
        .tags([h_tag, e_reply_tag])
        .sign_with_keys(&keys)
        .expect("sign event");

    let ok = client.send_event(event).await.expect("send event");

    assert!(
        !ok.accepted,
        "relay should have rejected reply to nonexistent parent, but accepted it"
    );
    assert!(
        ok.message.to_lowercase().contains("parent not found")
            || ok.message.to_lowercase().contains("not found"),
        "expected 'parent not found' in rejection message, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// Send a root message, then send a reply with a wrong root tag.
/// Verify: relay rejects with OK false, message contains "root tag does not match".
#[tokio::test]
#[ignore]
async fn test_nip10_root_mismatch_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Send a real root message.
    let real_parent_id = send_rest_message(&keys, &channel, "real parent for mismatch test").await;

    // Use a different random ID as the claimed root.
    let wrong_root_id = hex::encode([0xabu8; 32]);

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let h_tag = Tag::parse(["h", &channel]).expect("h tag");
    // wrong_root as "root" marker, real_parent as "reply" marker — mismatch.
    let e_root_tag = Tag::parse(["e", &wrong_root_id, "", "root"]).expect("e root tag");
    let e_reply_tag = Tag::parse(["e", &real_parent_id, "", "reply"]).expect("e reply tag");

    let event = EventBuilder::new(Kind::Custom(9), "reply with wrong root")
        .tags([h_tag, e_root_tag, e_reply_tag])
        .sign_with_keys(&keys)
        .expect("sign event");

    let ok = client.send_event(event).await.expect("send event");

    assert!(
        !ok.accepted,
        "relay should have rejected root mismatch, but accepted it"
    );
    assert!(
        ok.message
            .to_lowercase()
            .contains("root tag does not match")
            || ok.message.to_lowercase().contains("root"),
        "expected root mismatch in rejection message, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

// ── Phase 3: NIP-17 Gift Wraps ────────────────────────────────────────────────

/// Create a kind:1059 event signed by an ephemeral key (different from auth key).
/// Verify: relay accepts despite pubkey mismatch (gift wraps are exempt).
#[tokio::test]
#[ignore]
async fn test_nip17_gift_wrap_accepted() {
    let url = relay_url();
    let auth_keys = Keys::generate();
    let recipient_keys = Keys::generate();

    let mut client = SproutTestClient::connect(&url, &auth_keys)
        .await
        .expect("connect");

    // Sign with a different ephemeral key — not the auth key.
    let ephemeral_keys = Keys::generate();
    let p_tag = Tag::parse(["p", &recipient_keys.public_key().to_hex()]).expect("p tag");

    let gift_wrap = EventBuilder::new(Kind::Custom(1059), "encrypted-content")
        .tags([p_tag])
        .sign_with_keys(&ephemeral_keys)
        .expect("sign gift wrap");

    let ok = client.send_event(gift_wrap).await.expect("send gift wrap");

    assert!(
        ok.accepted,
        "relay rejected gift wrap (kind:1059): {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// Subscribe with `{kinds:[1059]}` and no `#p` filter.
/// Verify: relay sends CLOSED with message containing "p-gated" or "#p".
#[tokio::test]
#[ignore]
async fn test_nip17_gift_wrap_requires_p_filter() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let sid = sub_id("nip17-no-p");
    // No #p filter — should be rejected.
    let filter = Filter::new().kind(Kind::Custom(1059));

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("send REQ");

    // Drain until CLOSED.
    let msg = loop {
        let m = client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv message");
        match &m {
            RelayMessage::Eose { .. } | RelayMessage::Event { .. } => continue,
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
            let msg_lower = message.to_lowercase();
            assert!(
                msg_lower.contains("p-gated")
                    || msg_lower.contains("#p")
                    || msg_lower.contains("restricted"),
                "expected p-gated rejection in CLOSED message, got: {message}"
            );
        }
        other => panic!("expected CLOSED, got {other:?}"),
    }

    client.disconnect().await.expect("disconnect");
}

/// User A sends a kind:1059 gift wrap with `#p` = user B's pubkey.
/// User B subscribes with `{kinds:[1059], #p:[B_pubkey]}`.
/// Verify: B receives the gift wrap event.
#[tokio::test]
#[ignore]
async fn test_nip17_gift_wrap_recipient_receives() {
    let url = relay_url();
    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let b_pubkey_hex = keys_b.public_key().to_hex();

    // Connect B first and subscribe.
    let mut client_b = SproutTestClient::connect(&url, &keys_b)
        .await
        .expect("client B connect");

    let sid_b = sub_id("nip17-recv-b");
    let filter_b = Filter::new().kind(Kind::Custom(1059)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::P),
        [b_pubkey_hex.as_str()],
    );

    client_b
        .subscribe(&sid_b, vec![filter_b])
        .await
        .expect("client B subscribe");

    // Drain EOSE so we're ready for live events.
    client_b
        .collect_until_eose(&sid_b, Duration::from_secs(5))
        .await
        .expect("client B EOSE");

    // Connect A and send gift wrap addressed to B.
    let mut client_a = SproutTestClient::connect(&url, &keys_a)
        .await
        .expect("client A connect");

    let ephemeral_keys = Keys::generate();
    let p_tag = Tag::parse(["p", &b_pubkey_hex]).expect("p tag");
    let unique_content = format!("gift-wrap-{}", uuid::Uuid::new_v4());

    let gift_wrap = EventBuilder::new(Kind::Custom(1059), &unique_content)
        .tags([p_tag])
        .sign_with_keys(&ephemeral_keys)
        .expect("sign gift wrap");

    let ok = client_a
        .send_event(gift_wrap)
        .await
        .expect("send gift wrap");
    assert!(ok.accepted, "relay rejected gift wrap: {}", ok.message);

    // B should receive the gift wrap.
    let msg = client_b
        .recv_event(Duration::from_secs(5))
        .await
        .expect("client B recv gift wrap");

    match msg {
        RelayMessage::Event {
            subscription_id,
            event,
        } => {
            assert_eq!(
                subscription_id, sid_b,
                "event delivered to wrong subscription"
            );
            assert_eq!(
                event.kind,
                Kind::Custom(1059),
                "expected kind:1059, got {}",
                event.kind.as_u16()
            );
            assert_eq!(event.content, unique_content, "gift wrap content mismatch");
        }
        other => panic!("expected EVENT kind:1059, got {other:?}"),
    }

    client_a.disconnect().await.expect("disconnect A");
    client_b.disconnect().await.expect("disconnect B");
}

// ── Phase 4: DM Discovery ─────────────────────────────────────────────────────

/// Create a DM via REST, then subscribe as a participant to verify discovery events.
/// Verify: kind:39000 event received with `hidden` and `private` tags.
/// Verify: kind:44100 membership notification received.
#[tokio::test]
#[ignore]
async fn test_dm_discovery_events_emitted() {
    let url = relay_url();
    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let a_pubkey_hex = keys_a.public_key().to_hex();
    let b_pubkey_hex = keys_b.public_key().to_hex();

    // Connect A and subscribe to discovery + membership events BEFORE creating the DM.
    let mut client_a = SproutTestClient::connect(&url, &keys_a)
        .await
        .expect("client A connect");

    let sid_discovery = sub_id("dm-discovery-39000");
    let sid_membership = sub_id("dm-discovery-44100");

    // We'll subscribe with #p = A's pubkey for membership notifications.
    let membership_filter = Filter::new().kind(Kind::Custom(44100)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::P),
        [a_pubkey_hex.as_str()],
    );

    client_a
        .subscribe(&sid_membership, vec![membership_filter])
        .await
        .expect("subscribe membership");

    client_a
        .collect_until_eose(&sid_membership, Duration::from_secs(5))
        .await
        .expect("membership EOSE");

    // Create the DM via REST (A creates DM with B).
    let channel_id = create_dm(&keys_a, &b_pubkey_hex).await;

    // Subscribe to 39000 discovery events for this specific DM channel.
    let discovery_filter = Filter::new().kind(Kind::Custom(39000)).custom_tag(
        SingleLetterTag::lowercase(Alphabet::D),
        [channel_id.as_str()],
    );

    client_a
        .subscribe(&sid_discovery, vec![discovery_filter])
        .await
        .expect("subscribe discovery");

    // Collect 39000 events from history (EOSE).
    let discovery_events = client_a
        .collect_until_eose(&sid_discovery, Duration::from_secs(10))
        .await
        .expect("discovery EOSE");

    // Verify kind:39000 event has `hidden` and `private` tags.
    assert!(
        !discovery_events.is_empty(),
        "expected kind:39000 discovery event for DM channel {channel_id}, got none"
    );

    let discovery_event = &discovery_events[0];
    assert_eq!(
        discovery_event.kind,
        Kind::Custom(39000),
        "expected kind:39000, got {}",
        discovery_event.kind.as_u16()
    );

    let tags: Vec<Vec<String>> = discovery_event
        .tags
        .iter()
        .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
        .collect();

    let has_hidden = tags.iter().any(|t| t[0] == "hidden");
    let has_private = tags.iter().any(|t| t[0] == "private");

    assert!(
        has_hidden,
        "kind:39000 missing 'hidden' tag. tags: {tags:?}"
    );
    assert!(
        has_private,
        "kind:39000 missing 'private' tag. tags: {tags:?}"
    );

    // Verify kind:44100 membership notification was received for A.
    let membership_msg = client_a
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv kind:44100 membership notification");

    match membership_msg {
        RelayMessage::Event { event, .. } => {
            assert_eq!(
                event.kind,
                Kind::Custom(44100),
                "expected kind:44100 membership notification, got {}",
                event.kind.as_u16()
            );

            let tags: Vec<Vec<String>> = event
                .tags
                .iter()
                .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
                .collect();

            let has_p = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "p" && t[1] == a_pubkey_hex);
            assert!(
                has_p,
                "kind:44100 missing p tag = A's pubkey. tags: {tags:?}"
            );

            let has_h = tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "h" && t[1] == channel_id);
            assert!(
                has_h,
                "kind:44100 missing h tag = DM channel id. tags: {tags:?}"
            );
        }
        other => panic!("expected EVENT kind:44100, got {other:?}"),
    }

    client_a.disconnect().await.expect("disconnect");
}

// ── Phase 5: Regression Tests ─────────────────────────────────────────────────

/// Send a NIP-10 reply via WS, then query top-level channel messages via REST.
/// Verify: the reply does NOT appear in top-level results (only the root should).
/// This proves thread_metadata was created and replies are hidden from top-level.
#[tokio::test]
#[ignore]
async fn test_nip10_thread_reply_not_in_top_level() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Send root message via REST.
    let root_content = format!("root-toplevel-{}", uuid::Uuid::new_v4());
    let root_event_id = send_rest_message(&keys, &channel, &root_content).await;

    // Send reply via WS with NIP-10 e-tag.
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let reply_content = format!("reply-hidden-{}", uuid::Uuid::new_v4());
    let h_tag = Tag::parse(["h", &channel]).expect("h tag");
    let e_reply_tag = Tag::parse(["e", &root_event_id, "", "reply"]).expect("e reply tag");

    let reply_event = EventBuilder::new(Kind::Custom(9), &reply_content)
        .tags([h_tag, e_reply_tag])
        .sign_with_keys(&keys)
        .expect("sign reply");

    let ok = client.send_event(reply_event).await.expect("send reply");
    assert!(ok.accepted, "relay rejected reply: {}", ok.message);

    client.disconnect().await.expect("disconnect");

    // Query top-level messages via REST.
    let http_client = reqwest::Client::new();
    let messages_url = format!(
        "{}/api/channels/{}/messages?limit=50",
        relay_http_url(),
        channel
    );
    let resp = http_client
        .get(&messages_url)
        .header("X-Pubkey", &keys.public_key().to_hex())
        .send()
        .await
        .expect("get messages request");
    assert!(
        resp.status().is_success(),
        "get messages failed: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse messages response");
    let body_str = body.to_string();

    // Root should be present.
    assert!(
        body_str.contains(&root_content),
        "top-level messages missing root content. body: {body_str}"
    );
    // Reply must NOT appear at top level.
    assert!(
        !body_str.contains(&reply_content),
        "reply content should NOT appear in top-level messages, but it does. body: {body_str}"
    );
}

/// Send a kind:1059 gift wrap AND a kind:9 message with the same unique content.
/// Query Typesense directly to prove the gift wrap was NOT indexed while the
/// kind:9 message WAS. This bypasses all relay-level filtering (channel_id, #p)
/// and tests the actual indexing skip in dispatch_persistent_event.
///
/// Requires TYPESENSE_URL and TYPESENSE_API_KEY env vars (defaults to dev values).
#[tokio::test]
#[ignore]
async fn test_nip17_gift_wrap_not_searchable() {
    let url = relay_url();
    let keys_a = Keys::generate();
    let keys_b = Keys::generate();
    let channel = create_test_channel(&keys_a).await;

    let mut client = SproutTestClient::connect(&url, &keys_a)
        .await
        .expect("connect");

    let unique_token = format!("giftwrap-nosearch-{}", uuid::Uuid::new_v4().simple());

    // 1. Send kind:1059 gift wrap.
    let ephemeral_keys = Keys::generate();
    let p_tag = Tag::parse(["p", &keys_b.public_key().to_hex()]).expect("p tag");
    let gift_wrap = EventBuilder::new(Kind::Custom(1059), &unique_token)
        .tags([p_tag])
        .sign_with_keys(&ephemeral_keys)
        .expect("sign gift wrap");
    let ok = client.send_event(gift_wrap).await.expect("send gift wrap");
    assert!(ok.accepted, "relay rejected gift wrap: {}", ok.message);

    // 2. Send kind:9 control message with the same content.
    let ok2 = client
        .send_text_message(&keys_a, &channel, &unique_token, 9)
        .await
        .expect("send kind:9");
    assert!(ok2.accepted, "relay rejected kind:9: {}", ok2.message);

    client.disconnect().await.expect("disconnect");

    // Wait for async Typesense indexing.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // 3. Query Typesense DIRECTLY — bypasses all relay-level filtering.
    let ts_url =
        std::env::var("TYPESENSE_URL").unwrap_or_else(|_| "http://localhost:8108".to_string());
    let ts_key =
        std::env::var("TYPESENSE_API_KEY").unwrap_or_else(|_| "sprout_dev_key".to_string());

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{ts_url}/multi_search"))
        .header("X-TYPESENSE-API-KEY", &ts_key)
        .json(&serde_json::json!({
            "searches": [{
                "collection": "events",
                "q": unique_token,
                "query_by": "content",
                "per_page": 10
            }]
        }))
        .send()
        .await
        .expect("Typesense multi_search request");

    assert!(
        resp.status().is_success(),
        "Typesense returned {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("parse Typesense response");

    let hits = body["results"][0]["hits"].as_array().expect("hits array");

    // Control: kind:9 IS indexed.
    let has_kind9 = hits
        .iter()
        .any(|h| h["document"]["kind"].as_i64() == Some(9));
    assert!(
        has_kind9,
        "kind:9 control message not found in Typesense — indexing broken"
    );

    // Assertion: kind:1059 is NOT indexed.
    let has_kind1059 = hits
        .iter()
        .any(|h| h["document"]["kind"].as_i64() == Some(1059));
    assert!(
        !has_kind1059,
        "kind:1059 found in Typesense — gift wraps must NOT be indexed. hits: {hits:?}"
    );
}

/// Send 3 messages with varying relevance to a query, wait for indexing, then search.
/// Verify: the exact-match message is present in results (relevance-based, not just chronological).
#[tokio::test]
#[ignore]
async fn test_nip50_search_relevance_order() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Unique prefix to isolate this test's messages from other test runs.
    let prefix = uuid::Uuid::new_v4().simple().to_string();
    let msg1 = format!("{prefix} alpha bravo charlie"); // oldest, exact match
    let msg2 = format!("{prefix} delta echo foxtrot"); // middle, no match
    let msg3 = format!("{prefix} alpha bravo"); // newest, partial match

    let id1 = send_rest_message(&keys, &channel, &msg1).await;
    send_rest_message(&keys, &channel, &msg2).await;
    send_rest_message(&keys, &channel, &msg3).await;

    // Wait for Typesense indexing.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let sid = sub_id("nip50-relevance");
    let query = format!("{prefix} alpha bravo charlie");
    let filter = Filter::new()
        .kind(Kind::Custom(9))
        .search(&query)
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect until EOSE");

    // Must have at least 1 result.
    assert!(!events.is_empty(), "expected search results, got none");

    // The FIRST result must be the exact-match message (msg1), not the newer
    // partial match (msg3). This proves relevance ordering, not chronological.
    let first = &events[0];
    assert!(
        first.id.to_hex() == id1 || first.content.contains("alpha bravo charlie"),
        "expected exact-match message as FIRST result (relevance order), \
         but got: '{}'. All results: {:?}",
        first.content,
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );

    client.disconnect().await.expect("disconnect");
}

/// Send a kind:9 message, then subscribe with two filters in one REQ:
///   Filter A: wrong author — will NOT match
///   Filter B: no author restriction — WILL match
/// Verify: the message IS returned, proving dedup happens after per-filter
/// acceptance and OR semantics are preserved.
#[tokio::test]
#[ignore]
async fn test_historical_req_dedup_preserves_or_semantics() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    let content = format!("dedup-or-{}", uuid::Uuid::new_v4());
    let event_id = send_rest_message(&keys, &channel, &content).await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    // Generate a random wrong author key.
    let wrong_author = Keys::generate();

    let sid = sub_id("dedup-or");

    // Filter A: restricts to wrong author — will not match our message.
    let filter_a = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()])
        .author(wrong_author.public_key());

    // Filter B: no author restriction — will match our message.
    let filter_b = Filter::new()
        .kind(Kind::Custom(9))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()]);

    client
        .subscribe(&sid, vec![filter_a, filter_b])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect until EOSE");

    // Our message must be returned (filter B matches even though filter A doesn't).
    assert!(
        events
            .iter()
            .any(|e| e.id.to_hex() == event_id || e.content == content),
        "expected message to be returned via filter B, but it was missing. \
         events: {:?}",
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );

    client.disconnect().await.expect("disconnect");
}

/// REQ with `kinds:[]` must return zero historical events and EOSE.
/// This proves the empty-kinds sentinel is honored end-to-end (DB returns
/// zero rows instead of matching all kinds).
#[tokio::test]
#[ignore]
async fn test_empty_kinds_returns_zero_events() {
    let url = relay_url();
    let keys = Keys::generate();
    let channel = create_test_channel(&keys).await;

    // Send a message so there IS data in the channel.
    send_rest_message(&keys, &channel, "should not appear").await;

    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let sid = sub_id("empty-kinds");
    // kinds:[] = match nothing per NIP-01.
    let filter = Filter::new()
        .kinds(vec![] as Vec<Kind>)
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
        "kinds:[] must return zero events, got {}",
        events.len()
    );

    client.disconnect().await.expect("disconnect");
}
