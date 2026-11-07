//! Exercise immediate refill through the full harness, a local relay and real workers.
use super::*;
use axum::{
    extract::{ws::WebSocketUpgrade, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

struct RelayFixture {
    channels: Vec<Uuid>,
    pubkey: String,
    events: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<nostr::Event>>>,
}

async fn relay_socket(
    State(state): State<Arc<RelayFixture>>,
    upgrade: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> axum::response::Response {
    let Ok(upgrade) = upgrade else {
        return Json(json!({"pubkey": state.pubkey})).into_response();
    };
    upgrade.on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        let mut events = state.events.lock().await;
        socket.send(Message::Text(json!(["AUTH", "fixture"]).to_string().into())).await.unwrap();
        loop {
            tokio::select! {
                message = socket.recv() => {
                    let Some(Ok(Message::Text(text))) = message else { break };
                    let message: Value = serde_json::from_str(&text).unwrap();
                    let reply = match message[0].as_str() {
                        Some("AUTH" | "EVENT") => Some(json!(["OK", message[1]["id"], true, ""])),
                        Some("REQ") => Some(json!(["EOSE", message[1]])),
                        _ => None,
                    };
                    if let Some(reply) = reply {
                        if socket.send(Message::Text(reply.to_string().into())).await.is_err() { break }
                    }
                }
                batch = events.recv() => {
                    let Some(batch) = batch else { break };
                    for (channel, event) in state.channels.iter().zip(batch) {
                        let message = json!(["EVENT", format!("ch-{channel}"), event]);
                        if socket.send(Message::Text(message.to_string().into())).await.is_err() { return }
                    }
                }
            }
        }
    })
}

async fn relay_query(
    State(state): State<Arc<RelayFixture>>,
    Json(filters): Json<Value>,
) -> Json<Value> {
    let events: Vec<Value> = match filters[0]["kinds"][0].as_u64() {
        Some(39002) => state
            .channels
            .iter()
            .map(|id| json!({"tags": [["d", id]]}))
            .collect(),
        Some(39000) => state
            .channels
            .iter()
            .map(|id| json!({"tags": [["d", id], ["name", "refill"], ["t", "stream"]]}))
            .collect(),
        _ => vec![],
    };
    Json(json!(events))
}

fn worker_pids(path: &std::path::Path, prefix: &str) -> Vec<i32> {
    std::fs::read_dir(path)
        .unwrap()
        .filter_map(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_str()?
                .strip_prefix(prefix)?
                .parse()
                .ok()
        })
        .collect()
}

async fn wait_for_prompts(path: &std::path::Path, count: usize) {
    while worker_pids(path, "prompt-").len() < count {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn immediate_refill(lazy: bool) {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("adapter.py");
    let release = temp.path().join("release");
    std::fs::write(
        &script,
        r#"import json, os, pathlib, sys, time
root = pathlib.Path(os.environ['PROBE_DIR'])
(root / ('launch-' + str(os.getpid()))).touch()
for line in sys.stdin:
    message = json.loads(line)
    method = message.get('method')
    if method == 'initialize':
        result = {'protocolVersion': 1, 'agentInfo': {'name': 'refill-probe'}}
    elif method == 'session/new':
        result = {'sessionId': str(os.getpid())}
    elif method == 'session/prompt':
        (root / ('prompt-' + str(os.getpid()))).touch()
        while not (root / 'release').exists():
            time.sleep(0.01)
        result = {'stopReason': 'end_turn'}
    else:
        result = {}
    if 'id' in message:
        print(json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': result}), flush=True)
"#,
    )
    .unwrap();

    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let channels: Vec<_> = (0..3).map(|_| Uuid::new_v4()).collect();
    let author = nostr::Keys::generate();
    let state = Arc::new(RelayFixture {
        channels: channels.clone(),
        pubkey: author.public_key().to_hex(),
        events: tokio::sync::Mutex::new(events_rx),
    });
    let app = Router::new()
        .route("/", get(relay_socket))
        .route("/query", post(relay_query))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = build_mcp_servers_tests::test_config();
    config.relay_url = format!("ws://{address}");
    config.agent_command = "python3".into();
    config.agent_args = vec![script.to_str().unwrap().into()];
    config.persona_env_vars = vec![("PROBE_DIR".into(), temp.path().to_str().unwrap().into())];
    config.agents = 3;
    config.lazy_pool = lazy;
    config.idle_pool_sleep_secs = if lazy { 1 } else { 0 };
    config.presence_enabled = false;
    config.typing_enabled = false;
    config.no_base_prompt = true;
    config.context_message_limit = 0;
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let mut harness = tokio::task::JoinSet::new();
    harness.spawn(run_harness(
        config,
        shutdown_tx.clone(),
        shutdown_rx,
        ready_tx,
    ));

    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        std::panic::AssertUnwindSafe(async {
            ready_rx.await.unwrap();
            if lazy {
                assert!(worker_pids(temp.path(), "launch-").is_empty());
            }
            for round in 0..if lazy { 2 } else { 1 } {
                let events = channels
                    .iter()
                    .map(|channel| {
                        nostr::EventBuilder::new(nostr::Kind::Custom(9), format!("round {round}"))
                            .tags([
                                nostr::Tag::parse(["h".to_owned(), channel.to_string()]).unwrap()
                            ])
                            .sign_with_keys(&author)
                            .unwrap()
                    })
                    .collect();
                events_tx.send(events).unwrap();
                tokio::time::timeout(
                    Duration::from_secs(5),
                    wait_for_prompts(temp.path(), (round + 1) * 3),
                )
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "round {round}: expected {} prompt workers before the 30s maintenance delay, got {}",
                        (round + 1) * 3,
                        worker_pids(temp.path(), "prompt-").len()
                    )
                });
                assert_eq!(worker_pids(temp.path(), "launch-").len(), (round + 1) * 3);
                std::fs::write(&release, "done").unwrap();
                if lazy && round == 0 {
                    // A second wake is essential: the first lazy wake inherits the
                    // eager backdate, whereas this one follows recent maintenance.
                    use nix::{sys::signal::kill, unistd::Pid};
                    let pids = worker_pids(temp.path(), "launch-");
                    while pids
                        .iter()
                        .any(|pid| kill(Pid::from_raw(*pid), None).is_ok())
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    std::fs::remove_file(&release).unwrap();
                }
            }
        })
        .catch_unwind(),
    )
    .await;
    // Release prompts and exercise production shutdown even on a failed deadline.
    std::fs::write(&release, "done").unwrap();
    let _ = shutdown_tx.send(());
    let stopped = tokio::time::timeout(Duration::from_secs(6), harness.join_next()).await;
    if stopped.is_err() {
        harness.shutdown().await;
    }
    server.abort();
    outcome
        .expect("harness refill scenario exceeded its deadline")
        .expect("harness refill scenario failed");
    stopped.unwrap().unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn eager_pool_refills_immediately_through_harness_maintenance() {
    immediate_refill(false).await;
}

#[tokio::test]
async fn lazy_pool_refills_immediately_after_sleep_and_wake() {
    immediate_refill(true).await;
}
