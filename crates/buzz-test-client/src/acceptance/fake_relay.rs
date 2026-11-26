//! A scripted relay for the probe's tests: just enough NIP-01 and NIP-42 to
//! run every check, with knobs to delay, drop or close on chosen writes,
//! or to corrupt the events it serves.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, PublicKey};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderMap;
use tokio_tungstenite::tungstenite::Message;

use super::{AUTH_REQUIRED, PUBKEY_MISMATCH};

/// How the relay answers a write on a connection that skipped AUTH.
pub enum Unauthenticated {
    /// `OK false` with this reason.
    Reason(String),
    /// Close the socket without answering.
    Close,
}

/// What the relay does differently from accepting every write at once.
pub struct Script {
    /// Wait this long before the OK for these kinds.
    pub ack_delay: HashMap<u16, Duration>,
    /// Never answer writes of these kinds (they are still stored).
    pub no_ack: HashSet<u16>,
    /// Accept writes of these kinds, then close the socket.
    pub close_after: HashSet<u16>,
    /// The answer to a write without AUTH.
    pub unauthenticated: Unauthenticated,
    /// Accept a write signed by a key other than the authenticated one.
    pub accept_mismatched: bool,
    /// Serve events of these kinds, stored and live, with altered content
    /// under their original id and signature.
    pub corrupt: HashSet<u16>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            ack_delay: HashMap::new(),
            no_ack: HashSet::new(),
            close_after: HashSet::new(),
            unauthenticated: Unauthenticated::Reason(format!("{AUTH_REQUIRED} not authenticated")),
            accept_mismatched: false,
            corrupt: HashSet::new(),
        }
    }
}

type Outbox = mpsc::UnboundedSender<String>;

impl Script {
    /// `event` as the relay serves it.
    fn served(&self, event: &Event) -> Value {
        let mut value = json!(event);
        if self.corrupt.contains(&event.kind.as_u16()) {
            value["content"] = json!(format!("{} (corrupted)", event.content));
        }
        value
    }
}

struct Shared {
    script: Script,
    /// Every event written to the relay, accepted or not, in order.
    events: Mutex<Vec<Event>>,
    /// Live subscriptions: subscription id and the connection to send to.
    live: Mutex<Vec<(String, Outbox)>>,
    /// The headers of every WebSocket upgrade, in order.
    upgrades: Mutex<Vec<HeaderMap>>,
    /// The `relay` tag of every AUTH event, in order.
    auth_relays: Mutex<Vec<String>>,
}

/// A running scripted relay.
pub struct FakeRelay {
    /// Its `ws://` URL.
    pub url: String,
    shared: Arc<Shared>,
}

impl FakeRelay {
    /// Every event written so far, in order.
    pub fn events(&self) -> Vec<Event> {
        self.shared.events.lock().unwrap().clone()
    }

    /// The headers of every WebSocket upgrade so far, in order.
    pub fn upgrades(&self) -> Vec<HeaderMap> {
        self.shared.upgrades.lock().unwrap().clone()
    }

    /// The `relay` tag of every AUTH event so far, in order.
    pub fn auth_relays(&self) -> Vec<String> {
        self.shared.auth_relays.lock().unwrap().clone()
    }
}

/// Starts a relay that follows `script` on a free local port.
pub async fn start(script: Script) -> FakeRelay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let shared = Arc::new(Shared {
        script,
        events: Mutex::new(Vec::new()),
        live: Mutex::new(Vec::new()),
        upgrades: Mutex::new(Vec::new()),
        auth_relays: Mutex::new(Vec::new()),
    });
    let accept = shared.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(serve(tcp, accept.clone()));
        }
    });
    FakeRelay { url, shared }
}

// The handshake callback's `Result<Response, ErrorResponse>` is fixed by tungstenite.
#[allow(clippy::result_large_err)]
async fn serve(tcp: tokio::net::TcpStream, shared: Arc<Shared>) {
    // A plain HTTP request (the NIP-11 fetch) fails the upgrade; that is fine.
    let upgrade =
        tokio_tungstenite::accept_hdr_async(tcp, |request: &Request, response: Response| {
            let headers = request.headers().clone();
            shared.upgrades.lock().unwrap().push(headers);
            Ok(response)
        });
    let Ok(ws) = upgrade.await else {
        return;
    };
    let (mut sink, mut stream) = ws.split();
    let (out, mut outbox) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = outbox.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let send = |out: &Outbox, value: Value| {
        let _ = out.send(value.to_string());
    };
    send(&out, json!(["AUTH", "challenge"]));

    let mut authed: Option<PublicKey> = None;
    while let Some(Ok(frame)) = stream.next().await {
        let Message::Text(text) = frame else { continue };
        let Ok(Value::Array(msg)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match msg.first().and_then(Value::as_str) {
            Some("AUTH") => {
                let event: Event = serde_json::from_value(msg[1].clone()).unwrap();
                let relay = event.tags.iter().find(|t| t.kind().to_string() == "relay");
                let relay = relay.and_then(|t| t.content()).unwrap_or_default();
                shared.auth_relays.lock().unwrap().push(relay.to_owned());
                authed = Some(event.pubkey);
                send(&out, json!(["OK", event.id.to_hex(), true, ""]));
            }
            Some("REQ") => {
                let sub = msg[1].as_str().unwrap().to_string();
                let filter = &msg[2];
                if let Some(ids) = filter["ids"].as_array() {
                    let stored = shared.events.lock().unwrap().clone();
                    for event in stored {
                        if ids.contains(&json!(event.id.to_hex())) {
                            send(&out, json!(["EVENT", sub, shared.script.served(&event)]));
                        }
                    }
                }
                if filter["limit"] == json!(0) {
                    shared.live.lock().unwrap().push((sub.clone(), out.clone()));
                }
                send(&out, json!(["EOSE", sub]));
            }
            Some("EVENT") => {
                let event: Event = serde_json::from_value(msg[1].clone()).unwrap();
                shared.events.lock().unwrap().push(event.clone());
                let id = event.id.to_hex();
                let Some(pubkey) = authed else {
                    match &shared.script.unauthenticated {
                        Unauthenticated::Reason(reason) => {
                            send(&out, json!(["OK", id, false, reason]))
                        }
                        Unauthenticated::Close => break,
                    }
                    continue;
                };
                if event.pubkey != pubkey && !shared.script.accept_mismatched {
                    send(&out, json!(["OK", id, false, PUBKEY_MISMATCH]));
                    continue;
                }
                let kind = event.kind.as_u16();
                if kind == super::KIND_MESSAGE {
                    let served = shared.script.served(&event);
                    for (sub, to) in shared.live.lock().unwrap().iter() {
                        send(to, json!(["EVENT", sub, served]));
                    }
                }
                if shared.script.no_ack.contains(&kind) {
                    continue;
                }
                if let Some(delay) = shared.script.ack_delay.get(&kind) {
                    tokio::time::sleep(*delay).await;
                }
                send(&out, json!(["OK", id, true, ""]));
                if shared.script.close_after.contains(&kind) {
                    break;
                }
            }
            _ => {}
        }
    }
    drop(out);
    let _ = writer.await;
}
