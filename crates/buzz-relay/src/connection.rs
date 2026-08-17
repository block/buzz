//! WebSocket connection lifecycle: semaphore → challenge → recv/send/heartbeat loops → cleanup.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use dashmap::DashMap;
use futures_util::{Sink, SinkExt, StreamExt};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

use buzz_auth::{generate_challenge, AuthContext, LimitType};
use buzz_core::tenant::TenantContext;
use nostr::Filter;

use crate::handlers;
use crate::protocol::{ClientMessage, RelayMessage};
use crate::state::{
    run_registered_community_connection, AppState, CommunityConnectionControl,
    CommunityDisconnectReason,
};
use buzz_pubsub::EventTopic;

/// Maximum time a new socket may hold a connection slot without completing NIP-42 auth.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared mutable subscription map for a single WebSocket connection.
pub(crate) type ConnectionSubscriptions = Arc<Mutex<HashMap<String, Vec<Filter>>>>;

/// In-flight REQ handlers keyed by client subscription ID.
pub(crate) type PendingReqs = Arc<DashMap<String, (Uuid, CancellationToken)>>;

fn start_pending_req(
    pending_reqs: &PendingReqs,
    sub_id: &str,
    connection_cancel: &CancellationToken,
) -> (Uuid, CancellationToken) {
    let request_id = Uuid::new_v4();
    let request_cancel = connection_cancel.child_token();
    if let Some((_, previous_cancel)) =
        pending_reqs.insert(sub_id.to_owned(), (request_id, request_cancel.clone()))
    {
        previous_cancel.cancel();
    }
    (request_id, request_cancel)
}

fn finish_pending_req(pending_reqs: &PendingReqs, sub_id: &str, request_id: Uuid) {
    pending_reqs.remove_if(sub_id, |_, (active_id, _)| *active_id == request_id);
}

fn cancel_pending_req(pending_reqs: &PendingReqs, sub_id: &str) {
    if let Some((_, (_, request_cancel))) = pending_reqs.remove(sub_id) {
        request_cancel.cancel();
    }
}

/// Request for the writer to flush a restart close and report the result.
pub(crate) struct RestartClose {
    pub(crate) flushed: tokio::sync::oneshot::Sender<bool>,
}

/// Maximum outbound data frames buffered into the websocket sink before one flush.
const MAX_WS_SEND_BATCH: usize = 64;

/// NIP-42 authentication state for a single connection.
#[derive(Debug, Clone)]
pub enum AuthState {
    /// Challenge has been sent; awaiting a signed AUTH event from the client.
    Pending {
        /// The random challenge string sent to the client.
        challenge: String,
    },
    /// Client has successfully authenticated.
    Authenticated(AuthContext),
    /// Authentication attempt was rejected.
    Failed,
}

/// Per-connection state split by access pattern:
/// - `auth_state`: RwLock (read-heavy after initial auth)
/// - `subscriptions`: Mutex (write-heavy during REQ/CLOSE)
/// - `send_tx`, `ctrl_tx`, `cancel`: outside any lock (Clone+Send, no coordination needed)
pub struct ConnectionState {
    /// Unique identifier for this connection.
    pub conn_id: Uuid,
    /// The community this connection is bound to, resolved from the connection
    /// host at row zero (before any frame is read) and never overridable by
    /// client-supplied input. Every handler reads tenant scope from here.
    pub tenant: TenantContext,
    /// Remote socket address of the client.
    pub remote_addr: SocketAddr,
    /// Current NIP-42 authentication state.
    pub auth_state: RwLock<AuthState>,
    /// Active subscriptions keyed by subscription ID.
    pub subscriptions: ConnectionSubscriptions,
    /// Sender for outbound data messages (EVENT, NOTICE, OK, etc.).
    pub send_tx: mpsc::Sender<WsMessage>,
    /// Sender for outbound control frames (Pong, Close).
    /// Separate channel with priority drain — if this channel fills too,
    /// the connection is closed (writer is completely stalled).
    pub ctrl_tx: mpsc::Sender<WsMessage>,
    /// Token used to signal graceful shutdown of this connection's tasks.
    pub cancel: CancellationToken,
    /// Consecutive buffer-full events. Cancel only after `grace_limit`.
    /// Shared with `ConnectionManager::ConnEntry` so both direct sends and
    /// fan-out broadcasts track the same counter.
    pub backpressure_count: Arc<AtomicU8>,
    /// Configurable slow-client grace limit (from `Config::slow_client_grace_limit`).
    pub grace_limit: u8,
}

impl ConnectionState {
    /// Sends a data message to this connection's outbound channel.
    ///
    /// On a full buffer, increments the backpressure counter. The first
    /// `grace_limit` occurrences log a warning; sustained backpressure
    /// cancels the connection to prevent unbounded memory growth.
    pub fn send(&self, msg: String) -> bool {
        match self.send_tx.try_send(WsMessage::Text(msg.into())) {
            Ok(_) => {
                // Successful send resets the grace counter.
                self.backpressure_count.store(0, Ordering::Relaxed);
                true
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                let count = self.backpressure_count.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= self.grace_limit {
                    warn!(conn_id = %self.conn_id, count, "sustained backpressure — closing slow client");
                    metrics::counter!("buzz_ws_backpressure_disconnects_total").increment(1);
                    self.cancel.cancel();
                } else {
                    warn!(conn_id = %self.conn_id, count, grace = self.grace_limit, "send buffer full — grace {count}/{}", self.grace_limit);
                }
                false
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                debug!(conn_id = %self.conn_id, "send channel closed");
                false
            }
        }
    }
}

/// Entry point for a new WebSocket connection.
///
/// Acquires a connection semaphore permit, sends the NIP-42 AUTH challenge,
/// then drives the send, heartbeat, and receive loops until the connection closes.
pub async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    addr: SocketAddr,
    tenant: TenantContext,
) {
    let conn_id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let control = CommunityConnectionControl::new(cancel);
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    run_registered_community_connection(
        &registry,
        conn_id,
        community_id,
        control,
        move || async move { check_state.db.is_community_active(community_id).await },
        move |control| handle_active_connection(socket, run_state, addr, tenant, conn_id, control),
    )
    .await;
}

async fn handle_active_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    addr: SocketAddr,
    tenant: TenantContext,
    conn_id: Uuid,
    control: CommunityConnectionControl,
) {
    let cancel = control.cancellation_token();
    let disconnect_reason = control.disconnect_reason();
    let permit = match state.conn_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!("Connection limit reached, rejecting {addr}");
            return;
        }
    };

    let challenge = generate_challenge();

    let (tx, rx) = mpsc::channel::<WsMessage>(state.config.send_buffer_size);
    // Control channel for Pong/Close — small capacity, guaranteed delivery
    // even when the data buffer is full.
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);

    // Dedicated restart-close channel carries a flush acknowledgement. Keeping
    // ordinary control frames unchanged avoids coupling heartbeat/ban traffic
    // to graceful-shutdown delivery tracking.
    let (restart_tx, restart_rx) = mpsc::channel::<RestartClose>(1);

    let backpressure_count = Arc::new(AtomicU8::new(0));
    let subscriptions = Arc::new(Mutex::new(HashMap::new()));

    let conn = Arc::new(ConnectionState {
        conn_id,
        tenant,
        remote_addr: addr,
        auth_state: RwLock::new(AuthState::Pending {
            challenge: challenge.clone(),
        }),
        subscriptions: Arc::clone(&subscriptions),
        send_tx: tx.clone(),
        ctrl_tx: ctrl_tx.clone(),
        cancel: cancel.clone(),
        backpressure_count: Arc::clone(&backpressure_count),
        grace_limit: state.config.slow_client_grace_limit,
    });

    info!(conn_id = %conn_id, addr = %addr, "WebSocket connection established");
    metrics::counter!(
        "buzz_ws_connections_total",
        "community" => conn.tenant.host().to_owned()
    )
    .increment(1);

    let challenge_msg = RelayMessage::auth_challenge(&challenge);
    if tx
        .send(WsMessage::Text(challenge_msg.into()))
        .await
        .is_err()
    {
        warn!(conn_id = %conn_id, "Failed to send AUTH challenge — client disconnected immediately");
        return;
    }

    // Gauge incremented AFTER challenge send succeeds — early disconnects
    // don't leak. Decremented in the cleanup path below.
    metrics::gauge!("buzz_ws_connections_active").increment(1.0);

    // Register after challenge succeeds — avoids leaked entries on early disconnect.
    state.conn_manager.register(
        conn_id,
        tx.clone(),
        ctrl_tx.clone(),
        Some(restart_tx),
        cancel.clone(),
        conn.tenant.community(),
        Arc::clone(&backpressure_count),
        subscriptions,
        state.config.slow_client_grace_limit,
    );

    let (ws_send, ws_recv) = socket.split();

    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(
        ws_send,
        rx,
        ctrl_rx,
        restart_rx,
        send_cancel,
        disconnect_reason,
    ));

    let missed_pongs = Arc::new(AtomicU8::new(0));
    let heartbeat_cancel = cancel.clone();
    let heartbeat_task = tokio::spawn(heartbeat_loop(
        ctrl_tx,
        Arc::clone(&missed_pongs),
        heartbeat_cancel,
    ));

    let auth_timeout_conn = Arc::clone(&conn);
    let auth_timeout_cancel = cancel.clone();
    let auth_timeout_task = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(AUTH_TIMEOUT) => {
                let authenticated = matches!(
                    *auth_timeout_conn.auth_state.read().await,
                    AuthState::Authenticated(_)
                );
                if !authenticated {
                    warn!(
                        conn_id = %auth_timeout_conn.conn_id,
                        timeout_secs = AUTH_TIMEOUT.as_secs(),
                        "NIP-42 auth timeout — closing connection"
                    );
                    metrics::counter!("buzz_ws_auth_timeouts_total").increment(1);
                    auth_timeout_cancel.cancel();
                }
            }
            _ = auth_timeout_cancel.cancelled() => {}
        }
    });

    recv_loop(
        ws_recv,
        Arc::clone(&conn),
        Arc::clone(&state),
        Arc::clone(&missed_pongs),
        cancel.clone(),
    )
    .await;

    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = auth_timeout_task.await;

    cleanup_connection_subscriptions(&conn, &state).await;
    state.conn_manager.deregister(conn.conn_id);
    if let AuthState::Authenticated(ref auth_ctx) = *conn.auth_state.read().await {
        let remaining = state.conn_manager.connection_ids_for_pubkey_in_community(
            conn.tenant.community(),
            auth_ctx.pubkey.to_bytes().as_slice(),
        );
        if remaining.is_empty() {
            let _ = state
                .pubsub
                .clear_presence(&conn.tenant, &auth_ctx.pubkey)
                .await;
        }
    }
    metrics::gauge!("buzz_ws_connections_active").decrement(1.0);
    info!(conn_id = %conn_id, addr = %addr, "WebSocket connection closed");

    drop(permit);
}

/// Remove all subscription state owned by one connection.
///
/// Taking the connection subscription lock synchronizes teardown with REQ
/// registration: a handler already in its commit section finishes first and is
/// removed here; one entering afterward observes cancellation and does not
/// register.
async fn cleanup_connection_subscriptions(conn: &ConnectionState, state: &AppState) {
    let removed_subscriptions = {
        let mut subscriptions = conn.subscriptions.lock().await;
        subscriptions.clear();
        state.sub_registry.remove_connection(conn.conn_id)
    };
    for removed in removed_subscriptions {
        state
            .pubsub
            .release_topic(&conn.tenant, topic_for_subscription(removed.channel_id))
            .await;
    }
}

/// Outbound send loop with control-frame priority.
///
/// Control frames (Pong, Close) are drained first on every iteration,
/// giving them priority over data frames. If the underlying socket writer
/// is stalled, control frames queue in the small ctrl_rx buffer; callers
/// treat a full control channel as terminal (Bug 7 fix).
async fn send_loop(
    ws_send: futures_util::stream::SplitSink<WebSocket, WsMessage>,
    data_rx: mpsc::Receiver<WsMessage>,
    ctrl_rx: mpsc::Receiver<WsMessage>,
    restart_rx: mpsc::Receiver<RestartClose>,
    cancel: CancellationToken,
    disconnect_reason: watch::Receiver<Option<CommunityDisconnectReason>>,
) {
    send_loop_inner(
        ws_send,
        data_rx,
        ctrl_rx,
        restart_rx,
        cancel,
        disconnect_reason,
    )
    .await;
}

async fn send_loop_inner<S>(
    mut ws_send: S,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    mut restart_rx: mpsc::Receiver<RestartClose>,
    cancel: CancellationToken,
    disconnect_reason: watch::Receiver<Option<CommunityDisconnectReason>>,
) where
    S: Sink<WsMessage> + Unpin,
{
    loop {
        // Priority: drain all pending control frames before data.
        while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
            if ws_send.send(ctrl_msg).await.is_err() {
                return;
            }
        }

        tokio::select! {
            // Biased: restart > cancel > ordinary control > data. A restart
            // command owns shutdown delivery and must flush its 1012 before
            // cancellation can fall back to an unacknowledged close.
            biased;
            Some(restart) = restart_rx.recv() => {
                let sent = ws_send
                    .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::RESTART,
                        reason: axum::extract::ws::Utf8Bytes::from_static("relay restarting"),
                    })))
                    .await
                    .is_ok();
                let _ = restart.flushed.send(sent);
                break;
            }
            _ = cancel.cancelled() => {
                // Drain any queued control frames before closing. A ban
                // disconnect queues its `OK false "blocked: …"` reason frame on
                // ctrl and then cancels; without this drain the biased branch
                // would send Close first and the client would never learn why
                // (the top-of-loop drain does not run again after we break).
                // This makes "queue frame on ctrl, then cancel" a safe idiom.
                while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
                    if ws_send.send(ctrl_msg).await.is_err() {
                        break;
                    }
                }
                let close = disconnect_reason
                    .borrow()
                    .map_or(WsMessage::Close(None), |reason| reason.close_message());
                let _ = ws_send.send(close).await;
                break;
            }
            Some(ctrl_msg) = ctrl_rx.recv() => {
                if ws_send.send(ctrl_msg).await.is_err() {
                    break;
                }
            }
            Some(msg) = data_rx.recv() => {
                let mut batched = 1usize;
                if ws_send.feed(msg).await.is_err() {
                    break;
                }

                while batched < MAX_WS_SEND_BATCH {
                    match data_rx.try_recv() {
                        Ok(next) => {
                            if ws_send.feed(next).await.is_err() {
                                return;
                            }
                            batched += 1;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                if ws_send.flush().await.is_err() {
                    break;
                }
                metrics::histogram!("buzz_ws_send_batch_size").record(batched as f64);
            }
        }
    }
}

/// 3 missed pongs → disconnect.
///
/// Sends Ping through the control channel so it isn't blocked by a full
/// data buffer. Uses `try_send` to keep the select loop responsive to
/// cancellation — a full control channel means the writer is stalled.
async fn heartbeat_loop(
    ctrl_tx: mpsc::Sender<WsMessage>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // fetch_add returns the *previous* value before incrementing:
                //   prev=0 → now 1 (first miss)
                //   prev=1 → now 2 (second miss)
                //   prev=2 → now 3 (third miss → disconnect)
                let missed = missed_pongs.fetch_add(1, Ordering::Relaxed);
                if missed >= 2 {
                    warn!("3 missed pongs — closing connection");
                    cancel.cancel();
                    break;
                }
                if ctrl_tx.try_send(WsMessage::Ping(axum::body::Bytes::new())).is_err() {
                    warn!("control channel full — cannot send Ping, closing");
                    cancel.cancel();
                    break;
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
}

async fn recv_loop(
    mut ws_recv: futures_util::stream::SplitStream<WebSocket>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
    missed_pongs: Arc<AtomicU8>,
    cancel: CancellationToken,
) {
    let pending_reqs = Arc::new(DashMap::new());
    loop {
        tokio::select! {
            msg = ws_recv.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        let max_frame_bytes = state.config.max_frame_bytes;
                        if text.len() > max_frame_bytes {
                            warn!(
                                conn_id = %conn.conn_id,
                                bytes = text.len(),
                                max_frame_bytes,
                                "frame too large — disconnecting"
                            );
                            conn.send(format!(
                                r#"["NOTICE","error: frame too large ({} bytes, limit {})"]"#,
                                text.len(),
                                max_frame_bytes
                            ));
                            break;
                        }
                        trace!(len = text.len(), "frame received");
                        handle_text_message(
                            text.to_string(),
                            Arc::clone(&conn),
                            Arc::clone(&state),
                            Arc::clone(&pending_reqs),
                        ).await;
                    }
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        let max_frame_bytes = state.config.max_frame_bytes;
                        if bytes.len() > max_frame_bytes {
                            warn!(
                                conn_id = %conn.conn_id,
                                bytes = bytes.len(),
                                max_frame_bytes,
                                "binary frame too large — disconnecting"
                            );
                            conn.send(format!(
                                r#"["NOTICE","error: binary frame too large ({} bytes, limit {})"]"#,
                                bytes.len(),
                                max_frame_bytes
                            ));
                            break;
                        }
                        // Binary frames: attempt UTF-8 decode and treat as text. Some clients
                        // (notably certain Nostr libraries) send text payloads in binary frames.
                        // NIP-01 is text-only, but accepting binary is a common relay extension.
                        if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                            handle_text_message(
                                text,
                                Arc::clone(&conn),
                                Arc::clone(&state),
                                Arc::clone(&pending_reqs),
                            ).await;
                        }
                    }
                    Some(Ok(WsMessage::Pong(_))) => {
                        missed_pongs.store(0, Ordering::Relaxed);
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        // Send Pong through the control channel — priority
                        // delivery even when the data buffer is full (Bug 7 fix).
                        if conn.ctrl_tx.try_send(WsMessage::Pong(data)).is_err() {
                            // Control channel full means the socket writer is
                            // completely stalled — treat as terminal.
                            warn!(conn_id = %conn.conn_id, "control channel full — cannot send Pong, closing");
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => {
                        debug!("WebSocket closed by client");
                        break;
                    }
                    Some(Err(e)) => {
                        debug!("WebSocket error: {e}");
                        break;
                    }
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
}

async fn handle_text_message(
    text: String,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
    pending_reqs: PendingReqs,
) {
    let msg = match ClientMessage::parse(&text) {
        Ok(m) => m,
        Err(e) => {
            conn.send(RelayMessage::notice(&format!("invalid message: {e}")));
            return;
        }
    };

    if !enforce_ws_admission(&msg, &conn, &state).await {
        return;
    }

    match msg {
        ClientMessage::Auth(event) => {
            // Auth is synchronous in the WS loop — no span context is lost.
            let span = tracing::info_span!("ws.auth", conn_id = %conn.conn_id);
            handlers::auth::handle_auth(event, Arc::clone(&conn), Arc::clone(&state))
                .instrument(span)
                .await;
        }
        ClientMessage::Event(event) => {
            let conn = Arc::clone(&conn);
            let state = Arc::clone(&state);
            let permit = match state.handler_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    conn.send(RelayMessage::notice(
                        "rate-limited: too many concurrent requests",
                    ));
                    return;
                }
            };
            // Capture the parent span BEFORE the spawn so it is propagated into
            // the spawned future.  A bare `tokio::spawn` drops tracing context.
            let span = tracing::info_span!(
                "ws.event",
                conn_id = %conn.conn_id,
                event_id = tracing::field::Empty,
                kind = tracing::field::Empty,
            );
            tokio::spawn(
                async move {
                    handlers::event::handle_event(event, conn, state).await;
                    drop(permit);
                }
                .instrument(span),
            );
        }
        ClientMessage::Req { sub_id, filters } => {
            let conn = Arc::clone(&conn);
            let state = Arc::clone(&state);
            let permit = match state.handler_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    conn.send(request_rejection_message(
                        Some(&sub_id),
                        "rate-limited: too many concurrent requests",
                    ));
                    return;
                }
            };
            let (request_id, request_cancel) =
                start_pending_req(&pending_reqs, &sub_id, &conn.cancel);
            let span = tracing::info_span!("ws.req", conn_id = %conn.conn_id, sub_id = %sub_id);
            tokio::spawn(
                async move {
                    handlers::req::handle_req(
                        sub_id.clone(),
                        filters,
                        Arc::clone(&conn),
                        state,
                        request_cancel,
                        Arc::clone(&pending_reqs),
                        request_id,
                    )
                    .await;
                    finish_pending_req(&pending_reqs, &sub_id, request_id);
                    drop(permit);
                }
                .instrument(span),
            );
        }
        ClientMessage::Count { sub_id, filters } => {
            let conn = Arc::clone(&conn);
            let state = Arc::clone(&state);
            let permit = match state.handler_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    conn.send(RelayMessage::notice(
                        "rate-limited: too many concurrent requests",
                    ));
                    return;
                }
            };
            let span = tracing::info_span!("ws.count", conn_id = %conn.conn_id, sub_id = %sub_id);
            tokio::spawn(
                async move {
                    handlers::count::handle_count(sub_id, filters, conn, state).await;
                    drop(permit);
                }
                .instrument(span),
            );
        }
        ClientMessage::Close(sub_id) => {
            cancel_pending_req(&pending_reqs, &sub_id);
            handlers::close::handle_close(sub_id, Arc::clone(&conn), Arc::clone(&state)).await;
        }
    }
}

fn request_rejection_message(sub_id: Option<&str>, reason: &str) -> String {
    match sub_id {
        Some(sub_id) => RelayMessage::closed(sub_id, reason),
        None => RelayMessage::notice(reason),
    }
}

async fn enforce_ws_admission(
    msg: &ClientMessage,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    let is_event = matches!(msg, ClientMessage::Event(_));
    if !is_event && !matches!(msg, ClientMessage::Req { .. } | ClientMessage::Count { .. }) {
        return true;
    }

    let (pubkey, is_agent) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Authenticated(ctx) => (ctx.pubkey, ctx.agent_owner_pubkey.is_some()),
            _ => return true,
        }
    };

    let limits = &state.auth.config().rate_limits;
    let (ws_window_secs, ws_limit) =
        crate::admission::ws_admission_budget(limits.human_ws_events_per_sec);
    let ws_result = crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        &conn.tenant,
        &pubkey,
        LimitType::WsEvents,
        ws_window_secs,
        ws_limit,
    )
    .await;
    let sub_id = match msg {
        ClientMessage::Req { sub_id, .. } => Some(sub_id.as_str()),
        _ => None,
    };
    if !send_admission_result(conn, ws_result, sub_id) {
        return false;
    }

    if is_event {
        let message_limit = if is_agent {
            limits.agent_standard_messages_per_min
        } else {
            limits.human_messages_per_min
        };
        let message_result = crate::admission::check_principal(
            state.admission_rate_limiter.as_ref(),
            &conn.tenant,
            &pubkey,
            LimitType::Messages,
            60,
            message_limit,
        )
        .await;
        if !send_admission_result(conn, message_result, None) {
            return false;
        }
    }

    true
}

fn send_admission_result(
    conn: &ConnectionState,
    result: Result<(), crate::admission::AdmissionError>,
    sub_id: Option<&str>,
) -> bool {
    match result {
        Ok(()) => true,
        Err(crate::admission::AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "websocket", "reason" => "quota").increment(1);
            conn.send(request_rejection_message(
                sub_id,
                &format!("rate-limited: quota exceeded; retry in {reset_in_secs}s"),
            ));
            false
        }
        Err(crate::admission::AdmissionError::Unavailable) => {
            metrics::counter!("buzz_admission_rejections_total", "transport" => "websocket", "reason" => "unavailable").increment(1);
            conn.send(request_rejection_message(
                sub_id,
                "rate-limited: shared admission unavailable",
            ));
            false
        }
    }
}

fn topic_for_subscription(channel_id: Option<Uuid>) -> EventTopic {
    match channel_id {
        Some(channel_id) => EventTopic::Channel(channel_id),
        None => EventTopic::Global,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::sync::{oneshot, Notify};

    #[derive(Debug, Default)]
    struct MockSinkState {
        messages: Vec<WsMessage>,
        flush_count: usize,
        fail_after_flushes: Option<usize>,
    }

    #[derive(Debug, Clone)]
    struct MockSink {
        state: Arc<Mutex<MockSinkState>>,
    }

    impl MockSink {
        fn new(fail_after_flushes: Option<usize>) -> (Self, Arc<Mutex<MockSinkState>>) {
            let state = Arc::new(Mutex::new(MockSinkState {
                fail_after_flushes,
                ..MockSinkState::default()
            }));
            (
                Self {
                    state: Arc::clone(&state),
                },
                state,
            )
        }
    }

    impl Sink<WsMessage> for MockSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(self: std::pin::Pin<&mut Self>, item: WsMessage) -> Result<(), Self::Error> {
            self.state
                .lock()
                .expect("mock sink poisoned")
                .messages
                .push(item);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            let mut state = self.state.lock().expect("mock sink poisoned");
            state.flush_count += 1;
            if state
                .fail_after_flushes
                .is_some_and(|limit| state.flush_count >= limit)
            {
                return std::task::Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "mock flush failure",
                )));
            }
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            self.poll_flush(cx)
        }
    }

    fn ordinary_disconnect_reason() -> watch::Receiver<Option<CommunityDisconnectReason>> {
        let (_tx, rx) = watch::channel(None);
        rx
    }

    fn deleted_community_disconnect_reason() -> watch::Receiver<Option<CommunityDisconnectReason>> {
        let (tx, rx) = watch::channel(None);
        tx.send_replace(Some(CommunityDisconnectReason::CommunityDeleted));
        rx
    }

    fn text_payloads(messages: &[WsMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|msg| match msg {
                WsMessage::Text(text) => text.to_string(),
                other => panic!("unexpected websocket message in test: {other:?}"),
            })
            .collect()
    }

    async fn subscription_test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    fn subscription_test_conn_with_receiver() -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
        let (send_tx, send_rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let conn = Arc::new(ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(Uuid::new_v4()),
                "relay.example",
            ),
            remote_addr: "127.0.0.1:1234".parse().expect("socket addr"),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: "test".to_string(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        });
        (conn, send_rx)
    }

    fn subscription_test_conn() -> Arc<ConnectionState> {
        subscription_test_conn_with_receiver().0
    }

    fn spawn_blocked_registration(
        state: Arc<AppState>,
        conn: Arc<ConnectionState>,
        sub_id: &'static str,
        filters: Vec<Filter>,
        request_cancel: CancellationToken,
    ) -> (
        oneshot::Receiver<()>,
        Arc<Notify>,
        tokio::task::JoinHandle<Option<bool>>,
    ) {
        let (started_tx, started_rx) = oneshot::channel();
        let resume = Arc::new(Notify::new());
        let task_resume = Arc::clone(&resume);
        let task = tokio::spawn(async move {
            crate::handlers::req::register_subscription_after_check_for_test(
                sub_id,
                &filters,
                None,
                &conn,
                &state,
                &request_cancel,
                async move {
                    let _ = started_tx.send(());
                    task_resume.notified().await;
                },
            )
            .await
        });
        (started_rx, resume, task)
    }

    async fn assert_subscription_state_empty(state: &AppState, conn: &ConnectionState) {
        assert!(conn.subscriptions.lock().await.is_empty());
        assert_eq!(state.sub_registry.total_subscriptions(), 0);
        assert_eq!(
            state
                .pubsub
                .topic_refcount(&conn.tenant, EventTopic::Global)
                .await,
            0
        );
    }

    #[test]
    fn close_cancels_an_in_flight_req() {
        let pending_reqs = Arc::new(DashMap::new());
        let connection_cancel = CancellationToken::new();
        let (_, request_cancel) = start_pending_req(&pending_reqs, "history", &connection_cancel);

        cancel_pending_req(&pending_reqs, "history");

        assert!(request_cancel.is_cancelled());
        assert!(pending_reqs.is_empty());
    }

    #[test]
    fn replacement_req_cancels_only_the_previous_generation() {
        let pending_reqs = Arc::new(DashMap::new());
        let connection_cancel = CancellationToken::new();
        let (old_id, old_cancel) = start_pending_req(&pending_reqs, "live", &connection_cancel);
        let (new_id, new_cancel) = start_pending_req(&pending_reqs, "live", &connection_cancel);

        assert!(old_cancel.is_cancelled());
        assert!(!new_cancel.is_cancelled());

        finish_pending_req(&pending_reqs, "live", old_id);
        assert!(pending_reqs.contains_key("live"));

        finish_pending_req(&pending_reqs, "live", new_id);
        assert!(pending_reqs.is_empty());
    }

    #[test]
    fn connection_close_cancels_an_in_flight_req() {
        let pending_reqs = Arc::new(DashMap::new());
        let connection_cancel = CancellationToken::new();
        let (_, request_cancel) = start_pending_req(&pending_reqs, "history", &connection_cancel);

        connection_cancel.cancel();

        assert!(request_cancel.is_cancelled());
    }

    #[tokio::test]
    async fn close_prevents_a_blocked_req_from_registering_late() {
        let state = subscription_test_state().await;
        let conn = subscription_test_conn();
        let pending_reqs = Arc::new(DashMap::new());
        let (_, request_cancel) = start_pending_req(&pending_reqs, "history", &conn.cancel);
        let filters = vec![Filter::new().kind(nostr::Kind::TextNote)];
        let (started, resume, task) = spawn_blocked_registration(
            Arc::clone(&state),
            Arc::clone(&conn),
            "history",
            filters,
            request_cancel,
        );
        started.await.expect("blocked REQ started");

        cancel_pending_req(&pending_reqs, "history");
        let mut close = Box::pin(crate::handlers::close::handle_close(
            "history".to_string(),
            Arc::clone(&conn),
            Arc::clone(&state),
        ));
        assert!(futures_util::poll!(&mut close).is_pending());
        resume.notify_one();

        assert_eq!(task.await.expect("blocked REQ task"), Some(false));
        close.await;
        assert_subscription_state_empty(&state, &conn).await;
    }

    #[tokio::test]
    async fn disconnect_prevents_a_blocked_req_from_registering_late() {
        let state = subscription_test_state().await;
        let conn = subscription_test_conn();
        let pending_reqs = Arc::new(DashMap::new());
        let (_, request_cancel) = start_pending_req(&pending_reqs, "history", &conn.cancel);
        let filters = vec![Filter::new().kind(nostr::Kind::TextNote)];
        let (started, resume, task) = spawn_blocked_registration(
            Arc::clone(&state),
            Arc::clone(&conn),
            "history",
            filters,
            request_cancel,
        );
        started.await.expect("blocked REQ started");

        conn.cancel.cancel();
        let mut cleanup = Box::pin(cleanup_connection_subscriptions(&conn, &state));
        assert!(futures_util::poll!(&mut cleanup).is_pending());
        resume.notify_one();

        assert_eq!(task.await.expect("blocked REQ task"), Some(false));
        cleanup.await;
        assert_subscription_state_empty(&state, &conn).await;
    }

    #[tokio::test]
    async fn replacement_prevents_an_older_blocked_req_from_overwriting_state() {
        let state = subscription_test_state().await;
        let conn = subscription_test_conn();
        let pending_reqs = Arc::new(DashMap::new());
        let (_, old_cancel) = start_pending_req(&pending_reqs, "live", &conn.cancel);
        let old_filters = vec![Filter::new().kind(nostr::Kind::TextNote)];
        let (started, resume, old_task) = spawn_blocked_registration(
            Arc::clone(&state),
            Arc::clone(&conn),
            "live",
            old_filters,
            old_cancel,
        );
        started.await.expect("old REQ started");

        let (_, new_cancel) = start_pending_req(&pending_reqs, "live", &conn.cancel);
        let new_filters = vec![Filter::new().kind(nostr::Kind::Reaction)];
        let mut replacement = Box::pin(crate::handlers::req::register_subscription_if_active(
            "live",
            &new_filters,
            None,
            &conn,
            &state,
            &new_cancel,
        ));
        assert!(futures_util::poll!(&mut replacement).is_pending());
        resume.notify_one();

        assert_eq!(old_task.await.expect("old REQ task"), Some(false));
        assert_eq!(replacement.await, Some(true));
        assert_eq!(
            conn.subscriptions.lock().await.get("live"),
            Some(&new_filters)
        );
        assert_eq!(
            state.sub_registry.get_filters(conn.conn_id, "live"),
            Some(new_filters)
        );
        assert_eq!(state.sub_registry.total_subscriptions(), 1);
        assert_eq!(
            state
                .pubsub
                .topic_refcount(&conn.tenant, EventTopic::Global)
                .await,
            1
        );

        cancel_pending_req(&pending_reqs, "live");
        crate::handlers::close::handle_close(
            "live".to_string(),
            Arc::clone(&conn),
            Arc::clone(&state),
        )
        .await;
        assert_subscription_state_empty(&state, &conn).await;
    }

    #[tokio::test]
    async fn close_and_replacement_suppress_cancelled_search_output() {
        let state = subscription_test_state().await;
        let (conn, mut send_rx) = subscription_test_conn_with_receiver();
        let pending_reqs = Arc::new(DashMap::new());
        let (close_id, close_cancel) =
            start_pending_req(&pending_reqs, "search-close", &conn.cancel);

        cancel_pending_req(&pending_reqs, "search-close");
        crate::handlers::close::handle_close(
            "search-close".to_string(),
            Arc::clone(&conn),
            Arc::clone(&state),
        )
        .await;
        assert!(!crate::handlers::req::send_search_frame_if_active(
            &conn,
            &pending_reqs,
            "search-close",
            close_id,
            &close_cancel,
            "stale EVENT after CLOSE".to_string(),
        ));
        assert!(!crate::handlers::req::send_search_frame_if_active(
            &conn,
            &pending_reqs,
            "search-close",
            close_id,
            &close_cancel,
            RelayMessage::eose("search-close"),
        ));

        let closed = send_rx.try_recv().expect("CLOSED acknowledgement");
        let WsMessage::Text(closed) = closed else {
            panic!("expected CLOSED text frame");
        };
        assert!(closed.contains(r#"["CLOSED","search-close""#));
        assert!(send_rx.try_recv().is_err(), "no stale search output");

        let (old_id, old_cancel) = start_pending_req(&pending_reqs, "search-replace", &conn.cancel);
        let (_, _new_cancel) = start_pending_req(&pending_reqs, "search-replace", &conn.cancel);
        assert!(!crate::handlers::req::send_search_frame_if_active(
            &conn,
            &pending_reqs,
            "search-replace",
            old_id,
            &old_cancel,
            "stale EVENT after replacement".to_string(),
        ));
        assert!(!crate::handlers::req::send_search_frame_if_active(
            &conn,
            &pending_reqs,
            "search-replace",
            old_id,
            &old_cancel,
            RelayMessage::eose("search-replace"),
        ));
        assert!(send_rx.try_recv().is_err(), "no replaced-generation output");
    }

    #[test]
    fn req_rejections_are_subscription_scoped() {
        let reason = "rate-limited: too many concurrent requests";
        let closed: serde_json::Value =
            serde_json::from_str(&request_rejection_message(Some("history-123"), reason))
                .expect("parse CLOSED");
        assert_eq!(closed, serde_json::json!(["CLOSED", "history-123", reason]));

        let notice: serde_json::Value =
            serde_json::from_str(&request_rejection_message(None, reason)).expect("parse NOTICE");
        assert_eq!(notice, serde_json::json!(["NOTICE", reason]));
    }

    #[tokio::test]
    async fn send_loop_batches_queued_data_frames_into_one_flush() {
        let (data_tx, data_rx) = mpsc::channel(MAX_WS_SEND_BATCH);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        for i in 0..5 {
            data_tx
                .send(WsMessage::Text(format!("data-{i}").into()))
                .await
                .expect("queue data frame");
        }

        let (sink, state) = MockSink::new(Some(1));
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            CancellationToken::new(),
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1);
        assert_eq!(
            text_payloads(&state.messages),
            vec!["data-0", "data-1", "data-2", "data-3", "data-4"]
        );
    }

    #[tokio::test]
    async fn send_loop_batch_one_preserves_single_frame_flush_behavior() {
        let (data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        data_tx
            .send(WsMessage::Text("single".into()))
            .await
            .expect("queue data frame");

        let (sink, state) = MockSink::new(Some(1));
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            CancellationToken::new(),
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1);
        assert_eq!(text_payloads(&state.messages), vec!["single"]);
    }

    #[tokio::test]
    async fn send_loop_drains_control_before_batched_data_without_reordering() {
        let (data_tx, data_rx) = mpsc::channel(MAX_WS_SEND_BATCH);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(1);
        data_tx
            .send(WsMessage::Text("data-0".into()))
            .await
            .expect("queue data frame");
        data_tx
            .send(WsMessage::Text("data-1".into()))
            .await
            .expect("queue data frame");
        ctrl_tx
            .send(WsMessage::Text("control".into()))
            .await
            .expect("queue control frame");

        let (sink, state) = MockSink::new(Some(2));
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            CancellationToken::new(),
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 2);
        assert_eq!(
            text_payloads(&state.messages),
            vec!["control", "data-0", "data-1"]
        );
    }

    #[tokio::test]
    async fn send_loop_acknowledges_restart_after_flushing_exactly_one_1012() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (restart_tx, restart_rx) = mpsc::channel(1);
        let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel();
        restart_tx
            .send(RestartClose {
                flushed: flushed_tx,
            })
            .await
            .expect("queue restart close");

        let (sink, state) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            CancellationToken::new(),
            ordinary_disconnect_reason(),
        )
        .await;

        assert_eq!(flushed_rx.await, Ok(true));
        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1, "ack follows the close flush");
        assert_eq!(state.messages.len(), 1, "writer exits after restart close");
        match &state.messages[0] {
            WsMessage::Close(Some(close)) => {
                assert_eq!(close.code, axum::extract::ws::close_code::RESTART);
                assert_eq!(close.reason.as_str(), "relay restarting");
            }
            other => panic!("expected one 1012 restart close, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_loop_reports_restart_flush_failure() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (restart_tx, restart_rx) = mpsc::channel(1);
        let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel();
        restart_tx
            .send(RestartClose {
                flushed: flushed_tx,
            })
            .await
            .expect("queue restart close");

        let (sink, state) = MockSink::new(Some(1));
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            CancellationToken::new(),
            ordinary_disconnect_reason(),
        )
        .await;

        assert_eq!(flushed_rx.await, Ok(false));
        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1);
        assert_eq!(state.messages.len(), 1, "no fallback close is appended");
    }

    #[tokio::test]
    async fn send_loop_sends_policy_close_when_community_is_deleted() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (sink, state) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            cancel,
            deleted_community_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.messages.len(), 1);
        match &state.messages[0] {
            WsMessage::Close(Some(close)) => {
                assert_eq!(close.code, axum::extract::ws::close_code::POLICY);
                assert_eq!(close.reason.as_str(), "community deleted");
            }
            other => panic!("expected one 1008 deletion close, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_loop_sends_bare_close_for_ordinary_cancellation() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (sink, state) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            cancel,
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.messages.as_slice(), [WsMessage::Close(None)]);
    }

    #[tokio::test]
    async fn send_loop_flushes_queued_control_before_close_on_cancel() {
        // A ban disconnect queues its `OK false "blocked: …"` reason frame on
        // the control channel and then cancels the token (B3). The biased
        // select polls the cancel branch first, so the reason frame would be
        // stranded unless the cancel branch drains ctrl before emitting Close.
        // This test exercises `send_loop_inner` end-to-end to prove the reason
        // frame reaches the client, in order, ahead of the Close.
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(1);
        ctrl_tx
            .send(WsMessage::Text("blocked: you are banned".into()))
            .await
            .expect("queue ban reason frame");

        let cancel = CancellationToken::new();
        cancel.cancel();

        let (sink, state) = MockSink::new(None);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            restart_rx,
            cancel,
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(
            state.messages.len(),
            2,
            "reason frame then Close, nothing else"
        );
        match &state.messages[0] {
            WsMessage::Text(text) => {
                assert_eq!(text.as_str(), "blocked: you are banned")
            }
            other => panic!("expected the ban reason frame first, got {other:?}"),
        }
        assert!(
            matches!(state.messages[1], WsMessage::Close(None)),
            "ordinary cancellation retains the bare Close after the reason frame"
        );
    }
}
