use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const CHANNEL_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
const TARGET_EVENT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const EDITED_CONTENT: &str = "first line\n\nsecond line\n";

#[derive(Clone)]
struct RelayState {
    submitted_event: Arc<Mutex<Option<Value>>>,
}

async fn query_target_event() -> Json<Value> {
    Json(json!([{
        "id": TARGET_EVENT_ID,
        "tags": [["h", CHANNEL_ID]],
    }]))
}

async fn submit_event(State(state): State<RelayState>, Json(event): Json<Value>) -> Json<Value> {
    *state.submitted_event.lock().expect("lock submitted event") = Some(event);
    Json(json!({
        "event_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "accepted": true,
        "message": "",
    }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_edit_reads_content_from_stdin() {
    let submitted_event = Arc::new(Mutex::new(None));
    let state = RelayState {
        submitted_event: submitted_event.clone(),
    };
    let app = Router::new()
        .route("/query", post(query_target_event))
        .route("/events", post(submit_event))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock relay");
    let relay_url = format!(
        "http://{}",
        listener.local_addr().expect("read mock relay address")
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock relay");
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(["--relay", &relay_url, "--private-key", PRIVATE_KEY])
        .args([
            "messages",
            "edit",
            "--event",
            TARGET_EVENT_ID,
            "--content",
            "-",
        ])
        .env_remove("BUZZ_AUTH_TAG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz CLI");
    child
        .stdin
        .take()
        .expect("open child stdin")
        .write_all(EDITED_CONTENT.as_bytes())
        .expect("write edit content");
    let output = child.wait_with_output().expect("wait for buzz CLI");
    server.abort();

    assert!(
        output.status.success(),
        "buzz CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let event = submitted_event
        .lock()
        .expect("lock submitted event")
        .take()
        .expect("CLI should submit an edit event");
    assert_eq!(event["kind"], 40003);
    assert_eq!(event["content"], EDITED_CONTENT);
    assert!(event["tags"]
        .as_array()
        .expect("edit tags")
        .iter()
        .any(|tag| tag == &json!(["e", TARGET_EVENT_ID])));
}
