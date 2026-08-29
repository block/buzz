//! Real-binary trace-redaction canary test.
//!
//! Launches the built `buzz-acp` binary (via `CARGO_BIN_EXE_buzz-acp`) with
//! `RUST_LOG=trace`, wired to a fake in-test Nostr relay (WebSocket + HTTP
//! bridge on one port) and a fake ACP agent stub (a Python script speaking
//! JSON-RPC over stdio). Canaries are injected through every channel the
//! harness forwards — inbound event content, system prompt, non-secret MCP
//! config env, process env, and agent-emitted session/update frames — never argv. The
//! child's REAL stdout and stderr pipes are captured and grepped raw; the
//! binary writes no log file, so the two pipes are the complete log surface.
//!
//! Positive controls (metadata-only log shapes such as `method_hash`,
//! `title_hash`, `line_bytes`, `agent child stderr line`) must appear in the
//! same capture, so an empty or misrouted stream cannot pass vacuously.

use std::io::Read;
#[cfg(target_os = "macos")]
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag, ToBech32};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CONTENT_CANARY: &str = "BUZZ_TRACE_CONTENT_CANARY_7f4d21";
const THOUGHT_CANARY: &str = "BUZZ_TRACE_THOUGHT_CANARY_67ab09";
const TOOL_CANARY: &str = "BUZZ_TRACE_TOOL_OUTPUT_CANARY_5cc820";
const AUTH_TAG_CANARY: &str = "BUZZ_TRACE_AUTH_TAG_CANARY_e06fac";
const BEARER_CANARY: &str = "BUZZ_TRACE_BEARER_CANARY_d38891";
const CAPABILITY_CANARY: &str = "BUZZ_TRACE_CAPABILITY_CANARY_c1a70e";
const SYSTEM_PROMPT_CANARY: &str = "BUZZ_TRACE_SYSTEM_PROMPT_CANARY_239dbc";
const MCP_ENV_CANARY: &str = "BUZZ_TRACE_MCP_ENV_CANARY_3e7771";
const TITLE_CANARY: &str = "BUZZ_TRACE_TITLE_CANARY_08ab51";
const KIND_CANARY: &str = "BUZZ_TRACE_KIND_CANARY_903fd2";
const TOOL_ID_CANARY: &str = "BUZZ_TRACE_TOOL_ID_CANARY_921ced";
const STATUS_CANARY: &str = "BUZZ_TRACE_STATUS_CANARY_4955aa";
const COMMAND_CANARY: &str = "BUZZ_TRACE_COMMAND_CANARY_dbe712";
const RUN_ID_CANARY: &str = "BUZZ_TRACE_RUN_ID_CANARY_8f342c";
const UPDATE_TYPE_CANARY: &str = "BUZZ_TRACE_UPDATE_TYPE_CANARY_ca2851";
const CHILD_STDOUT_CANARY: &str = "BUZZ_TRACE_CHILD_STDOUT_CANARY_c8bc95";
const CHILD_STDERR_CANARY: &str = "BUZZ_TRACE_CHILD_STDERR_CANARY_3d73be";

#[cfg(target_os = "macos")]
#[test]
fn desktop_credential_stdin_is_consumed_without_argv_or_environment_exposure() {
    const PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const AUTH_CANARY: &str = "STACY_ACP_STDIN_AUTH_CANARY_d97341";

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind handshake probe");
    let relay_addr = listener.local_addr().expect("probe address");
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let listener_thread = std::thread::spawn(move || {
        let (_socket, _) = listener.accept().expect("accept buzz-acp connection");
        accepted_tx.send(()).expect("signal accepted connection");
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });

    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .env("BUZZ_ACP_CREDENTIAL_STDIN", "true")
        .env("BUZZ_RELAY_URL", format!("ws://{relay_addr}"))
        .env("BUZZ_ACP_AGENT_COMMAND", "/usr/bin/true")
        .env("BUZZ_ACP_LAZY_POOL", "true")
        .env("BUZZ_ACP_NO_PRESENCE", "true")
        .env("BUZZ_ACP_NO_TYPING", "true")
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn credential-stdin buzz-acp");
    let mut stderr = child.stderr.take().expect("credential probe stderr");
    let envelope = serde_json::to_vec(&json!({
        "private_key": PRIVATE_KEY,
        "auth_tag": AUTH_CANARY,
    }))
    .expect("serialize credential envelope");
    let mut stdin = child.stdin.take().expect("credential stdin pipe");
    stdin
        .write_all(&envelope)
        .expect("write credential envelope");
    drop(stdin);

    if accepted_rx.recv_timeout(Duration::from_secs(5)).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let mut diagnostic = String::new();
        stderr
            .read_to_string(&mut diagnostic)
            .expect("read credential probe diagnostic");
        panic!("buzz-acp did not reach relay connect: {diagnostic}");
    }
    let inspected = Command::new("/bin/ps")
        .args(["eww", "-p", &child.id().to_string()])
        .output()
        .expect("inspect buzz-acp argv and environment");
    let surface = String::from_utf8_lossy(&inspected.stdout);
    assert!(!surface.contains(PRIVATE_KEY));
    assert!(!surface.contains(AUTH_CANARY));

    let _ = child.kill();
    let _ = child.wait();
    let _ = release_tx.send(());
    listener_thread.join().expect("join handshake probe");
}

/// Fake ACP agent: answers initialize / session/new, and on session/prompt
/// emits one session/update frame per canary class plus a raw stderr line.
/// Child-emitted canaries are embedded in the private test stub; the same
/// values are independently placed in the parent environment to prove that
/// `env_clear` neither leaks them to the child nor logs them from the parent.
const AGENT_STUB: &str = r#"#!/usr/bin/python3
import json, os, sys

CANARIES = __CANARIES_JSON__

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

def canary(name):
    return CANARIES.get(name, "missing-" + name)

def update(u):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {"update": u}})

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    method = msg.get("method")
    msg_id = msg.get("id")
    if method == "initialize":
        for forbidden in ["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG",
                          "BUZZ_API_TOKEN", "BUZZ_ACP_PRIVATE_KEY", "BUZZ_ACP_API_TOKEN"]:
            if forbidden in os.environ:
                sys.stderr.write("ACP_CHILD_CREDENTIAL_DISCLOSURE:" + forbidden + "\n")
                sys.stderr.flush()
                sys.exit(92)
        send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
    elif method == "session/new":
        visible = json.dumps(msg.get("params", {}).get("mcpServers", []))
        for forbidden in ["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"]:
            if forbidden in visible:
                sys.stderr.write("ACP_MCP_SECRET_DISCLOSURE:" + forbidden + "\n")
                sys.stderr.flush()
                sys.exit(91)
        send({"jsonrpc": "2.0", "id": msg_id, "result": {"sessionId": "trace-session"}})
    elif method == "session/prompt":
        update({"sessionUpdate": "agent_thought_chunk", "content": {"text": canary("THOUGHT")}})
        update({"sessionUpdate": "tool_call", "title": canary("TITLE"), "kind": canary("KIND")})
        update({"sessionUpdate": "tool_call_update", "toolCallId": canary("TOOL_ID"),
                "status": canary("STATUS"),
                "content": [{"type": "content", "content": {"type": "text", "text": canary("TOOL_OUTPUT")}}]})
        update({"sessionUpdate": "available_commands_update",
                "availableCommands": [{"name": canary("COMMAND")}]})
        update({"sessionUpdate": "session_info_update",
                "_meta": {"goose": {"activeRunId": canary("RUN_ID")}}})
        update({"sessionUpdate": canary("UPDATE_TYPE")})
        update({"sessionUpdate": "agent_message_chunk", "content": {"text": canary("CHILD_STDOUT")}})
        sys.stderr.write(canary("CHILD_STDERR") + "\n")
        sys.stderr.flush()
        send({"jsonrpc": "2.0", "id": msg_id, "result": {"stopReason": "end_turn"}})
    elif msg_id is not None and method is not None:
        send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
"#;

const FIXTURE_CHANNEL: &str = "5a1e05f0-7c3d-4e6b-9a2f-1b8c33d90277";

struct RelayFixture {
    owner: Keys,
    agent_pubkey_hex: String,
}

/// One accepted connection: peek the first bytes to route between the NIP-01
/// WebSocket protocol and the HTTP bridge (`POST /query` etc.) which share
/// one port in production.
async fn handle_connection(stream: TcpStream, fixture: Arc<RelayFixture>) {
    let mut head = [0u8; 4];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match stream.peek(&mut head).await {
            Ok(n) if n >= head.len() => break,
            Ok(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            _ => return,
        }
    }
    if &head == b"POST" {
        handle_http(stream, fixture).await;
    } else {
        handle_ws(stream, fixture).await;
    }
}

/// Minimal HTTP bridge: answer the two discovery queries with fixture events
/// and everything else with an empty result set.
async fn handle_http(mut stream: TcpStream, _fixture: Arc<RelayFixture>) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(n)) => raw.extend_from_slice(&buf[..n]),
        }
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if raw.len() > 64 * 1024 {
            return;
        }
    };
    let headers_text = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let content_length = headers_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while raw.len() < header_end + content_length {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(n)) => raw.extend_from_slice(&buf[..n]),
        }
    }
    let body = String::from_utf8_lossy(&raw[header_end..header_end + content_length]).to_string();

    // Channel discovery: kind:39002 membership, then kind:39000 metadata.
    // Tag-only parsing on the harness side — no signatures required here.
    let response_body = if body.contains("39002") {
        json!([{ "tags": [["d", FIXTURE_CHANNEL]] }]).to_string()
    } else if body.contains("39000") {
        json!([{ "tags": [["d", FIXTURE_CHANNEL], ["name", "redaction-dm"], ["t", "dm"]] }])
            .to_string()
    } else {
        "[]".to_string()
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body,
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Minimal NIP-01/NIP-42 relay: AUTH challenge, OK every AUTH/EVENT, EOSE
/// every REQ, and deliver one owner-signed kind-9 canary event on the first
/// channel subscription.
async fn handle_ws(stream: TcpStream, fixture: Arc<RelayFixture>) {
    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    if ws
        .send(json!(["AUTH", "redaction-challenge"]).to_string().into())
        .await
        .is_err()
    {
        return;
    }
    let mut delivered_chat_event = false;
    while let Some(Ok(message)) = ws.next().await {
        let Ok(text) = message.to_text() else {
            continue;
        };
        let Ok(frame) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        match frame[0].as_str() {
            Some("AUTH") | Some("EVENT") => {
                let event_id = frame[1]["id"].as_str().unwrap_or_default().to_string();
                if ws
                    .send(json!(["OK", event_id, true, ""]).to_string().into())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Some("REQ") => {
                let sub_id = frame[1].as_str().unwrap_or_default().to_string();
                if sub_id.starts_with("ch-") && !delivered_chat_event {
                    delivered_chat_event = true;
                    let chat_event = EventBuilder::new(Kind::Custom(9), CONTENT_CANARY)
                        .tags([
                            Tag::parse(["h", FIXTURE_CHANNEL]).expect("channel tag"),
                            Tag::parse(["p", &fixture.agent_pubkey_hex]).expect("mention tag"),
                        ])
                        .sign_with_keys(&fixture.owner)
                        .expect("sign fixture chat event");
                    let event_json =
                        serde_json::to_value(&chat_event).expect("serialize chat event");
                    if ws
                        .send(json!(["EVENT", sub_id, event_json]).to_string().into())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                if ws
                    .send(json!(["EOSE", sub_id]).to_string().into())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            _ => {}
        }
    }
}

fn capture_pipe(mut pipe: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&buffer);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            writer
                .lock()
                .expect("capture buffer lock")
                .extend_from_slice(&chunk[..n]);
        }
    });
    buffer
}

fn snapshot(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&buffer.lock().expect("capture buffer lock")).into_owned()
}

fn terminate_and_wait(child: &mut Child) -> std::process::ExitStatus {
    #[allow(unsafe_code)]
    // SAFETY-FREE: nix wraps kill(2); SIGTERM triggers the harness's graceful
    // shutdown path, which is part of what this test asserts (exit success).
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGTERM,
    )
    .expect("send SIGTERM to buzz-acp");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait().expect("poll buzz-acp exit") {
            Some(status) => return status,
            None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            None => {
                child.kill().expect("kill wedged buzz-acp");
                panic!("buzz-acp did not exit within 20s of SIGTERM");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_binary_trace_logs_never_expose_content_or_credentials() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let agent_private_key_hex = agent.secret_key().to_secret_hex();
    // Include the bech32 form as a negative-control leak class even though raw
    // signing material is forbidden from the ACP-visible MCP envelope.
    let agent_private_key_bech32 = agent
        .secret_key()
        .to_bech32()
        .expect("secret key bech32 encoding");

    // Fake relay: WebSocket + HTTP bridge on one ephemeral port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake relay");
    let relay_addr = listener.local_addr().expect("fake relay address");
    let fixture = Arc::new(RelayFixture {
        owner: owner.clone(),
        agent_pubkey_hex: agent.public_key().to_hex(),
    });
    let relay_task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(handle_connection(stream, Arc::clone(&fixture)));
        }
    });

    let env_canaries = [
        ("THOUGHT", THOUGHT_CANARY),
        ("TOOL_OUTPUT", TOOL_CANARY),
        ("BEARER", BEARER_CANARY),
        ("ADAPTER_CAPABILITY", CAPABILITY_CANARY),
        ("TITLE", TITLE_CANARY),
        ("KIND", KIND_CANARY),
        ("TOOL_ID", TOOL_ID_CANARY),
        ("STATUS", STATUS_CANARY),
        ("COMMAND", COMMAND_CANARY),
        ("RUN_ID", RUN_ID_CANARY),
        ("UPDATE_TYPE", UPDATE_TYPE_CANARY),
        ("CHILD_STDOUT", CHILD_STDOUT_CANARY),
        ("CHILD_STDERR", CHILD_STDERR_CANARY),
    ];

    // Fake ACP agent stub on disk. Embed the child-output canaries so the
    // hardened empty child environment remains part of the real test path.
    let fixture_root = std::env::temp_dir().join(format!(
        "buzz-acp-redaction-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos(),
    ));
    let stub_dir = fixture_root
        .join(".buzz")
        .join(".scratch")
        .join("managed-agents")
        .join("ab".repeat(32));
    std::fs::create_dir_all(&stub_dir).expect("create stub dir");
    let stub_path = stub_dir.join("fake-acp-agent.py");
    let canaries_json = serde_json::to_string(
        &env_canaries
            .iter()
            .copied()
            .collect::<std::collections::BTreeMap<_, _>>(),
    )
    .expect("serialize child canaries");
    let agent_stub = AGENT_STUB.replace("__CANARIES_JSON__", &canaries_json);
    std::fs::write(&stub_path, agent_stub).expect("write agent stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod agent stub");
        std::fs::set_permissions(&stub_dir, std::fs::Permissions::from_mode(0o700))
            .expect("secure managed scratch fixture");
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .env("RUST_LOG", "trace")
        .env("BUZZ_RELAY_URL", format!("ws://{relay_addr}"))
        .env("BUZZ_PRIVATE_KEY", &agent_private_key_hex)
        .env("NOSTR_PRIVATE_KEY", "hostile-nostr-alias")
        .env("BUZZ_API_TOKEN", "hostile-api-token")
        .env("BUZZ_ACP_PRIVATE_KEY", "hostile-acp-key")
        .env("BUZZ_ACP_API_TOKEN", "hostile-acp-token")
        .env("BUZZ_ACP_AGENT_OWNER", owner.public_key().to_hex())
        // Invoke the interpreter directly. macOS Seatbelt does not reliably
        // preserve a script shebang's interpreter transition when the script
        // itself is the sandbox-exec command.
        .env("BUZZ_ACP_AGENT_COMMAND", "/usr/bin/python3")
        .env("BUZZ_ACP_AGENT_ARGS", stub_path.as_os_str())
        .env("BUZZ_ACP_CHILD_SCRATCH", &stub_dir)
        // Hostile system prompt canary flows through session/new params.
        .env("BUZZ_ACP_SYSTEM_PROMPT", SYSTEM_PROMPT_CANARY)
        // Non-empty MCP command makes the harness build MCP server config in
        // session/new. The stub never spawns it; secret absence is therefore
        // proven at the exact ACP-visible envelope.
        .env("BUZZ_ACP_MCP_COMMAND", "/usr/bin/true")
        .env("BUZZ_AUTH_TAG", AUTH_TAG_CANARY)
        .env("BUZZ_ACP_DISPLAY_NAME", MCP_ENV_CANARY)
        .env("BUZZ_ACP_CONTEXT_MESSAGE_LIMIT", "0")
        .env("BUZZ_ACP_NO_MEMORY", "true")
        .env("BUZZ_ACP_NO_PRESENCE", "true")
        .env("BUZZ_ACP_NO_TYPING", "true")
        .env("BUZZ_ACP_IDLE_TIMEOUT", "20")
        .current_dir(&stub_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (class, value) in env_canaries {
        command.env(format!("BUZZ_REDACTION_{class}"), value);
    }
    let mut child = command.spawn().expect("spawn buzz-acp binary");
    let stdout_capture = capture_pipe(child.stdout.take().expect("child stdout pipe"));
    let stderr_capture = capture_pipe(child.stderr.take().expect("child stderr pipe"));

    // Positive controls: metadata-only log shapes that the redacted trace
    // stream MUST contain once the canary turn ran. Their presence proves the
    // capture is the real log stream, not an empty or misrouted pipe.
    let positive_markers = [
        "method_hash",
        "title_hash",
        "kind_hash",
        "tool_id_hash",
        "status_hash",
        "run_id_hash",
        "update_type_hash",
        "agent child stderr line",
        "line_bytes",
    ];
    let turn_deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let stdout_text = snapshot(&stdout_capture);
        if positive_markers
            .iter()
            .all(|marker| stdout_text.contains(marker))
        {
            break;
        }
        assert!(
            Instant::now() < turn_deadline,
            "canary turn did not complete: missing positive markers {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            positive_markers
                .iter()
                .filter(|marker| !stdout_text.contains(*marker))
                .collect::<Vec<_>>(),
            stdout_text,
            snapshot(&stderr_capture),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Real exit status from child.wait(): SIGTERM must produce a graceful,
    // successful shutdown.
    let status = tokio::task::spawn_blocking(move || {
        let status = terminate_and_wait(&mut child);
        (status, child)
    })
    .await
    .expect("join shutdown task");
    relay_task.abort();
    let (exit_status, _child) = status;
    assert!(
        exit_status.success(),
        "buzz-acp must shut down cleanly after the canary turn: {exit_status:?}",
    );

    let stdout_text = snapshot(&stdout_capture);
    let stderr_text = snapshot(&stderr_capture);
    let leak_classes = [
        ("inbound_content", CONTENT_CANARY),
        ("thought", THOUGHT_CANARY),
        ("tool_output", TOOL_CANARY),
        ("private_key_hex", agent_private_key_hex.as_str()),
        ("private_key_bech32", agent_private_key_bech32.as_str()),
        ("auth_tag", AUTH_TAG_CANARY),
        ("bearer", BEARER_CANARY),
        ("adapter_capability", CAPABILITY_CANARY),
        ("hostile_system_prompt", SYSTEM_PROMPT_CANARY),
        ("mcp_env", MCP_ENV_CANARY),
        ("title", TITLE_CANARY),
        ("kind", KIND_CANARY),
        ("tool_id", TOOL_ID_CANARY),
        ("status", STATUS_CANARY),
        ("command", COMMAND_CANARY),
        ("run_id", RUN_ID_CANARY),
        ("update_type", UPDATE_TYPE_CANARY),
        ("outbound_content", CHILD_STDOUT_CANARY),
        ("child_stderr", CHILD_STDERR_CANARY),
    ];
    for (class, canary) in leak_classes {
        assert!(
            !stdout_text.contains(canary),
            "RUST_LOG=trace leaked {class} canary on real stdout",
        );
        assert!(
            !stderr_text.contains(canary),
            "RUST_LOG=trace leaked {class} canary on real stderr",
        );
    }

    let _ = std::fs::remove_dir_all(&fixture_root);
}
