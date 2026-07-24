//! Integration tests for [`RemoteTransport`] against an in-process fake
//! bridge speaking protocol v1 over a real WebSocket — dialed directly and
//! through an in-process SOCKS5 proxy.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use buzz_transport::protocol::{
    encode_bridge_frame, parse_client_frame, BridgeFrame, ClientFrame, MAX_FRAME_BYTES,
    PROTOCOL_VERSION,
};
use buzz_transport::remote::{RemoteTransport, RemoteTransportConfig};
use buzz_transport::{SignedEvent, Subscription, Transport, TransportError};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn signed_event(content: &str) -> SignedEvent {
    let event = EventBuilder::new(Kind::Custom(9), content)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    SignedEvent::from_nostr(&event).unwrap()
}

/// Instructions from a test to one bridge connection.
enum BridgeSend {
    Frame(BridgeFrame),
    Close,
}

/// One accepted bridge connection, post-handshake.
struct BridgeConn {
    /// Pubkey the client announced in its `hello`.
    hello_pubkey: String,
    /// Token carried in the `hello` frame, if any.
    hello_token: Option<String>,
    /// `Authorization` header from the WebSocket upgrade, if any.
    auth_header: Option<String>,
    /// Client frames received after `hello`.
    frames: mpsc::Receiver<ClientFrame>,
    /// Sends frames (or a close) to the client.
    send: mpsc::Sender<BridgeSend>,
}

/// Spawn a fake bridge that accepts connections, answers `hello` with a
/// `hello_ack` carrying `ack_version`, and hands each connection to the test.
// The Err variant's size is fixed by tungstenite's `accept_hdr_async`
// callback signature.
#[allow(clippy::result_large_err)]
async fn spawn_bridge(ack_version: u32) -> (String, mpsc::Receiver<BridgeConn>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (conn_tx, conn_rx) = mpsc::channel(4);

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let conn_tx = conn_tx.clone();
            tokio::spawn(async move {
                let auth_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                let slot = Arc::clone(&auth_slot);
                let callback = move |req: &Request, resp: Response| {
                    *slot.lock().unwrap() = req
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    Ok(resp)
                };
                let mut ws = tokio_tungstenite::accept_hdr_async(stream, callback)
                    .await
                    .unwrap();

                // Handshake: expect hello, reply hello_ack.
                let (hello_pubkey, hello_token) = loop {
                    match ws.next().await {
                        Some(Ok(Message::Text(text))) => {
                            match parse_client_frame(text.as_str()).unwrap() {
                                Some(ClientFrame::Hello {
                                    version,
                                    pubkey,
                                    token,
                                }) => {
                                    assert_eq!(version, PROTOCOL_VERSION);
                                    break (pubkey, token);
                                }
                                other => panic!("expected hello, got {other:?}"),
                            }
                        }
                        Some(Ok(_)) => continue,
                        other => panic!("connection ended before hello: {other:?}"),
                    }
                };
                let ack = BridgeFrame::HelloAck {
                    version: ack_version,
                };
                ws.send(Message::Text(encode_bridge_frame(&ack).unwrap().into()))
                    .await
                    .unwrap();

                let (frame_tx, frame_rx) = mpsc::channel(64);
                let (send_tx, mut send_rx) = mpsc::channel(64);
                let auth_header = auth_slot.lock().unwrap().clone();
                if conn_tx
                    .send(BridgeConn {
                        hello_pubkey,
                        hello_token,
                        auth_header,
                        frames: frame_rx,
                        send: send_tx,
                    })
                    .await
                    .is_err()
                {
                    return;
                }

                loop {
                    tokio::select! {
                        out = send_rx.recv() => match out {
                            Some(BridgeSend::Frame(frame)) => {
                                let text = encode_bridge_frame(&frame).unwrap();
                                if ws.send(Message::Text(text.into())).await.is_err() {
                                    return;
                                }
                            }
                            Some(BridgeSend::Close) | None => {
                                let _ = ws.close(None).await;
                                return;
                            }
                        },
                        msg = ws.next() => match msg {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(Some(frame)) = parse_client_frame(text.as_str()) {
                                    if frame_tx.send(frame).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                            Some(Ok(_)) => {}
                        },
                    }
                }
            });
        }
    });

    (format!("ws://127.0.0.1:{}/bridge", addr.port()), conn_rx)
}

fn config(url: &str, pubkey: &str, token: Option<&str>) -> RemoteTransportConfig {
    RemoteTransportConfig {
        url: url.to_string(),
        pubkey: pubkey.to_string(),
        token: token.map(str::to_owned),
        allow_insecure: false,
        socks_proxy: None,
    }
}

/// Spawn a fake bridge listening on a Unix domain socket, speaking the same
/// protocol as [`spawn_bridge`] but with one LF-terminated JSON frame per
/// line. Returns a `unix://` URL for [`RemoteTransportConfig::url`].
#[cfg(unix)]
async fn spawn_unix_bridge(ack_version: u32) -> (String, mpsc::Receiver<BridgeConn>) {
    use tokio_util::codec::{Framed, LinesCodec};

    // Unix socket paths must stay under SUN_LEN (104 bytes on macOS) —
    // keep the name short.
    let suffix = Uuid::new_v4().simple().to_string();
    let path =
        std::env::temp_dir().join(format!("bzt-{}-{}.sock", std::process::id(), &suffix[..8]));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let (conn_tx, conn_rx) = mpsc::channel(4);

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let conn_tx = conn_tx.clone();
            tokio::spawn(async move {
                let mut framed =
                    Framed::new(stream, LinesCodec::new_with_max_length(MAX_FRAME_BYTES));

                // Handshake: expect hello, reply hello_ack.
                let (hello_pubkey, hello_token) = match framed.next().await {
                    Some(Ok(line)) => match parse_client_frame(&line).unwrap() {
                        Some(ClientFrame::Hello {
                            version,
                            pubkey,
                            token,
                        }) => {
                            assert_eq!(version, PROTOCOL_VERSION);
                            (pubkey, token)
                        }
                        other => panic!("expected hello, got {other:?}"),
                    },
                    other => panic!("connection ended before hello: {other:?}"),
                };
                let ack = BridgeFrame::HelloAck {
                    version: ack_version,
                };
                framed
                    .send(encode_bridge_frame(&ack).unwrap())
                    .await
                    .unwrap();

                let (frame_tx, frame_rx) = mpsc::channel(64);
                let (send_tx, mut send_rx) = mpsc::channel(64);
                if conn_tx
                    .send(BridgeConn {
                        hello_pubkey,
                        hello_token,
                        auth_header: None,
                        frames: frame_rx,
                        send: send_tx,
                    })
                    .await
                    .is_err()
                {
                    return;
                }

                loop {
                    tokio::select! {
                        out = send_rx.recv() => match out {
                            Some(BridgeSend::Frame(frame)) => {
                                let text = encode_bridge_frame(&frame).unwrap();
                                if framed.send(text).await.is_err() {
                                    return;
                                }
                            }
                            Some(BridgeSend::Close) | None => {
                                let _ = SinkExt::<String>::close(&mut framed).await;
                                return;
                            }
                        },
                        line = framed.next() => match line {
                            Some(Ok(text)) => {
                                if let Ok(Some(frame)) = parse_client_frame(&text) {
                                    if frame_tx.send(frame).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            None | Some(Err(_)) => return,
                        },
                    }
                }
            });
        }
    });

    (format!("unix://{}", path.display()), conn_rx)
}

/// Authentication the fake SOCKS5 proxy demands from clients.
enum SocksAuth {
    None,
    UserPass {
        user: &'static str,
        pass: &'static str,
    },
}

/// Spawn a minimal in-process SOCKS5 server (RFC 1928 CONNECT, optional
/// RFC 1929 auth). Returns its URL and a counter of established tunnels.
async fn spawn_socks5(auth: SocksAuth) -> (String, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tunnels = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&tunnels);
    let auth = Arc::new(auth);

    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                return;
            };
            let counter = Arc::clone(&counter);
            let auth = Arc::clone(&auth);
            tokio::spawn(async move {
                // Method negotiation.
                let mut head = [0u8; 2];
                client.read_exact(&mut head).await.unwrap();
                assert_eq!(head[0], 0x05, "client must speak SOCKS5");
                let mut methods = vec![0u8; head[1] as usize];
                client.read_exact(&mut methods).await.unwrap();
                match &*auth {
                    SocksAuth::None => {
                        assert!(methods.contains(&0x00), "client must offer no-auth");
                        client.write_all(&[0x05, 0x00]).await.unwrap();
                    }
                    SocksAuth::UserPass { user, pass } => {
                        if !methods.contains(&0x02) {
                            client.write_all(&[0x05, 0xFF]).await.unwrap();
                            return;
                        }
                        client.write_all(&[0x05, 0x02]).await.unwrap();
                        let mut ver_ulen = [0u8; 2];
                        client.read_exact(&mut ver_ulen).await.unwrap();
                        assert_eq!(ver_ulen[0], 0x01);
                        let mut username = vec![0u8; ver_ulen[1] as usize];
                        client.read_exact(&mut username).await.unwrap();
                        let mut plen = [0u8; 1];
                        client.read_exact(&mut plen).await.unwrap();
                        let mut password = vec![0u8; plen[0] as usize];
                        client.read_exact(&mut password).await.unwrap();
                        if username == user.as_bytes() && password == pass.as_bytes() {
                            client.write_all(&[0x01, 0x00]).await.unwrap();
                        } else {
                            client.write_all(&[0x01, 0x01]).await.unwrap();
                            return;
                        }
                    }
                }

                // CONNECT: parse the target, dial it, splice the streams.
                let mut request = [0u8; 4];
                client.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[..3], &[0x05, 0x01, 0x00]);
                let target_host = match request[3] {
                    0x01 => {
                        let mut octets = [0u8; 4];
                        client.read_exact(&mut octets).await.unwrap();
                        std::net::IpAddr::from(octets).to_string()
                    }
                    0x03 => {
                        let mut len = [0u8; 1];
                        client.read_exact(&mut len).await.unwrap();
                        let mut domain = vec![0u8; len[0] as usize];
                        client.read_exact(&mut domain).await.unwrap();
                        String::from_utf8(domain).unwrap()
                    }
                    0x04 => {
                        let mut octets = [0u8; 16];
                        client.read_exact(&mut octets).await.unwrap();
                        std::net::IpAddr::from(octets).to_string()
                    }
                    other => panic!("unexpected SOCKS5 address type {other}"),
                };
                let mut port_bytes = [0u8; 2];
                client.read_exact(&mut port_bytes).await.unwrap();
                let target_port = u16::from_be_bytes(port_bytes);

                let mut upstream =
                    tokio::net::TcpStream::connect((target_host.as_str(), target_port))
                        .await
                        .unwrap();
                client
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });

    (format!("socks5://127.0.0.1:{}", addr.port()), tunnels)
}

async fn next_frame(conn: &mut BridgeConn) -> ClientFrame {
    timeout(TEST_TIMEOUT, conn.frames.recv())
        .await
        .expect("timed out waiting for client frame")
        .expect("bridge connection ended")
}

#[tokio::test]
async fn full_roundtrip_through_a_fake_bridge() {
    let (url, mut conns) = spawn_bridge(PROTOCOL_VERSION).await;
    let pubkey = Keys::generate().public_key().to_hex();

    let mut transport: Box<dyn Transport> = Box::new(
        RemoteTransport::connect(config(&url, &pubkey, Some("sekrit")))
            .await
            .unwrap(),
    );
    let mut conn = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();
    assert_eq!(conn.hello_pubkey, pubkey);
    assert_eq!(conn.auth_header.as_deref(), Some("Bearer sekrit"));
    assert_eq!(
        conn.hello_token.as_deref(),
        Some("sekrit"),
        "the token must also ride in the hello frame"
    );

    // Subscribe → the bridge sees the subscription verbatim.
    let channel_id = Uuid::new_v4();
    transport
        .subscribe(Subscription {
            channel_id,
            kinds: Some(vec![9, 40002]),
            require_mention: true,
            replay_since: Some(42),
        })
        .await
        .unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Subscribe {
            channel_id: ch,
            kinds,
            require_mention,
            replay_since,
        } => {
            assert_eq!(ch, channel_id);
            assert_eq!(kinds, Some(vec![9, 40002]));
            assert!(require_mention);
            assert_eq!(replay_since, Some(42));
        }
        other => panic!("expected subscribe frame, got {other:?}"),
    }

    // Bridge → client: a tampered event is dropped, a valid one delivered.
    let mut tampered = signed_event("evil");
    tampered.content = "tampered".to_string();
    conn.send
        .send(BridgeSend::Frame(BridgeFrame::Event {
            channel_id,
            event: Box::new(tampered),
        }))
        .await
        .unwrap();
    let valid = signed_event("hello from the bridge");
    conn.send
        .send(BridgeSend::Frame(BridgeFrame::Event {
            channel_id,
            event: Box::new(valid.clone()),
        }))
        .await
        .unwrap();
    let received = timeout(TEST_TIMEOUT, transport.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.channel_id, channel_id);
    assert_eq!(
        received.event, valid,
        "the tampered event must be dropped; only the valid one delivered"
    );

    // Client → bridge: publish and try_publish both land as event frames.
    let outbound = signed_event("hello from the agent");
    transport.publish(outbound.clone()).await.unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Event { event } => assert_eq!(*event, outbound),
        other => panic!("expected event frame, got {other:?}"),
    }
    let ephemeral = signed_event("typing…");
    transport.try_publish(ephemeral.clone()).unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Event { event } => assert_eq!(*event, ephemeral),
        other => panic!("expected event frame, got {other:?}"),
    }

    // Unsubscribe reaches the bridge.
    transport.unsubscribe(channel_id).await.unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Unsubscribe { channel_id: ch } => assert_eq!(ch, channel_id),
        other => panic!("expected unsubscribe frame, got {other:?}"),
    }

    timeout(TEST_TIMEOUT, transport.shutdown()).await.unwrap();
}

#[tokio::test]
async fn version_mismatch_fails_the_handshake() {
    let (url, _conns) = spawn_bridge(PROTOCOL_VERSION + 1).await;
    let pubkey = Keys::generate().public_key().to_hex();
    let err = RemoteTransport::connect(config(&url, &pubkey, None))
        .await
        .expect_err("handshake must fail on version mismatch");
    assert!(matches!(err, TransportError::Protocol(_)), "{err:?}");
}

#[tokio::test]
async fn plaintext_ws_to_remote_host_is_rejected_before_dialing() {
    // 192.0.2.0/24 is TEST-NET-1: never routable, and never dialed here
    // because validation rejects the URL first.
    let pubkey = Keys::generate().public_key().to_hex();
    let err = RemoteTransport::connect(config("ws://192.0.2.1:9/bridge", &pubkey, None))
        .await
        .expect_err("plaintext ws:// to a remote host must be rejected");
    assert!(matches!(err, TransportError::Insecure(_)), "{err:?}");
}

#[tokio::test]
async fn reconnect_resubscribes_and_flushes_buffered_publishes() {
    let (url, mut conns) = spawn_bridge(PROTOCOL_VERSION).await;
    let pubkey = Keys::generate().public_key().to_hex();

    let mut transport: Box<dyn Transport> = Box::new(
        RemoteTransport::connect(config(&url, &pubkey, None))
            .await
            .unwrap(),
    );
    let mut conn1 = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();

    let channel_id = Uuid::new_v4();
    transport
        .subscribe(Subscription::all(channel_id))
        .await
        .unwrap();
    assert!(matches!(
        next_frame(&mut conn1).await,
        ClientFrame::Subscribe { .. }
    ));

    // Bridge drops the connection: next_event yields the loss marker.
    conn1.send.send(BridgeSend::Close).await.unwrap();
    assert!(
        timeout(TEST_TIMEOUT, transport.next_event())
            .await
            .unwrap()
            .is_none(),
        "connection loss must surface as None"
    );

    // Publish while disconnected — buffered, not lost.
    let buffered = signed_event("published while offline");
    transport.publish(buffered.clone()).await.unwrap();

    // Reconnect: the transport re-dials, replays the subscription, then
    // flushes the buffered publish — in that order.
    transport.reconnect().await.unwrap();
    let mut conn2 = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();
    match next_frame(&mut conn2).await {
        ClientFrame::Subscribe { channel_id: ch, .. } => assert_eq!(ch, channel_id),
        other => panic!("expected resubscription first, got {other:?}"),
    }
    match next_frame(&mut conn2).await {
        ClientFrame::Event { event } => assert_eq!(*event, buffered),
        other => panic!("expected buffered publish, got {other:?}"),
    }

    // Live delivery works again on the new connection.
    let live = signed_event("after reconnect");
    conn2
        .send
        .send(BridgeSend::Frame(BridgeFrame::Event {
            channel_id,
            event: Box::new(live.clone()),
        }))
        .await
        .unwrap();
    let received = timeout(TEST_TIMEOUT, transport.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.event, live);

    timeout(TEST_TIMEOUT, transport.shutdown()).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn roundtrip_over_a_unix_socket() {
    let (url, mut conns) = spawn_unix_bridge(PROTOCOL_VERSION).await;
    let pubkey = Keys::generate().public_key().to_hex();

    // A unix:// carrier has no HTTP headers — the token rides in `hello`.
    let mut transport: Box<dyn Transport> = Box::new(
        RemoteTransport::connect(config(&url, &pubkey, Some("sock-sekrit")))
            .await
            .unwrap(),
    );
    let mut conn = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();
    assert_eq!(conn.hello_pubkey, pubkey);
    assert_eq!(conn.hello_token.as_deref(), Some("sock-sekrit"));

    // Full duplex over the socket: subscribe, receive, publish.
    let channel_id = Uuid::new_v4();
    transport
        .subscribe(Subscription::all(channel_id))
        .await
        .unwrap();
    assert!(matches!(
        next_frame(&mut conn).await,
        ClientFrame::Subscribe { .. }
    ));

    let inbound = signed_event("hello over the unix socket");
    conn.send
        .send(BridgeSend::Frame(BridgeFrame::Event {
            channel_id,
            event: Box::new(inbound.clone()),
        }))
        .await
        .unwrap();
    let received = timeout(TEST_TIMEOUT, transport.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.channel_id, channel_id);
    assert_eq!(received.event, inbound);

    let outbound = signed_event("published over the unix socket");
    transport.publish(outbound.clone()).await.unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Event { event } => assert_eq!(*event, outbound),
        other => panic!("expected event frame, got {other:?}"),
    }

    timeout(TEST_TIMEOUT, transport.shutdown()).await.unwrap();
}

#[tokio::test]
async fn roundtrip_through_a_socks5_proxy() {
    let (bridge_url, mut conns) = spawn_bridge(PROTOCOL_VERSION).await;
    let (proxy_url, tunnels) = spawn_socks5(SocksAuth::None).await;
    let pubkey = Keys::generate().public_key().to_hex();

    // Loopback destination through a loopback proxy — allowed without the
    // insecure opt-in.
    let mut cfg = config(&bridge_url, &pubkey, None);
    cfg.socks_proxy = Some(proxy_url);
    let mut transport: Box<dyn Transport> = Box::new(RemoteTransport::connect(cfg).await.unwrap());

    let mut conn = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();
    assert_eq!(conn.hello_pubkey, pubkey);
    assert_eq!(
        tunnels.load(Ordering::SeqCst),
        1,
        "the connection must actually flow through the proxy"
    );

    // Full duplex through the tunnel: subscribe, receive, publish.
    let channel_id = Uuid::new_v4();
    transport
        .subscribe(Subscription::all(channel_id))
        .await
        .unwrap();
    assert!(matches!(
        next_frame(&mut conn).await,
        ClientFrame::Subscribe { .. }
    ));

    let inbound = signed_event("hello through the tunnel");
    conn.send
        .send(BridgeSend::Frame(BridgeFrame::Event {
            channel_id,
            event: Box::new(inbound.clone()),
        }))
        .await
        .unwrap();
    let received = timeout(TEST_TIMEOUT, transport.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.event, inbound);

    let outbound = signed_event("published through the tunnel");
    transport.publish(outbound.clone()).await.unwrap();
    match next_frame(&mut conn).await {
        ClientFrame::Event { event } => assert_eq!(*event, outbound),
        other => panic!("expected event frame, got {other:?}"),
    }

    timeout(TEST_TIMEOUT, transport.shutdown()).await.unwrap();
}

#[tokio::test]
async fn socks5_username_password_authentication() {
    let (bridge_url, mut conns) = spawn_bridge(PROTOCOL_VERSION).await;
    let (proxy_url, tunnels) = spawn_socks5(SocksAuth::UserPass {
        user: "agent",
        pass: "hunter2",
    })
    .await;
    let pubkey = Keys::generate().public_key().to_hex();

    // Wrong password: the proxy rejects the subnegotiation and the dial
    // fails as a connection error.
    let mut wrong = config(&bridge_url, &pubkey, None);
    wrong.socks_proxy = Some(proxy_url.replace("socks5://", "socks5://agent:nope@"));
    let err = RemoteTransport::connect(wrong)
        .await
        .expect_err("wrong proxy credentials must fail the dial");
    assert!(matches!(err, TransportError::Connection(_)), "{err:?}");
    assert_eq!(tunnels.load(Ordering::SeqCst), 0);

    // Correct credentials: the tunnel opens and the handshake completes.
    let mut cfg = config(&bridge_url, &pubkey, None);
    cfg.socks_proxy = Some(proxy_url.replace("socks5://", "socks5://agent:hunter2@"));
    let transport = RemoteTransport::connect(cfg).await.unwrap();
    let conn = timeout(TEST_TIMEOUT, conns.recv()).await.unwrap().unwrap();
    assert_eq!(conn.hello_pubkey, pubkey);
    assert_eq!(tunnels.load(Ordering::SeqCst), 1);

    timeout(TEST_TIMEOUT, Box::new(transport).shutdown())
        .await
        .unwrap();
}

#[tokio::test]
async fn plaintext_ws_through_a_proxy_to_a_remote_host_is_rejected() {
    // Policy check happens before any dialing — the proxy never sees a
    // connection and the TEST-NET destination is never reached.
    let (proxy_url, tunnels) = spawn_socks5(SocksAuth::None).await;
    let pubkey = Keys::generate().public_key().to_hex();
    let mut cfg = config("ws://192.0.2.1:9/bridge", &pubkey, None);
    cfg.socks_proxy = Some(proxy_url);
    let err = RemoteTransport::connect(cfg)
        .await
        .expect_err("plaintext ws:// through a proxy to a remote host must be rejected");
    assert!(matches!(err, TransportError::Insecure(_)), "{err:?}");
    assert_eq!(tunnels.load(Ordering::SeqCst), 0);
}
