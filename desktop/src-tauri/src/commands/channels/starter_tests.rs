//! Starter-channel creation must report every channel it created, even when a
//! later step fails, so the frontend can route that channel's roster reads to
//! the writer.
use super::*;
use axum::{routing::post, Json, Router};
use nostr::Keys;
use serde_json::json;

#[tokio::test]
async fn created_starter_channels_are_reported_when_metadata_is_unavailable() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    // The relay accepts every create, but its replica never serves metadata.
    let router = Router::new()
        .route("/query", post(|| async { Json(json!([])) }))
        .route(
            "/events",
            post(|| async { Json(json!({"event_id": "0", "accepted": true, "message": ""})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = Keys::generate();
    *state.relay_url_override.lock().unwrap() = Some(url);

    let mut changed = Vec::new();
    let result = ensure_starter_channels_inner(&state, &mut changed).await;
    server.abort();

    assert_eq!(
        result.err().as_deref(),
        Some("starter channels created but metadata not yet available")
    );
    let scope = relay_api_base_url_with_override(&state);
    let expected: Vec<String> = STARTER_CHANNELS
        .iter()
        .map(|spec| starter_channel_uuid(&scope, spec.slug).to_string())
        .collect();
    assert_eq!(changed, expected);
}
