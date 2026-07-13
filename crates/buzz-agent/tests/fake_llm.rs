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

// ─── Steering (_goose/unstable/session/steer) ───────────────────────────────

/// Wait for the `activeRunId` advert buzz-agent emits at prompt start and
/// return the run id, so a steer can target the live turn.
async fn recv_active_run_id(h: &mut Harness) -> String {
    let v = h
        .recv_until(|v| {
            v.get("method") == Some(&json!("session/update"))
                && v["params"]["update"]["_meta"]["goose"]["activeRunId"].is_string()
        })
        .await;
    v["params"]["update"]["_meta"]["goose"]["activeRunId"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_folds_into_active_turn_without_cancelling() {
    // A two-round turn (tool call → text). A steer sent once the run is live
    // must (a) be accepted with the matching runId, (b) NOT cancel the turn —
    // it still ends with end_turn — and (c) reach the provider as a user turn.
    let (url, captures) = spawn_capturing_fake_llm(vec![
        openai_tool_call("call_steer", "fake__noop", json!({})),
        openai_text("acknowledged the steer"),
    ])
    .await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    let p_id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{"type":"text","text":"work on the original task"}],
            }),
        )
        .await;

    // Learn the run id, then steer into it before the turn finishes.
    let run_id = recv_active_run_id(&mut h).await;
    let steer_text = "STEER-CANARY: also consider the edge case";
    let s_id = h
        .send(
            "_goose/unstable/session/steer",
            json!({
                "sessionId": sid,
                "expectedRunId": run_id,
                "prompt": [{"type":"text","text": steer_text}],
            }),
        )
        .await;

    // Steer is accepted and echoes the run id it landed in.
    let mut steer_ok = false;
    let mut end_turn = false;
    for _ in 0..40 {
        let v = h.recv().await;
        if v["id"] == json!(s_id) {
            assert_eq!(
                v["result"]["runId"],
                json!(run_id),
                "steer ran into the live turn"
            );
            assert!(
                v["result"]["messageId"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("steer_")),
                "steer reply carries a messageId"
            );
            steer_ok = true;
        } else if v["id"] == json!(p_id) {
            // The turn was NOT cancelled — it completed normally.
            assert_eq!(v["result"]["stopReason"], "end_turn");
            end_turn = true;
        }
        if steer_ok && end_turn {
            break;
        }
    }
    assert!(steer_ok, "steer request was not accepted");
    assert!(end_turn, "turn did not complete with end_turn after steer");

    // The steered text reached the provider as a user message in some round.
    let reqs = captures.lock().await;
    let saw_steer = reqs.iter().any(|req| {
        req["messages"].as_array().is_some_and(|msgs| {
            msgs.iter().any(|m| {
                m["role"] == "user"
                    && m["content"]
                        .as_str()
                        .is_some_and(|c| c.contains(steer_text))
            })
        })
    });
    assert!(
        saw_steer,
        "steered text never reached the provider; captured requests: {reqs:#?}"
    );
    drop(reqs);
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_rejected_when_no_active_run() {
    // No prompt in flight → no active run → invalid_params.
    let url = spawn_fake_llm(vec![]).await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    let s_id = h
        .send(
            "_goose/unstable/session/steer",
            json!({
                "sessionId": sid,
                "expectedRunId": "run_does_not_exist",
                "prompt": [{"type":"text","text":"hello?"}],
            }),
        )
        .await;
    let v = h.recv_until(|v| v["id"] == json!(s_id)).await;
    assert_eq!(v["error"]["code"], -32602, "expected invalid_params");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_rejected_on_run_id_mismatch() {
    // A live run, but the caller targets a stale/wrong run id → invalid_params,
    // so the client falls back to cancel+merge instead of injecting blind.
    let (url, _captures) = spawn_capturing_fake_llm(vec![
        openai_tool_call("call_x", "fake__noop", json!({})),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    let p_id = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _live_run = recv_active_run_id(&mut h).await;

    let s_id = h
        .send(
            "_goose/unstable/session/steer",
            json!({
                "sessionId": sid,
                "expectedRunId": "run_stale_mismatch",
                "prompt": [{"type":"text","text":"too late"}],
            }),
        )
        .await;

    let mut saw_reject = false;
    for _ in 0..40 {
        let v = h.recv().await;
        if v["id"] == json!(s_id) {
            assert_eq!(
                v["error"]["code"], -32602,
                "mismatched runId must be rejected"
            );
            saw_reject = true;
        } else if v["id"] == json!(p_id) {
            // Turn finishes normally regardless of the rejected steer.
            break;
        }
    }
    assert!(saw_reject, "run-id mismatch was not rejected");
    h.shutdown().await;
}

// ─── Usage notification (_goose/unstable/session/update usage_update) ───────

/// An OpenAI chat completion response with a `usage` block (prompt_tokens +
/// completion_tokens). buzz-agent maps these to `accumulatedInputTokens` /
/// `accumulatedOutputTokens` in the `_goose/unstable/session/update` notification.
fn openai_text_with_usage(content: &str, input_tokens: u64, output_tokens: u64) -> Value {
    json!({
        "id": "cc-u", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens,
        },
    })
}

/// Returns true when `v` is a `_goose/unstable/session/update` usage_update
/// notification.
fn is_usage_update(v: &Value) -> bool {
    v.get("method") == Some(&json!("_goose/unstable/session/update"))
        && v["params"]["update"]["sessionUpdate"] == "usage_update"
}

/// Collect every frame that arrives BEFORE the message matching `until_pred`,
/// then return (frames_before, matching_frame).
async fn recv_until_with_drain<F>(h: &mut Harness, mut until_pred: F) -> (Vec<Value>, Value)
where
    F: FnMut(&Value) -> bool,
{
    let mut before = Vec::new();
    loop {
        let v = h.recv().await;
        if until_pred(&v) {
            return (before, v);
        }
        before.push(v);
    }
}

/// buzz-agent must emit `_goose/unstable/session/update` with `sessionUpdate:
/// "usage_update"` **before** the `session/prompt` response on each turn, and
/// must accumulate counters across turns (turn 2 reports turn1+turn2 sums).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_notification_emitted_before_prompt_response() {
    let url = spawn_fake_llm(vec![
        openai_text_with_usage("turn one reply", 10, 5),
        openai_text_with_usage("turn two reply", 20, 8),
    ])
    .await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    // ── Turn 1 ──────────────────────────────────────────────────────────────
    let p1 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"turn 1"}]}),
        )
        .await;

    let (frames_before_t1, response_t1) = recv_until_with_drain(&mut h, |v| v["id"] == p1).await;
    assert_eq!(
        response_t1["result"]["stopReason"], "end_turn",
        "turn 1 must complete with end_turn"
    );

    // A usage_update notification must appear in the frames before the response.
    let usage_t1 = frames_before_t1
        .iter()
        .find(|v| is_usage_update(v))
        .unwrap_or_else(|| {
            panic!(
                "expected _goose/unstable/session/update usage_update before turn-1 response; frames: {frames_before_t1:#?}"
            )
        });
    assert_eq!(
        usage_t1["params"]["update"]["sessionUpdate"], "usage_update",
        "sessionUpdate field must be 'usage_update'"
    );
    assert_eq!(
        usage_t1["params"]["update"]["accumulatedInputTokens"],
        json!(10u64),
        "turn 1 accumulated input tokens"
    );
    assert_eq!(
        usage_t1["params"]["update"]["accumulatedOutputTokens"],
        json!(5u64),
        "turn 1 accumulated output tokens"
    );

    // ── Turn 2 ──────────────────────────────────────────────────────────────
    let p2 = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"turn 2"}]}),
        )
        .await;

    let (frames_before_t2, response_t2) = recv_until_with_drain(&mut h, |v| v["id"] == p2).await;
    assert_eq!(
        response_t2["result"]["stopReason"], "end_turn",
        "turn 2 must complete with end_turn"
    );

    // Notification arrives before the response, with cumulative sums (10+20, 5+8).
    let usage_t2 = frames_before_t2
        .iter()
        .find(|v| is_usage_update(v))
        .unwrap_or_else(|| {
            panic!(
                "expected _goose/unstable/session/update usage_update before turn-2 response; frames: {frames_before_t2:#?}"
            )
        });
    assert_eq!(
        usage_t2["params"]["update"]["accumulatedInputTokens"],
        json!(30u64),
        "turn 2 accumulated input tokens must be 10+20=30"
    );
    assert_eq!(
        usage_t2["params"]["update"]["accumulatedOutputTokens"],
        json!(13u64),
        "turn 2 accumulated output tokens must be 5+8=13"
    );

    h.shutdown().await;
}

/// When the provider returns a response with no `usage` block, buzz-agent must
/// NOT emit a `_goose/unstable/session/update` notification for that turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_usage_turn_emits_no_usage_notification() {
    let url = spawn_fake_llm(vec![openai_text("no usage here")]).await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    let p_id = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;

    let (frames_before, response) = recv_until_with_drain(&mut h, |v| v["id"] == p_id).await;
    assert_eq!(
        response["result"]["stopReason"], "end_turn",
        "turn must complete with end_turn"
    );

    // No usage notification must appear in the frames before the response.
    let found = frames_before.iter().any(is_usage_update);
    assert!(
        !found,
        "expected NO usage_update notification when provider reports no usage; frames: {frames_before:#?}"
    );

    h.shutdown().await;
}

/// When a turn is cancelled AFTER the provider has already returned a response
/// (so token counts are observed), buzz-agent must still emit the usage
/// notification before the cancelled `session/prompt` response.
///
/// Setup: round 1 is a tool call WITH usage (tokens are captured). After the
/// tool_call_update notification (proving round 1 is fully processed), we gate
/// the round-2 LLM response behind a `oneshot` barrier that only releases after
/// cancel is sent. This guarantees the turn exits with `stopReason: "cancelled"`
/// deterministically, even on a slow CI worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_turn_with_usage_emits_notification_before_response() {
    use tokio::sync::oneshot;

    // Gate: the second LLM request (round 2) is held until we explicitly release it.
    let (gate_tx, gate_rx) = oneshot::channel::<()>();
    let gate_rx = Arc::new(tokio::sync::Mutex::new(Some(gate_rx)));

    // Round 1: tool call with usage — sets turn_input/output_tokens.
    // Round 2: gated — blocked until cancel fires, then released so the
    // in-flight TCP request can resolve. The queue is empty for round 2, so the
    // agent receives the fallback "no canned response" body which it treats as
    // an LLM error; the cancel check at the round boundary fires first because
    // the gate is only released after cancel is enqueued.
    let responses = vec![openai_tool_call_with_usage(
        "call_cancel_test",
        "fake__noop",
        json!({}),
        15,
        6,
    )];
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let gate_rx_clone = gate_rx.clone();
    tokio::spawn(async move {
        let mut request_num = 0usize;
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            let gate = gate_rx_clone.clone();
            request_num += 1;
            let req_num = request_num;
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
                // For request 2+ (round 2), wait for the gate to open before
                // responding. This ensures cancel is sent before round 2 resolves,
                // making stopReason: cancelled deterministic.
                if req_num >= 2 {
                    let rx = gate.lock().await.take();
                    if let Some(rx) = rx {
                        let _ = rx.await;
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

    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;

    let p_id = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"start work"}]}),
        )
        .await;

    // Wait for the activeRunId advert (agent is live).
    let _run_id = recv_active_run_id(&mut h).await;
    // Wait for tool_call_update — proves round 1 LLM response is fully processed
    // and tokens are captured before we send cancel.
    h.recv_until(|v| {
        v.get("method") == Some(&json!("session/update"))
            && v["params"]["update"]["sessionUpdate"] == "tool_call_update"
    })
    .await;

    // Now send cancel and release the round-2 gate. Cancel is enqueued before
    // round 2 can respond, so the turn exits with stopReason: cancelled.
    let c_id = h.send("session/cancel", json!({"sessionId": sid})).await;
    let _ = gate_tx.send(()); // unblock round 2

    let mut saw_usage_before_prompt_response = false;
    let mut saw_usage = false;
    let mut saw_cancel_ok = false;
    let mut saw_prompt_response = false;
    for _ in 0..40 {
        let v = h.recv().await;
        if v["id"] == json!(c_id) {
            saw_cancel_ok = true;
        } else if is_usage_update(&v) {
            saw_usage = true;
            if !saw_prompt_response {
                saw_usage_before_prompt_response = true;
            }
        } else if v["id"] == json!(p_id) {
            saw_prompt_response = true;
            // The gate guarantees stopReason: cancelled — not a race-driven error.
            assert_eq!(
                v["result"]["stopReason"], "cancelled",
                "turn must end with stopReason: cancelled"
            );
        }
        if saw_usage && saw_prompt_response && saw_cancel_ok {
            break;
        }
    }
    assert!(saw_cancel_ok, "session/cancel was not acknowledged");
    assert!(
        saw_usage,
        "expected usage_update notification for cancelled turn with observed tokens"
    );
    assert!(
        saw_usage_before_prompt_response,
        "usage_update must arrive before the session/prompt response"
    );

    h.shutdown().await;
}

/// A tool-call OpenAI response with a `usage` block. Used to capture tokens in
/// round 1 before a cancel fires at the round boundary.
fn openai_tool_call_with_usage(
    id: &str,
    name: &str,
    args: Value,
    input_tokens: u64,
    output_tokens: u64,
) -> Value {
    json!({
        "id": "cc-u2", "object": "chat.completion", "model": "fake-model",
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
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens,
        },
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_rejected_on_empty_prompt() {
    let (url, _captures) = spawn_capturing_fake_llm(vec![
        openai_tool_call("call_x", "fake__noop", json!({})),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&url).await;
    let sid = init_session(&mut h).await;
    let p_id = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let run_id = recv_active_run_id(&mut h).await;
    let s_id = h
        .send(
            "_goose/unstable/session/steer",
            json!({"sessionId": sid, "expectedRunId": run_id, "prompt": []}),
        )
        .await;
    let mut saw_reject = false;
    for _ in 0..40 {
        let v = h.recv().await;
        if v["id"] == json!(s_id) {
            assert_eq!(v["error"]["code"], -32602, "empty prompt must be rejected");
            saw_reject = true;
        } else if v["id"] == json!(p_id) {
            break;
        }
    }
    assert!(saw_reject, "empty steer prompt was not rejected");
    h.shutdown().await;
}
