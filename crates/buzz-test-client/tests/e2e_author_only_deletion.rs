//! End-to-end tests: NIP-09 deletion requests (kind:5) for author-only kinds
//! are exactly as private as the events they delete.
//!
//! A deletion request names its target ids and reveals when they were
//! deleted. For a kind in `AUTHOR_ONLY_KINDS` (here the NIP-ER reminder,
//! kind:30300) the relay must hide that deletion from everyone but its author
//! on every read surface, and must reject one that omits the matching `k` tag.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p buzz-test-client --test e2e_author_only_deletion -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{EventBuilder, EventId, Filter, Keys, Kind, Tag};
use reqwest::Client;
use serde_json::Value;

const KIND_EVENT_REMINDER: u16 = 30300;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("e2e-aod-{name}-{}", uuid::Uuid::new_v4())
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

fn tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.to_vec()).expect("valid tag")
}

fn build_reminder(keys: &Keys) -> nostr::Event {
    EventBuilder::new(
        Kind::Custom(KIND_EVENT_REMINDER),
        "nip44-ciphertext-placeholder",
    )
    .tags(vec![
        tag(&["d", &uuid::Uuid::new_v4().to_string()]),
        tag(&["alt", "Encrypted reminder"]),
    ])
    .sign_with_keys(keys)
    .unwrap()
}

fn build_deletion(keys: &Keys, tags: Vec<Tag>) -> nostr::Event {
    build_deletion_with_reason(keys, tags, "")
}

fn build_deletion_with_reason(keys: &Keys, tags: Vec<Tag>, reason: &str) -> nostr::Event {
    EventBuilder::new(Kind::EventDeletion, reason)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

async fn submit(client: &Client, keys: &Keys, event: &nostr::Event) -> (bool, String) {
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(event)
        .send()
        .await
        .expect("submit event");
    let ok = resp.status().is_success();
    let body: Value = resp.json().await.expect("parse response");
    if ok {
        (
            body["accepted"].as_bool().unwrap_or(false),
            body["message"].as_str().unwrap_or("").to_string(),
        )
    } else {
        (false, body["error"].as_str().unwrap_or("").to_string())
    }
}

async fn submit_ok(client: &Client, keys: &Keys, event: &nostr::Event) {
    let (accepted, msg) = submit(client, keys, event).await;
    assert!(accepted, "setup event rejected: {msg}");
}

/// Store a reminder as `author`, then delete it with a `k`-tagged kind:5
/// whose reason is a unique search term. Returns the reminder and deletion.
async fn store_deleted_reminder(client: &Client, author: &Keys) -> (nostr::Event, nostr::Event) {
    let reminder = build_reminder(author);
    submit_ok(client, author, &reminder).await;
    let deletion = build_deletion_with_reason(
        author,
        vec![
            tag(&["e", &reminder.id.to_hex()]),
            tag(&["k", &KIND_EVENT_REMINDER.to_string()]),
        ],
        &search_term(author),
    );
    submit_ok(client, author, &deletion).await;
    let reminder_reads = http_query_ids(client, author, &Filter::new().id(reminder.id)).await;
    assert!(reminder_reads.is_empty(), "the reminder must be deleted");
    (reminder, deletion)
}

/// A search term unique to `author`'s deletion reason.
fn search_term(author: &Keys) -> String {
    format!("aodreason{}", &author.public_key().to_hex()[..16])
}

/// Filters that each match only the deletion `store_deleted_reminder` left:
/// by kind, mixed kinds, and id. The id filter is the kindless COUNT case;
/// broader kindless filters can match `#p`-gated kinds, so the relay closes
/// them before any read runs.
fn deletion_filters(author: &Keys, deletion: EventId) -> Vec<Filter> {
    vec![
        Filter::new()
            .kind(Kind::EventDeletion)
            .author(author.public_key()),
        Filter::new()
            .kinds([Kind::EventDeletion, Kind::TextNote])
            .author(author.public_key()),
        Filter::new().id(deletion),
    ]
}

async fn http_query_ids(client: &Client, reader: &Keys, filter: &Filter) -> Vec<String> {
    let resp = client
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", reader.public_key().to_hex())
        .json(&vec![filter])
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
        .iter()
        .filter_map(|e| e["id"].as_str().map(str::to_string))
        .collect()
}

async fn http_count(client: &Client, reader: &Keys, filter: &Filter) -> u64 {
    let resp = client
        .post(format!("{}/count", relay_http_url()))
        .header("X-Pubkey", reader.public_key().to_hex())
        .json(&vec![filter])
        .send()
        .await
        .expect("count events");
    assert!(
        resp.status().is_success(),
        "count failed: {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("parse count response");
    body["count"].as_u64().expect("count field")
}

async fn ws_query_ids(ws: &mut BuzzTestClient, filter: &Filter) -> Vec<EventId> {
    let sid = sub_id("req");
    ws.subscribe(&sid, vec![filter.clone()]).await.expect("REQ");
    let events = ws
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE");
    ws.close_subscription(&sid).await.expect("CLOSE");
    events.into_iter().map(|e| e.id).collect()
}

async fn ws_count(ws: &mut BuzzTestClient, filter: &Filter) -> u64 {
    let sid = sub_id("count");
    ws.send_raw(&serde_json::json!(["COUNT", sid, filter]))
        .await
        .expect("COUNT");
    loop {
        match ws
            .recv_event(Duration::from_secs(5))
            .await
            .expect("COUNT reply")
        {
            RelayMessage::Count {
                subscription_id,
                count,
            } if subscription_id == sid => return count,
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sid => panic!("COUNT closed: {message}"),
            _ => {}
        }
    }
}

/// Every historical read surface returns the deletion to `reader` iff
/// `visible`: WS REQ, WS COUNT, HTTP `/query` and HTTP `/count`, by author
/// and by id; search never returns it to anyone else.
async fn assert_deletion_visibility(
    reader: &Keys,
    author: &Keys,
    deletion: EventId,
    visible: bool,
) {
    let client = http_client();
    let mut ws = BuzzTestClient::connect(&relay_url(), reader)
        .await
        .expect("connect reader");
    let expected = u64::from(visible);
    let mut mismatches = Vec::new();
    for filter in deletion_filters(author, deletion) {
        let observed = [
            ("WS REQ", ws_query_ids(&mut ws, &filter).await.len() as u64),
            ("WS COUNT", ws_count(&mut ws, &filter).await),
            (
                "HTTP /query",
                http_query_ids(&client, reader, &filter).await.len() as u64,
            ),
            ("HTTP /count", http_count(&client, reader, &filter).await),
        ];
        for (surface, n) in observed {
            if n != expected {
                mismatches.push(format!("{surface} {filter:?}: {n}, expected {expected}"));
            }
        }
    }
    // Search, through reads: databases created before migration 0008 index
    // kind:5 content and fresh ones do not, so the author may or may not get
    // a hit. Nobody else ever does.
    let search = Filter::new()
        .kind(Kind::EventDeletion)
        .search(search_term(author));
    let searched = [
        (
            "WS REQ search",
            ws_query_ids(&mut ws, &search)
                .await
                .iter()
                .map(EventId::to_hex)
                .collect::<Vec<_>>(),
        ),
        (
            "HTTP /query search",
            http_query_ids(&client, reader, &search).await,
        ),
    ];
    for (surface, ids) in searched {
        if ids.iter().any(|id| *id != deletion.to_hex()) || (!visible && !ids.is_empty()) {
            mismatches.push(format!("{surface}: {ids:?}, visible={visible}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    ws.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn author_only_deletion_hidden_from_other_readers() {
    let client = http_client();
    let author = Keys::generate();
    let (_, deletion) = store_deleted_reminder(&client, &author).await;
    assert_deletion_visibility(&Keys::generate(), &author, deletion.id, false).await;
}

#[tokio::test]
#[ignore]
async fn author_only_deletion_visible_to_author() {
    let client = http_client();
    let author = Keys::generate();
    let (_, deletion) = store_deleted_reminder(&client, &author).await;
    assert_deletion_visibility(&author, &author, deletion.id, true).await;
}

#[tokio::test]
#[ignore]
async fn author_only_deletion_live_fanout_reaches_only_author() {
    let client = http_client();
    let author = Keys::generate();
    let reminder = build_reminder(&author);
    submit_ok(&client, &author, &reminder).await;

    let live = Filter::new()
        .kind(Kind::EventDeletion)
        .author(author.public_key());
    let mut subscribers = Vec::new();
    for keys in [author.clone(), Keys::generate()] {
        let mut ws = BuzzTestClient::connect(&relay_url(), &keys)
            .await
            .expect("connect");
        let sid = sub_id("live");
        ws.subscribe(&sid, vec![live.clone()]).await.expect("REQ");
        assert!(ws
            .collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("EOSE")
            .is_empty());
        subscribers.push((ws, sid));
    }

    let deletion = build_deletion(
        &author,
        vec![
            tag(&["e", &reminder.id.to_hex()]),
            tag(&["k", &KIND_EVENT_REMINDER.to_string()]),
        ],
    );
    submit_ok(&client, &author, &deletion).await;

    for (i, (mut ws, sid)) in subscribers.into_iter().enumerate() {
        let mut delivered = false;
        while let Ok(msg) = ws.recv_event(Duration::from_secs(2)).await {
            if let RelayMessage::Event {
                subscription_id,
                event,
            } = msg
            {
                delivered |= subscription_id == sid && event.id == deletion.id;
            }
        }
        let is_author = i == 0;
        assert_eq!(delivered, is_author, "live delivery to subscriber {i}");
        ws.disconnect().await.expect("disconnect");
    }
}

#[tokio::test]
#[ignore]
async fn deletion_of_author_only_event_requires_matching_k_tag() {
    let client = http_client();
    let author = Keys::generate();
    let reminder = build_reminder(&author);
    submit_ok(&client, &author, &reminder).await;
    let target = tag(&["e", &reminder.id.to_hex()]);
    let coordinate = format!(
        "{KIND_EVENT_REMINDER}:{}:{}",
        author.public_key().to_hex(),
        reminder.tags.identifier().expect("d tag")
    );
    let coordinate = tag(&["a", &coordinate]);

    for tags in [
        vec![target.clone()],
        vec![target.clone(), tag(&["k", "1"])],
        vec![coordinate.clone()],
    ] {
        let (accepted, msg) =
            submit(&client, &author, &build_deletion(&author, tags.clone())).await;
        assert!(
            !accepted,
            "deletion {tags:?} must be rejected without k=30300 (got: {msg})"
        );
    }
    submit_ok(
        &client,
        &author,
        &build_deletion(
            &author,
            vec![coordinate, tag(&["k", &KIND_EVENT_REMINDER.to_string()])],
        ),
    )
    .await;
}

#[tokio::test]
#[ignore]
async fn untagged_deletion_of_soft_deleted_author_only_event_rejected() {
    let client = http_client();
    let author = Keys::generate();
    let (reminder, _) = store_deleted_reminder(&client, &author).await;
    let untagged = build_deletion(&author, vec![tag(&["e", &reminder.id.to_hex()])]);
    let (accepted, msg) = submit(&client, &author, &untagged).await;
    assert!(
        !accepted,
        "an untagged deletion of a soft-deleted reminder must be rejected (got: {msg})"
    );
}

#[tokio::test]
#[ignore]
async fn non_canonical_k_tag_is_not_a_private_marker() {
    let client = http_client();
    let author = Keys::generate();
    let reminder = build_reminder(&author);
    submit_ok(&client, &author, &reminder).await;
    for k in ["030300", "+30300"] {
        let deletion = build_deletion(
            &author,
            vec![tag(&["e", &reminder.id.to_hex()]), tag(&["k", k])],
        );
        let (accepted, msg) = submit(&client, &author, &deletion).await;
        assert!(
            !accepted,
            "k={k} must not satisfy the k=30300 rule (got: {msg})"
        );
    }
}

#[tokio::test]
#[ignore]
async fn public_deletion_cannot_claim_author_only_kind() {
    let client = http_client();
    let author = Keys::generate();
    let note = EventBuilder::text_note("public note")
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &note).await;
    let deletion = build_deletion(
        &author,
        vec![
            tag(&["e", &note.id.to_hex()]),
            tag(&["k", &KIND_EVENT_REMINDER.to_string()]),
        ],
    );
    let (accepted, msg) = submit(&client, &author, &deletion).await;
    assert!(
        !accepted,
        "a public deletion must not hide behind k=30300 (got: {msg})"
    );
}
