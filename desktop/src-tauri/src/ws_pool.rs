//! Persistent WebSocket connection pool for serverless mode.
//!
//! **Why this exists:** the original serverless transport opened a *fresh*
//! WebSocket per query and per publish (connect → send → close). A single
//! `get_channels` fires ~10 queries, creating a channel fires 2 publishes,
//! adding an agent fires several more — each a brand-new connection. Public
//! relays (damus, nos.lol, …) aggressively rate-limit connection storms and
//! event bursts ("rate-limited: you are noting too much"), so *every* action
//! started failing.
//!
//! This pool keeps **one long-lived connection per relay URL** and multiplexes
//! all queries and publishes over it. A background reader task per connection
//! dispatches incoming messages to in-flight requests by subscription id
//! (queries) or event id (OK acks). Connections are re-established on demand
//! after a drop. NIP-42 AUTH challenges are answered automatically.
//!
//! **Why hand-rolled here, but `nostr-relay-pool` in the agent (`sprout-acp`):**
//! the standalone agent binary adopts `nostr-relay-pool` for free auto-reconnect
//! / auto-resubscribe / dedup. The desktop deliberately does NOT, to avoid
//! pulling a *second* rustls/TLS stack (`async-wsocket`) into the Tauri process,
//! which already runs aws-lc-rs rustls for reqwest/media/tungstenite — two
//! crypto providers in one process risks a `CryptoProvider` install panic. This
//! pool is serverless-only and already self-heals (drops are detected via the
//! `alive` flag and reconnected on next use), so the agent's missed-message bug
//! does not apply here.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use nostr::Keys;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::relay::SubmitEventResponse;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

/// Max time to wait for a relay's EOSE before returning whatever arrived.
/// Reachable public relays respond in well under this; the cap stops a slow
/// relay from stalling the merge.
const QUERY_TIMEOUT: Duration = Duration::from_secs(6);
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(8);
/// Max time to establish a WebSocket before treating the relay as unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// How long to skip a relay after a connection failure (so we don't re-dial a
/// dead relay on every single query in a burst).
const FAILED_COOLDOWN: Duration = Duration::from_secs(30);

/// A pending query: collects events for one subscription until EOSE.
struct PendingQuery {
    events_tx: mpsc::UnboundedSender<nostr::Event>,
    done_tx: Option<oneshot::Sender<()>>,
}

/// A pending publish: waits for the OK ack for one event id.
struct PendingPublish {
    event_id: String,
    ok_tx: oneshot::Sender<SubmitEventResponse>,
}

/// Shared dispatch state for one connection's reader task.
#[derive(Default)]
struct Dispatch {
    queries: HashMap<String, PendingQuery>,
    publishes: HashMap<String, PendingPublish>,
}

/// A single persistent relay connection.
struct Conn {
    write: Mutex<WsWrite>,
    dispatch: Arc<Mutex<Dispatch>>,
    keys: Keys,
    relay_url: String,
    /// Set false when the reader task exits (connection dropped); the pool then
    /// re-establishes on next use.
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl Conn {
    async fn send(&self, text: String) -> Result<(), String> {
        let mut w = self.write.lock().await;
        w.send(Message::Text(text.into()))
            .await
            .map_err(|e| format!("relay send failed: {e}"))
    }
}

/// One persistent connection per relay URL.
#[derive(Default)]
pub struct RelayPool {
    /// Map of live connections. A std Mutex (not tokio) so `clear()` can run
    /// from the synchronous `apply_workspace` command. We never hold this lock
    /// across an `.await`; a `connect_lock` serializes dials instead.
    conns: std::sync::Mutex<HashMap<String, Arc<Conn>>>,
    /// Serializes connection establishment so concurrent first-use of the same
    /// relay doesn't open multiple sockets (thundering herd).
    connect_lock: Mutex<()>,
    /// Relays that recently failed to connect, with the time they may be
    /// retried. Lets a burst of queries skip a dead relay instead of each
    /// paying the full connect timeout.
    failed: std::sync::Mutex<HashMap<String, std::time::Instant>>,
    /// Live subscriptions → the relays they're open on (for CLOSE on teardown).
    live_subs: std::sync::Mutex<HashMap<String, Vec<String>>>,
}

impl RelayPool {
    pub fn new() -> Self {
        Self::default()
    }

    fn live(&self, relay_url: &str) -> Option<Arc<Conn>> {
        let conns = self.conns.lock().unwrap();
        conns.get(relay_url).and_then(|c| {
            if c.alive.load(std::sync::atomic::Ordering::Relaxed) {
                Some(c.clone())
            } else {
                None
            }
        })
    }

    /// Whether `relay_url` is in the failure cooldown window.
    fn in_cooldown(&self, relay_url: &str) -> bool {
        let mut failed = self.failed.lock().unwrap();
        match failed.get(relay_url) {
            Some(until) if std::time::Instant::now() < *until => true,
            Some(_) => {
                failed.remove(relay_url);
                false
            }
            None => false,
        }
    }

    /// Get (or establish) the persistent connection for `relay_url`.
    async fn get(&self, relay_url: &str, keys: &Keys) -> Result<Arc<Conn>, String> {
        if let Some(c) = self.live(relay_url) {
            return Ok(c);
        }
        // Skip relays that recently failed to connect (avoids paying the connect
        // timeout for every query in a burst to a dead relay).
        if self.in_cooldown(relay_url) {
            return Err(format!("relay {relay_url} in failure cooldown"));
        }
        // Serialize dials to the same relay (the std lock is never held across
        // the await below).
        let _dial = self.connect_lock.lock().await;
        if let Some(c) = self.live(relay_url) {
            return Ok(c);
        }
        match connect(relay_url, keys).await {
            Ok(conn) => {
                self.failed.lock().unwrap().remove(relay_url);
                self.conns
                    .lock()
                    .unwrap()
                    .insert(relay_url.to_string(), conn.clone());
                Ok(conn)
            }
            Err(e) => {
                self.failed.lock().unwrap().insert(
                    relay_url.to_string(),
                    std::time::Instant::now() + FAILED_COOLDOWN,
                );
                Err(e)
            }
        }
    }

    /// Run a REQ over the pooled connection, collecting events until EOSE (or
    /// timeout). Reuses the persistent socket — no per-call connect.
    pub async fn query(
        &self,
        relay_url: &str,
        keys: &Keys,
        filters: &[serde_json::Value],
    ) -> Result<Vec<nostr::Event>, String> {
        let conn = self.get(relay_url, keys).await?;
        let sub_id = format!("q-{}", uuid::Uuid::new_v4());
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = oneshot::channel();

        {
            let mut d = conn.dispatch.lock().await;
            d.queries.insert(
                sub_id.clone(),
                PendingQuery {
                    events_tx,
                    done_tx: Some(done_tx),
                },
            );
        }

        let mut req = vec![
            serde_json::Value::String("REQ".into()),
            serde_json::Value::String(sub_id.clone()),
        ];
        req.extend(filters.iter().cloned());
        let req_json = serde_json::Value::Array(req).to_string();
        if let Err(e) = conn.send(req_json).await {
            conn.dispatch.lock().await.queries.remove(&sub_id);
            return Err(e);
        }

        // Collect events until EOSE (done) or timeout.
        let mut events = Vec::new();
        let _ = tokio::time::timeout(QUERY_TIMEOUT, async {
            // Drain events while waiting for the done signal.
            tokio::pin!(done_rx);
            loop {
                tokio::select! {
                    Some(ev) = events_rx.recv() => events.push(ev),
                    _ = &mut done_rx => {
                        // Drain any straggler events already queued.
                        while let Ok(ev) = events_rx.try_recv() {
                            events.push(ev);
                        }
                        break;
                    }
                }
            }
        })
        .await;

        // Best-effort CLOSE + cleanup.
        let _ = conn
            .send(serde_json::json!(["CLOSE", sub_id]).to_string())
            .await;
        conn.dispatch.lock().await.queries.remove(&sub_id);
        Ok(events)
    }

    /// Open a PERSISTENT live subscription across all given relays. Every
    /// matching EVENT from any relay is forwarded to `sink` (deduped by id).
    /// The REQ stays open until [`RelayPool::unsubscribe`] is called with the
    /// returned sub id. This is how serverless gets standard Nostr realtime:
    /// subscribe to every relay at once and merge, so a message published to
    /// relay B (because relay A rate-limited the write) still streams back.
    pub async fn subscribe(
        &self,
        relay_urls: &[String],
        keys: &Keys,
        filter: serde_json::Value,
        sink: mpsc::UnboundedSender<nostr::Event>,
    ) -> String {
        let sub_id = format!("live-{}", uuid::Uuid::new_v4());
        let req_json = serde_json::Value::Array(vec![
            serde_json::Value::String("REQ".into()),
            serde_json::Value::String(sub_id.clone()),
            filter,
        ])
        .to_string();

        for url in relay_urls {
            // Best-effort: a dead relay just doesn't contribute events.
            let conn = match self.get(url, keys).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            {
                let mut d = conn.dispatch.lock().await;
                d.queries.insert(
                    sub_id.clone(),
                    PendingQuery {
                        events_tx: sink.clone(),
                        done_tx: None, // never completes — live stream
                    },
                );
            }
            let _ = conn.send(req_json.clone()).await;
        }
        // Track which relays this sub is on so we can CLOSE them later.
        self.live_subs
            .lock()
            .unwrap()
            .insert(sub_id.clone(), relay_urls.to_vec());
        sub_id
    }

    /// Close a live subscription on all relays it was opened on.
    pub async fn unsubscribe(&self, sub_id: &str) {
        let urls = self.live_subs.lock().unwrap().remove(sub_id);
        let Some(urls) = urls else { return };
        let close_json = serde_json::json!(["CLOSE", sub_id]).to_string();
        for url in urls {
            if let Some(conn) = self.live(&url) {
                conn.dispatch.lock().await.queries.remove(sub_id);
                let _ = conn.send(close_json.clone()).await;
            }
        }
    }

    /// Publish a signed event over the pooled connection and await OK.
    pub async fn publish(
        &self,
        relay_url: &str,
        keys: &Keys,
        event: &nostr::Event,
    ) -> Result<SubmitEventResponse, String> {
        let conn = self.get(relay_url, keys).await?;
        let event_id = event.id.to_hex();
        let (ok_tx, ok_rx) = oneshot::channel();

        {
            let mut d = conn.dispatch.lock().await;
            d.publishes.insert(
                event_id.clone(),
                PendingPublish {
                    event_id: event_id.clone(),
                    ok_tx,
                },
            );
        }

        let event_json = serde_json::json!(["EVENT", event]).to_string();
        if let Err(e) = conn.send(event_json).await {
            conn.dispatch.lock().await.publishes.remove(&event_id);
            eprintln!("sprout-desktop: [pool] {relay_url} send EVENT failed: {e}");
            return Err(e);
        }

        match tokio::time::timeout(PUBLISH_TIMEOUT, ok_rx).await {
            Ok(Ok(resp)) => {
                eprintln!(
                    "sprout-desktop: [pool] {relay_url} OK accepted={} msg={:?}",
                    resp.accepted, resp.message
                );
                Ok(resp)
            }
            // Channel dropped (reader died) — clean up and report.
            Ok(Err(_)) => {
                conn.dispatch.lock().await.publishes.remove(&event_id);
                Err("relay connection lost during publish".to_string())
            }
            // Many relays are slow/silent on OK — treat timeout as best-effort
            // success so writes don't spuriously fail in the UI.
            Err(_) => {
                conn.dispatch.lock().await.publishes.remove(&event_id);
                Ok(SubmitEventResponse {
                    event_id,
                    accepted: true,
                    message: "published (no OK received before timeout)".to_string(),
                })
            }
        }
    }

    /// Drop all connections (e.g. on workspace switch). Synchronous so it can be
    /// called from `apply_workspace`. Dropping a `Conn` drops its write half and
    /// the reader task's stream, closing the socket.
    pub fn clear(&self) {
        self.conns.lock().unwrap().clear();
        self.failed.lock().unwrap().clear();
        self.live_subs.lock().unwrap().clear();
    }
}

/// Establish a connection and spawn its background reader task.
async fn connect(relay_url: &str, keys: &Keys) -> Result<Arc<Conn>, String> {
    eprintln!("sprout-desktop: [pool] connecting to {relay_url}");
    let (ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(relay_url))
        .await
        .map_err(|_| format!("relay {relay_url} connection timed out"))?
        .map_err(|e| format!("relay connection failed: {e}"))?;
    eprintln!("sprout-desktop: [pool] connected to {relay_url}");
    let (write, read) = ws.split();
    let dispatch = Arc::new(Mutex::new(Dispatch::default()));
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));

    let conn = Arc::new(Conn {
        write: Mutex::new(write),
        dispatch: dispatch.clone(),
        keys: keys.clone(),
        relay_url: relay_url.to_string(),
        alive: alive.clone(),
    });

    // Background reader: dispatch incoming messages to pending requests.
    let reader_conn = conn.clone();
    tokio::spawn(async move {
        run_reader(read, reader_conn, dispatch, alive).await;
    });

    Ok(conn)
}

/// Reader loop: routes EVENT/EOSE/OK/AUTH/CLOSED to the right pending request.
async fn run_reader(
    mut read: WsRead,
    conn: Arc<Conn>,
    dispatch: Arc<Mutex<Dispatch>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
) {
    while let Some(msg) = read.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(arr) = arr.as_array() else { continue };
        let Some(tag) = arr.first().and_then(|v| v.as_str()) else {
            continue;
        };

        match tag {
            "EVENT" => {
                // ["EVENT", <sub>, <event>]
                let sub = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                if let Some(ev) = arr
                    .get(2)
                    .and_then(|v| serde_json::from_value::<nostr::Event>(v.clone()).ok())
                {
                    let d = dispatch.lock().await;
                    if let Some(q) = d.queries.get(sub) {
                        let _ = q.events_tx.send(ev);
                    }
                }
            }
            "EOSE" => {
                let sub = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let mut d = dispatch.lock().await;
                if let Some(q) = d.queries.get_mut(sub) {
                    if let Some(done) = q.done_tx.take() {
                        let _ = done.send(());
                    }
                }
            }
            "CLOSED" => {
                let sub = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let mut d = dispatch.lock().await;
                if let Some(q) = d.queries.get_mut(sub) {
                    if let Some(done) = q.done_tx.take() {
                        let _ = done.send(());
                    }
                }
            }
            "OK" => {
                // ["OK", <event_id>, <accepted>, <message>]
                let id = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                let message = arr
                    .get(3)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut d = dispatch.lock().await;
                if let Some(p) = d.publishes.remove(id) {
                    let _ = p.ok_tx.send(SubmitEventResponse {
                        event_id: p.event_id,
                        accepted,
                        message,
                    });
                }
            }
            "AUTH" => {
                // NIP-42 challenge — sign and answer over the same socket.
                if let Some(challenge) = arr.get(1).and_then(|v| v.as_str()) {
                    if let Ok(auth_json) =
                        crate::ws_relay::build_auth_message(&conn.keys, &conn.relay_url, challenge)
                    {
                        let _ = conn.send(auth_json).await;
                    }
                }
            }
            _ => {}
        }
    }

    // Connection closed: mark dead so the pool reconnects, and fail any
    // in-flight requests so callers don't hang.
    alive.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut d = dispatch.lock().await;
    for (_, q) in d.queries.drain() {
        if let Some(done) = q.done_tx {
            let _ = done.send(());
        }
    }
    d.publishes.clear(); // oneshot senders dropped → callers get RecvError
}
