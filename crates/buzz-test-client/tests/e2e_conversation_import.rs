//! End-to-end tests for NIP-CI conversation import.
//!
//! These tests require a running relay and are `#[ignore]`d by default.
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_conversation_import -- --ignored
//! ```
//!
//! Override the relay URL with `RELAY_URL`. All identifiers, hashes and
//! source references are synthetic.

use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};

const SOURCE_TYPE: &str = "test";

fn relay_http_url() -> String {
    std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "ws://localhost:3000".to_string())
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn now() -> u64 {
    Timestamp::now().as_secs()
}

fn hex64(byte: u8) -> String {
    hex::encode([byte; 32])
}

fn sign(keys: &Keys, kind: u16, content: &str, tags: Vec<Vec<&str>>, created_at: u64) -> Event {
    let tags = tags
        .into_iter()
        .map(|t| Tag::parse(t).expect("tag"))
        .collect::<Vec<_>>();
    // A proposal's `p` tags list every participant, the proposer included;
    // the builder drops a self `p` tag unless told otherwise.
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .allow_self_tagging()
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .expect("sign")
}

/// Submit an event over HTTP; returns `(accepted, message)`. A rejection is
/// `(false, <reason>)`.
async fn submit(keys: &Keys, event: &Event) -> (bool, String) {
    let resp = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).expect("event json"))
        .send()
        .await
        .expect("submit event");
    let ok = resp.status().is_success();
    let body: Value = resp.json().await.expect("response json");
    if ok {
        (
            body["accepted"].as_bool().unwrap_or(false),
            body["message"].as_str().unwrap_or_default().to_string(),
        )
    } else {
        (
            false,
            body["error"].as_str().unwrap_or_default().to_string(),
        )
    }
}

async fn accept(keys: &Keys, event: &Event) {
    let (accepted, message) = submit(keys, event).await;
    assert!(
        accepted,
        "kind:{} not accepted: {message}",
        event.kind.as_u16()
    );
}

async fn refused(keys: &Keys, event: &Event, reason: &str) {
    let (accepted, message) = submit(keys, event).await;
    assert!(
        !accepted,
        "kind:{} unexpectedly accepted",
        event.kind.as_u16()
    );
    assert_eq!(message, reason);
}

async fn query(keys: &Keys, filter: Value) -> Vec<Value> {
    let resp = reqwest::Client::new()
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(json!([filter]).to_string())
        .send()
        .await
        .expect("query");
    assert!(
        resp.status().is_success(),
        "query failed: {}",
        resp.status()
    );
    resp.json::<Value>()
        .await
        .expect("query json")
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Open a DM between `a` and `b` and return its channel id.
async fn create_dm(a: &Keys, b: &Keys) -> String {
    let b_hex = b.public_key().to_hex();
    let event = sign(a, 41010, "", vec![vec!["p", &b_hex]], now() - 10);
    let (accepted, message) = submit(a, &event).await;
    assert!(accepted, "DM open not accepted: {message}");
    let payload = message.strip_prefix("response:").expect("response: prefix");
    let parsed: Value = serde_json::from_str(payload).expect("response JSON");
    parsed["channel_id"]
        .as_str()
        .expect("channel_id")
        .to_string()
}

struct Participant<'a> {
    keys: &'a Keys,
    source_user: String,
    messages: u64,
    reactions: u64,
}

fn proposal(proposer: &Keys, channel: &str, participants: &[Participant<'_>]) -> Event {
    let at = now();
    let mut pubkeys: Vec<String> = participants
        .iter()
        .map(|p| p.keys.public_key().to_hex())
        .collect();
    pubkeys.sort();
    let manifest = hex64(0xab);
    let content = json!({
        "v": 1,
        "source": {
            "type": SOURCE_TYPE,
            "conversation": hex64(0xcd),
            "kind": "dm",
            "label": "Direct message",
        },
        "range": { "from": at - 86_400, "to": at - 60 },
        "participants": participants.iter().map(|p| json!({
            "pubkey": p.keys.public_key().to_hex(),
            "source_user": p.source_user,
            "messages": p.messages,
            "reactions": p.reactions,
        })).collect::<Vec<_>>(),
        "excluded_authors": 0,
        "manifest": manifest,
        "expires_at": at + 3_600,
    })
    .to_string();
    let mut tags = vec![vec!["h", channel]];
    tags.extend(pubkeys.iter().map(|p| vec!["p", p.as_str()]));
    tags.push(vec!["manifest", &manifest]);
    tags.push(vec!["import-source", SOURCE_TYPE]);
    sign(proposer, 42110, &content, tags, at)
}

fn consent(
    keys: &Keys,
    proposal: &Event,
    channel: &str,
    decision: &str,
    source_user: &str,
) -> Event {
    let proposal_id = proposal.id.to_hex();
    let proposer = proposal.pubkey.to_hex();
    let manifest = hex64(0xab);
    let content = json!({ "v": 1, "decision": decision, "source_user": source_user }).to_string();
    sign(
        keys,
        42111,
        &content,
        vec![
            vec!["e", &proposal_id],
            vec!["h", channel],
            vec!["manifest", &manifest],
            vec!["p", &proposer],
        ],
        now(),
    )
}

/// An imported kind:9 with tags in the canonical NIP-CI order.
fn imported_message(
    keys: &Keys,
    proposal: &Event,
    channel: &str,
    mentions: &[&Keys],
    source_ref: &str,
    created_at: u64,
) -> Event {
    let proposal_id = proposal.id.to_hex();
    let mut mention_hex: Vec<String> = mentions.iter().map(|k| k.public_key().to_hex()).collect();
    mention_hex.sort();
    let mut tags = vec![vec!["h", channel]];
    tags.extend(mention_hex.iter().map(|p| vec!["p", p.as_str()]));
    tags.push(vec!["import", &proposal_id]);
    tags.push(vec!["source", SOURCE_TYPE, source_ref]);
    sign(keys, 9, &format!("imported {source_ref}"), tags, created_at)
}

fn imported_reaction(keys: &Keys, proposal: &Event, channel: &str, target: &Event) -> Event {
    let proposal_id = proposal.id.to_hex();
    let target_id = target.id.to_hex();
    sign(
        keys,
        7,
        "+",
        vec![
            vec!["h", channel],
            vec!["e", &target_id],
            vec!["import", &proposal_id],
            vec!["source", SOURCE_TYPE, "reaction-target"],
        ],
        target.created_at.as_secs() + 1,
    )
}

async fn snapshot_event(keys: &Keys, proposal: &Event) -> Value {
    let events = query(
        keys,
        json!({ "kinds": [30623], "#d": [proposal.id.to_hex()], "limit": 5 }),
    )
    .await;
    events
        .into_iter()
        .max_by_key(|e| e["created_at"].as_u64().unwrap_or(0))
        .expect("snapshot exists")
}

async fn snapshot(keys: &Keys, proposal: &Event) -> Value {
    let newest = snapshot_event(keys, proposal).await;
    serde_json::from_str(newest["content"].as_str().expect("content")).expect("snapshot json")
}

/// The snapshot's `accepts_after` tag, if present.
async fn accepts_after(keys: &Keys, proposal: &Event) -> Option<u64> {
    let newest = snapshot_event(keys, proposal).await;
    newest["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .find(|t| t[0] == "accepts_after")
        .map(|t| t[1].as_str().expect("value").parse().expect("unix seconds"))
}

async fn mention_feed_ids(keys: &Keys) -> Vec<String> {
    query(
        keys,
        json!({ "#p": [keys.public_key().to_hex()], "feed_types": ["mentions"], "limit": 50 }),
    )
    .await
    .into_iter()
    .filter_map(|e| e["id"].as_str().map(str::to_string))
    .collect()
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn dm_import_session_end_to_end() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = create_dm(&alice, &bob).await;
    let at = now();
    let (alice_user, bob_user) = (hex64(0x11), hex64(0x22));
    let proposal = proposal(
        &alice,
        &dm,
        &[
            Participant {
                keys: &alice,
                source_user: alice_user,
                messages: 2,
                reactions: 0,
            },
            Participant {
                keys: &bob,
                source_user: bob_user.clone(),
                messages: 1,
                reactions: 1,
            },
        ],
    );
    accept(&alice, &proposal).await;
    assert_eq!(snapshot(&alice, &proposal).await["status"], "proposed");
    assert_eq!(accepts_after(&alice, &proposal).await, None);
    assert!(
        mention_feed_ids(&bob).await.contains(&proposal.id.to_hex()),
        "waiting proposal missing from the participant's mentions feed"
    );

    // Before Bob consents nothing backdated is accepted.
    let first = imported_message(&alice, &proposal, &dm, &[], "m1", at - 7_200);
    refused(&alice, &first, "invalid: import session not active").await;

    accept(&bob, &consent(&bob, &proposal, &dm, "accept", &bob_user)).await;
    assert_eq!(snapshot(&alice, &proposal).await["status"], "active");

    // Imports wait for the snapshot's accepts_after; an early one is
    // refused with a retryable reason and leaves no row behind.
    let opens_at = accepts_after(&alice, &proposal)
        .await
        .expect("an active snapshot carries accepts_after");
    assert!(opens_at > now(), "accepts_after {opens_at} is not ahead");
    refused(&alice, &first, "invalid: import session warming").await;
    // One extra second absorbs relay and test clocks rounding differently.
    let wait = (opens_at + 1).saturating_sub(now());
    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;

    // A message hours in the past is accepted under the active session.
    accept(&alice, &first).await;
    // Resending it is a duplicate, not a second count.
    let (accepted, message) = submit(&alice, &first).await;
    assert!(accepted && message.starts_with("duplicate"), "{message}");

    let outside = imported_message(&alice, &proposal, &dm, &[], "m0", at - 200_000);
    refused(
        &alice,
        &outside,
        "invalid: import timestamp outside declared range",
    )
    .await;

    // Bob's imported message mentions Alice; it must not reach her mentions.
    let mention = imported_message(&bob, &proposal, &dm, &[&alice], "m2", at - 7_000);
    accept(&bob, &mention).await;
    // The source type is bound to the session.
    let other_source = sign(
        &bob,
        9,
        "other source",
        vec![
            vec!["h", &dm],
            vec!["import", &proposal.id.to_hex()],
            vec!["source", "other", "m9"],
        ],
        at - 6_950,
    );
    refused(
        &bob,
        &other_source,
        "invalid: import source type does not match the session",
    )
    .await;
    let over = imported_message(&bob, &proposal, &dm, &[], "m3", at - 6_900);
    refused(&bob, &over, "invalid: import count exceeded").await;

    // A live mention does reach the feed, so the feed check below is live.
    let live = sign(
        &bob,
        9,
        "live mention",
        vec![vec!["h", &dm], vec!["p", &alice.public_key().to_hex()]],
        now(),
    );
    accept(&bob, &live).await;
    let feed = mention_feed_ids(&alice).await;
    assert!(
        feed.contains(&live.id.to_hex()),
        "live mention missing from feed"
    );
    assert!(
        !feed.contains(&mention.id.to_hex()),
        "imported mention reached the feed"
    );

    // Reactions must target imported events of the same proposal.
    let (live_id, proposal_hex) = (live.id.to_hex(), proposal.id.to_hex());
    let on_live = sign(
        &bob,
        7,
        "+",
        vec![
            vec!["h", &dm],
            vec!["e", &live_id],
            vec!["import", &proposal_hex],
            vec!["source", SOURCE_TYPE, "live"],
        ],
        at - 6_000,
    );
    refused(
        &bob,
        &on_live,
        "invalid: import reaction target not imported",
    )
    .await;
    accept(&bob, &imported_reaction(&bob, &proposal, &dm, &first)).await;

    // The imported message keeps its original timestamp.
    let stored = query(&alice, json!({ "ids": [first.id.to_hex()], "kinds": [9] })).await;
    assert_eq!(stored[0]["created_at"].as_u64(), Some(at - 7_200));

    let snap = snapshot(&alice, &proposal).await;
    let (a, b) = (alice.public_key().to_hex(), bob.public_key().to_hex());
    assert_eq!(snap["status"], "active");
    assert_eq!(accepts_after(&alice, &proposal).await, Some(opens_at));
    assert_eq!(snap["consents"][&a], "accept");
    assert_eq!(snap["consents"][&b], "accept");
    let mut required = vec![a.clone(), b.clone()];
    required.sort();
    assert_eq!(snap["required"], json!(required));

    // Someone outside the DM reads neither the snapshot nor the history.
    let stranger = Keys::generate();
    assert!(
        query(
            &stranger,
            json!({ "kinds": [30623], "#d": [proposal.id.to_hex()], "limit": 5 })
        )
        .await
        .is_empty(),
        "a non-member read the snapshot"
    );
    assert!(
        query(
            &stranger,
            json!({ "ids": [first.id.to_hex()], "kinds": [9] })
        )
        .await
        .is_empty(),
        "a non-member read imported history"
    );

    // Control records carry only their spec tags.
    let proposal_id = proposal.id.to_hex();
    let extra = sign(
        &alice,
        42112,
        r#"{"v":1}"#,
        vec![vec!["e", &proposal_id], vec!["h", &dm], vec!["ms", "1"]],
        now(),
    );
    refused(
        &alice,
        &extra,
        "invalid: import event malformed: unexpected tag ms",
    )
    .await;

    // Withdraw ends the session; nothing further is accepted.
    let withdraw = sign(
        &alice,
        42112,
        r#"{"v":1}"#,
        vec![vec!["e", &proposal_id], vec!["h", &dm]],
        now(),
    );
    accept(&alice, &withdraw).await;
    // Counters ride on control-event snapshots (and every 50 imports), so
    // the withdraw snapshot carries the totals.
    let snap = snapshot(&alice, &proposal).await;
    assert_eq!(snap["status"], "withdrawn");
    assert_eq!(
        snap["published"][&a],
        json!({ "messages": 1, "reactions": 0 })
    );
    assert_eq!(
        snap["published"][&b],
        json!({ "messages": 1, "reactions": 1 })
    );
    let late = imported_message(&alice, &proposal, &dm, &[], "m4", at - 5_000);
    refused(&alice, &late, "invalid: import session not active").await;
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn dm_proposal_participants_must_equal_members() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = create_dm(&alice, &bob).await;
    let only_alice = proposal(
        &alice,
        &dm,
        &[Participant {
            keys: &alice,
            source_user: hex64(0x11),
            messages: 1,
            reactions: 0,
        }],
    );
    refused(
        &alice,
        &only_alice,
        "invalid: import participants must equal the DM members",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn import_tags_without_a_session_are_refused() {
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = create_dm(&alice, &bob).await;
    let missing = sign(&alice, 42110, "{}", vec![vec!["h", &dm]], now());
    let orphan = imported_message(&alice, &missing, &dm, &[], "m1", now() - 3_600);
    refused(&alice, &orphan, "invalid: import proposal not found").await;

    let wrong_kind = sign(
        &alice,
        1,
        "note",
        vec![
            vec!["import", &hex64(0x01)],
            vec!["source", SOURCE_TYPE, "x"],
        ],
        now(),
    );
    refused(&alice, &wrong_kind, "invalid: import kind not allowed").await;
}
