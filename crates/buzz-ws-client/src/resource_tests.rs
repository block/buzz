use super::*;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::accept_async;

async fn peer(frames: Vec<Message>) -> (String, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let (stop, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        for frame in frames {
            if ws.send(frame).await.is_err() {
                return;
            }
        }
        let _ = wait.await;
    });
    (url, stop, task)
}
fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}
fn options() -> ConnectionOptions {
    ConnectionOptions {
        connect_timeout: Duration::from_secs(2),
        authentication_timeout: Duration::from_secs(2),
        ..ConnectionOptions::default()
    }
}
#[tokio::test]
async fn invalid_budgets_fail_before_network() {
    let bad = ConnectionOptions {
        max_frame_bytes: 0,
        ..options()
    };
    assert!(matches!(
        NostrWsConnection::connect_with_options("invalid", bad).await,
        Err(WsClientError::InvalidConnectionOptions)
    ));
}
#[tokio::test]
async fn auth_count_flood_closes_and_discards_buffer() {
    let (url, stop, task) = peer(vec![
        text(json!(["NOTICE", "1"])),
        text(json!(["NOTICE", "2"])),
        text(json!(["NOTICE", "3"])),
    ])
    .await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            max_buffered_messages: 2,
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.authenticate(&Keys::generate(), None).await,
        Err(WsClientError::ResourceLimit)
    ));
    assert!(conn.ws.is_none());
    assert!(conn.buffer.is_empty());
    assert_eq!(conn.buffered_bytes, 0);
    assert!(matches!(
        conn.next_event(Duration::from_secs(1)).await,
        Err(WsClientError::ConnectionClosed)
    ));
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn auth_byte_flood_is_independent_of_count_limit() {
    let (url, stop, task) = peer(vec![text(json!(["NOTICE", "a moderately long notice"]))]).await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            max_buffered_bytes: 16,
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.authenticate(&Keys::generate(), None).await,
        Err(WsClientError::ResourceLimit)
    ));
    assert!(conn.ws.is_none());
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn oversized_frame_fails_before_json_parsing() {
    let (url, stop, task) = peer(vec![Message::Text("not JSON".repeat(128).into())]).await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            max_frame_bytes: 128,
            max_message_bytes: 256,
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.next_event(Duration::from_secs(2)).await,
        Err(WsClientError::ResourceLimit)
    ));
    assert!(conn.ws.is_none());
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn buffered_removal_and_challenge_delivery_account_exact_bytes() {
    let (url, stop, task) = peer(vec![]).await;
    let mut conn = NostrWsConnection::connect_with_options(&url, options())
        .await
        .unwrap();
    conn.enqueue(
        RelayMessage::Notice {
            message: "first".into(),
        },
        19,
    )
    .unwrap();
    conn.enqueue(
        RelayMessage::Auth {
            challenge: "again".into(),
        },
        16,
    )
    .unwrap();
    conn.enqueue(
        RelayMessage::Ok(OkResponse {
            event_id: "test".into(),
            accepted: true,
            message: String::new(),
        }),
        25,
    )
    .unwrap();
    assert_eq!(conn.buffered_bytes, 60);
    assert!(
        conn.wait_for_ok("test", Duration::from_secs(1))
            .await
            .unwrap()
            .accepted
    );
    assert_eq!(conn.buffered_bytes, 35);
    assert!(matches!(
        conn.next_event(Duration::from_secs(1)).await.unwrap(),
        RelayMessage::Notice { .. }
    ));
    assert_eq!(conn.buffered_bytes, 16);
    assert!(matches!(
        conn.next_event(Duration::from_secs(1)).await.unwrap(),
        RelayMessage::Auth { .. }
    ));
    assert_eq!(conn.buffered_bytes, 0);
    assert_eq!(
        conn.wait_for_auth_challenge(Duration::from_millis(50))
            .await
            .unwrap(),
        "again"
    );
    assert!(matches!(
        conn.wait_for_auth_challenge(Duration::from_millis(50))
            .await,
        Err(WsClientError::NoAuthChallenge)
    ));
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn authentication_succeeds_and_preserves_unrelated_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let (stop, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        ws.send(text(json!(["NOTICE", "before"]))).await.unwrap();
        ws.send(text(json!(["AUTH", "challenge"]))).await.unwrap();
        let Message::Text(raw) = ws.next().await.unwrap().unwrap() else {
            panic!("expected AUTH")
        };
        let value: Value = serde_json::from_str(&raw).unwrap();
        let event: Event = serde_json::from_value(value[1].clone()).unwrap();
        event.verify().unwrap();
        assert_eq!(event.kind, nostr::Kind::Authentication);
        ws.send(text(json!(["NOTICE", "after"]))).await.unwrap();
        ws.send(text(json!(["OK", event.id.to_hex(), true, ""])))
            .await
            .unwrap();
        let _ = wait.await;
    });
    let mut conn = NostrWsConnection::connect_authenticated_with_options(
        &url,
        &Keys::generate(),
        None,
        options(),
    )
    .await
    .unwrap();
    assert_eq!(conn.buffer.len(), 2);
    assert!(conn.buffered_bytes > 0);
    assert!(
        matches!(conn.next_event(Duration::from_secs(1)).await.unwrap(),RelayMessage::Notice {message} if message=="before")
    );
    assert!(
        matches!(conn.next_event(Duration::from_secs(1)).await.unwrap(),RelayMessage::Notice {message} if message=="after")
    );
    assert_eq!(conn.buffered_bytes, 0);
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn ping_chatter_does_not_extend_receive_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(tcp).await.unwrap();
        loop {
            if ws.send(Message::Ping(vec![1].into())).await.is_err() {
                break;
            }
            if ws.next().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut conn = NostrWsConnection::connect_with_options(&url, options())
        .await
        .unwrap();
    assert!(matches!(
        timeout(
            Duration::from_secs(1),
            conn.next_event(Duration::from_millis(50))
        )
        .await
        .unwrap(),
        Err(WsClientError::Timeout)
    ));
    drop(conn);
    task.abort();
}
#[tokio::test]
async fn overall_auth_deadline_drops_silent_transport() {
    let (url, stop, task) = peer(vec![]).await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            authentication_timeout: Duration::from_millis(50),
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.authenticate(&Keys::generate(), None).await,
        Err(WsClientError::Timeout)
    ));
    assert!(conn.ws.is_none());
    let _ = stop.send(());
    task.await.unwrap();
}

#[tokio::test]
async fn fragmented_message_limit_is_independent_of_frame_limit() {
    use tokio_tungstenite::tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    let frames = (0..5)
        .map(|index| {
            Message::Frame(Frame::message(
                vec![b'x'; 64],
                OpCode::Data(if index == 0 {
                    Data::Text
                } else {
                    Data::Continue
                }),
                index == 4,
            ))
        })
        .collect();
    let (url, stop, task) = peer(frames).await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            max_frame_bytes: 128,
            max_message_bytes: 256,
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.next_event(Duration::from_secs(2)).await,
        Err(WsClientError::ResourceLimit)
    ));
    assert!(conn.ws.is_none());
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn wait_for_ok_flood_enforces_both_queue_budgets() {
    for byte_limit in [false, true] {
        let frames = vec![
            text(json!(["NOTICE", "first"])),
            text(json!(["NOTICE", "second"])),
        ];
        let (url, stop, task) = peer(frames).await;
        let options = if byte_limit {
            ConnectionOptions {
                max_buffered_bytes: 8,
                ..options()
            }
        } else {
            ConnectionOptions {
                max_buffered_messages: 1,
                ..options()
            }
        };
        let mut conn = NostrWsConnection::connect_with_options(&url, options)
            .await
            .unwrap();
        assert!(matches!(
            conn.wait_for_ok("expected", Duration::from_secs(2)).await,
            Err(WsClientError::ResourceLimit)
        ));
        assert!(conn.ws.is_none());
        assert_eq!(conn.buffered_bytes, 0);
        let _ = stop.send(());
        task.await.unwrap();
    }
}
#[tokio::test]
async fn challenge_buffered_during_ok_wait_is_consumed_once() {
    let (url, stop, task) = peer(vec![
        text(json!(["AUTH", "again"])),
        text(json!(["OK", "expected", true, ""])),
    ])
    .await;
    let mut conn = NostrWsConnection::connect_with_options(&url, options())
        .await
        .unwrap();
    conn.wait_for_ok("expected", Duration::from_secs(1))
        .await
        .unwrap();
    assert!(conn.pending_challenge.is_none());
    assert_eq!(conn.buffer.len(), 1);
    assert_eq!(
        conn.wait_for_auth_challenge(Duration::from_secs(1))
            .await
            .unwrap(),
        "again"
    );
    assert_eq!(conn.buffered_bytes, 0);
    assert!(matches!(
        conn.wait_for_auth_challenge(Duration::from_millis(50))
            .await,
        Err(WsClientError::NoAuthChallenge)
    ));
    let _ = stop.send(());
    task.await.unwrap();
}
#[tokio::test]
async fn outbound_json_is_bounded_and_closes_on_overflow() {
    let (url, stop, task) = peer(vec![]).await;
    let mut conn = NostrWsConnection::connect_with_options(
        &url,
        ConnectionOptions {
            max_frame_bytes: 128,
            max_message_bytes: 128,
            ..options()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        conn.send_raw(&json!(["EVENT", "x".repeat(10000)])).await,
        Err(WsClientError::ResourceLimit)
    ));
    assert!(conn.ws.is_none());
    let _ = stop.send(());
    task.await.unwrap();
    let mut writer = LimitedJson {
        bytes: Vec::new(),
        limit: 128,
        exceeded: false,
    };
    assert!(serde_json::to_writer(&mut writer, &json!("x".repeat(10000))).is_err());
    assert!(writer.exceeded);
    assert!(writer.bytes.len() <= 128);
}
#[tokio::test]
async fn stalled_handshake_respects_connect_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let (stop, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (_tcp, _) = listener.accept().await.unwrap();
        let _ = wait.await;
    });
    assert!(matches!(
        NostrWsConnection::connect_with_options(
            &url,
            ConnectionOptions {
                connect_timeout: Duration::from_millis(50),
                ..options()
            }
        )
        .await,
        Err(WsClientError::Timeout)
    ));
    let _ = stop.send(());
    task.await.unwrap();
}
