use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{routing::post, Json, Router};
use nostr::Keys;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[tokio::test]
async fn send_rejects_nul_stdin_before_submission_but_accepts_literal_text() {
    let submitted = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&submitted);
    let app = Router::new().route(
        "/events",
        post(move |Json(event): Json<Value>| {
            let captured = Arc::clone(&captured);
            async move {
                let event_id = event["id"].clone();
                captured.lock().unwrap().push(event);
                Json(json!({"event_id": event_id, "accepted": true}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // A disposable identity; no configured account or remote relay is used.
    let key = Keys::generate().secret_key().to_secret_hex();
    for content in [
        "\0prefix",
        "middle\0suffix",
        "trailing\0",
        "literal `0 text",
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_buzz"))
            .env_clear()
            .args([
                "--relay",
                &relay,
                "--private-key",
                &key,
                "messages",
                "send",
                "--channel",
                "123e4567-e89b-12d3-a456-426614174000",
                "--content",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(content.as_bytes()).await.unwrap();
        drop(stdin);
        let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
            .await
            .expect("send command terminates")
            .unwrap();
        if content.contains('\0') {
            assert!(!output.status.success(), "NUL input must fail");
            assert!(String::from_utf8_lossy(&output.stderr).contains("NUL byte"));
            assert!(submitted.lock().unwrap().is_empty(), "no /events request");
        } else {
            assert!(output.status.success(), "literal text must succeed");
            let events = submitted.lock().unwrap();
            assert_eq!(events.len(), 1, "the fake relay must observe valid sends");
            assert_eq!(events[0]["content"], content);
            let signed: nostr::Event = serde_json::from_value(events[0].clone()).unwrap();
            signed.verify().unwrap();
        }
    }
    server.abort();
}
