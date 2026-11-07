use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{any, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
use tempfile::NamedTempFile;
use tokio::net::TcpListener;

const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const EMPTY_MESSAGE_ERROR: &str =
    "message cannot be empty; provide content via --content or stdin, or attach at least one --file";

#[derive(Clone, Default)]
struct RelayDouble {
    request_count: Arc<AtomicUsize>,
    upload_count: Arc<AtomicUsize>,
    submitted_events: Arc<Mutex<Vec<Value>>>,
}

impl RelayDouble {
    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }

    fn upload_count(&self) -> usize {
        self.upload_count.load(Ordering::SeqCst)
    }

    fn submitted_events(&self) -> Vec<Value> {
        self.submitted_events.lock().unwrap().clone()
    }
}

async fn upload(State(state): State<RelayDouble>, _body: Bytes) -> Json<Value> {
    state.request_count.fetch_add(1, Ordering::SeqCst);
    state.upload_count.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        "url": "https://relay.test/media/fixture.png",
        "sha256": "fixture-sha256",
        "size": 8,
        "type": "image/png",
        "uploaded": 0
    }))
}

async fn submit_event(State(state): State<RelayDouble>, body: Bytes) -> Json<Value> {
    state.request_count.fetch_add(1, Ordering::SeqCst);
    let event = serde_json::from_slice(&body).unwrap();
    state.submitted_events.lock().unwrap().push(event);
    Json(json!({
        "event_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "accepted": true,
        "message": ""
    }))
}

async fn unexpected_request(State(state): State<RelayDouble>) -> StatusCode {
    state.request_count.fetch_add(1, Ordering::SeqCst);
    StatusCode::NOT_FOUND
}

async fn spawn_relay_double() -> (String, RelayDouble) {
    let state = RelayDouble::default();
    let app = Router::new()
        .route("/upload", put(upload))
        .route("/events", post(submit_event))
        .fallback(any(unexpected_request))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), state)
}

fn buzz_command(relay_url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command.args([
        "--relay",
        relay_url,
        "--private-key",
        PRIVATE_KEY,
        "messages",
        "send",
        "--channel",
        CHANNEL_ID,
    ]);
    command
}

fn assert_empty_message_rejected(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "unexpected status; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"], "user_error");
    assert_eq!(error["message"], EMPTY_MESSAGE_ERROR);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_content_without_file_is_rejected_before_external_effects() {
    let (relay_url, relay) = spawn_relay_double().await;
    let output = buzz_command(&relay_url)
        .args(["--content", ""])
        .output()
        .unwrap();

    assert_empty_message_rejected(&output);
    assert_eq!(relay.request_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn immediate_stdin_eof_without_file_is_rejected_before_external_effects() {
    let (relay_url, relay) = spawn_relay_double().await;
    let output = buzz_command(&relay_url)
        .args(["--content", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_empty_message_rejected(&output);
    assert_eq!(relay.request_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_only_content_without_file_is_rejected_before_external_effects() {
    for content in ["   ", "\t\t", "\n\n", "\r\n\r\n"] {
        let (relay_url, relay) = spawn_relay_double().await;
        let output = buzz_command(&relay_url)
            .args(["--content", content])
            .output()
            .unwrap();

        assert_empty_message_rejected(&output);
        assert_eq!(relay.request_count(), 0, "content: {content:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_content_with_file_reaches_upload_double_and_message_building() {
    use std::io::Write;

    let (relay_url, relay) = spawn_relay_double().await;
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();

    let output = buzz_command(&relay_url)
        .args(["--content", "", "--file", file.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(relay.upload_count(), 1);
    let events = relay.submitted_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]["content"],
        "\n![image](https://relay.test/media/fixture.png)"
    );
    assert!(events[0]["tags"].as_array().unwrap().iter().any(|tag| {
        tag.as_array().is_some_and(|parts| {
            parts.first() == Some(&json!("imeta"))
                && parts
                    .iter()
                    .any(|part| part == "url https://relay.test/media/fixture.png")
        })
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn text_without_file_keeps_the_existing_send_path() {
    let (relay_url, relay) = spawn_relay_double().await;
    let output = buzz_command(&relay_url)
        .args(["--content", "hola"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(relay.upload_count(), 0);
    let events = relay.submitted_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["content"], "hola");
}
