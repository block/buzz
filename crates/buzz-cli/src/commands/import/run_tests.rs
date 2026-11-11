//! `buzz import slack` end to end: `cmd_import_slack` against an in-process
//! relay that enforces the rules the importer depends on.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use nostr::Keys;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::importer::channel_uuid;
use super::state::{journal_path, ImportState};
use super::{cmd_import_slack, ImportSlackParams};
use crate::client::BuzzClient;
use crate::credentials::Credentials;

/// What the stand-in relay holds, plus switches for the faults under test.
#[derive(Default)]
struct RelayState {
    /// Every stored event, in arrival order.
    events: Vec<Value>,
    /// Channel UUID → whether it is archived.
    channels: HashMap<String, bool>,
    /// Store the next message containing this text, but report a failure:
    /// what an importer observes when the relay's answer is lost.
    lose_answer_for: Option<String>,
    /// Refuse the next message containing this text without storing it.
    refuse_once: Option<String>,
}

#[derive(Clone, Default)]
struct FakeRelay(Arc<Mutex<RelayState>>);

impl FakeRelay {
    fn state(&self) -> MutexGuard<'_, RelayState> {
        self.0.lock().expect("relay state")
    }

    /// The stored stream messages, in arrival order.
    fn messages(&self) -> Vec<Value> {
        self.state()
            .events
            .iter()
            .filter(|event| event["kind"] == 9)
            .cloned()
            .collect()
    }

    /// Text of each stored message after its attribution header.
    fn texts(&self) -> Vec<String> {
        self.messages()
            .iter()
            .filter_map(|event| event["content"].as_str())
            .map(|content| content.split_once("): ").map_or(content, |(_, text)| text))
            .map(str::to_string)
            .collect()
    }

    /// How many stored kind:9002 events carry `["archived", <value>]`.
    fn archive_requests(&self, value: &str) -> usize {
        self.state()
            .events
            .iter()
            .filter(|event| event["kind"] == 9002 && tag_value(event, "archived") == Some(value))
            .count()
    }
}

fn tag_value<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|tag| tag[0] == name)
        .and_then(|tag| tag[1].as_str())
}

type Answer = (StatusCode, Json<Value>);

fn accepted(id: &str) -> Answer {
    (
        StatusCode::OK,
        Json(json!({"event_id": id, "accepted": true, "message": ""})),
    )
}

fn not_accepted(id: &str, message: &str) -> Answer {
    (
        StatusCode::OK,
        Json(json!({"event_id": id, "accepted": false, "message": message})),
    )
}

fn invalid(message: &str) -> Answer {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message})))
}

async fn accept_event(State(relay): State<FakeRelay>, Json(event): Json<Value>) -> Answer {
    let id = event["id"].as_str().unwrap_or_default().to_string();
    let content = event["content"].as_str().unwrap_or_default().to_string();
    let channel = tag_value(&event, "h").unwrap_or_default().to_string();
    let mut relay = relay.state();
    if relay.events.iter().any(|held| held["id"] == event["id"]) {
        return not_accepted(&id, "duplicate: have it");
    }

    match event["kind"].as_u64() {
        Some(9007) => {
            if relay.channels.contains_key(&channel) {
                return not_accepted(&id, "duplicate: channel already exists");
            }
            relay.channels.insert(channel, false);
        }
        Some(9002) => match (tag_value(&event, "archived"), relay.channels.get(&channel)) {
            (Some("true"), Some(true)) => return invalid("invalid: channel is already archived"),
            (Some("false"), Some(false)) => return invalid("invalid: channel is not archived"),
            (Some(wanted), Some(_)) => {
                relay.channels.insert(channel, wanted == "true");
            }
            _ => {}
        },
        Some(9) => {
            if relay.channels.get(&channel) == Some(&true) {
                return invalid("invalid: channel is archived");
            }
            if relay
                .refuse_once
                .as_deref()
                .is_some_and(|marker| content.contains(marker))
            {
                relay.refuse_once = None;
                return not_accepted(&id, "error: storage failed");
            }
            if relay
                .lose_answer_for
                .as_deref()
                .is_some_and(|marker| content.contains(marker))
            {
                relay.lose_answer_for = None;
                relay.events.push(event);
                return not_accepted(&id, "error: storage failed");
            }
        }
        _ => {}
    }
    relay.events.push(event);
    accepted(&id)
}

/// Answers the two reads the importer makes: events by id, and a channel's
/// current kind:39000 metadata by `#d`.
async fn query_events(
    State(relay): State<FakeRelay>,
    Json(filters): Json<Vec<Value>>,
) -> Json<Vec<Value>> {
    let relay = relay.state();
    let strings = |filter: &Value, key: &str| -> Vec<String> {
        filter[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect()
    };
    let mut found = Vec::new();
    for filter in &filters {
        let ids: HashSet<String> = strings(filter, "ids").into_iter().collect();
        found.extend(
            relay
                .events
                .iter()
                .filter(|event| event["id"].as_str().is_some_and(|id| ids.contains(id)))
                .cloned(),
        );
        for channel in strings(filter, "#d") {
            if let Some(archived) = relay.channels.get(&channel) {
                let mut tags = vec![json!(["d", channel])];
                if *archived {
                    tags.push(json!(["archived", "true"]));
                }
                found.push(json!({"kind": 39000, "tags": tags}));
            }
        }
    }
    Json(found)
}

async fn spawn_relay() -> (FakeRelay, BuzzClient) {
    let relay = FakeRelay::default();
    let app = Router::new()
        .route("/events", post(accept_event))
        .route("/query", post(query_events))
        .with_state(relay.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let client = BuzzClient::new(
        format!("http://{addr}"),
        Credentials::from_keys(Keys::generate()),
    )
    .expect("client");
    (relay, client)
}

/// One Slack message record; `ts` is `<100 + index>.000100`.
fn record(index: usize, text: &str) -> Value {
    json!({"type": "message", "user": "U1", "text": text, "ts": ts(index)})
}

fn ts(index: usize) -> String {
    format!("{}.000100", 100 + index)
}

/// Write an export whose one channel, `C1`, holds `records`.
fn write_export(channel: Value, records: &[Value]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("buzz-import-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("general")).expect("mkdir");
    std::fs::write(dir.join("users.json"), r#"[{"id":"U1","name":"ada"}]"#).expect("users");
    std::fs::write(dir.join("channels.json"), json!([channel]).to_string()).expect("channels");
    std::fs::write(
        dir.join("general/2024-01-01.json"),
        Value::from(records.to_vec()).to_string(),
    )
    .expect("day file");
    dir
}

fn general() -> Value {
    json!({"id": "C1", "name": "general"})
}

/// An export of plain messages in an ordinary channel.
fn write_messages(texts: &[&str]) -> PathBuf {
    let records: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| record(index, text))
        .collect();
    write_export(general(), &records)
}

fn params(dir: &Path) -> ImportSlackParams {
    ImportSlackParams {
        export_dirs: vec![dir.display().to_string()],
        team_id: "T1".into(),
        state: None,
        channel_map: None,
        channels: None,
        dry_run: false,
        skip_reactions: false,
        user_map: None,
    }
}

/// Parameters that adopt the existing Buzz channel `uuid` for `C1`.
fn adopting(dir: &Path, uuid: &str) -> ImportSlackParams {
    let map = dir.join("crosswalk.csv");
    std::fs::write(
        &map,
        format!("slack_channel_id,buzz_channel_id\nC1,{uuid}\n"),
    )
    .expect("map");
    ImportSlackParams {
        channel_map: Some(map.display().to_string()),
        ..params(dir)
    }
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("buzz-import-state.json")
}

fn ledger(dir: &Path) -> ImportState {
    ImportState::load_for_workspace(&state_path(dir), "T1").expect("ledger")
}

#[tokio::test]
async fn a_message_whose_answer_was_lost_is_adopted_instead_of_sent_again() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["first", "second"]);
    relay.state().lose_answer_for = Some("second".into());

    // The relay stores "second" but reports failure: the run must not claim
    // success, and must remember the exact event it handed over.
    let error = cmd_import_slack(&client, params(&dir))
        .await
        .expect_err("an unconfirmed write is not a finished import");
    assert!(error.to_string().contains("re-run to resume"), "{error}");
    let interrupted = ledger(&dir);
    assert_eq!(interrupted.messages.len(), 1);
    assert_eq!(interrupted.pending.len(), 1);

    // The next run finds that event on the relay and publishes nothing new.
    cmd_import_slack(&client, params(&dir))
        .await
        .expect("resume completes");
    let resumed = ledger(&dir);
    assert_eq!(resumed.messages.len(), 2);
    assert!(resumed.pending.is_empty());
    assert_eq!(
        relay.texts(),
        ["first", "second"],
        "no message may be published twice"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_pending_message_the_relay_never_stored_is_signed_again() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["only"]);
    // An earlier run died after journaling its intent and before the relay
    // stored anything.
    std::fs::write(
        journal_path(&state_path(&dir)),
        format!(
            "{}\n",
            json!({"intent": {"key": format!("C1:{}", ts(0)), "id": "ab".repeat(32)}})
        ),
    )
    .expect("journal");

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("import completes");
    let finished = ledger(&dir);
    assert!(finished.pending.is_empty());
    assert_ne!(finished.messages[&format!("C1:{}", ts(0))], "ab".repeat(32));
    assert_eq!(relay.texts(), ["only"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_unconfirmed_message_stops_the_channel_so_the_retry_keeps_slack_order() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["first", "second", "third"]);
    relay.state().refuse_once = Some("second".into());

    // Importing "third" around the gap would leave "second" to be stamped
    // later than it, and no later run could move it back.
    cmd_import_slack(&client, params(&dir))
        .await
        .expect_err("the run stops at the unconfirmed message");
    assert_eq!(relay.texts(), ["first"]);

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("resume completes");
    assert_eq!(relay.texts(), ["first", "second", "third"]);
    let stamps: Vec<u64> = relay
        .messages()
        .iter()
        .filter_map(|event| event["created_at"].as_u64())
        .collect();
    assert!(
        stamps.windows(2).all(|pair| pair[0] < pair[1]),
        "every message needs its own second, in Slack order: {stamps:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_channel_that_already_holds_the_derived_id_is_never_written_into() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["private plan"]);
    // Someone else created a channel under the id this import would derive.
    relay
        .state()
        .channels
        .insert(channel_uuid("T1", "C1").to_string(), false);

    let error = cmd_import_slack(&client, params(&dir))
        .await
        .expect_err("history must not go into a channel the ledger did not create");
    assert!(error.to_string().contains("--channel-map"), "{error}");
    assert!(relay.messages().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_reply_to_a_root_that_can_never_be_imported_does_not_fail_every_run() {
    let (relay, client) = spawn_relay().await;
    let oversize = "x".repeat(70 * 1024);
    let mut reply = record(1, "the reply");
    reply["thread_ts"] = json!(ts(0));
    let dir = write_export(general(), &[record(0, &oversize), reply]);

    // The root is over the relay's size limit. Its reply keeps its content as
    // a top-level message, and the run is complete, not perpetually partial.
    cmd_import_slack(&client, params(&dir))
        .await
        .expect("a permanently skipped record is not a failed run");
    assert_eq!(relay.texts(), ["the reply"]);
    assert_eq!(tag_value(&relay.messages()[0], "e"), None);
    assert!(ledger(&dir).skipped.contains(&format!("C1:{}", ts(0))));

    let events_before = relay.state().events.len();
    cmd_import_slack(&client, params(&dir))
        .await
        .expect("re-run has nothing left to do");
    assert_eq!(relay.state().events.len(), events_before);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_dm_that_cannot_be_opened_is_left_out_without_stopping_the_run() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["channel message"]);
    // A self-DM has one participant, so it can never become a Buzz DM.
    std::fs::write(
        dir.join("dms.json"),
        r#"[{"id":"D1","created":1700000000,"members":["U1"]}]"#,
    )
    .expect("dms");
    std::fs::create_dir_all(dir.join("D1")).expect("mkdir");
    std::fs::write(
        dir.join("D1/2024-01-01.json"),
        json!([record(5, "note to self")]).to_string(),
    )
    .expect("dm day");

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("the rest of the workspace is imported");
    assert_eq!(relay.texts(), ["channel message"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn adopting_an_active_channel_does_not_ask_the_relay_to_unarchive_it() {
    let (relay, client) = spawn_relay().await;
    let dir = write_messages(&["history"]);
    let existing = uuid::Uuid::new_v4().to_string();
    relay.state().channels.insert(existing.clone(), false);

    // The relay refuses to unarchive a channel that is not archived, so an
    // unconditional request would make every adoption of a live channel fail.
    cmd_import_slack(&client, adopting(&dir, &existing))
        .await
        .expect("adoption of an active channel succeeds");
    assert_eq!(relay.texts(), ["history"]);
    assert_eq!(
        tag_value(&relay.messages()[0], "h"),
        Some(existing.as_str())
    );
    assert_eq!(relay.archive_requests("false"), 0);
    assert_eq!(relay.archive_requests("true"), 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_adopted_archived_channel_is_reopened_for_the_import_and_archived_again() {
    let (relay, client) = spawn_relay().await;
    // Slack says the source channel is active; the Buzz channel is archived.
    let dir = write_messages(&["history"]);
    let existing = uuid::Uuid::new_v4().to_string();
    relay.state().channels.insert(existing.clone(), true);

    cmd_import_slack(&client, adopting(&dir, &existing))
        .await
        .expect("import completes");
    assert_eq!(relay.texts(), ["history"]);
    assert!(relay.state().channels[&existing]);
    assert!(!ledger(&dir).channels["C1"].restore_archived);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_archive_an_earlier_run_already_made_is_recognised_not_reported_as_failed() {
    let (relay, client) = spawn_relay().await;
    let dir = write_export(
        json!({"id": "C1", "name": "general", "is_archived": true}),
        &[record(0, "history")],
    );
    cmd_import_slack(&client, params(&dir))
        .await
        .expect("import completes");
    let uuid = channel_uuid("T1", "C1").to_string();
    assert!(relay.state().channels[&uuid]);

    // The earlier run archived the channel but died before recording it.
    let mut state = ledger(&dir);
    state.channels.get_mut("C1").expect("channel").archived_done = false;
    state.save(&state_path(&dir)).expect("save");

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("an already-archived channel is a finished one");
    assert!(ledger(&dir).channels["C1"].archived_done);
    assert!(relay.state().channels[&uuid]);

    std::fs::remove_dir_all(&dir).ok();
}
