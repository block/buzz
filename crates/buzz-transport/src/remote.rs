//! Remote bridge transport — a [`Transport`] over an operator-supplied
//! endpoint speaking the protocol in [`crate::protocol`].
//!
//! This is the "bring your own network" implementation: point it at an
//! endpoint you run (a Slack bridge, a private-mesh gateway, a local
//! sidecar, …) and Buzz consumers see the same bidirectional stream of
//! signed events they would get from a relay. The endpoint can be written
//! in any language; see `PROTOCOL.md` for the implementor's guide.
//!
//! The URL scheme selects the carrier:
//!
//! - `wss://` (or loopback/opted-in `ws://`) — one JSON frame per WebSocket
//!   text message.
//! - `unix:///path/to/bridge.sock` — one LF-terminated JSON frame per line
//!   over a Unix domain socket. Local by construction (filesystem
//!   permissions are the access control), and the easiest carrier for a
//!   sidecar bridge or a test harness.
//!
//! WebSocket carriers can additionally be tunneled through a SOCKS5 proxy
//! ([`RemoteTransportConfig::socks_proxy`]) — Tor, an SSH `-D` tunnel, or
//! any private-overlay entry point. Bridge hostnames are resolved at the
//! proxy, so overlay-only names (`.onion`, mesh-internal DNS) work.
//!
//! A background tokio task owns the WebSocket (mirroring the harness relay
//! client): commands flow in over an mpsc channel, inbound events flow out
//! over another, and WebSocket pings are answered even while the consumer is
//! busy.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async_with_config, MaybeTlsStream, WebSocketStream,
};
#[cfg(unix)]
use tokio_util::codec::{Framed, LinesCodec};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::protocol::{
    encode_client_frame, parse_bridge_frame, BridgeFrame, ClientFrame, MAX_FRAME_BYTES,
    PROTOCOL_VERSION,
};
use crate::socks::SocksProxy;
use crate::{BoxFuture, SignedEvent, Subscription, Transport, TransportError, TransportEvent};

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// The connected carrier: JSON frames over a WebSocket or a Unix domain
/// socket (one LF-terminated line per frame).
enum BridgeStream {
    /// `wss://` / `ws://` — one frame per WebSocket text message. Boxed to
    /// keep the enum small next to the slim Unix variant.
    WebSocket(Box<WsStream>),
    /// `unix://` — newline-delimited JSON frames.
    #[cfg(unix)]
    Unix(Framed<tokio::net::UnixStream, LinesCodec>),
}

/// One item from the carrier, with control frames already handled.
enum StreamItem {
    /// A JSON text frame.
    Text(String),
    /// The carrier closed cleanly (or reached EOF).
    Closed,
    /// The carrier failed.
    Failed(String),
}

impl BridgeStream {
    /// Send one JSON text frame.
    async fn send_text(&mut self, text: String) -> Result<(), TransportError> {
        match self {
            Self::WebSocket(ws) => ws
                .send(Message::Text(text.into()))
                .await
                .map_err(|e| TransportError::Connection(e.to_string())),
            #[cfg(unix)]
            Self::Unix(framed) => framed
                .send(text)
                .await
                .map_err(|e| TransportError::Connection(e.to_string())),
        }
    }

    /// Wait for the next text frame, answering WebSocket pings internally.
    async fn next_item(&mut self) -> StreamItem {
        match self {
            Self::WebSocket(ws) => loop {
                match ws.next().await {
                    Some(Ok(Message::Text(text))) => return StreamItem::Text(text.to_string()),
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return StreamItem::Closed,
                    Some(Err(e)) => return StreamItem::Failed(e.to_string()),
                    Some(Ok(_)) => {} // binary/pong frames — protocol is text-only
                }
            },
            #[cfg(unix)]
            Self::Unix(framed) => match framed.next().await {
                Some(Ok(line)) => StreamItem::Text(line),
                None => StreamItem::Closed,
                Some(Err(e)) => StreamItem::Failed(e.to_string()),
            },
        }
    }

    /// Close the carrier, best-effort.
    async fn close(&mut self) {
        match self {
            Self::WebSocket(ws) => {
                let _ = ws.as_mut().close(None).await;
            }
            #[cfg(unix)]
            Self::Unix(framed) => {
                let _ = SinkExt::<String>::close(framed).await;
            }
        }
    }
}

/// Time allowed for the WebSocket upgrade plus `hello`/`hello_ack` exchange.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// Command channel capacity (subscribe/publish/etc.).
const CMD_CHANNEL_CAPACITY: usize = 256;
/// Inbound event channel capacity.
const EVENT_CHANNEL_CAPACITY: usize = 1024;
/// Maximum publishes buffered while disconnected; beyond this the oldest are
/// dropped with visible accounting.
const PENDING_PUBLISH_CAP: usize = 256;
/// Inbound deliveries parked in the background task while the consumer's
/// event channel is full; when this also fills, the carrier read pauses
/// (backpressure) instead of blocking command servicing.
const INBOUND_QUEUE_CAP: usize = 256;
/// Reconnect backoff: start value, doubling to the cap.
const RECONNECT_BACKOFF_START: Duration = Duration::from_secs(1);
/// Reconnect backoff ceiling.
const RECONNECT_BACKOFF_CAP: Duration = Duration::from_secs(30);

/// Configuration for [`RemoteTransport::connect`].
///
/// The `Debug` form redacts `token` and `socks_proxy` — both can carry
/// credentials (`socks5://user:pass@…`) that must not reach logs.
#[derive(Clone)]
pub struct RemoteTransportConfig {
    /// Bridge endpoint URL. `wss://` for a WebSocket bridge (plaintext
    /// `ws://` is accepted only for loopback hosts, `.onion` destinations
    /// via a loopback SOCKS5 proxy, or when `allow_insecure` is set), or
    /// `unix:///path/to/bridge.sock` for a local Unix-domain socket bridge.
    pub url: String,
    /// Hex-encoded public key this client publishes events as; sent in the
    /// `hello` frame so the bridge can scope mention filtering and access.
    pub pubkey: String,
    /// Optional bearer token. Sent in the `hello` frame on every carrier
    /// and, on WebSocket carriers, additionally as
    /// `Authorization: Bearer <token>` on the upgrade request.
    pub token: Option<String>,
    /// Permit plaintext `ws://` to non-loopback hosts. Off by default;
    /// intended for explicitly trusted private networks only.
    pub allow_insecure: bool,
    /// Optional SOCKS5 proxy (`socks5://[user:pass@]host:port`) to tunnel
    /// the bridge connection through — Tor, an SSH `-D` tunnel, or any
    /// private-overlay entry point. The bridge hostname is resolved at the
    /// proxy (`socks5h` behavior).
    pub socks_proxy: Option<String>,
}

impl std::fmt::Debug for RemoteTransportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteTransportConfig")
            .field("url", &self.url)
            .field("pubkey", &self.pubkey)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("allow_insecure", &self.allow_insecure)
            .field(
                "socks_proxy",
                &self.socks_proxy.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Commands sent from [`RemoteTransport`] to the background WebSocket task.
enum RemoteCommand {
    Subscribe(Subscription),
    Unsubscribe(Uuid),
    Publish(Box<SignedEvent>),
    Reconnect,
    Shutdown,
}

/// [`Transport`] implementation over a remote bridge WebSocket.
///
/// The `Debug` form intentionally shows no internals — the socket lives in a
/// background task.
pub struct RemoteTransport {
    /// Receiver for events forwarded by the background task. `None` marks
    /// connection loss.
    event_rx: mpsc::Receiver<Option<TransportEvent>>,
    /// Sender for commands to the background task.
    cmd_tx: mpsc::Sender<RemoteCommand>,
    /// Handle to the background task, taken by `shutdown`.
    bg_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for RemoteTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteTransport").finish_non_exhaustive()
    }
}

impl RemoteTransport {
    /// Dial the bridge, complete the `hello`/`hello_ack` handshake, and
    /// spawn the background socket task.
    pub async fn connect(config: RemoteTransportConfig) -> Result<Self, TransportError> {
        let ws = dial(&config).await?;
        info!(url = %config.url, "connected to remote transport bridge");

        let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAPACITY);
        let bg_handle = tokio::spawn(run_background_task(ws, config, cmd_rx, event_tx));

        Ok(Self {
            event_rx,
            cmd_tx,
            bg_handle: Some(bg_handle),
        })
    }
}

impl Transport for RemoteTransport {
    fn subscribe(
        &mut self,
        subscription: Subscription,
    ) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            self.cmd_tx
                .send(RemoteCommand::Subscribe(subscription))
                .await
                .map_err(|_| TransportError::Closed)
        })
    }

    fn unsubscribe(&mut self, channel_id: Uuid) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            self.cmd_tx
                .send(RemoteCommand::Unsubscribe(channel_id))
                .await
                .map_err(|_| TransportError::Closed)
        })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Option<TransportEvent>> {
        // The background task sends `None` to signal connection loss.
        Box::pin(async move { self.event_rx.recv().await.flatten() })
    }

    fn publish(&self, event: SignedEvent) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            self.cmd_tx
                .send(RemoteCommand::Publish(Box::new(event)))
                .await
                .map_err(|_| TransportError::Closed)
        })
    }

    fn try_publish(&self, event: SignedEvent) -> Result<(), TransportError> {
        match self
            .cmd_tx
            .try_send(RemoteCommand::Publish(Box::new(event)))
        {
            Ok(()) => Ok(()),
            // The trait allows try_publish to drop on a full queue — that is
            // not a closed transport, so don't report it as one.
            Err(mpsc::error::TrySendError::Full(cmd)) => {
                if let RemoteCommand::Publish(event) = cmd {
                    warn!(
                        event_id = %event.id,
                        "command queue full — dropping try_publish event"
                    );
                }
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(TransportError::Closed),
        }
    }

    fn reconnect(&mut self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            self.cmd_tx
                .send(RemoteCommand::Reconnect)
                .await
                .map_err(|_| TransportError::Closed)
        })
    }

    fn shutdown(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let _ = self.cmd_tx.send(RemoteCommand::Shutdown).await;
            if let Some(handle) = self.bg_handle.take() {
                let abort_handle = handle.abort_handle();
                if timeout(Duration::from_secs(5), handle).await.is_err() {
                    warn!("remote transport background task did not finish in 5s — aborting");
                    abort_handle.abort();
                }
            }
        })
    }
}

impl Drop for RemoteTransport {
    fn drop(&mut self) {
        // Best-effort shutdown signal; the task may already be done.
        let _ = self.cmd_tx.try_send(RemoteCommand::Shutdown);
        if let Some(handle) = self.bg_handle.take() {
            handle.abort();
        }
    }
}

/// True for `localhost` and loopback IP hosts.
fn host_is_loopback<S: AsRef<str>>(host: Option<&url::Host<S>>) -> bool {
    match host {
        Some(url::Host::Domain(domain)) => domain.as_ref().eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Reject URL schemes that would carry events in plaintext over a network.
fn validate_url_security(url: &url::Url, allow_insecure: bool) -> Result<(), TransportError> {
    match url.scheme() {
        "wss" => Ok(()),
        "ws" => {
            if allow_insecure || host_is_loopback(url.host().as_ref()) {
                Ok(())
            } else {
                Err(TransportError::Insecure(format!(
                    "plaintext ws:// to non-loopback host {:?} — use wss:// \
                     or opt in explicitly via allow_insecure",
                    url.host_str().unwrap_or_default()
                )))
            }
        }
        other => Err(TransportError::Connection(format!(
            "unsupported URL scheme {other:?} (expected wss:// or ws://)"
        ))),
    }
}

/// Security policy when dialing through a SOCKS5 proxy.
///
/// `wss://` is always fine — TLS runs end-to-end through the tunnel.
/// Plaintext `ws://` requires the proxy itself to be loopback — the
/// client→proxy hop is plain SOCKS5, so a remote proxy would carry the
/// frames unencrypted across the network. With a local proxy, plaintext is
/// accepted for `.onion` destinations (the overlay encrypts beyond the
/// proxy) or loopback destinations (nothing leaves the machine). The
/// explicit opt-in bypasses all of this.
fn validate_proxied_url_security(
    url: &url::Url,
    proxy: &SocksProxy,
    allow_insecure: bool,
) -> Result<(), TransportError> {
    match url.scheme() {
        "wss" => Ok(()),
        "ws" => {
            if allow_insecure {
                return Ok(());
            }
            let onion_destination = matches!(
                url.host(),
                Some(url::Host::Domain(domain))
                    if domain.to_ascii_lowercase().ends_with(".onion")
            );
            let proxy_is_local = host_is_loopback(Some(proxy.host()));
            if proxy_is_local && (onion_destination || host_is_loopback(url.host().as_ref())) {
                Ok(())
            } else {
                Err(TransportError::Insecure(format!(
                    "plaintext ws:// through a SOCKS5 proxy to {:?} — use wss://, \
                     a .onion destination via a loopback proxy, or opt in \
                     explicitly via allow_insecure",
                    url.host_str().unwrap_or_default()
                )))
            }
        }
        other => Err(TransportError::Connection(format!(
            "unsupported URL scheme {other:?} (expected wss:// or ws://)"
        ))),
    }
}

/// Dial the bridge over the carrier selected by the URL scheme and complete
/// the `hello`/`hello_ack` handshake.
async fn dial(config: &RemoteTransportConfig) -> Result<BridgeStream, TransportError> {
    let url = url::Url::parse(&config.url)
        .map_err(|e| TransportError::Connection(format!("invalid bridge URL: {e}")))?;

    let mut stream = timeout(HANDSHAKE_TIMEOUT, connect_carrier(config, &url))
        .await
        .map_err(|_| TransportError::Connection("bridge connect timed out".into()))??;

    match timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, config)).await {
        Ok(Ok(())) => Ok(stream),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(TransportError::Connection(
            "bridge did not send hello_ack within the handshake timeout".into(),
        )),
    }
}

/// Open the carrier for the bridge URL: `unix://` connects a Unix domain
/// socket; `wss://`/`ws://` dial a WebSocket, optionally through the
/// configured SOCKS5 proxy.
async fn connect_carrier(
    config: &RemoteTransportConfig,
    url: &url::Url,
) -> Result<BridgeStream, TransportError> {
    if url.scheme() == "unix" {
        if config.socks_proxy.is_some() {
            return Err(TransportError::Connection(
                "a SOCKS5 proxy cannot carry a unix:// bridge connection".into(),
            ));
        }
        return connect_unix(url).await;
    }

    let proxy = config
        .socks_proxy
        .as_deref()
        .map(SocksProxy::parse)
        .transpose()?;
    match &proxy {
        Some(proxy) => validate_proxied_url_security(url, proxy, config.allow_insecure)?,
        None => validate_url_security(url, config.allow_insecure)?,
    }

    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| TransportError::Connection(format!("invalid bridge URL: {e}")))?;
    if let Some(token) = &config.token {
        let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| {
            TransportError::Connection(format!("bridge token is not a valid header value: {e}"))
        })?;
        request.headers_mut().insert(AUTHORIZATION, value);
    }

    let ws_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_BYTES))
        .max_frame_size(Some(MAX_FRAME_BYTES));
    let ws = match &proxy {
        Some(proxy) => {
            let host = url
                .host()
                .ok_or_else(|| TransportError::Connection("bridge URL has no host".into()))?;
            let port = url
                .port_or_known_default()
                .ok_or_else(|| TransportError::Connection("bridge URL has no port".into()))?;
            let tunnel = proxy.connect(&host, port).await?;
            let (ws, _response) =
                client_async_tls_with_config(request, tunnel, Some(ws_config), None)
                    .await
                    .map_err(|e| TransportError::Connection(e.to_string()))?;
            ws
        }
        None => {
            let (ws, _response) = connect_async_with_config(request, Some(ws_config), false)
                .await
                .map_err(|e| TransportError::Connection(e.to_string()))?;
            ws
        }
    };
    Ok(BridgeStream::WebSocket(Box::new(ws)))
}

/// Connect a `unix:///path/to/bridge.sock` carrier. Local by construction —
/// filesystem permissions are the access control, so no TLS policy applies.
#[cfg(unix)]
async fn connect_unix(url: &url::Url) -> Result<BridgeStream, TransportError> {
    if url.host_str().is_some_and(|h| !h.is_empty()) {
        return Err(TransportError::Connection(
            "unix:// bridge URLs take an absolute path: unix:///path/to/bridge.sock".into(),
        ));
    }
    let path = url.path();
    if path.is_empty() || path == "/" {
        return Err(TransportError::Connection(
            "unix:// bridge URL has no socket path".into(),
        ));
    }
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|e| TransportError::Connection(format!("unix socket connect failed: {e}")))?;
    let codec = LinesCodec::new_with_max_length(MAX_FRAME_BYTES);
    Ok(BridgeStream::Unix(Framed::new(stream, codec)))
}

#[cfg(not(unix))]
async fn connect_unix(_url: &url::Url) -> Result<BridgeStream, TransportError> {
    Err(TransportError::Connection(
        "unix:// bridge sockets are not supported on this platform".into(),
    ))
}

/// `hello` → `hello_ack` over the connected carrier.
async fn handshake(
    stream: &mut BridgeStream,
    config: &RemoteTransportConfig,
) -> Result<(), TransportError> {
    let hello = ClientFrame::Hello {
        version: PROTOCOL_VERSION,
        pubkey: config.pubkey.clone(),
        token: config.token.clone(),
    };
    stream.send_text(encode_client_frame(&hello)?).await?;

    loop {
        match stream.next_item().await {
            StreamItem::Text(text) => match parse_bridge_frame(&text)? {
                Some(BridgeFrame::HelloAck { version }) => {
                    if version != PROTOCOL_VERSION {
                        return Err(TransportError::Protocol(format!(
                            "bridge speaks protocol version {version}, \
                             client speaks {PROTOCOL_VERSION}"
                        )));
                    }
                    return Ok(());
                }
                Some(BridgeFrame::Notice { message }) => {
                    warn!(%message, "bridge notice during handshake");
                }
                Some(other) => {
                    return Err(TransportError::Protocol(format!(
                        "bridge sent {other:?} before hello_ack"
                    )));
                }
                None => debug!("ignoring unknown frame during handshake"),
            },
            StreamItem::Closed => {
                return Err(TransportError::Connection(
                    "bridge closed during handshake".into(),
                ));
            }
            StreamItem::Failed(e) => {
                return Err(TransportError::Connection(format!(
                    "bridge carrier failed during handshake: {e}"
                )));
            }
        }
    }
}

/// Background task: owns the carrier stream, services commands, forwards
/// events.
///
/// Inbound deliveries are parked in a local queue and handed to the
/// consumer's channel via [`mpsc::Sender::reserve`], never an awaited
/// `send` — so a consumer that stops draining events can never wedge the
/// command path (subscribe/publish/shutdown stay serviced, and the carrier
/// read simply pauses once the queue fills).
async fn run_background_task(
    ws: BridgeStream,
    config: RemoteTransportConfig,
    mut cmd_rx: mpsc::Receiver<RemoteCommand>,
    event_tx: mpsc::Sender<Option<TransportEvent>>,
) {
    let mut ws = Some(ws);
    // Active subscriptions, re-sent after every reconnect.
    let mut subscriptions: HashMap<Uuid, Subscription> = HashMap::new();
    // Publishes buffered while disconnected, drained on reconnect.
    let mut pending_publish: VecDeque<SignedEvent> = VecDeque::new();
    // Deliveries (`Some` = event, `None` = disconnect marker) waiting for
    // room in `event_tx`.
    let mut inbound: VecDeque<Option<TransportEvent>> = VecDeque::new();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                None | Some(RemoteCommand::Shutdown) => {
                    if let Some(mut stream) = ws.take() {
                        stream.close().await;
                    }
                    return;
                }
                Some(RemoteCommand::Subscribe(sub)) => {
                    let frame = ClientFrame::subscribe(&sub);
                    subscriptions.insert(sub.channel_id, sub);
                    if let Some(stream) = ws.as_mut() {
                        if send_frame(stream, &frame).await.is_err() {
                            disconnect(&mut ws, &mut inbound);
                        }
                    }
                }
                Some(RemoteCommand::Unsubscribe(channel_id)) => {
                    subscriptions.remove(&channel_id);
                    if let Some(stream) = ws.as_mut() {
                        let frame = ClientFrame::Unsubscribe { channel_id };
                        if send_frame(stream, &frame).await.is_err() {
                            disconnect(&mut ws, &mut inbound);
                        }
                    }
                }
                Some(RemoteCommand::Publish(event)) => match ws.as_mut() {
                    Some(stream) => {
                        let frame = ClientFrame::Event { event };
                        if send_frame(stream, &frame).await.is_err() {
                            // Keep the event for the reconnect replay
                            // instead of losing it with the connection.
                            if let ClientFrame::Event { event } = frame {
                                buffer_publish(&mut pending_publish, *event);
                            }
                            disconnect(&mut ws, &mut inbound);
                        }
                    }
                    None => buffer_publish(&mut pending_publish, *event),
                },
                Some(RemoteCommand::Reconnect) => {
                    if ws.is_some() {
                        // Already connected — treat as a no-op.
                        debug!("reconnect requested while connected — ignoring");
                    } else {
                        match reconnect_loop(
                            &config,
                            &mut cmd_rx,
                            &mut subscriptions,
                            &mut pending_publish,
                        )
                        .await
                        {
                            Some(stream) => ws = Some(stream),
                            None => return, // shutdown requested while reconnecting
                        }
                    }
                }
            },
            // Forward parked deliveries as the consumer makes room.
            permit = event_tx.reserve(), if !inbound.is_empty() => match permit {
                Ok(permit) => {
                    if let Some(delivery) = inbound.pop_front() {
                        permit.send(delivery);
                    }
                }
                Err(_) => return, // consumer dropped the transport
            },
            // Read the carrier only while connected and while the parking
            // queue has room.
            item = next_stream_item(&mut ws),
                if ws.is_some() && inbound.len() < INBOUND_QUEUE_CAP => match item {
                StreamItem::Text(text) => {
                    if let Some(event) = handle_bridge_frame(&text) {
                        inbound.push_back(Some(event));
                    }
                }
                StreamItem::Closed => {
                    info!("bridge connection closed");
                    disconnect(&mut ws, &mut inbound);
                }
                StreamItem::Failed(e) => {
                    warn!("bridge carrier error: {e}");
                    disconnect(&mut ws, &mut inbound);
                }
            },
        }
    }
}

/// Await the next carrier item, or forever when disconnected (the select
/// guard keeps this branch disabled while `ws` is `None`).
async fn next_stream_item(ws: &mut Option<BridgeStream>) -> StreamItem {
    match ws.as_mut() {
        Some(stream) => stream.next_item().await,
        None => std::future::pending().await,
    }
}

/// Drop the carrier and queue the connection-loss marker (`None` on the
/// event channel) exactly once per disconnection.
fn disconnect(ws: &mut Option<BridgeStream>, inbound: &mut VecDeque<Option<TransportEvent>>) {
    if ws.take().is_some() {
        inbound.push_back(None);
    }
}

/// Buffer a publish while disconnected, dropping the oldest beyond the cap.
fn buffer_publish(pending: &mut VecDeque<SignedEvent>, event: SignedEvent) {
    if pending.len() >= PENDING_PUBLISH_CAP {
        if let Some(dropped) = pending.pop_front() {
            warn!(
                event_id = %dropped.id,
                "publish buffer full while disconnected — dropping oldest event"
            );
        }
    }
    pending.push_back(event);
}

/// Encode and send one client frame.
async fn send_frame(stream: &mut BridgeStream, frame: &ClientFrame) -> Result<(), TransportError> {
    let text = encode_client_frame(frame)?;
    stream
        .send_text(text)
        .await
        .inspect_err(|e| warn!("bridge send failed: {e}"))
}

/// Handle one inbound text frame from the bridge, returning the verified
/// delivery when the frame carries an event.
fn handle_bridge_frame(text: &str) -> Option<TransportEvent> {
    match parse_bridge_frame(text) {
        Ok(Some(BridgeFrame::Event { channel_id, event })) => {
            if let Err(e) = event.verify() {
                warn!(event_id = %event.id, "dropping bridge event with invalid signature: {e}");
                return None;
            }
            return Some(TransportEvent {
                channel_id,
                event: *event,
            });
        }
        Ok(Some(BridgeFrame::Ok {
            event_id,
            accepted,
            message,
        })) => {
            if accepted {
                debug!(%event_id, "bridge accepted event");
            } else {
                warn!(%event_id, %message, "bridge rejected event");
            }
        }
        Ok(Some(BridgeFrame::Eose { channel_id })) => {
            debug!(%channel_id, "bridge replay complete");
        }
        Ok(Some(BridgeFrame::Notice { message })) => {
            info!(%message, "bridge notice");
        }
        Ok(Some(BridgeFrame::HelloAck { .. })) => {
            debug!("ignoring duplicate hello_ack");
        }
        Ok(None) => debug!("ignoring unknown bridge frame"),
        Err(e) => warn!("ignoring malformed bridge frame: {e}"),
    }
    None
}

/// Redial with jitter-free exponential backoff until success or shutdown.
///
/// Returns `None` if shutdown was requested while reconnecting. Commands
/// arriving during backoff are folded into the pending state (publishes
/// buffered, subscription set updated) so nothing is lost across the gap.
async fn reconnect_loop(
    config: &RemoteTransportConfig,
    cmd_rx: &mut mpsc::Receiver<RemoteCommand>,
    subscriptions: &mut HashMap<Uuid, Subscription>,
    pending_publish: &mut VecDeque<SignedEvent>,
) -> Option<BridgeStream> {
    let mut backoff = RECONNECT_BACKOFF_START;
    loop {
        match dial(config).await {
            Ok(mut stream) => {
                info!(url = %config.url, "reconnected to remote transport bridge");
                let mut replay_ok = true;
                for sub in subscriptions.values() {
                    if send_frame(&mut stream, &ClientFrame::subscribe(sub))
                        .await
                        .is_err()
                    {
                        replay_ok = false;
                        break;
                    }
                }
                while replay_ok {
                    let Some(event) = pending_publish.pop_front() else {
                        break;
                    };
                    if send_frame(
                        &mut stream,
                        &ClientFrame::Event {
                            event: Box::new(event.clone()),
                        },
                    )
                    .await
                    .is_err()
                    {
                        // Requeue so the next reconnect can retry it.
                        pending_publish.push_front(event);
                        replay_ok = false;
                    }
                }
                if replay_ok {
                    return Some(stream);
                }
                warn!("connection dropped while replaying state — retrying");
            }
            Err(e) => {
                warn!("bridge reconnect failed: {e} — retrying in {backoff:?}");
            }
        }

        // Sleep with backoff, staying responsive to commands.
        let deadline = Instant::now() + backoff;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                _ = sleep(remaining) => break,
                cmd = cmd_rx.recv() => match cmd {
                    None | Some(RemoteCommand::Shutdown) => return None,
                    Some(RemoteCommand::Subscribe(sub)) => {
                        subscriptions.insert(sub.channel_id, sub);
                    }
                    Some(RemoteCommand::Unsubscribe(channel_id)) => {
                        subscriptions.remove(&channel_id);
                    }
                    Some(RemoteCommand::Publish(event)) => {
                        buffer_publish(pending_publish, *event);
                    }
                    Some(RemoteCommand::Reconnect) => {} // already reconnecting
                },
            }
        }
        backoff = (backoff * 2).min(RECONNECT_BACKOFF_CAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(url: &str) -> url::Url {
        url::Url::parse(url).unwrap()
    }

    #[test]
    fn wss_is_always_allowed() {
        assert!(validate_url_security(&parse("wss://bridge.example.com/buzz"), false).is_ok());
    }

    #[test]
    fn plain_ws_to_loopback_is_allowed() {
        assert!(validate_url_security(&parse("ws://localhost:9999/bridge"), false).is_ok());
        assert!(validate_url_security(&parse("ws://127.0.0.1:9999/bridge"), false).is_ok());
        assert!(validate_url_security(&parse("ws://[::1]:9999/bridge"), false).is_ok());
    }

    #[test]
    fn plain_ws_to_remote_host_is_rejected() {
        assert!(matches!(
            validate_url_security(&parse("ws://bridge.example.com/buzz"), false),
            Err(TransportError::Insecure(_))
        ));
        assert!(matches!(
            validate_url_security(&parse("ws://10.0.0.7:9999/bridge"), false),
            Err(TransportError::Insecure(_))
        ));
    }

    #[test]
    fn allow_insecure_opts_in_to_plain_ws() {
        assert!(validate_url_security(&parse("ws://10.0.0.7:9999/bridge"), true).is_ok());
    }

    #[test]
    fn non_websocket_schemes_are_rejected() {
        assert!(matches!(
            validate_url_security(&parse("https://bridge.example.com/buzz"), true),
            Err(TransportError::Connection(_))
        ));
    }

    #[test]
    fn proxied_wss_is_always_allowed() {
        let tor = SocksProxy::parse("socks5://127.0.0.1:9050").unwrap();
        assert!(validate_proxied_url_security(
            &parse("wss://bridge.example.com/buzz"),
            &tor,
            false
        )
        .is_ok());
    }

    #[test]
    fn proxied_plain_ws_is_allowed_for_onion_and_all_loopback_only() {
        let local_proxy = SocksProxy::parse("socks5://127.0.0.1:9050").unwrap();
        let remote_proxy = SocksProxy::parse("socks5://10.0.0.7:1080").unwrap();

        // Tor-style: the overlay provides the encryption.
        assert!(validate_proxied_url_security(
            &parse("ws://bridgeexample.onion/buzz"),
            &local_proxy,
            false
        )
        .is_ok());

        // Everything on-machine: fine.
        assert!(validate_proxied_url_security(
            &parse("ws://127.0.0.1:9999/bridge"),
            &local_proxy,
            false
        )
        .is_ok());

        // Plaintext leaves the machine via the proxy or beyond it: rejected.
        assert!(matches!(
            validate_proxied_url_security(
                &parse("ws://bridge.example.com/buzz"),
                &local_proxy,
                false
            ),
            Err(TransportError::Insecure(_))
        ));
        assert!(matches!(
            validate_proxied_url_security(
                &parse("ws://127.0.0.1:9999/bridge"),
                &remote_proxy,
                false
            ),
            Err(TransportError::Insecure(_))
        ));
        // A remote proxy makes even a .onion destination plaintext on the
        // client→proxy hop — Tor's encryption starts at the proxy.
        assert!(matches!(
            validate_proxied_url_security(
                &parse("ws://bridgeexample.onion/buzz"),
                &remote_proxy,
                false
            ),
            Err(TransportError::Insecure(_))
        ));

        // The explicit opt-in still applies.
        assert!(validate_proxied_url_security(
            &parse("ws://bridge.example.com/buzz"),
            &remote_proxy,
            true
        )
        .is_ok());
    }

    #[test]
    fn publish_buffer_drops_oldest_beyond_cap() {
        let keys = nostr::Keys::generate();
        let mut pending = VecDeque::new();
        for i in 0..(PENDING_PUBLISH_CAP + 3) {
            let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), format!("m{i}"))
                .sign_with_keys(&keys)
                .unwrap();
            buffer_publish(&mut pending, SignedEvent::from_nostr(&event).unwrap());
        }
        assert_eq!(pending.len(), PENDING_PUBLISH_CAP);
        assert_eq!(pending.front().map(|e| e.content.as_str()), Some("m3"));
    }
}
