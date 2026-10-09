//! End-to-end tests for kind:44300 instructions versions (NIP-AP).
//!
//! A version is one owner-signed, NIP-44-encrypted save of an agent's (`p`) or
//! a team's (`t`) instructions. These tests assert the relay contract from
//! NIP-AP "Instructions versions: kind:44300" and "Relay behavior":
//! - ingest validation of the subject tag, the `h` ban, and the content bound;
//! - author-only reads on REQ, `ids` lookup, live fan-out, WS and HTTP COUNT,
//!   and the HTTP bridge query, with the owner as the positive control;
//! - exclusion from full-text search, for the author too;
//! - per-subject `limit: 1` current-version reads and composite
//!   `(until, before_id)` history paging across tied timestamps.
//!
//! The relay does not decrypt; content here is an opaque placeholder.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_instructions_versions -- --ignored
//! ```

use std::collections::HashSet;
use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use serde_json::{json, Value};

const INSTRUCTIONS_VERSION_KIND: u16 = 44300;
const MAX_CONTENT_BYTES: usize = 218_548;
const CIPHERTEXT: &str = "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
    format!("e2e-instr-{name}-{}", uuid::Uuid::new_v4())
}

fn version_event(
    keys: &Keys,
    tags: Vec<Vec<&str>>,
    content: &str,
    created_at: u64,
) -> nostr::Event {
    EventBuilder::new(Kind::Custom(INSTRUCTIONS_VERSION_KIND), content)
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

fn agent_version(keys: &Keys, agent: &str, created_at: u64) -> nostr::Event {
    version_event(keys, vec![vec!["p", agent]], CIPHERTEXT, created_at)
}

fn team_version(keys: &Keys, team: &str, created_at: u64) -> nostr::Event {
    version_event(keys, vec![vec!["t", team]], CIPHERTEXT, created_at)
}

fn now() -> u64 {
    Timestamp::now().as_secs()
}

fn new_agent() -> String {
    Keys::generate().public_key().to_hex()
}

fn p_tag() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

async fn publish(client: &mut BuzzTestClient, event: nostr::Event) {
    let ok = client.send_event(event).await.expect("send version");
    assert!(
        ok.accepted,
        "relay rejected a valid version: {}",
        ok.message
    );
}

async fn req(client: &mut BuzzTestClient, name: &str, filter: Value) -> Vec<nostr::Event> {
    let sid = sub_id(name);
    client
        .send_raw(&json!(["REQ", sid, filter]))
        .await
        .expect("send REQ");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    client.close_subscription(&sid).await.expect("close");
    events
}

async fn ws_count(client: &mut BuzzTestClient, filter: Value) -> u64 {
    let sid = sub_id("count");
    client
        .send_raw(&json!(["COUNT", sid, filter]))
        .await
        .expect("send COUNT");
    loop {
        match client
            .recv_event(Duration::from_secs(5))
            .await
            .expect("recv")
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

async fn http_post(keys: &Keys, path: &str, filters: Value) -> (u16, Value) {
    let resp = reqwest::Client::new()
        .post(format!("{}{path}", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(&filters)
        .send()
        .await
        .expect("http request");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

fn ids(events: &[nostr::Event]) -> Vec<String> {
    events.iter().map(|e| e.id.to_hex()).collect()
}

/// Every envelope rule rejects with `invalid:`; tags are counted by first
/// element, so a valueless subject tag counts toward the exactly-one rule.
#[tokio::test]
#[ignore]
async fn test_instructions_version_rejects_malformed_envelopes() {
    let keys = Keys::generate();
    let agent = new_agent();
    let upper = agent.to_uppercase();
    let long_team = "t".repeat(65);
    let oversized = "A".repeat(MAX_CONTENT_BYTES + 1);
    let channel = uuid::Uuid::new_v4().to_string();
    let cases: Vec<(&str, Vec<Vec<&str>>, &str)> = vec![
        ("no subject", vec![], CIPHERTEXT),
        (
            "p and t",
            vec![vec!["p", &agent], vec!["t", "team-1"]],
            CIPHERTEXT,
        ),
        (
            "two p",
            vec![vec!["p", &agent], vec!["p", &agent]],
            CIPHERTEXT,
        ),
        (
            "two t",
            vec![vec!["t", "team-1"], vec!["t", "team-2"]],
            CIPHERTEXT,
        ),
        ("valueless p", vec![vec!["p"]], CIPHERTEXT),
        (
            "valueless t plus valued p",
            vec![vec!["t"], vec!["p", &agent]],
            CIPHERTEXT,
        ),
        (
            "three-element p",
            vec![vec!["p", &agent, "wss://relay"]],
            CIPHERTEXT,
        ),
        ("uppercase p", vec![vec!["p", &upper]], CIPHERTEXT),
        ("short p", vec![vec!["p", &agent[..63]]], CIPHERTEXT),
        ("empty t", vec![vec!["t", ""]], CIPHERTEXT),
        ("whitespace t", vec![vec!["t", "team 1"]], CIPHERTEXT),
        ("long t", vec![vec!["t", &long_team]], CIPHERTEXT),
        (
            "h tag",
            vec![vec!["p", &agent], vec!["h", &channel]],
            CIPHERTEXT,
        ),
        ("empty content", vec![vec!["p", &agent]], ""),
        ("oversized content", vec![vec!["p", &agent]], &oversized),
    ];

    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    for (name, tags, content) in cases {
        let ok = client
            .send_event(version_event(&keys, tags, content, now()))
            .await
            .expect("send version");
        assert!(!ok.accepted, "{name}: must be rejected");
        assert!(
            ok.message.starts_with("invalid:"),
            "{name}: expected an `invalid:` refusal, got: {}",
            ok.message
        );
    }
    client.disconnect().await.expect("disconnect");
}

/// Both subject forms are accepted: a lowercase-hex agent pubkey and a team id
/// using the 30178 grammar (colons allowed, case preserved), and content at
/// exactly the ciphertext bound.
#[tokio::test]
#[ignore]
async fn test_instructions_version_accepts_agent_and_team_subjects() {
    let keys = Keys::generate();
    let max = "A".repeat(MAX_CONTENT_BYTES);
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    publish(&mut client, agent_version(&keys, &new_agent(), now())).await;
    publish(
        &mut client,
        team_version(&keys, "builtin-team:Welcome", now()),
    )
    .await;
    publish(
        &mut client,
        version_event(&keys, vec![vec!["p", &new_agent()]], &max, now()),
    )
    .await;
    client.disconnect().await.expect("disconnect");
}

/// Historical REQ, `ids` lookup and WS COUNT: the owner sees the version; a
/// non-author's 44300-only filter is refused and a mixed-kind or `ids` filter that
/// would match it returns nothing.
#[tokio::test]
#[ignore]
async fn test_instructions_version_author_only_ws_reads() {
    let url = relay_url();
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let agent = new_agent();
    let event = agent_version(&owner, &agent, now());
    let id = event.id.to_hex();

    let mut owner_ws = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");
    publish(&mut owner_ws, event).await;
    let mut outsider_ws = BuzzTestClient::connect(&url, &outsider)
        .await
        .expect("connect");

    let by_kind =
        json!({"kinds": [44300], "authors": [owner.public_key().to_hex()], "#p": [agent]});
    // Mixed with public kind:1 so the filter is not closed up front and the
    // per-event author-only gate is what hides the version. (A kindless `#p`
    // filter for someone else is refused by the existing p-gated rule.)
    let by_tag = json!({"kinds": [1, 44300], "#p": [agent]});
    let by_id = json!({"ids": [id]});

    for filter in [&by_kind, &by_tag, &by_id] {
        assert_eq!(
            ids(&req(&mut owner_ws, "owner", filter.clone()).await),
            vec![id.clone()]
        );
        assert_eq!(
            ws_count(&mut owner_ws, filter.clone()).await,
            1,
            "owner count {filter}"
        );
    }
    for filter in [&by_tag, &by_id] {
        assert!(
            req(&mut outsider_ws, "outsider", filter.clone())
                .await
                .is_empty(),
            "non-author must not see the version via {filter}"
        );
        assert_eq!(
            ws_count(&mut outsider_ws, filter.clone()).await,
            0,
            "outsider count {filter}"
        );
    }

    let sid = sub_id("outsider-kind");
    outsider_ws
        .send_raw(&json!(["REQ", sid, by_kind]))
        .await
        .expect("send REQ");
    match outsider_ws
        .recv_event(Duration::from_secs(5))
        .await
        .expect("recv")
    {
        RelayMessage::Closed { message, .. } => {
            assert!(message.starts_with("restricted:"), "{message}")
        }
        other => panic!("expected CLOSED for another author's versions, got {other:?}"),
    }

    owner_ws.disconnect().await.expect("disconnect");
    outsider_ws.disconnect().await.expect("disconnect");
}

/// Live fan-out reaches the owner's subscription but not a non-author's
/// mixed-kind (1 + 44300) subscription that matches the version's `p` tag. A public kind:1
/// note with the same `p` tag proves the non-author's subscription is live.
#[tokio::test]
#[ignore]
async fn test_instructions_version_live_fanout_is_author_only() {
    let url = relay_url();
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let agent = new_agent();

    let mut owner_ws = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");
    let mut outsider_ws = BuzzTestClient::connect(&url, &outsider)
        .await
        .expect("connect");
    let owner_sid = sub_id("owner-live");
    let outsider_sid = sub_id("outsider-live");
    for (ws, sid) in [
        (&mut owner_ws, &owner_sid),
        (&mut outsider_ws, &outsider_sid),
    ] {
        ws.subscribe(
            sid,
            vec![Filter::new()
                .kinds([Kind::TextNote, Kind::Custom(INSTRUCTIONS_VERSION_KIND)])
                .custom_tags(p_tag(), [agent.as_str()])
                .since(Timestamp::now())],
        )
        .await
        .expect("subscribe");
        ws.collect_until_eose(sid, Duration::from_secs(5))
            .await
            .expect("eose");
    }

    let version = agent_version(&owner, &agent, now());
    let version_id = version.id;
    publish(&mut owner_ws, version).await;
    let control = EventBuilder::new(Kind::TextNote, "public control")
        .tag(Tag::parse(["p", agent.as_str()]).unwrap())
        .sign_with_keys(&owner)
        .unwrap();
    let control_id = control.id;
    publish(&mut owner_ws, control).await;

    let mut owner_seen = HashSet::new();
    while owner_seen.len() < 2 {
        if let RelayMessage::Event {
            subscription_id,
            event,
        } = owner_ws
            .recv_event(Duration::from_secs(5))
            .await
            .expect("owner live")
        {
            if subscription_id == owner_sid {
                owner_seen.insert(event.id);
            }
        }
    }
    assert!(
        owner_seen.contains(&version_id),
        "owner must receive the version live"
    );

    loop {
        if let RelayMessage::Event {
            subscription_id,
            event,
        } = outsider_ws
            .recv_event(Duration::from_secs(5))
            .await
            .expect("outsider live")
        {
            if subscription_id != outsider_sid {
                continue;
            }
            assert_ne!(event.id, version_id, "non-author received the version live");
            if event.id == control_id {
                break;
            }
        }
    }

    owner_ws.disconnect().await.expect("disconnect");
    outsider_ws.disconnect().await.expect("disconnect");
}

/// HTTP bridge `/query` and `/count`: the owner sees and counts the version;
/// a non-author's matching mixed-kind and `ids` queries return nothing and count zero.
#[tokio::test]
#[ignore]
async fn test_instructions_version_author_only_http_reads() {
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let agent = new_agent();
    let event = agent_version(&owner, &agent, now());
    let id = event.id.to_hex();
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");
    publish(&mut ws, event).await;
    ws.disconnect().await.expect("disconnect");

    for filter in [
        json!({"kinds": [1, 44300], "#p": [agent]}),
        json!({"ids": [id]}),
    ] {
        let (status, body) = http_post(&owner, "/query", json!([filter])).await;
        assert_eq!(status, 200);
        assert_eq!(body[0]["id"], json!(id), "owner query {filter}");
        let (_, body) = http_post(&owner, "/count", json!([filter])).await;
        assert_eq!(body["count"], json!(1), "owner count {filter}");

        let (status, body) = http_post(&outsider, "/query", json!([filter])).await;
        assert_eq!(status, 200);
        assert_eq!(body, json!([]), "non-author query {filter}");
        let (_, body) = http_post(&outsider, "/count", json!([filter])).await;
        assert_eq!(body["count"], json!(0), "non-author count {filter}");
    }
}

/// `limit: 1` returns the subject's current version even when the owner has
/// newer versions for other subjects, for both `#p` and `#t`.
#[tokio::test]
#[ignore]
async fn test_instructions_version_limit_one_selects_subject_current() {
    let owner = Keys::generate();
    let agent = new_agent();
    let team = format!("team-{}", uuid::Uuid::new_v4());
    let base = now() - 100;
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");

    let agent_old = agent_version(&owner, &agent, base);
    let agent_current = agent_version(&owner, &agent, base + 1);
    let team_old = team_version(&owner, &team, base);
    let team_current = team_version(&owner, &team, base + 1);
    let (agent_id, team_id) = (agent_current.id.to_hex(), team_current.id.to_hex());
    for e in [agent_old, agent_current, team_old, team_current] {
        publish(&mut ws, e).await;
    }
    for i in 0..5 {
        publish(&mut ws, agent_version(&owner, &new_agent(), base + 10 + i)).await;
        publish(
            &mut ws,
            team_version(
                &owner,
                &format!("other-{}", uuid::Uuid::new_v4()),
                base + 10 + i,
            ),
        )
        .await;
    }

    let author = owner.public_key().to_hex();
    let got = req(
        &mut ws,
        "agent-current",
        json!({"kinds": [44300], "authors": [author], "#p": [agent], "limit": 1}),
    )
    .await;
    assert_eq!(ids(&got), vec![agent_id]);
    let got = req(
        &mut ws,
        "team-current",
        json!({"kinds": [44300], "authors": [author], "#t": [team], "limit": 1}),
    )
    .await;
    assert_eq!(ids(&got), vec![team_id]);
    ws.disconnect().await.expect("disconnect");
}

/// The `#t` subject match is positional, not JSONB containment: a newer version
/// for another team must not take the `limit: 1` slot when it merely contains
/// the wanted value elsewhere — through an unrelated `["meta","t",<team>]` tag,
/// or (for the valid team id `t`) as the tag name itself. WS REQ and HTTP.
#[tokio::test]
#[ignore]
async fn test_instructions_version_team_subject_match_is_exact() {
    let owner = Keys::generate();
    let author = owner.public_key().to_hex();
    let base = now() - 100;
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");

    let wanted = format!("team-{}", uuid::Uuid::new_v4());
    for team in [wanted.as_str(), "t"] {
        let current = team_version(&owner, team, base);
        let current_id = current.id.to_hex();
        let decoy = version_event(
            &owner,
            vec![
                vec!["t", &format!("other-{}", uuid::Uuid::new_v4())],
                vec!["meta", "t", team],
            ],
            CIPHERTEXT,
            base + 1,
        );
        publish(&mut ws, current).await;
        publish(&mut ws, decoy).await;

        let filter = json!({"kinds": [44300], "authors": [author], "#t": [team], "limit": 1});
        let got = req(&mut ws, "team-exact", filter.clone()).await;
        assert_eq!(ids(&got), vec![current_id.clone()], "WS team {team}");
        let (status, body) = http_post(&owner, "/query", json!([filter])).await;
        assert_eq!(status, 200);
        assert_eq!(
            body.as_array().map(Vec::len),
            Some(1),
            "HTTP team {team}: {body}"
        );
        assert_eq!(body[0]["id"], json!(current_id), "HTTP team {team}");
    }
    ws.disconnect().await.expect("disconnect");
}

/// History pages by `(until, before_id)` across versions sharing one
/// `created_at`, on both WS REQ and HTTP `/query`: every version exactly once,
/// ordered by `created_at` desc then `id` asc, ending on an empty page.
#[tokio::test]
#[ignore]
async fn test_instructions_version_history_pages_tied_timestamps() {
    let owner = Keys::generate();
    let agent = new_agent();
    let base = now() - 100;
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");
    let mut expected: Vec<nostr::Event> = Vec::new();
    for i in 0..7 {
        let created_at = if i < 5 { base + 1 } else { base };
        let e = version_event(
            &owner,
            vec![vec!["p", &agent], vec!["alt", &i.to_string()]],
            CIPHERTEXT,
            created_at,
        );
        expected.push(e.clone());
        publish(&mut ws, e).await;
    }
    expected.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(a.id.to_hex().cmp(&b.id.to_hex()))
    });
    let expected = ids(&expected);

    let base_filter = json!({"kinds": [44300], "authors": [owner.public_key().to_hex()], "#p": [agent], "limit": 2});
    for transport in ["ws", "http"] {
        let mut filter = base_filter.clone();
        let mut seen: Vec<String> = Vec::new();
        loop {
            let page: Vec<(String, u64)> = if transport == "ws" {
                req(&mut ws, "history", filter.clone())
                    .await
                    .iter()
                    .map(|e| (e.id.to_hex(), e.created_at.as_secs()))
                    .collect()
            } else {
                let (status, body) = http_post(&owner, "/query", json!([filter])).await;
                assert_eq!(status, 200, "{body}");
                body.as_array()
                    .unwrap()
                    .iter()
                    .map(|e| {
                        (
                            e["id"].as_str().unwrap().to_string(),
                            e["created_at"].as_u64().unwrap(),
                        )
                    })
                    .collect()
            };
            let Some((last_id, last_at)) = page.last().cloned() else {
                break;
            };
            seen.extend(page.into_iter().map(|(id, _)| id));
            filter["until"] = json!(last_at);
            filter["before_id"] = json!(last_id);
            assert!(
                seen.len() <= expected.len(),
                "{transport}: paging repeated versions: {seen:?}"
            );
        }
        assert_eq!(
            seen, expected,
            "{transport}: history must list every version once, in order"
        );
    }
    ws.disconnect().await.expect("disconnect");
}

/// A `before_id` without `until`, or one that is not 64 hex characters, is
/// rejected on both transports rather than ignored.
#[tokio::test]
#[ignore]
async fn test_instructions_version_rejects_malformed_cursor() {
    let owner = Keys::generate();
    let agent = new_agent();
    let author = owner.public_key().to_hex();
    let bad = [
        json!({"kinds": [44300], "authors": [author], "#p": [agent], "before_id": "a".repeat(64)}),
        json!({"kinds": [44300], "authors": [author], "#p": [agent], "until": now(), "before_id": "a".repeat(63)}),
        json!({"kinds": [44300], "authors": [author], "#p": [agent], "until": now(), "before_id": "z".repeat(64)}),
    ];
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");
    for filter in bad {
        let (status, _) = http_post(&owner, "/query", json!([filter])).await;
        assert_eq!(status, 400, "HTTP must reject {filter}");

        let sid = sub_id("bad-cursor");
        ws.send_raw(&json!(["REQ", sid, filter]))
            .await
            .expect("send REQ");
        match ws.recv_event(Duration::from_secs(5)).await.expect("recv") {
            RelayMessage::Notice { message } => assert!(message.contains("before_id"), "{message}"),
            other => panic!("WS must reject {filter}, got {other:?}"),
        }
    }
    ws.disconnect().await.expect("disconnect");
}

/// Full-text search never returns a version, not even to its author.
#[tokio::test]
#[ignore]
async fn test_instructions_version_unsearchable_for_author() {
    let owner = Keys::generate();
    let token = format!("ivsearch{}", &new_agent()[..16]);
    let event = version_event(&owner, vec![vec!["p", &new_agent()]], &token, now());
    let mut ws = BuzzTestClient::connect(&relay_url(), &owner)
        .await
        .expect("connect");
    publish(&mut ws, event).await;

    let hits = req(
        &mut ws,
        "search",
        json!({"kinds": [INSTRUCTIONS_VERSION_KIND], "authors": [owner.public_key().to_hex()], "search": token}),
    )
    .await;
    assert!(
        hits.is_empty(),
        "author search returned versions: {:?}",
        ids(&hits)
    );
    ws.disconnect().await.expect("disconnect");
}
