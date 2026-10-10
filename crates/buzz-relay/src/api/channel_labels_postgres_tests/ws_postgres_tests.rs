//! Actual socket upgrade, NIP-42, EVENT/REQ/COUNT and local live delivery over the
//! production router. Real DB/Redis; no fabricated authenticated connection.

use std::{net::SocketAddr, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};

use super::*;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server(tokio::task::JoinHandle<std::io::Result<()>>);

impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn receive(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.expect("socket remains open").unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .expect("relay sends a bounded response")
}

async fn send(socket: &mut Socket, frame: Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .unwrap();
}

async fn connect(f: &Fixture, key: &Keys) -> (Server, Socket) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = crate::router::build_router(f.state.clone())
        .into_make_service_with_connect_info::<SocketAddr>();
    let server = Server(tokio::spawn(
        async move { axum::serve(listener, app).await },
    ));
    let mut request = format!("ws://{address}/").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", f.host.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    let challenge = receive(&mut socket).await;
    assert_eq!(challenge[0], "AUTH", "{challenge}");
    let relay = crate::api::bridge::nip42_expected_relay_url(
        &f.state.config.relay_url,
        &TenantContext::resolved(f.community, &f.host),
    );
    let auth = EventBuilder::auth(challenge[1].as_str().unwrap(), relay.parse().unwrap())
        .sign_with_keys(key)
        .unwrap();
    send(&mut socket, json!(["AUTH", auth])).await;
    let ack = receive(&mut socket).await;
    assert_eq!(ack[0], "OK", "{ack}");
    assert_eq!(ack[1], auth.id.to_hex(), "{ack}");
    assert_eq!(ack[2], true, "{ack}");
    (server, socket)
}

async fn query(socket: &mut Socket, subscription: &str, filter: Value) -> Vec<Event> {
    send(socket, json!(["REQ", subscription, filter])).await;
    let mut events = Vec::new();
    loop {
        let frame = receive(socket).await;
        assert_eq!(frame[1], subscription, "{frame}");
        match frame[0].as_str().unwrap() {
            "EVENT" => events.push(serde_json::from_value(frame[2].clone()).unwrap()),
            "EOSE" => return events,
            _ => panic!("unexpected query response {frame}"),
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn socket_discovery_ignores_stale_catalog_and_commands_deliver_canonical_live_state() {
    let f = Fixture::new().await;
    let channel = Uuid::new_v4();
    f.create(channel, &["team:infra"], "private").await;
    let original = f.snapshot(channel, &["team:infra"]).await;
    let (_server, mut socket) = connect(&f, &f.owner).await;

    // Model another serving replica whose channel-catalog invalidation is late.
    let cache_key = (f.community, f.owner.public_key().to_bytes().to_vec());
    f.state
        .accessible_channels_cache
        .insert(cache_key.clone(), vec![]);
    assert_eq!(
        f.state.accessible_channels_cache.get(&cache_key),
        Some(vec![])
    );
    let filter = json!({"kinds":[39000],"#L":["nip-cl"],"#l":["team:infra"],"limit":1});
    let http = f.read(&f.owner, "/query", filter.clone()).await;
    assert_eq!(
        http[0]["id"],
        original.id.to_hex(),
        "HTTP control must see the seeded channel"
    );
    let events = query(&mut socket, "catalog", filter.clone()).await;
    assert_eq!(
        events.len(),
        1,
        "WS discovery must not use stale catalog scope"
    );
    assert_eq!(events[0].id, original.id);
    send(&mut socket, json!(["COUNT", "count", filter])).await;
    let count = receive(&mut socket).await;
    assert_eq!(count[0], "COUNT", "{count}");
    assert_eq!(count[2]["count"], 1, "{count}");
    send(&mut socket, json!(["CLOSE", "catalog"])).await;
    assert_eq!(receive(&mut socket).await, json!(["CLOSED", "catalog", ""]));

    let initial = query(&mut socket, "live", json!({"kinds":[39000],"#h":[channel]})).await;
    assert_eq!(initial[0].id, original.id);
    let update = f.command(&f.owner, channel, 9002, &[("add-label", "live")]);
    send(&mut socket, json!(["EVENT", update])).await;
    // Publication and OK may arrive in either order, but both must arrive.
    let mut published = None;
    let mut acknowledged = false;
    for _ in 0..2 {
        let frame = receive(&mut socket).await;
        match frame[0].as_str().unwrap() {
            "EVENT" => {
                assert_eq!(frame[1], "live", "{frame}");
                let event: Event = serde_json::from_value(frame[2].clone()).unwrap();
                assert_eq!(
                    verify_snapshot(&event, f.state.relay_keypair.public_key(), channel)
                        .unwrap()
                        .values(),
                    &["live", "team:infra"]
                );
                assert!(event.created_at > original.created_at);
                published = Some(event);
            }
            "OK" => {
                assert_eq!(frame, json!(["OK", update.id, true, ""]));
                acknowledged = true;
            }
            _ => panic!("unexpected mutation response {frame}"),
        }
    }
    assert!(acknowledged && published.is_some());
    let published = published.unwrap();
    assert_eq!(
        f.snapshot(channel, &["live", "team:infra"]).await.id,
        published.id
    );
    send(&mut socket, json!(["EVENT", update])).await;
    assert_eq!(
        receive(&mut socket).await,
        json!(["OK", update.id, true, "duplicate: nip-cl-committed"])
    );
    send(&mut socket, json!(["CLOSE", "live"])).await;
    assert_eq!(receive(&mut socket).await, json!(["CLOSED", "live", ""]));
    let exact = query(
        &mut socket,
        "exact",
        json!({"kinds":[39000],"#d":[channel]}),
    )
    .await;
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].id, published.id);
    socket.close(None).await.unwrap();
}
