use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::{post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

const PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const CHANNEL_ID: &str = "11111111-1111-1111-1111-111111111111";
const MEMBER_PUBKEY: &str = "35c18ae273fccfaf80d629e20e7f8721b90499379addff533054acc2504c12b4";

#[derive(Clone, Default)]
struct RelayState {
    events: Arc<Mutex<Vec<Value>>>,
    query_response: Arc<Mutex<Value>>,
    upload_url: Arc<Mutex<String>>,
}

async fn accept_event(State(state): State<RelayState>, Json(event): Json<Value>) -> Json<Value> {
    let event_id = event["id"].clone();
    state.events.lock().expect("event capture lock").push(event);
    Json(json!({
        "event_id": event_id,
        "accepted": true,
        "message": "stored"
    }))
}

async fn answer_query(State(state): State<RelayState>) -> Json<Value> {
    Json(
        state
            .query_response
            .lock()
            .expect("query response lock")
            .clone(),
    )
}

async fn accept_upload(State(state): State<RelayState>) -> Json<Value> {
    let url = state.upload_url.lock().expect("upload URL lock").clone();
    Json(json!({
        "url": url,
        "sha256": "abc123",
        "size": 12,
        "type": "image/png",
        "uploaded": 0
    }))
}

async fn accepting_relay_with_query(query_response: Value) -> (String, RelayState) {
    let state = RelayState {
        query_response: Arc::new(Mutex::new(query_response)),
        ..RelayState::default()
    };
    let app = Router::new()
        .route("/events", post(accept_event))
        .route("/query", post(answer_query))
        .route("/upload", put(accept_upload))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let address = listener.local_addr().expect("listener address");
    *state.upload_url.lock().expect("upload URL lock") =
        format!("http://{address}/media/image.png");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    (format!("http://{address}"), state)
}

async fn accepting_relay() -> (String, RelayState) {
    accepting_relay_with_query(json!([])).await
}

fn buzz_command(relay: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command.args(["--relay", relay, "--private-key", PRIVATE_KEY]);
    command
}

fn run_buzz(relay: &str, args: &[&str]) -> Output {
    buzz_command(relay).args(args).output().expect("run buzz")
}

fn run_buzz_with_stdin(relay: &str, args: &[&str], stdin: &str) -> Output {
    let mut child = buzz_command(relay)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for buzz")
}

fn only_event(state: &RelayState) -> Value {
    let events = state.events.lock().expect("event capture lock");
    assert_eq!(events.len(), 1, "expected exactly one submitted event");
    events[0].clone()
}

fn has_tag(event: &Value, expected: &[&str]) -> bool {
    event["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts
                    .iter()
                    .map(|part| part.as_str())
                    .eq(expected.iter().copied().map(Some))
            })
        })
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn argument_content_and_success_projection_are_stable() {
    let (relay, state) = accepting_relay().await;
    let output = run_buzz(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "hello from argv",
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event = only_event(&state);
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("JSON stdout");
    assert_eq!(stdout["event_id"], event["id"]);
    assert_eq!(stdout["accepted"], true);
    assert_eq!(stdout["message"], "stored");
    assert_eq!(stdout["mention_pubkeys"], json!([]));
    assert_eq!(event["kind"], 9);
    assert_eq!(event["content"], "hello from argv");
    assert!(has_tag(&event, &["h", CHANNEL_ID]));
}

#[tokio::test(flavor = "multi_thread")]
async fn dash_reads_message_content_from_stdin() {
    let (relay, state) = accepting_relay().await;
    let output = run_buzz_with_stdin(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "-",
        ],
        "content from stdin",
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(only_event(&state)["content"], "content from stdin");
}

#[tokio::test(flavor = "multi_thread")]
async fn forum_post_kind_is_relay_visible() {
    let (relay, state) = accepting_relay().await;
    let output = run_buzz(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "forum root",
            "--kind",
            "45001",
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event = only_event(&state);
    assert_eq!(event["kind"], 45001);
    assert_eq!(event["content"], "forum root");
    assert!(has_tag(&event, &["h", CHANNEL_ID]));
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_member_mention_is_visible_in_event_and_success_json() {
    let membership = json!([{
        "kind": 39002,
        "tags": [["d", CHANNEL_ID], ["p", MEMBER_PUBKEY]]
    }]);
    let (relay, state) = accepting_relay_with_query(membership).await;
    let output = run_buzz(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "hello member",
            "--mention",
            MEMBER_PUBKEY,
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event = only_event(&state);
    assert!(
        has_tag(&event, &["p", MEMBER_PUBKEY]),
        "submitted event: {event}"
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("JSON stdout");
    assert_eq!(stdout["mention_pubkeys"], json!([MEMBER_PUBKEY]));
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_forum_comment_preserves_root_and_parent_markers() {
    let root_id = "a".repeat(64);
    let parent_id = "b".repeat(64);
    let parent = json!([{
        "id": parent_id,
        "kind": 45003,
        "tags": [["h", CHANNEL_ID], ["e", root_id, "", "root"]]
    }]);
    let (relay, state) = accepting_relay_with_query(parent).await;
    let output = run_buzz(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "nested comment",
            "--kind",
            "45003",
            "--reply-to",
            &parent_id,
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event = only_event(&state);
    assert_eq!(event["kind"], 45003);
    assert!(has_tag(&event, &["e", &root_id, "", "root"]));
    assert!(has_tag(&event, &["e", &parent_id, "", "reply"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachment_upload_contributes_content_and_imeta() {
    let (relay, state) = accepting_relay().await;
    let media_url = format!("{relay}/media/image.png");
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("image.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\nmock").expect("write PNG fixture");
    let path = path.to_string_lossy();
    let output = run_buzz(
        &relay,
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "with image",
            "--file",
            &path,
        ],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let event = only_event(&state);
    assert_eq!(
        event["content"],
        format!("with image\n![image]({media_url})")
    );
    assert!(has_tag(
        &event,
        &[
            "imeta",
            &format!("url {media_url}"),
            "m image/png",
            "x abc123",
            "size 12",
        ]
    ));
}

#[test]
fn unsupported_kind_keeps_user_error_and_exit_code() {
    let output = run_buzz(
        "http://127.0.0.1:9",
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "hello",
            "--kind",
            "42",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("JSON stderr");
    assert_eq!(stderr["error"], "user_error");
    assert_eq!(stderr["retryable"], false);
    assert!(stderr["message"]
        .as_str()
        .is_some_and(|message| message.contains("--kind 42 is not supported")));
}

#[test]
fn forum_comment_without_reply_keeps_user_error_and_exit_code() {
    let output = run_buzz(
        "http://127.0.0.1:9",
        &[
            "messages",
            "send",
            "--channel",
            CHANNEL_ID,
            "--content",
            "orphan comment",
            "--kind",
            "45003",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("JSON stderr");
    assert_eq!(stderr["error"], "user_error");
    assert!(stderr["message"]
        .as_str()
        .is_some_and(|message| message.contains("--reply-to is required")));
}
