//! Proves the `_Stop` veto: the turn must not end while the hook objects.
//!
//! This is buzz-agent's most load-bearing non-standard behaviour — it is why
//! the agent finishes its todo list instead of stopping halfway. Goose has the
//! same mechanism internally, but its `hook_manager` is private and its hooks
//! are subprocesses that cannot see buzz-dev-mcp's in-process todo state, so
//! we drive it from our own loop (see `src/hooks.rs`).
//!
//! Setup: a fake MCP server exposing `_Stop`, plus a fake provider that counts
//! how many times it is asked to generate. One user prompt with a hook that
//! objects twice must produce three provider calls, not one.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

/// Fake OpenAI-compatible provider that always answers "done" and counts calls.
fn spawn_counting_provider() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
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

            if request_line.contains("/models") {
                let payload = json!({ "object": "list", "data": [] }).to_string();
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        payload.len(), payload
                    ).as_bytes(),
                );
                let _ = stream.flush();
                continue;
            }

            counter.fetch_add(1, Ordering::SeqCst);

            let chunk = |delta: Value, finish: Value, usage: Value| {
                let mut c = json!({
                    "id": "c", "object": "chat.completion.chunk", "created": 1,
                    "model": "fake-model",
                    "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
                });
                if !usage.is_null() {
                    c["usage"] = usage;
                }
                format!("data: {c}\n\n")
            };
            let sse = format!(
                "{}{}data: [DONE]\n\n",
                chunk(
                    json!({ "role": "assistant", "content": "done" }),
                    Value::Null,
                    Value::Null
                ),
                chunk(
                    json!({}),
                    json!("stop"),
                    json!({ "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 })
                ),
            );
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    sse.len(), sse
                ).as_bytes(),
            );
            let _ = stream.flush();
        }
    });

    (format!("http://{addr}"), calls)
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

    fn await_response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout awaiting id={id}");
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_i64) == Some(id) {
                return msg;
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

/// Declare the fake MCP server, with `_Stop` objecting `objections` times.
fn mcp_servers(objections: usize) -> Value {
    json!([{
        "name": "devtools",
        "command": env!("CARGO_BIN_EXE_fake-mcp"),
        "args": [],
        "env": [
            { "name": "FAKE_MCP_STOP_HOOK", "value": "1" },
            { "name": "FAKE_MCP_STOP_TEXT", "value": "1 item still open" },
            { "name": "FAKE_MCP_STOP_COUNT", "value": objections.to_string() },
        ],
    }])
}

fn new_session(h: &mut Harness, cwd: &std::path::Path, objections: usize) -> String {
    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({
            "cwd": cwd.to_str().unwrap(),
            "mcpServers": mcp_servers(objections),
        }),
    );
    let resp = h.await_response(id);
    resp["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {resp}"))
        .to_string()
}

#[test]
fn stop_hook_veto_extends_the_turn() {
    let (base_url, calls) = spawn_counting_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    // Object twice, then allow.
    let session_id = new_session(&mut h, cwd.path(), 2);

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "do the work" }],
        }),
    );
    let resp = h.await_response(id);

    assert_eq!(
        resp["result"]["stopReason"], "end_turn",
        "turn should still end cleanly once the hook stops objecting: {resp}"
    );

    // The model says "done" immediately, so without the veto this is 1.
    // Two objections must force two more generations.
    let n = calls.load(Ordering::SeqCst);
    assert_eq!(
        n, 3,
        "expected 3 provider calls (initial + 2 vetoes), got {n} — the _Stop veto did not fire"
    );
}

#[test]
fn stop_hook_veto_is_capped() {
    let (base_url, calls) = spawn_counting_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    // Always objects. Must not trap the turn forever.
    let session_id = new_session(&mut h, cwd.path(), usize::MAX);

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "do the work" }],
        }),
    );
    let resp = h.await_response(id);

    assert_eq!(
        resp["result"]["stopReason"], "end_turn",
        "a permanently objecting hook must not hang the turn: {resp}"
    );

    // MAX_STOP_BLOCKS = 3, so: initial + 3 vetoes = 4, then give up.
    let n = calls.load(Ordering::SeqCst);
    assert_eq!(
        n, 4,
        "expected the veto to stop at MAX_STOP_BLOCKS (1 + 3 = 4 calls), got {n}"
    );
}

#[test]
fn no_stop_hook_means_no_extra_rounds() {
    let (base_url, calls) = spawn_counting_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    // No MCP servers at all: nothing to veto with.
    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": [] }),
    );
    let resp = h.await_response(id);
    let session_id = resp["result"]["sessionId"].as_str().unwrap().to_string();

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "hi" }],
        }),
    );
    let resp = h.await_response(id);
    assert_eq!(resp["result"]["stopReason"], "end_turn");

    let n = calls.load(Ordering::SeqCst);
    assert_eq!(n, 1, "no hook should mean exactly one generation, got {n}");
}

#[test]
fn stop_hook_tools_are_discovered_by_suffix() {
    // The extension name comes from buzz-acp's MCP binary file stem
    // (`buzz-acp/src/lib.rs:4145`), so discovery must not hardcode a name.
    // Here the server is called "devtools", not "developer" or "buzz-dev-mcp".
    let (base_url, calls) = spawn_counting_provider();
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let session_id = new_session(&mut h, cwd.path(), 1);

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "go" }],
        }),
    );
    let resp = h.await_response(id);
    assert_eq!(resp["result"]["stopReason"], "end_turn");

    let n = calls.load(Ordering::SeqCst);
    assert_eq!(
        n, 2,
        "one objection under a non-obvious extension name should yield 2 calls, got {n}"
    );

    let _ = Duration::from_secs(0);
}
