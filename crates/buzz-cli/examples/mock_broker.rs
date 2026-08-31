//! A throwaway broker host for local keyless-CLI development.
//!
//! It speaks just enough of the agent-broker contract (block/buzz#6790) to let
//! `buzz --agent-mode broker …` complete a round trip before the real beekeeper
//! host exists: it accepts `POST /v1/action`, echoes the request's `requestId`
//! and `action` back (so correlation holds), and returns a canned success
//! outcome per action. It signs nothing and touches no relay — it exists only to
//! prove the client wiring end to end.
//!
//! Run it in one terminal:
//!   cargo run -p buzz-cli --example mock_broker
//! then drive the CLI from another (see the crate's keyless docs).

use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

const ADDR: &str = "127.0.0.1:8787";
const FAKE_EVENT_ID: &str = "cacf5f811cc8ef3f4af3f92cc222f92a86cdf6a26728a144c8e63b74ab6db359";
const FAKE_PUBKEY: &str = "a02c4e0850e5e612b4ddf95dbe2f5c56467cf27c6552203bc833ff438fb31971";

#[tokio::main]
async fn main() {
    let app = Router::new().route("/v1/action", post(action));
    let listener = TcpListener::bind(ADDR).await.expect("bind");
    eprintln!("mock broker listening on http://{ADDR}  (Ctrl-C to stop)");
    axum::serve(listener, app).await.expect("serve");
}

/// Answer one action with a canned success envelope for the wake→reply slice.
async fn action(headers: axum::http::HeaderMap, body: Bytes) -> Response {
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    let request_id = request
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let action = request.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let credential = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<none>");
    eprintln!(
        "→ {action}  requestId={request_id}  auth={credential}\n  args={}",
        request.get("args").unwrap_or(&serde_json::Value::Null)
    );

    // A host verdict always rides in the body at HTTP 200; only the shape varies.
    let body = match action {
        "message.post" | "message.reply" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 9, "createdAt": 1_700_000_000u64 }),
        ),
        "reaction.add" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 7, "createdAt": 1_700_000_000u64 }),
        ),
        "profile.set" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 0, "createdAt": 1_700_000_000u64 }),
        ),
        "storage.address" => succeeded(
            request_id,
            action,
            serde_json::json!({
                "authorPubkey": FAKE_PUBKEY,
                "kind": 30174,
                "dTag": FAKE_EVENT_ID,
            }),
        ),
        "storage.get" => succeeded(
            request_id,
            action,
            serde_json::json!({ "value": "I am a keyless agent using broker-backed memory." }),
        ),
        "storage.put" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 30174, "createdAt": 1_700_000_000u64 }),
        ),
        "presence.set" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 20001, "createdAt": 1_700_000_000u64 }),
        ),
        "typing.set" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 20002, "createdAt": 1_700_000_000u64 }),
        ),
        "observer.emit" => succeeded(
            request_id,
            action,
            serde_json::json!({
                "accepted": request
                    .get("args")
                    .and_then(|args| args.get("frames"))
                    .and_then(|frames| frames.as_array())
                    .map_or(0, Vec::len),
            }),
        ),
        "liveness.ping" => succeeded(
            request_id,
            action,
            serde_json::json!({ "eventId": FAKE_EVENT_ID, "kind": 24200, "createdAt": 1_700_000_000u64 }),
        ),
        "channel.read" => succeeded(request_id, action, serde_json::json!({ "messages": [] })),
        other => serde_json::json!({
            "type": "broker_result",
            "protocolVersion": 1,
            "requestId": request_id,
            "status": "failed",
            "error": {
                "code": "unimplemented",
                "message": format!("mock broker does not implement '{other}'"),
            },
        }),
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("response")
}

fn succeeded(request_id: &str, action: &str, outcome: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "broker_result",
        "protocolVersion": 1,
        "requestId": request_id,
        "status": "succeeded",
        "action": action,
        "outcome": outcome,
    })
}
