//! Upstream relay client — single persistent WebSocket connection to the Sprout relay.
//!
//! Maintains one authenticated connection to the upstream Sprout relay, authenticated
//! via a `proxy:submit` API token. Handles NIP-42 auth automatically and reconnects
//! with exponential backoff on disconnect.

use std::sync::Arc;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use nostr::prelude::*;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, info, warn};

// ── Public types ─────────────────────────────────────────────────────────────

/// Messages forwarded from the upstream relay to the server layer.
#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    /// A relay message to route to a downstream client (raw JSON text).
    RelayMessage(String),
    /// The upstream connection was lost (reconnect in progress).
    Disconnected,
    /// The upstream connection was (re)established and authenticated.
    Connected,
}

// ── UpstreamClient ────────────────────────────────────────────────────────────

/// Inner state shared across the `Arc`.  Kept separate so `Arc<Inner>` can be
/// moved into `'static` spawned tasks without capturing `&self`.
struct Inner {
    relay_url: String,
    api_token: String,
    /// Keypair used to sign NIP-42 auth events.  Generated once at construction
    /// so the auth pubkey is stable across reconnects within a process lifetime.
    auth_keys: Keys,
    /// Whether we're currently connected and authenticated.
    connected: RwLock<bool>,
    /// The EventId of the most recently sent auth event.  Used to correlate
    /// OK responses: only an OK for this specific event ID marks us as authenticated.
    auth_event_id: tokio::sync::Mutex<Option<EventId>>,
    /// Active subscriptions: maps subscription_id → original REQ JSON.
    /// Replayed after reconnect so clients don't silently lose their subscriptions.
    active_subs: DashMap<String, String>,
}

/// Manages a single persistent, authenticated WebSocket connection to the upstream
/// Sprout relay.
///
/// Clone-friendly: all state is behind an `Arc`.
///
/// # Usage
///
/// ```no_run
/// # use sprout_proxy::upstream::UpstreamClient;
/// # use tokio::sync::mpsc;
/// # async fn example() {
/// let (inbound_tx, mut inbound_rx) = mpsc::channel(256);
/// let client = UpstreamClient::new("ws://localhost:3000", "sprout_mytoken123");
///
/// // Spawn the connection loop in the background.
/// let client2 = client.clone();
/// tokio::spawn(async move { client2.run(inbound_tx).await });
/// # }
/// ```
#[derive(Clone)]
pub struct UpstreamClient {
    inner: Arc<Inner>,
    /// Sender for outbound messages (JSON strings) TO the relay.
    /// Cloning the client shares the same outbound channel.
    outbound_tx: mpsc::Sender<String>,
    /// Receiver end — wrapped in `Arc<Mutex>` so the run loop can take it
    /// without needing `&mut self`.
    outbound_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>,
}

impl UpstreamClient {
    /// Create a new [`UpstreamClient`].
    ///
    /// `relay_url` — WebSocket URL of the upstream Sprout relay (e.g. `ws://localhost:3000`).
    /// `api_token` — A `sprout_*` API token with the `proxy:submit` scope.
    pub fn new(relay_url: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self::with_keys(relay_url, api_token, Keys::generate())
    }

    /// Create a new upstream client with explicit auth keys.
    ///
    /// Use this when the API token's `owner_pubkey` must match a specific keypair
    /// (e.g. the proxy's server key). The relay verifies that the NIP-42 auth
    /// event is signed by the token owner.
    pub fn with_keys(
        relay_url: impl Into<String>,
        api_token: impl Into<String>,
        auth_keys: Keys,
    ) -> Self {
        let (outbound_tx, outbound_rx) = mpsc::channel::<String>(128);

        Self {
            inner: Arc::new(Inner {
                relay_url: relay_url.into(),
                api_token: api_token.into(),
                auth_keys,
                connected: RwLock::new(false),
                auth_event_id: tokio::sync::Mutex::new(None),
                active_subs: DashMap::new(),
            }),
            outbound_tx,
            outbound_rx: Arc::new(tokio::sync::Mutex::new(outbound_rx)),
        }
    }

    // ── Send helpers ──────────────────────────────────────────────────────────

    /// Send an event to the upstream relay.
    pub async fn send_event(&self, event: Event) -> Result<(), crate::ProxyError> {
        let msg = ClientMessage::event(event).as_json();
        self.outbound_tx
            .send(msg)
            .await
            .map_err(|_| crate::ProxyError::Upstream("outbound channel closed".into()))
    }

    /// Send a REQ subscription to the upstream relay.
    /// Also stores the subscription so it can be replayed on reconnect.
    pub async fn send_req(
        &self,
        sub_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), crate::ProxyError> {
        let msg = ClientMessage::req(sub_id.clone(), filters).as_json();
        self.inner
            .active_subs
            .insert(sub_id.to_string(), msg.clone());
        self.outbound_tx
            .send(msg)
            .await
            .map_err(|_| crate::ProxyError::Upstream("outbound channel closed".into()))
    }

    /// Send a CLOSE to the upstream relay.
    /// Also removes the subscription from the active set.
    pub async fn send_close(&self, sub_id: SubscriptionId) -> Result<(), crate::ProxyError> {
        self.inner.active_subs.remove(&sub_id.to_string());
        let msg = ClientMessage::close(sub_id).as_json();
        self.outbound_tx
            .send(msg)
            .await
            .map_err(|_| crate::ProxyError::Upstream("outbound channel closed".into()))
    }

    /// Returns `true` if the upstream connection is currently established and authenticated.
    pub fn is_connected(&self) -> bool {
        self.inner.connected.try_read().map(|v| *v).unwrap_or(false)
    }

    // ── Run loop ──────────────────────────────────────────────────────────────

    /// Run the upstream connection loop.  Reconnects on disconnect with exponential
    /// backoff (1 → 2 → 4 → … → 30 seconds).
    ///
    /// This method runs forever; spawn it in a background task.
    pub async fn run(self, inbound_tx: mpsc::Sender<UpstreamEvent>) {
        let mut backoff_secs: u64 = 1;

        loop {
            match self.connect_once(&inbound_tx).await {
                Ok(()) => {
                    info!("upstream connection closed cleanly");
                    backoff_secs = 1;
                }
                Err(e) => {
                    error!("upstream connection error: {e}");
                }
            }

            let _ = inbound_tx.send(UpstreamEvent::Disconnected).await;
            *self.inner.connected.write().await = false;

            let delay = backoff_secs.min(30);
            warn!(delay_secs = delay, "reconnecting to upstream relay");
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            backoff_secs = (backoff_secs * 2).min(30);
        }
    }

    // ── Internal: single connection attempt ──────────────────────────────────

    /// Establish one WebSocket connection, authenticate, and pump messages until
    /// the socket closes or an error occurs.
    async fn connect_once(
        &self,
        inbound_tx: &mpsc::Sender<UpstreamEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(url = %self.inner.relay_url, "connecting to upstream relay");

        let (ws_stream, _response) =
            tokio_tungstenite::connect_async(self.inner.relay_url.as_str()).await?;

        let (mut sink, mut stream) = ws_stream.split();

        // Per-connection channel: the write loop owns the sink and drains this channel.
        let (write_tx, mut write_rx) = mpsc::channel::<String>(128);

        // Spawn the WebSocket write loop.  It owns `sink` and `write_rx`.
        let write_task = tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                debug!("→ upstream: {msg}");
                if let Err(e) = sink.send(WsMessage::Text(msg.into())).await {
                    error!("upstream write error: {e}");
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // Notify used to gate the bridge task until authentication succeeds.
        // This prevents outbound messages from being forwarded before the connection
        // is authenticated.
        let connected_notify = Arc::new(tokio::sync::Notify::new());

        // Bridge task: forward from the shared outbound_rx to the per-connection write_tx.
        // Waits for the connected_notify signal before starting to forward, ensuring
        // no messages are sent before authentication completes.
        let outbound_rx_arc = Arc::clone(&self.outbound_rx);
        let bridge_write_tx = write_tx.clone();
        let bridge_notify = Arc::clone(&connected_notify);
        let bridge_task = tokio::spawn(async move {
            // Wait until authentication succeeds before forwarding any messages.
            bridge_notify.notified().await;
            let mut rx = outbound_rx_arc.lock().await;
            while let Some(msg) = rx.recv().await {
                if bridge_write_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Read loop: relay → inbound_tx.
        let result = read_loop(
            &mut stream,
            &write_tx,
            inbound_tx,
            Arc::clone(&self.inner),
            connected_notify,
        )
        .await;

        // Clean up tasks.
        write_task.abort();
        bridge_task.abort();

        result
    }
}

// ── Free function: read loop ──────────────────────────────────────────────────
//
// Extracted as a free function so it does not capture `&self` — it receives
// only the `Arc<Inner>` it needs, which is `'static`.

async fn read_loop<S>(
    stream: &mut S,
    write_tx: &mpsc::Sender<String>,
    inbound_tx: &mpsc::Sender<UpstreamEvent>,
    inner: Arc<Inner>,
    connected_notify: Arc<tokio::sync::Notify>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut authenticated = false;

    while let Some(frame) = stream.next().await {
        match frame? {
            WsMessage::Text(text) => {
                let text_str = text.as_str();
                debug!("← upstream: {text_str}");

                let relay_msg = match RelayMessage::from_json(text_str) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("failed to parse relay message: {e} — raw: {text_str}");
                        continue;
                    }
                };

                match relay_msg {
                    // ── NIP-42 AUTH challenge ────────────────────────────────
                    RelayMessage::Auth { ref challenge } => {
                        debug!("received AUTH challenge: {challenge}");
                        match respond_to_auth_challenge(challenge, &inner, write_tx).await {
                            Ok(()) => debug!("sent AUTH response"),
                            Err(e) => {
                                error!("failed to send AUTH response: {e}");
                                return Err(e);
                            }
                        }
                    }

                    // ── OK response — check if it's for our auth event ───────
                    RelayMessage::Ok {
                        event_id,
                        ref status,
                        ref message,
                    } if !authenticated => {
                        // Only treat this OK as the auth response if the event_id
                        // matches the auth event we sent. If it's for something else
                        // (e.g. a queued event that got through), forward it downstream.
                        let stored_auth_id = *inner.auth_event_id.lock().await;
                        if stored_auth_id.as_ref() == Some(&event_id) {
                            if *status {
                                info!("upstream relay authenticated successfully");
                                authenticated = true;
                                *inner.connected.write().await = true;

                                // Replay any active subscriptions from before the reconnect.
                                let subs: Vec<String> = inner
                                    .active_subs
                                    .iter()
                                    .map(|entry| entry.value().clone())
                                    .collect();
                                if !subs.is_empty() {
                                    info!(
                                        count = subs.len(),
                                        "replaying active subscriptions after reconnect"
                                    );
                                    for req_json in subs {
                                        if let Err(e) = write_tx.send(req_json).await {
                                            error!("failed to replay subscription: {e}");
                                        }
                                    }
                                }

                                // Signal the bridge task to start forwarding.
                                connected_notify.notify_one();

                                let _ = inbound_tx.send(UpstreamEvent::Connected).await;
                            } else {
                                error!("upstream auth rejected: {message}");
                                return Err(Box::new(crate::ProxyError::Auth(format!(
                                    "upstream auth rejected: {message}"
                                ))));
                            }
                        } else {
                            // OK for a different event — forward downstream as normal.
                            debug!(
                                "received OK for non-auth event {} while not yet authenticated — forwarding",
                                event_id.to_hex()
                            );
                            if inbound_tx
                                .send(UpstreamEvent::RelayMessage(text_str.to_string()))
                                .await
                                .is_err()
                            {
                                debug!("inbound_tx closed — stopping upstream read loop");
                                return Ok(());
                            }
                        }
                    }

                    // ── All other messages → forward downstream ──────────────
                    _other => {
                        if inbound_tx
                            .send(UpstreamEvent::RelayMessage(text_str.to_string()))
                            .await
                            .is_err()
                        {
                            debug!("inbound_tx closed — stopping upstream read loop");
                            return Ok(());
                        }
                    }
                }
            }

            WsMessage::Ping(data) => {
                // tokio-tungstenite automatically responds to pings at the transport
                // layer when using connect_async, so we just log and continue.
                debug!(
                    "received PING ({} bytes) — tungstenite auto-pongs",
                    data.len()
                );
            }

            WsMessage::Close(frame) => {
                info!("upstream relay closed connection: {:?}", frame);
                return Ok(());
            }

            WsMessage::Binary(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => {
                // Ignore binary, pong, and raw frames.
            }
        }
    }

    Ok(())
}

// ── Free function: NIP-42 auth response ──────────────────────────────────────

/// Build and send a NIP-42 kind:22242 auth event in response to a challenge.
/// Stores the auth event's ID in `inner.auth_event_id` so the read loop can
/// correlate the OK response to this specific event.
async fn respond_to_auth_challenge(
    challenge: &str,
    inner: &Inner,
    write_tx: &mpsc::Sender<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let relay_tag = Tag::parse(["relay", &inner.relay_url])
        .map_err(|e| crate::ProxyError::Auth(format!("relay tag: {e}")))?;
    let challenge_tag = Tag::parse(["challenge", challenge])
        .map_err(|e| crate::ProxyError::Auth(format!("challenge tag: {e}")))?;
    let token_tag = Tag::parse(["auth_token", &inner.api_token])
        .map_err(|e| crate::ProxyError::Auth(format!("auth_token tag: {e}")))?;

    let auth_event = EventBuilder::new(
        Kind::Authentication, // kind:22242
        "",
    )
    .tags([relay_tag, challenge_tag, token_tag])
    .sign_with_keys(&inner.auth_keys)
    .map_err(|e| crate::ProxyError::Auth(format!("sign auth event: {e}")))?;

    // Store the auth event ID so the read loop can correlate the OK response.
    *inner.auth_event_id.lock().await = Some(auth_event.id);

    let msg = ClientMessage::auth(auth_event).as_json();
    write_tx.send(msg).await.map_err(|_| {
        Box::new(crate::ProxyError::Upstream(
            "write channel closed during auth".into(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_starts_disconnected() {
        let client = UpstreamClient::new("ws://localhost:3000", "sprout_test");
        assert!(!client.is_connected());
    }

    #[test]
    fn send_methods_queue_correctly_formatted_json() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = UpstreamClient::new("ws://localhost:3000", "sprout_test");

            // Queue an EVENT message.
            let keys = Keys::generate();
            let event = EventBuilder::new(Kind::TextNote, "hello")
                .tags([])
                .sign_with_keys(&keys)
                .unwrap();
            let event_id = event.id;
            client.send_event(event).await.unwrap();

            // Queue a REQ message.
            let sub_id = SubscriptionId::new("test-sub");
            let filters = vec![Filter::new().kind(Kind::TextNote)];
            client.send_req(sub_id.clone(), filters).await.unwrap();

            // Queue a CLOSE message.
            client.send_close(sub_id).await.unwrap();

            // Drain the channel and verify.
            let mut rx = client.outbound_rx.lock().await;
            let event_msg = rx.recv().await.unwrap();
            let req_msg = rx.recv().await.unwrap();
            let close_msg = rx.recv().await.unwrap();

            assert!(
                event_msg.contains("EVENT"),
                "expected EVENT in: {event_msg}"
            );
            assert!(
                event_msg.contains(&event_id.to_hex()),
                "expected event id in: {event_msg}"
            );
            assert!(req_msg.contains("REQ"), "expected REQ in: {req_msg}");
            assert!(
                req_msg.contains("test-sub"),
                "expected sub id in: {req_msg}"
            );
            assert!(
                close_msg.contains("CLOSE"),
                "expected CLOSE in: {close_msg}"
            );
        });
    }

    #[test]
    fn auth_event_has_required_tags() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let inner = Inner {
                relay_url: "ws://localhost:3000".into(),
                api_token: "sprout_mytoken".into(),
                auth_keys: Keys::generate(),
                connected: RwLock::new(false),
                auth_event_id: tokio::sync::Mutex::new(None),
                active_subs: DashMap::new(),
            };
            let (write_tx, mut write_rx) = mpsc::channel::<String>(8);

            respond_to_auth_challenge("test-challenge-abc", &inner, &write_tx)
                .await
                .unwrap();

            let msg = write_rx.recv().await.unwrap();
            // Should be ["AUTH", <event>]
            assert!(msg.contains("AUTH"), "expected AUTH in: {msg}");
            assert!(msg.contains("22242"), "expected kind 22242 in: {msg}");
            assert!(
                msg.contains("test-challenge-abc"),
                "expected challenge in: {msg}"
            );
            assert!(msg.contains("sprout_mytoken"), "expected token in: {msg}");
            assert!(
                msg.contains("ws://localhost:3000"),
                "expected relay url in: {msg}"
            );
        });
    }

    #[test]
    fn auth_event_id_stored_after_challenge_response() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let inner = Inner {
                relay_url: "ws://localhost:3000".into(),
                api_token: "sprout_mytoken".into(),
                auth_keys: Keys::generate(),
                connected: RwLock::new(false),
                auth_event_id: tokio::sync::Mutex::new(None),
                active_subs: DashMap::new(),
            };
            let (write_tx, mut write_rx) = mpsc::channel::<String>(8);

            // Before challenge: no auth_event_id stored.
            assert!(inner.auth_event_id.lock().await.is_none());

            respond_to_auth_challenge("my-challenge", &inner, &write_tx)
                .await
                .unwrap();

            // After challenge: auth_event_id should be set.
            let stored_id = *inner.auth_event_id.lock().await;
            assert!(
                stored_id.is_some(),
                "auth_event_id should be set after challenge"
            );

            // The AUTH message should contain the event ID.
            let msg = write_rx.recv().await.unwrap();
            let stored_hex = stored_id.unwrap().to_hex();
            assert!(
                msg.contains(&stored_hex),
                "AUTH message should contain the event ID {stored_hex}: {msg}"
            );
        });
    }

    #[test]
    fn send_req_tracks_subscription() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = UpstreamClient::new("ws://localhost:3000", "sprout_test");

            assert_eq!(client.inner.active_subs.len(), 0);

            let sub_id = SubscriptionId::new("tracked-sub");
            let filters = vec![Filter::new().kind(Kind::TextNote)];
            client.send_req(sub_id.clone(), filters).await.unwrap();

            assert_eq!(client.inner.active_subs.len(), 1);
            assert!(
                client.inner.active_subs.contains_key("tracked-sub"),
                "subscription should be tracked"
            );

            // CLOSE should remove it.
            client.send_close(sub_id).await.unwrap();
            assert_eq!(client.inner.active_subs.len(), 0);
        });
    }
}
