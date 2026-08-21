//! End-to-end stdio test: drive the binary over ACP exactly as `buzz-acp`
//! does, against a fake OpenAI-compatible provider.
//!
//! This is the test that proves the integration, not just the compile:
//!
//! 1. `initialize` negotiates and self-identifies as `buzz-agent`.
//! 2. `session/new` accepts a `systemPrompt` (Fizz) and MCP servers.
//! 3. **The persona actually reaches the model** — the fake provider asserts
//!    Fizz's text is present in the `system` field of the request body. This
//!    is the whole reason for driving goose as a library: goose's own ACP
//!    server never reads `systemPrompt` (zero hits in goose/crates/goose/src),
//!    so over plain ACP the persona is silently dropped.
//! 4. `session/prompt` runs a turn and returns a `stopReason`.
//! 5. `usage_update` arrives on the `_goose/unstable/session/update` channel
//!    BEFORE the `session/prompt` response — ordering that kind-44200 metrics
//!    depend on (`buzz-acp/src/lib.rs:701-706`).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

const FIZZ_PROMPT: &str = "You are Fizz, an energetic maker who turns ideas into action. Be upbeat, practical, and decisive.";

/// Minimal OpenAI-compatible `/chat/completions` server.
///
/// Returns the captured `system` prompt of the first request over a channel so
/// the test can assert the persona survived the trip.
fn spawn_fake_provider() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let tx = tx.clone();

            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }

            // Read headers, find content-length.
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
                if let Some(v) = trimmed.strip_prefix("content-length: ") {
                    content_length = v.parse().unwrap_or(0);
                } else if let Some(v) = trimmed.strip_prefix("Content-Length: ") {
                    content_length = v.parse().unwrap_or(0);
                }
            }

            let mut body = vec![0u8; content_length];
            use std::io::Read;
            let _ = reader.read_exact(&mut body);

            // Models endpoint: return a tiny catalog.
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

            // Chat completions: capture the system prompt, then answer.
            if let Ok(req) = serde_json::from_slice::<Value>(&body) {
                let system = req["messages"]
                    .as_array()
                    .and_then(|msgs| {
                        msgs.iter()
                            .find(|m| m["role"] == "system")
                            .and_then(|m| m["content"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                let _ = tx.send(system);
            }

            // Goose sets `stream: true` on the OpenAI provider
            // (goose-providers/src/openai.rs:650), so answer with SSE. A
            // non-streaming JSON body parses as an empty response and the turn
            // ends with zero tokens — which correctly suppresses
            // `usage_update`, but proves nothing.
            let chunk = |delta: Value, finish: Value, usage: Value| {
                let mut c = json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "fake-model",
                    "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
                });
                if !usage.is_null() {
                    c["usage"] = usage;
                }
                format!("data: {c}\n\n")
            };

            let body = format!(
                "{}{}{}data: [DONE]\n\n",
                chunk(
                    json!({ "role": "assistant", "content": "Buzzing along!" }),
                    Value::Null,
                    Value::Null
                ),
                chunk(json!({ "content": " Done. 🐝" }), Value::Null, Value::Null),
                chunk(
                    json!({}),
                    json!("stop"),
                    json!({ "prompt_tokens": 42, "completion_tokens": 7, "total_tokens": 49 })
                ),
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
            // Buzz-shaped config; config.rs translates these onto GOOSE_*.
            .env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "test-key")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_MAX_ROUNDS", "2")
            // Isolate goose's config/session state from the developer's real one.
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn buzz-agent-core");

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
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{msg}").expect("write");
        self.stdin.flush().expect("flush");
        id
    }

    /// Read lines until the response with `id` arrives, collecting every
    /// notification seen along the way (so ordering can be asserted).
    fn await_response(&mut self, id: i64) -> (Value, Vec<Value>) {
        let mut notifications = Vec::new();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read");
            assert_ne!(n, 0, "agent closed stdout before responding to id={id}");
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
fn full_turn_over_stdio_carries_the_persona() {
    let (base_url, system_rx) = spawn_fake_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    // 1. initialize
    let id = h.request(
        "initialize",
        json!({
            "protocolVersion": 2,
            "clientCapabilities": { "_meta": { "goose": { "customNotifications": true } } },
        }),
    );
    let (resp, _) = h.await_response(id);
    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], 2, "negotiates min(client, 2)");
    assert_eq!(
        result["agentInfo"]["name"], "buzz-agent",
        "identity must stay buzz-agent — kind-44200 `harness` attribution keys off this"
    );

    // 2. session/new carrying Fizz
    let id = h.request(
        "session/new",
        json!({
            "cwd": cwd.path().to_str().unwrap(),
            "mcpServers": [],
            "systemPrompt": FIZZ_PROMPT,
        }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"]
        .as_str()
        .expect("sessionId in session/new result")
        .to_string();
    assert!(session_id.starts_with("ses_"));

    // 3. session/prompt
    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "say hello" }],
        }),
    );
    let (resp, notifications) = h.await_response(id);

    assert!(
        resp["result"]["stopReason"].is_string(),
        "buzz-acp errors without stopReason: got {resp}"
    );

    // 4. THE point of the whole exercise: the persona reached the provider.
    let captured = system_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("provider never received a chat completion request");
    assert!(
        captured.contains("You are Fizz"),
        "persona did not reach the model. system prompt was:\n{captured}"
    );

    // 5. activeRunId advertised with _meta nested INSIDE update.
    let run_id_update = notifications.iter().find(|n| {
        n["method"] == "session/update"
            && n["params"]["update"]["sessionUpdate"] == "session_info_update"
            && n["params"]["update"]["_meta"]["goose"]["activeRunId"].is_string()
    });
    assert!(
        run_id_update.is_some(),
        "no activeRunId advertised — steering silently degrades to cancel+re-prompt. saw: {:#?}",
        notifications
            .iter()
            .map(|n| n["method"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
    );

    // 6. usage_update present, on the goose-namespaced channel.
    let usage_idx = notifications.iter().position(|n| {
        n["method"] == "_goose/unstable/session/update"
            && n["params"]["update"]["sessionUpdate"] == "usage_update"
    });
    let usage_idx =
        usage_idx.expect("no usage_update — kind-44200 turn metrics would silently vanish");

    let usage = &notifications[usage_idx]["params"]["update"];
    assert!(
        usage["accumulatedInputTokens"].as_u64().unwrap_or(0) > 0,
        "expected non-zero accumulated input tokens, got {usage}"
    );

    // Ordering is implicitly proven: `notifications` only contains messages
    // seen strictly BEFORE the session/prompt response was read.
}

#[test]
fn session_new_advertises_the_model_catalog() {
    let (base_url, _rx) = spawn_fake_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": [] }),
    );
    let (resp, _) = h.await_response(id);
    let result = &resp["result"];

    // buzz-acp reads `models.availableModels` to drive the desktop
    // ModelPicker and to resolve session/set_model targets
    // (`buzz-acp/src/acp.rs:1866`, `:1900`). Without it the picker degrades
    // to "current model only".
    let models = &result["models"];
    assert!(
        models.is_object(),
        "session/new must advertise a model catalog, got: {result}"
    );
    assert_eq!(models["currentModelId"], "fake-model");

    let available = models["availableModels"]
        .as_array()
        .expect("availableModels array");
    assert!(!available.is_empty());
    // Key must be `modelId` — that is what resolve_model_switch_method matches.
    assert!(
        available.iter().any(|m| m["modelId"] == "fake-model"),
        "current model must appear in availableModels, got: {available:#?}"
    );
    assert!(
        available.iter().all(|m| m["modelId"].is_string()),
        "every entry needs a modelId key"
    );
}

#[test]
fn set_model_applies_from_the_next_prompt() {
    let (base_url, system_rx) = spawn_fake_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let cwd = tempfile::tempdir().expect("cwd");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("initialize", json!({ "protocolVersion": 2 }));
    let _ = h.await_response(id);

    let id = h.request(
        "session/new",
        json!({ "cwd": cwd.path().to_str().unwrap(), "mcpServers": [] }),
    );
    let (resp, _) = h.await_response(id);
    let session_id = resp["result"]["sessionId"].as_str().unwrap().to_string();

    // Unknown session and empty modelId are both invalid_params.
    let id = h.request(
        "session/set_model",
        json!({ "sessionId": "ses_nope", "modelId": "x" }),
    );
    let (resp, _) = h.await_response(id);
    assert_eq!(resp["error"]["code"], -32602, "unknown session");

    let id = h.request(
        "session/set_model",
        json!({ "sessionId": session_id, "modelId": "  " }),
    );
    let (resp, _) = h.await_response(id);
    assert_eq!(resp["error"]["code"], -32602, "empty modelId");

    // A real switch is accepted...
    let id = h.request(
        "session/set_model",
        json!({ "sessionId": session_id, "modelId": "fake-model-2" }),
    );
    let (resp, _) = h.await_response(id);
    assert!(resp["error"].is_null(), "set_model rejected: {resp}");

    // ...and the next turn still completes (the override is applied at prompt
    // time, not mid-flight).
    let id = h.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": "hi" }],
        }),
    );
    let (resp, _) = h.await_response(id);
    assert!(
        resp["result"]["stopReason"].is_string(),
        "turn failed after set_model: {resp}"
    );

    let _ = system_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("provider never received a request after set_model");
}

#[test]
fn unknown_method_gets_method_not_found() {
    let (base_url, _rx) = spawn_fake_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request("does/not/exist", json!({}));
    let (resp, _) = h.await_response(id);
    assert_eq!(
        resp["error"]["code"], -32601,
        "silence would hang the harness waiting for a response"
    );
}

#[test]
fn session_new_rejects_relative_cwd() {
    let (base_url, _rx) = spawn_fake_provider();
    let home = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::start(&base_url, home.path());

    let id = h.request(
        "session/new",
        json!({ "cwd": "relative/path", "mcpServers": [] }),
    );
    let (resp, _) = h.await_response(id);
    assert_eq!(resp["error"]["code"], -32602);
}
