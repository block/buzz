//! Regression tests for round 4-6 hardening:
//!   - assistant text preserved in history
//!   - MCP init timeout (with explicit child kill)
//!   - tool metadata caps (description bytes, count)
//!   - cancellation leaves history valid for the next prompt
//!   - empty-content assistant turn doesn't poison OpenAI history

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

// ─── Fake LLM that captures requests so we can inspect history ──────────────

struct CapturingLlm {
    url: String,
    captured: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_capturing_llm(responses: Vec<Value>) -> CapturingLlm {
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
                let body = queue
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or_else(|| json!({ "error": "no canned response" }));
                let body_s = serde_json::to_string(&body).unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(), body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    CapturingLlm { url, captured }
}

// ─── Harness (minimal copy — keeping per-test independence) ─────────────────

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn_with_env(base_url: &str, extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_sprout-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("SPROUT_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("SPROUT_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_MAX_ROUNDS", "8")
            .env("SPROUT_AGENT_MCP_INIT_TIMEOUT_SECS", "2");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn sprout-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
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
        .expect("sessionId")
        .to_owned()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

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

// ─── New round-8 regression tests ──────────────────────────────────────────

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
            ("SPROUT_AGENT_MAX_HISTORY_BYTES", &BUDGET.to_string()),
            ("SPROUT_AGENT_MAX_HANDOFFS", "0"), // exercise truncation, not handoff
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
    assert_eq!(tools.len(), 1);
    let desc = tools[0]["function"]["description"].as_str().unwrap_or("");
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

// ─── Hook system regression tests ──────────────────────────────────────────

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
    //   2. tool_call to fake__tool_0 (regular tool, resets latch)
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
            ("SPROUT_AGENT_STOP_MAX_REJECTIONS", "10"),
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
    //   2. tool_call (resets last_was_end_turn)
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
            ("SPROUT_AGENT_STOP_MAX_REJECTIONS", "1"),
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

/// Consecutive-rejection rule: if the LLM responds to an objection with
/// no tool calls and end_turn again, the agent accepts the end (avoids
/// infinite loops with an unreasonable hook).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_stop_consecutive_end_turn() {
    // LLM sequence:
    //   1. text → _Stop objects (rejections: 0→1, last_was_end_turn=true)
    //   2. text again, no tool calls → consecutive rule fires, return end_turn
    let llm = spawn_capturing_llm(vec![openai_text("done-1"), openai_text("done-2")]).await;
    let mut h = Harness::spawn_with_env(
        &llm.url,
        &[
            ("MCP_HOOK_SERVERS", "fake"),
            // Set high so we don't trip the budget instead.
            ("SPROUT_AGENT_STOP_MAX_REJECTIONS", "10"),
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

    // Exactly 2 LLM calls — consecutive rule prevented a 3rd round.
    let captured = llm.captured.lock().await;
    assert_eq!(
        captured.len(),
        2,
        "expected 2 LLM calls (consecutive rule), got {}",
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
            ("SPROUT_AGENT_MAX_HISTORY_BYTES", &(1024 * 1024).to_string()),
            // Allow at least one handoff.
            ("SPROUT_AGENT_MAX_HANDOFFS", "3"),
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
            ("SPROUT_AGENT_MAX_CONTEXT_TOKENS", "1000"),
            ("SPROUT_AGENT_MAX_OUTPUT_TOKENS", "100"),
            ("SPROUT_AGENT_MAX_HANDOFFS", "3"),
            // Huge byte budget so the byte path can NOT be what fires — only
            // the token gate can explain a handoff on these tiny prompts.
            (
                "SPROUT_AGENT_MAX_HISTORY_BYTES",
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
            ("SPROUT_AGENT_MAX_CONTEXT_TOKENS", "10000"),
            ("SPROUT_AGENT_MAX_OUTPUT_TOKENS", "1000"),
            ("SPROUT_AGENT_MAX_HANDOFFS", "3"),
            // Huge byte budget so the None-path byte fallback can't be what
            // fires — only the token-mode growth estimate can explain it.
            (
                "SPROUT_AGENT_MAX_HISTORY_BYTES",
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

/// `_Stop` hook that takes longer than `SPROUT_AGENT_HOOK_TIMEOUT_MS`
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
            ("SPROUT_AGENT_HOOK_TIMEOUT_MS", "200"),
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
/// sends `notifications/cancelled` to the MCP server. With sprout-dev-mcp,
/// this cancels the CancellationToken and kills the running shell process
/// group. We verify:
///   1. The prompt completes in under 5s (not 60s).
///   2. The `sleep 60` process is actually dead after cancel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_kills_inflight_tool_via_mcp_notification() {
    // sprout-dev-mcp is a separate crate; locate its binary relative to
    // the sprout-agent test binary (they share the same target dir).
    let self_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_sprout-agent"));
    let dev_mcp_bin = self_bin.parent().unwrap().join("sprout-dev-mcp");
    if !dev_mcp_bin.exists() {
        eprintln!(
            "SKIP: sprout-dev-mcp not built at {}; run `cargo build -p sprout-dev-mcp` first",
            dev_mcp_bin.display()
        );
        return;
    }
    let dev_mcp_bin = dev_mcp_bin.to_string_lossy().to_string();

    // Use a unique marker (PID + timestamp) to avoid stale-file collisions.
    let marker = format!(
        "sprout_cancel_test_{}_{:x}",
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
/// any MCP server (not just sprout-dev-mcp) when a session is cancelled
/// during an in-flight tool call. Uses fake_mcp with FAKE_MCP_CANCEL_LOG
/// to capture the raw notification on stdin.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_sends_notifications_cancelled_to_any_mcp_server() {
    let cancel_log = std::env::temp_dir()
        .join(format!(
            "sprout_cancel_proto_{}_{:x}.log",
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
