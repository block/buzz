use std::sync::Arc;
use std::time::Duration;

use buzz_core::pairing::qr::decode_qr;
use buzz_core::pairing::{PairingSession, PayloadType};
use buzz_pair_relay::{run_server, Relay};
use buzz_pairing_client::{
    PairingCommand, PairingEvent, PairingSourceConfig, PairingSourceRuntime,
};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, JsonUtil, Keys, ToBech32};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const SUBSCRIPTION_ID: &str = "pair";

#[tokio::test]
async fn source_runtime_pairs_with_a_nip_ab_target_over_an_open_relay() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pairing relay");
    let address = listener.local_addr().expect("pairing relay address");
    tokio::spawn(run_server(listener, Arc::new(Relay::new())));

    let source_keys = Keys::generate();
    let source_nsec = source_keys
        .secret_key()
        .to_bech32()
        .expect("encode source nsec");
    let relay_http_url = format!("http://{address}");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut source = PairingSourceRuntime::new();
    source
        .start(
            PairingSourceConfig::new(relay_http_url.clone(), source_nsec.clone()),
            event_tx,
        )
        .await;

    let uri = match next_source_event(&mut event_rx).await {
        PairingEvent::Ready { uri } => uri,
        other => panic!("expected ready event, got {other:?}"),
    };
    let qr = decode_qr(uri.as_str()).expect("decode source QR");
    let pairing_relay_url = qr.relays.first().expect("QR relay").clone();
    let (mut target, offer) = PairingSession::new_target(&qr).expect("create target session");
    let target_sas = target.sas_code().expect("target SAS").to_string();

    let (mut target_socket, _) = tokio_tungstenite::connect_async(&pairing_relay_url)
        .await
        .expect("connect target");
    target_socket
        .send(Message::Text(
            serde_json::json!([
                "REQ",
                SUBSCRIPTION_ID,
                {
                    "kinds": [buzz_core::kind::KIND_PAIRING],
                    "#p": [target.pubkey().to_hex()],
                }
            ])
            .to_string()
            .into(),
        ))
        .await
        .expect("subscribe target");
    wait_for_eose(&mut target_socket).await;
    publish(&mut target_socket, &offer).await;

    match next_source_event(&mut event_rx).await {
        PairingEvent::SasReceived { code } => assert_eq!(code, target_sas),
        other => panic!("expected SAS event, got {other:?}"),
    }
    source
        .send(PairingCommand::ConfirmSas)
        .expect("confirm source SAS");

    let sas_confirm = next_target_event(&mut target_socket).await;
    assert_eq!(
        target
            .handle_sas_confirm(&sas_confirm)
            .expect("handle source confirmation"),
        target_sas
    );
    target.confirm_target_sas().expect("confirm target SAS");

    let payload_event = next_target_event(&mut target_socket).await;
    let (payload_type, payload) = target
        .handle_payload(&payload_event)
        .expect("handle source payload");
    assert_eq!(payload_type, PayloadType::Custom);
    let payload: Value = serde_json::from_str(payload.as_str()).expect("parse custom payload");
    assert_eq!(
        payload.get("relayUrl").and_then(Value::as_str),
        Some(relay_http_url.as_str())
    );
    assert_eq!(
        payload.get("pubkey").and_then(Value::as_str),
        Some(source_keys.public_key().to_hex().as_str())
    );
    assert_eq!(
        payload.get("nsec").and_then(Value::as_str),
        Some(source_nsec.as_str())
    );

    let complete = target.send_complete().expect("build complete event");
    publish(&mut target_socket, &complete).await;
    assert_eq!(
        next_source_event(&mut event_rx).await,
        PairingEvent::Complete
    );
    source.stop().await;
}

async fn next_source_event(receiver: &mut mpsc::UnboundedReceiver<PairingEvent>) -> PairingEvent {
    tokio::time::timeout(TEST_TIMEOUT, receiver.recv())
        .await
        .expect("source event timeout")
        .expect("source event channel closed")
}

async fn wait_for_eose<S>(socket: &mut S)
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = tokio::time::timeout(TEST_TIMEOUT, socket.next())
            .await
            .expect("EOSE timeout")
            .expect("target socket closed")
            .expect("target socket error");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(text.as_str()).expect("relay JSON");
        if value.as_array().is_some_and(|array| {
            array.first().and_then(Value::as_str) == Some("EOSE")
                && array.get(1).and_then(Value::as_str) == Some(SUBSCRIPTION_ID)
        }) {
            return;
        }
    }
}

async fn next_target_event<S>(socket: &mut S) -> Event
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = tokio::time::timeout(TEST_TIMEOUT, socket.next())
            .await
            .expect("target event timeout")
            .expect("target socket closed")
            .expect("target socket error");
        let Message::Text(text) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(text.as_str()).expect("relay JSON");
        let Some(array) = value.as_array() else {
            continue;
        };
        if array.first().and_then(Value::as_str) == Some("EVENT")
            && array.get(1).and_then(Value::as_str) == Some(SUBSCRIPTION_ID)
        {
            return serde_json::from_value(array.get(2).expect("event value").clone())
                .expect("parse event");
        }
    }
}

async fn publish<S>(socket: &mut S, event: &Event)
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    socket
        .send(Message::Text(
            format!("[\"EVENT\",{}]", JsonUtil::as_json(event)).into(),
        ))
        .await
        .expect("publish event");
}
