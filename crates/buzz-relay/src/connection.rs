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

    /// Shared transition lock for all terminal writers on this connection
    /// (root key-pairing, expiry, community deletion, auth deny-set hit,
    /// and the connection manager's close scan).  All terminal writers go
    /// through `CommunityConnectionControl` methods so no independently
    /// writable reason-sender clone lives outside the primitive.
    /// [FI-TRACE-CLOSE-CODE, FI-TRACE-CANCEL-RACE]
    pub(crate) community_control: crate::state::CommunityConnectionControl,
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
                    self.community_control.lifecycle_cancel();
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
            // Use lifecycle_cancel (holds the transition lock) so a concurrent
            // terminal writer that has won the reason but not yet enqueued its
            // denial frame completes try_send before the cancel wakes the
            // consumer.  [FI-TRACE-CANCEL-RACE, B2 fix]
            self.conn.community_control.lifecycle_cancel();
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
        auth_state: StdMutex::new(AuthState::Pending {
            challenge: challenge.clone(),
            started_at: Instant::now(),
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
        community_control: control.clone(),
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
        conn.terminal_ctrl_tx.clone(),
        Some(restart_tx),
        cancel.clone(),
        conn.tenant.community(),
        Arc::clone(&backpressure_count),
        subscriptions,
        state.config.slow_client_grace_limit,
        control.clone(),
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
    let heartbeat_task = tokio::spawn(heartbeat_loop(
        ctrl_tx,
        Arc::clone(&missed_pongs),
        control.clone(),
    ));

    let auth_timeout_conn = Arc::clone(&conn);
    let auth_timeout_cancel = cancel.clone();
    let auth_timeout_control = control.clone();
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
                    auth_timeout_control.lifecycle_cancel();
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
            control.clone(),
        )
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

    control.lifecycle_cancel();
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
///
/// Drain order: terminal_ctrl_rx (NIP-FI denial frame) → first_ctrl (the
/// in-flight ordinary control frame, if any) → ctrl_rx (remaining ordinary
/// control frames) → Close.  The terminal channel must be drained first so a
/// queued denial frame is delivered even when the ordinary control channel
/// (capacity 8) is saturated with Pings/Pongs.  [FI-INV-05, B1 fix]
async fn flush_terminal_frames<S>(
    sink: &mut S,
    terminal_ctrl_rx: &mut mpsc::Receiver<WsMessage>,
    ctrl_rx: &mut mpsc::Receiver<WsMessage>,
    disconnect_reason: &watch::Receiver<Option<CommunityDisconnectReason>>,
    first_ctrl: Option<WsMessage>,
) where
    S: Sink<WsMessage> + Unpin,
{
    let deadline = tokio::time::Instant::now() + WS_TERMINAL_FLUSH_TIMEOUT;
    // Terminal channel first — ensures denial frame precedes ordinary control
    // and Close even when ctrl_rx is saturated.
    while let Ok(terminal_msg) = terminal_ctrl_rx.try_recv() {
        if !matches!(
            tokio::time::timeout_at(deadline, sink.send(terminal_msg)).await,
            Ok(Ok(()))
        ) {
            return;
        }
    }
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
            match send_or_cancel(&mut ws_send, ctrl_msg.clone(), &cancel).await {
                WriterStep::Completed => {}
                WriterStep::Cancelled => {
                    // Cancelled mid-top-of-loop drain: drain terminal first, then
                    // the already-taken ctrl_msg, then remaining ctrl_rx, then Close.
                    flush_terminal_frames(
                        &mut ws_send,
                        &mut terminal_ctrl_rx,
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
                // R1: an already-winning NIP-FI denial must be delivered before
                // (and instead of) the 1012 restart close.  If a NIP-FI denial
                // has won the reason slot (disconnect_reason = AuthorizationDenied),
                // wait for its frame to be enqueued — the reason is set before
                // try_send under the transition lock, so we may see the reason
                // before the frame arrives.  Only when no denial reason is set
                // do we send the 1012 as before.  [FI-INV-05, R1 fix]
                let deadline = tokio::time::Instant::now() + WS_TERMINAL_FLUSH_TIMEOUT;
                let denial_reason = *disconnect_reason.borrow();
                // Attempt to receive the denial frame.  If reason is set,
                // wait up to the flush deadline for the enqueue to complete
                // (tiny window between reason publication and try_send).
                // If reason is unset, try_recv immediately (empty → no denial).
                let maybe_frame = if matches!(
                    denial_reason,
                    Some(CommunityDisconnectReason::AuthorizationDenied)
                ) {
                    // Reason won — wait for the frame with a bounded deadline.
                    // In the common case it is already present; in the race
                    // window it arrives within microseconds.
                    tokio::time::timeout_at(deadline, terminal_ctrl_rx.recv())
                        .await
                        .ok()
                        .flatten()
                } else {
                    terminal_ctrl_rx.try_recv().ok()
                };
                if let Some(denial_frame) = maybe_frame {
                    // A denial already won: deliver it and its close code ahead
                    // of the restart 1012.  The restart close is not sent
                    // (flushed=false) — the denial reason takes precedence.
                    if tokio::time::timeout_at(deadline, ws_send.send(denial_frame))
                        .await
                        .is_ok_and(|r| r.is_ok())
                    {
                        let close = disconnect_reason
                            .borrow()
                            .map_or(WsMessage::Close(None), |reason| reason.close_message());
                        let _ = tokio::time::timeout_at(deadline, ws_send.send(close)).await;
                    }
                    let _ = restart.flushed.send(false);
                } else {
                    let sent = matches!(
                        tokio::time::timeout_at(
                            deadline,
                            ws_send.send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                            code: axum::extract::ws::close_code::RESTART,
                            reason: axum::extract::ws::Utf8Bytes::from_static("relay restarting"),
                            }))),
                        ).await,
                        Ok(Ok(()))
                    );
                    let _ = restart.flushed.send(sent);
                }
                break;
            }
            _ = cancel.cancelled() => {
                // Drain the terminal NIP-FI denial frame first (if any), then
                // ordinary control frames, before writing Close.  The shared
                // flush_terminal_frames helper applies a single bounded deadline
                // to all three queues so a stalled socket cannot retain the
                // writer task indefinitely.  [FI-INV-05, B1 fix]
                flush_terminal_frames(
                    &mut ws_send,
                    &mut terminal_ctrl_rx,
                    &mut ctrl_rx,
                    &disconnect_reason,
                    None,
                )
                .await;
                break;
            }
            Some(ctrl_msg) = ctrl_rx.recv() => {
                match send_or_cancel(&mut ws_send, ctrl_msg.clone(), &cancel).await {
                    WriterStep::Completed => {}
                    WriterStep::Cancelled => {
                        flush_terminal_frames(
                            &mut ws_send,
                            &mut terminal_ctrl_rx,
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
                            &mut terminal_ctrl_rx,
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
                                        &mut terminal_ctrl_rx,
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
                            &mut terminal_ctrl_rx,
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
    control: CommunityConnectionControl,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        let cancelled = control.cancellation_token();
        tokio::select! {
            _ = interval.tick() => {
                // fetch_add returns the *previous* value before incrementing:
                //   prev=0 → now 1 (first miss)
                //   prev=1 → now 2 (second miss)
                //   prev=2 → now 3 (third miss → disconnect)
                let missed = missed_pongs.fetch_add(1, Ordering::Relaxed);
                if missed >= 2 {
                    warn!("3 missed pongs — closing connection");
                    control.lifecycle_cancel();
                    break;
                }
                if ctrl_tx.try_send(WsMessage::Ping(axum::body::Bytes::new())).is_err() {
                    warn!("control channel full — cannot send Ping, closing");
                    control.lifecycle_cancel();
                    break;
                }
            }
            _ = cancelled.cancelled() => break,
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
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
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
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
            community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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
                            None,
                            chrono::Utc::now(),
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
        let (_terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel(1);
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
            terminal_ctrl_rx,
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
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());
        let expiry_task = crate::nip_fi_session::spawn_nip_fi_expiry_task(
            deadline,
            gate,
            terminal_ctrl_tx,
            crate::nip_fi_session::NipFiWsRoute::Root,
            control,
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
            auth_state: std::sync::Mutex::new(AuthState::Failed),
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
            community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());
        let expiry_handle = spawn_nip_fi_expiry_task(
            already_expired,
            gate,
            terminal_ctrl_tx,
            NipFiWsRoute::Root,
            control,
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

    // ── R1 witnesses: denial wins over concurrent restart. ───────────────────
    //
    // Two complementary schedules that together prove the R1 fix:
    //
    // 1. b3_denial_precedes_restart_when_denial_already_won (below):
    //    Frame already on terminal_ctrl_rx when restart arm fires.  Reason is
    //    set; recv() returns immediately with the existing frame.
    //
    // 2. r1_denial_precedes_restart_reason_won_frame_arrives_during_recv (below):
    //    Reason set (AuthorizationDenied) but terminal_ctrl_rx is EMPTY when the
    //    restart arm checks.  The arm calls bounded recv(); a concurrent task
    //    enqueues the frame while recv() is waiting.  This covers the race window
    //    documented at state.rs:295-310 where reason is published under the
    //    transition lock before try_send.
    //
    // Mutation evidence (applies to both):
    //   A) Remove the `disconnect_reason` watch check + recv() from the restart
    //      arm → restarts always send 1012 even when denial won → first assertion
    //      panics (denial frame missing or flushed=true).
    //   B) Keep reason check but use try_recv() only (no bounded recv()) → race
    //      schedule 2 returns Err(Empty) → 1012 sent instead of denial → panics.

    #[tokio::test]
    async fn b3_denial_precedes_restart_when_denial_already_won() {
        use crate::nip_fi_session::NipFiWsRoute;
        use tokio::sync::mpsc;

        let (_data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let (restart_tx, restart_rx) = mpsc::channel(1);
        let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel::<bool>();
        let cancel = CancellationToken::new();

        // Enqueue the denial frame on the terminal channel.
        let denial = crate::nip_fi_session::authorization_denied_frame(NipFiWsRoute::Root);
        terminal_ctrl_tx
            .try_send(denial.clone())
            .expect("terminal channel is empty");

        // Queue a restart command — the biased select will fire restart_rx first.
        restart_tx
            .send(RestartClose {
                flushed: flushed_tx,
            })
            .await
            .expect("queue restart close");

        // Set the disconnect reason so denial close has a non-None close frame.
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());
        control.manager_disconnect_nip_fi(
            &terminal_ctrl_tx, // already consumed — this is a no-op try_send
        );
        let disconnect_reason = control.disconnect_reason();

        let (sink, state_arc) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            terminal_ctrl_rx,
            restart_rx,
            cancel,
            disconnect_reason,
        )
        .await;

        // restart.flushed must be false — the restart 1012 was not sent.
        assert_eq!(
            flushed_rx.await,
            Ok(false),
            "R1: restart.flushed must be false when denial already won (1012 not sent)"
        );

        let state = state_arc.lock().expect("mock sink poisoned");
        let msgs = &state.messages;
        assert!(
            !msgs.is_empty(),
            "R1: send_loop must write at least one frame when denial is queued"
        );
        // First frame must be the denial frame, not a 1012 close.
        assert!(
            matches!(&msgs[0], WsMessage::Text(t) if t.contains("authorization denied")),
            "R1: first frame must be the denial frame, not 1012 — \
             got {:?}. \
             Mutation: remove terminal_ctrl_rx check from restart arm → 1012 sent instead → panics.",
            msgs.first()
        );
        // Last frame must be a Close (the denial close code, not 1012).
        let last = msgs.last().expect("at least one frame");
        match last {
            WsMessage::Close(Some(close)) => {
                assert_ne!(
                    close.code,
                    axum::extract::ws::close_code::RESTART,
                    "R1: close code must not be 1012 (restart) when denial won"
                );
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::POLICY,
                    "R1: close code must be 1008 (POLICY) when denial won; got {}. \
                     Mutation: remove disconnect_reason.borrow() close_message() call → \
                     wrong close code → panics.",
                    close.code
                );
            }
            WsMessage::Close(None) => {
                panic!(
                    "R1: last frame must be Close(Some(1008 POLICY)), got Close(None) — \
                     denial close code must be present when denial won the reason slot"
                );
            }
            other => panic!("R1: last frame must be Close, got {other:?}"),
        }
    }

    // ── R1 witness 2: reason won but frame not yet enqueued (recv path) ───────
    //
    // This schedule exercises the specific race documented at state.rs:295-310:
    // `disconnect_reason` is set to AuthorizationDenied under the transition
    // lock BEFORE `try_send` completes.  The restart arm may see the reason
    // (AuthorizationDenied) but find terminal_ctrl_rx empty.  The R1 fix calls
    // bounded `recv()` in this case; the frame arrives while recv() is waiting.
    //
    // Setup:
    //   1. disconnect_reason set to AuthorizationDenied (reason won).
    //   2. terminal_ctrl_rx is EMPTY — frame not yet enqueued.
    //   3. restart command queued — biased select fires restart arm.
    //   4. Concurrently (after send_loop_inner starts), enqueue the denial frame.
    //
    // Mutation evidence:
    //   A) Replace recv() with try_recv() in the reason-won branch →
    //      try_recv() returns Err(Empty) → 1012 sent → flushed=true →
    //      flushed_rx assertion panics.
    //   B) Remove the disconnect_reason check entirely → always try_recv() →
    //      same result as A.
    #[tokio::test]
    async fn r1_denial_precedes_restart_reason_won_frame_arrives_during_recv() {
        use crate::nip_fi_session::NipFiWsRoute;
        use tokio::sync::mpsc;

        let (_data_tx, data_rx) = mpsc::channel::<WsMessage>(16);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let (restart_tx, restart_rx) = mpsc::channel(1);
        let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel::<bool>();
        let cancel = CancellationToken::new();

        // Set reason = AuthorizationDenied BUT do NOT enqueue the denial frame
        // yet.  We simulate the window between reason publication and try_send:
        // use a raw watch channel to publish the reason without going through
        // the full CommunityConnectionControl path (which would also try_send).
        let (reason_tx, reason_rx) =
            tokio::sync::watch::channel(Some(CommunityDisconnectReason::AuthorizationDenied));
        drop(reason_tx); // keep rx live; reason is already set

        // Queue the restart command — the biased select will fire restart_rx.
        restart_tx
            .send(RestartClose {
                flushed: flushed_tx,
            })
            .await
            .expect("queue restart");

        // Spawn a task that enqueues the denial frame after a brief yield,
        // simulating try_send completing while the restart arm's recv() waits.
        let denial = crate::nip_fi_session::authorization_denied_frame(NipFiWsRoute::Root);
        let terminal_ctrl_tx_for_sender = terminal_ctrl_tx.clone();
        let denial_for_sender = denial.clone();
        tokio::spawn(async move {
            // Yield to allow send_loop_inner to enter the restart arm and start
            // its bounded recv() before the frame is available.
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            terminal_ctrl_tx_for_sender
                .send(denial_for_sender)
                .await
                .expect("enqueue denial frame during recv wait");
        });

        let (sink, state_arc) = MockSink::new(None);
        send_loop_inner(
            sink,
            data_rx,
            ctrl_rx,
            terminal_ctrl_rx,
            restart_rx,
            cancel,
            reason_rx,
        )
        .await;

        // flushed must be false — denial won, restart 1012 not sent.
        assert_eq!(
            flushed_rx.await,
            Ok(false),
            "R1: restart.flushed must be false when denial reason is set \
             (1012 must not be sent even when frame arrives during recv)"
        );

        let state = state_arc.lock().expect("mock sink poisoned");
        let msgs = &state.messages;
        assert!(
            !msgs.is_empty(),
            "R1: send_loop must write at least one frame when denial reason is set"
        );
        // First frame must be the denial TEXT frame, not 1012.
        assert!(
            matches!(msgs.first(), Some(WsMessage::Text(t)) if t.contains("authorization denied")),
            "R1: first frame must be the denial frame, not 1012 — got {:?}. \
             Mutation: use try_recv() instead of recv() → Err(Empty) → 1012 sent instead.",
            msgs.first()
        );
        // Last frame must be Close(1008 POLICY).
        let last = msgs.last().expect("at least one frame");
        match last {
            WsMessage::Close(Some(close)) => {
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::POLICY,
                    "R1: close code must be 1008 POLICY when denial reason won; got {}",
                    close.code
                );
            }
            WsMessage::Close(None) => {
                panic!("R1: last frame must be Close(Some(1008 POLICY)), got Close(None)");
            }
            other => panic!("R1: last frame must be Close, got {other:?}"),
        }
    }

    // ── F5 loopback teardown: real connection epilogue removes subs + topic refcounts ──
    //
    // Proves the complete F5 teardown boundary using the production
    // `handle_active_connection` call path and real `ConnectionManager`/`sub_registry`/
    // `pubsub` infrastructure — no synthetic setup.
    //
    // Sequence:
    //   1. Build test state (no Postgres needed for REQ; auth uses DB for moderation,
    //      so this test runs in CI where DATABASE_URL is set).
    //   2. Start a plain loopback WS server calling `handle_active_connection`.
    //   3. Connect + complete NIP-42 auth (registers pubkey in conn_manager).
    //      Auth OK check verifies success flag (v[2] == true) to reject ban/internal errors.
    //   4. Send a REQ ["REQ", "sub1", {"kinds":[13534]}].
    //      Kind 13534 (NIP-43 membership list) hits `filters_are_nip43_membership_only`
    //      → skips DB accessible-channels lookup → passes p_gated/engram/author-only gates
    //      → is not huddle-liveness or search → reaches `acquire_effect()` and
    //      the `after_req_permit_acquired` hook (permit is held).
    //   5. Call `state.conn_manager.disconnect_nip_fi(pubkey)` from the test.
    //      This: wins reason → enqueues denial frame → fires cancel.
    //      The expiry task is now in quiescence (write lock blocked by permit).
    //   6. Release hook → handler proceeds: registers subscription in sub_registry,
    //      retains Global topic in pubsub, finishes REQ, drops permit.
    //   7. Quiescence unblocks → expiry task completes.
    //   8. recv_loop detects cancel → sends denial frame + POLICY close →
    //      connection epilogue:
    //        nip_fi_expiry_task.await (already done)
    //        sub_registry.remove_connection → 0 subscriptions
    //        pubsub.release_topic(Global) → 0 topic refcounts
    //   9. Observe exactly one canonical denial frame then the POLICY close on the WS.
    //  10. Wait for connection_finished.
    //  11. Assert: total_subscriptions() == 0 and topic_refcount(Global) == 0
    //      and topic_refcount(Channel) == 0.
    //
    // Mutation evidence:
    //   A) Remove `gate.quiesce().await` from expiry task cancel arm →
    //      task exits before REQ registers → remove_connection finds nothing →
    //      sub orphans after hook release → total_subscriptions() stays 1 at step 11
    //      if the connection_finished fires before remove_connection (hard race).
    //      More reliably: `task_handle.is_finished()` check added below panics.
    //   B) Remove `sub_registry.remove_connection` from connection epilogue →
    //      sub never removed → total_subscriptions() stays 1 → assertion panics.
    //   C) Remove `pubsub.release_topic` from connection epilogue →
    //      topic refcount stays 1 → assertion panics.
    //   D) Delete `after_req_permit_acquired(...)` from req.rs →
    //      arrived_rx times out → test panics (proves hook is at correct seam).
    //
    // Runs in the PG lane (`postgres_tests` wrapper below): auth's moderation
    // check needs Postgres and the admission gate needs Redis.
    async fn f5_loopback_teardown_epilogue_removes_subscription_and_topic_refcount_body() {
        use axum::{extract::ws::WebSocketUpgrade, routing::get, Router};
        use buzz_auth::VerifiedAssertion;
        use buzz_pubsub::EventTopic;
        use chrono::{Duration, Utc};
        use nostr::{EventBuilder, Keys, RelayUrl};
        use tokio::net::TcpListener;
        use tokio::sync::Notify;
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        use uuid::Uuid;

        // F5 requires Postgres (auth's moderation check) and Redis (the
        // recv_loop admission gate; the default test state's redis://127.0.0.1:1
        // would close REQ as "shared admission unavailable"). Both come from the
        // PG lane's environment and a missing or unreachable one fails the test.
        let db_url = crate::test_support::required_database_url();
        let pool = sqlx::PgPool::connect(&db_url)
            .await
            .unwrap_or_else(|e| panic!("F5: wrapper DB {db_url} unreachable: {e}"));
        let redis_url = crate::test_support::required_redis_url();
        let state_arc = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        let redis_pool = deadpool_redis::Config::from_url(&redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap_or_else(|e| panic!("F5: invalid Redis URL {redis_url}: {e}"));
        if let Err(e) = redis_pool.get().await {
            panic!("F5: Redis at {redis_url} unreachable: {e}");
        }
        let mut state_mut = (*state_arc).clone();
        state_mut.admission_rate_limiter =
            std::sync::Arc::new(buzz_pubsub::rate_limiter::RedisRateLimiter::new(redis_pool));
        let state = std::sync::Arc::new(state_mut);
        // Use a fresh non-nil UUID for the F5 test community.
        // The DB has a `chk_communities_id_not_nil` constraint that rejects Uuid::nil(),
        // and a unique constraint on `lower(host)` that requires each community to have a
        // distinct host.  Derive the host from the UUID for per-test-run uniqueness.
        let f5_community_uuid = Uuid::new_v4();
        let f5_community_host = format!("f5-test-{}.local", f5_community_uuid);
        let tenant = TenantContext::resolved(
            buzz_core::tenant::CommunityId::from_uuid(f5_community_uuid),
            f5_community_host.clone(),
        );
        // Far-future deadline so the gate is live but does NOT self-expire.
        // disconnect_nip_fi provides the deny path; the expiry task enters
        // quiescence when cancel fires.
        let member_keys = Keys::generate();
        let assertion = VerifiedAssertion::for_test(
            Some(member_keys.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );
        let pubkey_bytes = member_keys.public_key().to_bytes().to_vec();
        let community_id = tenant.community();

        let expected_relay_url: RelayUrl =
            crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &tenant)
                .parse()
                .expect("F5: expected NIP-42 relay URL");

        let connection_finished = Arc::new(Notify::new());

        let route_state = Arc::clone(&state);
        let route_tenant = tenant.clone();
        let route_finished = Arc::clone(&connection_finished);
        let assertion_c = assertion.clone();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("F5: bind test listener");
        let addr = listener.local_addr().expect("F5: listener addr");

        let server = tokio::spawn(async move {
            let app = Router::new().route(
                "/",
                get(move |ws: WebSocketUpgrade| {
                    let state = Arc::clone(&route_state);
                    let tenant = route_tenant.clone();
                    let finished = Arc::clone(&route_finished);
                    let assertion = assertion_c.clone();
                    async move {
                        ws.on_upgrade(move |socket| async move {
                            let cancel = CancellationToken::new();
                            let control = CommunityConnectionControl::new(cancel);
                            handle_active_connection(
                                socket,
                                state,
                                "127.0.0.1:1234".parse().expect("client addr"),
                                tenant,
                                Uuid::new_v4(),
                                control,
                                Some(assertion),
                                chrono::Utc::now(),
                            )
                            .await;
                            finished.notify_one();
                        })
                    }
                }),
            );
            axum::serve(listener, app)
                .await
                .expect("F5: serve lifecycle WS");
        });

        let (mut client, _) = connect_async(format!("ws://{addr}/"))
            .await
            .expect("F5: connect client");

        // ── NIP-42 auth exchange ──────────────────────────────────────────
        let challenge_frame =
            tokio::time::timeout(std::time::Duration::from_secs(2), client.next())
                .await
                .expect("F5: challenge timeout")
                .expect("F5: challenge item")
                .expect("F5: challenge message");
        let challenge_text = match challenge_frame {
            Message::Text(t) => t.to_string(),
            other => panic!("F5: expected text challenge; got {other:?}"),
        };
        let challenge_json: serde_json::Value =
            serde_json::from_str(&challenge_text).expect("F5: challenge JSON");
        assert_eq!(challenge_json[0], "AUTH", "F5: expected AUTH message");
        let challenge = challenge_json[1].as_str().expect("F5: challenge field");

        let auth_event = EventBuilder::auth(challenge, expected_relay_url)
            .sign_with_keys(&member_keys)
            .expect("F5: sign NIP-42 AUTH");
        client
            .send(Message::Text(
                serde_json::json!(["AUTH", auth_event]).to_string().into(),
            ))
            .await
            .expect("F5: send auth");

        // Drain OK message and any following messages until connection confirms
        // auth (OK ["OK", event_id, true, ""] message). Allow up to 3s for auth.
        // We check v[2] == true to reject OK(false) from ban/internal failures.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let frame = client
                    .next()
                    .await
                    .expect("F5: auth response item")
                    .expect("F5: auth response message");
                if let Message::Text(t) = &frame {
                    let v: serde_json::Value = serde_json::from_str(t).unwrap_or_default();
                    if v[0] == "OK" && v[2] == true {
                        break;
                    }
                }
            }
        })
        .await
        .expect(
            "F5: auth OK (success) timeout — DB required for auth (run in CI with DATABASE_URL)",
        );

        // ── Seed a channel + membership for the channel-topic assertion ───
        //
        // A channel-scoped REQ retains EventTopic::Channel; the channel-topic
        // release at connection.rs:638-642 is the production cleanup we bind.
        // RED: delete connection.rs:638-642 → channel topic stays at 1 after
        // teardown → the channel_refcount_after assertion below panics.
        //
        // The global sub (sub1) tests the Global topic path.
        // The channel sub (sub_channel) tests the Channel topic path.
        //
        // Seed: fresh community (non-nil UUID, per chk_communities_id_not_nil constraint)
        // + a fresh channel + membership for member_keys so accessibility check passes.
        let f5_channel_id = uuid::Uuid::new_v4();
        let creator_bytes = member_keys.public_key().to_bytes().to_vec();
        // Insert the test community (non-nil UUID; ON CONFLICT DO NOTHING for idempotency).
        sqlx::query(
            "INSERT INTO communities (id, host) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(f5_community_uuid)
        .bind(&f5_community_host)
        .execute(&pool)
        .await
        .expect("F5: seed test community");
        // Insert a fresh stream channel under the test community.
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, 'f5-channel-topic', 'stream', 'open', $3) \
             ON CONFLICT (community_id, id) DO NOTHING",
        )
        .bind(f5_channel_id)
        .bind(f5_community_uuid)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("F5: seed test channel");
        // Insert member_keys pubkey as a channel member.
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(f5_community_uuid)
        .bind(f5_channel_id)
        .bind(&creator_bytes)
        .execute(&pool)
        .await
        .expect("F5: seed channel membership");

        // ── Send channel-scoped REQ: registers sub + retains Channel topic ─
        //
        // This REQ completes synchronously (no hook, no permit race) and returns
        // EOSE. After EOSE, sub_channel is registered and Channel topic refcount = 1.
        // The hook fires only for the NEXT REQ (sub1).
        client
            .send(Message::Text(
                serde_json::json!(["REQ", "sub_channel", {"#h": [f5_channel_id.to_string()]}])
                    .to_string()
                    .into(),
            ))
            .await
            .expect("F5: send channel REQ");

        // Wait for EOSE from sub_channel (confirms sub registered + channel topic retained).
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let frame = client
                    .next()
                    .await
                    .expect("F5: channel EOSE item")
                    .expect("F5: channel EOSE message");
                if let Message::Text(t) = &frame {
                    let v: serde_json::Value = serde_json::from_str(t).unwrap_or_default();
                    if v[0] == "EOSE" && v[1] == "sub_channel" {
                        break;
                    }
                }
            }
        })
        .await
        .expect("F5: channel sub EOSE timeout — channel REQ must complete and send EOSE");

        // Verify channel topic is held (refcount = 1) before disconnect.
        // This establishes the positive precondition: teardown must release it.
        let channel_refcount_before = state
            .pubsub
            .topic_refcount(&tenant, EventTopic::Channel(f5_channel_id))
            .await;
        assert_eq!(
            channel_refcount_before, 1,
            "F5: channel topic refcount must be 1 after channel REQ registration"
        );

        // ── Global REQ completes first: positive Global precondition ─────
        //
        // sub_global registers a global subscription and retains
        // EventTopic::Global before any disconnect, so the post-teardown
        // zero is a real 1 → 0 transition.
        // RED (b): delete the Global release_topic in the connection epilogue
        // (connection.rs:632-637) → global refcount stays 1 → zero assert panics.
        client
            .send(Message::Text(
                serde_json::json!(["REQ", "sub_global", {"kinds": [13534]}])
                    .to_string()
                    .into(),
            ))
            .await
            .expect("F5: send global REQ");
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let frame = client
                    .next()
                    .await
                    .expect("F5: global EOSE item")
                    .expect("F5: global EOSE message");
                if let Message::Text(t) = &frame {
                    let v: serde_json::Value = serde_json::from_str(t).unwrap_or_default();
                    if v[0] == "EOSE" && v[1] == "sub_global" {
                        break;
                    }
                }
            }
        })
        .await
        .expect("F5: global sub EOSE timeout");
        assert_eq!(
            state
                .pubsub
                .topic_refcount(&tenant, EventTopic::Global)
                .await,
            1,
            "F5: Global topic refcount must be 1 after sub_global registration"
        );
        assert_eq!(
            state.sub_registry.total_subscriptions(),
            2,
            "F5: sub_channel + sub_global must both be registered before disconnect"
        );

        // ── Separate racing REQ: hold its effect permit ───────────────────
        let (arrived_rx, hook_release) =
            crate::nip_fi_test_hooks::req_permit_acquired_hook::arm(community_id);
        client
            .send(Message::Text(
                serde_json::json!(["REQ", "sub_race", {"kinds": [13534]}])
                    .to_string()
                    .into(),
            ))
            .await
            .expect("F5: send racing REQ");
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect(
                "F5: racing REQ must reach after_req_permit_acquired within 5s \
                 (permit held). Mutation D: delete the hook call → times out.",
            )
            .expect("F5: hook arrived channel closed");

        // ── Admin disconnect: real production path ────────────────────────
        let closed = state
            .conn_manager
            .disconnect_nip_fi("test-issuer", &pubkey_bytes);
        assert_eq!(
            closed, 1,
            "F5: conn_manager.disconnect_nip_fi must find exactly 1 connection"
        );

        // ── Terminal delivery WHILE the racing permit is still held ───────
        let denial_frame = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .expect("F5: denial frame timeout")
            .expect("F5: denial frame item")
            .expect("F5: denial frame message");
        let expected_denial = crate::protocol::RelayMessage::notice(
            buzz_auth::DenialClass::AuthorizationDenied.nostr_text(),
        );
        match &denial_frame {
            Message::Text(t) => assert_eq!(
                t.as_str(),
                expected_denial.as_str(),
                "F5: denial frame must be the canonical Root NOTICE denial frame"
            ),
            other => panic!("F5: expected Text denial frame; got {other:?}"),
        }
        let close_frame = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
            .await
            .expect("F5: close frame timeout")
            .expect("F5: close frame item")
            .expect("F5: close frame message");
        match &close_frame {
            Message::Close(Some(cf)) => assert_eq!(
                cf.code,
                tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                "F5: close code must be 1008 POLICY; got {:?}",
                cf.code
            ),
            other => panic!("F5: expected Close(Some(1008)); got {other:?}"),
        }

        // ── Blocked-epilogue assert (permit still held) ───────────────────
        //
        // Terminal delivery is complete, but the expiry task's cancel arm is
        // parked in gate.quiesce() on the held permit, and the epilogue awaits
        // that task before remove_connection.  Teardown must not have run.
        // RED (a): delete `gate.quiesce().await` at nip_fi_session.rs:272 →
        // epilogue runs while the permit is held → connection_finished fires
        // (and registrations are removed) → this assertion panics.
        let finished_early = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            connection_finished.notified(),
        )
        .await;
        assert!(
            finished_early.is_err(),
            "F5: connection teardown epilogue must stay blocked while the racing REQ \
             permit is held (quiescence barrier)"
        );
        assert_eq!(
            state.sub_registry.total_subscriptions(),
            2,
            "F5: epilogue must not have removed subscriptions while the permit is held"
        );

        // ── Release the permit → REQ registers → quiescence → teardown ───
        hook_release.notify_one();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            connection_finished.notified(),
        )
        .await
        .expect("F5: connection must finish within 10s after the permit is released");
        // ── Assert zero orphan subscriptions ─────────────────────────────
        assert_eq!(
            state.sub_registry.total_subscriptions(),
            0,
            "F5: sub_registry must have zero subscriptions after connection teardown. \
             Mutation B: remove sub_registry.remove_connection from epilogue → stays 1 → fails."
        );

        // ── Assert zero Global topic refcount ────────────────────────────
        // Kind 13534 (NIP-43 membership) is a global subscription (no #h channel tag),
        // so only EventTopic::Global is retained. After epilogue it must be released.
        let global_refcount = state
            .pubsub
            .topic_refcount(&tenant, EventTopic::Global)
            .await;
        assert_eq!(
            global_refcount, 0,
            "F5: global topic refcount must be zero after connection teardown. \
             Mutation C: remove pubsub.release_topic from epilogue → stays 1 → fails."
        );

        // ── Assert zero Channel topic refcount ───────────────────────────
        // The sub_channel subscription retains EventTopic::Channel(f5_channel_id).
        // After epilogue, the channel-topic release at connection.rs:638-642 must
        // have been called exactly once, bringing the refcount from 1 to 0.
        //
        // RED mutation: delete connection.rs:638-642 (the channel-topic release) →
        // the refcount stays at 1 after teardown → this assertion panics.
        // This binds the channel-cleanup regression to a real production mutation.
        let channel_refcount_after = state
            .pubsub
            .topic_refcount(&tenant, EventTopic::Channel(f5_channel_id))
            .await;
        assert_eq!(
            channel_refcount_after, 0,
            "F5: channel topic refcount must be zero after connection teardown. \
             The sub_channel subscription retained Channel({f5_channel_id}); \
             epilogue must release it via connection.rs:638-642. \
             RED: delete the channel release loop → refcount stays 1 → panics."
        );

        server.abort();
        let _ = server.await;
    }

    mod postgres_tests {
        #[tokio::test]
        #[ignore = "requires PostgreSQL and Redis"]
        async fn f5_loopback_teardown_epilogue_removes_subscription_and_topic_refcount() {
            super::f5_loopback_teardown_epilogue_removes_subscription_and_topic_refcount_body()
                .await;
        }
    }
}
