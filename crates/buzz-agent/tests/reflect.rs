//! Proves `[Reflect]`: a failed tool result must nudge the model to diagnose
//! before retrying.
//!
//! buzz-agent appended the text to the tool result itself (`agent.rs:364`).
//! Goose gives no interception point for that — `PostToolUseFailure` is
//! fire-and-forget and its output is discarded (`agent.rs:589-620`) — so we
//! deliver the same text via `steer()`, which goose drains at the round
//! boundary.
//!
//! Observable consequence: the reflection must appear in the *next* request
//! the provider receives. That is what this asserts, rather than trusting that
//! `steer()` was called.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// Fake provider: first generation emits a tool call, later ones answer "done".
/// Every request body is forwarded so the test can inspect what the model saw.
fn spawn_tool_calling_provider(tool_name: &str) -> (String, mpsc::Receiver<Value>) {
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
                // Ask for a tool the fake MCP server will fail.
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
                        "delta": { "role": "assistant", "content": "done" },
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
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn failing_mcp() -> Value {
    json!([{
        "name": "devtools",
        "command": env!("CARGO_BIN_EXE_fake-mcp"),
        "args": [],
        "env": [{ "name": "FAKE_MCP_TOOL_ERROR", "value": "1" }],
    }])
}

/// Concatenate every message body in a captured chat-completions request.
fn all_text(req: &Value) -> String {
    req["messages"]
        .as_array()
        .map(|msgs| {
            msgs.iter()
                .map(|m| m.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn failed_tool_result_injects_a_reflection() {
    let (base_url, requests) = spawn_tool_calling_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": failing_mcp() }),
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
            "prompt": [{ "type": "text", "text": "use the tool" }],
        }),
    );
    let (resp, notifications) = h.await_response(id);
    assert!(
        resp["result"]["stopReason"].is_string(),
        "turn failed: {resp}"
    );

    // The tool must actually have been reported as failed on the wire, or the
    // rest of this test proves nothing.
    let failed_update = notifications.iter().any(|n| {
        n["params"]["update"]["sessionUpdate"] == "tool_call_update"
            && n["params"]["update"]["status"] == "failed"
    });
    assert!(
        failed_update,
        "expected a failed tool_call_update; saw {:#?}",
        notifications
            .iter()
            .map(|n| n["params"]["update"]["sessionUpdate"].clone())
            .collect::<Vec<_>>()
    );

    // Collect what the provider saw. First request is the original prompt;
    // the reflection must appear in a later one.
    let mut seen = Vec::new();
    while let Ok(req) = requests.recv_timeout(Duration::from_secs(5)) {
        seen.push(req);
    }
    assert!(
        seen.len() >= 2,
        "expected a follow-up generation after the tool call, got {}",
        seen.len()
    );

    assert!(
        !all_text(&seen[0]).contains("[Reflect]"),
        "reflection must not appear before the tool has failed"
    );

    let later = seen[1..]
        .iter()
        .map(all_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        later.contains("[Reflect]"),
        "no [Reflect] nudge reached the model after a failed tool call.\nsaw:\n{later}"
    );
}

#[test]
fn successful_tool_result_injects_nothing() {
    let (base_url, requests) = spawn_tool_calling_provider("devtools__tool_0");
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    // Same flow, but the tool succeeds.
    let servers = json!([{
        "name": "devtools",
        "command": env!("CARGO_BIN_EXE_fake-mcp"),
        "args": [],
        "env": [],
    }]);
    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": servers }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"].as_str().unwrap().to_string();

    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "use the tool" }],
        }),
    );
    let (resp, _) = h.await_response(id);
    assert!(resp["result"]["stopReason"].is_string());

    let mut seen = Vec::new();
    while let Ok(req) = requests.recv_timeout(Duration::from_secs(5)) {
        seen.push(req);
    }
    let all = seen.iter().map(all_text).collect::<Vec<_>>().join("\n");
    assert!(
        !all.contains("[Reflect]"),
        "a successful tool call must not trigger a reflection"
    );
}
