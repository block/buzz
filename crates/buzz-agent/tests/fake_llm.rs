//! Integration test: fake LLM HTTP server + buzz-agent subprocess.
//!
//! Drives the agent through the ACP wire protocol and verifies:
//!   - initialize / session/new responses
//!   - tool_call (pending) → request_permission → tool_call_update
//!   - session/prompt response with stopReason=end_turn
//!   - concurrent prompt rejection

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

async fn spawn_fake_llm(responses: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.len() > 1_000_000 {
                        return;
                    }
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
    url
}

/// Like `spawn_fake_llm` but also captures the full JSON request body from each
/// incoming HTTP request. Returns (url, captured_requests).
async fn spawn_capturing_fake_llm(responses: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let captures: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let captures_clone = captures.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            let captures = captures_clone.clone();
            tokio::spawn(async move {
                // Read headers.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.len() > 2_000_000 {
                        return;
                    }
                }
                // Parse Content-Length from headers to read the body.
                let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let header_str = String::from_utf8_lossy(&buf[..header_end]);
                let content_length: usize = header_str
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_lowercase();
                        if lower.starts_with("content-length:") {
                            lower
                                .trim_start_matches("content-length:")
                                .trim()
                                .parse()
                                .ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);

                // Collect body bytes (some may already be in buf after headers).
                let mut body_buf = buf[header_end..].to_vec();
                while body_buf.len() < content_length {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body_buf.extend_from_slice(&tmp[..n]),
                    }
                }

                // Parse and store the request body.
                if let Ok(parsed) =
                    serde_json::from_slice::<Value>(&body_buf[..content_length.min(body_buf.len())])
                {
                    captures.lock().await.push(parsed);
                }

                // Send canned response.
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
    (url, captures)
}

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn(base_url: &str) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("BUZZ_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_MAX_ROUNDS", "4")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn buzz-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    async fn write(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(10), self.stdout.read_line(&mut line))
            .await
            .expect("recv timeout")
            .expect("read line");
        assert!(n > 0, "agent EOF");
        serde_json::from_str(&line).expect("non-JSON line")
    }

    /// Read messages until one matches `pred`.
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

async fn init_session(h: &mut Harness) -> String {
    h.send(
        "initialize",
        json!({"protocolVersion":2,"clientCapabilities":{}}),
    )
    .await;
    let r = h.recv().await;
    assert_eq!(r["result"]["protocolVersion"], 2);
    assert_eq!(r["result"]["agentInfo"]["name"], "buzz-agent");
    h.send("session/new", json!({"cwd":"/tmp","mcpServers":[]}))
        .await;
    let r = h.recv().await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();
    assert!(sid.starts_with("ses_"));
    sid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_only_end_turn() {
    let url = spawn_fake_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;
    let p_id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "hi" }],
            }),
        )
        .await;
    let v = h.recv_until(|v| v["id"] == json!(p_id)).await;
    assert_eq!(v["result"]["stopReason"], "end_turn");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_call_then_end_turn() {
    // Round 1: tool call (will fail with "unknown tool" since no MCP registered).
    // Round 2: text response → end_turn.
    let url = spawn_fake_llm(vec![
        openai_tool_call("call_xyz", "fake__do_thing", json!({"foo": "bar"})),
        openai_text("ok"),
    ])
    .await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;
    let p_id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{"type":"text","text":"do something"}],
            }),
        )
        .await;

    // Tool unknown: agent emits failed tool_call_update directly (no permission ask).
    let v = h
        .recv_until(|v| {
            v.get("method") == Some(&json!("session/update"))
                && v["params"]["update"]["sessionUpdate"] == "tool_call_update"
                && v["params"]["update"]["status"] == "failed"
        })
        .await;
    assert_eq!(v["params"]["update"]["toolCallId"], "call_xyz");

    // Final response.
    let v = h.recv_until(|v| v["id"] == json!(p_id)).await;
    assert_eq!(v["result"]["stopReason"], "end_turn");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_concurrent_prompts() {
    // Slow first response so the second prompt arrives mid-flight.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let body = openai_text("done").to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    });

    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;
    let p1 = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid, "prompt": [{"type":"text","text":"go"}],
            }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let p2 = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid, "prompt": [{"type":"text","text":"go again"}],
            }),
        )
        .await;

    let mut saw_p2_err = false;
    let mut saw_p1_ok = false;
    for _ in 0..10 {
        let v = h.recv().await;
        if v["id"] == json!(p2) {
            assert_eq!(v["error"]["code"], -32602);
            saw_p2_err = true;
        } else if v["id"] == json!(p1) {
            assert_eq!(v["result"]["stopReason"], "end_turn");
            saw_p1_ok = true;
        }
        if saw_p1_ok && saw_p2_err {
            break;
        }
    }
    assert!(saw_p2_err, "expected concurrent prompt rejection");
    assert!(saw_p1_ok, "first prompt didn't complete");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejects_oversized_line() {
    // Set a tiny max line and send something larger; agent must abort with an
    // io error and not OOM.
    let url = spawn_fake_llm(vec![]).await;
    let bin = env!("CARGO_BIN_EXE_buzz-agent");
    let mut cmd = tokio::process::Command::new(bin);
    cmd.env("BUZZ_AGENT_PROVIDER", "openai")
        .env("OPENAI_COMPAT_API_KEY", "test")
        .env("OPENAI_COMPAT_MODEL", "fake-model")
        .env("OPENAI_COMPAT_BASE_URL", &url)
        .env("BUZZ_AGENT_MAX_LINE_BYTES", "256")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // 1024-byte line — agent should reject and exit.
    let big = "x".repeat(1024);
    let _ = stdin.write_all(big.as_bytes()).await;
    let _ = stdin.write_all(b"\n").await;
    drop(stdin);
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("agent didn't exit after oversized line");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_new_rejects_oversized_system_prompt() {
    // A systemPrompt exceeding 512KB must produce a JSON-RPC error, not a panic.
    let url = spawn_fake_llm(vec![]).await;
    let mut h = Harness::spawn(&url).await;
    h.send(
        "initialize",
        json!({"protocolVersion":2,"clientCapabilities":{}}),
    )
    .await;
    let r = h.recv().await;
    assert_eq!(r["result"]["protocolVersion"], 2);

    // 600KB payload — exceeds the 512KB limit.
    let big_prompt = "x".repeat(600 * 1024);
    let id = h
        .send(
            "session/new",
            json!({"cwd":"/tmp","mcpServers":[],"systemPrompt": big_prompt}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(id)).await;
    assert!(
        r.get("error").is_some(),
        "expected JSON-RPC error for oversized systemPrompt, got: {r}"
    );
    let err_msg = r["error"]["message"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("512KB limit"),
        "error message should mention 512KB limit, got: {err_msg}"
    );
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn system_prompt_reaches_llm_system_role() {
    // Proves the full contract: systemPrompt sent via session/new → agent appends
    // it to the effective system prompt → LLM receives it in the system role.
    let canary = "CANARY_E2E_TEST_MARKER_7f3a9b";
    let (url, captures) = spawn_capturing_fake_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn(&url).await;

    // initialize.
    h.send(
        "initialize",
        json!({"protocolVersion":2,"clientCapabilities":{}}),
    )
    .await;
    let r = h.recv().await;
    assert_eq!(r["result"]["protocolVersion"], 2);

    // session/new with systemPrompt containing the canary.
    let sn_id = h
        .send(
            "session/new",
            json!({"cwd":"/tmp","mcpServers":[],"systemPrompt": canary}),
        )
        .await;
    let r = h.recv_until(|v| v["id"] == json!(sn_id)).await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();
    assert!(sid.starts_with("ses_"));

    // session/prompt — triggers the LLM call.
    let p_id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{"type":"text","text":"hello"}],
            }),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p_id)).await;

    // Inspect the captured LLM request.
    let reqs = captures.lock().await;
    assert!(!reqs.is_empty(), "expected at least one LLM request");
    let llm_req = &reqs[0];
    let messages = llm_req["messages"].as_array().expect("messages array");

    // First message should be the system role.
    let system_msg = &messages[0];
    assert_eq!(
        system_msg["role"], "system",
        "first message must be system role"
    );
    let system_content = system_msg["content"].as_str().unwrap_or("");

    // Canary must appear in the system message (proves systemPrompt was used as base).
    assert!(
        system_content.contains(canary),
        "system message must contain the canary string.\nGot: {system_content}"
    );

    // The agent's default prompt must NOT appear — it is suppressed when
    // the harness provides a systemPrompt.
    let default_prompt = "You are buzz-agent";
    assert!(
        !system_content.contains(default_prompt),
        "system message must NOT contain the default prompt when systemPrompt is provided.\nGot: {system_content}"
    );

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn system_prompt_absent_no_canary() {
    // Negative case: when systemPrompt is NOT sent in session/new, the canary
    // must NOT appear in the LLM system message.
    let canary = "CANARY_E2E_TEST_MARKER_7f3a9b";
    let (url, captures) = spawn_capturing_fake_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn(&url).await;

    // initialize.
    h.send(
        "initialize",
        json!({"protocolVersion":2,"clientCapabilities":{}}),
    )
    .await;
    let _ = h.recv().await;

    // session/new WITHOUT systemPrompt field.
    let sn_id = h
        .send("session/new", json!({"cwd":"/tmp","mcpServers":[]}))
        .await;
    let r = h.recv_until(|v| v["id"] == json!(sn_id)).await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();

    // session/prompt — triggers the LLM call.
    let p_id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{"type":"text","text":"hello"}],
            }),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p_id)).await;

    // Inspect the captured LLM request.
    let reqs = captures.lock().await;
    assert!(!reqs.is_empty(), "expected at least one LLM request");
    let llm_req = &reqs[0];
    let messages = llm_req["messages"].as_array().expect("messages array");
    let system_msg = &messages[0];
    assert_eq!(system_msg["role"], "system");
    let system_content = system_msg["content"].as_str().unwrap_or("");

    // Canary must NOT appear (it was never sent).
    assert!(
        !system_content.contains(canary),
        "system message must NOT contain canary when systemPrompt is absent.\nGot: {system_content}"
    );

    // But the agent's default prompt should still be there.
    assert!(
        system_content.contains("You are buzz-agent"),
        "system message must still contain the agent's default prompt"
    );

    h.shutdown().await;
}
