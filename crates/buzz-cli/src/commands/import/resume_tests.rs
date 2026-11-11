//! Resume behaviour of `buzz import slack`, driven through `cmd_import_slack`
//! against an in-process relay.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use nostr::Keys;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::state::{journal_path, ImportState};
use super::{cmd_import_slack, ImportSlackParams};
use crate::client::BuzzClient;
use crate::credentials::Credentials;

/// A relay stand-in: it stores every event it is sent and answers id lookups.
/// It can be told to store one message while reporting a failure, which is
/// what an importer observes when the relay's answer is lost.
#[derive(Clone, Default)]
struct FakeRelay {
    stored: Arc<Mutex<Vec<Value>>>,
    lose_answer_for: Arc<Mutex<Option<String>>>,
}

impl FakeRelay {
    /// Contents of the stored stream messages, in arrival order.
    fn messages(&self) -> Vec<String> {
        self.stored
            .lock()
            .expect("relay store")
            .iter()
            .filter(|event| event["kind"] == 9)
            .filter_map(|event| event["content"].as_str().map(str::to_string))
            .collect()
    }
}

async fn accept_event(State(relay): State<FakeRelay>, Json(event): Json<Value>) -> Json<Value> {
    let id = event["id"].as_str().unwrap_or_default().to_string();
    let content = event["content"].as_str().unwrap_or_default().to_string();
    let mut stored = relay.stored.lock().expect("relay store");
    if stored.iter().any(|held| held["id"] == event["id"]) {
        return Json(json!({"event_id": id, "accepted": false, "message": "duplicate: have it"}));
    }
    stored.push(event);

    let mut lose = relay.lose_answer_for.lock().expect("fault switch");
    if lose
        .as_deref()
        .is_some_and(|marker| content.contains(marker))
    {
        *lose = None;
        return Json(
            json!({"event_id": id, "accepted": false, "message": "error: storage failed"}),
        );
    }
    Json(json!({"event_id": id, "accepted": true, "message": ""}))
}

async fn query_events(
    State(relay): State<FakeRelay>,
    Json(filters): Json<Vec<Value>>,
) -> Json<Vec<Value>> {
    let wanted: HashSet<&str> = filters
        .iter()
        .filter_map(|filter| filter["ids"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let stored = relay.stored.lock().expect("relay store");
    Json(
        stored
            .iter()
            .filter(|event| event["id"].as_str().is_some_and(|id| wanted.contains(id)))
            .cloned()
            .collect(),
    )
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

/// A one-channel export holding `texts` as consecutive plain messages.
fn write_export(texts: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("buzz-import-resume-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("general")).expect("mkdir");
    std::fs::write(dir.join("users.json"), r#"[{"id":"U1","name":"ada"}]"#).expect("users");
    std::fs::write(
        dir.join("channels.json"),
        r#"[{"id":"C1","name":"general"}]"#,
    )
    .expect("channels");
    let day: Vec<Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            json!({"type": "message", "user": "U1", "text": text, "ts": format!("{}.000100", 100 + index)})
        })
        .collect();
    std::fs::write(
        dir.join("general/2024-01-01.json"),
        serde_json::to_string(&day).expect("serialize day"),
    )
    .expect("day file");
    dir
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

fn ledger(dir: &Path) -> ImportState {
    ImportState::load_for_workspace(&dir.join("buzz-import-state.json"), "T1").expect("ledger")
}

#[tokio::test]
async fn a_message_whose_answer_was_lost_is_adopted_instead_of_sent_again() {
    let (relay, client) = spawn_relay().await;
    let dir = write_export(&["first", "second"]);
    *relay.lose_answer_for.lock().expect("fault switch") = Some("second".into());

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
    let messages = relay.messages();
    assert_eq!(messages.len(), 2, "no message may be published twice");
    assert!(messages[0].ends_with("first") && messages[1].ends_with("second"));

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_pending_message_the_relay_never_stored_is_signed_again() {
    let (relay, client) = spawn_relay().await;
    let dir = write_export(&["only"]);
    // An earlier run died after journaling its intent and before the relay
    // stored anything.
    let state_path = dir.join("buzz-import-state.json");
    std::fs::write(
        journal_path(&state_path),
        format!(
            "{}\n",
            json!({"intent": {"key": "C1:100.000100", "id": "ab".repeat(32)}})
        ),
    )
    .expect("journal");

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("import completes");
    let finished = ledger(&dir);
    assert!(finished.pending.is_empty());
    assert_ne!(finished.messages["C1:100.000100"], "ab".repeat(32));
    assert_eq!(relay.messages().len(), 1);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn imported_messages_get_strictly_increasing_timestamps_in_slack_order() {
    let (relay, client) = spawn_relay().await;
    let dir = write_export(&["one", "two", "three"]);

    cmd_import_slack(&client, params(&dir))
        .await
        .expect("import completes");
    let stored = relay.stored.lock().expect("relay store");
    let stamped: Vec<(u64, &str)> = stored
        .iter()
        .filter(|event| event["kind"] == 9)
        .map(|event| {
            (
                event["created_at"].as_u64().expect("created_at"),
                event["content"].as_str().expect("content"),
            )
        })
        .collect();
    assert_eq!(stamped.len(), 3);
    assert!(stamped.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(stamped[0].1.ends_with("one") && stamped[2].1.ends_with("three"));

    std::fs::remove_dir_all(&dir).ok();
}
