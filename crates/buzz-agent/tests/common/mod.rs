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
use tokio::sync::{oneshot, Mutex, Notify};

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

pub struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr: Arc<StdMutex<String>>,
    stderr_changed: Arc<Notify>,
    next_id: i64,
}

impl Harness {
    pub async fn spawn_with_env(base_url: &str, extra: &[(&str, &str)]) -> Self {
        Self::spawn_with_stderr_gate(base_url, extra, None).await
    }

    /// Delay stderr collection until released, to exercise stdout/stderr ordering
    /// without changing the child or relying on scheduler timing.
    pub async fn spawn_with_stderr_gate(
        base_url: &str,
        extra: &[(&str, &str)],
        stderr_gate: Option<oneshot::Receiver<()>>,
    ) -> Self {
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
        let stderr_changed = Arc::new(Notify::new());
        let changed = Arc::clone(&stderr_changed);
        tokio::spawn(async move {
            if let Some(gate) = stderr_gate {
                // Dropping the sender (e.g. on assertion failure) also unblocks
                // collection, rather than leaving a detached reader waiting.
                let _ = gate.await;
            }
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
                changed.notify_waiters();
            }
        });
        Self {
            child,
            stdin,
            stdout,
            stderr: stderr_buf,
            stderr_changed,
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

    /// Snapshot only: receiving a response on stdout does not drain stderr.
    pub fn stderr_text(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Wait for a diagnostic in the independently collected stderr stream.
    /// Returns the matching snapshot so subsequent assertions see its prefix.
    pub async fn wait_for_stderr(&self, needle: &str, timeout: Duration) -> String {
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.stderr_changed.notified();
                tokio::pin!(changed);
                // Register before inspecting the buffer: a line collected between
                // the snapshot and await must not become a lost wakeup.
                changed.as_mut().enable();
                let stderr = self.stderr_text();
                if stderr.contains(needle) {
                    return stderr;
                }
                changed.await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for stderr diagnostic {needle:?}; stderr={}",
                self.stderr_text()
            )
        })
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
