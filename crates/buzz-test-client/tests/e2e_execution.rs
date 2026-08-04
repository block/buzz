//! End-to-end integration tests for NIP-EN execution-node event kinds.
//!
//! Covers the relay behaviour of the three kinds added for execution nodes:
//! - kind:30630 — execution-node announcement: addressable/replaceable,
//!   global-only (a stray `h` tag never channel-scopes it), and publicly
//!   readable — it carries an explicitly sanitized projection.
//! - kind:24201 — execution-node command: encrypted payload addressed to the
//!   node via `#p`. P-gated on REQ/COUNT (filters naming the kind must carry
//!   `#p = [self]`, with no `ids` exemption), result-gated on pull paths, and
//!   fanned out only to the `#p` addressee.
//! - kind:24202 — execution-node receipt: encrypted payload addressed to the
//!   owner via `#p`; same gating as commands.
//!
//! Commands and receipts sit in the ephemeral kind range: a WS EVENT is
//! fanned out without storage, and the store layer refuses to persist them
//! ("ephemeral events must not be stored"). HTTP `POST /events` rejects
//! them up front with a clean 400 ("only accepted via WebSocket") — the
//! same transport gate gift wraps and presence updates use — asserted
//! below. The pull-path gates (p-gate on /query + /count and REQ/COUNT)
//! are asserted on the filter shapes themselves; per-event result gating
//! is unit-tested in `buzz-core/src/filter.rs`.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_execution -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use nostr::{Alphabet, EventBuilder, EventId, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use reqwest::Client;
use serde_json::Value;

const KIND_EXECUTION_NODE_COMMAND: u16 = 24201;
const KIND_EXECUTION_NODE_RECEIPT: u16 = 24202;
const KIND_EXECUTION_NODE_ANNOUNCEMENT: u16 = 30630;

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
    format!("e2e-nipen-{name}-{}", uuid::Uuid::new_v4())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

fn p_tag() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

/// Build a kind:30630 announcement (sanitized node status, keyed by `d`).
fn build_announcement(
    keys: &Keys,
    node_id: &str,
    content: &str,
    extra_tags: Vec<Tag>,
) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", node_id]).unwrap()];
    tags.extend(extra_tags);
    EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build an announcement with an explicit `created_at`, so replacement-ordering
/// tests are deterministic instead of racing the whole-second timestamp the
/// default builder stamps.
fn build_announcement_at(
    keys: &Keys,
    node_id: &str,
    content: &str,
    created_at: Timestamp,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT), content)
        .tags(vec![Tag::parse(["d", node_id]).unwrap()])
        .custom_created_at(created_at)
        .sign_with_keys(keys)
        .unwrap()
}

/// Build a kind:24201/24202 event addressed to `recipient_hex` via `#p`.
/// The content stands in for the NIP-44 ciphertext the real node exchanges.
fn build_addressed_event(
    keys: &Keys,
    kind: u16,
    recipient_hex: &str,
    content: &str,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(vec![Tag::parse(["p", recipient_hex]).unwrap()])
        .sign_with_keys(keys)
        .unwrap()
}

/// Submit an event via the HTTP bridge without asserting success.
/// Returns (status, body) so rejection tests can assert on the status code.
async fn submit_event_http_raw(client: &Client, keys: &Keys, event: &nostr::Event) -> (u16, Value) {
    let pubkey_hex = keys.public_key().to_hex();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    (status, body)
}

/// Submit an event via the HTTP bridge and return (accepted, message).
async fn submit_event_http(client: &Client, keys: &Keys, event: &nostr::Event) -> (bool, String) {
    let (status, body) = submit_event_http_raw(client, keys, event).await;
    if status == 200 {
        let accepted = body["accepted"].as_bool().unwrap_or(false);
        let message = body["message"].as_str().unwrap_or("").to_string();
        (accepted, message)
    } else {
        // Rejections come back as `api_error` → `{"error": msg}` with no
        // `accepted`/`message` fields (see relay api/mod.rs).
        let message = body["error"].as_str().unwrap_or("").to_string();
        (false, message)
    }
}

/// Query events via the HTTP bridge without asserting success.
/// Returns (status, body) so gating tests can assert on 403s.
async fn query_events_http_raw(
    client: &Client,
    pubkey_hex: &str,
    filters: Vec<Filter>,
) -> (u16, Value) {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("query events");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse query response");
    (status, body)
}

/// Query events via the HTTP bridge, asserting success. Returns the JSON array.
async fn query_events_http(client: &Client, pubkey_hex: &str, filters: Vec<Filter>) -> Vec<Value> {
    let (status, body) = query_events_http_raw(client, pubkey_hex, filters).await;
    assert_eq!(status, 200, "query failed: {status} {body}");
    body.as_array()
        .cloned()
        .expect("query response is an array")
}

/// Count events via the HTTP bridge. Returns the count or (status, error).
async fn count_events_http(
    client: &Client,
    pubkey_hex: &str,
    filters: Vec<Filter>,
) -> Result<u64, (u16, String)> {
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("count events");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse count response");
    if status == 200 {
        Ok(body["count"].as_u64().unwrap_or(0))
    } else {
        let msg = body["error"].as_str().unwrap_or("").to_string();
        Err((status, msg))
    }
}

/// True if a JSON event from /query carries the given event id.
fn results_contain_id(results: &[Value], event_id_hex: &str) -> bool {
    results.iter().any(|e| e["id"] == event_id_hex)
}

/// Wait for the relay's terminal response to a REQ/COUNT on `sid` and assert
/// it is CLOSED with a "restricted:" message. Panics if the relay serves data
/// (EVENT/EOSE/COUNT) on the subscription instead.
async fn expect_closed_restricted(ws: &mut BuzzTestClient, sid: &str, context: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            panic!("timed out waiting for CLOSED ({context})");
        }
        match ws.recv_event(remaining).await.expect("recv response") {
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sid => {
                assert!(
                    message.contains("restricted:"),
                    "expected restricted CLOSED ({context}), got: {message}"
                );
                return;
            }
            RelayMessage::Event {
                subscription_id, ..
            } if subscription_id == sid => {
                panic!("expected CLOSED ({context}), relay delivered an EVENT instead");
            }
            RelayMessage::Eose { subscription_id } if subscription_id == sid => {
                panic!("expected CLOSED ({context}), relay sent EOSE instead");
            }
            RelayMessage::Count {
                subscription_id, ..
            } if subscription_id == sid => {
                panic!("expected CLOSED ({context}), relay answered the COUNT instead");
            }
            _ => {}
        }
    }
}

// ─── kind:30630 — announcements ──────────────────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_announcement_accepted_and_readable_by_other_users() {
    let client = http_client();
    let node_keys = Keys::generate();
    let reader_keys = Keys::generate();
    let reader_hex = reader_keys.public_key().to_hex();
    let node_id = format!("node-{}", uuid::Uuid::new_v4());
    let content = format!("sanitized-node-status-{}", uuid::Uuid::new_v4());

    let event = build_announcement(&node_keys, &node_id, &content, vec![]);
    let (accepted, msg) = submit_event_http(&client, &node_keys, &event).await;
    assert!(accepted, "announcement rejected: {msg}");

    // Announcements are NOT p-gated — a different authed user can query them.
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT))
        .author(node_keys.public_key());
    let results = query_events_http(&client, &reader_hex, vec![filter.clone()]).await;
    assert!(
        results.iter().any(|e| e["content"] == content),
        "non-author should be able to read a node announcement"
    );

    // COUNT is equally ungated for announcements.
    let count = count_events_http(&client, &reader_hex, vec![filter.clone()])
        .await
        .expect("count should succeed for a non-author on announcements");
    assert!(count >= 1, "non-author should count the announcement");

    // WS REQ by the non-author returns the announcement before EOSE.
    let mut ws = BuzzTestClient::connect(&relay_url(), &reader_keys)
        .await
        .expect("connect");
    let sid = sub_id("announcement-read");
    ws.subscribe(&sid, vec![filter]).await.expect("subscribe");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events");
    assert!(
        events.iter().any(|e| e.content == content),
        "non-author should receive the announcement via WS REQ"
    );
    ws.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_announcement_replacement_semantics() {
    // Announcements are addressable: same (pubkey, kind, d) replaces (LWW).
    let client = http_client();
    let node_keys = Keys::generate();
    let node_hex = node_keys.public_key().to_hex();
    let node_id = format!("node-{}", uuid::Uuid::new_v4());

    // Explicit distinct created_at values make replacement ordering
    // deterministic (same rationale as the NIP-ER replacement test): two
    // versions inside the same whole second would hit the stale-write
    // tiebreak and could keep v1.
    let v1_ts = Timestamp::now();
    let v2_ts = v1_ts + 2u64;
    let v1_content = format!("status-v1-{}", uuid::Uuid::new_v4());
    let v2_content = format!("status-v2-{}", uuid::Uuid::new_v4());

    let v1 = build_announcement_at(&node_keys, &node_id, &v1_content, v1_ts);
    let (accepted, msg) = submit_event_http(&client, &node_keys, &v1).await;
    assert!(accepted, "first announcement rejected: {msg}");

    let v2 = build_announcement_at(&node_keys, &node_id, &v2_content, v2_ts);
    let (accepted, msg) = submit_event_http(&client, &node_keys, &v2).await;
    assert!(accepted, "replacement announcement rejected: {msg}");

    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT))
        .author(node_keys.public_key());
    let results = query_events_http(&client, &node_hex, vec![filter]).await;

    let matching: Vec<&Value> = results
        .iter()
        .filter(|e| {
            e["tags"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .any(|t| t[0] == "d" && t[1] == node_id)
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "exactly one announcement version should remain after replacement, got {}",
        matching.len()
    );
    assert_eq!(
        matching[0]["content"], v2_content,
        "the surviving announcement should be the replacement"
    );
}

#[tokio::test]
#[ignore]
async fn test_announcement_with_stray_h_tag_stays_global() {
    // Announcements are global-only: a stray `h` tag must not channel-scope
    // them (the ingest pipeline sets channel_id = None), so the event is
    // accepted without any channel membership and remains globally readable.
    let client = http_client();
    let node_keys = Keys::generate();
    let reader_keys = Keys::generate();
    let node_id = format!("node-{}", uuid::Uuid::new_v4());
    let content = format!("sanitized-node-status-{}", uuid::Uuid::new_v4());
    let bogus_channel = uuid::Uuid::new_v4().to_string();

    let event = build_announcement(
        &node_keys,
        &node_id,
        &content,
        vec![Tag::parse(["h", &bogus_channel]).unwrap()],
    );
    let (accepted, msg) = submit_event_http(&client, &node_keys, &event).await;
    assert!(
        accepted,
        "announcement with stray h tag should be accepted as global: {msg}"
    );

    let filter = Filter::new()
        .kind(Kind::Custom(KIND_EXECUTION_NODE_ANNOUNCEMENT))
        .author(node_keys.public_key());
    let results =
        query_events_http(&client, &reader_keys.public_key().to_hex(), vec![filter]).await;
    assert!(
        results.iter().any(|e| e["content"] == content),
        "globally-stored announcement should be readable despite the stray h tag"
    );
}

// ─── kind:24201/24202 — commands and receipts ────────────────────────────────

#[tokio::test]
#[ignore]
async fn test_http_publish_of_commands_and_receipts_rejected_cleanly() {
    // Commands and receipts are WS-fanout-only: the HTTP bridge rejects them
    // up front with a clean 400 naming the transport rule — not a 500 from
    // the store layer's ephemeral guard. Announcements (kind:30630) stay
    // publishable over HTTP, asserted by the announcement tests above.
    let client = http_client();

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        let sender_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let recipient_hex = recipient_keys.public_key().to_hex();
        let content = format!("nip44-ciphertext-{}", uuid::Uuid::new_v4());
        let event = build_addressed_event(&sender_keys, kind, &recipient_hex, &content);

        let (status, body) = submit_event_http_raw(&client, &sender_keys, &event).await;
        assert_eq!(
            status, 400,
            "HTTP publish of kind {kind} must be a clean 400, got {status}: {body}"
        );
        // Rejections come back as `api_error` → `{"error": msg}`.
        let message = body["error"].as_str().unwrap_or("");
        assert!(
            message.contains("only accepted via WebSocket"),
            "kind {kind} rejection should name the WS-only transport rule, got: {message}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_command_and_receipt_live_fanout_only_to_addressee() {
    // WS-submitted commands/receipts are ephemeral: accepted, never stored,
    // and fanned out only to subscriptions that (a) survived the p-gate,
    // i.e. carry #p = [self], and (b) match the event's #p addressee.
    let url = relay_url();

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        let sender_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let bystander_keys = Keys::generate();
        let recipient_hex = recipient_keys.public_key().to_hex();
        let bystander_hex = bystander_keys.public_key().to_hex();

        // Recipient subscribes to events addressed to itself — this is the
        // buzz-node inbox filter shape ({kinds, #p: [self]}).
        let mut ws_recipient = BuzzTestClient::connect(&url, &recipient_keys)
            .await
            .expect("connect recipient");
        let sid_recipient = sub_id("exec-recipient");
        let recipient_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [recipient_hex.as_str()]);
        ws_recipient
            .subscribe(&sid_recipient, vec![recipient_filter])
            .await
            .expect("subscribe recipient");
        ws_recipient
            .collect_until_eose(&sid_recipient, Duration::from_secs(5))
            .await
            .expect("drain recipient EOSE");

        // Bystander subscribes to its own inbox — a legal subscription that
        // must never see traffic addressed to someone else.
        let mut ws_bystander = BuzzTestClient::connect(&url, &bystander_keys)
            .await
            .expect("connect bystander");
        let sid_bystander = sub_id("exec-bystander");
        let bystander_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [bystander_hex.as_str()]);
        ws_bystander
            .subscribe(&sid_bystander, vec![bystander_filter])
            .await
            .expect("subscribe bystander");
        ws_bystander
            .collect_until_eose(&sid_bystander, Duration::from_secs(5))
            .await
            .expect("drain bystander EOSE");

        // Sender publishes the addressed event over WS (ephemeral path).
        let mut ws_sender = BuzzTestClient::connect(&url, &sender_keys)
            .await
            .expect("connect sender");
        let content = format!("nip44-ciphertext-{}", uuid::Uuid::new_v4());
        let event = build_addressed_event(&sender_keys, kind, &recipient_hex, &content);
        let ok = ws_sender.send_event(event).await.expect("send event");
        assert!(ok.accepted, "kind {kind} rejected over WS: {}", ok.message);

        // The addressee receives it live.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut delivered = false;
        while !delivered {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                panic!("kind {kind} was not delivered to its #p addressee");
            }
            match ws_recipient.recv_event(remaining).await {
                Ok(RelayMessage::Event {
                    subscription_id,
                    event,
                }) if subscription_id == sid_recipient && event.content == content => {
                    delivered = true;
                }
                Ok(_) => {}
                Err(e) => panic!("recipient recv failed: {e}"),
            }
        }

        // The bystander must not receive it.
        match ws_bystander.recv_event(Duration::from_secs(2)).await {
            Err(TestClientError::Timeout) => {
                // Expected: nothing delivered to the non-addressee.
            }
            Ok(RelayMessage::Event { event, .. }) => {
                assert_ne!(
                    event.content, content,
                    "kind {kind} must NOT fan out to a non-addressee"
                );
            }
            Ok(_) => {
                // Other frame types (NOTICE, etc.) are fine.
            }
            Err(e) => panic!("unexpected bystander error: {e}"),
        }

        ws_sender.disconnect().await.expect("disconnect sender");
        ws_recipient
            .disconnect()
            .await
            .expect("disconnect recipient");
        ws_bystander
            .disconnect()
            .await
            .expect("disconnect bystander");
    }
}

#[tokio::test]
#[ignore]
async fn test_subscription_for_execution_kinds_requires_p_self() {
    // REQ filters that can match command/receipt kinds are p-gated: they must
    // carry #p = [self]. Naming the kind also revokes the usual `ids`
    // exemption — knowing an event id is NOT authorization for these kinds.
    let url = relay_url();
    let snoop_keys = Keys::generate();
    let victim_keys = Keys::generate();
    let victim_hex = victim_keys.public_key().to_hex();
    let some_event_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    let mut ws = BuzzTestClient::connect(&url, &snoop_keys)
        .await
        .expect("connect");

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        // (a) Kind filter without any #p → CLOSED restricted.
        let sid = sub_id("exec-no-p");
        ws.subscribe(&sid, vec![Filter::new().kind(Kind::Custom(kind))])
            .await
            .expect("subscribe");
        expect_closed_restricted(&mut ws, &sid, &format!("kind {kind} without #p")).await;

        // (b) Kind filter with #p naming someone else → CLOSED restricted.
        let sid = sub_id("exec-foreign-p");
        let filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [victim_hex.as_str()]);
        ws.subscribe(&sid, vec![filter]).await.expect("subscribe");
        expect_closed_restricted(&mut ws, &sid, &format!("kind {kind} with foreign #p")).await;

        // (c) Kind filter with explicit ids and no #p → still CLOSED: the ids
        // exemption does not apply to execution command/receipt kinds.
        let sid = sub_id("exec-ids-only");
        let filter = Filter::new()
            .kind(Kind::Custom(kind))
            .id(EventId::from_hex(some_event_id).unwrap());
        ws.subscribe(&sid, vec![filter]).await.expect("subscribe");
        expect_closed_restricted(&mut ws, &sid, &format!("kind {kind} ids-only")).await;
    }

    ws.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_http_query_of_execution_kinds_is_p_gated() {
    // Commands and receipts are never stored (ephemeral range), but their
    // filter shapes are still p-gated on the HTTP bridge: naming the kind
    // without `#p = [self]` is rejected with 403 up front, so the query
    // surface cannot even be probed. Explicit ids do not exempt these kinds.
    let client = http_client();
    let reader_keys = Keys::generate();
    let victim_keys = Keys::generate();
    let reader_hex = reader_keys.public_key().to_hex();
    let victim_hex = victim_keys.public_key().to_hex();
    let some_event_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        // Kind filter without #p hits the p-gate: 403. (This is the CLAUDE.md
        // "queries must specify kinds or they hit the p-gate" rule seen from
        // the other side.)
        let (status, _) = query_events_http_raw(
            &client,
            &reader_hex,
            vec![Filter::new().kind(Kind::Custom(kind))],
        )
        .await;
        assert_eq!(status, 403, "kind {kind} query without #p must 403");

        // Naming someone else's #p does not help: #p must equal self.
        let foreign_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [victim_hex.as_str()]);
        let (status, _) = query_events_http_raw(&client, &reader_hex, vec![foreign_filter]).await;
        assert_eq!(status, 403, "kind {kind} query with foreign #p must 403");

        // Explicit ids do not exempt execution kinds from the p-gate.
        let ids_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .id(EventId::from_hex(some_event_id).unwrap());
        let (status, _) = query_events_http_raw(&client, &reader_hex, vec![ids_filter]).await;
        assert_eq!(
            status, 403,
            "kind {kind} ids query without #p=self must 403"
        );

        // The addressee's own inbox shape ({kinds, #p: [self]}) is authorized.
        // Nothing is ever stored for these kinds, so the result is empty —
        // the gate decision is what's under test.
        let inbox_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [reader_hex.as_str()]);
        let results = query_events_http(&client, &reader_hex, vec![inbox_filter]).await;
        assert!(
            results.is_empty(),
            "ephemeral kind {kind} must never surface stored events"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_ws_published_execution_events_are_not_stored() {
    // WS-submitted commands/receipts are accepted and fanned out, but never
    // stored: the addressee's own historical pull right after publishing
    // returns nothing.
    let url = relay_url();
    let client = http_client();

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        let sender_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let recipient_hex = recipient_keys.public_key().to_hex();

        let mut ws_sender = BuzzTestClient::connect(&url, &sender_keys)
            .await
            .expect("connect sender");
        let content = format!("nip44-ciphertext-{}", uuid::Uuid::new_v4());
        let event = build_addressed_event(&sender_keys, kind, &recipient_hex, &content);
        let event_id_hex = event.id.to_hex();
        let ok = ws_sender.send_event(event).await.expect("send event");
        assert!(ok.accepted, "kind {kind} rejected over WS: {}", ok.message);
        ws_sender.disconnect().await.expect("disconnect sender");

        // The addressee's authorized inbox pull must come back empty — the
        // event was fan-out-only.
        let inbox_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [recipient_hex.as_str()]);
        let results = query_events_http(&client, &recipient_hex, vec![inbox_filter]).await;
        assert!(
            !results_contain_id(&results, &event_id_hex),
            "WS-published kind {kind} must not be stored"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_count_gating_for_execution_kinds() {
    let url = relay_url();
    let client = http_client();

    for kind in [KIND_EXECUTION_NODE_COMMAND, KIND_EXECUTION_NODE_RECEIPT] {
        let reader_keys = Keys::generate();
        let attacker_keys = Keys::generate();
        let reader_hex = reader_keys.public_key().to_hex();
        let attacker_hex = attacker_keys.public_key().to_hex();

        // WS COUNT with the kind but no #p → CLOSED restricted.
        let mut ws = BuzzTestClient::connect(&url, &attacker_keys)
            .await
            .expect("connect");
        let sid = sub_id("exec-count");
        let count_filter = Filter::new().kind(Kind::Custom(kind));
        let count_msg = serde_json::json!(["COUNT", sid, count_filter]);
        ws.send_raw(&count_msg).await.expect("send COUNT");
        expect_closed_restricted(&mut ws, &sid, &format!("COUNT kind {kind} without #p")).await;
        ws.disconnect().await.expect("disconnect");

        // HTTP COUNT with `#p = [self]` is authorized (these kinds are never
        // stored, so the count itself is 0 — the gate decision is the test).
        let inbox_filter = Filter::new()
            .kind(Kind::Custom(kind))
            .custom_tags(p_tag(), [reader_hex.as_str()]);
        let count = count_events_http(&client, &reader_hex, vec![inbox_filter])
            .await
            .unwrap_or_else(|(status, msg)| {
                panic!("addressee COUNT for kind {kind} failed: {status} {msg}")
            });
        assert_eq!(
            count, 0,
            "ephemeral kind {kind} must never count stored events"
        );

        // HTTP COUNT without #p from a non-addressee → 403.
        let result = count_events_http(
            &client,
            &attacker_hex,
            vec![Filter::new().kind(Kind::Custom(kind))],
        )
        .await;
        let (status, _) = result.expect_err("kind filter COUNT without #p must be rejected");
        assert_eq!(status, 403, "COUNT kind {kind} without #p must 403");
    }
}
