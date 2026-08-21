//! End-to-end against the **real** `buzz-dev-mcp`, not the fake.
//!
//! Everything else in this crate's test suite uses `fake-mcp`, which answers
//! whatever the test tells it to. That proves our plumbing but not that it
//! matches the server buzz actually ships. This test drives the real binary:
//! real tool names, real schemas, real in-process todo state, real
//! `_Stop`/`_PostCompact` semantics.
//!
//! The scenario is the one buzz-agent's `_Stop` veto exists for: the model
//! records an open todo item and then tries to stop. It must not be allowed
//! to.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::{json, Value};

/// Locate the real `buzz-dev-mcp` binary in the workspace target dir.
///
/// It is a separate binary crate, so `CARGO_BIN_EXE_` is unavailable here and
/// we resolve it by walking up to the workspace target dir.
fn buzz_dev_mcp() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        for profile in ["debug", "release"] {
            let candidate = dir.join("target").join(profile).join("buzz-dev-mcp");
            if candidate.is_file() {
                return candidate;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    // Fail loudly rather than skipping. buzz-dev-mcp is a workspace sibling
    // that `just ci` always builds, so a miss means the search is wrong -- and
    // a silent skip hides exactly the breakage this test exists to catch.
    panic!(
        "buzz-dev-mcp not found searching upward from {}. Run `cargo build -p buzz-dev-mcp`.",
        env!("CARGO_MANIFEST_DIR")
    );
}

/// Provider that drives a scripted conversation:
///   generation 1 → call `todo` to record one open item
///   generation 2+ → plain text (i.e. "I'm done")
///
/// Returns every request body so the test can see what the model was shown.
fn spawn_scripted_provider() -> (String, mpsc::Receiver<Value>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();

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

            if let Ok(req) = serde_json::from_slice::<Value>(&body) {
                let _ = tx.send(req);
            }
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;

            let usage = json!({ "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 });
            let sse = if n == 1 {
                // Record one OPEN todo item via the real `todo` tool.
                let args =
                    json!({ "todos": [{ "text": "finish the work", "done": false }] }).to_string();
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
                                "function": { "name": "devtools__todo", "arguments": args },
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
                        "delta": { "role": "assistant", "content": "All finished!" },
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

    (format!("http://{addr}"), rx, calls)
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
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn real_dev_mcp_stop_hook_blocks_end_of_turn() {
    let mcp = buzz_dev_mcp();

    let (base_url, requests, calls) = spawn_scripted_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({
            "cwd": cwd.path().to_str().unwrap(),
            "mcpServers": [{
                "name": "devtools",
                "command": mcp.to_str().unwrap(),
                "args": [],
                "env": [],
            }],
        }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed against real buzz-dev-mcp: {resp}"))
        .to_string();

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "do the work" }],
        }),
    );
    let (resp, notifications) = h.await_response(id);
    assert_eq!(
        resp["result"]["stopReason"], "end_turn",
        "turn should end cleanly once the veto budget is spent: {resp}"
    );

    // The real `todo` tool must have run.
    let todo_ran = notifications.iter().any(|n| {
        n["params"]["update"]["sessionUpdate"] == "tool_call"
            && n["params"]["update"]["title"]
                .as_str()
                .is_some_and(|t| t.contains("todo"))
    });
    assert!(
        todo_ran,
        "the real `todo` tool never ran: {notifications:#?}"
    );

    let mut seen = Vec::new();
    while let Ok(req) = requests.recv_timeout(Duration::from_secs(5)) {
        seen.push(req);
    }

    // The model tried to stop at generation 2 with an item still open. The
    // real `_Stop` hook must have objected, forcing further generations.
    let n = calls.load(Ordering::SeqCst);
    assert!(
        n >= 3,
        "real _Stop hook did not block end-of-turn: only {n} generations \
         (expected the todo item to force at least one more)"
    );

    // And the objection the real server produces must have reached the model.
    let all = seen
        .iter()
        .map(|r| r["messages"].to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("open todo items"),
        "buzz-dev-mcp's objection text never reached the model.\nsaw:\n{all}"
    );
}

#[test]
fn real_dev_mcp_advertises_its_tools() {
    let mcp = buzz_dev_mcp();

    let (base_url, requests, _) = spawn_scripted_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({
            "cwd": cwd.path().to_str().unwrap(),
            "mcpServers": [{
                "name": "devtools",
                "command": mcp.to_str().unwrap(),
                "args": [],
                "env": [],
            }],
        }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"].as_str().unwrap().to_string();

    let id = h.request(
        "session/prompt",
        json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": "hi" }] }),
    );
    let _ = h.await_response(id);

    let first = requests
        .recv_timeout(Duration::from_secs(10))
        .expect("no provider request");
    let tools = first["tools"].to_string();

    // The shipped tool surface, prefixed by extension name.
    for tool in ["shell", "read_file", "str_replace", "todo"] {
        assert!(
            tools.contains(&format!("devtools__{tool}")),
            "real buzz-dev-mcp tool `{tool}` was not advertised to the model"
        );
    }

    // Known deviation from buzz-agent, asserted so it cannot regress silently:
    // goose's allowlist gates advertising and dispatch together, so the hooks
    // stay visible and we instruct the model not to call them instead.
    assert!(
        tools.contains("devtools___Stop"),
        "expected _Stop to remain visible (see build_agent's KNOWN DEVIATION)"
    );
    let system = first["messages"][0]["content"].as_str().unwrap_or_default();
    assert!(
        system.contains("lifecycle hooks"),
        "hook guidance missing from the system prompt: {system}"
    );
}
