//! End-to-end test of the whole seam: two [`RemoteTransport`] clients talk
//! protocol v1 over real WebSockets to a bridge whose "network" is an
//! [`InMemoryHub`] — the same shape as a production bridge onto Slack or a
//! private mesh, with the hub playing the private network.
//!
//! ```text
//! alice: RemoteTransport ──ws──┐                ┌──ws── bob: RemoteTransport
//!                              ├── hub bridge ──┤
//!                              └─ InMemoryHub ──┘
//! ```

use std::collections::HashMap;

use buzz_transport::memory::InMemoryHub;
use buzz_transport::protocol::{encode_bridge_frame, parse_client_frame, BridgeFrame, ClientFrame};
use buzz_transport::remote::{RemoteTransport, RemoteTransportConfig};
use buzz_transport::{SignedEvent, Subscription, Transport, TransportEvent};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn signed_event(keys: &Keys, channel: Uuid, content: &str) -> SignedEvent {
    let event = EventBuilder::new(Kind::Custom(9), content)
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    SignedEvent::from_nostr(&event).unwrap()
}

/// Spawn a bridge that serves protocol v1 over WebSocket and carries events
/// between clients through `hub`. Each `subscribe` a client sends is
/// acknowledged on `subs_seen` once it is live on the hub, so tests can
/// publish without racing subscription registration.
async fn spawn_hub_bridge(hub: InMemoryHub) -> (String, mpsc::UnboundedReceiver<(String, Uuid)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (subs_tx, subs_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let hub = hub.clone();
            let subs_tx = subs_tx.clone();
            tokio::spawn(async move {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut ws_sink, mut ws_stream) = ws.split();

                // Handshake: hello → hello_ack, remembering the client pubkey.
                let pubkey = match ws_stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        match parse_client_frame(text.as_str()).unwrap() {
                            Some(ClientFrame::Hello { pubkey, .. }) => pubkey,
                            other => panic!("expected hello, got {other:?}"),
                        }
                    }
                    other => panic!("connection ended before hello: {other:?}"),
                };
                let ack = BridgeFrame::HelloAck {
                    version: buzz_transport::protocol::PROTOCOL_VERSION,
                };
                ws_sink
                    .send(Message::Text(encode_bridge_frame(&ack).unwrap().into()))
                    .await
                    .unwrap();

                // Writer task: everything the bridge sends funnels through
                // one channel so delivery pumps never touch the sink.
                let (out_tx, mut out_rx) = mpsc::channel::<BridgeFrame>(64);
                tokio::spawn(async move {
                    while let Some(frame) = out_rx.recv().await {
                        let text = encode_bridge_frame(&frame).unwrap();
                        if ws_sink.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                });

                // The client's outbound events enter the hub through one
                // participant; each subscription gets its own participant
                // whose pump forwards hub deliveries back over the socket.
                let publisher = hub.connect(pubkey.clone());
                let mut pumps: HashMap<Uuid, tokio::task::JoinHandle<()>> = HashMap::new();

                while let Some(msg) = ws_stream.next().await {
                    let Ok(Message::Text(text)) = msg else {
                        break;
                    };
                    match parse_client_frame(text.as_str()) {
                        Ok(Some(ClientFrame::Subscribe {
                            channel_id,
                            kinds,
                            require_mention,
                            replay_since,
                        })) => {
                            // Register on the hub *before* spawning the pump
                            // so no event published after this frame is lost.
                            let mut conn = hub.connect(pubkey.clone());
                            conn.subscribe(Subscription {
                                channel_id,
                                kinds,
                                require_mention,
                                replay_since,
                            })
                            .await
                            .unwrap();
                            let _ = subs_tx.send((pubkey.clone(), channel_id));

                            let own_pubkey = pubkey.clone();
                            let out_tx = out_tx.clone();
                            let pump = tokio::spawn(async move {
                                while let Some(TransportEvent { channel_id, event }) =
                                    conn.next_event().await
                                {
                                    // This bridge chooses not to echo a
                                    // client's own events back to it.
                                    if event.pubkey == own_pubkey {
                                        continue;
                                    }
                                    let frame = BridgeFrame::Event {
                                        channel_id,
                                        event: Box::new(event),
                                    };
                                    if out_tx.send(frame).await.is_err() {
                                        return;
                                    }
                                }
                            });
                            // Re-subscribing replaces the previous pump.
                            if let Some(old) = pumps.insert(channel_id, pump) {
                                old.abort();
                            }
                        }
                        Ok(Some(ClientFrame::Unsubscribe { channel_id })) => {
                            if let Some(pump) = pumps.remove(&channel_id) {
                                pump.abort();
                            }
                        }
                        Ok(Some(ClientFrame::Event { event })) => {
                            let event_id = event.id.clone();
                            if let Err(e) = publisher.try_publish(*event) {
                                let nack = BridgeFrame::Ok {
                                    event_id,
                                    accepted: false,
                                    message: e.to_string(),
                                };
                                let _ = out_tx.send(nack).await;
                            }
                        }
                        _ => {}
                    }
                }
                for pump in pumps.into_values() {
                    pump.abort();
                }
            });
        }
    });

    (format!("ws://127.0.0.1:{}/bridge", addr.port()), subs_rx)
}

async fn connect(url: &str, pubkey: String) -> RemoteTransport {
    RemoteTransport::connect(RemoteTransportConfig {
        url: url.to_string(),
        pubkey,
        token: None,
        allow_insecure: false,
        socks_proxy: None,
    })
    .await
    .unwrap()
}

async fn recv(transport: &mut dyn Transport) -> TransportEvent {
    timeout(TEST_TIMEOUT, transport.next_event())
        .await
        .expect("timed out waiting for an event")
        .expect("stream ended unexpectedly")
}

#[tokio::test]
async fn two_remote_clients_exchange_events_through_a_hub_backed_bridge() {
    let hub = InMemoryHub::new();
    let (url, mut subs_seen) = spawn_hub_bridge(hub).await;

    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let alice_pubkey = alice_keys.public_key().to_hex();
    let bob_pubkey = bob_keys.public_key().to_hex();
    let channel = Uuid::new_v4();

    let mut alice: Box<dyn Transport> = Box::new(connect(&url, alice_pubkey.clone()).await);
    let mut bob: Box<dyn Transport> = Box::new(connect(&url, bob_pubkey.clone()).await);

    alice.subscribe(Subscription::all(channel)).await.unwrap();
    bob.subscribe(Subscription::all(channel)).await.unwrap();

    // Wait until the bridge has both subscriptions live on the hub —
    // publishing earlier could race the (fire-and-forget) subscribe frames.
    let mut pending = vec![alice_pubkey.clone(), bob_pubkey.clone()];
    while !pending.is_empty() {
        let (pubkey, sub_channel) = timeout(TEST_TIMEOUT, subs_seen.recv())
            .await
            .expect("timed out waiting for subscriptions to register")
            .expect("bridge stopped");
        assert_eq!(sub_channel, channel);
        pending.retain(|p| p != &pubkey);
    }

    // Alice → hub → Bob.
    alice
        .publish(signed_event(&alice_keys, channel, "hi bob"))
        .await
        .unwrap();
    let bob_got = recv(bob.as_mut()).await;
    assert_eq!(bob_got.channel_id, channel);
    assert_eq!(bob_got.event.content, "hi bob");
    assert_eq!(bob_got.event.pubkey, alice_pubkey);
    bob_got.event.verify().unwrap();

    // Bob → hub → Alice. Alice's first delivery is Bob's reply, not an echo
    // of her own event.
    bob.publish(signed_event(&bob_keys, channel, "hi alice"))
        .await
        .unwrap();
    let alice_got = recv(alice.as_mut()).await;
    assert_eq!(alice_got.event.content, "hi alice");
    assert_eq!(alice_got.event.pubkey, bob_pubkey);

    alice.shutdown().await;
    bob.shutdown().await;
}
