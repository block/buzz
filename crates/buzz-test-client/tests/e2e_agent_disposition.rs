//! End-to-end integration tests for NIP-AD (Agent Disposition, kind:44300).
//!
//! These tests verify:
//! - Write-path validation: `e`/`p`/`disposition` tag shape, content/tag
//!   agreement, and the mandatory `h` tag (rejected at ingest when absent —
//!   see design.md Decision 9)
//! - World-readability: a disposition is readable by a third identity that is
//!   neither the publishing agent nor the request's author
//! - Append-only storage: multiple dispositions for the same agent (and for
//!   the same request, across its lifecycle) are never collapsed or replaced
//! - Server-side filtering: `#h` and `#e` narrow a query; `#disposition` does
//!   NOT (see design.md Decision 10 and NIP-AD.md "Not a query filter") — a
//!   consumer isolating one state filters client-side over the tag instead
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_agent_disposition -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable.

use std::time::Duration;

use nostr::{EventBuilder, Filter, Keys, Kind, Tag};
use reqwest::Client;
use serde_json::Value;

const KIND_AGENT_DISPOSITION: u16 = 44300;
const KIND_STREAM_MESSAGE: u16 = 9;
const KIND_CREATE_GROUP: u16 = 9007;

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

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

/// Submit a signed event via `POST /events`. Returns `(accepted, message)`,
/// matching the sibling e2e suites' convention: rejections may come back as
/// HTTP 200 with `accepted:false` or as a non-200 `{"error": ...}` body.
async fn submit_event_http(client: &Client, keys: &Keys, event: &nostr::Event) -> (bool, String) {
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    if status == 200 {
        let accepted = body["accepted"].as_bool().unwrap_or(false);
        let message = body["message"].as_str().unwrap_or("").to_string();
        (accepted, message)
    } else {
        let message = body["error"].as_str().unwrap_or("").to_string();
        (false, message)
    }
}

/// Query events via the HTTP bridge. Returns the JSON array of events.
async fn query_events_http(client: &Client, pubkey_hex: &str, filters: Vec<Filter>) -> Vec<Value> {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", pubkey_hex)
        .header("Content-Type", "application/json")
        .json(&filters)
        .send()
        .await
        .expect("query events");
    assert!(
        resp.status().is_success(),
        "query failed: {}",
        resp.status()
    );
    resp.json::<Vec<Value>>()
        .await
        .expect("parse query response")
}

/// Read the value of an event's first tag matching `key`.
fn tag_value<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event["tags"].as_array()?.iter().find_map(|t| {
        let t = t.as_array()?;
        if t.first()?.as_str()? == key {
            t.get(1)?.as_str()
        } else {
            None
        }
    })
}

/// Create an open-visibility channel so any generated identity (creator,
/// agent, or a wholly unrelated third-party auditor) can read and write
/// without a separate membership step. Returns the channel UUID string.
async fn create_test_channel(client: &Client, creator: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_GROUP), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("nip-ad-e2e-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(creator)
        .unwrap();
    let (accepted, message) = submit_event_http(client, creator, &event).await;
    assert!(accepted, "channel creation rejected: {message}");
    channel_uuid.to_string()
}

/// Publish a request message (an ordinary kind:9 channel message) and return
/// its event id hex — the id a disposition's `e` tag will reference.
async fn publish_request(
    client: &Client,
    requester: &Keys,
    channel_id: &str,
    text: &str,
) -> String {
    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), text)
        .tags(vec![Tag::parse(["h", channel_id]).unwrap()])
        .sign_with_keys(requester)
        .unwrap();
    let (accepted, message) = submit_event_http(client, requester, &event).await;
    assert!(accepted, "request message rejected: {message}");
    event.id.to_hex()
}

/// Build a well-formed kind:44300 disposition event.
fn build_disposition(
    agent: &Keys,
    channel_id: &str,
    request_id: &str,
    requester_hex: &str,
    disposition: &str,
    reason: &str,
) -> nostr::Event {
    build_disposition_at(
        agent,
        channel_id,
        request_id,
        requester_hex,
        disposition,
        reason,
        None,
    )
}

/// Same as [`build_disposition`], with an optional explicit `created_at` —
/// needed to deterministically order two dispositions for the same request.
/// Nostr timestamps are whole-second precision (see NIP-AD.md's "Same-second
/// ties"), so two events signed back-to-back in a test can otherwise land in
/// the same second and make "latest wins" assertions flaky by construction.
fn build_disposition_at(
    agent: &Keys,
    channel_id: &str,
    request_id: &str,
    requester_hex: &str,
    disposition: &str,
    reason: &str,
    created_at: Option<nostr::Timestamp>,
) -> nostr::Event {
    let content = serde_json::json!({ "disposition": disposition, "reason": reason }).to_string();
    let mut builder = EventBuilder::new(Kind::Custom(KIND_AGENT_DISPOSITION), content).tags(vec![
        Tag::parse(["e", request_id]).unwrap(),
        Tag::parse(["h", channel_id]).unwrap(),
        Tag::parse(["p", requester_hex]).unwrap(),
        Tag::parse(["disposition", disposition]).unwrap(),
    ]);
    if let Some(ts) = created_at {
        builder = builder.custom_created_at(ts);
    }
    builder.sign_with_keys(agent).unwrap()
}

#[tokio::test]
#[ignore]
async fn disposition_accepted_and_readable_by_unrelated_third_party() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let auditor = Keys::generate(); // neither requester nor agent — pure third party

    let channel_id = create_test_channel(&client, &requester).await;
    let request_id =
        publish_request(&client, &requester, &channel_id, "@agent please summarize").await;

    let disposition_event = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "refused",
        "outside my delegation",
    );
    let (accepted, message) = submit_event_http(&client, &agent, &disposition_event).await;
    assert!(accepted, "well-formed disposition rejected: {message}");

    // The auditor never touched requester's or agent's keys, and was never
    // added as a channel member — only the channel's open visibility and its
    // own signature authenticate this read. This is the "any third party can
    // verify" property NIP-AD exists to provide.
    let results = query_events_http(
        &client,
        &auditor.public_key().to_hex(),
        vec![Filter::new()
            .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
            .custom_tag(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                channel_id.clone(),
            )],
    )
    .await;

    let found = results
        .iter()
        .find(|e| e["id"].as_str() == Some(&disposition_event.id.to_hex()))
        .unwrap_or_else(|| panic!("disposition not found in third-party read: {results:?}"));
    assert_eq!(tag_value(found, "disposition"), Some("refused"));
    assert_eq!(tag_value(found, "e"), Some(request_id.as_str()));
    // Plaintext content readable with no decryption — the whole point.
    let content: Value = serde_json::from_str(found["content"].as_str().unwrap()).unwrap();
    assert_eq!(content["reason"], "outside my delegation");
}

#[tokio::test]
#[ignore]
async fn dispositions_are_append_only_across_lifecycle() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let request_id = publish_request(&client, &requester, &channel_id, "@agent do a thing").await;
    let requester_hex = requester.public_key().to_hex();

    // errored, then completed for the SAME request — both must persist;
    // neither is a NIP-33-style replacement of the other (kind 44300 is a
    // regular stored kind, never replaceable). Backdate `errored` by 2s so
    // the two events can't tie on created_at's whole-second precision —
    // this test asserts the ordinary "later created_at wins" path; the
    // same-second tiebreaker (event id, lexicographically greatest) is a
    // separate rule documented in NIP-AD.md, exercised where the actual
    // state-derivation helper is implemented (CLI `dispositions list` /
    // desktop accountability store), not here at the protocol layer.
    let backdated = nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 2);
    let errored = build_disposition_at(
        &agent,
        &channel_id,
        &request_id,
        &requester_hex,
        "errored",
        "tool call failed: connection reset",
        Some(backdated),
    );
    let (accepted, msg) = submit_event_http(&client, &agent, &errored).await;
    assert!(accepted, "errored disposition rejected: {msg}");

    let completed = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester_hex,
        "completed",
        "",
    );
    let (accepted, msg) = submit_event_http(&client, &agent, &completed).await;
    assert!(accepted, "completed disposition rejected: {msg}");

    let results = query_events_http(
        &client,
        &requester_hex,
        vec![Filter::new()
            .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
            .custom_tag(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
                request_id.clone(),
            )],
    )
    .await;

    let ids: Vec<&str> = results.iter().filter_map(|e| e["id"].as_str()).collect();
    assert!(
        ids.contains(&errored.id.to_hex().as_str())
            && ids.contains(&completed.id.to_hex().as_str()),
        "both errored and completed dispositions must survive as independent events, got: {ids:?}"
    );
    assert_eq!(
        results.len(),
        2,
        "no last-write-wins collapse expected for an append-only kind, got {} events",
        results.len()
    );

    // Lifecycle: latest by created_at is the current state. Both events were
    // signed in this order, so completed (signed second) is current.
    let latest = results
        .iter()
        .max_by_key(|e| e["created_at"].as_i64().unwrap_or(0))
        .unwrap();
    assert_eq!(tag_value(latest, "disposition"), Some("completed"));
}

#[tokio::test]
#[ignore]
async fn h_and_e_filter_server_side_disposition_does_not() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let requester_hex = requester.public_key().to_hex();

    let request_a = publish_request(&client, &requester, &channel_id, "@agent request A").await;
    let request_b = publish_request(&client, &requester, &channel_id, "@agent request B").await;

    let disp_a = build_disposition(
        &agent,
        &channel_id,
        &request_a,
        &requester_hex,
        "completed",
        "",
    );
    let disp_b = build_disposition(
        &agent,
        &channel_id,
        &request_b,
        &requester_hex,
        "refused",
        "not authorized for that action",
    );
    for (ev, label) in [(&disp_a, "A"), (&disp_b, "B")] {
        let (accepted, msg) = submit_event_http(&client, &agent, ev).await;
        assert!(accepted, "disposition {label} rejected: {msg}");
    }

    // #h scopes to the channel: both dispositions come back.
    let by_channel = query_events_http(
        &client,
        &requester_hex,
        vec![Filter::new()
            .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
            .custom_tag(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                channel_id.clone(),
            )],
    )
    .await;
    let by_channel_ids: Vec<&str> = by_channel.iter().filter_map(|e| e["id"].as_str()).collect();
    assert!(by_channel_ids.contains(&disp_a.id.to_hex().as_str()));
    assert!(by_channel_ids.contains(&disp_b.id.to_hex().as_str()));

    // #e scopes to exactly one request: only that request's disposition comes back.
    let by_request = query_events_http(
        &client,
        &requester_hex,
        vec![Filter::new()
            .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
            .custom_tag(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
                request_b.clone(),
            )],
    )
    .await;
    assert_eq!(
        by_request.len(),
        1,
        "an #e filter must isolate exactly one request's disposition(s)"
    );
    assert_eq!(tag_value(&by_request[0], "disposition"), Some("refused"));

    // Isolating "only refused" is a CLIENT-side filter over the #h-scoped
    // set — #disposition is deliberately never sent as a query key (see
    // NIP-AD.md "Not a query filter"; the underlying nostr crate's
    // Filter.generic_tags accepts only single-letter tag keys and would
    // silently ignore a multi-character "#disposition" filter rather than
    // erroring, which would silently over-return instead of narrowing).
    let refused_only: Vec<&Value> = by_channel
        .iter()
        .filter(|e| tag_value(e, "disposition") == Some("refused"))
        .collect();
    assert_eq!(refused_only.len(), 1);
    assert_eq!(
        refused_only[0]["id"].as_str(),
        Some(disp_b.id.to_hex().as_str())
    );
}

#[tokio::test]
#[ignore]
async fn disposition_rejected_without_h_tag() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    // Deliberately skip channel creation — this disposition must be
    // rejected for its missing `h` tag before channel access is even
    // considered, so no real channel is needed to prove the point.
    let request_id = "a".repeat(64);
    let content = serde_json::json!({"disposition": "completed", "reason": ""}).to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_DISPOSITION), content)
        .tags(vec![
            Tag::parse(["e", &request_id]).unwrap(),
            Tag::parse(["p", &requester.public_key().to_hex()]).unwrap(),
            Tag::parse(["disposition", "completed"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();

    let (accepted, message) = submit_event_http(&client, &agent, &event).await;
    assert!(!accepted, "disposition without an h tag must be rejected");
    assert!(
        message.contains("h tag") || message.contains("channel"),
        "expected a channel-scope rejection message, got: {message}"
    );
}

#[tokio::test]
#[ignore]
async fn disposition_rejected_with_invalid_state_value() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let request_id = publish_request(&client, &requester, &channel_id, "@agent do something").await;

    let event = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "maybe-later", // not one of completed|refused|errored
        "",
    );
    let (accepted, message) = submit_event_http(&client, &agent, &event).await;
    assert!(!accepted, "invalid disposition value must be rejected");
    assert!(
        message.contains("must be one of"),
        "expected a disposition-enum rejection message, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// Joined protocol path: signed request -> real relay ingest -> disposition
// publication -> read back -> shared-verifier binding and accounting.
//
// Everything above tests one layer. These tests exercise the whole contract
// against a live relay using the SAME verifier the CLI and desktop use
// (`buzz_core::disposition`), so a change that makes one layer disagree with
// another fails here rather than in review. Three review rounds found
// defects that each individual layer's tests passed straight through.
// ---------------------------------------------------------------------------

/// Publish a NIP-AD v1 request: marked, targeting exactly one agent, with
/// the matching `p` mention. Mirrors what the composer and
/// `buzz messages send --request-agent` both produce.
async fn publish_v1_request(
    client: &Client,
    requester: &Keys,
    channel_id: &str,
    agent_hex: &str,
    text: &str,
) -> String {
    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), text)
        .tags(vec![
            Tag::parse(["h", channel_id]).unwrap(),
            Tag::parse(["t", "request"]).unwrap(),
            Tag::parse(["agent", agent_hex]).unwrap(),
            Tag::parse(["p", agent_hex]).unwrap(),
        ])
        .sign_with_keys(requester)
        .unwrap();
    let (accepted, message) = submit_event_http(client, requester, &event).await;
    assert!(accepted, "v1 request rejected: {message}");
    event.id.to_hex()
}

/// Owned storage so `EventView`s can borrow from relay JSON.
struct FetchedEvent {
    id: String,
    pubkey: String,
    kind: u16,
    created_at: i64,
    content: String,
    tags: Vec<Vec<String>>,
}

impl FetchedEvent {
    fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            pubkey: value.get("pubkey")?.as_str()?.to_string(),
            kind: value.get("kind").and_then(Value::as_u64).unwrap_or(0) as u16,
            created_at: value.get("created_at").and_then(Value::as_i64).unwrap_or(0),
            content: value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tags: value
                .get("tags")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            Some(
                                row.as_array()?
                                    .iter()
                                    .filter_map(|c| c.as_str().map(String::from))
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn view(&self) -> buzz_core::disposition::EventView<'_> {
        buzz_core::disposition::EventView {
            id: &self.id,
            pubkey: &self.pubkey,
            kind: self.kind,
            created_at: self.created_at,
            content: &self.content,
            tags: &self.tags,
        }
    }
}

/// Read a channel's marked requests and dispositions back from the relay,
/// exactly as a third-party auditor would.
async fn fetch_channel_state(
    client: &Client,
    reader_hex: &str,
    channel_id: &str,
) -> (Vec<FetchedEvent>, Vec<FetchedEvent>) {
    let requests = query_events_http(
        client,
        reader_hex,
        vec![Filter::new()
            .kind(Kind::Custom(KIND_STREAM_MESSAGE))
            .custom_tags(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                [channel_id.to_string()],
            )
            .custom_tags(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::T),
                ["request".to_string()],
            )],
    )
    .await;
    let dispositions = query_events_http(
        client,
        reader_hex,
        vec![Filter::new()
            .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
            .custom_tags(
                nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                [channel_id.to_string()],
            )],
    )
    .await;
    (
        requests
            .iter()
            .filter_map(FetchedEvent::from_json)
            .collect(),
        dispositions
            .iter()
            .filter_map(FetchedEvent::from_json)
            .collect(),
    )
}

/// The whole contract, end to end: a request the agent answers is reported
/// resolved by an unrelated third party reading only public relay state.
#[tokio::test]
#[ignore]
async fn live_path_target_agent_resolves_its_own_request() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let auditor = Keys::generate(); // neither the agent nor the requester
    let agent_hex = agent.public_key().to_hex();
    let channel_id = create_test_channel(&client, &requester).await;

    let request_id = publish_v1_request(
        &client,
        &requester,
        &channel_id,
        &agent_hex,
        "@agent summarize yesterday",
    )
    .await;

    let event = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "completed",
        "done",
    );
    let (accepted, message) = submit_event_http(&client, &agent, &event).await;
    assert!(accepted, "disposition rejected: {message}");

    let (requests, dispositions) =
        fetch_channel_state(&client, &auditor.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert_eq!(
        acc.settled,
        vec![request_id],
        "the target agent's completion must resolve its own request; got {acc:?}"
    );
    assert!(acc.unanswered.is_empty());
    assert!(acc.invalid_requests.is_empty());
    assert!(acc.all_resolved());
}

/// The cross-principal spoof, live: a human `p`-mentioned on the request
/// publishes a `completed`. The relay stores it (structural validity is all
/// it checks), but the request stays unanswered for every consumer.
#[tokio::test]
#[ignore]
async fn live_path_mentioned_human_cannot_resolve_the_request() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let human = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;

    // The request mentions the human as well as targeting the agent.
    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), "@agent and @human, fyi")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["t", "request"]).unwrap(),
            Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["p", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["p", &human.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&requester)
        .unwrap();
    let (accepted, message) = submit_event_http(&client, &requester, &event).await;
    assert!(accepted, "request rejected: {message}");
    let request_id = event.id.to_hex();

    let spoof = build_disposition(
        &human,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "completed",
        "I'll say it's done",
    );
    let (stored, _) = submit_event_http(&client, &human, &spoof).await;
    assert!(
        stored,
        "the relay checks structural validity only — this event is well-formed \
         and IS stored. The protection is consumer-side binding, which is what \
         the assertions below verify."
    );

    let (requests, dispositions) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert_eq!(
        acc.unanswered,
        vec![request_id],
        "a merely p-mentioned human must not close the agent's obligation; got {acc:?}"
    );
    assert!(acc.settled.is_empty());
    assert!(!acc.all_resolved());
}

/// Another agent, live: signs a `completed` for a request addressed to a
/// different agent. Stored, never bound.
#[tokio::test]
#[ignore]
async fn live_path_another_agent_cannot_resolve_the_request() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let other_agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let request_id = publish_v1_request(
        &client,
        &requester,
        &channel_id,
        &agent.public_key().to_hex(),
        "@agent handle this",
    )
    .await;

    let spoof = build_disposition(
        &other_agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "completed",
        "",
    );
    submit_event_http(&client, &other_agent, &spoof).await;

    let (requests, dispositions) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert_eq!(acc.unanswered, vec![request_id]);
}

/// A marked request naming no agent is classified invalid, NOT counted as an
/// unanswered agent failure. Blaming an agent for a composer fault produces a
/// gap nobody can ever clear.
#[tokio::test]
#[ignore]
async fn live_path_targetless_marker_is_invalid_not_unanswered() {
    let client = http_client();
    let requester = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;

    // `publish_request` deliberately emits the pre-v1 shape: marker-less.
    // Here we add the marker but no `agent` target — the exact event an old
    // client or a buggy composer would produce.
    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), "someone look at this")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["t", "request"]).unwrap(),
        ])
        .sign_with_keys(&requester)
        .unwrap();
    let (accepted, message) = submit_event_http(&client, &requester, &event).await;
    assert!(accepted, "request rejected: {message}");

    let (requests, dispositions) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert!(
        acc.unanswered.is_empty(),
        "a targetless marker is a protocol fault, not an agent gap; got {acc:?}"
    );
    assert_eq!(acc.invalid_requests.len(), 1);
    assert_eq!(
        acc.invalid_requests[0].1,
        buzz_core::disposition::InvalidRequest::MissingAgentTarget
    );
    assert!(
        !acc.all_resolved(),
        "an invalid request blocks a clean claim"
    );
}

/// A multi-target request is unsupported in v1 — classified invalid rather
/// than given a request-wide state that either agent could discharge.
#[tokio::test]
#[ignore]
async fn live_path_multi_target_request_is_unsupported() {
    let client = http_client();
    let requester = Keys::generate();
    let agent_a = Keys::generate();
    let agent_b = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;

    let event = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), "@a @b both look")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["t", "request"]).unwrap(),
            Tag::parse(["agent", &agent_a.public_key().to_hex()]).unwrap(),
            Tag::parse(["agent", &agent_b.public_key().to_hex()]).unwrap(),
            Tag::parse(["p", &agent_a.public_key().to_hex()]).unwrap(),
            Tag::parse(["p", &agent_b.public_key().to_hex()]).unwrap(),
        ])
        .sign_with_keys(&requester)
        .unwrap();
    submit_event_http(&client, &requester, &event).await;

    // Even with BOTH agents answering — the case that used to manufacture a
    // conflict out of two correct responses — the request is simply
    // unsupported, not conflicted and not resolved.
    for agent in [&agent_a, &agent_b] {
        let d = build_disposition(
            agent,
            &channel_id,
            &event.id.to_hex(),
            &requester.public_key().to_hex(),
            "completed",
            "",
        );
        submit_event_http(&client, agent, &d).await;
    }

    let (requests, dispositions) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert_eq!(
        acc.unsupported_requests[0].1,
        buzz_core::disposition::UnsupportedRequest::MultipleAgentTargets
    );
    assert!(
        acc.disputed.is_empty(),
        "two correct answers are not a dispute"
    );
    assert!(acc.settled.is_empty());
    assert!(
        acc.unanswered.is_empty(),
        "an unsupported request is never an agent's gap"
    );
}

/// `responded` is stored, read back, and reported open — never resolved.
/// This is the state that keeps a harness from claiming completion.
#[tokio::test]
#[ignore]
async fn live_path_responded_is_open_not_resolved() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let request_id = publish_v1_request(
        &client,
        &requester,
        &channel_id,
        &agent.public_key().to_hex(),
        "@agent look into this",
    )
    .await;

    let event = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "responded",
        "",
    );
    let (accepted, message) = submit_event_http(&client, &agent, &event).await;
    assert!(
        accepted,
        "`responded` must be a valid wire state: {message}"
    );

    let (requests, dispositions) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let request_views: Vec<_> = requests.iter().map(FetchedEvent::view).collect();
    let disposition_views: Vec<_> = dispositions.iter().map(FetchedEvent::view).collect();

    let acc = buzz_core::disposition::account(
        &request_views,
        &disposition_views,
        buzz_core::disposition::Coverage::complete(),
    );
    assert_eq!(
        acc.open,
        vec![request_id],
        "responded is answered but not settled"
    );
    assert!(acc.settled.is_empty());
    assert!(!acc.all_resolved());
}

/// HTTP and WebSocket ingest are separate code paths in the relay. A request
/// published over one must be indistinguishable from the same request
/// published over the other — otherwise the desktop app (WS) and the CLI
/// (HTTP) would hand consumers differently-shaped obligations, and the
/// verifier's answer would depend on which client happened to send it.
///
/// This publishes two independently signed but structurally identical
/// requests, one per transport, and asserts the relay returns tag sets that
/// classify to the same obligation shape.
#[tokio::test]
#[ignore]
async fn live_path_http_and_ws_requests_are_indistinguishable() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let agent_hex = agent.public_key().to_hex();
    let channel_id = create_test_channel(&client, &requester).await;

    let tags = vec![
        Tag::parse(["h", &channel_id]).unwrap(),
        Tag::parse(["t", "request"]).unwrap(),
        Tag::parse(["agent", &agent_hex]).unwrap(),
        Tag::parse(["p", &agent_hex]).unwrap(),
    ];

    let via_http = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), "over http")
        .tags(tags.clone())
        .sign_with_keys(&requester)
        .unwrap();
    let (accepted, message) = submit_event_http(&client, &requester, &via_http).await;
    assert!(accepted, "HTTP request rejected: {message}");

    let via_ws = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE), "over ws")
        .tags(tags)
        .sign_with_keys(&requester)
        .unwrap();
    let mut conn =
        buzz_ws_client::NostrWsConnection::connect_authenticated(&relay_url(), &requester, None)
            .await
            .expect("ws connect + NIP-42 auth failed");
    let ok = conn
        .send_event(via_ws.clone())
        .await
        .expect("ws EVENT failed");
    assert!(ok.accepted, "WS request rejected: {}", ok.message);
    conn.disconnect().await.ok();

    let (requests, _) =
        fetch_channel_state(&client, &requester.public_key().to_hex(), &channel_id).await;
    let http_stored = requests
        .iter()
        .find(|e| e.id == via_http.id.to_hex())
        .expect("HTTP-published request not readable");
    let ws_stored = requests
        .iter()
        .find(|e| e.id == via_ws.id.to_hex())
        .expect("WS-published request not readable");

    // Tags survive both ingest paths byte-for-byte. If either transport
    // normalized, reordered, or dropped a tag, this is where it shows.
    assert_eq!(
        http_stored.tags, ws_stored.tags,
        "the two transports stored different tag sets"
    );

    // And the verifier derives the same obligation from each — the property
    // that actually matters to consumers.
    let http_class = buzz_core::disposition::classify_request(&http_stored.view());
    let ws_class = buzz_core::disposition::classify_request(&ws_stored.view());
    match (&http_class, &ws_class) {
        (
            buzz_core::disposition::RequestClass::Valid(a),
            buzz_core::disposition::RequestClass::Valid(b),
        ) => {
            assert_eq!(a.channel_id, b.channel_id);
            assert_eq!(a.requester_pubkey, b.requester_pubkey);
            assert_eq!(a.target_agent_pubkey, b.target_agent_pubkey);
            assert_eq!(a.target_agent_pubkey, agent_hex);
        }
        other => panic!("both transports must yield a valid obligation, got {other:?}"),
    }
}

/// The CLI auditor verifies every event's id and signature before applying a
/// single NIP-AD rule, which is what makes it an independent auditor rather
/// than a semantic verifier trusting the relay. That guarantee has a
/// precondition nothing else states: **the relay must return signatures on
/// read.** If it ever stripped them, verification would reject everything and
/// `buzz dispositions list` would report an empty channel — a silent, total
/// failure that every unit test would sail past, because unit fixtures never
/// round-trip through the relay.
#[tokio::test]
#[ignore]
async fn live_path_reads_return_verifiable_signatures() {
    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let channel_id = create_test_channel(&client, &requester).await;
    let request_id = publish_v1_request(
        &client,
        &requester,
        &channel_id,
        &agent.public_key().to_hex(),
        "@agent check the signature path",
    )
    .await;
    let disposition = build_disposition(
        &agent,
        &channel_id,
        &request_id,
        &requester.public_key().to_hex(),
        "responded",
        "",
    );
    let (accepted, message) = submit_event_http(&client, &agent, &disposition).await;
    assert!(accepted, "disposition rejected: {message}");

    for (label, filter) in [
        (
            "requests",
            Filter::new()
                .kind(Kind::Custom(KIND_STREAM_MESSAGE))
                .custom_tags(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                    [channel_id.clone()],
                ),
        ),
        (
            "dispositions",
            Filter::new()
                .kind(Kind::Custom(KIND_AGENT_DISPOSITION))
                .custom_tags(
                    nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                    [channel_id.clone()],
                ),
        ),
    ] {
        let events =
            query_events_http(&client, &requester.public_key().to_hex(), vec![filter]).await;
        assert!(!events.is_empty(), "{label}: nothing returned to verify");
        for value in &events {
            let event: nostr::Event = serde_json::from_value(value.clone())
                .unwrap_or_else(|e| panic!("{label}: read event is not a parseable Nostr event ({e}) — the auditor cannot verify what it cannot parse: {value}"));
            event.verify().unwrap_or_else(|e| {
                panic!("{label}: read event failed signature verification ({e}) — every consumer that verifies before auditing would silently see an empty channel")
            });
        }
    }
}

// ---------------------------------------------------------------------------
// The joined completion path: real CLI binary, real parser, real signing.
//
// Every other test here builds and submits events directly. That proves the
// protocol and the accounting, and proves nothing about whether an agent can
// actually reach a settled state in normal operation — which matters because
// the harness deliberately cannot emit `completed`, making the CLI the only
// path to it. A hand-built event would not have caught the NIP documenting
// `--state` when the flag is `--disposition`.
// ---------------------------------------------------------------------------

/// Path to the built CLI. Override with `BUZZ_CLI_BIN`.
fn cli_binary() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("BUZZ_CLI_BIN") {
        return std::path::PathBuf::from(p);
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .unwrap_or_else(|_| format!("{}/../../target", env!("CARGO_MANIFEST_DIR")));
    std::path::PathBuf::from(target).join("release/buzz")
}

fn run_cli(keys: &Keys, args: &[&str]) -> (bool, String) {
    let out = std::process::Command::new(cli_binary())
        .args(args)
        .env("BUZZ_RELAY_URL", relay_url())
        .env("BUZZ_PRIVATE_KEY", keys.secret_key().to_secret_hex())
        .output()
        .expect("run buzz CLI");
    let merged = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), merged)
}

/// A managed agent settles its own request through the real CLI, and an
/// unrelated authorized reader sees a settled obligation.
///
/// Also pins the two authorization facts that make `completed` meaningful:
/// the CLI refuses a signer that is not the obligation's target, and the
/// flag the NIP tells agents to use is the flag the parser accepts.
#[tokio::test]
#[ignore]
async fn live_path_agent_settles_its_own_request_through_the_cli() {
    let cli = cli_binary();
    if !cli.exists() {
        panic!(
            "CLI binary not found at {}. Build it first: \
             cargo build --release -p buzz-cli (or set BUZZ_CLI_BIN).",
            cli.display()
        );
    }

    let client = http_client();
    let requester = Keys::generate();
    let agent = Keys::generate();
    let auditor = Keys::generate();
    let agent_hex = agent.public_key().to_hex();
    let channel_id = create_test_channel(&client, &requester).await;

    // The CLI refuses to send a message mentioning a non-member, so the agent
    // joins first — the same step a real deployment performs when attaching a
    // managed agent to a channel.
    let (ok, out) = run_cli(
        &requester,
        &[
            "channels",
            "add-member",
            "--channel",
            &channel_id,
            "--pubkey",
            &agent_hex,
            "--role",
            "bot",
        ],
    );
    assert!(ok, "could not add the agent to the channel: {out}");

    // The request goes out through the CLI's own marking path, not a
    // hand-assembled event.
    let (ok, out) = run_cli(
        &requester,
        &[
            "--format",
            "compact",
            "messages",
            "send",
            "--channel",
            &channel_id,
            "--content",
            "@agent please do the thing",
            "--request-agent",
            &agent_hex,
        ],
    );
    assert!(ok, "CLI request send failed: {out}");
    let request_id = serde_json::from_str::<Value>(out.trim())
        .ok()
        .and_then(|v| v["event_id"].as_str().map(String::from))
        .unwrap_or_else(|| panic!("no event_id in CLI output: {out}"));

    // The agent settles it. `--disposition` is the flag the NIP instructs
    // agents to use; if they ever diverge this fails here rather than in a
    // channel nobody can settle.
    let (ok, out) = run_cli(
        &agent,
        &[
            "dispositions",
            "emit",
            "--request",
            &request_id,
            "--disposition",
            "completed",
            "--reason",
            "did the thing",
        ],
    );
    assert!(ok, "agent could not settle its own request: {out}");

    // A non-target must be refused by the CLI, before anything is published.
    let (ok, out) = run_cli(
        &requester,
        &[
            "dispositions",
            "emit",
            "--request",
            &request_id,
            "--disposition",
            "completed",
        ],
    );
    assert!(!ok, "a non-target was allowed to settle: {out}");
    assert!(
        out.contains("is not the agent"),
        "expected a target-agent refusal, got: {out}"
    );

    // An unrelated authorized reader audits the channel.
    let (ok, out) = run_cli(
        &auditor,
        &[
            "--format",
            "compact",
            "dispositions",
            "list",
            "--channel",
            &channel_id,
        ],
    );
    assert!(ok, "auditor read failed: {out}");
    let report: Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("bad CLI JSON ({e}): {out}"));

    assert_eq!(
        report["settled"].as_array().map(Vec::len),
        Some(1),
        "the obligation must read as settled: {report}"
    );
    assert_eq!(report["settled"][0].as_str(), Some(request_id.as_str()));
    assert!(report["unanswered"].as_array().unwrap().is_empty());
    assert_eq!(
        report["unverifiable_events"].as_u64(),
        Some(0),
        "every audited event must have verified"
    );

    let row = report["dispositions"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["request_id"].as_str() == Some(request_id.as_str()))
        .expect("a row for the settled request");
    assert_eq!(row["outcome"].as_str(), Some("settled"));
    assert_eq!(row["disposition"].as_str(), Some("completed"));
    assert_eq!(
        row["reason"].as_str(),
        Some("did the thing"),
        "the agent's stated reason must survive to the auditor"
    );
}
