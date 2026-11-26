//! Runs the real `buzz` binary against a local fake relay to check the
//! process exit code of `raw publish` when the relay refuses the event.
//!
//! The child gets none of the caller's `BUZZ_*` variables, so an agent's
//! inherited auth tag or federated identity cannot change the outcome.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

/// A relay bridge whose `/events` answers HTTP 200 with `accepted:false`,
/// as the relay does for a duplicate reaction. Returns its URL and a count
/// of submitted events.
async fn refusing_relay() -> (String, Arc<AtomicUsize>) {
    let submitted = Arc::new(AtomicUsize::new(0));
    let counter = submitted.clone();
    let app = Router::new().route(
        "/events",
        post(move |body: String| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let event: serde_json::Value = serde_json::from_str(&body).unwrap();
                let id = event["id"].as_str().unwrap().to_string();
                (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    format!(
                        r#"{{"event_id":"{id}","accepted":false,"message":"duplicate: reaction already exists"}}"#
                    ),
                )
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), submitted)
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_publish_exits_2_when_the_relay_refuses() {
    let (url, submitted) = refusing_relay().await;
    let key = nostr::Keys::generate().secret_key().to_secret_hex();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_buzz"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("BUZZ_") {
            cmd.env_remove(name);
        }
    }
    cmd.args([
        "--relay",
        url.as_str(),
        "--private-key",
        key.as_str(),
        "raw",
        "publish",
        "--kind",
        "7",
        "--content",
        "+",
    ]);
    let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a refused publish is a relay error; stderr: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "a refusal prints nothing to stdout"
    );
    assert!(stderr.contains("relay_error"), "stderr: {stderr}");
    assert_eq!(submitted.load(Ordering::SeqCst), 1);
}
