//! Idle sessions are evicted after `BUZZ_AGENT_SESSION_IDLE_TIMEOUT_SECS`;
//! prompting an evicted session is rejected so the harness recreates it.

mod common;

use std::time::Duration;

use serde_json::{json, Value};

use common::{openai_text, spawn_capturing_llm, Harness};

async fn init_session(h: &mut Harness) -> String {
    h.send(
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .await;
    let _ = h.recv().await;
    h.send("session/new", json!({"cwd": "/", "mcpServers": []}))
        .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    r["result"]["sessionId"]
        .as_str()
        .expect("sessionId")
        .to_owned()
}

async fn prompt(h: &mut Harness, session_id: &str) -> Value {
    let id = h
        .send(
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": "hello"}],
            }),
        )
        .await;
    h.recv_until(|v| v.get("id") == Some(&json!(id))).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_session_is_evicted_and_later_prompts_are_rejected() {
    let llm = spawn_capturing_llm(vec![openai_text("hi")]).await;
    let mut h =
        Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_SESSION_IDLE_TIMEOUT_SECS", "1")]).await;
    let session_id = init_session(&mut h).await;

    // One reap interval (1s) past the 1s timeout.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    let r = prompt(&mut h, &session_id).await;
    let message = r["error"]["message"].as_str().unwrap_or_default();
    assert_eq!(r["error"]["code"], json!(-32602), "{r}");
    assert!(message.contains("unknown session"), "{r}");
    assert!(llm.captured.lock().await.is_empty(), "no turn should run");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_session_survives_and_zero_disables_eviction() {
    let llm = spawn_capturing_llm(vec![openai_text("hi"), openai_text("hi again")]).await;
    let mut h =
        Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_SESSION_IDLE_TIMEOUT_SECS", "0")]).await;
    let session_id = init_session(&mut h).await;

    tokio::time::sleep(Duration::from_millis(2500)).await;

    let r = prompt(&mut h, &session_id).await;
    assert_eq!(r["result"]["stopReason"], json!("end_turn"), "{r}");
    h.shutdown().await;
}
