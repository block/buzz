//! `session/cancel` — the last ACP method without coverage.
//!
//! buzz-acp sends this as a *notification* (no id, no response expected) and
//! then waits for the in-flight `session/prompt` to return. Two things must
//! hold or the harness wedges:
//!
//! 1. The prompt response arrives, with `stopReason: "cancelled"`.
//! 2. Every announced tool call reaches a terminal state — buzz-agent
//!    guaranteed this (`agent.rs:470-477`) because the desktop UI otherwise
//!    shows a spinner forever.
//!
//! Goose plumbs the cancellation token into MCP tool calls and emits
//! `notifications/cancelled` per in-flight request (`mcp_client.rs:687-690`),
//! so this is a cooperative drain rather than a hard abort.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

mod approve;

/// Provider whose first generation calls a tool that will hang.
fn spawn_hanging_tool_provider() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let requests = Arc::new(AtomicUsize::new(0));

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    break;
                }
                let t = line.trim_end();
                if t.is_empty() {
                    break;
                }
                if let Some(v) = t
                    .strip_prefix("content-length: ")
                    .or_else(|| t.strip_prefix("Content-Length: "))
                {
                    content_length = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            let respond = |stream: &mut std::net::TcpStream, ct: &str, payload: String| {
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: {ct}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
            };

            if request_line.contains("/models") {
                respond(
                    &mut stream,
                    "application/json",
                    json!({ "object": "list", "data": [] }).to_string(),
                );
                continue;
            }

            let request_number = requests.fetch_add(1, Ordering::SeqCst);
            let (call, done) = if request_number == 0 {
                (
                    json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 1,
                        "model": "fake-model",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "role": "assistant",
                                "tool_calls": [{
                                    "index": 0,
                                    "id": "call_1",
                                    "type": "function",
                                    "function": { "name": "devtools__tool_0", "arguments": "{}" },
                                }],
                            },
                            "finish_reason": null,
                        }],
                    }),
                    json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 1,
                        "model": "fake-model",
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }],
                        "usage": { "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 },
                    }),
                )
            } else {
                (
                    json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 1,
                        "model": "fake-model",
                        "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "done" }, "finish_reason": null }],
                    }),
                    json!({
                        "id": "c", "object": "chat.completion.chunk", "created": 1,
                        "model": "fake-model",
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                        "usage": { "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 },
                    }),
                )
            };
            respond(
                &mut stream,
                "text/event-stream",
                format!("data: {call}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            );
        }
    });

    format!("http://{addr}")
}

/// Provider that accepts the completion POST but never sends response
/// headers, modelling a queued or wedged gateway.
///
/// `/models` still answers so session setup succeeds; only the completion
/// request stalls. The socket is parked, not closed: a closed socket is a
/// *fast* failure, and the scenario under test is the slow one.
fn spawn_header_stalling_provider() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");

    std::thread::spawn(move || {
        let mut parked: Vec<std::net::TcpStream> = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));

            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    break;
                }
                let t = line.trim_end();
                if t.is_empty() {
                    break;
                }
                if let Some(v) = t
                    .strip_prefix("content-length: ")
                    .or_else(|| t.strip_prefix("Content-Length: "))
                {
                    content_length = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            if request_line.contains("/models") {
                let payload = json!({ "object": "list", "data": [] }).to_string();
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
                continue;
            }

            // A completion request: hold the connection open with no headers.
            parked.push(stream);
        }
    });

    format!("http://{addr}")
}

struct Harness {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    fn start(base_url: &str, home: &std::path::Path) -> Self {
        Self::start_with_env(base_url, home, &[])
    }

    fn start_with_env(base_url: &str, home: &std::path::Path, env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-agent"));
        command
            .env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "k")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .expect("write");
        self.stdin.flush().expect("flush");
        id
    }

    /// Fire-and-forget, exactly as buzz-acp sends `session/cancel`.
    fn notify(&mut self, method: &str, params: Value) {
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "method": method, "params": params })
        )
        .expect("write");
        self.stdin.flush().expect("flush");
    }

    /// Pump agent output until the authorization gate has been answered, so a
    /// subsequent cancel lands while the tool is genuinely in flight rather
    /// than while the ask is still outstanding.
    ///
    /// Without this the cancel resolves the permission wait instead, the tool
    /// is never dispatched, and a test named "cancel mid tool call" would in
    /// fact exercise "cancel before tool call" — which is a different path and
    /// cannot produce `notifications/cancelled`.
    fn approve_next_tool_call(&mut self) {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout before asking for permission");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if approve::is_permission_request(&msg) {
                let response = approve::approve(&msg);
                writeln!(self.stdin, "{response}").expect("write approval");
                self.stdin.flush().expect("flush approval");
                return;
            }
        }
    }

    fn await_response(&mut self, id: i64) -> (Value, Vec<Value>) {
        let mut notifications = Vec::new();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout awaiting id={id}");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            // Answer the authorization gate so the tool under test actually
            // runs. This suite's subject is not the permission boundary (see
            // `permission_boundary.rs`), so approval is automatic.
            if approve::is_permission_request(&msg) {
                let response = approve::approve(&msg);
                writeln!(self.stdin, "{response}").expect("write approval");
                self.stdin.flush().expect("flush approval");
                continue;
            }
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return (msg, notifications);
            }
            if msg.get("method").is_some() {
                notifications.push(msg);
            }
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Block until `fake_mcp` records that it received the `tools/call`, so a
/// cancel that follows is genuinely mid-flight.
fn wait_for_call_received(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "MCP server never received the tool call: {}",
        path.display()
    );
}

fn new_session(
    h: &mut Harness,
    cwd: &std::path::Path,
    cancel_log: &std::path::Path,
    call_received: &std::path::Path,
) -> String {
    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({
            "cwd": cwd.to_str().unwrap(),
            "mcpServers": [{
                "name": "devtools",
                "command": env!("CARGO_BIN_EXE_fake-mcp"),
                "args": [],
                "env": [
                    // Tool call blocks ~forever, so cancel lands mid-flight.
                    { "name": "FAKE_MCP_TOOL_DELAY", "value": "999" },
                    { "name": "FAKE_MCP_CANCEL_LOG", "value": cancel_log.to_str().unwrap() },
                    { "name": "FAKE_MCP_CALL_RECEIVED", "value": call_received.to_str().unwrap() },
                ],
            }],
        }),
    );
    let (resp, _) = h.await_response(id);
    resp["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {resp}"))
        .to_string()
}

#[test]
fn hung_tool_respects_the_configured_timeout() {
    let base_url = spawn_hanging_tool_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let logs = tempfile::tempdir().expect("logs");
    let cancel_log = logs.path().join("cancelled.jsonl");
    let call_received = logs.path().join("call-received");

    let mut h = Harness::start_with_env(
        &base_url,
        home.path(),
        &[("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "1")],
    );
    let session_id = new_session(&mut h, cwd.path(), &cancel_log, &call_received);

    let started = Instant::now();
    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "run the slow tool" }],
        }),
    );
    let (resp, notifications) = h.await_response(id);

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "configured one-second tool timeout did not bound the turn"
    );
    assert_eq!(resp["result"]["stopReason"], "end_turn", "{resp}");
    assert!(
        notifications.iter().any(|notification| {
            notification["params"]["update"]["sessionUpdate"] == "tool_call_update"
                && notification["params"]["update"]["status"] == "failed"
        }),
        "timed-out tool never reached a failed terminal state: {notifications:?}"
    );
}

#[test]
fn cancel_mid_tool_call_returns_cancelled() {
    let base_url = spawn_hanging_tool_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let logs = tempfile::tempdir().expect("logs");
    let cancel_log = logs.path().join("cancelled.jsonl");
    let call_received = logs.path().join("call-received");

    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path(), &cancel_log, &call_received);

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "run the slow tool" }],
        }),
    );

    // Let the turn reach the hanging tool, then cancel. Authorization has to
    // be answered first, or the cancel resolves the permission wait and the
    // tool is never dispatched at all.
    h.approve_next_tool_call();
    wait_for_call_received(&call_received);
    h.notify("session/cancel", json!({ "sessionId": session_id }));

    let started = Instant::now();
    let (resp, notifications) = h.await_response(id);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(30),
        "cancel took {elapsed:?}; the tool sleeps 999s, so this did not unblock"
    );
    assert_eq!(
        resp["result"]["stopReason"], "cancelled",
        "expected stopReason=cancelled: {resp}"
    );

    // Every announced tool call must reach a terminal state, or the desktop
    // spins forever (buzz-agent agent.rs:470-477).
    let announced: Vec<String> = notifications
        .iter()
        .filter(|n| n["params"]["update"]["sessionUpdate"] == "tool_call")
        .filter_map(|n| {
            n["params"]["update"]["toolCallId"]
                .as_str()
                .map(String::from)
        })
        .collect();
    let terminal: Vec<String> = notifications
        .iter()
        .filter(|n| n["params"]["update"]["sessionUpdate"] == "tool_call_update")
        .filter_map(|n| {
            n["params"]["update"]["toolCallId"]
                .as_str()
                .map(String::from)
        })
        .collect();

    for id in &announced {
        assert!(
            terminal.contains(id),
            "tool call {id} was announced but never resolved — UI would hang.\n\
             announced={announced:?} terminal={terminal:?}"
        );
    }

    // activeRunId must be cleared, or a later steer targets a dead run.
    let cleared = notifications.iter().any(|n| {
        n["params"]["update"]["sessionUpdate"] == "session_info_update"
            && n["params"]["update"]["_meta"]["goose"]["activeRunId"].is_null()
    });
    assert!(cleared, "activeRunId was never cleared after cancel");
}

#[test]
fn cancel_propagates_notifications_cancelled_to_mcp() {
    let base_url = spawn_hanging_tool_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let logs = tempfile::tempdir().expect("logs");
    let cancel_log = logs.path().join("cancelled.jsonl");
    let call_received = logs.path().join("call-received");

    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path(), &cancel_log, &call_received);

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "run the slow tool" }],
        }),
    );

    h.approve_next_tool_call();
    wait_for_call_received(&call_received);
    h.notify("session/cancel", json!({ "sessionId": session_id }));
    let _ = h.await_response(id);

    // Goose should tell the MCP server to stop (mcp_client.rs:687-690).
    // Give it a moment to flush.
    let mut saw = false;
    for _ in 0..20 {
        if let Ok(contents) = std::fs::read_to_string(&cancel_log) {
            if contents.contains("notifications/cancelled") {
                saw = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        saw,
        "MCP server never received notifications/cancelled — a hung tool would \
         keep running after the turn ended"
    );
}

/// Cancellation must interrupt stream *construction*, not just consumption.
///
/// Goose's `Provider::stream()` awaits the HTTP POST — response headers
/// included — before returning the stream, so a queued or wedged gateway
/// stalls the turn before there is any stream to select over. The loop must
/// cover that await with the same cancellation/keepalive select it uses for
/// chunks, or `session/cancel` is deferred until headers arrive or the
/// provider timeout expires. This is the pre-headers stall the between-chunks
/// tests cannot reach.
#[test]
fn cancel_during_header_stall_returns_promptly() {
    let base_url = spawn_header_stalling_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");

    // A long provider timeout, so a prompt response inside the deadline can
    // only come from cancellation — not from the request timing out.
    let mut h = Harness::start_with_env(
        &base_url,
        home.path(),
        &[("BUZZ_AGENT_LLM_TIMEOUT_SECS", "300")],
    );

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);
    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": [] }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {resp}"))
        .to_string();

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "say hello" }],
        }),
    );

    // Let the completion POST reach the provider and park pre-headers. There
    // is no ack channel from the stalling provider, so this is a generous
    // fixed wait; too short only makes the test weaker, never flaky-failing.
    std::thread::sleep(Duration::from_secs(2));
    h.notify("session/cancel", json!({ "sessionId": session_id }));

    let started = Instant::now();
    let (resp, _) = h.await_response(id);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "cancel during a pre-headers provider stall took {elapsed:?}; \
         stream construction is not covered by the cancellation select"
    );
    assert_eq!(
        resp["result"]["stopReason"], "cancelled",
        "expected stopReason=cancelled: {resp}"
    );
}

#[test]
fn cancel_for_unknown_session_is_ignored() {
    let base_url = spawn_hanging_tool_provider();
    let home = tempfile::tempdir().expect("home");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    // Must not crash the agent: it is a notification, so there is nothing to
    // reply to, and buzz-acp would hang if the process died here.
    h.notify("session/cancel", json!({ "sessionId": "ses_nope" }));

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let (resp, _) = h.await_response(id);
    assert_eq!(resp["result"]["agentInfo"]["name"], "buzz-agent");
}
