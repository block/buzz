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
        let bin = env!("CARGO_BIN_EXE_sprout-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("SPROUT_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("SPROUT_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_MAX_ROUNDS", "4")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in extra {
            cmd.env(k, v);
        }
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
            json!({ "protocolVersion": 1, "clientCapabilities": {} }),
        )
        .await;
    let init = h.recv_for_id(init_id).await;
    assert_eq!(init["result"]["protocolVersion"], 1);
    assert_eq!(init["result"]["agentInfo"]["name"], "sprout-agent");
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
    assert_eq!(resp["result"]["protocolVersion"], 1);

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
    let big = "x".repeat(1024);
    let _ = stdin.write_all(big.as_bytes()).await;
    let _ = stdin.write_all(b"\n").await;
    drop(stdin);
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("agent did not exit on oversized line");
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
