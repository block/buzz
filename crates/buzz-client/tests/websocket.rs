use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use buzz_client::{BuzzClient, BuzzClientConfig, BuzzIdentity, ClientError, RelayMessage};
use futures_util::StreamExt;
use nostr::{EventBuilder, Filter, Keys, Kind};
use serde_json::{json, Value};

#[derive(Clone)]
struct WsStubState {
    received: Arc<Mutex<Vec<Value>>>,
    auth_accepted: bool,
    publish_accepted: bool,
    outbound: Arc<Vec<Value>>,
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WsStubState>) -> Response {
    ws.on_upgrade(move |socket| run_ws_stub(socket, state))
}

async fn run_ws_stub(mut socket: WebSocket, state: WsStubState) {
    send_json(&mut socket, &json!(["AUTH", "test-challenge"])).await;
    let Some(auth_request) = recv_json(&mut socket).await else {
        return;
    };
    state.received.lock().unwrap().push(auth_request.clone());
    let auth_event_id = auth_request[1]["id"].as_str().unwrap();
    send_json(
        &mut socket,
        &json!([
            "OK",
            auth_event_id,
            state.auth_accepted,
            if state.auth_accepted {
                "authenticated"
            } else {
                "denied"
            }
        ]),
    )
    .await;
    if !state.auth_accepted {
        return;
    }

    let Some(request) = recv_json(&mut socket).await else {
        return;
    };
    state.received.lock().unwrap().push(request.clone());
    match request[0].as_str() {
        Some("EVENT") => {
            let event_id = request[1]["id"].as_str().unwrap();
            send_json(
                &mut socket,
                &json!([
                    "OK",
                    event_id,
                    state.publish_accepted,
                    if state.publish_accepted {
                        "published"
                    } else {
                        "rejected"
                    }
                ]),
            )
            .await;
        }
        Some("REQ") => {
            for message in state.outbound.iter() {
                send_json(&mut socket, message).await;
            }
            if let Some(close) = recv_json(&mut socket).await {
                state.received.lock().unwrap().push(close);
            }
        }
        _ => {}
    }
}

async fn send_json(socket: &mut WebSocket, value: &Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn recv_json(socket: &mut WebSocket) -> Option<Value> {
    while let Some(message) = socket.next().await {
        match message.ok()? {
            Message::Text(text) => return serde_json::from_str(text.as_str()).ok(),
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.ok()?,
            Message::Close(_) => return None,
            _ => {}
        }
    }
    None
}

async fn ws_stub(
    auth_accepted: bool,
    publish_accepted: bool,
    outbound: Vec<Value>,
) -> (String, Arc<Mutex<Vec<Value>>>) {
    let received = Arc::new(Mutex::new(Vec::new()));
    let state = WsStubState {
        received: received.clone(),
        auth_accepted,
        publish_accepted,
        outbound: Arc::new(outbound),
    };
    let app = Router::new().route("/", get(ws_handler)).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), received)
}

async fn wait_for_received(received: &Arc<Mutex<Vec<Value>>>, expected_len: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while received.lock().unwrap().len() < expected_len {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn client(relay_url: &str, with_auth_tag: bool) -> BuzzClient {
    let agent = Keys::generate();
    let auth_tag = with_auth_tag.then(|| {
        buzz_sdk::nip_oa::compute_auth_tag(&Keys::generate(), &agent.public_key(), "kind=20001")
            .unwrap()
    });
    BuzzClient::new(
        BuzzClientConfig::new(relay_url),
        BuzzIdentity::from_keys(agent, auth_tag.as_deref()).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn subscription_forwards_auth_and_typed_relay_lifecycle() {
    let event = EventBuilder::text_note("live")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let ok_id = "a".repeat(64);
    let (relay_url, received) = ws_stub(
        true,
        true,
        vec![
            json!(["EVENT", "live-sub", event]),
            json!(["EOSE", "live-sub"]),
            json!(["NOTICE", "maintenance soon"]),
            json!(["OK", ok_id, true, "accepted"]),
            json!(["AUTH", "new-challenge"]),
            json!(["CLOSED", "live-sub", "server restart"]),
        ],
    )
    .await;
    let client = client(&relay_url, true);
    let filters = [Filter::new().kind(Kind::TextNote)];
    let mut subscription = client.subscribe("live-sub", &filters).await.unwrap();
    assert_eq!(subscription.id(), "live-sub");

    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Event {
            ref subscription_id,
            ..
        }) if subscription_id == "live-sub"
    ));
    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Eose {
            ref subscription_id
        }) if subscription_id == "live-sub"
    ));
    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Notice { ref message }) if message == "maintenance soon"
    ));
    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Ok(ref response)) if response.accepted
    ));
    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Auth { ref challenge }) if challenge == "new-challenge"
    ));
    assert!(matches!(
        subscription.next_event(Duration::from_secs(1)).await,
        Ok(RelayMessage::Closed {
            ref subscription_id,
            ref message,
        }) if subscription_id == "live-sub" && message == "server restart"
    ));
    subscription.cancel().await.unwrap();
    wait_for_received(&received, 3).await;

    let received = received.lock().unwrap();
    assert_eq!(received[0][0], "AUTH");
    let auth_tags = received[0][1]["tags"].as_array().unwrap();
    assert_eq!(
        auth_tags
            .iter()
            .filter(|tag| tag[0].as_str() == Some("auth"))
            .count(),
        1
    );
    assert_eq!(received[1], json!(["REQ", "live-sub", filters[0]]));
    assert_eq!(received[2], json!(["CLOSE", "live-sub"]));
}

#[tokio::test]
async fn subscription_timeout_is_a_client_timeout_and_can_cancel() {
    let (relay_url, received) = ws_stub(true, true, Vec::new()).await;
    let client = client(&relay_url, false);
    let mut subscription = client
        .subscribe("quiet", &[Filter::new().kind(Kind::TextNote)])
        .await
        .unwrap();

    assert!(matches!(
        subscription.next_event(Duration::from_millis(10)).await,
        Err(ClientError::Timeout)
    ));
    subscription.cancel().await.unwrap();
    wait_for_received(&received, 3).await;
    assert_eq!(received.lock().unwrap()[2], json!(["CLOSE", "quiet"]));
}

#[tokio::test]
async fn ephemeral_publish_returns_typed_acceptance_and_rejection() {
    for accepted in [true, false] {
        let (relay_url, received) = ws_stub(true, accepted, Vec::new()).await;
        let client = client(&relay_url, true);
        let event = EventBuilder::new(Kind::Custom(20001), "online")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let event_id = event.id.to_hex();
        let result = client.publish_ephemeral(event).await;

        if accepted {
            let response = result.unwrap();
            assert_eq!(response.event_id, event_id);
            assert_eq!(response.message, "published");
        } else {
            assert!(matches!(
                result,
                Err(ClientError::Rejected {
                    event_id: rejected_id,
                    ref message,
                }) if rejected_id == event_id && message == "rejected"
            ));
        }
        assert_eq!(received.lock().unwrap()[1][0], "EVENT");
    }
}

#[tokio::test]
async fn authentication_rejection_is_typed() {
    let (relay_url, _) = ws_stub(false, true, Vec::new()).await;
    let client = client(&relay_url, false);

    assert!(matches!(
        client
            .subscribe("denied", &[Filter::new().kind(Kind::TextNote)])
            .await,
        Err(ClientError::WebSocket(
            buzz_ws_client::WsClientError::AuthFailed(_)
        ))
    ));
}
