use std::process::{Command, Output};

use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

const PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000001";

async fn relay_with_response(status: StatusCode, body: Value) -> String {
    let app = Router::new().route(
        "/query",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let address = listener.local_addr().expect("listener address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });
    format!("http://{address}")
}

fn run_buzz(relay: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(["--relay", relay, "--private-key", PRIVATE_KEY])
        .args(args)
        .output()
        .expect("run buzz")
}

fn metadata_events() -> Value {
    json!([
        {
            "id": "a".repeat(64),
            "pubkey": "b".repeat(64),
            "created_at": 20,
            "kind": 39000,
            "content": "",
            "tags": [
                ["d", "11111111-1111-1111-1111-111111111111"],
                ["name", "general"],
                ["about", "General discussion"],
                ["public"]
            ],
            "sig": "c".repeat(128)
        },
        {
            "id": "d".repeat(64),
            "pubkey": "e".repeat(64),
            "created_at": 10,
            "kind": 39000,
            "content": "",
            "tags": [
                ["d", "22222222-2222-2222-2222-222222222222"],
                ["name", "private"],
                ["about", "Private discussion"],
                ["private"]
            ],
            "sig": "f".repeat(128)
        }
    ])
}

#[tokio::test(flavor = "multi_thread")]
async fn full_output_and_visibility_filter_are_stable() {
    let relay = relay_with_response(StatusCode::OK, metadata_events()).await;
    let output = run_buzz(
        &relay,
        &["channels", "list", "--visibility", "open", "--limit", "5"],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON stdout");
    assert_eq!(
        value,
        json!([{
            "channel_id": "11111111-1111-1111-1111-111111111111",
            "name": "general",
            "description": "General discussion",
            "created_at": 20
        }])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn compact_output_projection_is_stable() {
    let relay = relay_with_response(StatusCode::OK, metadata_events()).await;
    let output = run_buzz(
        &relay,
        &["--format", "compact", "channels", "list", "--limit", "5"],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON stdout");
    assert_eq!(
        value,
        json!([
            {
                "channel_id": "11111111-1111-1111-1111-111111111111",
                "name": "general"
            },
            {
                "channel_id": "22222222-2222-2222-2222-222222222222",
                "name": "private"
            }
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_forbidden_keeps_auth_error_and_exit_code() {
    let relay = relay_with_response(StatusCode::FORBIDDEN, json!({"error": "forbidden"})).await;
    let output = run_buzz(&relay, &["channels", "list"]);

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let value: Value = serde_json::from_slice(&output.stderr).expect("JSON stderr");
    assert_eq!(value["error"], "auth_error");
    assert_eq!(value["retryable"], false);
    assert!(value["message"]
        .as_str()
        .is_some_and(|message| message.contains("relay error 403: forbidden")));
}
