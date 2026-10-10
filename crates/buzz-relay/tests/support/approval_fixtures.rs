use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

pub async fn post_event(
    client: &reqwest::Client,
    url: &str,
    host: &str,
    keys: &Keys,
    event: &Event,
) -> Value {
    let body = serde_json::to_vec(event).unwrap();
    let auth = EventBuilder::new(Kind::HttpAuth, "")
        .tags([
            Tag::parse(["u", &format!("http://{host}/events")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
            Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    client
        .post(format!("{url}/events"))
        .header("host", host)
        .header(
            "authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&auth).unwrap())
            ),
        )
        .body(body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub async fn frame(ws: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Message::Text(text) = ws.next().await.unwrap().unwrap() {
                return serde_json::from_str(&text).unwrap();
            }
        }
    })
    .await
    .unwrap()
}
pub async fn connect(address: &str, host: &str, keys: &Keys) -> Socket {
    let mut request = format!("ws://{address}").into_client_request().unwrap();
    request.headers_mut().insert("host", host.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let challenge = frame(&mut ws).await;
    assert_eq!(challenge[0], "AUTH");
    let auth = EventBuilder::new(Kind::Authentication, "")
        .tags([
            Tag::parse(["relay", &format!("ws://{host}")]).unwrap(),
            Tag::parse(["challenge", challenge[1].as_str().unwrap()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    ws.send(Message::Text(json!(["AUTH", auth]).to_string().into()))
        .await
        .unwrap();
    let ack = frame(&mut ws).await;
    assert_eq!(ack[2], true, "WS auth failed: {ack}");
    ws
}
