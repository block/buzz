//! WebSocket connection lifecycle: semaphore → challenge → recv/send/heartbeat loops → cleanup.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::{Sink, SinkExt, StreamExt};
use tokio::sync::{mpsc, watch, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

use buzz_auth::{generate_challenge, AuthContext};
use buzz_core::tenant::TenantContext;
use nostr::Filter;

use crate::handlers;
use crate::metrics::AuthOutcome;
use crate::protocol::{ClientMessage, RelayMessage};
use crate::rejection::{enforce_ws_admission, request_rejection_message, RejectionTarget};
use crate::state::{
    run_registered_community_connection, AppState, CommunityConnectionControl,
    CommunityDisconnectReason,
};

/// Maximum time a new socket may hold a connection slot without completing NIP-42 auth.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum time the writer may spend flushing terminal frames after cancellation.
/// This stays well inside the process-wide 30-second hard drain.
const WS_TERMINAL_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Shared mutable subscription map for a single WebSocket connection.
pub(crate) type ConnectionSubscriptions = Arc<Mutex<HashMap<String, Vec<Filter>>>>;

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
        /// When the challenge was delivered and this attempt began.
        started_at: Instant,
    },
    /// Client has successfully authenticated.
    Authenticated(AuthContext),
    /// Authentication attempt was rejected.
    Failed,
}

/// Per-connection state split by access pattern:
/// - `auth_state`: synchronous mutex (short, non-awaiting transitions; drop-safe cleanup)
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
    pub auth_state: StdMutex<AuthState>,
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
    fn lock_auth_state(&self) -> std::sync::MutexGuard<'_, AuthState> {
        self.auth_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Snapshot the current authentication state without holding its lock over an await.
    pub(crate) fn auth_state_snapshot(&self) -> AuthState {
        self.lock_auth_state().clone()
    }

    fn transition_pending_auth(&self, next: AuthState, outcome: AuthOutcome) -> bool {
        let mut auth = self.lock_auth_state();
        let AuthState::Pending { started_at, .. } = &*auth else {
            return false;
        };
        let duration = started_at.elapsed();
        let became_authenticated = matches!(next, AuthState::Authenticated(_));
        *auth = next;
        crate::metrics::record_auth_outcome(outcome, duration);
        if became_authenticated {
            // Keep the gauge update under the same state lock as the
            // Pending -> Authenticated transition. Cleanup must never observe
            // Authenticated before its increment and decrement first.
            metrics::gauge!("buzz_ws_authenticated_connections_active").increment(1.0);
        }
        true
    }

    /// Atomically finish the initial challenge as authenticated.
    pub(crate) fn authenticate(&self, auth_context: AuthContext) -> bool {
        self.transition_pending_auth(AuthState::Authenticated(auth_context), AuthOutcome::Success)
    }

    /// Atomically finish the initial challenge with a bounded denial.
    pub(crate) fn reject_auth(&self, outcome: AuthOutcome) -> bool {
        debug_assert!(!matches!(outcome, AuthOutcome::Success));
        self.transition_pending_auth(AuthState::Failed, outcome)
    }

    /// Finish a pending challenge on timeout and preserve the historical rule
    /// that an already-failed connection is closed when its timeout expires.
    fn expire_auth(&self) -> bool {
        let mut auth = self.lock_auth_state();
        match &*auth {
            AuthState::Pending { started_at, .. } => {
                let duration = started_at.elapsed();
                *auth = AuthState::Failed;
                crate::metrics::record_auth_outcome(AuthOutcome::Timeout, duration);
                true
            }
            AuthState::Failed => true,
            AuthState::Authenticated(_) => false,
        }
    }

    /// Finalize authentication accounting when a connection closes.
    ///
    /// Replacing the state with `Failed` makes cleanup idempotent: an
    /// authenticated gauge can be decremented at most once, and a pending
    /// attempt can receive at most one disconnect/shutdown terminal.
    fn finish_auth_on_close(&self, outcome: AuthOutcome) -> Option<AuthContext> {
        debug_assert!(matches!(
            outcome,
            AuthOutcome::Disconnect | AuthOutcome::Shutdown
        ));
        let mut auth = self.lock_auth_state();
        match std::mem::replace(&mut *auth, AuthState::Failed) {
            AuthState::Pending { started_at, .. } => {
                crate::metrics::record_auth_outcome(outcome, started_at.elapsed());
                None
            }
            AuthState::Authenticated(auth_context) => {
                metrics::gauge!("buzz_ws_authenticated_connections_active").decrement(1.0);
                Some(auth_context)
            }
            AuthState::Failed => None,
        }
    }

    /// Let the cancellation watcher claim only a still-pending attempt.
    /// Authenticated cleanup remains owned by `AuthLifecycleGuard`, which must
    /// retain the authentication context for presence cleanup after task joins.
    fn finish_pending_auth_on_cancel(&self, outcome: AuthOutcome) -> bool {
        debug_assert!(matches!(
            outcome,
            AuthOutcome::Disconnect | AuthOutcome::Shutdown
        ));
        let mut auth = self.lock_auth_state();
        let AuthState::Pending { started_at, .. } = &*auth else {
            return false;
        };
        let duration = started_at.elapsed();
        *auth = AuthState::Failed;
        crate::metrics::record_auth_outcome(outcome, duration);
        true
    }

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

/// Owns authentication accounting for exactly the lifetime of the production
/// connection future. Explicit teardown selects the precise close outcome;
/// aborts and panics fall back to `disconnect` and cancel child tasks.
struct AuthLifecycleGuard {
    conn: Arc<ConnectionState>,
    finished: bool,
}

impl AuthLifecycleGuard {
    fn new(conn: Arc<ConnectionState>) -> Self {
        Self {
            conn,
            finished: false,
        }
    }

    fn finish(&mut self, outcome: AuthOutcome) -> Option<AuthContext> {
        self.finished = true;
        self.conn.finish_auth_on_close(outcome)
    }
}

impl Drop for AuthLifecycleGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.conn.cancel.cancel();
            self.conn.finish_auth_on_close(AuthOutcome::Disconnect);
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
        auth_state: StdMutex::new(AuthState::Pending {
            challenge: challenge.clone(),
            started_at: Instant::now(),
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
    crate::metrics::record_auth_attempt_started();
    let mut auth_lifecycle = AuthLifecycleGuard::new(Arc::clone(&conn));

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
                if auth_timeout_conn.expire_auth() {
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

    // Cancellation races database-backed AUTH work. This watcher claims the
    // pending lifecycle under the same lock as success/denial transitions, so
    // whichever terminal happens first wins and a late handler cannot overwrite it.
    let auth_cancel_conn = Arc::clone(&conn);
    let auth_cancel_state = Arc::clone(&state);
    let auth_cancel_token = cancel.clone();
    let auth_cancel_task = tokio::spawn(async move {
        auth_cancel_token.cancelled().await;
        let outcome = if auth_cancel_state.shutting_down.load(Ordering::Acquire) {
            AuthOutcome::Shutdown
        } else {
            AuthOutcome::Disconnect
        };
        auth_cancel_conn.finish_pending_auth_on_cancel(outcome);
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
    let close_outcome = if state.shutting_down.load(Ordering::Acquire) {
        AuthOutcome::Shutdown
    } else {
        AuthOutcome::Disconnect
    };
    // Terminalize before joining writer/heartbeat tasks. A blocked socket sink
    // must not keep the authenticated gauge high during shutdown.
    let authenticated = auth_lifecycle.finish(close_outcome);

    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = auth_timeout_task.await;
    let _ = auth_cancel_task.await;

    for removed in state.sub_registry.remove_connection(conn.conn_id) {
        if removed.scope.is_global() {
            state
                .pubsub
                .release_topic(&conn.tenant, buzz_pubsub::EventTopic::Global)
                .await;
        }
        for &channel_id in removed.scope.channel_ids() {
            state
                .pubsub
                .release_topic(&conn.tenant, buzz_pubsub::EventTopic::Channel(channel_id))
                .await;
        }
    }
    state.conn_manager.deregister(conn.conn_id);
    if let Some(auth_ctx) = authenticated {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterStep {
    Completed,
    Cancelled,
    Failed,
}

async fn send_or_cancel<S>(
    sink: &mut S,
    message: WsMessage,
    cancel: &CancellationToken,
) -> WriterStep
where
    S: Sink<WsMessage> + Unpin,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => WriterStep::Cancelled,
        result = sink.send(message) => {
            if result.is_ok() { WriterStep::Completed } else { WriterStep::Failed }
        }
    }
}

async fn feed_or_cancel<S>(
    sink: &mut S,
    message: WsMessage,
    cancel: &CancellationToken,
) -> WriterStep
where
    S: Sink<WsMessage> + Unpin,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => WriterStep::Cancelled,
        result = sink.feed(message) => {
            if result.is_ok() { WriterStep::Completed } else { WriterStep::Failed }
        }
    }
}

async fn flush_or_cancel<S>(sink: &mut S, cancel: &CancellationToken) -> WriterStep
where
    S: Sink<WsMessage> + Unpin,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => WriterStep::Cancelled,
        result = sink.flush() => {
            if result.is_ok() { WriterStep::Completed } else { WriterStep::Failed }
        }
    }
}

/// Best-effort terminal delivery with one shared deadline. A socket that never
/// becomes writable cannot retain its connection task or semaphore permit.
async fn flush_terminal_frames<S>(
    sink: &mut S,
    ctrl_rx: &mut mpsc::Receiver<WsMessage>,
    disconnect_reason: &watch::Receiver<Option<CommunityDisconnectReason>>,
    first_ctrl: Option<WsMessage>,
) where
    S: Sink<WsMessage> + Unpin,
{
    let deadline = tokio::time::Instant::now() + WS_TERMINAL_FLUSH_TIMEOUT;
    if let Some(ctrl_msg) = first_ctrl {
        if !matches!(
            tokio::time::timeout_at(deadline, sink.send(ctrl_msg)).await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
    while let Ok(ctrl_msg) = ctrl_rx.try_recv() {
        if !matches!(
            tokio::time::timeout_at(deadline, sink.send(ctrl_msg)).await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
    let close = disconnect_reason
        .borrow()
        .map_or(WsMessage::Close(None), |reason| reason.close_message());
    let _ = tokio::time::timeout_at(deadline, sink.send(close)).await;
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
            match send_or_cancel(&mut ws_send, ctrl_msg.clone(), &cancel).await {
                WriterStep::Completed => {}
                WriterStep::Cancelled => {
                    flush_terminal_frames(
                        &mut ws_send,
                        &mut ctrl_rx,
                        &disconnect_reason,
                        Some(ctrl_msg),
                    )
                    .await;
                    return;
                }
                WriterStep::Failed => return,
            }
        }

        tokio::select! {
            // Biased: restart > cancel > ordinary control > data. A restart
            // command owns shutdown delivery and must flush its 1012 before
            // cancellation can fall back to an unacknowledged close.
            biased;
            Some(restart) = restart_rx.recv() => {
                let sent = matches!(
                    tokio::time::timeout(
                        WS_TERMINAL_FLUSH_TIMEOUT,
                        ws_send.send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                        code: axum::extract::ws::close_code::RESTART,
                        reason: axum::extract::ws::Utf8Bytes::from_static("relay restarting"),
                        }))),
                    ).await,
                    Ok(Ok(()))
                );
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
                flush_terminal_frames(&mut ws_send, &mut ctrl_rx, &disconnect_reason, None).await;
                break;
            }
            Some(ctrl_msg) = ctrl_rx.recv() => {
                match send_or_cancel(&mut ws_send, ctrl_msg.clone(), &cancel).await {
                    WriterStep::Completed => {}
                    WriterStep::Cancelled => {
                        flush_terminal_frames(
                            &mut ws_send,
                            &mut ctrl_rx,
                            &disconnect_reason,
                            Some(ctrl_msg),
                        )
                        .await;
                        break;
                    }
                    WriterStep::Failed => break,
                }
            }
            Some(msg) = data_rx.recv() => {
                let mut batched = 1usize;
                match feed_or_cancel(&mut ws_send, msg, &cancel).await {
                    WriterStep::Completed => {}
                    WriterStep::Cancelled => {
                        flush_terminal_frames(
                            &mut ws_send,
                            &mut ctrl_rx,
                            &disconnect_reason,
                            None,
                        )
                        .await;
                        break;
                    }
                    WriterStep::Failed => break,
                }

                while batched < MAX_WS_SEND_BATCH {
                    match data_rx.try_recv() {
                        Ok(next) => {
                            match feed_or_cancel(&mut ws_send, next, &cancel).await {
                                WriterStep::Completed => {}
                                WriterStep::Cancelled => {
                                    flush_terminal_frames(
                                        &mut ws_send,
                                        &mut ctrl_rx,
                                        &disconnect_reason,
                                        None,
                                    )
                                    .await;
                                    return;
                                }
                                WriterStep::Failed => return,
                            }
                            batched += 1;
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                match flush_or_cancel(&mut ws_send, &cancel).await {
                    WriterStep::Completed => {}
                    WriterStep::Cancelled => {
                        flush_terminal_frames(
                            &mut ws_send,
                            &mut ctrl_rx,
                            &disconnect_reason,
                            None,
                        )
                        .await;
                        break;
                    }
                    WriterStep::Failed => break,
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
                        handle_text_message(text.to_string(), Arc::clone(&conn), Arc::clone(&state)).await;
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
                            handle_text_message(text, Arc::clone(&conn), Arc::clone(&state)).await;
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

async fn handle_text_message(text: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
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
            // AUTH remains inline so only one frame can race the connection's
            // pending lifecycle, but cancellation can preempt dependency waits.
            let span = tracing::info_span!("ws.auth", conn_id = %conn.conn_id);
            tokio::select! {
                biased;
                _ = conn.cancel.cancelled() => {}
                _ = handlers::auth::handle_auth(event, Arc::clone(&conn), Arc::clone(&state))
                    .instrument(span) => {}
            }
        }
        ClientMessage::Event(event) => {
            let conn = Arc::clone(&conn);
            let state = Arc::clone(&state);
            let permit = match state.handler_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    // Correlate to the event id: a bare NOTICE here strands the
                    // client's pending publish exactly as an over-quota one did.
                    conn.send(request_rejection_message(
                        RejectionTarget::Event(event.id),
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
        ClientMessage::Req {
            sub_id,
            filters,
            before_ids,
        } => {
            let conn = Arc::clone(&conn);
            let state = Arc::clone(&state);
            let permit = match state.handler_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    conn.send(request_rejection_message(
                        RejectionTarget::Subscription(&sub_id),
                        "rate-limited: too many concurrent requests",
                    ));
                    return;
                }
            };
            let span = tracing::info_span!("ws.req", conn_id = %conn.conn_id, sub_id = %sub_id);
            tokio::spawn(
                async move {
                    handlers::req::handle_req(sub_id, filters, before_ids, conn, state).await;
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
                    conn.send(request_rejection_message(
                        RejectionTarget::Subscription(&sub_id),
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
            handlers::close::handle_close(sub_id, Arc::clone(&conn), Arc::clone(&state)).await;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{extract::ws::WebSocketUpgrade, routing::get, Router};
    use metrics_util::debugging::DebugValue;
    use std::sync::{Arc, Mutex};

    use buzz_auth::AuthMethod;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl};
    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    /// A connection whose outbound frames a test can read back.
    ///
    /// Lives here, next to `ConnectionState`, so the crate has one place that
    /// knows how to build one. Shared with `crate::rejection`'s tests.
    pub(crate) fn test_conn_with_auth(
        auth: AuthState,
    ) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
        let (send_tx, send_rx) = mpsc::channel(4);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(4);
        let conn = ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().expect("socket addr"),
            auth_state: StdMutex::new(auth),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        };
        (Arc::new(conn), send_rx)
    }

    /// An authenticated connection — the only state admission quotas apply to.
    pub(crate) fn authenticated_state() -> AuthState {
        AuthState::Authenticated(auth_context())
    }

    fn auth_context() -> AuthContext {
        AuthContext {
            pubkey: Keys::generate().public_key(),
            scopes: Vec::new(),
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        }
    }

    fn pending_state() -> AuthState {
        AuthState::Pending {
            challenge: "test-challenge".to_owned(),
            started_at: Instant::now(),
        }
    }

    type MetricSnapshot = Vec<(
        metrics_util::CompositeKey,
        Option<metrics::Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    )>;

    fn counter_value(snapshot: &MetricSnapshot, name: &str, outcome: Option<&str>) -> u64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                if key.key().name() != name {
                    return None;
                }
                let labels = key.key().labels().collect::<Vec<_>>();
                if outcome.is_some_and(|expected| {
                    !labels
                        .iter()
                        .any(|label| label.key() == "outcome" && label.value() == expected)
                }) {
                    return None;
                }
                let DebugValue::Counter(value) = value else {
                    panic!("{name} must be a counter");
                };
                Some(*value)
            })
            .unwrap_or_default()
    }

    fn labeled_counter_value(
        snapshot: &MetricSnapshot,
        name: &str,
        label_key: &str,
        label_value: &str,
    ) -> u64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                (key.key().name() == name
                    && key
                        .key()
                        .labels()
                        .any(|label| label.key() == label_key && label.value() == label_value))
                .then(|| match value {
                    DebugValue::Counter(value) => *value,
                    _ => panic!("{name} must be a counter"),
                })
            })
            .unwrap_or_default()
    }

    fn labeled_gauge_value(snapshot: &MetricSnapshot, name: &str, labels: &[(&str, &str)]) -> f64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                let matches = key.key().name() == name
                    && labels.iter().all(|(expected_key, expected_value)| {
                        key.key().labels().any(|label| {
                            label.key() == *expected_key && label.value() == *expected_value
                        })
                    });
                matches.then(|| match value {
                    DebugValue::Gauge(value) => value.into_inner(),
                    _ => panic!("{name} must be a gauge"),
                })
            })
            .unwrap_or_default()
    }

    fn authenticated_gauge(snapshot: &MetricSnapshot) -> f64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                if key.key().name() != "buzz_ws_authenticated_connections_active" {
                    return None;
                }
                let DebugValue::Gauge(value) = value else {
                    panic!("authenticated connections must be a gauge");
                };
                Some(value.into_inner())
            })
            .unwrap_or_default()
    }

    pub(crate) fn read_frame(rx: &mut mpsc::Receiver<WsMessage>) -> serde_json::Value {
        match rx.try_recv().expect("a frame was sent") {
            WsMessage::Text(text) => serde_json::from_str(&text).expect("valid JSON frame"),
            other => panic!("unexpected websocket message: {other:?}"),
        }
    }

    /// Exercise the real state-transition methods for every terminal. The
    /// attempt counter must reconcile with exactly one terminal per completed
    /// attempt, and repeated/racing terminal calls must not drive the active
    /// authenticated gauge below zero.
    #[tokio::test(flavor = "current_thread")]
    async fn auth_lifecycle_reconciles_every_terminal_and_never_leaks_gauge() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);

        crate::metrics::record_auth_attempt_started();
        let (success, _rx) = test_conn_with_auth(pending_state());
        assert!(success.authenticate(auth_context()));
        assert!(
            !success.authenticate(auth_context()),
            "a terminal attempt cannot authenticate twice"
        );
        assert!(
            !success.finish_pending_auth_on_cancel(AuthOutcome::Disconnect),
            "the cancellation watcher must leave authenticated context for lifecycle cleanup"
        );
        assert!(success
            .finish_auth_on_close(AuthOutcome::Disconnect)
            .is_some());
        assert!(
            success
                .finish_auth_on_close(AuthOutcome::Shutdown)
                .is_none(),
            "cleanup must be idempotent"
        );

        for outcome in [
            AuthOutcome::Invalid,
            AuthOutcome::Banned,
            AuthOutcome::BanCheckError,
            AuthOutcome::AllowlistCheckError,
            AuthOutcome::AllowlistDenied,
            AuthOutcome::RelayMembershipCheckError,
            AuthOutcome::NotRelayMember,
        ] {
            crate::metrics::record_auth_attempt_started();
            let (denied, _rx) = test_conn_with_auth(pending_state());
            assert!(denied.reject_auth(outcome));
            assert!(!denied.reject_auth(outcome), "a denial cannot record twice");
            assert!(denied
                .finish_auth_on_close(AuthOutcome::Disconnect)
                .is_none());
        }

        crate::metrics::record_auth_attempt_started();
        let (timed_out, _rx) = test_conn_with_auth(pending_state());
        assert!(timed_out.expire_auth());
        assert!(timed_out
            .finish_auth_on_close(AuthOutcome::Disconnect)
            .is_none());

        for outcome in [AuthOutcome::Disconnect, AuthOutcome::Shutdown] {
            crate::metrics::record_auth_attempt_started();
            let (closed, _rx) = test_conn_with_auth(pending_state());
            assert!(closed.finish_auth_on_close(outcome).is_none());
            assert!(closed.finish_auth_on_close(outcome).is_none());
        }

        let snapshot = snapshotter.snapshot().into_vec();
        let attempts = counter_value(&snapshot, "buzz_auth_attempts_total", None);
        let outcomes = AuthOutcome::ALL
            .iter()
            .map(|outcome| {
                let value = counter_value(
                    &snapshot,
                    "buzz_auth_outcomes_total",
                    Some(outcome.as_str()),
                );
                assert_eq!(
                    value,
                    1,
                    "{} terminal must be recorded exactly once",
                    outcome.as_str()
                );
                value
            })
            .sum::<u64>();

        assert_eq!(attempts, AuthOutcome::ALL.len() as u64);
        assert_eq!(attempts, outcomes);
        assert_eq!(authenticated_gauge(&snapshot), 0.0);
    }

    /// Post-terminal AUTH floods traverse the production frame dispatcher but
    /// cannot mint rollout-gating challenge lifecycles or outcomes.
    #[tokio::test(flavor = "current_thread")]
    async fn auth_flood_after_terminal_only_increments_protocol_noise_metric() {
        const FRAMES_PER_STATE: u64 = 64;

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);
        let state = crate::state::tests::test_state().await;
        let mut authoritative_attempts = 0;
        let mut authoritative_outcomes = 0;

        for (auth_state, expected_state) in [
            (
                authenticated_state(),
                crate::metrics::AuthPostTerminalState::Authenticated,
            ),
            (
                AuthState::Failed,
                crate::metrics::AuthPostTerminalState::Failed,
            ),
        ] {
            let (conn, mut rx) = test_conn_with_auth(auth_state);
            for sequence in 0..FRAMES_PER_STATE {
                let event = EventBuilder::new(Kind::Authentication, format!("noise-{sequence}"))
                    .sign_with_keys(&Keys::generate())
                    .expect("sign AUTH noise event");
                let raw = serde_json::json!(["AUTH", event]).to_string();
                handle_text_message(raw, Arc::clone(&conn), Arc::clone(&state)).await;
                let frame = read_frame(&mut rx);
                assert_eq!(frame[2], false);
            }
            assert!(!conn.cancel.is_cancelled());
            let snapshot = snapshotter.snapshot().into_vec();
            authoritative_attempts += counter_value(&snapshot, "buzz_auth_attempts_total", None);
            authoritative_outcomes += counter_value(&snapshot, "buzz_auth_outcomes_total", None);
            assert_eq!(
                labeled_counter_value(
                    &snapshot,
                    "buzz_auth_post_terminal_frames_total",
                    "state",
                    expected_state.as_str(),
                ),
                FRAMES_PER_STATE
            );
        }

        let snapshot = snapshotter.snapshot().into_vec();
        authoritative_attempts += counter_value(&snapshot, "buzz_auth_attempts_total", None);
        authoritative_outcomes += counter_value(&snapshot, "buzz_auth_outcomes_total", None);
        assert_eq!(
            authoritative_attempts, 0,
            "post-terminal protocol noise cannot create authoritative attempts"
        );
        assert_eq!(
            authoritative_outcomes, 0,
            "post-terminal protocol noise cannot create authoritative outcomes"
        );
    }

    /// Aborting the production lifecycle owner must synchronously terminalize
    /// pending accounting and release an authenticated gauge exactly once.
    #[tokio::test(flavor = "current_thread")]
    async fn aborting_lifecycle_owner_is_drop_safe() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);

        crate::metrics::record_auth_attempt_started();
        let (authenticated, _rx) = test_conn_with_auth(pending_state());
        assert!(authenticated.authenticate(auth_context()));
        let authenticated_started = Arc::new(Notify::new());
        let task = {
            let conn = Arc::clone(&authenticated);
            let started = Arc::clone(&authenticated_started);
            tokio::spawn(async move {
                let _guard = AuthLifecycleGuard::new(conn);
                started.notify_one();
                std::future::pending::<()>().await;
            })
        };
        authenticated_started.notified().await;
        task.abort();
        assert!(task.await.expect_err("task was aborted").is_cancelled());

        crate::metrics::record_auth_attempt_started();
        let (pending, _rx) = test_conn_with_auth(pending_state());
        let pending_started = Arc::new(Notify::new());
        let task = {
            let conn = Arc::clone(&pending);
            let started = Arc::clone(&pending_started);
            tokio::spawn(async move {
                let _guard = AuthLifecycleGuard::new(conn);
                started.notify_one();
                std::future::pending::<()>().await;
            })
        };
        pending_started.notified().await;
        task.abort();
        assert!(task.await.expect_err("task was aborted").is_cancelled());

        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(
            counter_value(&snapshot, "buzz_auth_attempts_total", None),
            2
        );
        assert_eq!(
            counter_value(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Success.as_str()),
            ),
            1
        );
        assert_eq!(
            counter_value(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Disconnect.as_str()),
            ),
            1
        );
        assert_eq!(authenticated_gauge(&snapshot), 0.0);
        assert!(matches!(pending.auth_state_snapshot(), AuthState::Failed));
    }

    /// Drive the real upgraded WebSocket lifecycle until AUTH is waiting for
    /// the sole database connection. Production shutdown must claim the
    /// pending attempt before that dependency is released, and the late DB
    /// result must not turn the terminal into success.
    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_terminalizes_auth_stalled_on_database_before_late_success() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);

        // Occupy the pool's only connection attempt with a fake PostgreSQL
        // endpoint that accepts TCP and never completes the startup handshake.
        // The production AUTH query then waits for the pool permit without
        // requiring a developer database or relying on timing alone.
        let fake_database = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake PostgreSQL endpoint");
        let database_url = format!(
            "postgres://buzz@{}/buzz",
            fake_database.local_addr().expect("fake database address")
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_lazy(&database_url)
            .expect("create lifecycle test pool");
        let blocker_pool = pool.clone();
        let blocker = tokio::spawn(async move { blocker_pool.acquire().await });
        let (blocked_database_stream, _) = fake_database
            .accept()
            .await
            .expect("pool reached fake PostgreSQL endpoint");
        let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        let tenant = TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            "test.local".to_owned(),
        );
        let expected_relay_url: RelayUrl =
            crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant)
                .parse()
                .expect("expected NIP-42 relay URL");

        let connection_finished = Arc::new(Notify::new());
        let route_state = Arc::clone(&state);
        let route_tenant = tenant.clone();
        let route_finished = Arc::clone(&connection_finished);
        let app = Router::new().route(
            "/",
            get(move |ws: WebSocketUpgrade| {
                let state = Arc::clone(&route_state);
                let tenant = route_tenant.clone();
                let finished = Arc::clone(&route_finished);
                async move {
                    ws.on_upgrade(move |socket| async move {
                        let cancel = CancellationToken::new();
                        let control = CommunityConnectionControl::new(cancel);
                        handle_active_connection(
                            socket,
                            state,
                            "127.0.0.1:1234".parse().expect("client address"),
                            tenant,
                            Uuid::new_v4(),
                            control,
                        )
                        .await;
                        finished.notify_one();
                    })
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind lifecycle WebSocket listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve lifecycle WebSocket");
        });

        let (mut client, _) = connect_async(format!("ws://{address}/"))
            .await
            .expect("connect lifecycle WebSocket");
        let challenge_frame = client
            .next()
            .await
            .expect("challenge frame")
            .expect("read challenge");
        let Message::Text(challenge_text) = challenge_frame else {
            panic!("expected text challenge")
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("parse challenge");
        assert_eq!(challenge_json[0], "AUTH");
        let challenge = challenge_json[1].as_str().expect("challenge string");
        let auth_event = EventBuilder::auth(challenge, expected_relay_url)
            .sign_with_keys(&Keys::generate())
            .expect("sign NIP-42 AUTH");
        client
            .send(Message::Text(
                serde_json::json!(["AUTH", auth_event]).to_string().into(),
            ))
            .await
            .expect("send NIP-42 AUTH");

        let attempts_before_shutdown = tokio::time::timeout(Duration::from_secs(2), async {
            let mut attempts = 0;
            loop {
                let snapshot = snapshotter.snapshot().into_vec();
                attempts += counter_value(&snapshot, "buzz_auth_attempts_total", None);
                if labeled_gauge_value(
                    &snapshot,
                    "buzz_db_pool_waiters",
                    &[("pool_role", "writer"), ("operation", "authorization")],
                ) >= 1.0
                {
                    break attempts;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("AUTH reached the blocked production DB acquisition");
        assert_eq!(
            attempts_before_shutdown, 1,
            "the production-issued challenge must own the attempt start"
        );

        state.begin_shutdown();
        assert_eq!(state.conn_manager.drain_all(), 1);
        tokio::time::timeout(Duration::from_secs(2), connection_finished.notified())
            .await
            .expect("production connection lifecycle finishes after shutdown");

        // Release the dependency only after production shutdown has claimed
        // the pending lifecycle, then prove no late handler result can win.
        drop(blocked_database_stream);
        blocker.abort();
        let _ = blocker.await;
        pool.close().await;

        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(
            counter_value(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Shutdown.as_str()),
            ),
            1
        );
        assert_eq!(
            counter_value(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Success.as_str()),
            ),
            0,
            "releasing the dependency after shutdown must not record late success"
        );
        assert_eq!(authenticated_gauge(&snapshot), 0.0);

        drop(client);
        server.abort();
        let _ = server.await;
    }

    /// Drives the real `handle_text_message` with every handler permit held, so
    /// the EVENT saturation branch is reached through production dispatch rather
    /// than by calling its helpers directly.
    ///
    /// This must go through `handle_text_message`: a test that renders the
    /// rejection frame itself stays green when the call site inside the match
    /// arm is reverted to a bare `NOTICE`.
    #[tokio::test]
    async fn saturated_handler_rejects_an_event_on_the_ok_channel() {
        let state = crate::state::tests::test_state().await;
        // An unauthenticated connection skips the admission quotas, so the
        // semaphore is the only gate the frame can trip.
        let (conn, mut rx) = test_conn_with_auth(AuthState::Failed);

        let permits = state.handler_semaphore.available_permits();
        let _held = Arc::clone(&state.handler_semaphore)
            .acquire_many_owned(permits as u32)
            .await
            .expect("hold every handler permit");

        let event = EventBuilder::new(Kind::TextNote, "hello")
            .sign_with_keys(&Keys::generate())
            .expect("sign event");
        let event_id = event.id.to_hex();
        let raw = serde_json::json!(["EVENT", event]).to_string();

        handle_text_message(raw, Arc::clone(&conn), Arc::clone(&state)).await;

        let frame = read_frame(&mut rx);
        assert_eq!(
            frame[0], "OK",
            "an EVENT turned away for handler saturation must be rejected on the \
             OK channel — a NOTICE carries no event id, so the client's pending \
             publish cannot be settled and the send only times out"
        );
        assert_eq!(frame[1], event_id);
        assert_eq!(frame[2], false);
        assert_eq!(frame[3], "rate-limited: too many concurrent requests");
    }

    /// The REQ arm of the same branch still settles on CLOSED.
    #[tokio::test]
    async fn saturated_handler_rejects_a_req_on_the_closed_channel() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut rx) = test_conn_with_auth(AuthState::Failed);

        let permits = state.handler_semaphore.available_permits();
        let _held = Arc::clone(&state.handler_semaphore)
            .acquire_many_owned(permits as u32)
            .await
            .expect("hold every handler permit");

        let raw = serde_json::json!(["REQ", "history-abc", {"kinds": [1]}]).to_string();
        handle_text_message(raw, Arc::clone(&conn), Arc::clone(&state)).await;

        let frame = read_frame(&mut rx);
        assert_eq!(frame[0], "CLOSED");
        assert_eq!(frame[1], "history-abc");
    }

    /// COUNT refusals follow NIP-45 and close the named query.
    #[tokio::test]
    async fn saturated_handler_rejects_a_count_on_the_closed_channel() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut rx) = test_conn_with_auth(AuthState::Failed);

        let permits = state.handler_semaphore.available_permits();
        let _held = Arc::clone(&state.handler_semaphore)
            .acquire_many_owned(permits as u32)
            .await
            .expect("hold every handler permit");

        let raw = serde_json::json!(["COUNT", "count-abc", {"kinds": [1]}]).to_string();
        handle_text_message(raw, Arc::clone(&conn), Arc::clone(&state)).await;

        let frame = read_frame(&mut rx);
        assert_eq!(frame[0], "CLOSED");
        assert_eq!(frame[1], "count-abc");
        assert_eq!(frame[2], "rate-limited: too many concurrent requests");
    }

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

    #[derive(Debug)]
    struct NeverReadySink {
        ready_polled: Arc<Notify>,
    }

    impl Sink<WsMessage> for NeverReadySink {
        type Error = std::io::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            self.ready_polled.notify_one();
            std::task::Poll::Pending
        }

        fn start_send(self: std::pin::Pin<&mut Self>, _item: WsMessage) -> Result<(), Self::Error> {
            panic!("a never-ready sink must not accept a frame")
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
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

    /// Cancellation must break a writer blocked in `poll_ready`, and terminal
    /// close delivery gets one bounded best-effort window before teardown wins.
    #[tokio::test(start_paused = true)]
    async fn cancelled_never_ready_sink_cannot_retain_writer_task() {
        let (data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let ready_polled = Arc::new(Notify::new());
        data_tx
            .send(WsMessage::Text("blocked".into()))
            .await
            .expect("queue blocked frame");

        let writer = tokio::spawn(send_loop_inner(
            NeverReadySink {
                ready_polled: Arc::clone(&ready_polled),
            },
            data_rx,
            ctrl_rx,
            restart_rx,
            cancel.clone(),
            ordinary_disconnect_reason(),
        ));

        ready_polled.notified().await;
        cancel.cancel();
        tokio::task::yield_now().await;
        tokio::time::advance(WS_TERMINAL_FLUSH_TIMEOUT + Duration::from_millis(1)).await;
        writer
            .await
            .expect("writer exits after bounded terminal flush");
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
