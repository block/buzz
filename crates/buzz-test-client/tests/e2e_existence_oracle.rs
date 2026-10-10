//! End-to-end tests: a write that references an event, artifact or workflow
//! the sender cannot use or read gets exactly the response it would get for
//! an ID that does not exist.
//!
//! Each test sends the same write as an unauthorized sender against targets
//! that are missing, soft-deleted, in another channel, in a private channel
//! the sender has not joined, author-only, or of the wrong kind, over HTTP and
//! WS, and requires one identical HTTP status, HTTP error and WS `OK`
//! message. A positive control shows the legitimate write still succeeds.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p buzz-test-client --test e2e_existence_oracle -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, EventId, Keys, Kind, Tag};
use reqwest::Client;
use serde_json::Value;

const KIND_EVENT_REMINDER: u16 = 30300;
const MISSING: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

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

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client")
}

fn tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.to_vec()).expect("valid tag")
}

fn sign(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> nostr::Event {
    EventBuilder::new(Kind::Custom(kind), content)
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

/// Submit `event` as `sender` over HTTP and WS and return how each rejected
/// it: the HTTP status and error, and the WS `OK` message.
async fn rejection(client: &Client, sender: &Keys, event: nostr::Event) -> (u16, String, String) {
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", sender.public_key().to_hex())
        .json(&event)
        .send()
        .await
        .expect("submit event");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.expect("parse response");
    let http_msg = body["error"]
        .as_str()
        .or_else(|| body["message"].as_str())
        .unwrap_or("")
        .to_string();
    let mut ws = BuzzTestClient::connect(&relay_url(), sender)
        .await
        .expect("connect");
    let ok = ws.send_event(event).await.expect("OK");
    ws.disconnect().await.expect("disconnect");
    assert!(!ok.accepted, "WS must reject the event");
    (status, http_msg, ok.message)
}

/// Every case must be rejected over HTTP and WS with `expected`, and with
/// one HTTP status across all cases.
async fn assert_uniform(
    client: &Client,
    sender: &Keys,
    expected: &str,
    cases: Vec<(&str, nostr::Event)>,
) {
    let mut status_seen = None;
    let mut mismatches = Vec::new();
    for (case, event) in cases {
        let (status, http_msg, ws_msg) = rejection(client, sender, event).await;
        if status < 400 && http_msg.is_empty() {
            mismatches.push(format!("{case}: HTTP accepted"));
        }
        if http_msg != expected {
            mismatches.push(format!("{case}: HTTP {http_msg:?}"));
        }
        if ws_msg != expected {
            mismatches.push(format!("{case}: WS {ws_msg:?}"));
        }
        if *status_seen.get_or_insert(status) != status {
            mismatches.push(format!("{case}: HTTP status {status}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "expected {expected:?}:\n{}",
        mismatches.join("\n")
    );
}

async fn create_channel(client: &Client, owner: &Keys, visibility: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let event = sign(
        owner,
        9007,
        "",
        vec![
            tag(&["h", &id]),
            tag(&["name", &format!("eo-{id}")]),
            tag(&["channel_type", "stream"]),
            tag(&["visibility", visibility]),
        ],
    );
    submit_ok(client, owner, &event).await;
    id
}

async fn add_member(client: &Client, owner: &Keys, channel: &str, member: &Keys) {
    let event = sign(
        owner,
        9000,
        "",
        vec![
            tag(&["h", channel]),
            tag(&["p", &member.public_key().to_hex()]),
        ],
    );
    submit_ok(client, owner, &event).await;
}

async fn post(client: &Client, author: &Keys, kind: u16, channel: &str) -> nostr::Event {
    let event = sign(
        author,
        kind,
        &uuid::Uuid::new_v4().to_string(),
        vec![tag(&["h", channel])],
    );
    submit_ok(client, author, &event).await;
    event
}

async fn delete(client: &Client, author: &Keys, target: &nostr::Event) {
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![tag(&["e", &target.id.to_hex()])])
        .sign_with_keys(author)
        .unwrap();
    submit_ok(client, author, &deletion).await;
}

/// The author's targets in every place an outsider cannot use them.
struct Targets {
    /// An open channel the outsider has not joined.
    open: String,
    live: nostr::Event,
    deleted: nostr::Event,
    hidden: nostr::Event,
    reminder: nostr::Event,
}

async fn targets(client: &Client, author: &Keys, kind: u16) -> Targets {
    let open = create_channel(client, author, "open").await;
    let private = create_channel(client, author, "private").await;
    let live = post(client, author, kind, &open).await;
    let deleted = post(client, author, kind, &open).await;
    delete(client, author, &deleted).await;
    let hidden = post(client, author, kind, &private).await;
    let reminder = sign(
        author,
        KIND_EVENT_REMINDER,
        "nip44-ciphertext-placeholder",
        vec![
            tag(&["d", &uuid::Uuid::new_v4().to_string()]),
            tag(&["alt", "Encrypted reminder"]),
        ],
    );
    submit_ok(client, author, &reminder).await;
    Targets {
        open,
        live,
        deleted,
        hidden,
        reminder,
    }
}

fn hex(event: &nostr::Event) -> String {
    event.id.to_hex()
}

/// Reactions derive their channel from the target, so a target the reactor
/// cannot use, including someone else's author-only event, is rejected as
/// missing. Reacting to another user's author-only event used to be stored.
#[tokio::test]
#[ignore]
async fn reaction_does_not_reveal_target() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let t = targets(&client, &author, 9).await;
    // A live message the reactor could read, in an archived channel they belong to.
    let archived = create_channel(&client, &author, "open").await;
    add_member(&client, &author, &archived, &other).await;
    let in_archive = post(&client, &author, 9, &archived).await;
    let archive = sign(
        &author,
        9002,
        "",
        vec![tag(&["h", &archived]), tag(&["archived", "true"])],
    );
    submit_ok(&client, &author, &archive).await;
    let react = |target: &str| sign(&other, 7, "+", vec![tag(&["e", target])]);
    assert_uniform(
        &client,
        &other,
        "invalid: reaction target event not found",
        vec![
            ("missing", react(MISSING)),
            ("soft-deleted", react(&hex(&t.deleted))),
            ("private channel", react(&hex(&t.hidden))),
            ("author-only", react(&hex(&t.reminder))),
            ("archived channel", react(&hex(&in_archive))),
        ],
    )
    .await;

    // Positive controls: the author reacts to their reminder, and a member
    // reacts in the open channel.
    submit_ok(
        &client,
        &author,
        &sign(&author, 7, "+", vec![tag(&["e", &hex(&t.reminder)])]),
    )
    .await;
    add_member(&client, &author, &t.open, &other).await;
    submit_ok(&client, &other, &react(&hex(&t.live))).await;
}

/// An edit of a target that is not the sender's, or not in the edit's
/// channel, or unreadable, is rejected exactly like a missing target.
#[tokio::test]
#[ignore]
async fn edit_does_not_reveal_target() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let t = targets(&client, &author, 9).await;
    let own = create_channel(&client, &other, "open").await;
    add_member(&client, &other, &own, &author).await;
    let not_yours = post(&client, &author, 9, &own).await;
    let edit = |target: &str| {
        sign(
            &other,
            40003,
            "edited",
            vec![tag(&["e", target]), tag(&["h", &own])],
        )
    };
    assert_uniform(
        &client,
        &other,
        "invalid: edit target not found or not editable by you",
        vec![
            ("missing", edit(MISSING)),
            ("soft-deleted", edit(&hex(&t.deleted))),
            ("other channel", edit(&hex(&t.live))),
            ("private channel", edit(&hex(&t.hidden))),
            ("no channel / author-only", edit(&hex(&t.reminder))),
            ("not yours", edit(&hex(&not_yours))),
        ],
    )
    .await;

    // Positive control: the sender edits their own message.
    let mine = post(&client, &other, 9, &own).await;
    submit_ok(&client, &other, &edit(&hex(&mine))).await;
}

/// A vote's channel and readability check runs before its kind check, so an
/// unusable target never reveals its kind.
#[tokio::test]
#[ignore]
async fn forum_vote_does_not_reveal_target() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let t = targets(&client, &author, 45001).await;
    let wrong_kind = post(&client, &author, 9, &t.open).await;
    let own = create_channel(&client, &other, "open").await;
    let vote = |target: &str| {
        sign(
            &other,
            45002,
            "+",
            vec![tag(&["e", target]), tag(&["h", &own])],
        )
    };
    assert_uniform(
        &client,
        &other,
        "invalid: vote target event not found",
        vec![
            ("missing", vote(MISSING)),
            ("soft-deleted", vote(&hex(&t.deleted))),
            ("other channel", vote(&hex(&t.live))),
            ("private channel", vote(&hex(&t.hidden))),
            ("author-only", vote(&hex(&t.reminder))),
            ("other channel, wrong kind", vote(&hex(&wrong_kind))),
        ],
    )
    .await;

    // Positive control: a vote on a forum post in the vote's channel.
    let mine = post(&client, &other, 45001, &own).await;
    submit_ok(&client, &other, &vote(&hex(&mine))).await;
}

/// A report on an event the reporter cannot read is rejected exactly like a
/// report on a missing event. Such reports used to be queued.
#[tokio::test]
#[ignore]
async fn report_does_not_reveal_target() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let t = targets(&client, &author, 9).await;
    let report = |target: &str| {
        sign(
            &other,
            1984,
            "",
            vec![
                tag(&["e", target, "spam"]),
                tag(&["p", &author.public_key().to_hex()]),
            ],
        )
    };
    assert_uniform(
        &client,
        &other,
        "invalid: report target event not found",
        vec![
            ("missing", report(MISSING)),
            ("soft-deleted", report(&hex(&t.deleted))),
            ("private channel", report(&hex(&t.hidden))),
            ("author-only", report(&hex(&t.reminder))),
        ],
    )
    .await;

    // Positive controls: a readable event, and a report by pubkey.
    submit_ok(&client, &other, &report(&hex(&t.live))).await;
    let by_pubkey = sign(
        &other,
        1984,
        "",
        vec![tag(&["p", &author.public_key().to_hex(), "spam"])],
    );
    submit_ok(&client, &other, &by_pubkey).await;
}

/// A reply whose parent is outside the reply's channel or unreadable is
/// rejected exactly like a reply to a missing parent.
#[tokio::test]
#[ignore]
async fn reply_does_not_reveal_parent() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let t = targets(&client, &author, 9).await;
    let own = create_channel(&client, &other, "open").await;
    let reply = |parent: &str| {
        sign(
            &other,
            9,
            "reply",
            vec![tag(&["e", parent, "", "reply"]), tag(&["h", &own])],
        )
    };
    let (_, expected, _) = rejection(&client, &other, reply(MISSING)).await;
    assert!(
        expected.contains("reply parent not found"),
        "missing parent: {expected:?}"
    );
    assert_uniform(
        &client,
        &other,
        &expected,
        vec![
            ("missing", reply(MISSING)),
            ("soft-deleted", reply(&hex(&t.deleted))),
            ("other channel", reply(&hex(&t.live))),
            ("private channel", reply(&hex(&t.hidden))),
            ("no channel / author-only", reply(&hex(&t.reminder))),
        ],
    )
    .await;

    // Positive control: a reply in the parent's channel.
    let mine = post(&client, &other, 9, &own).await;
    submit_ok(&client, &other, &reply(&hex(&mine))).await;
}

fn artifact_revision(
    author: &Keys,
    artifact: &str,
    channel: &str,
    op: &str,
    kind_type: &str,
    prev: Option<&EventId>,
) -> nostr::Event {
    let mut tags = vec![
        tag(&["ar", "1"]),
        tag(&["d", artifact]),
        tag(&["h", channel]),
        tag(&["type", kind_type]),
        tag(&["op", op]),
    ];
    if op != "delete" {
        tags.push(tag(&["title", "Task"]));
    }
    tags.extend(prev.map(|p| tag(&["prev", &p.to_hex()])));
    sign(author, 45010, "", tags)
}

fn d_tag(event: &nostr::Event) -> String {
    event
        .tags
        .iter()
        .find_map(|t| {
            let s = t.as_slice();
            (s.first().map(String::as_str) == Some("d")).then(|| s[1].clone())
        })
        .expect("d tag")
}

/// Any artifact head outside a channel the sender can write answers
/// `head unavailable`, before the revision, type and deleted checks.
#[tokio::test]
#[ignore]
async fn artifact_revision_does_not_reveal_head() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let open = create_channel(&client, &author, "open").await;
    let private = create_channel(&client, &author, "private").await;
    let own = create_channel(&client, &other, "open").await;
    let create = |channel: &str| {
        artifact_revision(
            &author,
            &uuid::Uuid::new_v4().to_string(),
            channel,
            "create",
            "buzz.task",
            None,
        )
    };
    let live = create(&open);
    submit_ok(&client, &author, &live).await;
    let hidden = create(&private);
    submit_ok(&client, &author, &hidden).await;
    let gone = create(&open);
    submit_ok(&client, &author, &gone).await;
    let gone_delete = artifact_revision(
        &author,
        &d_tag(&gone),
        &open,
        "delete",
        "buzz.task",
        Some(&gone.id),
    );
    submit_ok(&client, &author, &gone_delete).await;

    let rev = |head: &nostr::Event, channel: &str, op: &str, ty: &str, prev: &EventId| {
        artifact_revision(&other, &d_tag(head), channel, op, ty, Some(prev))
    };
    let missing = artifact_revision(
        &other,
        &uuid::Uuid::new_v4().to_string(),
        &own,
        "update",
        "buzz.task",
        Some(&live.id),
    );
    assert_uniform(
        &client,
        &other,
        "conflict: artifact head unavailable",
        vec![
            ("missing", missing),
            (
                "other channel, current prev",
                rev(&live, &own, "update", "buzz.task", &live.id),
            ),
            (
                "other channel, stale prev",
                rev(&live, &own, "update", "buzz.task", &hidden.id),
            ),
            (
                "other channel, wrong type",
                rev(&live, &own, "update", "buzz.note", &live.id),
            ),
            (
                "other channel, delete",
                rev(&live, &own, "delete", "buzz.task", &live.id),
            ),
            (
                "private channel, update",
                rev(&hidden, &own, "update", "buzz.task", &hidden.id),
            ),
            (
                "private channel, move",
                rev(&hidden, &own, "move", "buzz.task", &hidden.id),
            ),
            (
                "deleted, restore",
                rev(&gone, &own, "restore", "buzz.task", &gone_delete.id),
            ),
            (
                "deleted, update",
                rev(&gone, &own, "update", "buzz.task", &gone_delete.id),
            ),
        ],
    )
    .await;

    // Positive control: the sender revises an artifact in their channel.
    let mine = artifact_revision(
        &other,
        &uuid::Uuid::new_v4().to_string(),
        &own,
        "create",
        "buzz.task",
        None,
    );
    submit_ok(&client, &other, &mine).await;
    submit_ok(
        &client,
        &other,
        &rev(&mine, &own, "update", "buzz.task", &mine.id),
    )
    .await;
}

/// Triggering someone else's workflow is rejected exactly like triggering a
/// workflow that does not exist.
#[tokio::test]
#[ignore]
async fn workflow_trigger_does_not_reveal_workflow() {
    let client = http_client();
    let author = Keys::generate();
    let other = Keys::generate();
    let channel = create_channel(&client, &author, "open").await;
    add_member(&client, &author, &channel, &other).await;
    let define = |id: &str| {
        sign(
            &author,
            30620,
            "name: existence-oracle\ntrigger:\n  on: message_posted\nsteps:\n  - id: pause\n    action: delay\n    duration: 1s\n",
            vec![tag(&["d", id]), tag(&["h", &channel])],
        )
    };
    let live = uuid::Uuid::new_v4().to_string();
    submit_ok(&client, &author, &define(&live)).await;
    let deleted = uuid::Uuid::new_v4().to_string();
    submit_ok(&client, &author, &define(&deleted)).await;
    let coordinate = format!("30620:{}:{deleted}", author.public_key().to_hex());
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags(vec![tag(&["a", &coordinate])])
        .sign_with_keys(&author)
        .unwrap();
    submit_ok(&client, &author, &deletion).await;

    let trigger = |keys: &Keys, id: &str| sign(keys, 46020, "{}", vec![tag(&["d", id])]);
    assert_uniform(
        &client,
        &other,
        "invalid: workflow not found",
        vec![
            (
                "missing",
                trigger(&other, &uuid::Uuid::new_v4().to_string()),
            ),
            ("soft-deleted", trigger(&other, &deleted)),
            ("someone else's", trigger(&other, &live)),
        ],
    )
    .await;

    // Positive control: the owner triggers their workflow.
    submit_ok(&client, &author, &trigger(&author, &live)).await;
}

/// GET `path` as `reader` and return the HTTP status and parsed body.
async fn read_as(client: &Client, reader: &Keys, path: &str) -> (u16, Value) {
    let resp = client
        .get(format!("{}{path}", relay_http_url()))
        .header("X-Pubkey", reader.public_key().to_hex())
        .send()
        .await
        .expect("read workflow");
    let status = resp.status().as_u16();
    (status, resp.json().await.expect("parse response"))
}

/// Reading the runs or approvals of a workflow in a channel the caller cannot
/// read answers exactly like reading a workflow that does not exist.
#[tokio::test]
#[ignore]
async fn workflow_read_does_not_reveal_workflow() {
    let client = http_client();
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let channel = create_channel(&client, &owner, "private").await;
    let live = uuid::Uuid::new_v4().to_string();
    let secret = save_webhook_workflow(&client, &owner, &channel, &live).await;
    let run = start_webhook_run(&client, &live, &secret).await;
    let missing = uuid::Uuid::new_v4().to_string();
    let paths = |id: &str| {
        [
            format!("/workflows/{id}/runs"),
            format!("/workflows/{id}/runs/{run}/approvals"),
        ]
    };

    for (missing_path, live_path) in paths(&missing).iter().zip(paths(&live).iter()) {
        let expected = read_as(&client, &outsider, missing_path).await;
        assert_eq!(expected.0, 404, "{missing_path}: {:?}", expected.1);
        assert_eq!(
            read_as(&client, &outsider, live_path).await,
            expected,
            "{live_path} must answer like a missing workflow"
        );
    }

    // Positive control: the owner reads their workflow's run and its approvals.
    let [runs_path, approvals_path] = paths(&live);
    let (status, body) = read_as(&client, &owner, &runs_path).await;
    assert_eq!(status, 200, "owner runs read failed: {body}");
    assert!(
        body["runs"]
            .as_array()
            .is_some_and(|runs| runs.iter().any(|r| r["id"] == run.as_str())),
        "owner runs must include {run}: {body}"
    );
    let (status, body) = read_as(&client, &owner, &approvals_path).await;
    assert_eq!(status, 200, "owner approvals read failed: {body}");
    assert!(body["approvals"].is_array());
}

/// Save a webhook-triggered workflow `id` in `channel` and return its secret.
async fn save_webhook_workflow(client: &Client, owner: &Keys, channel: &str, id: &str) -> String {
    let definition = sign(
        owner,
        30620,
        "name: existence-oracle\ntrigger:\n  on: webhook\nsteps:\n  - id: pause\n    action: delay\n    duration: 1s\n",
        vec![tag(&["d", id]), tag(&["h", channel])],
    );
    let (accepted, message) = submit(client, owner, &definition).await;
    assert!(accepted, "webhook workflow rejected: {message}");
    let response: Value = serde_json::from_str(
        message
            .strip_prefix("response:")
            .expect("workflow save response"),
    )
    .expect("parse workflow save response");
    response["webhook_secret"]
        .as_str()
        .expect("webhook secret")
        .to_string()
}

/// Call webhook `id` with the right secret, assert it starts a run exactly as
/// documented, and return the run id.
async fn start_webhook_run(client: &Client, id: &str, secret: &str) -> String {
    let (status, body) = call_webhook(client, id, Some(secret)).await;
    assert_eq!(status, 202, "valid webhook call failed: {body}");
    body["run_id"]
        .as_str()
        .and_then(|run| uuid::Uuid::parse_str(run).ok())
        .unwrap_or_else(|| panic!("webhook response lacks a run_id: {body}"))
        .to_string()
}

/// POST the public webhook endpoint for `id` with an optional secret header and
/// return the HTTP status and parsed body.
async fn call_webhook(client: &Client, id: &str, secret: Option<&str>) -> (u16, Value) {
    let mut request = client.post(format!("{}/hooks/{id}", relay_http_url()));
    if let Some(secret) = secret {
        request = request.header("x-webhook-secret", secret);
    }
    let resp = request.send().await.expect("call webhook");
    let status = resp.status().as_u16();
    (status, resp.json().await.expect("parse response"))
}

/// Until a caller proves the webhook secret, the unauthenticated webhook
/// endpoint answers every workflow exactly like a missing one, so it never
/// reveals that a workflow exists or what its trigger is.
#[tokio::test]
#[ignore]
async fn webhook_does_not_reveal_workflow() {
    let client = http_client();
    let owner = Keys::generate();
    let channel = create_channel(&client, &owner, "open").await;
    let webhook = uuid::Uuid::new_v4().to_string();
    let secret = save_webhook_workflow(&client, &owner, &channel, &webhook).await;
    let other_trigger = uuid::Uuid::new_v4().to_string();
    let message_posted = sign(
        &owner,
        30620,
        "name: existence-oracle\ntrigger:\n  on: message_posted\nsteps:\n  - id: pause\n    action: delay\n    duration: 1s\n",
        vec![tag(&["d", &other_trigger]), tag(&["h", &channel])],
    );
    submit_ok(&client, &owner, &message_posted).await;

    let missing = uuid::Uuid::new_v4().to_string();
    let expected = call_webhook(&client, &missing, Some(&secret)).await;
    assert_eq!(expected.0, 404, "missing workflow: {:?}", expected.1);
    for (case, id, provided) in [
        (
            "non-webhook workflow",
            &other_trigger,
            Some(secret.as_str()),
        ),
        ("absent secret", &webhook, None),
        ("wrong secret", &webhook, Some("wrong-secret")),
    ] {
        assert_eq!(
            call_webhook(&client, id, provided).await,
            expected,
            "{case} must answer like a missing workflow"
        );
    }

    // Positive control: the right secret starts a run.
    start_webhook_run(&client, &webhook, &secret).await;
}
