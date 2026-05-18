//! Integration test for OpenAI auto-upgrade chat→responses.
//!
//! Starts a tiny HTTP server that:
//!   1. accepts a POST to /chat/completions, replies 400 with a body that
//!      mentions `/v1/responses` (mirrors the Databricks GPT-5.5 signal);
//!   2. accepts a POST to /responses, replies 200 with a Responses-shaped
//!      JSON envelope.
//!
//! Spawns `sprout-agent` with `provider=openai` + `OPENAI_COMPAT_API=auto`
//! pointed at the fake server, drives one prompt through the ACP wire
//! protocol, and verifies the prompt completes with `stopReason=end_turn`
//! — which can only happen if the second (Responses) request succeeded.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Spawns a single-shot fake provider. Returns the base URL (e.g.
/// `http://127.0.0.1:54321`). The server stays up for the lifetime of
/// the process — we don't need to clean it up explicitly.
fn spawn_fake_provider() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(false).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let chat_hits = Arc::new(AtomicUsize::new(0));
    let responses_hits = Arc::new(AtomicUsize::new(0));
    let chat = chat_hits.clone();
    let resp = responses_hits.clone();

    std::thread::spawn(move || {
        loop {
            let (mut sock, _) = match listener.accept() {
                Ok(p) => p,
                Err(_) => return,
            };
            let chat = chat.clone();
            let resp = resp.clone();
            std::thread::spawn(move || {
                sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
                // Read request head + body. Naive: read until we have the
                // request line + headers, then read Content-Length bytes.
                let mut buf = Vec::with_capacity(4096);
                let mut tmp = [0u8; 4096];
                loop {
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.len() > 256 * 1024 {
                        return;
                    }
                }
                let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                // Drain the body to satisfy keep-alive; we don't actually
                // need it.
                let cl = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .or_else(|| l.strip_prefix("Content-Length:"))
                    })
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < head_end + cl {
                    match sock.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }

                let (status, body) = if head.contains("POST /chat/completions") {
                    chat.fetch_add(1, Ordering::SeqCst);
                    let body = json!({
                        "error": {
                            "code": "BAD_REQUEST",
                            "message": "Function tools with reasoning_effort are not supported for gpt-5.5 in /v1/chat/completions. Please use /v1/responses instead."
                        }
                    })
                    .to_string();
                    (400u16, body)
                } else if head.contains("POST /responses") {
                    resp.fetch_add(1, Ordering::SeqCst);
                    let body = json!({
                        "status": "completed",
                        "output": [{
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": "ok from responses"}]
                        }]
                    })
                    .to_string();
                    (200u16, body)
                } else {
                    (404u16, "{}".to_string())
                };
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    _ => "Not Found",
                };
                let resp_text = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = sock.write_all(resp_text.as_bytes());
                let _ = sock.shutdown(std::net::Shutdown::Write);
            });
        }
    });

    (url, chat_hits, responses_hits)
}

#[tokio::test]
async fn openai_auto_upgrades_chat_to_responses_on_databricks_signal() {
    let (base_url, chat_hits, resp_hits) = spawn_fake_provider();

    let bin = env!("CARGO_BIN_EXE_sprout-agent");
    let mut cmd = Command::new(bin);
    cmd.env("SPROUT_AGENT_PROVIDER", "openai")
        .env("OPENAI_COMPAT_API_KEY", "test")
        .env("OPENAI_COMPAT_MODEL", "gpt-5.5")
        .env("OPENAI_COMPAT_BASE_URL", &base_url)
        // No OPENAI_COMPAT_API — must default to "auto" so the upgrade
        // path is enabled.
        .env_remove("OPENAI_COMPAT_API")
        .env("SPROUT_AGENT_LLM_TIMEOUT_SECS", "5")
        .env("SPROUT_AGENT_MAX_ROUNDS", "4")
        .env("SPROUT_AGENT_MCP_INIT_TIMEOUT_SECS", "2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let mut child = cmd.spawn().expect("spawn sprout-agent");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    async fn send(stdin: &mut tokio::process::ChildStdin, v: serde_json::Value) {
        let line = format!("{v}\n");
        stdin.write_all(line.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();
    }
    async fn recv(stdout: &mut BufReader<tokio::process::ChildStdout>) -> serde_json::Value {
        let mut line = String::new();
        timeout(Duration::from_secs(8), stdout.read_line(&mut line))
            .await
            .expect("recv timed out")
            .expect("recv io");
        serde_json::from_str(&line).expect("recv json")
    }

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {},
                       "clientInfo": {"name": "auto-upgrade-test"}}
        }),
    )
    .await;
    let init = recv(&mut stdout).await;
    assert!(init.get("result").is_some(), "initialize: {init}");

    let cwd = std::env::current_dir().unwrap();
    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "session/new",
            "params": {"cwd": cwd.to_string_lossy(), "mcpServers": []}
        }),
    )
    .await;
    let sess = recv(&mut stdout).await;
    let sid = sess["result"]["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("session/new failed: {sess}"))
        .to_string();

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "session/prompt",
            "params": {"sessionId": sid,
                       "prompt": [{"type": "text", "text": "hi"}]}
        }),
    )
    .await;

    // Drain notifications until we see the response for id=3.
    let mut stop_reason: Option<String> = None;
    for _ in 0..40 {
        let msg = recv(&mut stdout).await;
        if msg.get("id") == Some(&json!(3)) {
            if let Some(r) = msg.get("result") {
                stop_reason = r
                    .get("stopReason")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            break;
        }
    }
    assert_eq!(stop_reason.as_deref(), Some("end_turn"));
    assert_eq!(
        chat_hits.load(Ordering::SeqCst),
        1,
        "must have tried chat first"
    );
    assert!(
        resp_hits.load(Ordering::SeqCst) >= 1,
        "must have upgraded to responses"
    );

    drop(stdin);
    let _ = child.wait().await;
}
