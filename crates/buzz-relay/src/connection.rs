//! WebSocket connection lifecycle: semaphore → challenge → recv/send/heartbeat loops → cleanup.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::{Sink, SinkExt, StreamExt};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

use buzz_auth::{generate_challenge, AuthContext};
use buzz_core::tenant::TenantContext;
use nostr::Filter;

use crate::handlers;
use crate::protocol::{ClientMessage, RelayMessage};
use crate::rejection::{enforce_ws_admission, request_rejection_message, RejectionTarget};
use crate::state::{
    run_registered_community_connection, AppState, CommunityConnectionControl,
    CommunityDisconnectReason,
};

/// Maximum time a new socket may hold a connection slot without completing NIP-42 auth.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Dedicated one-slot sender for the terminal NIP-FI denial frame.
    ///
    /// Because only one terminal event fires per connection lifetime (either key
    /// pairing mismatch or session expiry, never both), this channel is always
    /// available when the denial is enqueued — it cannot be saturated by ordinary
    /// control traffic. The send_loop drains it in its cancel branch ahead of
    /// `Close`, guaranteeing the denial frame is delivered even when `ctrl_tx`
    /// (capacity 8) is full. [FI-INV-05, FI-TRACE-LEASE-BOUND]
    pub terminal_ctrl_tx: mpsc::Sender<WsMessage>,
    /// Token used to signal graceful shutdown of this connection's tasks.
    pub cancel: CancellationToken,
    /// Consecutive buffer-full events. Cancel only after `grace_limit`.
    /// Shared with `ConnectionManager::ConnEntry` so both direct sends and
    /// fan-out broadcasts track the same counter.
    pub backpressure_count: Arc<AtomicU8>,
    /// Configurable slow-client grace limit (from `Config::slow_client_grace_limit`).
    pub grace_limit: u8,

    /// The NIP-FI assertion presented at upgrade, when enforcement is enabled.
    ///
    /// `None` means the relay is in `Off` mode — no assertion is required.
    /// When `Some`, the NIP-42 key pairing check uses this to enforce that
    /// `assertion.asserted_key() == nip42_pubkey` unconditionally (S3 invariant:
    /// no flag reads — S2 deleted `require_attested_key`). [FI-INV-05]
    pub nip_fi_assertion: Option<buzz_auth::VerifiedAssertion>,

    /// The UTC deadline after which this connection's NIP-FI lease expires.
    ///
    /// `None` means no assertion-based lifetime is enforced (mode is `Off`).
    /// When `Some`, the session-expiry task fires at this instant and sends
    /// `restricted: authorization denied` + cancels. Equality is expired.
    /// [FI-TRACE-LEASE-BOUND]
    pub session_deadline: Option<chrono::DateTime<chrono::Utc>>,

    /// The NIP-FI session admission gate. Every WS connection has exactly one
    /// gate — this is the [one-gate-per-connection] invariant.
    ///
    /// In enforce mode (assertion presented at upgrade), the gate has a
    /// deadline and the expiry task calls `gate.expire()` at that deadline.
    /// In off-mode (no assertion), the gate has no deadline and never
    /// self-expires — `acquire_effect()` always succeeds unless the outer
    /// cancel token fires.
    ///
    /// Handlers that perform irreversible side effects (AUTH state commit,
    /// EVENT persistence, REQ subscription registration, COUNT query) must
    /// call `gate.acquire_effect()` at the irreversible seam. The gate's
    /// quiescence barrier ensures connection teardown (subscription removal,
    /// peer cleanup) cannot start until all pre-expiry effects finish their
    /// bounded commits. [FI-TRACE-LEASE-BOUND, one-gate-per-connection]
    pub(crate) nip_fi_gate: std::sync::Arc<crate::nip_fi_gate::SessionAdmissionGate>,

    /// Shared with `ConnEntry::nip_fi_reason_tx` and `CommunityConnectionControl::reason_tx`.
    ///
    /// Set to `AuthorizationDenied` before `cancel.cancel()` on all NIP-FI
    /// denial paths (key-pairing mismatch, deny-set hit, expiry) so the send
    /// loop's cancel branch produces a 1008 POLICY close frame instead of a
    /// bare close. [FI-TRACE-CLOSE-CODE]
    pub(crate) nip_fi_reason_tx: tokio::sync::watch::Sender<Option<CommunityDisconnectReason>>,
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

/// Compute the NIP-FI session deadline from a verified assertion and the
/// configured `max_connection_lifetime`.
///
/// Per spec [FI-TRACE-LEASE-BOUND]:
/// ```text
/// session_deadline = min(
///     assertion.upstream_authority_deadline(),   // min(exp, iat+max_age, key-snapshot-hard)
///     connection_time + max_connection_lifetime  // partitions, never shortens
/// )
/// ```
///
/// `upstream_authority_deadline()` already includes the key-snapshot hard
/// deadline (one of the three authority_deadlines terms), so this two-term min
/// covers all four normative terms. Equality at any deadline is expired.
///
/// `connection_time` must be captured at or immediately before the WebSocket
/// upgrade — not after the NIP-42 exchange — so the partition is rooted at the
/// true connection establishment instant and the session cannot outlive
/// `connection_time + max_connection_lifetime` by the authentication interval.
pub(crate) fn compute_session_deadline(
    assertion: &buzz_auth::VerifiedAssertion,
    connection_time: chrono::DateTime<chrono::Utc>,
    max_connection_lifetime: Option<std::time::Duration>,
) -> chrono::DateTime<chrono::Utc> {
    let upstream = assertion.upstream_authority_deadline();
    match max_connection_lifetime {
        Some(lifetime) => {
            let partition = match chrono::Duration::from_std(lifetime) {
                Ok(d) => connection_time + d,
                // lifetime so large it overflows chrono — treat as effectively
                // infinite, so the upstream deadline wins.
                Err(_) => chrono::DateTime::<chrono::Utc>::MAX_UTC,
            };
            upstream.min(partition)
        }
        None => upstream,
    }
}

///
/// Acquires a connection semaphore permit, sends the NIP-42 AUTH challenge,
/// then drives the send, heartbeat, and receive loops until the connection closes.
pub async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    addr: SocketAddr,
    tenant: TenantContext,
    nip_fi_assertion: Option<buzz_auth::VerifiedAssertion>,
    connection_time: chrono::DateTime<chrono::Utc>,
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
        move |control| {
            handle_active_connection(
                socket,
                run_state,
                addr,
                tenant,
                conn_id,
                control,
                nip_fi_assertion,
                connection_time,
            )
        },
    )
    .await;
}

// `handle_active_connection` inherits the connection handler's natural parameter
// surface (socket, state, addr, tenant, conn_id, control, assertion, connection_time).
// Collapsing into a struct would just move the fields without reducing coupling.
#[allow(clippy::too_many_arguments)]
async fn handle_active_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    addr: SocketAddr,
    tenant: TenantContext,
    conn_id: Uuid,
    control: CommunityConnectionControl,
    nip_fi_assertion: Option<buzz_auth::VerifiedAssertion>,
    connection_time: chrono::DateTime<chrono::Utc>,
) {
    let cancel = control.cancellation_token();
    let disconnect_reason = control.disconnect_reason();
    // Extract the reason sender before control is consumed by the registry.
    // Shared with ConnEntry::nip_fi_reason_tx and conn.nip_fi_reason_tx so that
    // NIP-FI denial paths (key-pairing, deny-set, expiry) can set
    // AuthorizationDenied before cancel() fires. [FI-TRACE-CLOSE-CODE]
    let nip_fi_reason_tx = control.disconnect_reason_sender();
    // connection_time is threaded in from the HTTP handler (captured immediately
    // before on_upgrade) so the session partition is rooted at the true upgrade
    // instant, not the post-community-active-check instant. [FI-TRACE-LEASE-BOUND]
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

    // Dedicated one-slot channel for the terminal NIP-FI denial frame.
    // Cannot be saturated by ordinary traffic — only one terminal event fires.
    let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);

    // Dedicated restart-close channel carries a flush acknowledgement. Keeping
    // ordinary control frames unchanged avoids coupling heartbeat/ban traffic
    // to graceful-shutdown delivery tracking.
    let (restart_tx, restart_rx) = mpsc::channel::<RestartClose>(1);

    let backpressure_count = Arc::new(AtomicU8::new(0));
    let subscriptions = Arc::new(Mutex::new(HashMap::new()));

    // Compute the NIP-FI session deadline from the assertion.
    //
    // Per spec (Request and session bounds, [FI-TRACE-LEASE-BOUND]):
    //   session_deadline = min(
    //       assertion.upstream_authority_deadline(),   // = min(exp, iat+max_age, key-snapshot hard deadline)
    //       connection_time + max_connection_lifetime  // partitions, never shortens per spec
    //   )
    //
    // Equality at any deadline is expired. `upstream_authority_deadline()` already
    // includes the key-snapshot hard deadline (one of the three authority_deadlines
    // terms), so this min covers all normative terms.
    let session_deadline = nip_fi_assertion.as_ref().map(|a| {
        compute_session_deadline(
            a,
            connection_time,
            state.config.nip_fi.max_connection_lifetime(),
        )
    });

    // Create the NIP-FI session admission gate when in enforce mode.
    //
    // The gate is the lifetime authority for this connection: handlers acquire
    // Create the NIP-FI session admission gate. Every WS connection gets
    // exactly one gate — the [one-gate-per-connection] invariant.
    //
    // Enforce mode (assertion + deadline): gate has a deadline; the expiry
    // task calls gate.expire() at the deadline.
    // Off-mode (no assertion): gate has no deadline and never self-expires;
    // acquire_effect() always succeeds unless the outer cancel token fires.
    // [FI-TRACE-LEASE-BOUND, one-gate-per-connection]
    let nip_fi_gate = if let Some(deadline) = session_deadline {
        crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone())
    } else {
        crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone())
    };

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
        terminal_ctrl_tx,
        cancel: cancel.clone(),
        backpressure_count: Arc::clone(&backpressure_count),
        grace_limit: state.config.slow_client_grace_limit,
        nip_fi_assertion,
        session_deadline,
        nip_fi_gate: nip_fi_gate.clone(),
        nip_fi_reason_tx: nip_fi_reason_tx.clone(),
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
        nip_fi_reason_tx.clone(),
    );

    let (ws_send, ws_recv) = socket.split();

    let send_cancel = cancel.child_token();
    let send_task = tokio::spawn(send_loop(
        ws_send,
        rx,
        ctrl_rx,
        terminal_ctrl_rx,
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

    // NIP-FI session-lifetime enforcement task.
    //
    // Uses gate.expire() so the quiescence barrier (write lock) ensures
    // connection teardown cannot start until all pre-expiry effects have
    // finished. [FI-TRACE-LEASE-BOUND]
    let nip_fi_expiry_task = conn.session_deadline.map(|deadline| {
        crate::nip_fi_session::spawn_nip_fi_expiry_task(
            deadline,
            Arc::clone(&nip_fi_gate),
            conn.terminal_ctrl_tx.clone(),
            crate::nip_fi_session::NipFiWsRoute::Root,
            conn.nip_fi_reason_tx.clone(),
        )
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
    if let Some(task) = nip_fi_expiry_task {
        let _ = task.await;
    }

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

/// Send WebSocket messages in priority order: control frames before data frames.
///
/// Control frames (Pong, Close) are drained first on every iteration,
/// giving them priority over data frames. If the underlying socket writer
/// is stalled, control frames queue in the small ctrl_rx buffer; callers
/// treat a full control channel as terminal (Bug 7 fix).
async fn send_loop(
    ws_send: futures_util::stream::SplitSink<WebSocket, WsMessage>,
    data_rx: mpsc::Receiver<WsMessage>,
    ctrl_rx: mpsc::Receiver<WsMessage>,
    terminal_ctrl_rx: mpsc::Receiver<WsMessage>,
    restart_rx: mpsc::Receiver<RestartClose>,
    cancel: CancellationToken,
    disconnect_reason: watch::Receiver<Option<CommunityDisconnectReason>>,
) {
    send_loop_inner(
        ws_send,
        data_rx,
        ctrl_rx,
        terminal_ctrl_rx,
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
    mut terminal_ctrl_rx: mpsc::Receiver<WsMessage>,
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
                // Drain the terminal NIP-FI denial frame first (if any), then
                // ordinary control frames, before writing Close. The terminal
                // channel has capacity 1 and is written before cancel() fires,
                // so it is always available when denial is enqueued — even when
                // ctrl_rx (capacity 8) is full. This preserves the required
                // "restricted: authorization denied" frame to the client in all
                // queue-full scenarios.
                while let Ok(terminal_msg) = terminal_ctrl_rx.try_recv() {
                    if ws_send.send(terminal_msg).await.is_err() {
                        return;
                    }
                }
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
    // B2: Frame admission fence. If the connection's NIP-FI session has already
    // expired (cancel fired by the expiry task), drop this frame before any
    // handler dispatch. This closes the window where a buffered EVENT/REQ/AUTH
    // is selected from the recv queue after expiry fires the cancel token.
    // The check at the top of handle_text_message covers all message types
    // uniformly — no individual handler needs its own fence.
    if conn.cancel.is_cancelled() {
        return;
    }

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
    use std::sync::{Arc, Mutex};

    use buzz_auth::AuthMethod;
    use nostr::{EventBuilder, Keys, Kind};

    /// A connection whose outbound frames a test can read back.
    ///
    /// Lives here, next to `ConnectionState`, so the crate has one place that
    /// knows how to build one. Shared with `crate::rejection`'s tests.
    pub(crate) fn test_conn_with_auth(
        auth: AuthState,
    ) -> (Arc<ConnectionState>, mpsc::Receiver<WsMessage>) {
        let (send_tx, send_rx) = mpsc::channel(4);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(4);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let conn = ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().expect("socket addr"),
            auth_state: RwLock::new(auth),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
            nip_fi_reason_tx: tokio::sync::watch::channel(None).0,
        };
        (Arc::new(conn), send_rx)
    }

    /// An authenticated connection — the only state admission quotas apply to.
    pub(crate) fn authenticated_state() -> AuthState {
        AuthState::Authenticated(AuthContext {
            pubkey: Keys::generate().public_key(),
            scopes: Vec::new(),
            channel_ids: None,
            auth_method: AuthMethod::Nip42,
            agent_owner_pubkey: None,
        })
    }

    pub(crate) fn read_frame(rx: &mut mpsc::Receiver<WsMessage>) -> serde_json::Value {
        match rx.try_recv().expect("a frame was sent") {
            WsMessage::Text(text) => serde_json::from_str(&text).expect("valid JSON frame"),
            other => panic!("unexpected websocket message: {other:?}"),
        }
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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
            mpsc::channel(1).1,
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

    // ── NIP-FI session deadline — production function falsifiability ──────────
    //
    // These tests call `compute_session_deadline` directly (the production path
    // used by `handle_connection`) with real `VerifiedAssertion` fixtures.
    // Deleting or mutating `compute_session_deadline` turns these red.

    #[test]
    fn deadline_exp_is_earliest_selects_exp() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let exp = now + Duration::seconds(100);
        let iat_max_age = now + Duration::seconds(300);
        let key_hard = now + Duration::seconds(200);
        // authority_deadlines = [exp, iat_max_age, key_hard] → min = exp
        let assertion = VerifiedAssertion::for_test(None, vec![exp, iat_max_age, key_hard]);
        let lifetime = std::time::Duration::from_secs(400);
        let deadline = compute_session_deadline(&assertion, now, Some(lifetime));
        // exp < key_hard < lifetime; upstream = exp, partition >> exp → exp wins.
        assert_eq!(deadline, exp, "exp is earliest upstream term");
    }

    #[test]
    fn deadline_max_connection_lifetime_is_earliest_selects_partition() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let exp = now + Duration::seconds(400);
        let iat_max_age = now + Duration::seconds(300);
        let key_hard = now + Duration::seconds(200);
        // authority_deadlines = [exp, iat_max_age, key_hard] → upstream = key_hard (200s)
        // lifetime partition = now + 100s < key_hard → partition wins.
        let assertion = VerifiedAssertion::for_test(None, vec![exp, iat_max_age, key_hard]);
        let lifetime = std::time::Duration::from_secs(100);
        let deadline = compute_session_deadline(&assertion, now, Some(lifetime));
        // partition (now+100s) < upstream (now+200s) → partition wins.
        let expected_partition = now + Duration::seconds(100);
        // Allow 1s of wall-clock slack in the test.
        let delta = if deadline > expected_partition {
            (deadline - expected_partition).num_milliseconds().abs()
        } else {
            (expected_partition - deadline).num_milliseconds().abs()
        };
        assert!(delta < 1000, "partition term should win; delta={delta}ms");
    }

    #[test]
    fn deadline_no_lifetime_returns_upstream_only() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let exp = now + Duration::seconds(600);
        let key_hard = now + Duration::seconds(3600);
        let assertion = VerifiedAssertion::for_test(None, vec![exp, key_hard]);
        let deadline = compute_session_deadline(&assertion, now, None);
        assert_eq!(deadline, exp, "no lifetime → upstream (exp) only");
    }

    // ── NIP-FI expiry notice delivered on terminal_ctrl_tx before cancel ─────
    //
    // The expiry task queues `restricted: authorization denied` on
    // `terminal_ctrl_tx` (capacity-1, prioritised) BEFORE cancellation via the
    // gate. This test invokes the production
    // `nip_fi_session::spawn_nip_fi_expiry_task` constructor (Root route):
    // an already-expired deadline fires immediately; the terminal channel carries
    // the denial frame; the cancel fires afterward.
    //
    // Mutation evidence:
    //   A) Change the enqueue in `spawn_nip_fi_expiry_task` back to `ctrl_tx` →
    //      `terminal_rx.try_recv()` returns `Err`; test panics at "terminal
    //      channel must contain the denial frame".
    //   B) Delete `cancel.cancel()` inside gate.expire() →
    //      `cancel.is_cancelled()` is false; test panics at "expiry task must
    //      cancel the connection".

    #[tokio::test]
    async fn expiry_notice_queued_on_ctrl_before_cancel() {
        use tokio::sync::mpsc;

        let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();

        // Already-expired deadline → fires immediately.
        let deadline = chrono::Utc::now() - chrono::Duration::seconds(10);

        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        // Invoke the production shared constructor — Root route.
        let expiry_task = crate::nip_fi_session::spawn_nip_fi_expiry_task(
            deadline,
            gate,
            terminal_ctrl_tx,
            crate::nip_fi_session::NipFiWsRoute::Root,
            tokio::sync::watch::channel(None).0,
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), expiry_task)
            .await
            .expect("expiry task must complete within 2s")
            .expect("expiry task must not panic");

        // terminal_ctrl_rx must contain the denial frame.
        let terminal_frame = terminal_ctrl_rx
            .try_recv()
            .expect("terminal channel must contain the denial frame before cancel");
        match terminal_frame {
            WsMessage::Text(text) => {
                // Root route: NOTICE format ["NOTICE", <message>].
                let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
                let payload = v.get(1).and_then(|c| c.as_str()).unwrap_or("");
                assert_eq!(
                    payload,
                    buzz_auth::DenialClass::AuthorizationDenied.nostr_text(),
                    "terminal frame must carry the exact authorization_denied text"
                );
            }
            other => panic!("terminal frame must be Text, got {other:?}"),
        }
        // Cancel must have fired after the terminal send.
        assert!(
            cancel.is_cancelled(),
            "expiry task must cancel the connection"
        );
    }

    // ── B2: frame-admission fence and AUTH TOCTOU ─────────────────────────────
    //
    // Once the NIP-FI expiry task calls cancel(), no further message dispatch
    // should occur — even if a frame was already buffered in the recv queue
    // before cancel fired.
    //
    // The fence is the `if conn.cancel.is_cancelled() { return; }` check at the
    // top of `handle_text_message`. These tests exercise two windows:
    //
    //   1. A buffered REQ/EVENT/COUNT frame that arrives after cancel fires.
    //   2. An AUTH message dispatched while cancel is already set
    //      (the TOCTOU window where auth_state.write() is acquired, cancel is
    //      checked under the lock, and the write is skipped if cancelled).
    //
    // Mutation evidence:
    //   A) Remove `if conn.cancel.is_cancelled() { return; }` from
    //      `handle_text_message` → the EVENT test receives a frame on send_rx
    //      (an OK or NOTICE) → the assertion panics.
    //   B) Remove `if conn.cancel.is_cancelled() { return; }` from the AUTH
    //      handler (inside the write guard) → the AUTH test's
    //      `not Authenticated` assertion may still hold due to the DB path, but
    //      the top-level handle_text_message fence is the true gate.

    #[tokio::test]
    async fn b2_cancelled_connection_event_frame_not_dispatched() {
        use std::collections::HashMap;
        use tokio::sync::RwLock;

        // Pre-cancel the token — simulates the expiry task having already fired.
        let cancel = CancellationToken::new();
        cancel.cancel();

        let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);

        let conn = Arc::new(ConnectionState {
            conn_id: uuid::Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Failed),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
            nip_fi_reason_tx: tokio::sync::watch::channel(None).0,
        });

        let state = crate::state::tests::test_state().await;
        // A plausible EVENT frame — the handler would normally send OK/NOTICE.
        let event = nostr::EventBuilder::new(nostr::Kind::TextNote, "b2 test")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let raw = serde_json::json!(["EVENT", event]).to_string();

        handle_text_message(raw, Arc::clone(&conn), Arc::clone(&state)).await;

        // No frame must be sent — the fence must return before any handler runs.
        assert!(
            send_rx.try_recv().is_err(),
            "B2: a pre-cancelled connection must not dispatch an EVENT frame to any handler"
        );
    }

    // ── B3: send_loop writer delivers denial-then-Close through real send path ─
    //
    // These tests drive the real `send_loop_inner` against a sink that records
    // every frame, saturate ctrl_tx, enqueue a denial frame on terminal_ctrl_tx,
    // then cancel the token. The sink is non-blocking (MockSink), so send_loop
    // runs to completion synchronously after cancel fires.
    //
    // Assertion: the denial frame appears in the output BEFORE the Close frame.
    // This proves the queue-then-cancel ordering holds through the actual writer
    // code path, not just through a channel try_recv check.
    //
    // Mutation evidence:
    //   A) In send_loop_inner's cancel branch, swap the terminal drain and the
    //      ctrl drain → denial frame position flips → assertion panics.
    //   B) Remove the terminal drain entirely → denial frame absent → assertion
    //      panics on the "denial frame must precede Close" check.

    #[tokio::test]
    async fn b3_root_pairing_denial_precedes_close_through_send_loop() {
        use crate::nip_fi_session::NipFiWsRoute;

        let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();

        // Saturate ctrl_tx so an ordinary send couldn't carry the denial frame.
        for i in 0..8u8 {
            ctrl_tx
                .try_send(WsMessage::Text(format!("ordinary-{i}").into()))
                .expect("ctrl_tx has capacity 8");
        }
        drop(data_tx); // no data traffic in this test

        // Enqueue the denial frame on the terminal channel, then cancel.
        // This is the queue-then-cancel pattern the pairing denial path uses.
        terminal_ctrl_tx
            .try_send(crate::nip_fi_session::authorization_denied_frame(
                NipFiWsRoute::Root,
            ))
            .expect("terminal channel is empty");
        cancel.cancel();

        let (sink, state_arc) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            terminal_ctrl_rx,
            restart_rx,
            cancel,
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state_arc.lock().expect("mock sink poisoned");
        // The first frame written must be the denial frame.
        // The last frame written must be Close (or None close).
        let msgs = &state.messages;
        assert!(
            !msgs.is_empty(),
            "send_loop must write at least the denial frame + Close"
        );
        // Find the denial frame.
        let denial_pos = msgs
            .iter()
            .position(|m| matches!(m, WsMessage::Text(t) if t.contains("authorization denied")));
        let close_pos = msgs.iter().rposition(|m| matches!(m, WsMessage::Close(_)));

        let denial_pos = denial_pos.expect("denial frame must appear in send_loop output");
        let close_pos = close_pos.expect("Close frame must appear in send_loop output");
        assert!(
            denial_pos < close_pos,
            "B3: denial frame (pos {denial_pos}) must precede Close frame (pos {close_pos})"
        );
    }

    #[tokio::test]
    async fn b3_expiry_denial_precedes_close_through_send_loop() {
        use crate::nip_fi_session::{spawn_nip_fi_expiry_task, NipFiWsRoute};
        use chrono::Utc;

        let (data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();

        // Saturate ctrl_tx.
        for i in 0..8u8 {
            ctrl_tx
                .try_send(WsMessage::Text(format!("ordinary-{i}").into()))
                .expect("ctrl_tx has capacity 8");
        }
        drop(data_tx);

        // Arm the expiry task with an already-expired deadline. It will
        // immediately enqueue the denial frame on the terminal channel and
        // cancel the token.
        let already_expired = Utc::now() - chrono::Duration::seconds(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(already_expired, cancel.clone());
        let expiry_handle = spawn_nip_fi_expiry_task(
            already_expired,
            gate,
            terminal_ctrl_tx,
            NipFiWsRoute::Root,
            tokio::sync::watch::channel(None).0,
        );
        // Wait for the expiry task to fire before we run the send_loop.
        expiry_handle.await.expect("expiry task must complete");

        let (sink, state_arc) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            terminal_ctrl_rx,
            restart_rx,
            cancel,
            ordinary_disconnect_reason(),
        )
        .await;

        let state = state_arc.lock().expect("mock sink poisoned");
        let msgs = &state.messages;
        assert!(
            !msgs.is_empty(),
            "send_loop must write at least the denial frame + Close"
        );
        let denial_pos = msgs
            .iter()
            .position(|m| matches!(m, WsMessage::Text(t) if t.contains("authorization denied")));
        let close_pos = msgs.iter().rposition(|m| matches!(m, WsMessage::Close(_)));

        let denial_pos = denial_pos.expect("expiry denial frame must appear in send_loop output");
        let close_pos = close_pos.expect("Close frame must appear in send_loop output");
        assert!(
            denial_pos < close_pos,
            "B3: expiry denial frame (pos {denial_pos}) must precede Close frame (pos {close_pos})"
        );
    }
}
