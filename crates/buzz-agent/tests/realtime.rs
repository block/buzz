use base64::{engine::general_purpose::STANDARD, Engine};
use buzz_agent::realtime::RealtimeConnection;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

async fn receive<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let message = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

// tungstenite fixes the callback error type to an HTTP response.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn realtime_audio_tools_cancel_and_truncate_wire() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "ws://{}/v1/realtime?model=frankie",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(
            tcp,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.uri(), "/v1/realtime?model=frankie");
                assert_eq!(request.headers()["authorization"], "Bearer local-test");
                assert!(!request.headers().contains_key("OpenAI-Beta"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        ws.send(Message::Text(
            json!({"type":"session.created","session":{"id":"s1","type":"realtime"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let update = receive(&mut ws).await;
        assert_eq!(update["type"], "session.update");
        assert_eq!(
            update["session"]["audio"]["input"]["format"],
            json!({"type":"audio/pcm","rate":24000})
        );
        assert_eq!(update["session"]["output_modalities"], json!(["audio"]));
        ws.send(Message::Text(
            json!({"type":"session.updated"}).to_string().into(),
        ))
        .await
        .unwrap();
        let audio = receive(&mut ws).await;
        assert_eq!(audio["type"], "input_audio_buffer.append");
        assert_eq!(
            STANDARD.decode(audio["audio"].as_str().unwrap()).unwrap(),
            vec![1, 0, 2, 0]
        );
        assert_eq!(
            receive(&mut ws).await,
            json!({"type":"input_audio_buffer.commit"})
        );
        assert_eq!(receive(&mut ws).await, json!({"type":"response.create"}));
        for event in [
            json!({"type":"response.created","response":{"id":"r1"}}),
            json!({"type":"response.output_audio.delta","response_id":"r1","item_id":"i1","content_index":0,"delta":"AQACAA=="}),
            json!({"type":"response.function_call_arguments.done","response_id":"r1","call_id":"c1","name":"lookup","arguments":"{}"}),
        ] {
            ws.send(Message::Text(event.to_string().into()))
                .await
                .unwrap();
        }
        let output = receive(&mut ws).await;
        assert_eq!(
            output,
            json!({"type":"conversation.item.create","item":{"type":"function_call_output","call_id":"c1","output":"found"}})
        );
        assert_eq!(
            receive(&mut ws).await,
            json!({"type":"response.cancel","response_id":"r1"})
        );
        assert_eq!(
            receive(&mut ws).await,
            json!({"type":"conversation.item.truncate","item_id":"i1","content_index":0,"audio_end_ms":42})
        );
        assert_eq!(
            receive(&mut ws).await,
            json!({"type":"input_audio_buffer.clear"})
        );
        ws.send(Message::Text(
            json!({"type":"response.done","response":{"id":"r1","status":"cancelled"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        assert!(matches!(
            ws.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));
    });
    let connection = RealtimeConnection::connect(&endpoint, "local-test")
        .await
        .unwrap();
    let (mut client, mut reader) = connection.split();
    client.update_session(json!({"type":"realtime","output_modalities":["audio"],"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"turn_detection":null}}})).await.unwrap();
    assert_eq!(
        reader.next_event().await.unwrap()["type"],
        "session.updated"
    );
    assert!(client.append_audio(&[0]).await.is_err());
    assert!(client.append_audio(&vec![0; 48002]).await.is_err());
    client.append_audio(&[1, 0, 2, 0]).await.unwrap();
    client.commit_audio().await.unwrap();
    client.create_response().await.unwrap();
    assert_eq!(reader.next_event().await.unwrap()["response"]["id"], "r1");
    assert_eq!(reader.next_event().await.unwrap()["delta"], "AQACAA==");
    assert_eq!(reader.next_event().await.unwrap()["call_id"], "c1");
    client
        .create_item(json!({"type":"function_call_output","call_id":"c1","output":"found"}))
        .await
        .unwrap();
    client.cancel_response("r1").await.unwrap();
    client.truncate_audio("i1", 0, 42).await.unwrap();
    client.clear_audio().await.unwrap();
    assert_eq!(
        reader.next_event().await.unwrap()["response"]["status"],
        "cancelled"
    );
    client.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn invalid_frames_poison_connection_without_echoing_provider_data() {
    for message in [
        Message::Text("not-json secret".into()),
        Message::Text(json!({"not_type":"secret"}).to_string().into()),
        Message::Text(
            json!({"type":"error","error":{"message":"secret"}})
                .to_string()
                .into(),
        ),
        Message::Binary(vec![1, 2].into()),
        Message::Text("x".repeat(1024 * 1024 + 1).into()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(tcp).await.unwrap();
            ws.send(Message::Text(
                json!({"type":"session.created"}).to_string().into(),
            ))
            .await
            .unwrap();
            // The receiver rejects oversize frames from their header and stops draining.
            let _ = tokio::time::timeout(Duration::from_secs(5), ws.send(message)).await;
        });
        let connection = RealtimeConnection::connect(&endpoint, "test")
            .await
            .unwrap();
        let (client, mut reader) = connection.split();
        let error = reader.next_event().await.unwrap_err().to_string();
        assert!(!error.contains("secret"));
        assert!(reader
            .next_event()
            .await
            .unwrap_err()
            .to_string()
            .contains("terminal"));
        drop(client);
        drop(reader);
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn credentials_never_use_non_loopback_cleartext() {
    for endpoint in [
        "ws://example.com/v1/realtime",
        "https://example.com",
        "wss://user:secret@example.com",
        "wss://example.com/#fragment",
    ] {
        assert!(RealtimeConnection::connect(endpoint, "test").await.is_err());
    }
}

#[tokio::test]
async fn dropping_read_is_terminal_not_silent_resume() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"session.created"}).to_string().into(),
        ))
        .await
        .unwrap();
        let _ = done_rx.await;
    });
    let connection = RealtimeConnection::connect(&endpoint, "test")
        .await
        .unwrap();
    let (_client, mut reader) = connection.split();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), reader.next_event())
            .await
            .is_err()
    );
    assert!(reader
        .next_event()
        .await
        .unwrap_err()
        .to_string()
        .contains("terminal"));
    done_tx.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn pending_provider_read_does_not_block_cancellation_write() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"session.created", "session":{"id":"duplex"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        // No response until the client writes: a serial read-then-write deadlocks.
        assert_eq!(
            receive(&mut ws).await,
            json!({"type":"response.cancel","response_id":"r-duplex"})
        );
        ws.send(Message::Text(
            json!({"type":"response.done","response":{"id":"r-duplex","status":"cancelled"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    });
    let connection = RealtimeConnection::connect(&endpoint, "test")
        .await
        .unwrap();
    assert_eq!(connection.session()["id"], "duplex");
    let (mut sender, mut receiver) = connection.split();
    let read = tokio::spawn(async move { receiver.next_event().await });
    sender.cancel_response("r-duplex").await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(terminal["response"]["status"], "cancelled");
    server.await.unwrap();
}
