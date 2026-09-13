//! Shared subprocess test harness for the buzz-agent ACP integration suites.
//!
//! Every integration test file is its own crate, so this module is included
//! with `mod common;` in each and compiles once per binary — each binary uses
//! only the subset it needs, hence the module-wide `dead_code` allow.
//!
//! It drives a real `buzz-agent` child over the ACP wire against a fake LLM
//! (`CapturingLlm`, which records each request body) and answers the
//! `session/request_permission` surface (`approve_permission`, selecting the
//! offered `allow_once` option by `kind`, never a hardcoded `optionId`).

#![allow(dead_code)]

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Translate a canned non-streaming `chat.completion` body into the SSE stream
/// goose's openai-compatible provider actually consumes.
///
/// The canned bodies are written in the plain `chat.completion` shape because
/// that is what reads clearly in a test. goose requests `stream: true` and
/// parses `chat.completion.chunk` frames, so a plain body is silently ignored:
/// the turn ends `end_turn` with no tool call and no permission ask, which
/// looks like a broken gate rather than a broken fixture. Converting here keeps
/// the fixtures readable and the wire correct.
fn to_sse(body: &Value) -> String {
    let choice = &body["choices"][0];
    let message = &choice["message"];
    let finish = choice["finish_reason"].clone();

    let mut delta = json!({ "role": "assistant" });
    if let Some(content) = message["content"].as_str() {
        delta["content"] = json!(content);
    }
    if let Some(calls) = message["tool_calls"].as_array() {
        delta["tool_calls"] = Value::Array(
            calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    json!({
                        "index": index,
                        "id": call["id"].clone(),
                        "type": "function",
                        "function": call["function"].clone(),
                    })
                })
                .collect(),
        );
    }

    let chunk = json!({
        "id": body["id"].clone(),
        "object": "chat.completion.chunk",
        "created": 1,
        "model": body["model"].clone(),
        "choices": [{ "index": 0, "delta": delta, "finish_reason": Value::Null }],
    });
    let done = json!({
        "id": body["id"].clone(),
        "object": "chat.completion.chunk",
        "created": 1,
        "model": body["model"].clone(),
        "choices": [{ "index": 0, "delta": {}, "finish_reason": finish }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7 },
    });
    format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

pub struct CapturingLlm {
    pub url: String,
    pub captured: Arc<Mutex<Vec<Value>>>,
}

pub async fn spawn_capturing_llm(responses: Vec<Value>) -> CapturingLlm {
    spawn_capturing_llm_with_status(responses.into_iter().map(|v| (200u16, v)).collect()).await
}

/// Like `spawn_capturing_llm` but each canned response carries its own HTTP
/// status, so a test can serve a real provider rejection (e.g. a context-window
/// 400) instead of only success bodies.
pub async fn spawn_capturing_llm_with_status(responses: Vec<(u16, Value)>) -> CapturingLlm {
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
                // The catalog endpoint always succeeds and consumes no canned
                // response: it is a startup lookup, not part of the scripted
                // conversation, and letting it pop the queue would desync every
                // subsequent turn.
                let request_line = String::from_utf8_lossy(&buf[..header_end]).to_string();
                if request_line.starts_with("GET") || request_line.contains("/models") {
                    let payload = json!({
                        "object": "list",
                        "data": [{ "id": "fake-model", "object": "model" }],
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        payload.len(),
                        payload,
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                    return;
                }

                if let Ok(req) = serde_json::from_slice::<Value>(&buf[header_end..]) {
                    captured.lock().await.push(req);
                }
                let (status, body) = queue
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or_else(|| (200, json!({ "error": "no canned response" })));
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    _ => "Error",
                };
                // Success bodies go out as SSE (what goose asks for); error
                // bodies stay JSON, which is what a real provider returns for a
                // non-200 and what the provider's error path parses.
                let (content_type, body_s) = if status == 200 {
                    ("text/event-stream", to_sse(&body))
                } else {
                    ("application/json", serde_json::to_string(&body).unwrap())
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(), body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    CapturingLlm { url, captured }
}

pub struct Harness {
    /// Per-harness config/data root. Held so it outlives the child: goose reads
    /// `HOME`/`XDG_*` at startup, and a dropped tempdir would pull the session
    /// store out from under a live agent.
    _home: tempfile::TempDir,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr: Arc<StdMutex<String>>,
    next_id: i64,
}

impl Harness {
    pub async fn spawn_with_env(base_url: &str, extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-agent");
        let home = tempfile::tempdir().expect("home");
        let mut cmd = tokio::process::Command::new(bin);
        // Same shape as the other integration harnesses: goose resolves the
        // provider from `BUZZ_AGENT_PROVIDER`, and every run gets its own
        // config/data root so concurrent tests cannot share a session store or
        // reach the developer's real goose config.
        cmd.env("BUZZ_AGENT_PROVIDER", "openai-compat")
            .env("BUZZ_AGENT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("GOOSE_DISABLE_KEYRING", "1")
            .env("RUST_LOG", "warn")
            .env("BUZZ_AGENT_MAX_ROUNDS", "8");
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
            _home: home,
            child,
            stdin,
            stdout,
            stderr: stderr_buf,
            next_id: 1,
        }
    }

    pub async fn spawn(base_url: &str) -> Self {
        Self::spawn_with_env(base_url, &[]).await
    }

    pub async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    pub async fn write(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    pub async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(15), self.stdout.read_line(&mut line))
            .await
            .expect("recv timeout")
            .expect("read line");
        assert!(n > 0, "agent EOF; stderr={}", self.stderr_text());
        serde_json::from_str(&line).expect("non-JSON line")
    }

    pub async fn recv_until<F: FnMut(&Value) -> bool>(&mut self, mut pred: F) -> Value {
        loop {
            let v = self.recv().await;
            if pred(&v) {
                return v;
            }
        }
    }

    /// Like `recv_until`, but auto-approves any `session/request_permission`
    /// seen while waiting. Tests that exercise tool execution, not the
    /// permission boundary (that lives in `permission_boundary.rs`), must
    /// approve a model-issued tool call so it reaches the server.
    pub async fn recv_until_approving<F: FnMut(&Value) -> bool>(&mut self, mut pred: F) -> Value {
        loop {
            let v = self.recv().await;
            if v.get("method") == Some(&json!("session/request_permission")) {
                let resp = approve_permission(&v);
                self.write(resp).await;
                continue;
            }
            if pred(&v) {
                return v;
            }
        }
    }

    pub async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
    }

    pub fn stderr_text(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

pub fn openai_text(content: &str) -> Value {
    json!({
        "id": "cc-1", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
    })
}

pub fn openai_tool_call(id: &str, name: &str, args: Value) -> Value {
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

/// Select the offered option whose `kind == "allow_once"` and return the
/// `session/request_permission` response. Mirrors buzz-acp's answering side,
/// which selects by `kind`, never by a hardcoded `optionId`. Centralizing this
/// means a future option-id rename can't silently turn allow into a denial.
pub fn approve_permission(request: &Value) -> Value {
    let option_id = request["params"]["options"]
        .as_array()
        .and_then(|opts| opts.iter().find(|o| o["kind"] == "allow_once"))
        .and_then(|o| o["optionId"].as_str())
        .expect("request must offer an allow_once option");
    json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } },
    })
}
