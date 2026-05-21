use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, RelayUrl, Tag, Url};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, warn};

// ── Timeouts ──────────────────────────────────────────────────────────────────

/// How long to wait for an OK acknowledgement after sending an event.
const SEND_EVENT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for EOSE after sending a REQ.
const SUBSCRIBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for the TCP + WebSocket handshake in `do_connect`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Capacity of the command channel.
const CMD_CHANNEL_CAPACITY: usize = 64;

// ── Public error type ─────────────────────────────────────────────────────────

/// Errors that can occur when communicating with a Sprout relay.
#[derive(Debug, Error)]
pub enum RelayClientError {
    /// A WebSocket transport error occurred.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// Failed to serialize or deserialize JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Failed to build a Nostr event.
    #[error("Nostr event builder error: {0}")]
    EventBuilder(String),

    /// Failed to parse a URL.
    #[error("URL parse error: {0}")]
    Url(String),

    /// A relay response was not received within the allowed time.
    #[error("Timeout waiting for relay message")]
    Timeout,

    /// The WebSocket connection was closed before the operation completed.
    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    /// The relay sent a message that was not expected in the current context.
    #[error("Unexpected relay message: {0}")]
    UnexpectedMessage(String),

    /// The relay rejected the NIP-42 authentication attempt.
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// No `AUTH` challenge was received from the relay within the timeout.
    #[error("No AUTH challenge received from relay")]
    NoAuthChallenge,
}

impl From<nostr::event::builder::Error> for RelayClientError {
    fn from(e: nostr::event::builder::Error) -> Self {
        RelayClientError::EventBuilder(e.to_string())
    }
}

// ── Public relay message type ─────────────────────────────────────────────────

/// A message received from a Nostr relay.
#[derive(Debug, Clone)]
pub enum RelayMessage {
    /// An event matching an active subscription.
    Event {
        /// The subscription ID this event belongs to.
        subscription_id: String,
        /// The Nostr event payload.
        event: Box<Event>,
    },
    /// Acknowledgement of a published event.
    Ok(OkResponse),
    /// End-of-stored-events marker for a subscription.
    Eose {
        /// The subscription ID that has reached end-of-stored-events.
        subscription_id: String,
    },
    /// The relay closed a subscription, usually with an error.
    Closed {
        /// The subscription ID that was closed.
        subscription_id: String,
        /// Human-readable reason for the closure.
        message: String,
    },
    /// A human-readable notice from the relay.
    Notice {
        /// The notice text.
        message: String,
    },
    /// A NIP-42 authentication challenge from the relay.
    Auth {
        /// The challenge string to sign.
        challenge: String,
    },
}

/// The relay's response to a published event (NIP-01 `OK` message).
#[derive(Debug, Clone)]
pub struct OkResponse {
    /// Hex-encoded ID of the event that was acknowledged.
    pub event_id: String,
    /// Whether the relay accepted the event.
    pub accepted: bool,
    /// Human-readable reason string (empty when accepted without comment).
    pub message: String,
}

// ── Internal types ────────────────────────────────────────────────────────────

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Commands sent from `RelayClient` to the background WebSocket task.
#[allow(clippy::large_enum_variant)]
enum RelayCommand {
    SendEvent {
        event: Event,
        reply: oneshot::Sender<Result<OkResponse, RelayClientError>>,
    },
    Subscribe {
        sub_id: String,
        filters: Vec<Filter>,
        reply: oneshot::Sender<Result<Vec<Event>, RelayClientError>>,
    },
    CloseSubscription {
        sub_id: String,
        reply: oneshot::Sender<Result<(), RelayClientError>>,
    },
    Shutdown,
}

/// A subscription waiting for EOSE.
struct PendingSubscription {
    events: Vec<Event>,
    reply: oneshot::Sender<Result<Vec<Event>, RelayClientError>>,
    deadline: tokio::time::Instant,
}

/// State owned exclusively by the background task.
struct BgState {
    /// Active subscriptions: sub_id → filters (for reconnect replay).
    active_subscriptions: HashMap<String, Vec<Filter>>,
    /// Pending OK waiters: event_id → (reply, deadline).
    pending_ok: HashMap<
        String,
        (
            oneshot::Sender<Result<OkResponse, RelayClientError>>,
            tokio::time::Instant,
        ),
    >,
    /// Pending EOSE collectors: sub_id → collector.
    pending_eose: HashMap<String, PendingSubscription>,
}

impl BgState {
    fn new() -> Self {
        Self {
            active_subscriptions: HashMap::new(),
            pending_ok: HashMap::new(),
            pending_eose: HashMap::new(),
        }
    }

    /// Resolve all pending operations with `ConnectionClosed` (called on reconnect).
    fn cancel_pending(&mut self) {
        for (_, (reply, _)) in self.pending_ok.drain() {
            let _ = reply.send(Err(RelayClientError::ConnectionClosed));
        }
        for (_, sub) in self.pending_eose.drain() {
            let _ = sub.reply.send(Err(RelayClientError::ConnectionClosed));
        }
    }

    /// Expire any pending operations whose deadline has passed.
    fn expire_timed_out(&mut self) {
        let now = tokio::time::Instant::now();

        let expired_ok: Vec<String> = self
            .pending_ok
            .iter()
            .filter(|(_, (_, dl))| now >= *dl)
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired_ok {
            if let Some((reply, _)) = self.pending_ok.remove(&k) {
                let _ = reply.send(Err(RelayClientError::Timeout));
            }
        }

        let expired_eose: Vec<String> = self
            .pending_eose
            .iter()
            .filter(|(_, sub)| now >= sub.deadline)
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired_eose {
            if let Some(sub) = self.pending_eose.remove(&k) {
                let _ = sub.reply.send(Err(RelayClientError::Timeout));
            }
        }
    }
}

// ── Background task ───────────────────────────────────────────────────────────

/// Perform a single NIP-42 connection + auth handshake.
/// Returns the authenticated WebSocket stream on success.
async fn do_connect(
    relay_url: &str,
    keys: &Keys,
    api_token: Option<&str>,
    auth_tag: Option<&Tag>,
) -> Result<WsStream, RelayClientError> {
    let parsed = relay_url
        .parse::<url::Url>()
        .map_err(|e| RelayClientError::Url(e.to_string()))?;

    let (mut ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(parsed.as_str()))
        .await
        .map_err(|_| RelayClientError::ConnectionClosed)? // timeout → treat as connection failure
        .map_err(RelayClientError::WebSocket)?;

    debug!("connected to relay at {relay_url}");

    // Wait for AUTH challenge (5s timeout).
    let challenge = wait_for_auth_challenge(&mut ws, Duration::from_secs(5)).await?;

    let auth_event = build_auth_event(&challenge, relay_url, keys, api_token, auth_tag)?;
    let event_id = auth_event.id.to_hex();
    debug!("sending AUTH event {event_id}");
    let auth_msg = serde_json::to_string(&json!(["AUTH", auth_event]))?;
    ws.send(Message::Text(auth_msg.into())).await?;

    let ok = wait_for_ok(&mut ws, &event_id, Duration::from_secs(5)).await?;
    if !ok.accepted {
        return Err(RelayClientError::AuthFailed(ok.message));
    }

    debug!("NIP-42 authentication successful");
    Ok(ws)
}

/// Wait for an AUTH challenge frame, responding to Pings along the way.
async fn wait_for_auth_challenge(
    ws: &mut WsStream,
    timeout_dur: Duration,
) -> Result<String, RelayClientError> {
    let deadline = tokio::time::Instant::now() + timeout_dur;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return Err(RelayClientError::NoAuthChallenge);
        }
        let raw = tokio::time::timeout(remaining, ws.next())
            .await
            .map_err(|_| RelayClientError::NoAuthChallenge)?
            .ok_or(RelayClientError::ConnectionClosed)?
            .map_err(RelayClientError::WebSocket)?;
        match raw {
            Message::Text(text) => {
                if let RelayMessage::Auth { challenge } = parse_relay_message(&text)? {
                    return Ok(challenge);
                }
            }
            Message::Ping(data) => {
                ws.send(Message::Pong(data)).await?;
            }
            Message::Close(_) => return Err(RelayClientError::ConnectionClosed),
            _ => {}
        }
    }
}

/// Wait for an OK frame matching `event_id`, responding to Pings along the way.
async fn wait_for_ok(
    ws: &mut WsStream,
    event_id: &str,
    timeout_dur: Duration,
) -> Result<OkResponse, RelayClientError> {
    let deadline = tokio::time::Instant::now() + timeout_dur;
    loop {
        let remaining = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return Err(RelayClientError::Timeout);
        }
        let raw = tokio::time::timeout(remaining, ws.next())
            .await
            .map_err(|_| RelayClientError::Timeout)?
            .ok_or(RelayClientError::ConnectionClosed)?
            .map_err(RelayClientError::WebSocket)?;
        match raw {
            Message::Text(text) => match parse_relay_message(&text)? {
                RelayMessage::Ok(ok) if ok.event_id == event_id => return Ok(ok),
                _ => {} // discard other messages during handshake
            },
            Message::Ping(data) => {
                ws.send(Message::Pong(data)).await?;
            }
            Message::Close(_) => return Err(RelayClientError::ConnectionClosed),
            _ => {}
        }
    }
}

/// Build a NIP-42 AUTH event for the given challenge.
///
/// If `auth_tag` is provided (NIP-OA owner attestation), it is included in the
/// AUTH event so the relay can use it for membership delegation fallback.
#[allow(clippy::result_large_err)]
fn build_auth_event(
    challenge: &str,
    relay_url: &str,
    keys: &Keys,
    api_token: Option<&str>,
    auth_tag: Option<&Tag>,
) -> Result<Event, RelayClientError> {
    let relay_nostr_url =
        RelayUrl::parse(relay_url).map_err(|e| RelayClientError::Url(e.to_string()))?;
    if let Some(token) = api_token {
        let mut tags = vec![
            Tag::parse(["relay", relay_url])
                .map_err(|e| RelayClientError::EventBuilder(e.to_string()))?,
            Tag::parse(["challenge", challenge])
                .map_err(|e| RelayClientError::EventBuilder(e.to_string()))?,
            Tag::parse(["auth_token", token])
                .map_err(|e| RelayClientError::EventBuilder(e.to_string()))?,
        ];
        if let Some(t) = auth_tag {
            tags.push(t.clone());
        }
        Ok(EventBuilder::new(Kind::Authentication, "")
            .tags(tags)
            .sign_with_keys(keys)?)
    } else if let Some(t) = auth_tag {
        // Cannot use EventBuilder::auth() shortcut — it doesn't accept extra tags.
        let tags = vec![
            Tag::parse(["relay", relay_url])
                .map_err(|e| RelayClientError::EventBuilder(e.to_string()))?,
            Tag::parse(["challenge", challenge])
                .map_err(|e| RelayClientError::EventBuilder(e.to_string()))?,
            t.clone(),
        ];
        Ok(EventBuilder::new(Kind::Authentication, "")
            .tags(tags)
            .sign_with_keys(keys)?)
    } else {
        Ok(EventBuilder::auth(challenge, relay_nostr_url).sign_with_keys(keys)?)
    }
}

/// Send a NIP-42 AUTH response for a mid-session challenge.
///
/// Fire-and-forget: we don't wait for the relay's OK. If the relay rejects
/// the re-auth it will close the connection, which triggers our reconnect logic.
async fn send_auth_response(
    ws: &mut WsStream,
    challenge: &str,
    relay_url: &str,
    keys: &Keys,
    api_token: Option<&str>,
    auth_tag: Option<&Tag>,
) {
    let result: Result<(), RelayClientError> = async {
        let auth_event = build_auth_event(challenge, relay_url, keys, api_token, auth_tag)?;
        let msg = serde_json::to_string(&json!(["AUTH", auth_event]))?;
        ws.send(Message::Text(msg.into())).await?;
        debug!("sent AUTH response for mid-session challenge");
        Ok(())
    }
    .await;
    if let Err(e) = result {
        warn!("failed to respond to mid-session AUTH challenge: {e}");
    }
}

/// Handle a single WebSocket message in the background task.
///
/// Returns `false` if the connection has been lost (Close frame or error).
async fn handle_ws_message(
    msg: Message,
    ws: &mut WsStream,
    state: &mut BgState,
    keys: &Keys,
    relay_url: &str,
    api_token: Option<&str>,
    auth_tag: Option<&Tag>,
) -> bool {
    match msg {
        Message::Text(text) => {
            let relay_msg = match parse_relay_message(&text) {
                Ok(m) => m,
                Err(e) => {
                    warn!("failed to parse relay message: {e}");
                    return true;
                }
            };
            match relay_msg {
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    if let Some(sub) = state.pending_eose.get_mut(&subscription_id) {
                        sub.events.push(*event);
                    } else {
                        debug!("EVENT for unknown/completed subscription {subscription_id}");
                    }
                }
                RelayMessage::Ok(ok) => {
                    if let Some((reply, _)) = state.pending_ok.remove(&ok.event_id) {
                        let _ = reply.send(Ok(ok));
                    } else {
                        debug!("OK for unknown event {}", ok.event_id);
                    }
                }
                RelayMessage::Eose { subscription_id } => {
                    if let Some(sub) = state.pending_eose.remove(&subscription_id) {
                        let _ = sub.reply.send(Ok(sub.events));
                        // One-shot subscription fulfilled — don't replay on reconnect.
                        state.active_subscriptions.remove(&subscription_id);
                    } else {
                        debug!("EOSE for unknown subscription {subscription_id}");
                    }
                }
                RelayMessage::Closed {
                    subscription_id,
                    message,
                } => {
                    warn!("subscription {subscription_id} closed by relay: {message}");
                    state.active_subscriptions.remove(&subscription_id);
                    if let Some(sub) = state.pending_eose.remove(&subscription_id) {
                        let _ = sub.reply.send(Err(RelayClientError::ConnectionClosed));
                    }
                }
                RelayMessage::Notice { message } => {
                    debug!("relay NOTICE: {message}");
                }
                RelayMessage::Auth { challenge } => {
                    debug!("received mid-session AUTH challenge — re-authenticating");
                    send_auth_response(ws, &challenge, relay_url, keys, api_token, auth_tag).await;
                }
            }
            true
        }
        Message::Ping(data) => {
            if let Err(e) = ws.send(Message::Pong(data)).await {
                warn!("failed to send Pong: {e}");
                return false;
            }
            true
        }
        Message::Close(_) => {
            debug!("relay sent Close frame");
            false
        }
        _ => true,
    }
}

/// Reconnect with backoff, cancel pending ops, then replay subscriptions.
///
/// Returns `true` on successful reconnect, `false` if the task should exit
/// (Shutdown received or command channel closed during backoff).
///
/// Processes commands during backoff sleeps so that Shutdown is honoured
/// promptly and new operations fail fast with `ConnectionClosed`.
async fn do_reconnect(
    ws: &mut WsStream,
    state: &mut BgState,
    cmd_rx: &mut mpsc::Receiver<RelayCommand>,
    keys: &Keys,
    relay_url: &str,
    api_token: Option<&str>,
    auth_tag: Option<&Tag>,
) -> bool {
    warn!("relay connection lost — reconnecting…");
    state.cancel_pending();

    let mut delay = Duration::from_secs(1);
    loop {
        match do_connect(relay_url, keys, api_token, auth_tag).await {
            Ok(new_ws) => {
                tracing::info!("reconnected to relay at {relay_url}");
                *ws = new_ws;

                // Replay active subscriptions.
                let subs: Vec<(String, Vec<Filter>)> = state
                    .active_subscriptions
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (sub_id, filters) in subs {
                    let mut msg: Vec<Value> = Vec::with_capacity(2 + filters.len());
                    msg.push(json!("REQ"));
                    msg.push(json!(sub_id));
                    for f in &filters {
                        match serde_json::to_value(f) {
                            Ok(v) => msg.push(v),
                            Err(e) => warn!("failed to serialize filter for {sub_id}: {e}"),
                        }
                    }
                    let text = match serde_json::to_string(&Value::Array(msg)) {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("failed to serialize REQ for {sub_id}: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = ws.send(Message::Text(text.into())).await {
                        warn!("failed to resubscribe to {sub_id}: {e}");
                    }
                }
                return true;
            }
            Err(e) => {
                warn!("reconnect failed: {e}, retrying in {delay:?}");
                // Wait for backoff delay while still processing commands.
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(RelayCommand::Shutdown) | None => {
                                debug!("shutdown during reconnect");
                                state.cancel_pending();
                                return false;
                            }
                            // Fail new operations immediately — we're disconnected.
                            Some(RelayCommand::SendEvent { reply, .. }) => {
                                let _ = reply.send(Err(RelayClientError::ConnectionClosed));
                            }
                            Some(RelayCommand::Subscribe { reply, .. }) => {
                                let _ = reply.send(Err(RelayClientError::ConnectionClosed));
                            }
                            Some(RelayCommand::CloseSubscription { reply, .. }) => {
                                let _ = reply.send(Err(RelayClientError::ConnectionClosed));
                            }
                        }
                    }
                }
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// The main background task loop.
///
/// Owns the WebSocket, responds to Pings, routes relay messages to pending
/// waiters, and handles reconnection transparently.
async fn run_background_task(
    mut ws: WsStream,
    mut cmd_rx: mpsc::Receiver<RelayCommand>,
    keys: Keys,
    relay_url: String,
    api_token: Option<String>,
    auth_tag: Option<Tag>,
) {
    let mut state = BgState::new();
    // Ticker for expiring timed-out pending operations (~1s granularity).
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // ── Incoming WebSocket message ────────────────────────────────────
            raw = ws.next() => {
                let needs_reconnect = match raw {
                    Some(Ok(msg)) => {
                        !handle_ws_message(
                            msg, &mut ws, &mut state, &keys, &relay_url, api_token.as_deref(), auth_tag.as_ref(),
                        ).await
                    }
                    Some(Err(e)) => { warn!("WebSocket error: {e}"); true }
                    None => { debug!("WebSocket stream ended"); true }
                };
                if needs_reconnect
                    && !do_reconnect(&mut ws, &mut state, &mut cmd_rx, &keys, &relay_url, api_token.as_deref(), auth_tag.as_ref()).await
                {
                    return; // Shutdown received during reconnect
                }
            }

            // ── Command from RelayClient ──────────────────────────────────────
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(RelayCommand::SendEvent { event, reply }) => {
                        let event_id = event.id.to_hex();
                        let msg = match serde_json::to_string(&json!(["EVENT", event])) {
                            Ok(t) => t,
                            Err(e) => { let _ = reply.send(Err(e.into())); continue; }
                        };
                        if let Err(e) = ws.send(Message::Text(msg.into())).await {
                            let _ = reply.send(Err(RelayClientError::WebSocket(e)));
                            if !do_reconnect(&mut ws, &mut state, &mut cmd_rx, &keys, &relay_url, api_token.as_deref(), auth_tag.as_ref()).await {
                                return;
                            }
                            continue;
                        }
                        let deadline = tokio::time::Instant::now() + SEND_EVENT_TIMEOUT;
                        state.pending_ok.insert(event_id, (reply, deadline));
                    }

                    Some(RelayCommand::Subscribe { sub_id, filters, reply }) => {
                        let mut msg: Vec<Value> = Vec::with_capacity(2 + filters.len());
                        msg.push(json!("REQ"));
                        msg.push(json!(sub_id));
                        let mut ser_err: Option<serde_json::Error> = None;
                        for f in &filters {
                            match serde_json::to_value(f) {
                                Ok(v) => msg.push(v),
                                Err(e) => { ser_err = Some(e); break; }
                            }
                        }
                        if let Some(e) = ser_err {
                            let _ = reply.send(Err(e.into()));
                            continue;
                        }
                        let text = match serde_json::to_string(&Value::Array(msg)) {
                            Ok(t) => t,
                            Err(e) => { let _ = reply.send(Err(e.into())); continue; }
                        };
                        if let Err(e) = ws.send(Message::Text(text.into())).await {
                            let _ = reply.send(Err(RelayClientError::WebSocket(e)));
                            if !do_reconnect(&mut ws, &mut state, &mut cmd_rx, &keys, &relay_url, api_token.as_deref(), auth_tag.as_ref()).await {
                                return;
                            }
                            continue;
                        }
                        state.active_subscriptions.insert(sub_id.clone(), filters);
                        let deadline = tokio::time::Instant::now() + SUBSCRIBE_TIMEOUT;
                        state.pending_eose.insert(sub_id, PendingSubscription {
                            events: Vec::new(),
                            reply,
                            deadline,
                        });
                    }

                    Some(RelayCommand::CloseSubscription { sub_id, reply }) => {
                        state.active_subscriptions.remove(&sub_id);
                        if let Some(sub) = state.pending_eose.remove(&sub_id) {
                            let _ = sub.reply.send(Err(RelayClientError::ConnectionClosed));
                        }
                        let msg = match serde_json::to_string(&json!(["CLOSE", sub_id])) {
                            Ok(t) => t,
                            Err(e) => { let _ = reply.send(Err(e.into())); continue; }
                        };
                        if let Err(e) = ws.send(Message::Text(msg.into())).await {
                            let _ = reply.send(Err(RelayClientError::WebSocket(e)));
                            if !do_reconnect(&mut ws, &mut state, &mut cmd_rx, &keys, &relay_url, api_token.as_deref(), auth_tag.as_ref()).await {
                                return;
                            }
                            continue;
                        }
                        let _ = reply.send(Ok(()));
                    }

                    Some(RelayCommand::Shutdown) | None => {
                        debug!("background task shutting down");
                        state.cancel_pending();
                        return;
                    }
                }
            }

            // ── Timeout ticker ────────────────────────────────────────────────
            _ = tick.tick() => {
                state.expire_timed_out();
            }
        }
    }
}

// ── Public client ─────────────────────────────────────────────────────────────

/// Shared handle to the background task. When the last `Arc` clone drops,
/// the task is signalled to shut down and then aborted as a safety net.
struct BgTaskHandle {
    cmd_tx: mpsc::Sender<RelayCommand>,
    handle: JoinHandle<()>,
}

impl Drop for BgTaskHandle {
    fn drop(&mut self) {
        let _ = self.cmd_tx.try_send(RelayCommand::Shutdown);
        self.handle.abort();
    }
}

/// Clone-able WebSocket client for the Sprout relay.
///
/// Internally, a background tokio task owns the WebSocket connection. All
/// clones share the same command channel to that task. The background task:
/// - Responds to Ping frames immediately (prevents relay disconnect)
/// - Handles mid-session AUTH challenges automatically
/// - Reconnects with exponential backoff on connection loss
/// - Processes Shutdown commands even during reconnect backoff
/// - Replays active subscriptions after reconnect
///
/// When the last clone is dropped, the background task is automatically
/// shut down via [`BgTaskHandle`]'s `Drop` implementation.
#[derive(Clone)]
pub struct RelayClient {
    /// Shared background task handle — Drop sends Shutdown + abort.
    bg: std::sync::Arc<BgTaskHandle>,
    keys: Keys,
    /// WebSocket URL of the relay (e.g. "ws://localhost:3000").
    relay_url: String,
    /// Shared reqwest client for HTTP calls (media upload only).
    http: reqwest::Client,
    /// Optional NIP-OA auth tag injected into every signed event.
    auth_tag: Option<nostr::Tag>,
}

impl RelayClient {
    /// Connect to the relay and start the background task.
    ///
    /// Performs the initial NIP-42 handshake synchronously so startup failures
    /// are surfaced immediately. After that, reconnection is automatic.
    ///
    /// `auth_tag` is an optional NIP-OA tag that will be injected into every
    /// event signed via [`sign_event`](Self::sign_event).
    pub async fn connect(
        relay_url: &str,
        keys: &Keys,
        api_token: Option<&str>,
        auth_tag: Option<nostr::Tag>,
    ) -> Result<Self, RelayClientError> {
        let ws = do_connect(relay_url, keys, api_token, auth_tag.as_ref()).await?;

        let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAPACITY);

        let bg_keys = keys.clone();
        let bg_relay_url = relay_url.to_string();
        let bg_api_token = api_token.map(|t| t.to_string());
        let bg_auth_tag = auth_tag.clone();

        let handle = tokio::spawn(async move {
            run_background_task(ws, cmd_rx, bg_keys, bg_relay_url, bg_api_token, bg_auth_tag).await;
        });

        Ok(Self {
            bg: std::sync::Arc::new(BgTaskHandle { cmd_tx, handle }),
            keys: keys.clone(),
            relay_url: relay_url.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| RelayClientError::Url(format!("HTTP client build failed: {e}")))?,
            auth_tag,
        })
    }

    /// Sign an event builder, injecting the NIP-OA auth tag if configured.
    ///
    /// This is the canonical signing path in the MCP server. All event creation
    /// should go through this method to ensure consistent auth tag injection.
    ///
    /// **Callers MUST NOT add `auth` tags to the builder before calling this
    /// method.** The only `auth` tag that may appear in the signed event is the
    /// one injected by this method. Any pre-existing `auth` tag — whether
    /// `self.auth_tag` is configured or not — is rejected immediately.
    pub fn sign_event(&self, builder: EventBuilder) -> Result<Event, RelayClientError> {
        let builder = if let Some(ref tag) = self.auth_tag {
            builder.tags([tag.clone()])
        } else {
            builder
        };
        let event = builder
            .sign_with_keys(&self.keys)
            .map_err(RelayClientError::from)?;

        // Enforce: auth tags may only come from self.auth_tag injection.
        // - If auth_tag is Some: exactly 1 auth tag must exist (the one we injected)
        // - If auth_tag is None: zero auth tags must exist (no caller bypass)
        let auth_count = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .count();
        let expected = if self.auth_tag.is_some() { 1 } else { 0 };
        if auth_count != expected {
            return Err(RelayClientError::EventBuilder(format!(
                "event has {auth_count} auth tags — expected {expected}; callers must not add auth tags manually"
            )));
        }

        Ok(event)
    }

    /// Returns the WebSocket URL the client connected to.
    pub fn relay_url(&self) -> &str {
        &self.relay_url
    }

    /// Returns the HTTP base URL for the relay's REST API.
    /// Converts ws:// → http:// and wss:// → https://, strips trailing slash.
    pub fn relay_http_url(&self) -> String {
        relay_ws_to_http(&self.relay_url)
    }

    pub(crate) fn pubkey_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }

    /// Returns a reference to the shared reqwest HTTP client.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    /// Returns a reference to the Nostr signing keys.
    pub fn keys(&self) -> &nostr::Keys {
        &self.keys
    }

    /// Returns the NIP-OA auth tag JSON string for use in HTTP `x-auth-tag` headers.
    ///
    /// Returns `None` if no auth tag is configured (direct-member agents).
    pub fn auth_tag_json(&self) -> Option<String> {
        self.auth_tag
            .as_ref()
            .and_then(|t| serde_json::to_string(t.as_slice()).ok())
    }

    /// Returns the relay's server authority (host or host:port) for BUD-11 server tags.
    ///
    /// Uses the same logic as the desktop client's `extract_server_authority`:
    /// default ports (80/443) are omitted, non-default ports are included.
    /// Returns `None` for localhost (no server tag in dev mode).
    pub fn server_domain(&self) -> Option<String> {
        // Convert ws:// → http://, wss:// → https:// for url::Url parsing.
        let http_url = self
            .relay_url
            .replace("wss://", "https://")
            .replace("ws://", "http://");
        let parsed = url::Url::parse(&http_url).ok()?;
        let host = parsed.host_str()?;
        if host.is_empty() || host == "localhost" {
            return None;
        }
        match parsed.port() {
            Some(port) => Some(format!("{host}:{port}")),
            None => Some(host.to_string()),
        }
    }

    /// One-shot query: send REQ with auto-generated sub_id, collect events until EOSE.
    ///
    /// This is the primary read path for the MCP server. Equivalent to calling
    /// `subscribe()` with a random sub_id.
    pub async fn query(&self, filters: Vec<Filter>) -> Result<Vec<Event>, RelayClientError> {
        let sub_id = format!("q-{}", uuid::Uuid::new_v4().simple());
        self.subscribe(&sub_id, filters).await
    }

    /// Publish a signed Nostr event to the relay and wait for the `OK` acknowledgement.
    ///
    /// Defense-in-depth: validates that the event carries the expected number of
    /// `auth` tags before publishing. This catches any code path that bypasses
    /// [`sign_event`](Self::sign_event).
    pub async fn send_event(&self, event: Event) -> Result<OkResponse, RelayClientError> {
        // Verify the event was authored by this client's keypair.
        if event.pubkey != self.keys.public_key() {
            return Err(RelayClientError::EventBuilder(format!(
                "send_event rejected: event pubkey {} does not match client pubkey {}",
                event.pubkey.to_hex(),
                self.keys.public_key().to_hex()
            )));
        }

        // Defense-in-depth: validate auth tags match configuration exactly.
        let auth_tags: Vec<&nostr::Tag> = event
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"))
            .collect();

        match (&self.auth_tag, auth_tags.as_slice()) {
            // Configured: exactly 1 auth tag that matches our configured tag byte-for-byte
            (Some(expected), [actual]) => {
                if actual.as_slice() != expected.as_slice() {
                    return Err(RelayClientError::EventBuilder(
                        "send_event rejected: auth tag does not match configured attestation"
                            .into(),
                    ));
                }
            }
            // Configured but wrong count
            (Some(_), tags) => {
                return Err(RelayClientError::EventBuilder(format!(
                    "send_event rejected: expected 1 auth tag, found {}",
                    tags.len()
                )));
            }
            // Unconfigured: no auth tags allowed
            (None, tags) if !tags.is_empty() => {
                return Err(RelayClientError::EventBuilder(format!(
                    "send_event rejected: auth tags not allowed when unconfigured, found {}",
                    tags.len()
                )));
            }
            // Unconfigured, no auth tags: OK
            (None, _) => {}
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        self.bg
            .cmd_tx
            .send(RelayCommand::SendEvent {
                event,
                reply: reply_tx,
            })
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?;
        reply_rx
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?
    }

    /// Open a subscription with the given filters and collect all stored events until `EOSE`.
    pub async fn subscribe(
        &self,
        sub_id: &str,
        filters: Vec<Filter>,
    ) -> Result<Vec<Event>, RelayClientError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.bg
            .cmd_tx
            .send(RelayCommand::Subscribe {
                sub_id: sub_id.to_string(),
                filters,
                reply: reply_tx,
            })
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?;
        reply_rx
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?
    }

    /// Send a `CLOSE` message to the relay and remove the subscription from the active set.
    pub async fn close_subscription(&self, sub_id: &str) -> Result<(), RelayClientError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.bg
            .cmd_tx
            .send(RelayCommand::CloseSubscription {
                sub_id: sub_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?;
        reply_rx
            .await
            .map_err(|_| RelayClientError::ConnectionClosed)?
    }

    /// Signal the background task to shut down.
    ///
    /// The task will also be aborted when the last `RelayClient` clone is
    /// dropped, so calling this explicitly is optional but allows a prompt stop.
    pub async fn close(&self) -> Result<(), RelayClientError> {
        let _ = self.bg.cmd_tx.send(RelayCommand::Shutdown).await;
        Ok(())
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Convert a WebSocket URL to its HTTP equivalent.
/// Converts `ws://` → `http://` and `wss://` → `https://`, strips trailing slash.
///
/// Extracted as a free function so it can be unit-tested without a live connection.
pub(crate) fn relay_ws_to_http(url: &str) -> String {
    url.replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Parse a raw relay text frame into a typed [`RelayMessage`].
#[allow(clippy::result_large_err)]
pub fn parse_relay_message(text: &str) -> Result<RelayMessage, RelayClientError> {
    let arr: Vec<Value> = serde_json::from_str(text)?;

    let msg_type = arr
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?;

    match msg_type {
        "EVENT" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let event: Event = serde_json::from_value(
                arr.get(2)
                    .cloned()
                    .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?,
            )?;
            Ok(RelayMessage::Event {
                subscription_id: sub_id,
                event: Box::new(event),
            })
        }
        "OK" => {
            let event_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            let message = arr
                .get(3)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Ok(OkResponse {
                event_id,
                accepted,
                message,
            }))
        }
        "EOSE" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Eose {
                subscription_id: sub_id,
            })
        }
        "CLOSED" => {
            let sub_id = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            let message = arr
                .get(2)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Closed {
                subscription_id: sub_id,
                message,
            })
        }
        "NOTICE" => {
            let message = arr
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(RelayMessage::Notice { message })
        }
        "AUTH" => {
            let challenge = arr
                .get(1)
                .and_then(|v| v.as_str())
                .ok_or_else(|| RelayClientError::UnexpectedMessage(text.to_string()))?
                .to_string();
            Ok(RelayMessage::Auth { challenge })
        }
        other => Err(RelayClientError::UnexpectedMessage(format!(
            "unknown message type: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── relay_ws_to_http ──────────────────────────────────────────────────────

    #[test]
    fn relay_ws_to_http_plain() {
        assert_eq!(
            relay_ws_to_http("ws://localhost:3000"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn relay_ws_to_http_secure() {
        assert_eq!(
            relay_ws_to_http("wss://relay.example.com"),
            "https://relay.example.com"
        );
    }

    #[test]
    fn relay_ws_to_http_strips_trailing_slash() {
        assert_eq!(
            relay_ws_to_http("ws://localhost:3000/"),
            "http://localhost:3000"
        );
    }

    #[test]
    fn relay_ws_to_http_with_path() {
        assert_eq!(
            relay_ws_to_http("wss://relay.example.com/nostr"),
            "https://relay.example.com/nostr"
        );
    }

    // ── parse_relay_message ───────────────────────────────────────────────────

    #[test]
    fn parse_ok_accepted() {
        let text = r#"["OK","abc123",true,""]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Ok(ok) => {
                assert_eq!(ok.event_id, "abc123");
                assert!(ok.accepted);
                assert_eq!(ok.message, "");
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn parse_ok_rejected() {
        let text = r#"["OK","abc123",false,"blocked: spam"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Ok(ok) => {
                assert_eq!(ok.event_id, "abc123");
                assert!(!ok.accepted);
                assert_eq!(ok.message, "blocked: spam");
            }
            _ => panic!("expected Ok"),
        }
    }

    #[test]
    fn parse_eose() {
        let text = r#"["EOSE","sub-1"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Eose { subscription_id } => {
                assert_eq!(subscription_id, "sub-1");
            }
            _ => panic!("expected Eose"),
        }
    }

    #[test]
    fn parse_notice() {
        let text = r#"["NOTICE","hello from relay"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Notice { message } => {
                assert_eq!(message, "hello from relay");
            }
            _ => panic!("expected Notice"),
        }
    }

    #[test]
    fn parse_notice_empty() {
        // NOTICE with no message field — should default to empty string.
        let text = r#"["NOTICE"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Notice { message } => {
                assert_eq!(message, "");
            }
            _ => panic!("expected Notice"),
        }
    }

    #[test]
    fn parse_auth() {
        let text = r#"["AUTH","some-challenge-string"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Auth { challenge } => {
                assert_eq!(challenge, "some-challenge-string");
            }
            _ => panic!("expected Auth"),
        }
    }

    #[test]
    fn parse_closed() {
        let text = r#"["CLOSED","sub-2","error: rate-limited"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, "sub-2");
                assert_eq!(message, "error: rate-limited");
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn parse_closed_no_message() {
        let text = r#"["CLOSED","sub-3"]"#;
        let msg = parse_relay_message(text).unwrap();
        match msg {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id, "sub-3");
                assert_eq!(message, "");
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn parse_unknown_type_returns_error() {
        let text = r#"["UNKNOWN","data"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
        match result.unwrap_err() {
            RelayClientError::UnexpectedMessage(msg) => {
                assert!(msg.contains("unknown message type"));
            }
            e => panic!("expected UnexpectedMessage, got {e:?}"),
        }
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let text = "not json at all";
        let result = parse_relay_message(text);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RelayClientError::Json(_)));
    }

    #[test]
    fn parse_empty_array_returns_error() {
        let text = "[]";
        let result = parse_relay_message(text);
        assert!(result.is_err());
        match result.unwrap_err() {
            RelayClientError::UnexpectedMessage(_) => {}
            e => panic!("expected UnexpectedMessage, got {e:?}"),
        }
    }

    #[test]
    fn parse_auth_missing_challenge_returns_error() {
        let text = r#"["AUTH"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
    }

    #[test]
    fn parse_eose_missing_sub_id_returns_error() {
        let text = r#"["EOSE"]"#;
        let result = parse_relay_message(text);
        assert!(result.is_err());
    }

    // ── sign_event auth tag injection ────────────────────────────────────────

    /// Build a minimal `RelayClient` without a live relay connection.
    ///
    /// Only `keys` and `auth_tag` matter for `sign_event`; the other fields
    /// are inert stubs (the background task immediately exits, the HTTP
    /// client is never used).
    fn make_client(keys: Keys, auth_tag: Option<nostr::Tag>) -> RelayClient {
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let handle = tokio::runtime::Handle::current().spawn(async {});
        RelayClient {
            bg: std::sync::Arc::new(BgTaskHandle { cmd_tx, handle }),
            keys,
            relay_url: "ws://127.0.0.1:1".to_string(),
            http: reqwest::Client::new(),
            auth_tag,
        }
    }

    #[tokio::test]
    async fn test_sign_event_injects_auth_tag() {
        let keys = Keys::generate();
        // Real NIP-OA tag format: ["auth", "<64-char-hex-pubkey>", "<conditions>", "<128-char-hex-sig>"]
        let owner_pubkey = "a".repeat(64);
        let conditions = "";
        let signature = "b".repeat(128);
        let auth_tag = nostr::Tag::parse(["auth", &owner_pubkey, conditions, &signature]).unwrap();

        // With auth_tag: the signed event must contain it.
        let client = make_client(keys.clone(), Some(auth_tag.clone()));
        let event = client
            .sign_event(EventBuilder::new(Kind::TextNote, "hello").tags([]))
            .expect("sign_event should succeed");

        let tag_values: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
            .collect();
        assert!(
            tag_values
                .iter()
                .any(|t| t.first().map(|s| s.as_str()) == Some("auth")
                    && t.get(1).map(|s| s.as_str()) == Some(owner_pubkey.as_str())
                    && t.get(3).map(|s| s.as_str()) == Some(signature.as_str())),
            "expected NIP-OA auth tag in event; got: {tag_values:?}"
        );

        // Without auth_tag: the signed event must NOT contain an auth tag.
        let client_no_auth = make_client(keys, None);
        let event_no_auth = client_no_auth
            .sign_event(EventBuilder::new(Kind::TextNote, "hello").tags([]))
            .expect("sign_event should succeed");

        let has_auth_tag = event_no_auth
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(|s| s.as_str()).unwrap_or("") == "auth");
        assert!(!has_auth_tag, "expected no auth tag when auth_tag is None");
    }

    #[tokio::test]
    async fn test_sign_event_rejects_duplicate_auth_tag() {
        let keys = Keys::generate();
        // Real NIP-OA tag format: ["auth", "<64-char-hex-pubkey>", "<conditions>", "<128-char-hex-sig>"]
        let owner_pubkey = "c".repeat(64);
        let conditions = "";
        let signature = "d".repeat(128);
        let auth_tag = nostr::Tag::parse(["auth", &owner_pubkey, conditions, &signature]).unwrap();

        // Case 1: client has auth_tag configured, caller also pre-adds one → duplicate → reject.
        let client = make_client(keys.clone(), Some(auth_tag.clone()));
        let builder = EventBuilder::new(Kind::TextNote, "oops").tags([auth_tag.clone()]);
        let result = client.sign_event(builder);
        assert!(
            result.is_err(),
            "sign_event should return an error when the event would have duplicate auth tags"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("auth tags"),
            "error message should mention auth tags; got: {err_msg}"
        );

        // Case 2: client has NO auth_tag configured, but caller manually adds one → bypass → reject.
        let client_no_auth = make_client(keys, None);
        let builder_with_manual = EventBuilder::new(Kind::TextNote, "bypass").tags([auth_tag]);
        let result2 = client_no_auth.sign_event(builder_with_manual);
        assert!(
            result2.is_err(),
            "sign_event should reject a manually added auth tag even when auth_tag is None"
        );
        let err_msg2 = result2.unwrap_err().to_string();
        assert!(
            err_msg2.contains("auth tags"),
            "error message should mention auth tags; got: {err_msg2}"
        );
    }

    // ── send_event auth tag validation ───────────────────────────────────────

    #[tokio::test]
    async fn test_send_event_rejects_forged_auth_tag() {
        let keys = Keys::generate();
        let real_tag = nostr::Tag::parse(["auth", &"a".repeat(64), "", &"b".repeat(128)]).unwrap();
        let forged_tag =
            nostr::Tag::parse(["auth", &"c".repeat(64), "", &"d".repeat(128)]).unwrap();

        let client = make_client(keys.clone(), Some(real_tag));

        // Build event with forged auth tag, bypassing sign_event
        let event = EventBuilder::new(Kind::TextNote, "forged")
            .tags([forged_tag])
            .sign_with_keys(&keys)
            .unwrap();

        let result = client.send_event(event).await;
        assert!(result.is_err(), "send_event should reject forged auth tag");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("does not match"),
            "error should mention mismatch: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_send_event_rejects_auth_tag_when_unconfigured() {
        let keys = Keys::generate();
        let sneaky_tag =
            nostr::Tag::parse(["auth", &"a".repeat(64), "", &"b".repeat(128)]).unwrap();

        let client = make_client(keys.clone(), None);

        let event = EventBuilder::new(Kind::TextNote, "sneaky")
            .tags([sneaky_tag])
            .sign_with_keys(&keys)
            .unwrap();

        let result = client.send_event(event).await;
        assert!(
            result.is_err(),
            "send_event should reject auth tags when unconfigured"
        );
    }

    #[tokio::test]
    async fn test_send_event_rejects_wrong_pubkey() {
        let client_keys = Keys::generate();
        let other_keys = Keys::generate();
        let client = make_client(client_keys, None);

        // Event signed by a different keypair
        let event = EventBuilder::new(Kind::TextNote, "wrong author")
            .tags([])
            .sign_with_keys(&other_keys)
            .unwrap();

        let result = client.send_event(event).await;
        assert!(
            result.is_err(),
            "send_event should reject events from wrong pubkey"
        );
    }

    // ── Integration tests: mini relay ─────────────────────────────────────────
    //
    // Each test spins up a lightweight in-process WebSocket server that performs
    // the NIP-42 handshake, then runs a caller-supplied scenario closure.
    // The closure receives the split sink+stream so it can drive the test.

    #[cfg(test)]
    mod integration {
        use super::*;
        use futures_util::stream::{SplitSink, SplitStream};
        use futures_util::{SinkExt, StreamExt};
        use std::future::Future;
        use tokio::net::TcpListener;
        use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

        type PlainWs = WebSocketStream<tokio::net::TcpStream>;

        /// Spawn a mini relay that performs NIP-42 auth handshake then runs `scenario`.
        /// Returns the `ws://127.0.0.1:{port}` URL.
        async fn spawn_mini_relay<F, Fut>(scenario: F) -> String
        where
            F: FnOnce(SplitSink<PlainWs, Message>, SplitStream<PlainWs>) -> Fut + Send + 'static,
            Fut: Future<Output = ()> + Send,
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();

            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let ws = accept_async(stream).await.unwrap();
                let (mut sink, mut stream) = ws.split();

                // Send AUTH challenge.
                sink.send(Message::Text(
                    r#"["AUTH","test-challenge"]"#.to_string().into(),
                ))
                .await
                .unwrap();

                // Wait for AUTH response, send OK for the event.
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
                        if arr.first().and_then(|v| v.as_str()) == Some("AUTH") {
                            let event_id = arr[1]["id"].as_str().unwrap().to_string();
                            sink.send(Message::Text(
                                format!(r#"["OK","{}",true,""]"#, event_id).into(),
                            ))
                            .await
                            .unwrap();
                            break;
                        }
                    }
                }

                scenario(sink, stream).await;
            });

            format!("ws://127.0.0.1:{}", port)
        }

        // ── Test 1: background task responds to Ping when idle ────────────────

        #[tokio::test]
        async fn bg_responds_to_ping_without_caller_activity() {
            let url = spawn_mini_relay(|mut sink, mut stream| async move {
                // Send a Ping — background task should Pong immediately.
                sink.send(Message::Ping(b"abc".to_vec().into()))
                    .await
                    .unwrap();

                // Drain until we see the Pong.
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Pong(data) = msg {
                        assert_eq!(data.as_ref(), b"abc");
                        return;
                    }
                }
                panic!("never received Pong");
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();
            // Give the background task a moment to process the Ping.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = client.close().await;
        }

        // ── Test 2: background task handles mid-session AUTH challenge ────────

        #[tokio::test]
        async fn bg_handles_mid_session_auth_challenge() {
            let url = spawn_mini_relay(|mut sink, mut stream| async move {
                // Send a fresh AUTH challenge after initial handshake.
                sink.send(Message::Text(
                    r#"["AUTH","challenge-2"]"#.to_string().into(),
                ))
                .await
                .unwrap();

                // Expect a new AUTH event with kind 22242.
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
                        if arr.first().and_then(|v| v.as_str()) == Some("AUTH") {
                            let kind = arr[1]["kind"].as_u64().unwrap();
                            assert_eq!(kind, 22242, "expected kind 22242 (Authentication)");
                            // Verify the challenge tag is present.
                            let tags = arr[1]["tags"].as_array().unwrap();
                            let has_challenge = tags.iter().any(|t| {
                                t.as_array().and_then(|a| a.get(1)).and_then(|v| v.as_str())
                                    == Some("challenge-2")
                            });
                            assert!(has_challenge, "AUTH event missing challenge tag");
                            return;
                        }
                    }
                }
                panic!("never received AUTH response");
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = client.close().await;
        }

        // ── Test 3: send_event receives OK response ───────────────────────────

        #[tokio::test]
        async fn send_event_receives_ok_response() {
            let url = spawn_mini_relay(|mut sink, mut stream| async move {
                // Wait for EVENT, send matching OK.
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
                        if arr.first().and_then(|v| v.as_str()) == Some("EVENT") {
                            let event_id = arr[1]["id"].as_str().unwrap().to_string();
                            sink.send(Message::Text(
                                format!(r#"["OK","{}",true,""]"#, event_id).into(),
                            ))
                            .await
                            .unwrap();
                            return;
                        }
                    }
                }
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            let event = EventBuilder::new(Kind::Custom(9), "test")
                .tags([])
                .sign_with_keys(&keys)
                .unwrap();
            let expected_id = event.id.to_hex();

            let ok = client.send_event(event).await.unwrap();
            assert_eq!(ok.event_id, expected_id);
            assert!(ok.accepted);
            assert_eq!(ok.message, "");

            let _ = client.close().await;
        }

        // ── Test 4: subscribe collects events until EOSE ──────────────────────

        #[tokio::test]
        async fn subscribe_collects_events_until_eose() {
            let url = spawn_mini_relay(|mut sink, mut stream| async move {
                // Wait for REQ.
                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        let arr: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
                        if arr.first().and_then(|v| v.as_str()) == Some("REQ") {
                            let sub_id = arr[1].as_str().unwrap().to_string();

                            // Build 3 minimal valid events.
                            let relay_keys = Keys::generate();
                            for i in 0u8..3 {
                                let ev = EventBuilder::new(Kind::TextNote, format!("msg {i}"))
                                    .tags([])
                                    .sign_with_keys(&relay_keys)
                                    .unwrap();
                                let frame = serde_json::to_string(&serde_json::json!([
                                    "EVENT", sub_id, ev
                                ]))
                                .unwrap();
                                sink.send(Message::Text(frame.into())).await.unwrap();
                            }

                            // Send EOSE.
                            sink.send(Message::Text(format!(r#"["EOSE","{}"]"#, sub_id).into()))
                                .await
                                .unwrap();
                            return;
                        }
                    }
                }
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            let events = client
                .subscribe("sub-1", vec![Filter::new()])
                .await
                .unwrap();
            assert_eq!(events.len(), 3, "expected 3 events before EOSE");

            let _ = client.close().await;
        }

        // ── Test 5: send_event times out when relay never sends OK ────────────
        //
        // We connect first (real time), then pause time and advance past the
        // 10s SEND_EVENT_TIMEOUT to avoid a real wait.

        #[tokio::test]
        async fn send_event_times_out_when_no_ok() {
            let url = spawn_mini_relay(|_sink, mut stream| async move {
                // Consume the EVENT frame but never respond with OK.
                // Hold connection open by draining until the client drops.
                while let Some(Ok(_)) = stream.next().await {}
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            let event = EventBuilder::new(Kind::Custom(9), "timeout-test")
                .tags([])
                .sign_with_keys(&keys)
                .unwrap();

            // Pause time AFTER connecting so the auth handshake completes normally.
            tokio::time::pause();

            // Start the send — enqueues EVENT to background task.
            let send_fut = client.send_event(event);
            tokio::pin!(send_fut);

            // Yield to let the background task send the EVENT frame.
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;

            // Advance time past the 10s SEND_EVENT_TIMEOUT + 1s tick granularity.
            tokio::time::advance(Duration::from_secs(12)).await;
            // Let the background task's tick fire and expire the pending_ok entry.
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;

            let result = send_fut.await;
            assert!(
                matches!(result, Err(RelayClientError::Timeout)),
                "expected Timeout, got: {:?}",
                result
            );

            let _ = client.close().await;
        }

        // ── Test 6: close_subscription sends CLOSE message to relay ──────────

        #[tokio::test]
        async fn close_subscription_sends_close_message() {
            let (close_tx, close_rx) = tokio::sync::oneshot::channel::<String>();
            let close_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(close_tx)));

            let url = spawn_mini_relay({
                let close_tx = close_tx.clone();
                move |mut sink, mut stream| async move {
                    // Handle REQ, send EOSE immediately so subscribe() returns.
                    while let Some(Ok(msg)) = stream.next().await {
                        if let Message::Text(text) = msg {
                            let arr: Vec<serde_json::Value> =
                                serde_json::from_str(&text).unwrap_or_default();
                            match arr.first().and_then(|v| v.as_str()) {
                                Some("REQ") => {
                                    let sub_id = arr[1].as_str().unwrap().to_string();
                                    sink.send(Message::Text(
                                        format!(r#"["EOSE","{}"]"#, sub_id).into(),
                                    ))
                                    .await
                                    .unwrap();
                                }
                                Some("CLOSE") => {
                                    let sub_id = arr[1].as_str().unwrap().to_string();
                                    if let Some(tx) = close_tx.lock().await.take() {
                                        let _ = tx.send(sub_id);
                                    }
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            })
            .await;

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            // Subscribe (EOSE comes back immediately).
            client
                .subscribe("sub-close", vec![Filter::new()])
                .await
                .unwrap();

            // Close the subscription — relay should receive CLOSE.
            client.close_subscription("sub-close").await.unwrap();

            let closed_id = tokio::time::timeout(Duration::from_secs(2), close_rx)
                .await
                .expect("timed out waiting for CLOSE")
                .expect("channel dropped");

            assert_eq!(closed_id, "sub-close");

            let _ = client.close().await;
        }

        // ── Test 7: reconnect on transport close ──────────────────────────────
        //
        // Strategy: use a shared TcpListener that accepts two connections.
        // First connection: close immediately after auth.
        // Second connection: full relay that handles send_event.

        #[tokio::test]
        async fn bg_reconnects_on_transport_close() {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let url = format!("ws://127.0.0.1:{}", port);

            // Channel so the test can wait until the second relay is ready.
            let (ok_tx, ok_rx) = tokio::sync::oneshot::channel::<()>();
            let ok_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(ok_tx)));

            tokio::spawn({
                let ok_tx = ok_tx.clone();
                async move {
                    // ── Connection 1: close right after auth ──────────────────
                    {
                        let (stream, _) = listener.accept().await.unwrap();
                        let ws = accept_async(stream).await.unwrap();
                        let (mut sink, mut stream) = ws.split();

                        sink.send(Message::Text(
                            r#"["AUTH","test-challenge"]"#.to_string().into(),
                        ))
                        .await
                        .unwrap();

                        while let Some(Ok(msg)) = stream.next().await {
                            if let Message::Text(text) = msg {
                                let arr: Vec<serde_json::Value> =
                                    serde_json::from_str(&text).unwrap_or_default();
                                if arr.first().and_then(|v| v.as_str()) == Some("AUTH") {
                                    let event_id = arr[1]["id"].as_str().unwrap().to_string();
                                    sink.send(Message::Text(
                                        format!(r#"["OK","{}",true,""]"#, event_id).into(),
                                    ))
                                    .await
                                    .unwrap();
                                    break;
                                }
                            }
                        }
                        // Drop sink+stream — closes the WS connection.
                    }

                    // ── Connection 2: full relay that handles EVENT ────────────
                    {
                        let (stream, _) = listener.accept().await.unwrap();
                        let ws = accept_async(stream).await.unwrap();
                        let (mut sink, mut stream) = ws.split();

                        sink.send(Message::Text(
                            r#"["AUTH","test-challenge"]"#.to_string().into(),
                        ))
                        .await
                        .unwrap();

                        while let Some(Ok(msg)) = stream.next().await {
                            if let Message::Text(text) = msg {
                                let arr: Vec<serde_json::Value> =
                                    serde_json::from_str(&text).unwrap_or_default();
                                match arr.first().and_then(|v| v.as_str()) {
                                    Some("AUTH") => {
                                        let event_id = arr[1]["id"].as_str().unwrap().to_string();
                                        sink.send(Message::Text(
                                            format!(r#"["OK","{}",true,""]"#, event_id).into(),
                                        ))
                                        .await
                                        .unwrap();
                                        // Signal AFTER auth handshake completes.
                                        if let Some(tx) = ok_tx.lock().await.take() {
                                            let _ = tx.send(());
                                        }
                                    }
                                    Some("EVENT") => {
                                        let event_id = arr[1]["id"].as_str().unwrap().to_string();
                                        sink.send(Message::Text(
                                            format!(r#"["OK","{}",true,""]"#, event_id).into(),
                                        ))
                                        .await
                                        .unwrap();
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            });

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            // Wait for the second relay to complete auth (background task reconnected).
            tokio::time::timeout(Duration::from_secs(5), ok_rx)
                .await
                .expect("timed out waiting for reconnect")
                .unwrap();

            // send_event should succeed on the new connection.
            let event = EventBuilder::new(Kind::Custom(9), "after-reconnect")
                .tags([])
                .sign_with_keys(&keys)
                .unwrap();
            let ok = client.send_event(event).await.unwrap();
            assert!(ok.accepted);

            let _ = client.close().await;
        }

        // ── Test 8: shutdown during reconnect ─────────────────────────────

        #[tokio::test]
        async fn shutdown_during_reconnect_exits_promptly() {
            // Connect to a relay that closes immediately after auth,
            // then never accepts again. The background task enters the
            // reconnect loop. Verify that Shutdown is processed during
            // reconnect backoff — the task exits gracefully, NOT via abort.
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let url = format!("ws://127.0.0.1:{}", port);

            tokio::spawn(async move {
                // Accept one connection, auth, then close.
                let (stream, _) = listener.accept().await.unwrap();
                let ws = accept_async(stream).await.unwrap();
                let (mut sink, mut stream) = ws.split();

                sink.send(Message::Text(
                    r#"["AUTH","test-challenge"]"#.to_string().into(),
                ))
                .await
                .unwrap();

                while let Some(Ok(msg)) = stream.next().await {
                    if let Message::Text(text) = msg {
                        let arr: Vec<serde_json::Value> =
                            serde_json::from_str(&text).unwrap_or_default();
                        if arr.first().and_then(|v| v.as_str()) == Some("AUTH") {
                            let event_id = arr[1]["id"].as_str().unwrap().to_string();
                            sink.send(Message::Text(
                                format!(r#"["OK","{}",true,""]"#, event_id).into(),
                            ))
                            .await
                            .unwrap();
                            break;
                        }
                    }
                }
                // Drop — closes connection, triggering reconnect.
                // Don't accept any more connections — reconnect will fail forever.
                drop(listener);
            });

            let keys = Keys::generate();
            let client = RelayClient::connect(&url, &keys, None, None).await.unwrap();

            // Give the background task time to notice the close and enter
            // the reconnect backoff loop.
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Send Shutdown via close() — the reconnect loop processes this
            // during its backoff sleep and exits the task gracefully.
            let _ = client.close().await;

            // Wait for the task to process Shutdown. Do NOT drop the client
            // yet — we want to prove the task exits via Shutdown processing,
            // not via BgTaskHandle::drop calling abort().
            tokio::time::sleep(Duration::from_millis(500)).await;

            // After graceful shutdown, the background task dropped its cmd_rx.
            // Sending another command should fail with a closed-channel error,
            // proving the task exited on its own.
            let result = client.bg.cmd_tx.send(RelayCommand::Shutdown).await;
            assert!(
                result.is_err(),
                "cmd channel should be closed after graceful shutdown —                  task exited via Shutdown, not abort"
            );
        }
    }
}
