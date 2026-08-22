use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use buzz_client::{
    BuzzClient, CommunityEndpoint, DeliveryOutcome, ListChannelsRequest, MessageKind,
    SendMessageRequest,
};
use nostr::Keys;
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct RelayState {
    submitted: Arc<Mutex<Vec<Value>>>,
}

#[tokio::test]
async fn independent_public_api_lists_then_sends() {
    let state = RelayState::default();
    let app = Router::new()
        .route(
            "/query",
            post(|| async {
                Json(json!([{
                    "id": "a".repeat(64),
                    "created_at": 1,
                    "kind": 39000,
                    "tags": [
                        ["d", "11111111-1111-1111-1111-111111111111"],
                        ["name", "independent"],
                        ["public"]
                    ]
                }]))
            }),
        )
        .route(
            "/events",
            post(
                |State(state): State<RelayState>, Json(event): Json<Value>| async move {
                    let event_id = event["id"].clone();
                    state
                        .submitted
                        .lock()
                        .expect("submitted lock")
                        .push(event);
                    Json(json!({
                        "event_id": event_id,
                        "accepted": true,
                        "message": "stored"
                    }))
                },
            ),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("test relay");
    });

    let client = BuzzClient::builder(
        CommunityEndpoint::parse(&format!("http://{address}")).expect("endpoint"),
        Arc::new(Keys::generate()),
    )
    .build()
    .await
    .expect("client");
    let channels = client
        .list_channels(ListChannelsRequest::default())
        .await
        .expect("channels");
    let channel = channels.first().expect("one channel");
    let result = client
        .send_message(SendMessageRequest {
            channel_id: channel.channel_id,
            content: "hello from outside the workspace".into(),
            kind: MessageKind::Stream,
            reply_to: None,
            broadcast: false,
            attachments: Vec::new(),
            mentions: Vec::new(),
        })
        .await
        .expect("send");

    assert!(matches!(result.delivery, DeliveryOutcome::Accepted(_)));
    let submitted = state.submitted.lock().expect("submitted lock");
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["kind"], 9);
    assert_eq!(submitted[0]["content"], "hello from outside the workspace");
}
