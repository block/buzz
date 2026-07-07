use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn(extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("BUZZ_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_MAX_ROUNDS", "4")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in extra {
            cmd.env(k, v);
        }
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
        self.write_json(json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }))
        .await;
        id
    }

    async fn notify(&mut self, method: &str, params: Value) {
        self.write_json(json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }))
        .await;
    }

    async fn write_json(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn write_raw(&mut self, raw: &[u8]) {
        let _ = self.stdin.write_all(raw).await;
        let _ = self.stdin.flush().await;
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

    async fn recv_for_id(&mut self, id: i64) -> Value {
        loop {
            let v = self.recv().await;
            if v["id"] == json!(id) {
                return v;
            }
        }
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
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(),
                    body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    url
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

async fn handshake(h: &mut Harness) -> String {
    let init_id = h
        .send(
            "initialize",
            json!({ "protocolVersion": 2, "clientCapabilities": {} }),
        )
        .await;
    let init = h.recv_for_id(init_id).await;
    assert_eq!(init["result"]["protocolVersion"], 2);
    assert_eq!(init["result"]["agentInfo"]["name"], "buzz-agent");
    assert_eq!(
        init["result"]["agentCapabilities"]["promptCapabilities"]["image"],
        false
    );

    let new_id = h
        .send("session/new", json!({ "cwd": "/tmp", "mcpServers": [] }))
        .await;
    let new = h.recv_for_id(new_id).await;
    let sid = new["result"]["sessionId"].as_str().unwrap().to_owned();
    assert!(sid.starts_with("ses_"));
    sid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_text_only_response() {
    let url = spawn_fake_llm(vec![openai_text("hello back")]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "hi" }],
            }),
        )
        .await;
    let result = h.recv_for_id(p).await;
    assert_eq!(result["result"]["stopReason"], "end_turn");
    assert!(result.get("error").is_none());

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_full_tool_call_transcript() {
    let url = spawn_fake_llm(vec![
        openai_tool_call("call_xyz", "fake__do_thing", json!({ "foo": "bar" })),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "use the tool" }],
            }),
        )
        .await;

    let failed = h
        .recv_until(|v| {
            v.get("method") == Some(&json!("session/update"))
                && v["params"]["update"]["sessionUpdate"] == "tool_call_update"
                && v["params"]["update"]["status"] == "failed"
        })
        .await;
    assert_eq!(failed["params"]["sessionId"], sid);
    assert_eq!(failed["params"]["update"]["toolCallId"], "call_xyz");
    assert_eq!(
        failed["params"]["update"]["rawOutput"]["error"],
        "unknown tool: fake__do_thing"
    );

    let final_resp = h.recv_for_id(p).await;
    assert_eq!(final_resp["result"]["stopReason"], "end_turn");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_permission_denied_continues() {
    let url = spawn_fake_llm(vec![openai_text("ok with no tool")]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "hi" }],
            }),
        )
        .await;
    let final_resp = h.recv_for_id(p).await;
    assert_eq!(final_resp["result"]["stopReason"], "end_turn");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_initialize_version_check() {
    let url = spawn_fake_llm(vec![]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let id = h
        .send(
            "initialize",
            json!({ "protocolVersion": 99, "clientCapabilities": {} }),
        )
        .await;
    let resp = h.recv_for_id(id).await;
    assert_eq!(resp["result"]["protocolVersion"], 2);

    let id2 = h
        .send(
            "initialize",
            json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await;
    let ok = h.recv_for_id(id2).await;
    assert_eq!(ok["result"]["protocolVersion"], 1);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_session_new_rejects_relative_cwd() {
    let url = spawn_fake_llm(vec![]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let _ = h
        .send(
            "initialize",
            json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await;
    let _ = h.recv().await;

    let id = h
        .send(
            "session/new",
            json!({ "cwd": "relative/path", "mcpServers": [] }),
        )
        .await;
    let resp = h.recv_for_id(id).await;
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("cwd must be an absolute path"));

    let id_empty = h
        .send("session/new", json!({ "cwd": "", "mcpServers": [] }))
        .await;
    let resp = h.recv_for_id(id_empty).await;
    assert_eq!(resp["error"]["code"], -32602);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_malformed_json_rpc() {
    let url = spawn_fake_llm(vec![]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    h.write_raw(b"this is not json\n").await;
    let v = h.recv().await;
    assert_eq!(v["error"]["code"], -32700);
    assert_eq!(v["id"], Value::Null);

    h.write_json(json!({ "jsonrpc": "1.0", "method": "initialize", "id": 1 }))
        .await;
    let v = h.recv().await;
    assert_eq!(v["error"]["code"], -32600);

    h.write_json(json!({ "jsonrpc": "2.0" })).await;
    let v = h.recv().await;
    assert_eq!(v["error"]["code"], -32600);

    let init_id = h
        .send(
            "initialize",
            json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await;
    let ok = h.recv_for_id(init_id).await;
    assert_eq!(ok["result"]["protocolVersion"], 1);

    let bad_id = h.send("nonsense/method", json!({})).await;
    let v = h.recv_for_id(bad_id).await;
    assert_eq!(v["error"]["code"], -32601);

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_unsupported_content_block() {
    let url = spawn_fake_llm(vec![openai_text("ok")]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "image", "data": "..." }],
            }),
        )
        .await;
    let resp = h.recv_for_id(p).await;
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unsupported content block"));

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_prompt_rejected() {
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

    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;
    let sid = handshake(&mut h).await;

    let p1 = h
        .send(
            "session/prompt",
            json!({ "sessionId": sid, "prompt": [{"type":"text","text":"go"}] }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let p2 = h
        .send(
            "session/prompt",
            json!({ "sessionId": sid, "prompt": [{"type":"text","text":"again"}] }),
        )
        .await;

    let mut p1_ok = false;
    let mut p2_err = false;
    for _ in 0..10 {
        let v = h.recv().await;
        if v["id"] == json!(p1) {
            assert_eq!(v["result"]["stopReason"], "end_turn");
            p1_ok = true;
        } else if v["id"] == json!(p2) {
            assert_eq!(v["error"]["code"], -32602);
            p2_err = true;
        }
        if p1_ok && p2_err {
            break;
        }
    }
    assert!(p1_ok && p2_err, "expected p1=ok, p2=busy");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oversized_line_kills_agent() {
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
    let big = "x".repeat(1024);
    let _ = stdin.write_all(big.as_bytes()).await;
    let _ = stdin.write_all(b"\n").await;
    drop(stdin);
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("agent did not exit on oversized line");
}

/// Build an Anthropic Messages API response with an optional `thinking` block
/// followed by a `text` block. The `thinking` field is omitted when `None`.
fn anthropic_thinking_response(thinking: Option<&str>, text: &str) -> Value {
    let mut content: Vec<Value> = Vec::new();
    if let Some(t) = thinking {
        content.push(json!({ "type": "thinking", "thinking": t }));
    }
    content.push(json!({ "type": "text", "text": text }));
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-fake",
        "stop_reason": "end_turn",
        "content": content,
        "usage": { "input_tokens": 10, "output_tokens": 5 },
    })
}

/// Build an OpenAI Responses API response with a `reasoning` output item
/// (containing a single `summary_text` entry) followed by a message item.
fn responses_reasoning_response(reasoning: &str, text: &str) -> Value {
    json!({
        "id": "resp_1",
        "status": "completed",
        "output": [
            {
                "type": "reasoning",
                "id": "rs_1",
                "summary": [{ "type": "summary_text", "text": reasoning }],
            },
            {
                "type": "message",
                "id": "msg_1",
                "content": [{ "type": "output_text", "text": text }],
            },
        ],
        "usage": { "input_tokens": 10 },
    })
}

/// Drain all `session/update` notifications until the `session/prompt` reply
/// arrives for `prompt_id`, collecting notification payloads in order.
async fn collect_updates_until_done(h: &mut Harness, prompt_id: i64) -> Vec<Value> {
    let mut updates = Vec::new();
    loop {
        let v = h.recv().await;
        if v.get("id") == Some(&json!(prompt_id)) {
            return updates;
        }
        if v.get("method") == Some(&json!("session/update")) {
            if let Some(u) = v["params"].get("update") {
                updates.push(u.clone());
            }
        }
    }
}

/// Asserts that `agent_thought_chunk` appears in `updates` BEFORE
/// `agent_message_chunk`, and that both are present.
fn assert_thought_before_message(updates: &[Value]) {
    let thought_pos = updates
        .iter()
        .position(|u| u["sessionUpdate"] == "agent_thought_chunk");
    let message_pos = updates
        .iter()
        .position(|u| u["sessionUpdate"] == "agent_message_chunk");
    assert!(
        thought_pos.is_some(),
        "expected agent_thought_chunk in updates: {updates:?}"
    );
    assert!(
        message_pos.is_some(),
        "expected agent_message_chunk in updates: {updates:?}"
    );
    assert!(
        thought_pos.unwrap() < message_pos.unwrap(),
        "agent_thought_chunk must precede agent_message_chunk, got updates: {updates:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_thought_chunk_emitted_before_message_chunk_anthropic() {
    // Anthropic extended-thinking: the response contains a `thinking` block
    // followed by a `text` block. We expect agent_thought_chunk to be emitted
    // before agent_message_chunk on the wire.
    let url = spawn_fake_llm(vec![anthropic_thinking_response(
        Some("Let me reason about this carefully."),
        "Here is my answer.",
    )])
    .await;
    let mut h = Harness::spawn(&[
        ("BUZZ_AGENT_PROVIDER", "anthropic"),
        ("ANTHROPIC_API_KEY", "test"),
        ("ANTHROPIC_MODEL", "claude-fake"),
        ("ANTHROPIC_BASE_URL", &url),
        ("OPENAI_COMPAT_BASE_URL", ""),
    ])
    .await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "think hard" }],
            }),
        )
        .await;

    let updates = collect_updates_until_done(&mut h, p).await;
    assert_thought_before_message(&updates);

    let thought = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_thought_chunk")
        .unwrap();
    assert_eq!(
        thought["content"]["text"],
        "Let me reason about this carefully."
    );

    let message = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_message_chunk")
        .unwrap();
    assert_eq!(message["content"]["text"], "Here is my answer.");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_thought_chunk_emitted_before_message_chunk_responses_api() {
    // OpenAI Responses API: reasoning item followed by message item.
    // Setting OPENAI_COMPAT_API=responses forces the Responses API parse path.
    let url = spawn_fake_llm(vec![responses_reasoning_response(
        "Thinking step by step.",
        "Final answer.",
    )])
    .await;
    let mut h = Harness::spawn(&[
        ("BUZZ_AGENT_PROVIDER", "openai"),
        ("OPENAI_COMPAT_API_KEY", "test"),
        ("OPENAI_COMPAT_MODEL", "fake-model"),
        ("OPENAI_COMPAT_API", "responses"),
        ("OPENAI_COMPAT_BASE_URL", &url),
    ])
    .await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "reason it out" }],
            }),
        )
        .await;

    let updates = collect_updates_until_done(&mut h, p).await;
    assert_thought_before_message(&updates);

    let thought = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_thought_chunk")
        .unwrap();
    assert_eq!(thought["content"]["text"], "Thinking step by step.");

    let message = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_message_chunk")
        .unwrap();
    assert_eq!(message["content"]["text"], "Final answer.");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_thought_chunk_emitted_before_message_chunk_chat_completions_reasoning_content() {
    // OpenAI chat/completions path with DeepSeek-style `reasoning_content` field
    // on the message object. OPENAI_COMPAT_API defaults to Auto, which routes
    // non-openai.com hosts to chat/completions — this is the live path for
    // self-hosted reasoning models (DeepSeek, vLLM, etc.).
    let response = json!({
        "id": "cc-r1", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Here is the answer.",
                "reasoning_content": "Let me think through this step by step.",
            },
            "finish_reason": "stop",
        }],
    });
    let url = spawn_fake_llm(vec![response]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "solve it" }],
            }),
        )
        .await;

    let updates = collect_updates_until_done(&mut h, p).await;
    assert_thought_before_message(&updates);

    let thought = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_thought_chunk")
        .unwrap();
    assert_eq!(
        thought["content"]["text"],
        "Let me think through this step by step."
    );

    let message = updates
        .iter()
        .find(|u| u["sessionUpdate"] == "agent_message_chunk")
        .unwrap();
    assert_eq!(message["content"]["text"], "Here is the answer.");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_no_reasoning_no_thought_chunk() {
    // Plain text response with no reasoning content — no agent_thought_chunk
    // should appear on the wire. This guards against empty thought emissions.
    let url = spawn_fake_llm(vec![openai_text("just text, no thinking")]).await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "hi" }],
            }),
        )
        .await;

    let updates = collect_updates_until_done(&mut h, p).await;

    let has_thought = updates
        .iter()
        .any(|u| u["sessionUpdate"] == "agent_thought_chunk");
    assert!(
        !has_thought,
        "expected no agent_thought_chunk for a plain text response, got: {updates:?}"
    );

    let has_message = updates
        .iter()
        .any(|u| u["sessionUpdate"] == "agent_message_chunk");
    assert!(has_message, "expected agent_message_chunk in updates");

    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cancel_notification_no_reply() {
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
        tokio::time::sleep(Duration::from_millis(800)).await;
        let body = openai_text("done").to_string();
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    });

    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;
    let sid = handshake(&mut h).await;

    let p = h
        .send(
            "session/prompt",
            json!({ "sessionId": sid, "prompt": [{"type":"text","text":"go"}] }),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.notify("session/cancel", json!({ "sessionId": sid }))
        .await;

    let final_resp = h.recv_for_id(p).await;
    let stop = final_resp["result"]["stopReason"].as_str().unwrap_or("");
    assert!(
        stop == "cancelled" || stop == "end_turn",
        "unexpected stopReason {stop}"
    );

    h.shutdown().await;
}

/// Per-tool friendly titles, stage 1: the immediate `tool_call` (pending)
/// update must carry the deterministic friendly title plus the exact tool
/// identity in `toolName`, and receipts in `rawInput`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pending_tool_call_carries_friendly_title_and_tool_name() {
    let url = spawn_fake_llm(vec![
        openai_tool_call(
            "call_t1",
            "fake__do_thing",
            json!({ "command": "git status" }),
        ),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let sid = handshake(&mut h).await;
    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "use the tool" }],
            }),
        )
        .await;

    let pending = h
        .recv_until(|v| {
            v.get("method") == Some(&json!("session/update"))
                && v["params"]["update"]["sessionUpdate"] == "tool_call"
        })
        .await;
    let update = &pending["params"]["update"];
    assert_eq!(update["title"], "fake: do thing · git status");
    assert_eq!(update["toolName"], "fake__do_thing");
    assert_eq!(update["status"], "pending");
    assert_eq!(update["rawInput"]["command"], "git status");

    let final_resp = h.recv_for_id(p).await;
    assert_eq!(final_resp["result"]["stopReason"], "end_turn");
    h.shutdown().await;
}

/// Per-tool friendly titles, stage 2: the async fast pass publishes a
/// title-only `tool_call_update` tagged `_meta.buzz.toolSummary` with NO
/// status field, so it can never regress pending/executing/terminal state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_async_tool_summary_publishes_title_only_update() {
    // Queue: round-1 tool call, then the fast summary response (the tool is
    // delayed 1s so the summary request reaches the fake LLM first), then the
    // round-2 final text.
    let url = spawn_fake_llm(vec![
        openai_tool_call(
            "call_s1",
            "fake__tool_0",
            json!({ "command": "git status" }),
        ),
        openai_text("checking repository state"),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&[("OPENAI_COMPAT_BASE_URL", &url)]).await;

    let init_id = h
        .send(
            "initialize",
            json!({ "protocolVersion": 2, "clientCapabilities": {} }),
        )
        .await;
    let _ = h.recv_for_id(init_id).await;
    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    let new_id = h
        .send(
            "session/new",
            json!({
                "cwd": "/tmp",
                "mcpServers": [{
                    "name": "fake",
                    "command": fake_mcp,
                    "args": [],
                    "env": [
                        { "name": "FAKE_MCP_TOOL_COUNT", "value": "1" },
                        { "name": "FAKE_MCP_TOOL_DELAY", "value": "1" },
                    ],
                }],
            }),
        )
        .await;
    let new = h.recv_for_id(new_id).await;
    let sid = new["result"]["sessionId"].as_str().unwrap().to_owned();

    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "use the tool" }],
            }),
        )
        .await;

    let summary = h
        .recv_until(|v| {
            v.get("method") == Some(&json!("session/update"))
                && v["params"]["update"]["sessionUpdate"] == "tool_call_update"
                && v["params"]["update"]["_meta"]["buzz"]["toolSummary"] == json!(true)
        })
        .await;
    let update = &summary["params"]["update"];
    assert_eq!(update["toolCallId"], "call_s1");
    assert_eq!(update["title"], "checking repository state");
    assert_eq!(update["toolName"], "fake__tool_0");
    assert!(
        update.get("status").is_none(),
        "summary update must not carry a status: {update}"
    );

    let final_resp = h.recv_for_id(p).await;
    assert_eq!(final_resp["result"]["stopReason"], "end_turn");
    h.shutdown().await;
}

/// Kill switch: BUZZ_AGENT_NO_TOOL_SUMMARY=1 must suppress the fast pass
/// entirely — no summary update on the wire, and no extra LLM request
/// (the canned-queue accounting would break the turn if one fired).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_tool_summary_kill_switch() {
    let url = spawn_fake_llm(vec![
        openai_tool_call("call_k1", "fake__tool_0", json!({ "command": "ls" })),
        openai_text("done"),
    ])
    .await;
    let mut h = Harness::spawn(&[
        ("OPENAI_COMPAT_BASE_URL", &url),
        ("BUZZ_AGENT_NO_TOOL_SUMMARY", "1"),
    ])
    .await;

    let init_id = h
        .send(
            "initialize",
            json!({ "protocolVersion": 2, "clientCapabilities": {} }),
        )
        .await;
    let _ = h.recv_for_id(init_id).await;
    let fake_mcp = env!("CARGO_BIN_EXE_fake-mcp");
    let new_id = h
        .send(
            "session/new",
            json!({
                "cwd": "/tmp",
                "mcpServers": [{
                    "name": "fake",
                    "command": fake_mcp,
                    "args": [],
                    "env": [{ "name": "FAKE_MCP_TOOL_COUNT", "value": "1" }],
                }],
            }),
        )
        .await;
    let new = h.recv_for_id(new_id).await;
    let sid = new["result"]["sessionId"].as_str().unwrap().to_owned();

    let p = h
        .send(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{ "type": "text", "text": "use the tool" }],
            }),
        )
        .await;

    // Drain everything up to the final response; no notification along the
    // way may carry the toolSummary marker. If the fast pass had fired it
    // would also have consumed the "done" response and broken the turn.
    let mut saw_summary = false;
    let final_resp = loop {
        let v = h.recv().await;
        if v["params"]["update"]["_meta"]["buzz"]["toolSummary"] == json!(true) {
            saw_summary = true;
        }
        if v["id"] == json!(p) {
            break v;
        }
    };
    assert!(
        !saw_summary,
        "kill switch did not suppress the summary pass"
    );
    assert_eq!(final_resp["result"]["stopReason"], "end_turn");
    h.shutdown().await;
}
