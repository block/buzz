//! End-to-end tests: NIP-09 deletion requests (kind:5) for author-only kinds
//! are exactly as private as the events they delete.
//!
//! A deletion request names its target ids and reveals when they were
//! deleted. For a kind in `AUTHOR_ONLY_KINDS` (here the NIP-ER reminder,
//! kind:30300) the relay must hide that deletion from everyone but its author
//! on every read surface, and must reject one that omits the matching `k` tag.
//! A deletion from anyone without authority over its target is rejected
//! before any of that, with one message whether the target is missing, live,
//! soft-deleted, or in any channel, so the rejection reveals nothing about it.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p buzz-test-client --test e2e_author_only_deletion -- --ignored
//! ```
//!
//! On a database built from the desired-state schema (`schema/schema.sql`),
//! which indexes kind:5 content, also set `BUZZ_TEST_SEARCH_INDEXES_DELETIONS=1`.
//! Without it, the search test only checks that results stay within the
//! permitted sets, not that search actually returns them.

use std::collections::BTreeSet;
use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
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
/// and by id.
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
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    ws.disconnect().await.expect("disconnect");
}

/// Whether the relay's `search_tsv` indexes kind:5 content. The desired
/// schema (`schema/schema.sql`, which CI applies) does; an empty database
/// initialized through migration 0008 gets a positive allowlist that doesn't.
fn search_indexes_deletions() -> bool {
    std::env::var("BUZZ_TEST_SEARCH_INDEXES_DELETIONS").is_ok_and(|v| v == "1")
}

/// Search results for `filter` over WS REQ and HTTP `/query`, as hex ids.
async fn search_ids(reader: &Keys, filter: &Filter) -> [(&'static str, BTreeSet<String>); 2] {
    let mut ws = BuzzTestClient::connect(&relay_url(), reader)
        .await
        .expect("connect reader");
    let ws_ids = ws_query_ids(&mut ws, filter)
        .await
        .iter()
        .map(EventId::to_hex)
        .collect();
    ws.disconnect().await.expect("disconnect");
    let http_ids = http_query_ids(&http_client(), reader, filter)
        .await
        .into_iter()
        .collect();
    [("WS REQ search", ws_ids), ("HTTP /query search", http_ids)]
}

#[tokio::test]
#[ignore]
async fn author_only_deletion_search_reaches_only_author() {
    let client = http_client();
    let author = Keys::generate();
    let (_, private) = store_deleted_reminder(&client, &author).await;
    // Public control: an ordinary deletion carrying the same reason word.
    let note = EventBuilder::text_note("public note")
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &note).await;
    let public = build_deletion_with_reason(
        &author,
        vec![tag(&["e", &note.id.to_hex()]), tag(&["k", "1"])],
        &search_term(&author),
    );
    submit_ok(&client, &author, &public).await;

    let search = Filter::new()
        .kind(Kind::EventDeletion)
        .search(search_term(&author));
    let (private, public) = (private.id.to_hex(), public.id.to_hex());
    let strict = search_indexes_deletions();
    let mut mismatches = Vec::new();
    for (reader, name, allowed) in [
        (
            &author,
            "author",
            BTreeSet::from([private.clone(), public.clone()]),
        ),
        (&Keys::generate(), "other", BTreeSet::from([public.clone()])),
    ] {
        for (surface, ids) in search_ids(reader, &search).await {
            let ok = if strict {
                ids == allowed
            } else {
                ids.is_subset(&allowed)
            };
            if !ok {
                mismatches.push(format!(
                    "{surface} as {name}: {ids:?}, expected {allowed:?} (strict={strict})"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
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

/// The one rejection a sender without authority over a deletion target gets,
/// whether the target exists, is soft-deleted, or lives in any channel.
const DENIED: &str = "invalid: deletion target not found or not deletable by you";

/// Submit `deletion` as `sender` over HTTP and WS and return how each
/// rejected it: the HTTP status and error, and the WS `OK` message.
async fn rejection(
    client: &Client,
    sender: &Keys,
    deletion: nostr::Event,
) -> (u16, String, String) {
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", sender.public_key().to_hex())
        .json(&deletion)
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    let http_msg = body["error"].as_str().unwrap_or("").to_string();
    let mut ws = BuzzTestClient::connect(&relay_url(), sender)
        .await
        .expect("connect");
    let ok = ws.send_event(deletion).await.expect("OK");
    ws.disconnect().await.expect("disconnect");
    assert!(!ok.accepted, "WS must reject the deletion");
    (status, http_msg, ok.message)
}

/// Submit `deletion` as `sender` over HTTP and WS; both must give the
/// generic denial, which cannot reveal the target's kind.
async fn assert_rejected_as_non_author(client: &Client, sender: &Keys, deletion: nostr::Event) {
    let (status, http_msg, ws_msg) = rejection(client, sender, deletion).await;
    assert!(status >= 400, "HTTP must reject the deletion");
    for msg in [http_msg, ws_msg] {
        assert_eq!(msg, DENIED);
    }
}

async fn create_channel(client: &Client, owner: &Keys, visibility: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            tag(&["h", &id]),
            tag(&["name", &format!("aod-{id}")]),
            tag(&["channel_type", "stream"]),
            tag(&["visibility", visibility]),
        ])
        .sign_with_keys(owner)
        .unwrap();
    submit_ok(client, owner, &event).await;
    id
}

async fn post_message(client: &Client, author: &Keys, channel: &str) -> nostr::Event {
    let message = EventBuilder::new(Kind::Custom(9), uuid::Uuid::new_v4().to_string())
        .tags(vec![tag(&["h", channel])])
        .sign_with_keys(author)
        .unwrap();
    submit_ok(client, author, &message).await;
    message
}

/// A sender without authority over a deletion target gets one rejection,
/// identical in HTTP status, HTTP error and WS `OK` message, for kind 5 and
/// kind 9005 alike, whether the target is missing, live, soft-deleted, in a
/// private channel the sender has not joined, or in another channel.
#[tokio::test]
#[ignore]
async fn unauthorized_deletion_does_not_reveal_target_existence() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let open = create_channel(&client, &author, "open").await;
    let private = create_channel(&client, &author, "private").await;
    // A channel `other` owns, so 9005 there passes as owner/admin authority.
    let others = create_channel(&client, &other, "open").await;
    let live = post_message(&client, &author, &open).await;
    let deleted = post_message(&client, &author, &open).await;
    submit_ok(
        &client,
        &author,
        &build_deletion(&author, vec![tag(&["e", &deleted.id.to_hex()])]),
    )
    .await;
    let hidden = post_message(&client, &author, &private).await;
    let reminder = build_reminder(&author);
    submit_ok(&client, &author, &reminder).await;
    let missing = "a".repeat(64);

    assert_ne!(live.id, deleted.id);
    let delete = |target: &str| build_deletion(&other, vec![tag(&["e", target])]);
    let redact = |target: &str, channel: &str| {
        EventBuilder::new(Kind::Custom(9005), "")
            .tags(vec![tag(&["e", target]), tag(&["h", channel])])
            .sign_with_keys(&other)
            .unwrap()
    };
    let cases = [
        ("kind 5, missing", delete(&missing)),
        ("kind 5, live", delete(&live.id.to_hex())),
        ("kind 5, soft-deleted", delete(&deleted.id.to_hex())),
        ("kind 5, private channel", delete(&hidden.id.to_hex())),
        ("kind 5, global reminder", delete(&reminder.id.to_hex())),
        ("9005, missing", redact(&missing, &others)),
        ("9005, live", redact(&live.id.to_hex(), &open)),
        ("9005, soft-deleted", redact(&deleted.id.to_hex(), &open)),
        (
            "9005, private channel",
            redact(&hidden.id.to_hex(), &private),
        ),
        ("9005, other channel", redact(&live.id.to_hex(), &others)),
        ("9005, no channel", redact(&reminder.id.to_hex(), &others)),
    ];
    let mut expected = None;
    for (case, deletion) in cases {
        let (status, http_msg, ws_msg) = rejection(&client, &other, deletion).await;
        assert_eq!(http_msg, DENIED, "{case}: HTTP");
        assert_eq!(ws_msg, DENIED, "{case}: WS");
        assert_eq!(
            *expected.get_or_insert(status),
            status,
            "{case}: HTTP status"
        );
    }
}

#[tokio::test]
#[ignore]
async fn non_author_deletion_does_not_reveal_target_kind() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let live = build_reminder(&author);
    submit_ok(&client, &author, &live).await;
    let (deleted, _) = store_deleted_reminder(&client, &author).await;

    for target in [&live, &deleted] {
        for k in [None, Some("30350"), Some("30300")] {
            let mut tags = vec![tag(&["e", &target.id.to_hex()])];
            tags.extend(k.map(|k| tag(&["k", k])));
            assert_rejected_as_non_author(&client, &other, build_deletion(&other, tags)).await;
        }
    }
}

/// An author removed from a private channel can no longer delete their
/// messages there, so kind 5 and kind 9005 (sent from a channel they still
/// write to) must reject exactly as for a missing target, while the channel
/// owner and the author's own still-writable messages keep working.
#[tokio::test]
#[ignore]
async fn removed_author_deletion_does_not_reveal_target() {
    let client = http_client();
    let owner = Keys::generate();
    let author = Keys::generate();
    let private = create_channel(&client, &owner, "private").await;
    let own = create_channel(&client, &author, "open").await;
    let membership = |kind: u16| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(vec![
                tag(&["h", &private]),
                tag(&["p", &author.public_key().to_hex()]),
            ])
            .sign_with_keys(&owner)
            .unwrap()
    };
    submit_ok(&client, &owner, &membership(9000)).await;
    let retained = post_message(&client, &author, &private).await;
    let moderated = post_message(&client, &author, &private).await;
    submit_ok(&client, &owner, &membership(9001)).await;
    let missing = "a".repeat(64);

    let delete = |target: &str| build_deletion(&author, vec![tag(&["e", target])]);
    let redact = |keys: &Keys, target: &str, channel: &str| {
        EventBuilder::new(Kind::Custom(9005), "")
            .tags(vec![tag(&["e", target]), tag(&["h", channel])])
            .sign_with_keys(keys)
            .unwrap()
    };
    let cases = [
        ("kind 5, missing", delete(&missing)),
        ("kind 5, retained", delete(&retained.id.to_hex())),
        ("9005, missing", redact(&author, &missing, &own)),
        (
            "9005, retained",
            redact(&author, &retained.id.to_hex(), &own),
        ),
    ];
    let mut expected = None;
    for (case, deletion) in cases {
        let (status, http_msg, ws_msg) = rejection(&client, &author, deletion).await;
        assert_eq!(http_msg, DENIED, "{case}: HTTP");
        assert_eq!(ws_msg, DENIED, "{case}: WS");
        assert_eq!(
            *expected.get_or_insert(status),
            status,
            "{case}: HTTP status"
        );
    }

    submit_ok(
        &client,
        &owner,
        &redact(&owner, &moderated.id.to_hex(), &private),
    )
    .await;
    let still_writable = post_message(&client, &author, &own).await;
    submit_ok(&client, &author, &delete(&still_writable.id.to_hex())).await;
}

/// An NIP-AR artifact revision by `author` homed in `channel`.
fn artifact_revision(
    author: &Keys,
    artifact: &str,
    channel: &str,
    op: &str,
    prev: Option<&EventId>,
) -> nostr::Event {
    let mut tags = vec![
        tag(&["ar", "1"]),
        tag(&["d", artifact]),
        tag(&["h", channel]),
        tag(&["type", "buzz.task"]),
        tag(&["op", op]),
        tag(&["title", "Task"]),
    ];
    tags.extend(prev.map(|p| tag(&["prev", &p.to_hex()])));
    EventBuilder::new(Kind::Custom(45010), "")
        .tags(tags)
        .sign_with_keys(author)
        .unwrap()
}

/// A live artifact by `author` homed in `channel`.
async fn post_artifact(client: &Client, author: &Keys, channel: &str) -> nostr::Event {
    let create = artifact_revision(
        author,
        &uuid::Uuid::new_v4().to_string(),
        channel,
        "create",
        None,
    );
    submit_ok(client, author, &create).await;
    create
}

/// An archived channel blocks every deletion of its events, so kind 5 and
/// kind 9005 must reject as for a missing target before any error about the
/// target's kind or location, even for the author who owns the channel.
#[tokio::test]
#[ignore]
async fn archived_target_deletion_does_not_reveal_target() {
    let client = http_client();
    let owner = Keys::generate();
    let archived = create_channel(&client, &owner, "open").await;
    let active = create_channel(&client, &owner, "open").await;
    let message = post_message(&client, &owner, &archived).await;
    let artifact = post_artifact(&client, &owner, &archived).await;
    // Moving an artifact out of a channel leaves a removal marker there.
    let moved = post_artifact(&client, &owner, &archived).await;
    let d = moved.tags.identifier().unwrap().to_string();
    submit_ok(
        &client,
        &owner,
        &artifact_revision(&owner, &d, &active, "move", Some(&moved.id)),
    )
    .await;
    let marker = http_query_ids(
        &client,
        &owner,
        &Filter::new().kind(Kind::Custom(45011)).custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
            archived.clone(),
        ),
    )
    .await
    .pop()
    .expect("artifact removal marker");
    let archive = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![tag(&["h", &archived]), tag(&["archived", "true"])])
        .sign_with_keys(&owner)
        .unwrap();
    submit_ok(&client, &owner, &archive).await;
    let missing = "a".repeat(64);

    let delete = |target: &str, k: Option<&str>| {
        let mut tags = vec![tag(&["e", target])];
        tags.extend(k.map(|k| tag(&["k", k])));
        build_deletion(&owner, tags)
    };
    let redact = |target: &str, channel: &str| {
        EventBuilder::new(Kind::Custom(9005), "")
            .tags(vec![tag(&["e", target]), tag(&["h", channel])])
            .sign_with_keys(&owner)
            .unwrap()
    };
    let message_id = message.id.to_hex();
    let cases = [
        ("kind 5, missing", delete(&missing, None)),
        ("kind 5, message", delete(&message_id, None)),
        ("kind 5, wrong k", delete(&message_id, Some("30300"))),
        ("kind 5, artifact", delete(&artifact.id.to_hex(), None)),
        ("9005, missing", redact(&missing, &active)),
        ("9005, archived target", redact(&message_id, &active)),
    ];
    let mut expected = None;
    for (case, deletion) in cases {
        let (status, http_msg, ws_msg) = rejection(&client, &owner, deletion).await;
        assert_eq!(http_msg, DENIED, "{case}: HTTP");
        assert_eq!(ws_msg, DENIED, "{case}: WS");
        assert_eq!(
            *expected.get_or_insert(status),
            status,
            "{case}: HTTP status"
        );
    }
    // A 9005 naming the archived channel is refused for that channel before
    // the target is read, so the removal marker matches a missing target.
    assert_eq!(
        rejection(&client, &owner, redact(&marker, &archived)).await,
        rejection(&client, &owner, redact(&missing, &archived)).await,
    );

    // In an active channel the owner still gets the target diagnostics.
    let live = post_message(&client, &owner, &active).await;
    let (_, http_msg, _) =
        rejection(&client, &owner, delete(&live.id.to_hex(), Some("30300"))).await;
    assert!(
        http_msg.contains("does not match the deletion target's kind"),
        "{http_msg}"
    );
    let live_artifact = post_artifact(&client, &owner, &active).await;
    let (_, http_msg, _) =
        rejection(&client, &owner, delete(&live_artifact.id.to_hex(), None)).await;
    assert!(
        http_msg.contains("artifacts cannot be deleted with kind 5"),
        "{http_msg}"
    );
}

/// Whether `reader`'s open subscription `sid` receives any event within 2s.
async fn receives_any(ws: &mut BuzzTestClient, sid: &str) -> bool {
    let mut delivered = false;
    while let Ok(msg) = ws.recv_event(Duration::from_secs(2)).await {
        delivered |=
            matches!(msg, RelayMessage::Event { subscription_id, .. } if subscription_id == sid);
    }
    delivered
}

/// Deleting a private deletion would store a public event naming it, so the
/// author is refused (NIP-09 gives such a deletion no effect), everyone else
/// gets the shared denial, and no reader but the author ever sees either.
#[tokio::test]
#[ignore]
async fn deletion_of_private_deletion_rejected() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let (_, private) = store_deleted_reminder(&client, &author).await;
    let own = create_channel(&client, &author, "open").await;

    let live = Filter::new()
        .kind(Kind::EventDeletion)
        .author(author.public_key());
    let mut watcher = BuzzTestClient::connect(&relay_url(), &other)
        .await
        .expect("connect");
    let sid = sub_id("chain");
    watcher.subscribe(&sid, vec![live]).await.expect("REQ");
    assert!(watcher
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("EOSE")
        .is_empty());

    let target = private.id.to_hex();
    for k in [None, Some("5")] {
        let mut tags = vec![tag(&["e", &target])];
        tags.extend(k.map(|k| tag(&["k", k])));
        let (status, http_msg, ws_msg) =
            rejection(&client, &author, build_deletion(&author, tags)).await;
        assert!(status >= 400, "k={k:?}: HTTP must reject");
        for msg in [http_msg, ws_msg] {
            assert_eq!(
                msg, "invalid: cannot delete a private deletion request",
                "k={k:?}"
            );
        }
    }
    // A 9005 cannot reach the global private deletion from any channel.
    let redact = EventBuilder::new(Kind::Custom(9005), "")
        .tags(vec![tag(&["e", &target]), tag(&["h", &own])])
        .sign_with_keys(&author)
        .unwrap();
    let (_, http_msg, ws_msg) = rejection(&client, &author, redact).await;
    assert_eq!((http_msg.as_str(), ws_msg.as_str()), (DENIED, DENIED));

    let missing = "a".repeat(64);
    assert_eq!(
        rejection(
            &client,
            &other,
            build_deletion(&other, vec![tag(&["e", &target])])
        )
        .await,
        rejection(
            &client,
            &other,
            build_deletion(&other, vec![tag(&["e", &missing])])
        )
        .await,
    );
    assert_rejected_as_non_author(
        &client,
        &other,
        build_deletion(&other, vec![tag(&["e", &target])]),
    )
    .await;

    // Only the private deletion exists: the author alone reads it.
    assert_deletion_visibility(&author, &author, private.id, true).await;
    assert_deletion_visibility(&other, &author, private.id, false).await;
    assert!(
        !receives_any(&mut watcher, &sid).await,
        "no deletion may reach another reader live"
    );
    watcher.disconnect().await.expect("disconnect");

    // Deleting a public deletion is unchanged.
    let note = EventBuilder::text_note("public note")
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &note).await;
    let public = build_deletion(&author, vec![tag(&["e", &note.id.to_hex()])]);
    submit_ok(&client, &author, &public).await;
    submit_ok(
        &client,
        &author,
        &build_deletion(&author, vec![tag(&["e", &public.id.to_hex()])]),
    )
    .await;
}

/// A soft-deleted target keeps its channel: re-deleting it faces the same
/// membership and archive gates as a live one, and an accepted re-deletion is
/// readable by that channel's members only. Re-deletions now retain the
/// target's stored channel, so historical access, `#h` filtering and live
/// routing use that channel rather than treating a new re-deletion as global.
#[tokio::test]
#[ignore]
async fn soft_deleted_target_keeps_its_channel() {
    let client = http_client();
    let owner = Keys::generate();
    let author = Keys::generate();
    let outsider = Keys::generate();
    let private = create_channel(&client, &owner, "private").await;
    let archived = create_channel(&client, &author, "open").await;
    let membership = |kind: u16| {
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(vec![
                tag(&["h", &private]),
                tag(&["p", &author.public_key().to_hex()]),
            ])
            .sign_with_keys(&owner)
            .unwrap()
    };
    let delete =
        |target: &nostr::Event| build_deletion(&author, vec![tag(&["e", &target.id.to_hex()])]);
    let soft_deleted = |channel: String| {
        let client = client.clone();
        let author = author.clone();
        async move {
            let message = post_message(&client, &author, &channel).await;
            submit_ok(&client, &author, &delete(&message)).await;
            message
        }
    };

    submit_ok(&client, &owner, &membership(9000)).await;
    let redeletable = soft_deleted(private.clone()).await;
    let redeletion = build_deletion_with_reason(
        &author,
        vec![tag(&["e", &redeletable.id.to_hex()])],
        "re-deletion",
    );
    assert_ne!(redeletion.id, delete(&redeletable).id);

    // Subscribe before the re-deletion: the member by channel, the outsider
    // by author. A global control deletion proves the outsider's is alive.
    let h = nostr::SingleLetterTag::lowercase(nostr::Alphabet::H);
    let mut watchers = Vec::new();
    for (reader, filter) in [
        (
            &owner,
            Filter::new()
                .kind(Kind::EventDeletion)
                .custom_tag(h, private.clone()),
        ),
        (
            &outsider,
            Filter::new()
                .kind(Kind::EventDeletion)
                .author(author.public_key()),
        ),
    ] {
        let mut ws = BuzzTestClient::connect(&relay_url(), reader)
            .await
            .expect("connect");
        let sid = sub_id("redeletion");
        ws.subscribe(&sid, vec![filter]).await.expect("REQ");
        ws.collect_until_eose(&sid, Duration::from_secs(5))
            .await
            .expect("EOSE");
        watchers.push((ws, sid));
    }
    let (accepted, msg) = submit(&client, &author, &redeletion).await;
    assert!(
        accepted && !msg.starts_with("duplicate:"),
        "re-deletion: {msg}"
    );
    let note = EventBuilder::text_note("control")
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &note).await;
    let control = delete(&note);
    submit_ok(&client, &author, &control).await;
    let [member_ids, outsider_ids] = {
        let mut out = [Vec::new(), Vec::new()];
        for (slot, (mut ws, sid)) in out.iter_mut().zip(watchers) {
            loop {
                match ws.recv_event(Duration::from_secs(2)).await {
                    Ok(RelayMessage::Event {
                        subscription_id,
                        event,
                    }) if subscription_id == sid => slot.push(event.id),
                    Ok(RelayMessage::Closed {
                        subscription_id,
                        message,
                    }) if subscription_id == sid => panic!("{sid} closed: {message}"),
                    Ok(_) => {}
                    Err(TestClientError::Timeout) => break,
                    Err(e) => panic!("{sid} receive failed: {e}"),
                }
            }
            ws.disconnect().await.expect("disconnect");
        }
        out
    };
    assert_eq!(member_ids, vec![redeletion.id], "member live delivery");
    assert_eq!(outsider_ids, vec![control.id], "outsider live delivery");

    let removed = soft_deleted(private.clone()).await;
    submit_ok(&client, &owner, &membership(9001)).await;
    let in_archive = soft_deleted(archived.clone()).await;
    let archive = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![tag(&["h", &archived]), tag(&["archived", "true"])])
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &archive).await;

    let missing = build_deletion(&author, vec![tag(&["e", &"a".repeat(64)])]);
    let expected = rejection(&client, &author, missing).await;
    assert_eq!(expected.1, DENIED);
    for (case, target) in [("removed", &removed), ("archived", &in_archive)] {
        assert_eq!(
            rejection(&client, &author, delete(target)).await,
            expected,
            "{case}"
        );
    }

    for (reader, name, visible) in [(&owner, "member", true), (&outsider, "outsider", false)] {
        for filter in [
            Filter::new().id(redeletion.id),
            Filter::new()
                .kind(Kind::EventDeletion)
                .custom_tag(h, private.clone()),
        ] {
            let ids = http_query_ids(&client, reader, &filter).await;
            assert_eq!(
                ids.contains(&redeletion.id.to_hex()),
                visible,
                "{name} {filter:?}: {ids:?}"
            );
        }
    }
}

/// The single-target rule depends only on the request, so a deletion naming
/// two targets gets the same error whatever those targets are.
#[tokio::test]
#[ignore]
async fn multi_target_deletion_error_ignores_targets() {
    let client = http_client();
    let author = Keys::generate();
    let owner = Keys::generate();
    let open = create_channel(&client, &author, "open").await;
    let elsewhere = create_channel(&client, &owner, "private").await;
    let foreign = post_message(&client, &owner, &elsewhere).await;
    let live = post_message(&client, &author, &open).await;
    let reminder = build_reminder(&author);
    submit_ok(&client, &author, &reminder).await;
    let artifact = post_artifact(&client, &author, &open).await;
    let coordinate = format!(
        "{KIND_EVENT_REMINDER}:{}:{}",
        author.public_key().to_hex(),
        reminder.tags.identifier().expect("d tag")
    );
    let missing = "a".repeat(64);
    let targets = [
        ("missing", missing.clone()),
        ("live", live.id.to_hex()),
        ("other channel", foreign.id.to_hex()),
        ("private kind", reminder.id.to_hex()),
        ("artifact", artifact.id.to_hex()),
    ];
    let shapes = [
        ("two e", "(got e=2, a=0)", "e", missing.as_str()),
        ("e plus a", "(got e=1, a=1)", "a", coordinate.as_str()),
    ];
    for (shape, counts, second, value) in shapes {
        let expected = format!(
            "invalid: deletion events must reference exactly one target via e or a tag {counts}"
        );
        let mut status = None;
        for (case, target) in &targets {
            let deletion =
                build_deletion(&author, vec![tag(&["e", target]), tag(&[second, value])]);
            let (code, http_msg, ws_msg) = rejection(&client, &author, deletion).await;
            assert_eq!(http_msg, expected, "{shape}, {case}: HTTP");
            assert_eq!(ws_msg, expected, "{shape}, {case}: WS");
            assert_eq!(*status.get_or_insert(code), code, "{shape}, {case}: status");
        }
    }
}
