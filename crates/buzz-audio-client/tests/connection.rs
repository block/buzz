use buzz_audio_client::connection::{connect, parse_peers};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::test]
async fn authenticates_with_host_signer_and_preserves_v2_media() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/huddle/room/audio", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket.send(Message::Ping(vec![7].into())).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"challenge","challenge":"host-challenge"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let auth = loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => break serde_json::from_str::<Value>(&text).unwrap(),
                Message::Pong(bytes) => assert_eq!(bytes.as_ref(), &[7]),
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(
            auth,
            json!({"type":"auth","event":{"signed":"host-challenge"},"parent_channel_id":"parent","protocol_version":2})
        );
        socket.send(Message::Text(json!({"type":"joined","peer_index":0,"peers":[{"peer_index":1,"pubkey":"a".repeat(64)}]}).to_string().into())).await.unwrap();
        socket
            .send(Message::Binary(vec![1, 2, 3].into()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Binary(vec![4, 5, 6].into())
        );
    });
    let mut joined = connect(&url, Some("parent"), None, |challenge| async move {
        assert_eq!(challenge, "host-challenge");
        Ok(json!({"signed":challenge}))
    })
    .await
    .unwrap();
    assert_eq!(joined.index, 0);
    assert_eq!(joined.peers[0].epoch, 0);
    assert_eq!(joined.peers[0].pubkey, "a".repeat(64));
    assert_eq!(
        joined.socket.next().await.unwrap().unwrap(),
        Message::Binary(vec![1, 2, 3].into())
    );
    joined
        .socket
        .send(Message::Binary(vec![4, 5, 6].into()))
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn invalid_challenge_never_reaches_signer() {
    for challenge in [
        json!({"type":"joined","challenge":"wrong"}),
        json!({"type":"challenge","challenge":""}),
        json!({"type":"challenge","challenge":"x".repeat(4097)}),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(challenge.to_string().into()))
                .await
                .unwrap();
            assert!(socket
                .next()
                .await
                .is_none_or(|r| r.is_err() || matches!(r, Ok(Message::Close(_)))));
        });
        assert!(connect(&url, None, None, |_| async {
            panic!("must not sign an invalid challenge")
        })
        .await
        .is_err());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_admission_drops_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (ready, received) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        ready.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(result.is_none_or(|r| r.is_err() || matches!(r, Ok(Message::Close(_)))));
    });
    let client = tokio::spawn(async move {
        connect(&url, None, None, |_| async { panic!("no challenge") }).await
    });
    received.await.unwrap();
    client.abort();
    assert!(matches!(client.await, Err(e) if e.is_cancelled()));
    server.await.unwrap();
}

#[test]
fn rejects_invalid_rosters_without_truncation() {
    for peers in [
        json!([{"peer_index":256,"pubkey":"a".repeat(64)}]),
        json!([{"peer_index":1,"pubkey":"bad"}]),
        json!([{"peer_index":1,"pubkey":"a".repeat(64),"epoch":256}]),
        json!([{"peer_index":1,"pubkey":"a".repeat(64)},{"peer_index":1,"pubkey":"b".repeat(64)}]),
    ] {
        assert!(parse_peers(&json!({"peers":peers})).is_err());
    }
    assert!(parse_peers(
        &json!({"peers":vec![json!({"peer_index":1,"pubkey":"a".repeat(64)});257]})
    )
    .is_err());
}
