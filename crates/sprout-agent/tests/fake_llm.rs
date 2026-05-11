//! Integration test: fake LLM HTTP server + sprout-agent subprocess.
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

// ─── Fake LLM server ────────────────────────────────────────────────────────

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

// ─── ACP harness ────────────────────────────────────────────────────────────

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn(base_url: &str) -> Self {
        let bin = env!("CARGO_BIN_EXE_sprout-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("SPROUT_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("SPROUT_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_MAX_ROUNDS", "4")
            .stdin(Stdio::piped())
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

// ─── Canned LLM responses (OpenAI-compat shape) ─────────────────────────────

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
        json!({"protocolVersion":1,"clientCapabilities":{}}),
    )
    .await;
    let r = h.recv().await;
    assert_eq!(r["result"]["protocolVersion"], 1);
    assert_eq!(r["result"]["agentInfo"]["name"], "sprout-agent");
    h.send("session/new", json!({"cwd":"/tmp","mcpServers":[]}))
        .await;
    let r = h.recv().await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_owned();
    assert!(sid.starts_with("ses_"));
    sid
}

// ─── Tests ──────────────────────────────────────────────────────────────────

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
    let bin = env!("CARGO_BIN_EXE_sprout-agent");
    let mut cmd = tokio::process::Command::new(bin);
    cmd.env("SPROUT_AGENT_PROVIDER", "openai")
        .env("OPENAI_COMPAT_API_KEY", "test")
        .env("OPENAI_COMPAT_MODEL", "fake-model")
        .env("OPENAI_COMPAT_BASE_URL", &url)
        .env("SPROUT_AGENT_MAX_LINE_BYTES", "256")
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
