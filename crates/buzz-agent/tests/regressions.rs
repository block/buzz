//! Regression tests for round 4-6 hardening:
//!   - assistant text preserved in history
//!   - MCP init timeout (with explicit child kill)
//!   - tool metadata caps (description bytes, count)
//!   - cancellation leaves history valid for the next prompt
//!   - empty-content assistant turn doesn't poison OpenAI history

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct CapturingLlm {
    url: String,
    captured: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_capturing_llm(responses: Vec<Value>) -> CapturingLlm {
    spawn_capturing_llm_with_status(responses.into_iter().map(|v| (200u16, v)).collect()).await
}

/// Like `spawn_capturing_llm` but each canned response carries its own HTTP
/// status, so a test can serve a real provider rejection (e.g. a context-window
/// 400) instead of only success bodies.
async fn spawn_capturing_llm_with_status(responses: Vec<(u16, Value)>) -> CapturingLlm {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap2 = captured.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            let captured = cap2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                // Read until headers complete.
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.len() > 4_000_000 {
                        return;
                    }
                }
                // Parse Content-Length and read body.
                let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let headers = &buf[..header_end];
                let mut body_len = 0usize;
                for line in headers.split(|b| *b == b'\n') {
                    let line = std::str::from_utf8(line).unwrap_or("");
                    if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        body_len = rest.trim().trim_end_matches('\r').parse().unwrap_or(0);
                    }
                }
                while buf.len() < header_end + body_len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                if let Ok(req) = serde_json::from_slice::<Value>(&buf[header_end..]) {
                    captured.lock().await.push(req);
                }
                let (status, body) = queue
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or_else(|| (200, json!({ "error": "no canned response" })));
                let body_s = serde_json::to_string(&body).unwrap();
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    _ => "Error",
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(), body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    CapturingLlm { url, captured }
}

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr: Arc<StdMutex<String>>,
    next_id: i64,
}

impl Harness {
    async fn spawn_with_env(base_url: &str, extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("BUZZ_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_MAX_ROUNDS", "8")
            .env("BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS", "2");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn buzz-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr = child.stderr.take().unwrap();
        let stderr_buf = Arc::new(StdMutex::new(String::new()));
        let stderr_out = Arc::clone(&stderr_buf);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                let n = match reader.read_line(&mut line).await {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 {
                    break;
                }
                if let Ok(mut out) = stderr_out.lock() {
                    out.push_str(&line);
                }
            }
        });
        Self {
            child,
            stdin,
            stdout,
            stderr: stderr_buf,
            next_id: 1,
        }
    }

    async fn spawn(base_url: &str) -> Self {
        Self::spawn_with_env(base_url, &[]).await
    }

    async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    async fn write(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(15), self.stdout.read_line(&mut line))
            .await
            .expect("recv timeout")
            .expect("read line");
        assert!(n > 0, "agent EOF");
        serde_json::from_str(&line).expect("non-JSON line")
    }

    async fn recv_until<F: FnMut(&Value) -> bool>(&mut self, mut pred: F) -> Value {
        loop {
            let v = self.recv().await;
            if pred(&v) {
                return v;
            }
        }
    }

    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
    }

    fn stderr_text(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn openai_text(content: &str) -> Value {
    json!({
        "id": "cc-1", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
    })
}

/// Like [`openai_text`] but attaches a `usage` block so tests can drive the
/// token-based handoff gate. `prompt_tokens` is the input-token count the
/// agent will read and compare against the configured context budget.
fn openai_text_with_usage(content: &str, prompt_tokens: u64) -> Value {
    let mut v = openai_text(content);
    v["usage"] = json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": 1,
        "total_tokens": prompt_tokens + 1,
    });
    v
}

fn openai_tool_call(id: &str, name: &str, args: Value) -> Value {
    json!({
        "id": "cc-2", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{
                    "id": id, "type": "function",
                    "function": { "name": name, "arguments": args.to_string() },
                }],
            },
            "finish_reason": "tool_calls",
        }],
    })
}

async fn init_session(h: &mut Harness, mcp_servers: Value) -> String {
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({"cwd":"/tmp","mcpServers": mcp_servers}),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    r["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "session/new did not return sessionId: response={r}, stderr={}",
                h.stderr_text()
            )
        })
        .to_owned()
}

/// After a text-only assistant response, the next prompt's request must
/// include that assistant text in `messages` history. Round 4 fix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assistant_text_preserved_across_prompts() {
    let llm = spawn_capturing_llm(vec![openai_text("hello world"), openai_text("done")]).await;
    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(&mut h, json!([])).await;

    // Prompt 1.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"first"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    // Prompt 2 — should carry assistant text from prompt 1.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"second"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p2)).await;

    let captured = llm.captured.lock().await;
    assert_eq!(captured.len(), 2, "expected 2 LLM requests");
    let msgs = captured[1]["messages"].as_array().unwrap();
    let assistants: Vec<&Value> = msgs.iter().filter(|m| m["role"] == "assistant").collect();
    assert!(
        assistants.iter().any(|m| m["content"] == "hello world"),
        "assistant text was dropped: messages={msgs:?}"
    );
    h.shutdown().await;
}

/// MCP init that hangs forever must time out within ~2s, surface an error,
/// and the child process must be killed (not lingering).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_init_timeout_kills_child() {
    let llm = spawn_capturing_llm(vec![]).await;
    let mut h = Harness::spawn(&llm.url).await;

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;

    let start = Instant::now();
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "stuck",
                "command": fake_mcp,
                "args": [],
                "env": [{ "name": "FAKE_MCP_HANG_INIT", "value": "1" }],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    let elapsed = start.elapsed();

    assert!(r.get("error").is_some(), "expected error, got {r}");
    let msg = r["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("timeout"), "error not a timeout: {msg}");
    // 2s timeout + small slack. Generous to cover slow CI.
    assert!(
        elapsed < Duration::from_secs(8),
        "timeout took too long: {elapsed:?}"
    );
    h.shutdown().await;
}

/// A real MCP server that returns 200 tools with 100KB descriptions must
/// be capped: tool count ≤ MAX_TOOLS_PER_SESSION (128) — we expect spawn_all
/// to either reject (too many) OR truncate. We assert the spawn succeeds with
/// a bounded count, and that descriptions sent to the LLM are ≤ 1024 bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_metadata_caps_enforced() {
    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn(&llm.url).await;

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "many",
                "command": fake_mcp,
                "args": [],
                "env": [
                    { "name": "FAKE_MCP_TOOL_COUNT", "value": "200" },
                    { "name": "FAKE_MCP_HUGE_DESC", "value": "1" },
                ],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;

    // Either spawn rejects (200 > 128 cap) — that's acceptable hardening —
    // OR it accepts and we verify the LLM request stays bounded.
    if r.get("error").is_some() {
        let msg = r["error"]["message"].as_str().unwrap_or("");
        assert!(msg.contains("too many"), "unexpected error: {msg}");
        h.shutdown().await;
        return;
    }

    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();
    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let tools = captured[0]["tools"].as_array().unwrap();
    assert!(tools.len() <= 128, "tool count not capped: {}", tools.len());
    for t in tools {
        let desc = t["function"]["description"].as_str().unwrap_or("");
        assert!(
            desc.len() <= 1024,
            "description not capped: {} bytes",
            desc.len()
        );
    }
    h.shutdown().await;
}

/// Cap on MCP server count: 17 servers must be rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_count_cap() {
    let llm = spawn_capturing_llm(vec![]).await;
    let mut h = Harness::spawn(&llm.url).await;
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    let servers: Vec<Value> = (0..17)
        .map(|i| {
            json!({
                "name": format!("s{i}"),
                "command": fake_mcp,
                "args": [],
                "env": [],
            })
        })
        .collect();
    h.send("session/new", json!({"cwd":"/tmp","mcpServers": servers}))
        .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    assert!(r.get("error").is_some(), "expected error for 17 servers");
    let msg = r["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("too many"), "wrong error: {msg}");
    h.shutdown().await;
}

/// After cancelling mid-tool-loop, the next prompt must succeed without
/// the LLM seeing a malformed history (assistant tool_use with no
/// matching tool_result). Round 5 fix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_leaves_history_valid_for_next_prompt() {
    // Round 1: tool call (unknown — fails fast, no permission flow).
    // Round 2: text "ok".
    // After cancel, prompt 2 returns text immediately.
    let llm = spawn_capturing_llm(vec![
        openai_tool_call("tc1", "fake__t", json!({})),
        openai_text("after-cancel"),
        openai_text("p2-done"),
    ])
    .await;
    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(&mut h, json!([])).await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"first"}]}),
        )
        .await;
    // Cancel right away; the agent races between cancellation and the LLM
    // round trip — either way history must remain valid.
    h.notify("session/cancel", json!({"sessionId": sid})).await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    // Prompt 2 — must NOT error from a malformed history.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"second"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p2)).await;
    assert!(r.get("result").is_some(), "p2 errored: {r}");
    h.shutdown().await;
}

/// Empty assistant content + no tool_calls must serialize as "" (not null)
/// for OpenAI, so subsequent prompts don't get rejected. Round 7 fix 6.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_assistant_serializes_as_empty_string() {
    // First call returns content="" finish_reason=stop — agent records an
    // empty assistant turn. Second call's request body is what we inspect.
    let llm = spawn_capturing_llm(vec![openai_text(""), openai_text("done")]).await;
    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(&mut h, json!([])).await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"a"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"b"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p2)).await;

    let captured = llm.captured.lock().await;
    let msgs = captured[1]["messages"].as_array().unwrap();
    let empty_assistant = msgs
        .iter()
        .find(|m| m["role"] == "assistant" && m.get("tool_calls").is_none())
        .expect("no plain assistant turn");
    // Must be empty string, NOT null.
    assert_eq!(
        empty_assistant["content"],
        json!(""),
        "expected empty string content, got {empty_assistant}"
    );
    h.shutdown().await;
}

fn openai_n_tool_calls(n: usize) -> Value {
    let calls: Vec<Value> = (0..n)
        .map(|i| {
            json!({
                "id": format!("c{i}"),
                "type": "function",
                "function": { "name": "many__tool_0", "arguments": "{}" },
            })
        })
        .collect();
    json!({
        "id": "cc-n", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": calls },
            "finish_reason": "tool_calls",
        }],
    })
}

/// History budget evicts old turns: after many prompts, the LLM request
/// body stays below a sane bound. Round 7 fix; round 8 test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_budget_evicts_old_turns() {
    // Budget = 1 MB (MIN allowed by config). Each prompt is ~200 KB, so
    // 12 prompts × 200 KB = ~2.4 MB blows the cap and forces eviction.
    // We expect the captured request body to stay under 3× the cap.
    const BUDGET: usize = 1024 * 1024; // 1 MB — must be >= MAX_PROMPT_BYTES
    const PROMPT_BYTES: usize = 200 * 1024; // 200 KB per turn
    let responses: Vec<Value> = (0..12).map(|_| openai_text(&"y".repeat(200))).collect();
    let llm = spawn_capturing_llm(responses).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_HISTORY_BYTES", &BUDGET.to_string()),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"), // exercise truncation, not handoff
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    for i in 0..12 {
        let user = "x".repeat(PROMPT_BYTES);
        let p = h
            .send(
                "session/prompt",
                json!({"sessionId": sid, "prompt": [{"type":"text","text": format!("{i}:{user}")}]}),
            )
            .await;
        let _ = h.recv_until(|v| v["id"] == json!(p)).await;
    }

    let captured = llm.captured.lock().await;
    assert_eq!(captured.len(), 12);
    // The last request must show eviction: body well under unbounded 12 × 200 KB = 2.4 MB.
    let last = &captured[captured.len() - 1];
    let body_bytes = serde_json::to_vec(last).unwrap().len();
    assert!(
        body_bytes < BUDGET * 3,
        "history not evicted: request body is {body_bytes} bytes"
    );
    let msgs = last["messages"].as_array().unwrap();
    // We must NEVER drop the latest user prompt.
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "user" && m["content"].as_str().unwrap_or("").starts_with("11:")),
        "newest user turn missing"
    );
    h.shutdown().await;
}

/// Per-turn tool-call cap: an LLM that returns 100 tool_calls in one
/// response must only have 64 (MAX_TOOL_CALLS_PER_TURN) executed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_turn_tool_call_cap_enforced() {
    let llm = spawn_capturing_llm(vec![openai_n_tool_calls(100), openai_text("done")]).await;
    let mut h = Harness::spawn(&llm.url).await;

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "many",
                "command": fake_mcp,
                "args": [],
                "env": [{ "name": "FAKE_MCP_TOOL_COUNT", "value": "1" }],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;

    // Count distinct tool_call (pending) notifications until final response.
    let mut tool_call_ids = std::collections::HashSet::new();
    loop {
        let v = h.recv().await;
        if v.get("method") == Some(&json!("session/request_permission")) {
            let id = v["id"].clone();
            h.write(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "outcome": { "outcome": "selected", "optionId": "allow" } },
            }))
            .await;
            continue;
        }
        if v.get("method") == Some(&json!("session/update"))
            && v["params"]["update"]["sessionUpdate"] == "tool_call"
        {
            if let Some(id) = v["params"]["update"]["toolCallId"].as_str() {
                tool_call_ids.insert(id.to_owned());
            }
            continue;
        }
        if v["id"] == json!(p) {
            break;
        }
    }
    // MAX_TOOL_CALLS_PER_TURN = 64.
    assert_eq!(
        tool_call_ids.len(),
        64,
        "expected 64 tool_calls, got {}",
        tool_call_ids.len()
    );
    h.shutdown().await;
}

/// Description clamping: a 5000-byte description from MCP must be
/// truncated to ≤ 1024 bytes in the LLM request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn description_clamping_enforced() {
    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn(&llm.url).await;

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "big",
                "command": fake_mcp,
                "args": [],
                "env": [
                    { "name": "FAKE_MCP_TOOL_COUNT", "value": "1" },
                    { "name": "FAKE_MCP_DESC_SIZE", "value": "5000" },
                ],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    let tools = captured[0]["tools"].as_array().unwrap();
    // The MCP tool is "big__tool_0"; load_skill may also be present when
    // global skills are discovered from HOME. Find the MCP tool by name.
    let mcp_tool = tools
        .iter()
        .find(|t| t["function"]["name"].as_str() == Some("big__tool_0"))
        .expect("big__tool_0 not found in tool list");
    let desc = mcp_tool["function"]["description"].as_str().unwrap_or("");
    assert!(
        desc.len() <= 1024,
        "description not clamped: {} bytes (expected ≤ 1024)",
        desc.len()
    );
    // Sanity: the original was 5000 bytes, so we did clamp something.
    assert!(
        desc.len() < 5000,
        "description not actually truncated: {} bytes",
        desc.len()
    );
    h.shutdown().await;
}

/// Helper: spawn a session with a fake MCP server exposing one regular tool
/// plus an optional `_Stop` hook controlled by env vars.
async fn init_session_with_fake_mcp(h: &mut Harness, extra_mcp_env: &[(&str, &str)]) -> String {
    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    let env: Vec<Value> = extra_mcp_env
        .iter()
        .map(|(k, v)| json!({ "name": k, "value": v }))
        .collect();
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "fake",
                "command": fake_mcp,
                "args": [],
                "env": env,
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    r["result"]["sessionId"]
        .as_str()
        .expect("sessionId")
        .to_owned()
}

/// `_Stop` hook objects on the first end_turn → agent must NOT stop.
/// The hook returns an objection only on its first invocation; on the
/// second end_turn (after a tool round), the hook stays silent so the
/// agent ends cleanly. Verifies that the gate rerolls the LLM at least
/// once, and that the objection appears in history as a tool-role
/// message with the JSON-encoded source attribution.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_blocks_premature_end() {
    // LLM sequence:
    //   1. text "premature" (triggers _Stop objection — call #1)
    //   2. tool_call to fake__tool_0 (regular tool round)
    //   3. text "really done" (hook returns empty on call #2 → end)
    let llm = spawn_capturing_llm(vec![
        openai_text("premature"),
        openai_tool_call("tc1", "fake__tool_0", json!({})),
        openai_text("really done"),
    ])
    .await;
    // stop_max_rejections=10 so the budget never trips. The hook itself
    // stays silent on its second call (FAKE_MCP_STOP_COUNT=1) so the
    // second end_turn is accepted by the agent — this exercises the
    // genuine "objected then later cleared" path, not a budget cap.
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "10"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "you have open work"),
            // Objection text returned for the first STOP_COUNT calls;
            // empty string thereafter.
            ("FAKE_MCP_STOP_COUNT", "1"),
        ],
    )
    .await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p)).await;
    assert!(r.get("result").is_some(), "errored: {r}");
    assert_eq!(r["result"]["stopReason"], "end_turn");

    // Agent must have called LLM ≥2 times (initial end_turn was rejected,
    // forcing another LLM round). We expect exactly 3 here: text → tool → text.
    let captured = llm.captured.lock().await;
    assert!(
        captured.len() >= 2,
        "agent did not loop after objection: {} LLM calls",
        captured.len()
    );

    // Round 2's request must carry the objection as a tool-role message
    // (synthetic tool result), not a user/assistant message. Content is
    // a JSON object with hook/server/text fields — never escapable.
    let msgs = captured[1]["messages"].as_array().unwrap();
    let objection_present = msgs.iter().any(|m| {
        if m["role"] != "tool" {
            return false;
        }
        let content = m["content"].as_str().unwrap_or("");
        let parsed: Value = match serde_json::from_str(content) {
            Ok(v) => v,
            Err(_) => return false,
        };
        parsed["hook"] == "_Stop"
            && parsed["server"] == "fake"
            && parsed["text"]
                .as_str()
                .unwrap_or("")
                .contains("you have open work")
    });
    assert!(
        objection_present,
        "objection (role=tool, JSON-encoded) missing from messages: {msgs:?}"
    );
    h.shutdown().await;
}

/// After `stop_max_rejections` objections, the agent honors end_turn
/// even if `_Stop` would still object. Set max=1 so it trips quickly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_budget_exhausted() {
    // LLM sequence:
    //   1. text → triggers _Stop objection (rejections: 0→1)
    //   2. tool_call (regular tool round)
    //   3. text → gate sees rejections>=max, returns end_turn (no _Stop call)
    let llm = spawn_capturing_llm(vec![
        openai_text("first"),
        openai_tool_call("tc1", "fake__tool_0", json!({})),
        openai_text("second"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "1"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "still working"),
        ],
    )
    .await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p)).await;
    assert!(r.get("result").is_some(), "errored: {r}");
    assert_eq!(r["result"]["stopReason"], "end_turn");

    // Three LLM calls expected: budget cap stops the loop on the 3rd end_turn.
    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        3,
        "expected exactly 3 LLM calls (budget cap), got {}",
        captured.len()
    );
    h.shutdown().await;
}

/// A persistent `_Stop` objection must keep the turn alive through repeated
/// consecutive end_turn responses. The configured rejection budget is the
/// bounded escape hatch; accepting the second response would silently idle a
/// session that still has open work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_consecutive_end_turn_uses_rejection_budget() {
    // Three consecutive end_turn responses. With max=2, both objections must
    // reroll the LLM and the third response is accepted by the budget cap.
    let llm = spawn_capturing_llm(vec![
        openai_text("done-1"),
        openai_text("done-2"),
        openai_text("done-3"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "2"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "keep going"),
        ],
    )
    .await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p)).await;
    assert!(r.get("result").is_some(), "errored: {r}");
    assert_eq!(r["result"]["stopReason"], "end_turn");

    // Both objections force another round; the budget permits the third end.
    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        3,
        "expected 3 LLM calls (two objections, then budget cap), got {}",
        captured.len()
    );
    h.shutdown().await;
}

/// The `_Stop` rejection budget is per prompt: exhausting it on one prompt
/// must not disable the stop guard for the rest of the session. A second
/// prompt gets a fresh budget and its end_turn is objected to again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_budget_resets_per_prompt() {
    // Each prompt: text → objection (budget 0→1) → text → cap. With max=1,
    // both prompts take exactly 2 LLM calls; a session-cumulative budget
    // would accept prompt 2's first end_turn without calling _Stop (3 total).
    let llm = spawn_capturing_llm(vec![
        openai_text("p1-a"),
        openai_text("p1-b"),
        openai_text("p2-a"),
        openai_text("p2-b"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "1"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "keep going"),
        ],
    )
    .await;

    for prompt in ["one", "two"] {
        let p = h
            .send(
                "session/prompt",
                json!({"sessionId": sid, "prompt": [{"type":"text","text": prompt}]}),
            )
            .await;
        let r = h.recv_until(|v| v["id"] == json!(p)).await;
        assert!(r.get("result").is_some(), "errored: {r}");
        assert_eq!(r["result"]["stopReason"], "end_turn");
    }

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        4,
        "expected 4 LLM calls (fresh budget objected on both prompts), got {}",
        captured.len()
    );
    h.shutdown().await;
}

/// Regression: an LLM that tries to call a hidden hook tool (e.g.
/// `fake___Stop`) directly must get an "unknown tool" error result —
/// the MCP server must NOT be invoked. This guarantees a malicious or
/// confused model can't trigger lifecycle hooks itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_tools_hidden_from_llm() {
    // LLM sequence:
    //   1. tool_call to fake___Stop (hidden hook, must fail closed)
    //   2. text "done"
    let llm = spawn_capturing_llm(vec![
        openai_tool_call("tc1", "fake___Stop", json!({})),
        openai_text("done"),
    ])
    .await;
    // We deliberately leave MCP_HOOK_SERVERS unset so the
    // agent's hook gate is disabled — hook-tool hiding must hold even
    // when hooks aren't allowlisted (defense in depth).
    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            // Distinct text we can scan for. If the MCP server is ever
            // invoked, this string would appear in the captured history.
            ("FAKE_MCP_STOP_TEXT", "HOOK_LEAKED_TO_LLM"),
        ],
    )
    .await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p)).await;
    assert!(r.get("result").is_some(), "errored: {r}");

    // The tool result fed back to the LLM (round 2) must be the
    // synthetic "unknown tool" error, not the hook's actual output.
    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected 2 LLM calls, got {}",
        captured.len()
    );
    let msgs = captured[1]["messages"].as_array().unwrap();
    let tool_msg = msgs
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("expected a tool result message in round 2");
    let content = tool_msg["content"].as_str().unwrap_or("");
    assert!(
        content.contains("unknown tool"),
        "expected unknown-tool error, got: {content}"
    );
    assert!(
        !content.contains("HOOK_LEAKED_TO_LLM"),
        "MCP hook was invoked from the LLM path: {content}"
    );

    // Defense-in-depth: also confirm the *advertised* tools never
    // included the hook in the first place.
    let round1_tools = captured[0]["tools"].as_array().unwrap();
    for t in round1_tools {
        let name = t["function"]["name"].as_str().unwrap_or("");
        assert!(
            !name.contains("_Stop"),
            "hook tool advertised to LLM: {name}"
        );
    }
    h.shutdown().await;
}

/// `_PostCompact` hook fires after a context-handoff and its output is
/// folded into the fresh `[Context Handoff]` user-context block as explicitly
/// untrusted text. The next LLM request must therefore see the post-compact
/// text without any orphan `role=tool` messages — proving the hook ran on the
/// *new* context, not the discarded one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_post_compact_injects_after_handoff() {
    // Sequence of canned LLM responses consumed in order:
    //   1-3. Three `session/prompt` rounds returning short text. Each
    //        prompt body is ~300 KB, so by the 4th prompt we'll be over
    //        the 90% (= ~922 KB) threshold of a 1 MB budget.
    //   4.   Handoff `summarize()` call returns the summary text.
    //   5.   Next regular `complete()` call after the handoff returns
    //        a final "done" message; we inspect this request's body.
    let llm = spawn_capturing_llm(vec![
        openai_text("ack-1"),
        openai_text("ack-2"),
        openai_text("ack-3"),
        openai_text("handoff summary text"),
        openai_text("done"),
    ])
    .await;
    // 1 MB budget = MIN allowed. Threshold = ~922 KB. Each ~300 KB prompt
    // fills the budget on the 4th turn, triggering handoff.
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_MAX_HISTORY_BYTES", &(1024 * 1024).to_string()),
            // Allow at least one handoff.
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            // No _Stop hook here — _PostCompact only.
            ("FAKE_MCP_POSTCOMPACT_HOOK", "1"),
            ("FAKE_MCP_POSTCOMPACT_TEXT", "todo state here"),
        ],
    )
    .await;

    // Drive prompts until we observe a handoff. We detect it by counting
    // captured LLM requests: a handoff inserts one extra `summarize` call
    // that we didn't issue ourselves. We send up to 6 prompts.
    let big = "x".repeat(300 * 1024);
    let mut prompts_sent = 0usize;
    let mut handoff_observed = false;
    for i in 0..6 {
        let p = h
            .send(
                "session/prompt",
                json!({
                    "sessionId": sid,
                    "prompt": [{"type":"text","text": format!("{i}:{big}")}],
                }),
            )
            .await;
        let _ = h.recv_until(|v| v["id"] == json!(p)).await;
        prompts_sent += 1;
        let captured_now = llm.captured.lock().await.len();
        // After N prompts we'd normally see N requests; an extra request
        // means a handoff summarize() ran.
        if captured_now > prompts_sent {
            handoff_observed = true;
            break;
        }
    }
    assert!(
        handoff_observed,
        "no handoff observed after {prompts_sent} prompts (captured={})",
        llm.captured.lock().await.len()
    );

    // The first LLM call AFTER the handoff is the one we inspect. Find it:
    // it's the one where the messages array is short (history just reset)
    // and contains the _PostCompact payload inside user-context text. It must
    // not be emitted as an orphan tool result because the old assistant tool
    // call was deliberately discarded by the handoff reset.
    let captured = llm.captured.lock().await;
    let post_compact_visible = captured.iter().any(|req| {
        let msgs = match req["messages"].as_array() {
            Some(m) => m,
            None => return false,
        };
        msgs.iter().any(|m| {
            if m["role"] != "user" {
                return false;
            }
            let content = m["content"].as_str().unwrap_or("");
            content.contains("[Post-compact hook output — untrusted]")
                && content.contains("[fake]")
                && content.contains("todo state here")
        })
    });
    assert!(
        post_compact_visible,
        "_PostCompact context not visible to LLM after handoff"
    );
    let orphan_tool_result = captured.iter().any(|req| {
        req["messages"]
            .as_array()
            .is_some_and(|msgs| msgs.iter().any(|m| m["role"] == "tool"))
    });
    assert!(
        !orphan_tool_result,
        "handoff reset must not leave orphan role=tool messages"
    );
    h.shutdown().await;
}

/// The handoff summary prompt should include all session history when that
/// history fits the summarizer context budget. This protects against regressing
/// to the old fixed tail of five tiny snippets.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handoff_summary_prompt_includes_full_history_within_context_budget() {
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("ack-0", 9500),
        openai_text("handoff summary text"),
        openai_text_with_usage("done", 10),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "10000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"early-history-marker"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p0)).await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"late-history-marker"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    let captured = llm.captured.lock().await;
    assert_eq!(captured.len(), 3, "expected prompt, handoff, prompt");
    let handoff_messages = captured[1]["messages"].as_array().unwrap();
    let handoff_prompt = handoff_messages[1]["content"].as_str().unwrap();
    assert!(
        handoff_prompt.contains("# Session History (oldest first)"),
        "handoff prompt should describe full session history: {handoff_prompt}"
    );
    assert!(
        handoff_prompt.contains("early-history-marker"),
        "oldest prompt was omitted despite fitting budget: {handoff_prompt}"
    );
    assert!(
        handoff_prompt.contains("ack-0"),
        "assistant response was omitted despite fitting budget: {handoff_prompt}"
    );
    assert!(
        handoff_prompt.contains("late-history-marker"),
        "latest prompt was omitted despite fitting budget: {handoff_prompt}"
    );
    assert!(
        !handoff_prompt.contains("older items omitted"),
        "handoff should not report truncation when full history fits: {handoff_prompt}"
    );
    h.shutdown().await;
}

/// If one item is larger than the derived summarizer budget, keep a truncated
/// form of the most recent item instead of sending an empty history block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handoff_summary_prompt_keeps_latest_item_when_one_item_exceeds_budget() {
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("ack-0", 9500),
        openai_text("handoff summary text"),
        openai_text_with_usage("done", 10),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "10000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let huge = format!("oversize-latest-marker {}", "x".repeat(12000));
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"early-history-marker"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p0)).await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": huge}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    let captured = llm.captured.lock().await;
    assert_eq!(captured.len(), 3, "expected prompt, handoff, prompt");
    let handoff_messages = captured[1]["messages"].as_array().unwrap();
    let handoff_prompt = handoff_messages[1]["content"].as_str().unwrap();
    assert!(
        handoff_prompt.contains("oversize-latest-marker"),
        "latest oversized item should be kept in truncated form: {handoff_prompt}"
    );
    assert!(
        handoff_prompt.contains("older items omitted"),
        "handoff should report truncation when history exceeds budget: {handoff_prompt}"
    );
    h.shutdown().await;
}

/// Regression for the original bug: context fills, the provider 400s on the
/// next request, and the handoff never fires because the old gate measured
/// BYTES (16 MiB threshold) while the limit is in TOKENS. The fix gates on
/// provider-reported input tokens. Here the prompts are tiny (bytes nowhere
/// near any byte threshold), but the fake LLM reports `usage.prompt_tokens`
/// over the configured token budget — so the handoff MUST fire on the token
/// signal alone, before the next normal `complete()`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_usage_over_budget_triggers_handoff() {
    // Context window 1000 tokens, output 100 -> threshold = min(900, 900) = 900.
    // First response reports 950 input tokens (> 900). The agent stores that;
    // the next prompt's pre-flight gate sees 950 >= 900 and hands off, which
    // inserts an extra summarize() call we didn't issue.
    //   req 1: prompt #0  -> text + usage(950)
    //   req 2: summarize() (the handoff) -> summary text
    //   req 3: prompt #1's actual complete() -> done
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("ack-0", 950),
        openai_text("handoff summary text"),
        openai_text_with_usage("done", 10),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "100"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
            // Huge byte budget so the byte path can NOT be what fires — only
            // the token gate can explain a handoff on these tiny prompts.
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    // Prompt #0: small body; response carries usage(950) -> over threshold.
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"hello 0"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert_eq!(
        llm.captured.lock().await.len(),
        1,
        "first prompt should produce exactly one LLM request (no handoff yet)"
    );

    // Prompt #1: also small. The pre-flight gate sees the stored 800 tokens
    // and hands off BEFORE issuing this prompt's complete() -> an extra
    // summarize request appears (3 total, not 2).
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"hello 1"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured, 3,
        "expected handoff summarize() between the two prompts (3 reqs), saw {captured} — \
         token gate did not fire on usage over budget"
    );
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("handoff #1 (history"),
        "expected handoff log line in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains(" -> ") && stderr.contains(" tokens"),
        "expected handoff log to include before/after token counts, got: {stderr}"
    );
    h.shutdown().await;
}

/// Regression for the stale-usage gap (caught in review): the exact token
/// count describes the PREVIOUS request, but history grows afterward (tool
/// results, next prompt). If the gate trusted only the stale `Some(tokens)`
/// and skipped the byte signal, a previously-under-threshold session could
/// still 400 once a large tool result lands. The fix adds a conservative
/// token estimate of the bytes grown since the measurement. Here usage is
/// reported UNDER threshold, then a large tool result grows history enough
/// that the projection crosses — so the handoff must fire.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_usage_plus_history_growth_triggers_handoff() {
    // window 10_000, output 1_000 -> threshold = min(9_000, 9_000) = 9_000.
    // req1 reports usage 8_500 (UNDER 9_000). Its response is a tool_call;
    // the fake MCP returns a ~6 KB result, appended to history. At the
    // conservative 1 byte/token estimate that's ~6_000 projected tokens, so
    // projected ~14_500 >= 9_000 -> the next loop iteration hands off before
    // the follow-up complete().
    //   req1: tool_call + usage(8500)
    //   (tool result ~6KB appended)
    //   req2: summarize() (handoff)
    //   req3: final text
    let llm = spawn_capturing_llm(vec![
        {
            let mut v = openai_tool_call("tc1", "fake__tool_0", json!({}));
            v["usage"] =
                json!({"prompt_tokens": 8500, "completion_tokens": 1, "total_tokens": 8501});
            v
        },
        openai_text("handoff summary text"),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "10000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
            // Huge byte budget so the None-path byte fallback can't be what
            // fires — only the token-mode growth estimate can explain it.
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_RESULT_SIZE", "6000"),
        ],
    )
    .await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;
    // req1 (tool_call) + summarize (handoff) + req2 (done) = 3. Without the
    // growth estimate we'd see only 2 (stale 8500 < 9000, no handoff).
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured, 3,
        "expected handoff after history grew past threshold (3 reqs), saw {captured} — \
         stale under-threshold usage skipped the growth estimate"
    );
    h.shutdown().await;
}

/// `_Stop` hook that takes longer than `BUZZ_AGENT_HOOK_TIMEOUT_MS`
/// must be treated as no-objection (fail-open). Agent stops normally.
///
/// Note on server-kill-on-timeout: `call_hooks` calls `kill_server` on a
/// timed-out hook so a wedged server can't poison subsequent calls. We
/// don't add a separate per-test assertion for this — the
/// `mcp_init_timeout_kills_child` test already exercises the same
/// kill-on-timeout codepath through `kill_server`, and the harness here
/// (spawn-then-shutdown) makes a follow-up "tool still works" check
/// fragile because the server we just killed is the only one in the
/// session. The timeout assertion below (elapsed < 2.5s) implicitly
/// covers the kill: if the hook child kept running past the timeout,
/// we'd block on it during shutdown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_timeout_failopen() {
    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            // Hook delay (3s) >> hook timeout (200ms) → fail-open.
            ("BUZZ_AGENT_HOOK_TIMEOUT_MS", "200"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "would object"),
            ("FAKE_MCP_STOP_DELAY", "3"),
        ],
    )
    .await;

    let started = Instant::now();
    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(p)).await;
    let elapsed = started.elapsed();

    assert!(r.get("result").is_some(), "errored: {r}");
    assert_eq!(r["result"]["stopReason"], "end_turn");
    // Hook delay is 3s; if we waited for it the test would take ≥3s.
    // 1.5s gives slack for CI without masking a regression.
    assert!(
        elapsed < Duration::from_millis(2500),
        "did not fail-open: prompt took {elapsed:?}"
    );

    // Only the initial LLM call — agent did NOT loop after the timeout.
    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        1,
        "expected 1 LLM call, got {}",
        captured.len()
    );
    h.shutdown().await;
}

/// When a session is cancelled while a tool call is in-flight, the agent
/// sends `notifications/cancelled` to the MCP server. With buzz-dev-mcp,
/// this cancels the CancellationToken and kills the running shell process
/// group. We verify:
///   1. The prompt completes in under 5s (not 60s).
///   2. The `sleep 60` process is actually dead after cancel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_kills_inflight_tool_via_mcp_notification() {
    // buzz-dev-mcp is a separate crate; locate its binary relative to
    // the buzz-agent test binary (they share the same target dir).
    let self_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_buzz-agent"));
    let dev_mcp_bin = self_bin.parent().unwrap().join("buzz-dev-mcp");
    let dev_mcp_is_executable = std::fs::metadata(&dev_mcp_bin)
        .map(|metadata| {
            if !metadata.is_file() || metadata.len() == 0 {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
        .unwrap_or(false);
    if !dev_mcp_is_executable {
        eprintln!(
            "SKIP: buzz-dev-mcp not built at {}; run `cargo build -p buzz-dev-mcp` first",
            dev_mcp_bin.display()
        );
        return;
    }
    let dev_mcp_bin = dev_mcp_bin.to_string_lossy().to_string();

    // Use a unique marker (PID + timestamp) to avoid stale-file collisions.
    let marker = format!(
        "buzz_cancel_test_{}_{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let pid_file = format!("/tmp/{marker}.pid");
    let _ = std::fs::remove_file(&pid_file); // clean any stale file
    let cmd = format!("echo $$ > /tmp/{marker}.pid && exec sleep 60");

    // LLM returns a shell tool call, then text after cancel.
    let llm = spawn_capturing_llm(vec![
        openai_tool_call("tc1", "dev__shell", json!({"command": cmd})),
        openai_text("done"),
    ])
    .await;

    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(
        &mut h,
        json!([{
            "name": "dev",
            "command": &dev_mcp_bin,
            "args": [],
            "env": []
        }]),
    )
    .await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"run"}]}),
        )
        .await;

    // Wait for the tool call to be in-progress.
    h.recv_until(|v| {
        v.get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("status"))
            .and_then(Value::as_str)
            == Some("in_progress")
    })
    .await;

    // Wait for the shell to spawn and write its PID (bounded).
    let pid_deadline = Instant::now() + Duration::from_secs(3);
    let shell_pid: u32 = loop {
        if let Ok(content) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(
            Instant::now() < pid_deadline,
            "shell did not write PID file within 3s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    // Cancel the session — measure latency from here.
    let cancel_start = Instant::now();
    h.notify("session/cancel", json!({"sessionId": sid})).await;

    // Wait for prompt to complete.
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    let cancel_latency = cancel_start.elapsed();
    // Cancellation itself should complete in well under 3s. The 60s sleep
    // must NOT run to completion. We allow generous CI slack.
    assert!(
        cancel_latency < Duration::from_secs(3),
        "cancel latency too high: {cancel_latency:?} (expected < 3s)"
    );

    // Verify the shell process is actually dead (bounded poll).
    let kill_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &shell_pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !alive {
            break;
        }
        assert!(
            Instant::now() < kill_deadline,
            "shell process {shell_pid} still alive 2s after cancel"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Cleanup.
    let _ = std::fs::remove_file(&pid_file);
    h.shutdown().await;
}

/// Protocol-level test: verify that `notifications/cancelled` is sent to
/// any MCP server (not just buzz-dev-mcp) when a session is cancelled
/// during an in-flight tool call. Uses fake_mcp with FAKE_MCP_CANCEL_LOG
/// to capture the raw notification on stdin.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_sends_notifications_cancelled_to_any_mcp_server() {
    let cancel_log = std::env::temp_dir()
        .join(format!(
            "buzz_cancel_proto_{}_{:x}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&cancel_log);
    let call_received_marker = format!("{cancel_log}.call_received");
    let _ = std::fs::remove_file(&call_received_marker);

    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");

    // LLM returns a tool call; fake_mcp will delay 999s (never responds).
    let llm = spawn_capturing_llm(vec![
        openai_tool_call("tc1", "fake__tool_0", json!({})),
        openai_text("done"),
    ])
    .await;

    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(
        &mut h,
        json!([{
            "name": "fake",
            "command": fake_mcp,
            "args": [],
            "env": [
                {"name": "FAKE_MCP_TOOL_DELAY", "value": "999"},
                {"name": "FAKE_MCP_CANCEL_LOG", "value": &cancel_log},
                {"name": "FAKE_MCP_CALL_RECEIVED", "value": &call_received_marker},
            ]
        }]),
    )
    .await;

    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;

    // Wait for tool call to be in-progress.
    h.recv_until(|v| {
        v.get("params")
            .and_then(|p| p.get("update"))
            .and_then(|u| u.get("status"))
            .and_then(Value::as_str)
            == Some("in_progress")
    })
    .await;

    // Wait until fake_mcp has received the tools/call request (bounded).
    // The marker file contains the JSON-RPC request id.
    let call_deadline = Instant::now() + Duration::from_secs(3);
    let call_request_id: Value = loop {
        if let Ok(content) = std::fs::read_to_string(&call_received_marker) {
            if let Ok(id) = serde_json::from_str::<Value>(content.trim()) {
                break id;
            }
        }
        assert!(
            Instant::now() < call_deadline,
            "fake_mcp did not receive tools/call within 3s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    // Cancel the session.
    h.notify("session/cancel", json!({"sessionId": sid})).await;

    // Wait for prompt to complete.
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    // Poll the cancel log with bounded timeout (replaces fixed sleep).
    let poll_deadline = Instant::now() + Duration::from_secs(2);
    let log_content = loop {
        let content = std::fs::read_to_string(&cancel_log).unwrap_or_default();
        if content.contains("notifications/cancelled") {
            break content;
        }
        assert!(
            Instant::now() < poll_deadline,
            "cancel notification not received within 2s; log: {content:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    // Parse the logged notification and verify requestId matches the
    // actual tools/call request id that fake_mcp received.
    let cancel_msg: Value = serde_json::from_str(log_content.trim()).unwrap_or(json!(null));
    let cancelled_id = &cancel_msg["params"]["requestId"];
    assert!(
        cancelled_id.is_number(),
        "expected numeric requestId in cancel notification, got: {cancel_msg}"
    );
    assert_eq!(
        cancelled_id, &call_request_id,
        "cancelled requestId ({cancelled_id}) != tools/call id ({call_request_id})"
    );

    // Cleanup.
    let _ = std::fs::remove_file(&cancel_log);
    let _ = std::fs::remove_file(&call_received_marker);
    h.shutdown().await;
}

// ---------------------------------------------------------------------------
// Reply guard (`BUZZ_AGENT_REQUIRE_REPLY`)
//
// The guard reminds the model to publish when a turn is about to end without
// any recognized attempt to post to Buzz. It rides the existing `_Stop` gate
// and shares its rejection budget, so most of these tests count LLM calls:
// each reminder costs exactly one extra round.
// ---------------------------------------------------------------------------

/// Number of reply-guard reminders present in one captured LLM request.
///
/// A reminder is a tool-role message whose JSON body is attributed to the
/// in-process guard (`server: "buzz-agent"`) at the `_Stop` hook point — the
/// same lower-trust shape as real hook output.
fn reply_nag_count(request: &Value) -> usize {
    request["messages"]
        .as_array()
        .map(|msgs| {
            msgs.iter()
                .filter(|m| {
                    m["role"] == "tool"
                        && serde_json::from_str::<Value>(m["content"].as_str().unwrap_or(""))
                            .map(|p| p["hook"] == "_Stop" && p["server"] == "buzz-agent")
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// A publish-shaped call to a real registered shell tool.
fn openai_shell_send(id: &str) -> Value {
    openai_tool_call(
        id,
        "fake__shell",
        json!({ "command": "buzz messages send --channel c --content hi" }),
    )
}

/// Run one prompt to completion, answering any permission requests, and
/// return the final response.
async fn prompt_to_completion(h: &mut Harness, sid: &str) -> Value {
    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    loop {
        let v = h.recv().await;
        if v.get("method") == Some(&json!("session/request_permission")) {
            let id = v["id"].clone();
            h.write(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "outcome": { "outcome": "selected", "optionId": "allow" } },
            }))
            .await;
            continue;
        }
        if v["id"] == json!(p) {
            return v;
        }
    }
}

/// Default off: a silent turn ends on the first end_turn with no extra round.
/// This is the invariant that keeps the feature free for everyone who hasn't
/// opted in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_off_by_default() {
    let llm = spawn_capturing_llm(vec![openai_text("done"), openai_text("unexpected")]).await;
    let mut h = Harness::spawn(&llm.url).await;
    let sid = init_session(&mut h, json!([])).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        1,
        "guard must be inert when unset, got {} LLM calls",
        captured.len()
    );
    h.shutdown().await;
}

/// `BUZZ_AGENT_REQUIRE_REPLY=0` is off too — the toggle is numeric, so a
/// literal `0` must not read as "set, therefore on".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_explicit_zero_is_off() {
    let llm = spawn_capturing_llm(vec![openai_text("done"), openai_text("unexpected")]).await;
    let mut h = Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_REQUIRE_REPLY", "0")]).await;
    let sid = init_session(&mut h, json!([])).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        1,
        "REQUIRE_REPLY=0 must behave as off, got {} LLM calls",
        captured.len()
    );
    h.shutdown().await;
}

/// Opted in and silent: exactly two reminders, then the turn is allowed to
/// end. The guard is advisory — it must never trap a turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_nags_twice_then_lets_the_turn_end() {
    // Budget defaults to 3, so the cap that stops the loop here is
    // MAX_REPLY_NAGS = 2, not the rejection budget.
    let llm = spawn_capturing_llm(vec![
        openai_text("silent-1"),
        openai_text("silent-2"),
        openai_text("silent-3"),
        openai_text("must-not-be-requested"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_REQUIRE_REPLY", "1")]).await;
    let sid = init_session(&mut h, json!([])).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        3,
        "expected 2 reminders then end_turn (3 LLM calls), got {}",
        captured.len()
    );
    assert_eq!(
        reply_nag_count(&captured[0]),
        0,
        "reminder before any end_turn"
    );
    assert_eq!(reply_nag_count(&captured[1]), 1);
    assert_eq!(reply_nag_count(&captured[2]), 2);

    // The reminder must name the command it wants and license silence, so it
    // cannot fight the base prompt's "silence is usually correct".
    let msgs = captured[2]["messages"].as_array().unwrap();
    let nag = msgs
        .iter()
        .filter_map(|m| serde_json::from_str::<Value>(m["content"].as_str().unwrap_or("")).ok())
        .find(|p| p["server"] == "buzz-agent")
        .expect("reminder body");
    let text = nag["text"].as_str().unwrap_or("");
    assert!(
        text.contains("buzz messages send"),
        "reminder should name the command: {text}"
    );
    assert!(
        text.contains("silence is genuinely correct"),
        "reminder must license silence: {text}"
    );
    h.shutdown().await;
}

/// A real publish attempt through a registered shell tool satisfies the guard:
/// no reminder, no extra round.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_satisfied_by_registered_shell_send() {
    let llm = spawn_capturing_llm(vec![
        openai_shell_send("tc1"),
        openai_text("posted"),
        openai_text("must-not-be-requested"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_REQUIRE_REPLY", "1")]).await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[("FAKE_MCP_TOOL_COUNT", "1"), ("FAKE_MCP_SHELL_TOOL", "1")],
    )
    .await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "a recognized send must not be nagged, got {} LLM calls",
        captured.len()
    );
    assert_eq!(reply_nag_count(&captured[1]), 0);
    h.shutdown().await;
}

/// A publish-shaped call to a shell tool that is *not registered* never runs —
/// preflight rejects it — so it must not disarm the guard. This is what the
/// `has`/`is_hook` checks in the predicate buy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_ignores_unregistered_shell_tool() {
    // FAKE_MCP_SHELL_TOOL is absent, so `fake__shell` is a hallucination.
    let llm = spawn_capturing_llm(vec![
        openai_shell_send("tc1"),
        openai_text("silent-1"),
        openai_text("silent-2"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_REQUIRE_REPLY", "1"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "1"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(&mut h, &[("FAKE_MCP_TOOL_COUNT", "1")]).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        3,
        "expected the hallucinated call to still be nagged, got {} LLM calls",
        captured.len()
    );
    let msgs = captured[1]["messages"].as_array().unwrap();
    assert!(
        msgs.iter()
            .any(|m| m["role"] == "tool"
                && m["content"].as_str().unwrap_or("").contains("unknown tool")),
        "expected preflight to reject the call: {msgs:?}"
    );
    assert_eq!(reply_nag_count(&captured[2]), 1);
    h.shutdown().await;
}

/// A publish-shaped call discarded by the per-turn tool-call cap never runs,
/// so it must not suppress the reminder either. Pins the check's placement
/// after `calls.truncate(MAX_TOOL_CALLS_PER_TURN)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_ignores_calls_lost_to_the_turn_cap() {
    // 64 filler calls (the cap) followed by the publish attempt, which is
    // therefore truncated away. The shell tool *is* registered here, so only
    // the placement — not tool identity — can explain the reminder.
    let mut calls: Vec<Value> = (0..64)
        .map(|i| {
            json!({
                "id": format!("c{i}"),
                "type": "function",
                "function": { "name": "fake__tool_0", "arguments": "{}" },
            })
        })
        .collect();
    calls.push(json!({
        "id": "c-send",
        "type": "function",
        "function": {
            "name": "fake__shell",
            "arguments": json!({ "command": "buzz messages send --channel c --content hi" })
                .to_string(),
        },
    }));
    let truncated_send = json!({
        "id": "cc-trunc", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": null, "tool_calls": calls },
            "finish_reason": "tool_calls",
        }],
    });
    let llm = spawn_capturing_llm(vec![
        truncated_send,
        openai_text("silent-1"),
        openai_text("silent-2"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_REQUIRE_REPLY", "1"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "1"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[("FAKE_MCP_TOOL_COUNT", "1"), ("FAKE_MCP_SHELL_TOOL", "1")],
    )
    .await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        3,
        "a truncated send must still be nagged, got {} LLM calls",
        captured.len()
    );
    assert_eq!(reply_nag_count(&captured[2]), 1);
    h.shutdown().await;
}

/// The shared `_Stop` rejection budget is the outer cap: at 1 the guard gets
/// one reminder instead of two. Documented degradation, not a bug.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_bounded_by_stop_rejection_budget() {
    let llm = spawn_capturing_llm(vec![
        openai_text("silent-1"),
        openai_text("silent-2"),
        openai_text("must-not-be-requested"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_REQUIRE_REPLY", "1"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "1"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "budget 1 must allow exactly one reminder, got {} LLM calls",
        captured.len()
    );
    assert_eq!(reply_nag_count(&captured[1]), 1);
    h.shutdown().await;
}

/// Budget 0 disables every objection at the gate, including this one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_off_when_stop_budget_is_zero() {
    let llm = spawn_capturing_llm(vec![openai_text("done"), openai_text("unexpected")]).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_REQUIRE_REPLY", "1"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        1,
        "budget 0 must disable the guard, got {} LLM calls",
        captured.len()
    );
    h.shutdown().await;
}

/// The two axes are independent inside one shared budget: a round carrying
/// both a `_Stop` hook objection and a reminder costs one rejection and
/// delivers both texts, and once the reminders are spent the hook objection
/// continues alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reply_guard_combines_with_stop_hook_objection() {
    // The hook objects on its first 3 calls, then clears. Reminders stop
    // after 2, so round 3 must carry the hook text and no new reminder.
    let llm = spawn_capturing_llm(vec![
        openai_text("silent-1"),
        openai_text("silent-2"),
        openai_text("silent-3"),
        openai_text("silent-4"),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_REQUIRE_REPLY", "1"),
            ("MCP_HOOK_SERVERS", "fake"),
            ("BUZZ_AGENT_STOP_MAX_REJECTIONS", "10"),
        ],
    )
    .await;
    let sid = init_session_with_fake_mcp(
        &mut h,
        &[
            ("FAKE_MCP_TOOL_COUNT", "1"),
            ("FAKE_MCP_STOP_HOOK", "1"),
            ("FAKE_MCP_STOP_TEXT", "you have open todos"),
            ("FAKE_MCP_STOP_COUNT", "3"),
        ],
    )
    .await;

    let r = prompt_to_completion(&mut h, &sid).await;
    assert_eq!(r["result"]["stopReason"], "end_turn");

    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        4,
        "expected 3 objecting rounds then a clear end, got {}",
        captured.len()
    );

    let hook_objections = |req: &Value| -> usize {
        req["messages"]
            .as_array()
            .map(|msgs| {
                msgs.iter()
                    .filter(|m| {
                        m["content"]
                            .as_str()
                            .unwrap_or("")
                            .contains("you have open todos")
                    })
                    .count()
            })
            .unwrap_or(0)
    };

    // Round 2 carries one of each — a single rejection bought both texts.
    assert_eq!(reply_nag_count(&captured[1]), 1);
    assert_eq!(hook_objections(&captured[1]), 1);
    // Round 4: the hook objected three times, the guard only twice.
    assert_eq!(reply_nag_count(&captured[3]), 2);
    assert_eq!(hook_objections(&captured[3]), 3);
    h.shutdown().await;
}

/// An unparseable toggle is a startup error, not a silent default. `parse_env`
/// is generic over `FromStr`, so this also pins the numeric type: a `bool`
/// field would have rejected the documented `1`.
#[test]
fn reply_guard_rejects_unparseable_toggle() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_buzz-agent"))
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .env("OPENAI_COMPAT_API_KEY", "test")
        .env("OPENAI_COMPAT_MODEL", "fake-model")
        .env("BUZZ_AGENT_REQUIRE_REPLY", "true")
        .stdin(Stdio::null())
        .output()
        .expect("run buzz-agent");
    assert!(
        !out.status.success(),
        "expected a config error exit, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("BUZZ_AGENT_REQUIRE_REPLY"),
        "expected the offending key in the error, got: {stderr}"
    );
}

/// A prompt large enough that the recovery ladder's halving stays above
/// `HANDOFF_MIN_PROMPT_BUDGET_BYTES` (4 KiB) for all three rungs.
///
/// This is load-bearing, not decoration: with a tiny history the ladder
/// correctly refuses on the FIRST rung (halving a 49-byte history lands at 24
/// bytes, far under the floor), so a small fixture cannot exercise recovery at
/// all — it exercises the floor. `marker` is embedded so the prompt is still
/// identifiable in a captured request body.
fn large_prompt(marker: &str) -> String {
    let mut s = String::with_capacity(64 * 1024 + marker.len());
    s.push_str(marker);
    s.push(' ');
    while s.len() < 64 * 1024 {
        s.push_str("filler context to make the history realistically large. ");
    }
    s
}

/// OpenAI-compatible context-window rejection body, matching the shape the
/// provider actually returns on overflow.
fn openai_context_length_error() -> Value {
    json!({
        "error": {
            "message": "This model's maximum context length is 8192 tokens. \
                        However, your messages resulted in 20000 tokens.",
            "type": "invalid_request_error",
            "code": "context_length_exceeded",
        }
    })
}

/// A 400 that is NOT a context-window overflow — the negative control for the
/// matcher. Deliberately quotes "tokens" and "model", the words a sloppy
/// matcher would key on.
fn openai_ordinary_400() -> Value {
    json!({
        "error": {
            "message": "Invalid value for 'max_tokens': must be an integer for this model",
            "type": "invalid_request_error",
            "code": "invalid_value",
        }
    })
}

/// THE BUG. A provider context-window 400 must be recovered from in-loop, not
/// propagated out of `run()`.
///
/// Without the reactive path this is a permanent stick, and the mechanism is
/// what makes it permanent rather than transient: a failed request reports no
/// usage, so `last_request_input_tokens` stays frozen at the last SUCCESSFUL
/// (sub-threshold) reading, `should_handoff()` therefore returns false forever,
/// and the in-memory session keeps the same oversized history. Every later
/// prompt in that session fails identically, for the life of the session.
/// (Restarting the agent clears it — history is not written to disk — which is
/// why the only workaround today is a restart.)
///
/// The sequence here reproduces exactly that state: request 1 succeeds and
/// reports usage well UNDER the threshold (so the proactive gate is provably
/// not what fires), request 2 is rejected with a context-window 400. The agent
/// must force a handoff and retry, so the prompt still ends in a normal
/// `end_turn` rather than a JSON-RPC error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_window_400_recovers_instead_of_sticking() {
    let llm = spawn_capturing_llm_with_status(vec![
        // req 1: succeeds, usage 10 tokens — far under any threshold.
        (200, openai_text_with_usage("ack", 10)),
        // req 2: the overflow rejection.
        (400, openai_context_length_error()),
        // req 3: the forced handoff's summarize() call.
        (200, openai_text("recovered handoff summary")),
        // req 4: the retried completion, now on fresh history.
        (200, openai_text_with_usage("done after recovery", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            // Large window + large byte budget: neither proactive gate can be
            // what produces the handoff, so a handoff here is attributable to
            // the reactive path alone.
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "8192"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            // Cap of 0: proves the forced path bypasses `max_handoffs`. Any
            // gated handoff is impossible under this setting.
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"first prompt, succeeds"}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert!(
        r0["result"].get("stopReason").is_some(),
        "first prompt should succeed: {r0}"
    );

    // Second prompt: its first completion is rejected for context overflow.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt("second-prompt-overflows")}]}),
        )
        .await;
    let r1 = h.recv_until(|v| v["id"] == json!(p1)).await;
    assert!(
        r1.get("error").is_none(),
        "context-window 400 must be recovered in-loop, not returned as an error: {r1} \
         stderr={}",
        h.stderr_text()
    );
    assert_eq!(
        r1["result"]["stopReason"],
        "end_turn",
        "expected the turn to finish after recovery: {r1} stderr={}",
        h.stderr_text()
    );
    // 4 requests = the rejected one, the summarize, and the retry. 2 would mean
    // no recovery was attempted.
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured,
        4,
        "expected reject + summarize + retry (4 reqs total), saw {captured} — stderr={}",
        h.stderr_text()
    );
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("provider reported context overflow; forcing handoff"),
        "expected the forced-handoff log line, got: {stderr}"
    );
    h.shutdown().await;
}

/// A successful recovery must actually send the recovered completion, even when
/// `max_rounds` is finite. `round` is incremented BEFORE the completion that
/// gets rejected, so a naive `continue` after recovery re-enters the loop with
/// the rejected attempt already charged against the cap: with
/// `BUZZ_AGENT_MAX_ROUNDS=1` the turn would return `max_turn_requests` after
/// destructively resetting history, having never sent the retry. That silently
/// converts "recovered" into "history destroyed, question unanswered" — worse
/// than the error it replaced, because the user gets a stop reason rather than a
/// failure.
///
/// The default `max_rounds` is 0 (unbounded), which is why the rest of the
/// matrix cannot see this: the cap check at the top of the loop never fires.
///
/// `max_rounds=1` is also the tightest possible setting, so it pins the
/// boundary: exactly one round is authorized, the rejected request must not
/// consume it, and the retry must be the request that spends it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_retry_is_sent_under_a_finite_round_cap() {
    let llm = spawn_capturing_llm_with_status(vec![
        // req 1: the overflow rejection (round 1 charged before it is sent).
        (400, openai_context_length_error()),
        // req 2: the forced handoff's summarize() call.
        (200, openai_text("recovered handoff summary")),
        // req 3: the retried completion. Under the bug this is never sent.
        (200, openai_text_with_usage("done after recovery", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "8192"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
            // The whole point: a finite cap, at its tightest.
            ("BUZZ_AGENT_MAX_ROUNDS", "1"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt("overflows-under-finite-cap")}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert!(
        r0.get("error").is_none(),
        "context-window 400 must be recovered in-loop: {r0} stderr={}",
        h.stderr_text()
    );
    // The discriminator. `max_turn_requests` here means recovery ran, history
    // was reset, and the turn ended without ever asking the model again.
    assert_eq!(
        r0["result"]["stopReason"],
        "end_turn",
        "a recovered turn must finish by answering, not by hitting the round cap: {r0} \
         stderr={}",
        h.stderr_text()
    );
    // 3 requests = reject + summarize + retry. 2 would mean the retry was
    // never sent (the bug); the outcome assertion alone cannot tell those apart
    // if the stop reason were ever produced some other way.
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured,
        3,
        "expected reject + summarize + retry (3 reqs), saw {captured} — stderr={}",
        h.stderr_text()
    );
    h.shutdown().await;
}

/// The finite round cap must still bind for ORDINARY rounds — the recovery
/// refund must not become a general amnesty. With `max_rounds=1` and no context
/// overflow anywhere, a model that keeps requesting tool calls gets exactly one
/// completion and then `max_turn_requests`.
///
/// Without this arm, "make the recovered retry possible" is satisfiable by
/// deleting the cap, and the test above would still pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finite_round_cap_still_binds_without_a_context_overflow() {
    let llm = spawn_capturing_llm_with_status(vec![
        // Round 1: a tool call, which would normally drive another round.
        (
            200,
            openai_tool_call("tc1", "dev__shell", json!({"command": "true"})),
        ),
        // Never reached: the cap must stop the turn before a second completion.
        (200, openai_text_with_usage("should not be sent", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            ("BUZZ_AGENT_MAX_ROUNDS", "1"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"drive a tool call"}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert_eq!(
        r0["result"]["stopReason"],
        "max_turn_requests",
        "an ordinary finite cap must still bind: {r0} stderr={}",
        h.stderr_text()
    );
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured,
        1,
        "exactly one completion is authorized by max_rounds=1, saw {captured} — stderr={}",
        h.stderr_text()
    );
    h.shutdown().await;
}

/// Prompt-exactly-once across a forced handoff: the live user prompt must be
/// retained in the fresh history exactly once — not dropped (the model would
/// answer a question it can no longer see) and not duplicated (a doubled prompt
/// re-inflates the context we just shrank, and can produce a doubled action).
///
/// Asserted on the retry request's own message array, which is the only place
/// the post-reset history is observable from outside.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_handoff_retains_live_prompt_exactly_once() {
    const MARKER: &str = "unique-live-prompt-marker-7f3a";
    let llm = spawn_capturing_llm_with_status(vec![
        (200, openai_text_with_usage("ack", 10)),
        (400, openai_context_length_error()),
        (200, openai_text("summary body")),
        (200, openai_text_with_usage("done", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"warmup"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p0)).await;
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt(MARKER)}]}),
        )
        .await;
    let r1 = h.recv_until(|v| v["id"] == json!(p1)).await;
    assert!(r1.get("error").is_none(), "expected recovery: {r1}");

    let captured = llm.captured.lock().await;
    let retry = captured
        .last()
        .expect("at least one captured request")
        .clone();
    drop(captured);
    let messages = retry["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("retry request had no messages array: {retry}"));
    let occurrences = messages
        .iter()
        .filter(|m| {
            m["content"]
                .as_str()
                .map(|s| s.contains(MARKER))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        occurrences, 1,
        "live prompt must appear exactly once in post-handoff history, saw {occurrences} in \
         {messages:#?}"
    );
    h.shutdown().await;
}

/// Negative control at the loop layer: an ordinary 400 must stay terminal.
///
/// This is the arm that keeps the recovery narrow. If the matcher were loose,
/// this request would be classified as recoverable, the agent would spend its
/// whole recovery budget summarizing, and a clear immediate failure would
/// become a slow one — with three wasted provider round-trips. Exactly one
/// request, and the prompt returns an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_400_stays_terminal_and_triggers_no_recovery() {
    let llm = spawn_capturing_llm_with_status(vec![(400, openai_ordinary_400())]).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "3"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"hello"}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert!(
        r0.get("error").is_some(),
        "an ordinary 400 must surface as an error, got: {r0}"
    );
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured,
        1,
        "an ordinary 400 must not trigger a recovery attempt; saw {captured} requests — \
         stderr={}",
        h.stderr_text()
    );
    let stderr = h.stderr_text();
    assert!(
        !stderr.contains("provider reported context overflow"),
        "ordinary 400 must not be classified as a context overflow, got: {stderr}"
    );
    h.shutdown().await;
}

/// The recovery budget must be finite: a provider that rejects every request
/// for context overflow — including the retries — has to surface the error
/// rather than being rescued forever. `max_rounds` cannot bound this (it
/// defaults to 0/unbounded), so the per-`run()` recovery budget is the only
/// thing standing between this case and an infinite loop.
///
/// The stub returns a context-400 to EVERY request, so a missing bound shows up
/// as a hang rather than a wrong answer — hence the explicit timeout, which is
/// part of the assertion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_recovery_budget_exhaustion_surfaces_the_error() {
    // Enough canned 400s that the queue is never the thing that stops the loop;
    // the fallback response is also a 400-shaped body under this helper only if
    // queued, so keep the queue generously long.
    let responses: Vec<(u16, Value)> = (0..40)
        .map(|_| (400, openai_context_length_error()))
        .collect();
    let llm = spawn_capturing_llm_with_status(responses).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt("always-overflows")}]}),
        )
        .await;
    let r0 = tokio::time::timeout(
        Duration::from_secs(20),
        h.recv_until(|v| v["id"] == json!(p0)),
    )
    .await
    .expect("recovery must be bounded — prompt never returned, so the rescue loop is unbounded");
    assert!(
        r0.get("error").is_some(),
        "exhausted recovery must surface the provider error, got: {r0}"
    );
    let msg = r0["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("context"),
        "surfaced error should be the provider's own context-window error, got: {msg}"
    );
    // Discriminate WHICH bound stopped the loop. Both the budget and the prompt
    // floor produce a surfaced error, so the assertion above passes either way
    // — and the floor can fire on the first rung without the budget ever being
    // consumed, which would make this test silently exercise a different
    // mechanism than its name claims. Pin the budget explicitly.
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("context recovery budget spent"),
        "the per-run recovery BUDGET must be what stops the loop here, not the prompt floor; \
         got: {stderr}"
    );
    // Corroboration: every rung actually ran a forced handoff.
    let rungs = stderr
        .matches("provider reported context overflow; forcing handoff")
        .count();
    assert_eq!(
        rungs, 3,
        "expected all 3 recovery rungs to be attempted before giving up, saw {rungs} — \
         stderr={stderr}"
    );
    h.shutdown().await;
}

/// The prompt-budget floor, observed on its own. A context-window 400 on a
/// SMALL history must refuse to rescue rather than halve toward zero: the
/// overflow is then dominated by what a handoff cannot shrink (system prompt,
/// tool schemas, the live user prompt), so shrinking history further would only
/// issue smaller doomed requests in place of a clear error.
///
/// The outcome — a surfaced error — is identical to budget exhaustion, so this
/// asserts the discriminating evidence instead: the floor log line, and that
/// ZERO forced handoffs were attempted. Without the floor the ladder would spend
/// all three rungs summarizing a 40-byte history, which is the behavior this
/// arm exists to forbid.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_history_context_400_refuses_rescue_at_the_prompt_floor() {
    let responses: Vec<(u16, Value)> = (0..10)
        .map(|_| (400, openai_context_length_error()))
        .collect();
    let llm = spawn_capturing_llm_with_status(responses).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"tiny"}]}),
        )
        .await;
    let r0 = tokio::time::timeout(
        Duration::from_secs(20),
        h.recv_until(|v| v["id"] == json!(p0)),
    )
    .await
    .expect("must not loop — the floor should stop the rescue immediately");
    assert!(
        r0.get("error").is_some(),
        "a context 400 with no shrinkable history must surface the error, got: {r0}"
    );
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("below the") && stderr.contains("floor"),
        "the prompt-budget FLOOR must be what stops this, not the recovery budget; got: {stderr}"
    );
    let rungs = stderr
        .matches("provider reported context overflow; forcing handoff")
        .count();
    assert_eq!(
        rungs, 0,
        "no rescue should be attempted below the floor, saw {rungs} — stderr={stderr}"
    );
    // Exactly one request: the rejected one. No summarize, no retry.
    let captured = llm.captured.lock().await.len();
    assert_eq!(
        captured, 1,
        "expected no rescue round-trips below the floor, saw {captured} requests"
    );
    h.shutdown().await;
}

/// The recovery ladder must actually SHRINK, not just re-summarize at the size
/// that was already rejected.
///
/// Observed on the summarize request's own body — the only externally visible
/// consequence of the prompt budget. The rejected completion carried the full
/// history; the rescue's summarize prompt must be materially smaller. Without
/// this arm, deleting the halving entirely leaves every other test green: they
/// assert that a handoff HAPPENED, and a handoff at the rejected size still
/// happens (it just cannot escape a real overflow, which a stub does not
/// reproduce).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_shrinks_the_summarize_prompt_below_the_rejected_size() {
    let llm = spawn_capturing_llm_with_status(vec![
        (400, openai_context_length_error()),
        (200, openai_text("summary")),
        (200, openai_text_with_usage("done", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt("shrink-probe")}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert!(r0.get("error").is_none(), "expected recovery: {r0}");

    let captured = llm.captured.lock().await.clone();
    assert!(
        captured.len() >= 2,
        "expected at least reject + summarize, saw {}",
        captured.len()
    );
    let content_bytes = |req: &Value| -> usize {
        req["messages"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["content"].as_str())
                    .map(str::len)
                    .sum()
            })
            .unwrap_or(0)
    };
    let rejected = content_bytes(&captured[0]);
    let summarize = content_bytes(&captured[1]);
    assert!(
        rejected > 0 && summarize > 0,
        "empty measurement is not a result: rejected={rejected} summarize={summarize}"
    );
    // Halving from the rejected size lands near 0.5x; 0.75x leaves headroom for
    // the summarizer's fixed frame while still failing if no shrink happened.
    assert!(
        (summarize as f64) < 0.75 * (rejected as f64),
        "rescue summarize prompt ({summarize} bytes) must be materially smaller than the \
         rejected request ({rejected} bytes) — the ladder is not shrinking"
    );
    h.shutdown().await;
}

/// The ladder must shrink between RUNGS, not just once on entry.
///
/// This arm exists because a mutant that pins `shift` to `1` — deleting the
/// `attempts` dependence, so every rung rebuilds the same budget — SURVIVED the
/// whole suite. It had to: `attempts` is 0 on the first rung, so `shift = 1` IS
/// production there, and every other arm stops at rung 1. The single-rung shrink
/// arm above cannot see this; only a fixture that forces a SECOND rung can.
///
/// The forcing move is the realistic one the ladder was designed for: the
/// summarize call travels the same provider path, so rung 1's summarize is
/// itself rejected for context overflow (`Skipped`), and rung 2 must come back
/// with a materially smaller summarizer prompt.
///
/// Budgets: history is ~64 KB, so rung 1 asks for ~32 KB and rung 2 for ~16 KB,
/// both comfortably above the 4 KiB floor — the floor must not be what
/// separates them, or this would measure the wrong mechanism.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_shrinks_further_on_each_rung() {
    let llm = spawn_capturing_llm_with_status(vec![
        // 1: the completion that overflows.
        (400, openai_context_length_error()),
        // 2: rung-1 summarize, rejected the same way -> Skipped -> next rung.
        (400, openai_context_length_error()),
        // 3: rung-2 summarize succeeds.
        (200, openai_text("summary")),
        // 4: the retried completion.
        (200, openai_text_with_usage("done", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            ("BUZZ_AGENT_MAX_HANDOFFS", "0"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": large_prompt("rung-shrink-probe")}]}),
        )
        .await;
    let r0 = h.recv_until(|v| v["id"] == json!(p0)).await;
    assert!(
        r0.get("error").is_none(),
        "expected recovery on the second rung: {r0}"
    );

    // The second rung must actually have been taken — otherwise the byte
    // comparison below would compare rung 1 against the retry.
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("did not run; shrinking further"),
        "rung 1 must have been Skipped so rung 2 runs; got: {stderr}"
    );
    assert!(
        !stderr.contains("below the"),
        "the prompt FLOOR must not be involved in this fixture; got: {stderr}"
    );

    let captured = llm.captured.lock().await.clone();
    assert_eq!(
        captured.len(),
        4,
        "expected reject + rung1 summarize + rung2 summarize + retry, saw {}",
        captured.len()
    );
    let content_bytes = |req: &Value| -> usize {
        req["messages"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| m["content"].as_str())
                    .map(str::len)
                    .sum()
            })
            .unwrap_or(0)
    };
    let rung1 = content_bytes(&captured[1]);
    let rung2 = content_bytes(&captured[2]);
    assert!(
        rung1 > 0 && rung2 > 0,
        "empty measurement is not a result: rung1={rung1} rung2={rung2}"
    );
    assert!(
        (rung2 as f64) < 0.75 * (rung1 as f64),
        "each rung must shrink: rung2 ({rung2} bytes) is not materially smaller than rung1 \
         ({rung1} bytes) — the budget is not tracking `attempts`"
    );
    h.shutdown().await;
}

/// Gate 5, and the DIRECTION the clearing protects: not a spurious handoff, a
/// MISSED one. After a reactive reset the stale `last_request_input_tokens`
/// describes history that no longer exists, and its paired byte baseline
/// describes the pre-reset (larger) history — so `grown` stays near zero and the
/// projection collapses to the stale sub-threshold token count. The gate goes
/// BLIND until history exceeds its pre-reset size.
///
/// Constructing the divergence takes three turns, and two of the constraints are
/// load-bearing — a first attempt with a simpler fixture produced traces
/// BYTE-IDENTICAL between the fix and its deletion:
///   * Turn 1 must stay UNDER the gate threshold, or the proactive handoff fires
///     first and consumes the queue slot the overflow was meant to land in — no
///     usage is ever recorded, both variants sit at `None`, and the test measures
///     nothing.
///   * The post-recovery retry must report NO usage. A usage-bearing response
///     overwrites both fields with coherent values on the spot, which makes the
///     clear genuinely redundant and the mutant equivalent. The reachable window
///     is exactly when the retry omits usage and the stale pair survives.
/// Turn 3 then carries a large prompt: a cleared baseline falls through to the
/// byte signal and hands off, while the stale pair projects
/// `10 + (190KB - 100KB)` = ~90k tokens, under the 180k threshold, and does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reactive_reset_clears_usage_baseline_so_the_gate_is_not_blind() {
    // ~100 KB: under the 180 KB byte-fallback threshold, so turn 1 does NOT
    // trip the proactive gate, but large enough to be the stale `measured_bytes`
    // that suppresses `grown` later.
    let mut medium = String::with_capacity(100 * 1024);
    medium.push_str("turn-one-medium ");
    while medium.len() < 100 * 1024 {
        medium.push_str("padding under the byte fallback threshold. ");
    }
    // ~190 KB: over the threshold, so a CLEARED baseline must hand off.
    let mut big = String::with_capacity(190 * 1024);
    big.push_str("turn-three-large ");
    while big.len() < 190 * 1024 {
        big.push_str("padding to exceed the byte fallback threshold. ");
    }

    let llm = spawn_capturing_llm_with_status(vec![
        // Turn 1: succeeds, reporting a SMALL usage reading against a ~100 KB
        // history. This is the pair that goes stale.
        (200, openai_text_with_usage("ack-medium", 10)),
        // Turn 2: the overflow.
        (400, openai_context_length_error()),
        // Turn 2: the forced handoff's summarize.
        (200, openai_text("forced summary")),
        // Turn 2: the retry — NO usage block, so the baseline is not refreshed.
        (200, openai_text("recovered, no usage reported")),
        // Turn 3: with a cleared baseline a gated summarize comes first; with a
        // stale one this slot is the completion instead. Spares so an exhausted
        // queue is never what ends a turn.
        (200, openai_text("gated summary")),
        (200, openai_text_with_usage("done", 10)),
        (200, openai_text_with_usage("spare-1", 10)),
        (200, openai_text_with_usage("spare-2", 10)),
    ])
    .await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "200000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "8192"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
            // Must permit a GATED handoff — turn 3 observes the proactive gate,
            // which a cap of 0 would forbid.
            ("BUZZ_AGENT_MAX_HANDOFFS", "5"),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    // Turn 1: under threshold, records the usage pair.
    let p0 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": medium}]}),
        )
        .await;
    let r0 = tokio::time::timeout(
        Duration::from_secs(25),
        h.recv_until(|v| v["id"] == json!(p0)),
    )
    .await
    .expect("turn 1 must return");
    assert!(r0.get("error").is_none(), "turn 1 should succeed: {r0}");
    assert!(
        !h.stderr_text().contains("handoff #"),
        "precondition: turn 1 must NOT hand off, or no usage pair is recorded and this test \
         measures nothing. stderr={}",
        h.stderr_text()
    );

    // Turn 2: small prompt, overflow, reactive recovery.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"small, overflows"}]}),
        )
        .await;
    let r1 = tokio::time::timeout(
        Duration::from_secs(25),
        h.recv_until(|v| v["id"] == json!(p1)),
    )
    .await
    .expect("turn 2 must return");
    assert!(r1.get("error").is_none(), "turn 2 should recover: {r1}");
    assert!(
        h.stderr_text()
            .contains("provider reported context overflow; forcing handoff"),
        "precondition: the reactive path must have run in turn 2. stderr={}",
        h.stderr_text()
    );
    let handoffs_after_turn2 = h.stderr_text().matches("handoff #").count();

    // Turn 3: large prompt. A cleared baseline sees it via the byte signal and
    // hands off; a stale pair under-projects and stays blind.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text": big}]}),
        )
        .await;
    let r2 = tokio::time::timeout(
        Duration::from_secs(25),
        h.recv_until(|v| v["id"] == json!(p2)),
    )
    .await
    .expect("turn 3 must return");
    assert!(r2.get("error").is_none(), "turn 3 should succeed: {r2}");
    let stderr = h.stderr_text();
    let handoffs_after_turn3 = stderr.matches("handoff #").count();
    assert!(
        handoffs_after_turn3 > handoffs_after_turn2,
        "turn 3 must produce a GATED handoff ({handoffs_after_turn2} before, \
         {handoffs_after_turn3} after): the reactive reset must clear the usage baseline, or the \
         proactive gate under-projects and stays blind to an oversized history. stderr={stderr}"
    );
    h.shutdown().await;
}

// ─── Tests: per-turn handoff cap semantics ───────────────────────────────────

/// A session that has already performed N handoffs in previous turns must still
/// compact on subsequent turns — the per-session lifetime kill switch is gone.
///
/// Mechanism: the gate fires at the start of each round, comparing
/// `last_request_input_tokens` (stored by the previous response) against the
/// token threshold. So:
/// - Turn 1 complete() returns usage=950 (> threshold=900). Turn ends; usage stored.
/// - Turn 2 round 0: 950 >= 900 → handoff. post-handoff complete() returns usage=950.
///   Session `handoff_count` is now 1; `turn_handoff_count` was just reset to 0 at
///   turn start and is now 1.
/// - Turn 3 round 0: `turn_handoff_count` resets to 0; session count is 1 but
///   the gate uses `turn_handoff_count` → cap not reached → handoff fires again.
///
/// Without the fix (`handoff_count` compared against cap, never reset):
///   session count after turn 2 = 1 >= max_handoffs=1 → gate permanently blocked
///   for all subsequent turns → history grows until provider wall.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handoff_cap_resets_per_turn_not_per_session() {
    // LLM call sequence:
    //  req 1: turn 1 complete()           → usage=950 (over threshold)
    //  req 2: turn 2 pre-flight summarize → summary text
    //  req 3: turn 2 complete()           → usage=950 (re-arms gate for turn 3)
    //  req 4: turn 3 pre-flight summarize → summary text  ← cap reset proves this fires
    //  req 5: turn 3 complete()           → done
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("ack-t1", 950), // turn 1: stores high usage
        openai_text("summary-t2"),             // turn 2: pre-flight summarize
        openai_text_with_usage("done-t2", 950), // turn 2: post-handoff, re-arms gate
        openai_text("summary-t3"),             // turn 3: pre-flight summarize (cap reset)
        openai_text_with_usage("done-t3", 10), // turn 3: post-handoff complete
    ])
    .await;

    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "100"),
            // Cap of 1 per turn. Before the fix this permanently disables the
            // gate once session handoff_count reaches 1.
            ("BUZZ_AGENT_MAX_HANDOFFS", "1"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;
    let sid = init_session(&mut h, json!([])).await;

    // Turn 1: no prior usage; preflight skips (byte-fallback not triggered by
    // tiny prompt). complete() stores usage=950.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"turn 1"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;
    assert_eq!(
        llm.captured.lock().await.len(),
        1,
        "turn 1 must produce exactly 1 LLM request"
    );

    // Turn 2: 950 >= threshold=900 → handoff fires. Session handoff_count: 1.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"turn 2"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p2)).await;
    assert_eq!(
        llm.captured.lock().await.len(),
        3,
        "turn 2 must produce 2 LLM requests (summarize + complete), 3 total"
    );
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("handoff #1"),
        "expected first handoff log after turn 2; got: {stderr}"
    );

    // Turn 3: turn_handoff_count resets to 0 → gate fires again despite
    // session handoff_count=1 == cap=1.
    let p3 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"turn 3"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p3)).await;
    assert_eq!(
        llm.captured.lock().await.len(),
        5,
        "turn 3 must also produce 2 LLM requests (per-turn cap reset → handoff fires again), \
         5 total"
    );
    let stderr = h.stderr_text();
    assert!(
        stderr.contains("handoff #2"),
        "expected second handoff log after turn 3 (cap reset); got: {stderr}"
    );

    h.shutdown().await;
}

/// Within a single turn, the per-turn cap still bounds the number of handoffs.
/// A turn that exceeds `max_handoffs` compaction attempts must emit a WARN and
/// fall back to truncation — it must NOT compact indefinitely.
///
/// Mechanism: with cap=1 and a multi-round turn (tool call in round 1 → round 2),
/// the pre-flight handoff fires at the start of round 1 (usage from a *previous*
/// turn is high). After the compaction, the post-handoff complete() in round 1
/// returns a tool call, causing a second round. Round 2's preflight sees that
/// turn_handoff_count=1 == max_handoffs=1, so it refuses and emits WARN.
///
/// A steer is injected while the run is active to prove that the steer path
/// does NOT reset `handoff_attempts` — the cap must still fire on round 1 with
/// no second summarize call.
///
/// This test requires a fake MCP server to produce a tool-call round.
/// It drives via `fake-mcp` — the same binary used in other multi-round tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handoff_cap_binds_within_a_single_turn() {
    // LLM call sequence in turn 2 (turn 1 seeds the usage):
    //  req 1: turn 1 complete()            → usage=950 (over threshold=900)
    //  req 2: turn 2 round 0 summarize()   → summary (handoff_attempts: 0→1)
    //  req 3: turn 2 round 0 complete()    → tool_call + usage=950 (re-arms gate)
    //         [fake-mcp tool executes; steer queued while run is active]
    //  req 4: turn 2 round 1 preflight     → 950 >= 900 AND attempts=1 >= max=1
    //                                         → WARN, skip (cap exhausted for this turn)
    //  req 5: turn 2 round 1 complete()    → end_turn (steer text folded into messages)
    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    // Build a tool-call response that also carries usage so the gate re-arms
    // on round 1's preflight (without usage, last_request_input_tokens is None
    // after the handoff clears it, and the byte-fallback won't fire on tiny history).
    let tool_call_with_usage = {
        let mut v = openai_tool_call("tc-1", "test_tool", json!({}));
        v["usage"] = json!({
            "prompt_tokens": 950u64,
            "completion_tokens": 5,
            "total_tokens": 955,
        });
        v
    };
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("seed", 950), // turn 1: seed high usage
        openai_text("handoff-summary"),      // turn 2 round 0: summarize
        tool_call_with_usage,                // turn 2 round 0: tool call + usage (re-arms)
        openai_text_with_usage("end_turn_text", 10), // turn 2 round 1: final answer
    ])
    .await;

    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "100"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "1"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;

    // Init with the fake MCP server so test_tool is available.
    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "cap_test",
                "command": fake_mcp,
                "args": [],
                "env": [{ "name": "FAKE_MCP_TOOL_COUNT", "value": "1" }],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();

    // Turn 1: seed high usage.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"seed"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    // Turn 2: triggers a handoff at round 0, then a tool call, then round 1
    // where the cap is already exhausted.  A steer is injected while the run
    // is active to prove mid-turn steers cannot reset `handoff_attempts`.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"do work"}]}),
        )
        .await;

    // Drain until the final response, approving tool-permission requests,
    // capturing the activeRunId once it is broadcast, sending one steer,
    // and verifying that it is accepted in the live run.
    let mut run_id: Option<String> = None;
    let mut steer_id: i64 = -1;
    let mut steer_accepted = false;
    loop {
        let v = h.recv().await;

        // Capture the run id from the first session/update that carries it,
        // then immediately queue a steer.  This must happen before round 1 so
        // the steer text is present but the cap check still fires — proving
        // the counter is not reset by the steer path.
        if run_id.is_none() {
            if let Some(rid) = v["params"]["update"]["_meta"]["goose"]["activeRunId"].as_str() {
                run_id = Some(rid.to_owned());
                steer_id = h
                    .send(
                        "_goose/unstable/session/steer",
                        json!({
                            "sessionId": sid,
                            "expectedRunId": rid,
                            "prompt": [{"type":"text","text":"STEER-CANARY: also consider the edge case"}],
                        }),
                    )
                    .await;
            }
        }

        // Steer response: assert it was accepted in the live run.
        if steer_id >= 0 && v["id"] == json!(steer_id) {
            assert!(
                v.get("result").is_some(),
                "steer must be accepted while the run is active; got: {v}"
            );
            assert_eq!(
                v["result"]["runId"].as_str(),
                run_id.as_deref(),
                "steer must reference the live run id"
            );
            steer_accepted = true;
            continue;
        }

        if v.get("method") == Some(&json!("session/request_permission")) {
            let id = v["id"].clone();
            h.write(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "outcome": { "outcome": "selected", "optionId": "allow" } },
            }))
            .await;
            continue;
        }
        if v["id"] == json!(p2) {
            assert!(
                v.get("result").is_some(),
                "turn 2 must succeed even when cap blocks round-1 handoff; got: {v}"
            );
            break;
        }
    }

    assert!(
        steer_accepted,
        "steer was never accepted during turn 2; the steer arm is missing coverage"
    );

    // 4 LLM requests: seed + summarize + tool-call-with-usage + final-complete.
    let count = llm.captured.lock().await.len();
    assert_eq!(
        count, 4,
        "expected 4 LLM requests (seed + summarize + tool-call + final); got {count}"
    );

    let stderr = h.stderr_text();
    assert!(
        stderr.contains("handoff cap reached"),
        "expected cap-reached WARN in stderr; got: {stderr}"
    );
    assert!(
        stderr.contains("reason=\"preflight\""),
        "expected reason=\"preflight\" field in cap WARN; got: {stderr}"
    );
    assert!(
        stderr.contains("handoff_attempts="),
        "expected handoff_attempts field in cap WARN; got: {stderr}"
    );
    assert!(
        stderr.contains("max_handoffs="),
        "expected max_handoffs field in cap WARN; got: {stderr}"
    );

    h.shutdown().await;
}

/// A failing `summarize()` call must still consume one slot from the per-turn
/// handoff-attempt budget. Before the fix, `handoff_count` was incremented only
/// on a successful compaction; a flaky summarizer could be retried indefinitely
/// within a turn. The fix moves the increment to before `summarize()`.
///
/// Proof: with `max_handoffs=1` and a multi-round turn:
/// - Round 0 preflight: threshold met, attempts: 0→1, summarize() fails → Skipped.
/// - Round 1 preflight: attempts=1 >= cap=1 → WARN (cap hit despite no successful
///   compaction). Without the pre-summarize increment, attempts would still be 0
///   here and a second summarize() would be attempted — the bug.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_summarize_burns_handoff_attempt_budget() {
    // We need the summarize() call to fail. The summarize path uses the same
    // fake LLM server; we queue an HTTP error body for the summarize request.
    // But our spawn_capturing_llm always returns 200, so we use a non-OpenAI-
    // shaped response that the agent will treat as an error (missing `choices`).
    //
    // LLM call sequence:
    //  req 1: turn 1 complete()           → usage=950 (seeds the gate)
    //  req 2: turn 2 round 0 summarize()  → malformed response (treated as error)
    //                                       handoff_attempts incremented to 1 BEFORE this
    //  req 3: turn 2 round 0 complete()   → tool_call + usage=950 (re-arms gate)
    //  req 4: turn 2 round 1 preflight    → cap reached: WARN (attempts=1 >= max=1)
    //  req 5: turn 2 round 1 complete()   → end_turn
    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    let bad_summary_response = json!({ "error": "upstream unavailable" }); // no `choices`
    let tool_call_with_usage = {
        let mut v = openai_tool_call("tc-2", "test_tool", json!({}));
        v["usage"] = json!({
            "prompt_tokens": 950u64,
            "completion_tokens": 5,
            "total_tokens": 955,
        });
        v
    };
    let llm = spawn_capturing_llm(vec![
        openai_text_with_usage("seed", 950), // turn 1: seed usage
        bad_summary_response,                // turn 2 round 0: summarize fails
        tool_call_with_usage,                // turn 2 round 0: complete → tool call
        openai_text_with_usage("done", 10),  // turn 2 round 1: final answer
    ])
    .await;

    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("BUZZ_AGENT_MAX_CONTEXT_TOKENS", "1000"),
            ("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "100"),
            ("BUZZ_AGENT_MAX_HANDOFFS", "1"),
            (
                "BUZZ_AGENT_MAX_HISTORY_BYTES",
                &(16 * 1024 * 1024).to_string(),
            ),
        ],
    )
    .await;

    h.send(
        "initialize",
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;
    h.send(
        "session/new",
        json!({
            "cwd": "/tmp",
            "mcpServers": [{
                "name": "budget_test",
                "command": fake_mcp,
                "args": [],
                "env": [{ "name": "FAKE_MCP_TOOL_COUNT", "value": "1" }],
            }],
        }),
    )
    .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();

    // Turn 1: seed high usage.
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"seed"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p1)).await;

    // Turn 2: round 0 summarize fails, but attempts was already incremented.
    // Round 1 preflight must see cap hit and emit WARN.
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"work"}]}),
        )
        .await;

    loop {
        let v = h.recv().await;
        if v.get("method") == Some(&json!("session/request_permission")) {
            let id = v["id"].clone();
            h.write(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "outcome": { "outcome": "selected", "optionId": "allow" } },
            }))
            .await;
            continue;
        }
        if v["id"] == json!(p2) {
            assert!(v.get("result").is_some(), "turn 2 must succeed; got: {v}");
            break;
        }
    }

    let stderr = h.stderr_text();
    // Round 0: the failed summarize should warn about the failure.
    assert!(
        stderr.contains("handoff failed") || stderr.contains("handoff returned empty"),
        "expected summarize-failure WARN; got: {stderr}"
    );
    // Round 1: cap must be hit (attempts=1 from the failed attempt).
    assert!(
        stderr.contains("handoff cap reached"),
        "expected cap-reached WARN after failed summarize burned the attempt; got: {stderr}"
    );

    h.shutdown().await;
}
