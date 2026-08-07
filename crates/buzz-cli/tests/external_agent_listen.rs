use std::net::TcpListener as StdTcpListener;
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Json, Response};
use axum::routing::{get, post};
use axum::Router;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const TEST_PRIVATE_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TEST_PUBLIC_KEY: &str = "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";
const SENDER_PRIVATE_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const CHANNEL_A: &str = "11111111-1111-1111-1111-111111111111";
const CHANNEL_B: &str = "22222222-2222-2222-2222-222222222222";

#[derive(Clone)]
struct FakeRelayState {
    report_tx: Arc<Mutex<Option<oneshot::Sender<Vec<Value>>>>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[cfg(unix)]
#[derive(Clone)]
struct SignalRelayState {
    ready_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    close_tx: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

async fn fake_relay_upgrade(State(state): State<FakeRelayState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| fake_relay_session(socket, state))
}

async fn fake_channel_query() -> Json<Value> {
    Json(json!([
        {
            "id": "3000000000000000000000000000000000000000000000000000000000000001",
            "pubkey": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "kind": 39000,
            "content": "",
            "created_at": 1785100000,
            "tags": [["d", CHANNEL_B]]
        },
        {
            "id": "3000000000000000000000000000000000000000000000000000000000000002",
            "pubkey": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "kind": 39000,
            "content": "",
            "created_at": 1785100001,
            "tags": [["d", CHANNEL_A]]
        }
    ]))
}

#[cfg(unix)]
async fn signal_relay_upgrade(
    State(state): State<SignalRelayState>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| signal_relay_session(socket, state))
}

async fn recv_json(socket: &mut WebSocket) -> Value {
    match socket
        .recv()
        .await
        .expect("client should send a WebSocket frame")
        .expect("client WebSocket frame should be valid")
    {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("valid client JSON"),
        other => panic!("expected text frame, got {other:?}"),
    }
}

fn signed_message(channel: &str, index: usize) -> Value {
    let keys = Keys::parse(SENDER_PRIVATE_KEY).expect("valid sender key");
    let event = EventBuilder::new(Kind::Custom(40002), format!("fixture message {index}"))
        .tags([
            Tag::parse(["h", channel]).expect("valid h tag"),
            Tag::parse(["p", TEST_PUBLIC_KEY]).expect("valid p tag"),
        ])
        .sign_with_keys(&keys)
        .expect("signed fixture event");
    serde_json::to_value(event).expect("event JSON")
}

async fn fake_relay_session(mut socket: WebSocket, state: FakeRelayState) {
    socket
        .send(Message::Text(
            json!(["AUTH", "external-agent-test-challenge"])
                .to_string()
                .into(),
        ))
        .await
        .expect("send AUTH challenge");

    let auth = recv_json(&mut socket).await;
    assert_eq!(auth[0], "AUTH");
    let auth_event_id = auth[1]["id"].as_str().expect("AUTH event id");
    socket
        .send(Message::Text(
            json!(["OK", auth_event_id, true, ""]).to_string().into(),
        ))
        .await
        .expect("accept AUTH event");

    let mut requests = Vec::new();
    for index in 0..2 {
        let request = recv_json(&mut socket).await;
        let sub_id = request[1].as_str().expect("subscription id").to_string();
        let channel = request[2]["#h"][0]
            .as_str()
            .expect("single channel filter")
            .to_string();
        requests.push(request);

        socket
            .send(Message::Text(
                json!(["EVENT", sub_id, signed_message(&channel, index)])
                    .to_string()
                    .into(),
            ))
            .await
            .expect("send event");
        socket
            .send(Message::Text(json!(["EOSE", sub_id]).to_string().into()))
            .await
            .expect("send EOSE");
    }

    state
        .report_tx
        .lock()
        .expect("report lock")
        .take()
        .expect("report sender")
        .send(requests)
        .ok();
    let _ = socket.send(Message::Close(None)).await;
    state
        .shutdown_tx
        .lock()
        .expect("shutdown lock")
        .take()
        .expect("shutdown sender")
        .send(())
        .ok();
}

#[cfg(unix)]
async fn signal_relay_session(mut socket: WebSocket, state: SignalRelayState) {
    socket
        .send(Message::Text(
            json!(["AUTH", "external-agent-signal-test"])
                .to_string()
                .into(),
        ))
        .await
        .expect("send AUTH challenge");
    let auth = recv_json(&mut socket).await;
    let auth_event_id = auth[1]["id"].as_str().expect("AUTH event id");
    socket
        .send(Message::Text(
            json!(["OK", auth_event_id, true, ""]).to_string().into(),
        ))
        .await
        .expect("accept AUTH event");

    let request = recv_json(&mut socket).await;
    let sub_id = request[1].as_str().expect("subscription id").to_string();
    socket
        .send(Message::Text(json!(["EOSE", sub_id]).to_string().into()))
        .await
        .expect("send EOSE");
    state
        .ready_tx
        .lock()
        .expect("ready lock")
        .take()
        .expect("ready sender")
        .send(())
        .ok();

    let mut received_close = false;
    while let Some(frame) = socket.recv().await {
        match frame.expect("valid client frame") {
            Message::Text(text) => {
                let message: Value =
                    serde_json::from_str(text.as_str()).expect("valid client JSON");
                if message[0] == "CLOSE" && message[1] == sub_id {
                    received_close = true;
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    state
        .close_tx
        .lock()
        .expect("close report lock")
        .take()
        .expect("close report sender")
        .send(received_close)
        .ok();
    state
        .shutdown_tx
        .lock()
        .expect("shutdown lock")
        .take()
        .expect("shutdown sender")
        .send(())
        .ok();
}

fn listen_command(relay_url: &str, channels: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
    command.arg("listen");
    for channel in channels {
        command.args(["--channel", channel]);
    }
    command
        .args(["--mentions-of-me", "--envelope", "v1", "--no-reconnect"])
        .env("BUZZ_PRIVATE_KEY", TEST_PRIVATE_KEY)
        .env("BUZZ_RELAY_URL", relay_url)
        .env_remove("BUZZ_AUTH_TAG");
    command
}

async fn start_fake_relay() -> (
    std::net::SocketAddr,
    oneshot::Receiver<Vec<Value>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake relay");
    let address = listener.local_addr().expect("fake relay address");
    let (report_tx, report_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = FakeRelayState {
        report_tx: Arc::new(Mutex::new(Some(report_tx))),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
    };
    let app = Router::new()
        .route("/", get(fake_relay_upgrade))
        .route("/query", post(fake_channel_query))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("fake relay server");
    });
    (address, report_rx, server)
}

#[cfg(unix)]
async fn start_signal_relay() -> (
    std::net::SocketAddr,
    oneshot::Receiver<()>,
    oneshot::Receiver<bool>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind signal relay");
    let address = listener.local_addr().expect("signal relay address");
    let (ready_tx, ready_rx) = oneshot::channel();
    let (close_tx, close_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = SignalRelayState {
        ready_tx: Arc::new(Mutex::new(Some(ready_tx))),
        close_tx: Arc::new(Mutex::new(Some(close_tx))),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
    };
    let app = Router::new()
        .route("/", get(signal_relay_upgrade))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("signal relay server");
    });
    (address, ready_rx, close_rx, server)
}

fn assert_scoped_requests(requests: &[Value]) {
    assert_eq!(requests.len(), 2);
    let subscription_ids: Vec<&str> = requests
        .iter()
        .map(|request| request[1].as_str().expect("subscription id"))
        .collect();
    assert_ne!(subscription_ids[0], subscription_ids[1]);
    let mut channels: Vec<&str> = requests
        .iter()
        .map(|request| {
            assert_eq!(request.as_array().expect("REQ array").len(), 3);
            assert_eq!(request[0], "REQ");
            assert_eq!(request[2]["#h"].as_array().expect("#h array").len(), 1);
            assert_eq!(request[2]["#p"], json!([TEST_PUBLIC_KEY]));
            request[2]["#h"][0].as_str().expect("channel id")
        })
        .collect();
    channels.sort_unstable();
    assert_eq!(channels, vec![CHANNEL_A, CHANNEL_B]);
}

fn assert_event_stream(output: &std::process::Output) {
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout UTF-8");
    let records: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout NDJSON"))
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|record| record["type"] == "event")
            .count(),
        2
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["state"] == "eose")
            .count(),
        1
    );
    assert_eq!(
        records.first().expect("connected record")["state"],
        "connected"
    );
    assert_eq!(records.last().expect("fatal record")["state"], "fatal");
}

#[cfg(unix)]
async fn assert_graceful_signal(signal: &str) {
    let (address, ready_rx, close_rx, server) = start_signal_relay().await;
    let mut command = listen_command(&format!("http://{address}"), &[CHANNEL_A]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command.spawn().expect("spawn buzz listen");
    let pid = child.id().to_string();

    tokio::time::timeout(Duration::from_secs(10), ready_rx)
        .await
        .expect("listener should reach EOSE")
        .expect("ready report");
    let signal_status = Command::new("kill")
        .args([signal, &pid])
        .status()
        .expect("send process signal");
    assert!(signal_status.success());

    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            child.wait_with_output().expect("listen process output")
        }),
    )
    .await
    .expect("listen should stop after signal")
    .expect("listen wait task");
    assert!(
        close_rx.await.expect("CLOSE report"),
        "listener should send Nostr CLOSE before disconnect"
    );
    server.await.expect("signal relay task");

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "graceful signal should not emit stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records: Vec<Value> = String::from_utf8(output.stdout)
        .expect("stdout UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout NDJSON"))
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|record| record["state"] == "connected")
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["state"] == "eose")
            .count(),
        1
    );
    assert!(
        records.iter().all(|record| record["state"] != "fatal"),
        "graceful signal must not emit fatal"
    );
}

#[test]
fn listen_network_failure_is_machine_classifiable() {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind unused port");
    let address = listener.local_addr().expect("unused port address");
    drop(listener);

    let output = listen_command(&format!("http://{address}"), &[CHANNEL_A, CHANNEL_B])
        .output()
        .expect("buzz listen should start");

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    let records: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout NDJSON"))
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["type"], "lifecycle");
    assert_eq!(records[0]["state"], "fatal");

    let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
    let error: Value = serde_json::from_str(stderr.trim()).expect("stderr error JSON");
    assert_eq!(error["error"], "network_error");
    assert_eq!(error["retryable"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_channel_listen_uses_scoped_subscriptions_and_one_eose() {
    let (address, report_rx, server) = start_fake_relay().await;

    let relay_url = format!("http://{address}");
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            listen_command(&relay_url, &[CHANNEL_A, CHANNEL_B])
                .output()
                .expect("buzz listen should run")
        }),
    )
    .await
    .expect("buzz listen should exit")
    .expect("listen process task");
    let requests = report_rx.await.expect("fake relay request report");
    server.await.expect("fake relay task");

    assert_scoped_requests(&requests);
    assert_event_stream(&output);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mentions_only_discovers_visible_channels_before_listening() {
    let (address, report_rx, server) = start_fake_relay().await;

    let relay_url = format!("http://{address}");
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            listen_command(&relay_url, &[])
                .output()
                .expect("buzz listen should run")
        }),
    )
    .await
    .expect("buzz listen should exit")
    .expect("listen process task");
    let requests = report_rx.await.expect("fake relay request report");
    server.await.expect("fake relay task");

    assert_scoped_requests(&requests);
    assert_event_stream(&output);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_and_sigterm_close_subscriptions_cleanly() {
    assert_graceful_signal("-INT").await;
    assert_graceful_signal("-TERM").await;
}
