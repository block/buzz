//! Proves turn-to-turn memory: prompt 2 must see what happened in prompt 1.
//!
//! This is the riskiest property on this branch and the one every other test
//! misses. buzz-agent no longer writes conversation history to goose's sqlite
//! store -- `TurnState` holds it in memory and buzz carries it across prompts
//! itself. Every other integration test sends a *single* `session/prompt`, so
//! all of them would still pass if the history were dropped between turns and
//! each prompt started from nothing.
//!
//! What is asserted is what the *provider* receives, not what the agent
//! reports: the second request body must contain the first turn's user text
//! and the first turn's assistant reply. A test that only checked the second
//! answer would pass against a model that ignored history entirely.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

mod approve;

/// Fake OpenAI-compatible provider that forwards every request's `messages`
/// array, so the test can inspect exactly what history the model was shown.
fn spawn_history_capturing_provider() -> (String, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();

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
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(v) = trimmed.to_lowercase().strip_prefix("content-length: ") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }

            let mut body = vec![0u8; content_length];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            if request_line.contains("/models") {
                let payload = json!({
                    "object": "list",
                    "data": [{ "id": "fake-model", "object": "model" }],
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                continue;
            }

            // A fixed, distinctive reply. An earlier version of this test
            // varied the answer by whether the request mentioned the secret,
            // which was useless: turn 1's own user message mentions it, so
            // both turns got the same answer and the assertion proved nothing.
            let answer = "noted-and-filed";
            if let Ok(req) = serde_json::from_slice::<Value>(&body) {
                let _ = tx.send(req["messages"].clone());
            }

            let chunk = |delta: Value, finish: Value| {
                let c = json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fake-model",
                    "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
                });
                format!("data: {c}\n\n")
            };

            let body = format!(
                "{}{}data: [DONE]\n\n",
                chunk(
                    json!({ "role": "assistant", "content": answer }),
                    Value::Null
                ),
                chunk(json!({}), json!("stop")),
            );

            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
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
        let exe = env!("CARGO_BIN_EXE_buzz-agent");
        let mut child = Command::new(exe)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "test-key")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_MAX_ROUNDS", "2")
            .env("GOOSE_DISABLE_KEYRING", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn buzz-agent");

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
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{line}").expect("write");
        self.stdin.flush().expect("flush");
        id
    }

    /// Read until the response with `id` arrives, discarding notifications.
    fn await_response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert!(n > 0, "agent closed stdout before responding to {id}");
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            // Answer the authorization gate so the tool under test actually
            // runs. This suite's subject is not the permission boundary (see
            // `permission_boundary.rs`), so approval is automatic.
            if approve::is_permission_request(&value) {
                let response = approve::approve(&value);
                writeln!(self.stdin, "{response}").expect("write approval");
                self.stdin.flush().expect("flush approval");
                continue;
            }
            if value["id"] == json!(id) {
                return value;
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
fn a_second_prompt_sees_the_first_turn() {
    let (base_url, messages_rx) = spawn_history_capturing_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request(
        "initialize",
        json!({ "protocolVersion": 2, "clientCapabilities": {} }),
    );
    h.await_response(id);

    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": [] }),
    );
    let session_id = h.await_response(id)["result"]["sessionId"]
        .as_str()
        .expect("sessionId")
        .to_string();

    // Turn 1: tell it something only this conversation knows.
    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "Remember the secret word: ZUCCHINI" }],
        }),
    );
    assert!(
        h.await_response(id)["result"]["stopReason"].is_string(),
        "turn 1 did not complete"
    );
    let first = messages_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("provider never saw turn 1");
    assert_eq!(
        first.as_array().map(Vec::len),
        Some(2),
        "turn 1 should be system + the user prompt, got: {first:#?}"
    );

    // Turn 2: a separate session/prompt on the SAME session.
    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "What was the secret word?" }],
        }),
    );
    assert!(
        h.await_response(id)["result"]["stopReason"].is_string(),
        "turn 2 did not complete"
    );
    let second = messages_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("provider never saw turn 2");

    let rendered = serde_json::to_string(&second).expect("serialize");

    // The user's turn-1 message must still be there. Without it the model has
    // no way to answer, and every single-prompt test in this suite would still
    // pass -- which is exactly why this test exists.
    assert!(
        rendered.contains("ZUCCHINI"),
        "turn 2 lost turn 1's user message; history is not carried across \
         prompts. messages were:\n{second:#?}"
    );

    // And turn 1's *assistant* reply, so the model sees a conversation rather
    // than two disconnected questions. Dropping only this half is the subtler
    // failure: the model still answers, just without knowing what it said.
    assert!(
        rendered.contains("noted-and-filed"),
        "turn 2 lost turn 1's assistant reply. messages were:\n{second:#?}"
    );

    assert!(
        second.as_array().map(Vec::len).unwrap_or(0) >= 4,
        "expected system + 2 user + 1 assistant at minimum, got: {second:#?}"
    );
}
