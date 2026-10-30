//! Starter-channel creation must report every channel it created, even when a
//! later step fails, so the frontend can route that channel's roster reads to
//! the writer.
use super::*;
use axum::{routing::post, Json, Router};
use nostr::Keys;
use serde_json::json;

type Submitted = std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

/// A relay that accepts and records every submitted event but never serves
/// any from `/query`.
async fn accepting_relay() -> (String, Submitted, tokio::task::JoinHandle<()>) {
    let submitted = Submitted::default();
    let captured = submitted.clone();
    let router = Router::new()
        .route("/query", post(|| async { Json(json!([])) }))
        .route(
            "/events",
            post(move |Json(event): Json<serde_json::Value>| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push(event);
                    Json(json!({"event_id": "0", "accepted": true, "message": ""}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, submitted, server)
}

#[tokio::test]
async fn created_starter_channels_are_reported_when_metadata_is_unavailable() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    // The relay accepts every create, but its replica never serves metadata.
    let (url, _submitted, server) = accepting_relay().await;
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

fn starter_test_channel(id: &str, name: &str, is_member: bool) -> ChannelInfo {
    ChannelInfo {
        id: id.to_string(),
        name: name.to_string(),
        channel_type: "stream".to_string(),
        visibility: "open".to_string(),
        description: String::new(),
        topic: None,
        purpose: None,
        member_count: 0,
        member_pubkeys: Vec::new(),
        last_message_at: None,
        archived_at: None,
        participants: Vec::new(),
        participant_pubkeys: Vec::new(),
        is_member,
        ttl_seconds: None,
        ttl_deadline: None,
    }
}

#[tokio::test]
async fn existing_join_only_starter_channel_is_joined_never_created() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (url, submitted, server) = accepting_relay().await;
    let state = crate::app_state::build_app_state();
    let keys = Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(url);

    let announcements_id = "6d5d7b9e-0277-44e5-9448-dbc967a3cea9";
    let mut channels = vec![
        starter_test_channel("029f8049-8716-4407-94df-be8d1622b7af", "general", true),
        starter_test_channel(
            "fee88e92-da9f-5958-a910-48db3264ad38",
            "welcome-everyone",
            true,
        ),
        starter_test_channel(announcements_id, "Announcements", false),
        starter_test_channel("e62570dd-33ad-42c5-b92b-75f2689f9694", "random", false),
    ];
    let mut changed = Vec::new();
    let result =
        ensure_starter_channel_memberships(&state, &keys, &mut channels, &mut changed).await;
    server.abort();

    assert_eq!(result, Ok(()));
    assert_eq!(changed, vec![announcements_id.to_string()]);
    assert!(channels[2].is_member);
    assert!(!channels[3].is_member, "non-starter channels stay unjoined");
    let submitted = submitted.lock().unwrap();
    assert_eq!(submitted.len(), 1, "only the join is sent: {submitted:?}");
    assert_eq!(submitted[0]["kind"], json!(9021));
    assert_eq!(submitted[0]["tags"][0], json!(["h", announcements_id]));
}
