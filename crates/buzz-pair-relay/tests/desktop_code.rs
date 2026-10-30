//! Full desktop-code wire exchange through the production relay and source state machine.
use std::{sync::Arc, time::Duration};

use buzz_core::pairing::{crypto, PairingSession, PayloadType, SessionState};
use buzz_pair_relay::{run_server, Relay};
use futures_util::{SinkExt, StreamExt};
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use zeroize::Zeroizing;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn receive(socket: &mut Socket) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("relay response deadline")
        .expect("connection open")
        .expect("frame");
    serde_json::from_str(frame.to_text().expect("text frame")).expect("relay JSON")
}

async fn subscribe(socket: &mut Socket, pubkey: PublicKey) {
    socket
        .send(Message::Text(
            json!(["REQ", "pair", {
                "kinds": [24134], "#p": [pubkey.to_hex()]
            }])
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(receive(socket).await[0], "EOSE");
}

async fn forward(sender: &mut Socket, receiver: &mut Socket, event: Event) -> Event {
    sender
        .send(Message::Text(json!(["EVENT", event]).to_string().into()))
        .await
        .unwrap();
    let ack = receive(sender).await;
    assert_eq!(ack[0], "OK");
    assert_eq!(ack[2], true, "relay must accept each protocol event: {ack}");
    let delivery = receive(receiver).await;
    assert_eq!(delivery[0], "EVENT");
    let received: Event = serde_json::from_value(delivery[2].clone()).unwrap();
    received.verify().unwrap();
    received
}

fn target_message(keys: &Keys, source: PublicKey, message: Value) -> Event {
    let encrypted = nip44::encrypt(
        keys.secret_key(),
        &source,
        message.to_string(),
        nip44::Version::V2,
    )
    .unwrap();
    EventBuilder::new(Kind::Custom(24134), encrypted)
        .tags([Tag::public_key(source)])
        .sign_with_keys(keys)
        .unwrap()
}

fn decrypt(keys: &Keys, event: &Event) -> Value {
    serde_json::from_str(&nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content).unwrap())
        .unwrap()
}

#[tokio::test]
async fn fifth_correct_guess_delivers_identity_and_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(run_server(listener, Arc::new(Relay::new())));
    let (mut source, qr) = PairingSession::new_source(url.clone());
    let target = Keys::generate();
    let (mut source_socket, _) = connect_async(&url).await.unwrap();
    let (mut target_socket, _) = connect_async(&url).await.unwrap();
    subscribe(&mut source_socket, source.pubkey()).await;
    subscribe(&mut target_socket, target.public_key()).await;
    let session_id = crypto::derive_session_id(&qr.session_secret);
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let offer = target_message(
        &target,
        source.pubkey(),
        json!({
            "type": "offer", "session_id": hex(&session_id), "version": 1,
            "confirmation": "desktop-code-v1"
        }),
    );
    let offer = forward(&mut target_socket, &mut source_socket, offer).await;
    assert!(source.handle_offer_with_confirmation(&offer).unwrap().1);
    let (code, challenge) = source.start_desktop_code().unwrap();
    let challenge = forward(&mut source_socket, &mut target_socket, challenge).await;
    assert_eq!(
        decrypt(&target, &challenge),
        json!({"type": "desktop-code"})
    );
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for attempt in 1..=5 {
        let submission = target_message(
            &target,
            source.pubkey(),
            json!({
                "type": "code-submit", "code": if attempt == 5 { &code } else { wrong },
                "request_id": attempt.to_string()
            }),
        );
        let submission = forward(&mut target_socket, &mut source_socket, submission).await;
        let (response, accepted) = source.handle_target_code(&submission).unwrap();
        assert_eq!(accepted, attempt == 5);
        let response = forward(&mut source_socket, &mut target_socket, response).await;
        let message = decrypt(&target, &response);
        if attempt < 5 {
            assert_eq!(
                message,
                json!({"type":"code-rejected", "request_id":attempt.to_string(), "remaining_attempts":5-attempt})
            );
        } else {
            let ecdh =
                nostr::util::generate_shared_key(target.secret_key(), &source.pubkey()).unwrap();
            let (_, sas_input) = crypto::derive_sas(&ecdh, &qr.session_secret);
            let proof = crypto::derive_transcript_hash(
                &session_id,
                &source.pubkey().to_bytes(),
                &target.public_key().to_bytes(),
                &sas_input,
                &qr.session_secret,
            );
            assert_eq!(
                message,
                json!({"type":"sas-confirm", "transcript_hash":hex(&proof)})
            );
        }
    }
    // Source event seven must reach the target and decrypt to the identity.
    let identity = Keys::generate();
    let secret = identity.secret_key().to_bech32().unwrap();
    let payload = source
        .send_payload(PayloadType::Nsec, Zeroizing::new(secret.clone()))
        .unwrap();
    let payload = forward(&mut source_socket, &mut target_socket, payload).await;
    let imported = decrypt(&target, &payload);
    assert_eq!(imported["payload"], secret);
    assert_eq!(
        Keys::parse(imported["payload"].as_str().unwrap())
            .unwrap()
            .public_key(),
        identity.public_key()
    );
    // Target event seven must reach source and finish the session.
    let complete = target_message(
        &target,
        source.pubkey(),
        json!({"type":"complete", "success":true}),
    );
    let complete = forward(&mut target_socket, &mut source_socket, complete).await;
    source.handle_complete(&complete).unwrap();
    assert_eq!(source.state(), SessionState::Completed);
    server.abort();
}
