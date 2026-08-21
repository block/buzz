//! `_goose/unstable/session/steer` — the last ACP method without coverage.
//!
//! Steering injects a message into a turn *without* cancelling it. buzz-acp
//! prefers this over cancel+re-prompt because the latter loses the model's
//! in-progress work, and it guards the call with optimistic concurrency:
//!
//! * `activeRunId` is advertised at `params.update._meta.goose.activeRunId`
//!   — `_meta` nests INSIDE `update` (`buzz-acp/src/acp.rs:1607-1613`).
//! * A steer carries `expectedRunId`. Mismatch or no active run must be an
//!   error, so the harness can fall back to cancel+merge
//!   (`buzz-acp/src/pool.rs:329-366`).
//!
//! Goose's own `steer()` has the same semantics as buzz-agent's: the message
//! is queued and drained at the round boundary (`agent.rs:1951-1974`), and a
//! pending steer prevents the turn from ending (`agent.rs:2876-2878`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// Provider whose first generation calls a slow tool (giving the test a window
/// to steer), and whose later generations answer with text.
///
/// Every request body is forwarded so the test can prove the steered message
/// actually reached the model.
fn spawn_provider(tool_name: &str) -> (String, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let tool_name = tool_name.to_string();

    std::thread::spawn(move || {
        let mut generation = 0usize;
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

            if let Ok(req) = serde_json::from_slice::<Value>(&body) {
                let _ = tx.send(req);
            }
            generation += 1;

            let usage = json!({ "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 });
            let sse = if generation == 1 {
                let call = json!({
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
                                "function": { "name": tool_name, "arguments": "{}" },
                            }],
                        },
                        "finish_reason": null,
                    }],
                });
                let done = json!({
                    "id": "c", "object": "chat.completion.chunk", "created": 1,
                    "model": "fake-model",
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }],
                    "usage": usage,
                });
                format!("data: {call}\n\ndata: {done}\n\ndata: [DONE]\n\n")
            } else {
                let text = json!({
                    "id": "c", "object": "chat.completion.chunk", "created": 1,
                    "model": "fake-model",
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": "ok" },
                        "finish_reason": null,
                    }],
                });
                let done = json!({
                    "id": "c", "object": "chat.completion.chunk", "created": 1,
                    "model": "fake-model",
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                    "usage": usage,
                });
                format!("data: {text}\n\ndata: {done}\n\ndata: [DONE]\n\n")
            };

            respond(&mut stream, "text/event-stream", sse);
        }
    });

    (format!("http://{addr}"), rx)
}

struct Harness {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    fn start(base_url: &str, home: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_buzz-agent"))
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
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn");
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

    fn await_response(&mut self, id: i64) -> (Value, Vec<Value>) {
        let mut notifications = Vec::new();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout awaiting id={id}");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return (msg, notifications);
            }
            if msg.get("method").is_some() {
                notifications.push(msg);
            }
        }
    }

    /// Read notifications until `activeRunId` is advertised, returning it.
    ///
    /// Also proves the nesting depth: buzz-acp reads
    /// `params.update._meta.goose.activeRunId`, and a `_meta` placed one level
    /// too high silently degrades steering to cancel+re-prompt forever.
    fn await_active_run_id(&mut self) -> String {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout before advertising activeRunId");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(run_id) = msg["params"]["update"]["_meta"]["goose"]["activeRunId"].as_str()
            {
                return run_id.to_string();
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

fn new_session(h: &mut Harness, cwd: &std::path::Path) -> String {
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
                // Slow enough that a steer lands mid-turn, short enough that
                // the turn still completes on its own.
                "env": [{ "name": "FAKE_MCP_TOOL_DELAY", "value": "3" }],
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
fn steer_injects_without_cancelling_the_turn() {
    let (base_url, requests) = spawn_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path());

    let prompt_id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "start working" }],
        }),
    );

    let run_id = h.await_active_run_id();
    assert!(run_id.starts_with("run_"), "unexpected runId: {run_id}");

    let steer_id = h.request(
        "_goose/unstable/session/steer",
        json!({
            "sessionId": session_id,
            "expectedRunId": run_id,
            "prompt": [{ "type": "text", "text": "ALSO check the logs" }],
        }),
    );
    let (steer_resp, _) = h.await_response(steer_id);
    assert!(
        steer_resp["error"].is_null(),
        "steer was rejected: {steer_resp}"
    );
    assert_eq!(steer_resp["result"]["runId"], run_id);

    // The turn must complete normally — steering is NOT a cancel.
    let (resp, _) = h.await_response(prompt_id);
    assert_eq!(
        resp["result"]["stopReason"], "end_turn",
        "steering must not cancel the turn: {resp}"
    );

    // And the steered text must have reached the model.
    let mut seen = Vec::new();
    while let Ok(req) = requests.recv_timeout(Duration::from_secs(5)) {
        seen.push(req);
    }
    let all = seen
        .iter()
        .map(|r| r["messages"].to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("ALSO check the logs"),
        "steered message never reached the model.\nsaw:\n{all}"
    );
}

#[test]
fn steer_with_stale_run_id_is_rejected() {
    let (base_url, _requests) = spawn_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path());

    let prompt_id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "start working" }],
        }),
    );
    let _run_id = h.await_active_run_id();

    // Optimistic concurrency: a stale id must fail so buzz-acp can fall back
    // to cancel+merge rather than silently steering the wrong turn.
    let steer_id = h.request(
        "_goose/unstable/session/steer",
        json!({
            "sessionId": session_id,
            "expectedRunId": "run_stale",
            "prompt": [{ "type": "text", "text": "too late" }],
        }),
    );
    let (steer_resp, _) = h.await_response(steer_id);
    assert_eq!(
        steer_resp["error"]["code"], -32602,
        "stale runId must be rejected: {steer_resp}"
    );

    let (resp, _) = h.await_response(prompt_id);
    assert!(resp["result"]["stopReason"].is_string());
}

#[test]
fn steer_outside_a_turn_is_rejected() {
    let (base_url, _requests) = spawn_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path());

    // No prompt in flight: there is nothing to steer.
    let steer_id = h.request(
        "_goose/unstable/session/steer",
        json!({
            "sessionId": session_id,
            "expectedRunId": "run_anything",
            "prompt": [{ "type": "text", "text": "hello?" }],
        }),
    );
    let (resp, _) = h.await_response(steer_id);
    assert_eq!(
        resp["error"]["code"], -32602,
        "steer with no active run must be an error: {resp}"
    );
}

#[test]
fn active_run_id_is_cleared_when_the_turn_ends() {
    let (base_url, _requests) = spawn_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());
    let session_id = new_session(&mut h, cwd.path());

    let prompt_id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "go" }],
        }),
    );
    let (_resp, notifications) = h.await_response(prompt_id);

    // A trailing null clears it; without this a later steer would target a
    // dead run and the harness would never fall back.
    let cleared = notifications.iter().any(|n| {
        n["params"]["update"]["sessionUpdate"] == "session_info_update"
            && n["params"]["update"]["_meta"]["goose"]["activeRunId"].is_null()
            && n["params"]["update"]["_meta"]["goose"]
                .as_object()
                .is_some_and(|m| m.contains_key("activeRunId"))
    });
    assert!(
        cleared,
        "activeRunId was never explicitly cleared after the turn"
    );

    // And a steer afterwards must now be rejected.
    let steer_id = h.request(
        "_goose/unstable/session/steer",
        json!({
            "sessionId": session_id,
            "expectedRunId": "run_whatever",
            "prompt": [{ "type": "text", "text": "late" }],
        }),
    );
    let (resp, _) = h.await_response(steer_id);
    assert_eq!(resp["error"]["code"], -32602);
}
