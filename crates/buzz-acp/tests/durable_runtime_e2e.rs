use futures_util::{SinkExt, StreamExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use buzz_acp::e2e_support::{
    exercise_process_backed_adapter_recovery, DurableRuntimeTestConfig, DurableRuntimeTestHarness,
};
use buzz_runtime::{
    process_matches_marker, read_runner_receipt, read_runtime_receipt, Capability, JobListFilter,
    JobRecord, JobStartRequest, JobState, ResumeMode, RunnerReceiptState, RuntimeClient,
};
use buzz_sdk::ThreadRef;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

const ACP_TURN_HARD_LIMIT: Duration = Duration::from_secs(2);
const NORMAL_JOB_DURATION: Duration = Duration::from_secs(4);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

fn owner_directory(path: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).expect("create owner-only directory");
}

#[cfg(unix)]
fn fake_lh(root: &Path, duration: Duration) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let executable = root.join("fake-lh");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf 'starting %s\\n' \"$*\"\nsleep {}\nprintf 'JAC-575 receipt verified\\n'\n",
            duration.as_secs()
        ),
    )
    .expect("write fake LH executable");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("make fake LH executable runnable");
    executable
}

#[cfg(windows)]
fn fake_lh(root: &Path, duration: Duration) -> PathBuf {
    let executable = root.join("fake-lh.cmd");
    let ping_count = duration.as_secs().saturating_add(1);
    std::fs::write(
        &executable,
        format!(
            "@echo starting %*\r\n@ping -n {ping_count} 127.0.0.1 >NUL\r\n@echo JAC-575 receipt verified\r\n"
        ),
    )
    .expect("write fake LH command");
    executable
}

fn packaged_runtime_bundle(root: &Path) -> PathBuf {
    let bundle = root.join("bundle");
    owner_directory(&bundle);
    let source = PathBuf::from(env!("CARGO_BIN_EXE_buzz-acp"));
    let executable_name = |stem: &str| {
        if cfg!(windows) {
            format!("{stem}.exe")
        } else {
            stem.to_owned()
        }
    };
    for stem in ["buzz-acp", "buzz-agent", "buzz-dev-mcp"] {
        let target = bundle.join(executable_name(stem));
        std::fs::copy(&source, &target).expect("copy packaged runtime fixture binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
                .expect("make packaged runtime fixture executable");
        }
    }
    bundle
        .join(executable_name("buzz-acp"))
        .canonicalize()
        .expect("canonicalize packaged runtime fixture")
}

struct RelayFixture {
    url: String,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    subscriptions: Arc<Mutex<Vec<String>>>,
    inject_tx: broadcast::Sender<Event>,
    task: JoinHandle<()>,
}

impl RelayFixture {
    async fn start(agents: &[&Keys], channel_id: Uuid, owner: &Keys) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local relay protocol fixture");
        let address = listener.local_addr().expect("read relay fixture address");
        let channel = channel_id.to_string();
        let mut membership_tags =
            vec![Tag::parse(["d", channel.as_str()]).expect("membership d tag")];
        membership_tags.extend(agents.iter().map(|agent| {
            Tag::parse(["p", agent.public_key().to_hex().as_str()]).expect("membership p tag")
        }));
        let membership = EventBuilder::new(Kind::Custom(39_002), "")
            .tags(membership_tags)
            .sign_with_keys(owner)
            .expect("sign relay membership fixture");
        let metadata = EventBuilder::new(
            Kind::Custom(39_000),
            r#"{"name":"durable-runtime","type":"group"}"#,
        )
        .tags([
            Tag::parse(["d", channel.as_str()]).expect("metadata d tag"),
            Tag::parse(["name", "durable-runtime"]).expect("metadata name tag"),
            Tag::parse(["t", "stream"]).expect("metadata channel-type tag"),
        ])
        .sign_with_keys(owner)
        .expect("sign relay metadata fixture");
        let query_events = Arc::new((
            serde_json::to_value(membership).expect("serialize membership fixture"),
            serde_json::to_value(metadata).expect("serialize metadata fixture"),
        ));
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriptions = Arc::new(Mutex::new(Vec::new()));
        let (inject_tx, _) = broadcast::channel(16);
        let server_events = events.clone();
        let server_subscriptions = subscriptions.clone();
        let server_inject = inject_tx.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let events = server_events.clone();
                let subscriptions = server_subscriptions.clone();
                let inject_rx = server_inject.subscribe();
                let inject_tx = server_inject.clone();
                let query_events = query_events.clone();
                tokio::spawn(async move {
                    let mut first = [0_u8; 4];
                    let count = loop {
                        let Ok(count) = socket.peek(&mut first).await else {
                            return;
                        };
                        if count >= 3 {
                            break count;
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    };
                    if count >= 3 && &first[..3] == b"GET" {
                        serve_relay_websocket(socket, inject_rx, subscriptions).await;
                    } else {
                        serve_relay_http(socket, events, query_events, inject_tx).await;
                    }
                });
            }
        });
        Self {
            url: format!("ws://{address}"),
            events,
            subscriptions,
            inject_tx,
            task,
        }
    }

    async fn publish(&self, channel_id: Uuid, event: Event, state_dir: &Path) {
        let expected = format!("ch-{channel_id}");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self
                .subscriptions
                .lock()
                .expect("lock relay subscriptions")
                .contains(&expected)
            {
                self.inject_tx
                    .send(event)
                    .expect("deliver signed event to subscribed runtime");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for runtime channel subscription; subscriptions={:?}; runtime:\n{}",
                self.subscriptions.lock().expect("lock relay subscriptions"),
                std::fs::read_to_string(state_dir.join("packaged-runtime.log"))
                    .unwrap_or_default()
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn published_job_chain(&self, job_id: Uuid) -> Vec<PublishedJobEvent> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let chain = {
                let events = self.events.lock().expect("lock published relay events");
                events
                    .iter()
                    .filter(|event| event_has_job_tag(event, job_id))
                    .filter_map(published_job_event)
                    .collect::<Vec<_>>()
            };
            if chain.iter().any(|event| event.is_terminal) {
                return chain;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for terminal public job event"
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

impl Drop for RelayFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
struct PublishedJobEvent {
    kind: u16,
    seq: Option<u64>,
    is_terminal: bool,
}

fn event_has_job_tag(event: &serde_json::Value, job_id: Uuid) -> bool {
    let expected = job_id.to_string();
    event
        .get("tags")
        .and_then(|tags| tags.as_array())
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array().is_some_and(|parts| {
                    parts.first().and_then(|part| part.as_str()) == Some("job")
                        && parts.get(1).and_then(|part| part.as_str()) == Some(expected.as_str())
                })
            })
        })
}

fn published_job_event(event: &serde_json::Value) -> Option<PublishedJobEvent> {
    let kind = u16::try_from(event.get("kind")?.as_u64()?).ok()?;
    if !(43_001..=43_006).contains(&kind) {
        return None;
    }
    let payload = event
        .get("content")
        .and_then(|content| content.as_str())
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
    Some(PublishedJobEvent {
        kind,
        seq: payload
            .as_ref()
            .and_then(|payload| payload.get("seq"))
            .and_then(|seq| seq.as_u64()),
        is_terminal: matches!(kind, 43_004 | 43_006),
    })
}

async fn serve_relay_http(
    mut socket: TcpStream,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    query_events: Arc<(serde_json::Value, serde_json::Value)>,
    inject_tx: broadcast::Sender<Event>,
) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let (header_end, content_length) = loop {
        let Ok(count) = socket.read(&mut chunk).await else {
            return;
        };
        if count == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..count]);
        assert!(
            request.len() <= 1024 * 1024,
            "relay fixture request exceeded one MiB"
        );
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        break (header_end + 4, content_length);
    };
    while request.len() < header_end + content_length {
        let Ok(count) = socket.read(&mut chunk).await else {
            return;
        };
        if count == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..count]);
    }
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let path = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let body = &request[header_end..header_end + content_length];
    let response_body = if path == "/events" {
        if let Ok(event) = serde_json::from_slice::<serde_json::Value>(body) {
            if let Ok(signed_event) = serde_json::from_value::<Event>(event.clone()) {
                let _ = inject_tx.send(signed_event);
            }
            events
                .lock()
                .expect("lock relay fixture event capture")
                .push(event);
        }
        r#"{"accepted":true}"#.to_owned()
    } else if path == "/query" {
        let request = String::from_utf8_lossy(body);
        let selected = if request.contains("39002") {
            vec![query_events.0.clone()]
        } else if request.contains("39000") {
            vec![query_events.1.clone()]
        } else {
            Vec::new()
        };
        serde_json::to_string(&selected).expect("serialize relay query response")
    } else {
        "[]".to_owned()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

async fn serve_relay_websocket(
    socket: TcpStream,
    mut inject_rx: broadcast::Receiver<Event>,
    subscriptions: Arc<Mutex<Vec<String>>>,
) {
    let Ok(mut websocket) = tokio_tungstenite::accept_async(socket).await else {
        return;
    };
    if websocket
        .send(Message::Text(
            r#"["AUTH","durable-runtime-fixture"]"#.into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let mut connection_subscriptions = Vec::new();
    loop {
        tokio::select! {
            incoming = websocket.next() => {
                let Some(Ok(message)) = incoming else {
                    break;
                };
                match message {
                    Message::Text(text) => {
                        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        let Some(kind) = frame.get(0).and_then(|value| value.as_str()) else {
                            continue;
                        };
                        if kind == "REQ" {
                            if let Some(id) = frame.get(1).and_then(|id| id.as_str()) {
                                if !connection_subscriptions.iter().any(|known| known == id) {
                                    connection_subscriptions.push(id.to_owned());
                                }
                                let mut shared = subscriptions.lock().expect("lock relay subscriptions");
                                if !shared.iter().any(|known| known == id) {
                                    shared.push(id.to_owned());
                                }
                            }
                        }
                        let reply = match kind {
                            "AUTH" | "EVENT" => frame
                                .get(1)
                                .and_then(|event| event.get("id"))
                                .and_then(|id| id.as_str())
                                .map(|id| serde_json::json!(["OK", id, true, ""])),
                            "REQ" => frame
                                .get(1)
                                .and_then(|id| id.as_str())
                                .map(|id| serde_json::json!(["EOSE", id])),
                            _ => None,
                        };
                        if let Some(reply) = reply {
                            if websocket
                                .send(Message::Text(reply.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        if websocket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            injected = inject_rx.recv() => {
                let Ok(event) = injected else {
                    continue;
                };
                for subscription in connection_subscriptions
                    .iter()
                    .filter(|subscription| subscription.starts_with("ch-"))
                {
                    let frame = serde_json::json!(["EVENT", subscription, event]);
                    if websocket
                        .send(Message::Text(frame.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}
fn mention(keys: &Keys, agent: &Keys, channel_id: Uuid, content: &str) -> Event {
    let channel = channel_id.to_string();
    let agent_pubkey = agent.public_key().to_hex();
    EventBuilder::new(Kind::Custom(9), content)
        .tags([
            Tag::parse(["h", channel.as_str()]).expect("mention channel tag"),
            Tag::parse(["p", agent_pubkey.as_str()]).expect("mention recipient tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign accepted mention")
}

async fn wait_for_completed(store: &buzz_runtime::StoreHandle, expected: u64, state_dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let depths = store
            .queue_depths()
            .await
            .expect("read durable inbox counts");
        if depths.completed >= expected {
            return;
        }
        if Instant::now() >= deadline {
            let runtime_log =
                std::fs::read_to_string(state_dir.join("packaged-runtime.log")).unwrap_or_default();
            let adapter_trace =
                std::fs::read_to_string(state_dir.join("packaged-acp-methods.trace"))
                    .unwrap_or_default();
            panic!(
                "timed out waiting for {expected} completed inbox events: {depths:?}\nruntime:\n{runtime_log}\nadapter:\n{adapter_trace}"
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_for_active_assignment(store: &buzz_runtime::StoreHandle, source_event_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if store
            .active_assignment()
            .await
            .expect("read active assignment")
            .is_some_and(|assignment| {
                assignment.source_event_id.as_deref() == Some(source_event_id)
            })
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for source-bound active assignment"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_for_active_job(
    store: &buzz_runtime::StoreHandle,
    source_event_id: &str,
) -> JobRecord {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(job) = store
            .list_jobs(JobListFilter::default())
            .await
            .expect("read durable jobs")
            .into_iter()
            .find(|job| {
                job.source_event_id.as_deref() == Some(source_event_id)
                    && matches!(job.state, JobState::Accepted | JobState::Running)
            })
        {
            return job;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for assignment-owned governed job"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
async fn wait_until(target: Instant) {
    while Instant::now() < target {
        tokio::time::sleep(POLL_INTERVAL.min(target.saturating_duration_since(Instant::now())))
            .await;
    }
}

async fn exercise_durable_runtime(duration: Duration) {
    let duration = duration.max(NORMAL_JOB_DURATION);
    let temp = tempfile::tempdir().expect("create durable runtime fixture directory");
    let state_dir = temp.path().join("runtime");
    let maintainer_state_dir = temp.path().join("maintainer-runtime");
    let workspace = temp.path().join("workspace");
    owner_directory(&state_dir);
    owner_directory(&maintainer_state_dir);
    owner_directory(&workspace);
    let workspace = workspace
        .canonicalize()
        .expect("canonicalize approved workspace root");

    let receipt_path = state_dir.join("runtime-receipt.json");
    let maintainer_receipt_path = maintainer_state_dir.join("runtime-receipt.json");
    let lh_executable = fake_lh(temp.path(), duration)
        .canonicalize()
        .expect("canonicalize fake allowlisted LH executable");
    let runner_executable = packaged_runtime_bundle(temp.path());
    let keys = Keys::generate();
    let sender = Keys::generate();
    let maintainer = Keys::generate();
    let channel_id = Uuid::new_v4();
    let first = mention(
        &sender,
        &keys,
        channel_id,
        "@sage run the governed JAC-575 repair",
    );
    let dm_channel = channel_id;
    let maintainer_mention = maintainer.public_key().to_hex();
    let review_request = buzz_sdk::build_message(
        dm_channel,
        "Maintainer, review the active JAC-575 repair",
        None,
        &[maintainer_mention.as_str()],
        false,
        &[],
    )
    .expect("build Sage review DM")
    .sign_with_keys(&keys)
    .expect("sign Sage review DM");
    let sage_mention = keys.public_key().to_hex();
    let review_reply = buzz_sdk::build_message(
        dm_channel,
        "Reviewed: keep the receipt and runner identity evidence",
        Some(&ThreadRef {
            root_event_id: review_request.id,
            parent_event_id: review_request.id,
        }),
        &[sage_mention.as_str()],
        false,
        &[],
    )
    .expect("build Maintainer threaded reply")
    .sign_with_keys(&maintainer)
    .expect("sign Maintainer threaded reply");
    review_request
        .verify()
        .expect("verify signed Sage review DM");
    review_reply
        .verify()
        .expect("verify signed Maintainer threaded reply");
    let relay = RelayFixture::start(&[&keys, &maintainer], channel_id, &sender).await;
    let config = DurableRuntimeTestConfig {
        runtime_id: "jac-575-e2e-runtime".into(),
        state_dir: state_dir.clone(),
        receipt_path: receipt_path.clone(),
        lh_executable: lh_executable.clone(),
        workspace_roots: vec![workspace.clone()],
        runner_executable: runner_executable.clone(),
        keys: keys.clone(),
        owner_pubkey: sender.public_key().to_hex(),
        allowed_pubkeys: vec![maintainer.public_key().to_hex()],
        relay_url: relay.url.clone(),
        auto_job: Some(JobStartRequest {
            channel_id,
            source_event_id: Some(first.id.to_hex()),
            driver: "lh".into(),
            argv: vec![
                "lockdown".into(),
                "run".into(),
                "--issue".into(),
                "JAC-575".into(),
            ],
            cwd: workspace.to_string_lossy().into_owned(),
            summary: "Run the receipt-verified JAC-575 repair".into(),
        }),
        auto_reply: Some(review_request.clone()),
    };
    let maintainer_config = DurableRuntimeTestConfig {
        runtime_id: "jac-575-maintainer-runtime".into(),
        state_dir: maintainer_state_dir.clone(),
        receipt_path: maintainer_receipt_path,
        lh_executable,
        workspace_roots: vec![workspace.clone()],
        runner_executable: runner_executable.clone(),
        keys: maintainer.clone(),
        owner_pubkey: sender.public_key().to_hex(),
        allowed_pubkeys: vec![keys.public_key().to_hex()],
        relay_url: relay.url.clone(),
        auto_job: None,
        auto_reply: Some(review_reply.clone()),
    };

    let maintainer_runtime = DurableRuntimeTestHarness::start(maintainer_config)
        .await
        .expect("start Maintainer coworker runtime fixture");
    let maintainer_store = maintainer_runtime.store();

    let runtime = DurableRuntimeTestHarness::start(config)
        .await
        .expect("start durable runtime fixture");
    let generation = runtime.generation();
    let store = runtime.store();
    let job_started_at = Instant::now();
    relay.publish(channel_id, first.clone(), &state_dir).await;
    wait_for_completed(&store, 1, &state_dir).await;
    wait_for_completed(&maintainer_store, 1, &maintainer_state_dir).await;
    wait_for_completed(&store, 2, &state_dir).await;
    wait_for_active_assignment(&store, &first.id.to_hex()).await;
    let accepted = wait_for_active_job(&store, &first.id.to_hex())
        .await
        .to_status();
    assert_eq!(accepted.state, JobState::Running);
    assert_eq!(accepted.progress_seq, 1);
    let runner_pid = accepted
        .runner_pid
        .expect("accepted job exposes local runner PID");
    let runner_marker = accepted
        .runner_start_marker
        .clone()
        .expect("accepted job exposes runner start marker");
    assert!(process_matches_marker(runner_pid, &runner_marker));

    let second = mention(
        &sender,
        &keys,
        channel_id,
        "@sage keep me posted while JAC-575 continues",
    );
    relay.publish(channel_id, second, &state_dir).await;
    wait_for_completed(&store, 3, &state_dir).await;
    assert_eq!(
        store
            .queue_depths()
            .await
            .expect("read durable inbox counts")
            .completed,
        3,
        "both owner mentions and the coworker reply must be durably accounted for"
    );
    assert_eq!(
        store
            .list_jobs(Default::default())
            .await
            .expect("list pair-scoped durable jobs")
            .len(),
        1,
        "the second mention must not start a competing job or runtime"
    );

    assert!(
        serde_json::to_string(&review_reply.tags)
            .expect("serialize threaded reply tags")
            .contains(&review_request.id.to_hex()),
        "Maintainer reply must retain the Sage DM thread root"
    );
    assert!(
        maintainer_store
            .queue_depths()
            .await
            .expect("count Maintainer-consumed review request")
            .completed
            >= 1,
        "Maintainer runtime must consume Sage's signed review request"
    );
    wait_for_completed(&store, 3, &state_dir).await;
    assert_eq!(
        store
            .queue_depths()
            .await
            .expect("count consumed collaboration reply")
            .completed,
        3,
        "Sage must consume the Maintainer reply without losing prior mentions"
    );

    let desktop_client = RuntimeClient::from_receipt(&receipt_path, Capability::Controller)
        .await
        .expect("reattach Desktop controller without owning runtime lifetime");
    assert_eq!(
        desktop_client
            .status()
            .await
            .expect("read runtime status")
            .generation,
        generation
    );
    drop(desktop_client);

    let runtime = runtime
        .restart()
        .await
        .expect("restart and recover runtime supervisor");
    let restarted_generation = runtime.generation();
    assert_ne!(
        restarted_generation, generation,
        "a replacement runtime process must fence stale clients with a new generation"
    );
    let restarted_receipt =
        read_runtime_receipt(&receipt_path).expect("read recovered runtime receipt");
    assert_eq!(restarted_receipt.generation, restarted_generation);
    let controller = RuntimeClient::from_receipt(&receipt_path, Capability::Controller)
        .await
        .expect("authenticate after runtime recovery");
    let recovered = controller
        .jobs_status(accepted.job_id)
        .await
        .expect("recover detached job status");
    assert_eq!(recovered.runner_pid, Some(runner_pid));
    assert_eq!(
        recovered.runner_start_marker.as_deref(),
        Some(runner_marker.as_str())
    );

    let adapter_recovery = exercise_process_backed_adapter_recovery(
        &store,
        channel_id,
        &workspace,
        Path::new(env!("CARGO_BIN_EXE_buzz-acp")),
        &temp.path().join("acp-methods.trace"),
    )
    .await
    .expect("kill and respawn the ACP adapter with durable session recovery");
    assert_eq!(adapter_recovery.session_id, "jac-575-acp-session");
    assert_eq!(adapter_recovery.resume_mode, ResumeMode::Resume);
    assert_eq!(
        adapter_recovery.methods,
        vec![
            "initialize".to_owned(),
            "session/new".to_owned(),
            "initialize".to_owned(),
            "session/resume".to_owned(),
        ],
        "replacement adapter must resume the exact persisted session"
    );
    let after_adapter_restart = controller
        .jobs_status(accepted.job_id)
        .await
        .expect("active job survives ACP adapter replacement");
    assert_eq!(after_adapter_restart.runner_pid, Some(runner_pid));

    wait_until(job_started_at + ACP_TURN_HARD_LIMIT + Duration::from_millis(100)).await;
    let beyond_turn_deadline = controller
        .jobs_status(accepted.job_id)
        .await
        .expect("query runner after ACP hard-turn deadline");
    assert_eq!(beyond_turn_deadline.state, JobState::Running);
    assert_eq!(beyond_turn_deadline.runner_pid, Some(runner_pid));
    assert!(process_matches_marker(runner_pid, &runner_marker));

    let terminal_deadline = job_started_at + duration + Duration::from_secs(10);
    let terminal = loop {
        let status = controller
            .jobs_status(accepted.job_id)
            .await
            .expect("poll detached job through authenticated control");
        if status.state.is_terminal() {
            break status;
        }
        assert!(
            Instant::now() < terminal_deadline,
            "detached job did not become terminal"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    };
    assert_eq!(terminal.state, JobState::Succeeded);
    assert_eq!(terminal.exit_code, Some(0));
    assert_eq!(terminal.runner_pid, Some(runner_pid));

    let terminal_receipt = read_runner_receipt(&state_dir, accepted.job_id, accepted.attempt)
        .expect("read successful terminal runner receipt");
    assert_eq!(terminal_receipt.state, RunnerReceiptState::Succeeded);
    assert_eq!(terminal_receipt.runner_pid, runner_pid);
    assert_eq!(terminal_receipt.runner_start_marker, runner_marker);
    assert_eq!(terminal_receipt.exit_code, Some(0));

    let chain = relay.published_job_chain(accepted.job_id).await;
    let kinds: Vec<_> = chain.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds.first(),
        Some(&43_001),
        "request must lead the durable event chain"
    );
    assert_eq!(
        kinds.get(1),
        Some(&43_002),
        "acceptance must follow request"
    );
    let progress: Vec<_> = chain
        .iter()
        .filter(|event| event.kind == 43_003)
        .map(|event| event.seq.expect("progress event carries sequence"))
        .collect();
    assert!(
        !progress.is_empty(),
        "runtime must publish real progress before success"
    );
    assert!(progress.windows(2).all(|window| window[0] < window[1]));
    assert_eq!(progress.last().copied(), Some(terminal.progress_seq));
    let successful_terminals: Vec<_> = chain
        .iter()
        .filter(|event| event.kind == 43_004 && event.is_terminal)
        .collect();
    assert_eq!(
        successful_terminals.len(),
        1,
        "exactly one terminal success event is durable"
    );
    assert_eq!(chain.iter().filter(|event| event.is_terminal).count(), 1);
    assert_eq!(
        kinds.last(),
        Some(&43_004),
        "success must terminate the ordered chain"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jac_575_survives_turn_runtime_and_desktop_restart() {
    exercise_durable_runtime(NORMAL_JOB_DURATION).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "release acceptance proves detached work outlives two minutes"]
async fn jac_575_release_acceptance_runs_past_120_seconds() {
    let seconds = std::env::var("BUZZ_DURABILITY_CANARY_SECS")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .expect("BUZZ_DURABILITY_CANARY_SECS must be an integer number of seconds")
        })
        .unwrap_or(121);
    assert!(seconds > 120, "release acceptance must exceed two minutes");
    exercise_durable_runtime(Duration::from_secs(seconds)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "packaged three-hour durability release canary"]
async fn packaged_three_hour_canary() {
    let seconds = std::env::var("BUZZ_DURABILITY_CANARY_SECS")
        .expect("BUZZ_DURABILITY_CANARY_SECS=10800 is required for the packaged canary")
        .parse::<u64>()
        .expect("BUZZ_DURABILITY_CANARY_SECS must be an integer number of seconds");
    assert!(
        seconds >= 10_800,
        "packaged durability canary must run for at least three hours"
    );
    exercise_durable_runtime(Duration::from_secs(seconds)).await;
}
