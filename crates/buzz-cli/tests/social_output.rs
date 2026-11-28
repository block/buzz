use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::Command;

#[tokio::test]
async fn social_reads_honor_output_format() {
    let event = json!({
        "id": "a".repeat(64),
        "pubkey": "b".repeat(64),
        "kind": 1,
        "content": "hello",
        "created_at": 123,
        "tags": [["auth", "c".repeat(64), "kind=1", "d".repeat(128)]],
        "sig": "e".repeat(128),
    });
    let response = json!([event]);
    let app = Router::new().route(
        "/query",
        post(move || {
            let response = response.clone();
            async move { Json(response) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let key = "1".repeat(64);
    let id = "a".repeat(64);

    for args in [
        vec!["event", "--event", id.as_str()],
        vec!["notes", "--pubkey", id.as_str()],
        vec!["contacts", "--pubkey", id.as_str()],
        vec!["list", "--pubkey", id.as_str(), "--kind", "10000"],
    ] {
        for format in [Some("json"), Some("compact"), Some("agent"), None] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
            command
                .env_clear()
                .env("BUZZ_OUTPUT_FORMAT", "agent")
                .args(["--relay", &relay, "--private-key", &key])
                .kill_on_drop(true);
            if let Some(format) = format {
                command.args(["--format", format]);
            }
            let output = tokio::time::timeout(
                Duration::from_secs(10),
                command.arg("social").args(&args).output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                output.status.success(),
                "{args:?} {format:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
            let expected = match format {
                Some("json") => event.clone(),
                Some("compact") => json!({
                    "id": event["id"],
                    "content": "hello",
                    "created_at": 123,
                }),
                _ => {
                    let mut projected = event.clone();
                    projected.as_object_mut().unwrap().remove("sig");
                    projected["tags"] = json!([["auth", "c".repeat(64)]]);
                    projected
                }
            };
            assert_eq!(actual, json!([expected]), "{args:?} {format:?}");
        }
    }
    server.abort();
}
