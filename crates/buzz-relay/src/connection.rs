//! WebSocket connection lifecycle: semaphore → challenge → recv/send/heartbeat loops → cleanup.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::{Sink, SinkExt, StreamExt};
use sha2::Digest;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

use buzz_auth::{
    generate_challenge, AuthContext, AuthenticatedClientPeer, LimitType, SealedTransportEvidence,
};
use buzz_core::client_binding_bootstrap::{
    ClientBindingBootstrapInputV1, ClientBindingScopeV1, CLIENT_BINDING_BOOTSTRAP_SUB_ID,
};
use buzz_core::tenant::TenantContext;
use chrono::{DateTime, Utc};
use nostr::{Filter, Keys};

use crate::authorization_runtime::{
    ConnectionLocalStatusContract, CurrentStatusAuthorization, CurrentStatusSink,
    StatusSessionError, UnchangedBootstrapDelivery,
};
use crate::handlers;
use crate::protocol::{ClientMessage, RelayMessage};
use crate::state::{run_registered_community_connection, AppState};
use buzz_pubsub::EventTopic;

/// Maximum time a new socket may hold a connection slot without completing NIP-42 auth.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const STATUS_TASK_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared mutable subscription map for a single WebSocket connection.
pub(crate) type ConnectionSubscriptions = Arc<Mutex<HashMap<String, Vec<Filter>>>>;

/// Finalized connection authentication plus optional opaque transport peer.
///
/// The peer type is generic so provenance verification can supply its sealed
/// privacy-safe key without teaching the connection layer about proxy headers,
/// socket addresses, or the key's representation. Raw connection addresses
/// are deliberately not accepted by this API.
#[derive(Debug, Clone)]
pub struct AuthenticatedConnectionContext<Peer = AuthenticatedClientPeer> {
    authorization: AuthContext,
    authenticated_client_peer: Option<Peer>,
}

impl<Peer> AuthenticatedConnectionContext<Peer> {
    /// Finalize a connection with the optional verifier-produced peer key.
    pub fn new(authorization: AuthContext, authenticated_client_peer: Option<Peer>) -> Self {
        Self {
            authorization,
            authenticated_client_peer,
        }
    }

    /// Existing NIP-42 connection authorization.
    pub const fn authorization(&self) -> &AuthContext {
        &self.authorization
    }

    /// Opaque authenticated end-client peer, when transport provenance supplied one.
    pub const fn authenticated_client_peer(&self) -> Option<&Peer> {
        self.authenticated_client_peer.as_ref()
    }

    /// Consume the context for a downstream authenticated connection owner.
    pub fn into_parts(self) -> (AuthContext, Option<Peer>) {
        (self.authorization, self.authenticated_client_peer)
    }
}

impl<Peer> std::ops::Deref for AuthenticatedConnectionContext<Peer> {
    type Target = AuthContext;

    fn deref(&self) -> &Self::Target {
        &self.authorization
    }
}

impl From<AuthContext> for AuthenticatedConnectionContext {
    fn from(authorization: AuthContext) -> Self {
        Self::new(authorization, None)
    }
}

/// Request for the writer to flush a restart close and report the result.
pub(crate) struct RestartClose {
    pub(crate) flushed: tokio::sync::oneshot::Sender<bool>,
}

/// Exact durable-delivery identity carried through the connection writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusWriteIdentity {
    pub(crate) delivery_id: Uuid,
    pub(crate) claim_id: Uuid,
    pub(crate) payload_digest: [u8; 32],
    pub(crate) wire_digest: [u8; 32],
}

/// Proof returned only after the exact status frame reaches `Sink::flush()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusWriteAck {
    pub(crate) identity: Option<StatusWriteIdentity>,
}

/// One serialized bootstrap or status write owned by the socket task.
pub(crate) struct StatusWrite {
    pub(crate) text: String,
    pub(crate) identity: Option<StatusWriteIdentity>,
    pub(crate) drain_data_first: bool,
    pub(crate) deadline: tokio::time::Instant,
    pub(crate) flushed: oneshot::Sender<Result<StatusWriteAck, ()>>,
}

/// Cloneable exact-connection handle for the physically acknowledged writer.
#[derive(Clone)]
pub(crate) struct StatusWriter {
    tx: mpsc::Sender<StatusWrite>,
}

impl StatusWriter {
    pub(crate) const fn new(tx: mpsc::Sender<StatusWrite>) -> Self {
        Self { tx }
    }

    /// Queue one command and await its physical flush acknowledgement.
    pub(crate) async fn write(
        &self,
        text: String,
        identity: Option<StatusWriteIdentity>,
        drain_data_first: bool,
        deadline: tokio::time::Instant,
    ) -> Result<StatusWriteAck, ()> {
        let (flushed, acknowledgement) = oneshot::channel();
        let command = StatusWrite {
            text,
            identity,
            drain_data_first,
            deadline,
            flushed,
        };
        tokio::time::timeout_at(deadline, self.tx.send(command))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        tokio::time::timeout_at(deadline, acknowledgement)
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?
    }
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
    Authenticated(AuthenticatedConnectionContext),
    /// Authentication attempt was rejected.
    Failed,
}

/// Exact canonical session grants established by the Enforce AUTH owner.
#[derive(Clone)]
pub(crate) struct CanonicalWebsocketSession {
    read: buzz_auth::FinalizedAuthContext,
    event_assertion: buzz_auth::VerifiedFederatedAssertion,
}

impl CanonicalWebsocketSession {
    pub(crate) fn new(
        read: buzz_auth::FinalizedAuthContext,
        event_assertion: buzz_auth::VerifiedFederatedAssertion,
    ) -> Self {
        Self {
            read,
            event_assertion,
        }
    }

    pub(crate) fn event_admission(
        &self,
    ) -> (
        buzz_auth::FinalizedAuthContext,
        buzz_auth::VerifiedFederatedAssertion,
    ) {
        (self.read.clone(), self.event_assertion.clone())
    }

    fn authorization(
        &self,
        ingress: crate::authorization_runtime::ProtectedIngress,
    ) -> Option<&buzz_auth::FinalizedAuthContext> {
        match ingress {
            crate::authorization_runtime::ProtectedIngress::WebSocketQuery
            | crate::authorization_runtime::ProtectedIngress::WebSocketCount => Some(&self.read),
            _ => None,
        }
    }
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
    /// Optional corporate identity JWT captured from the WebSocket upgrade request.
    pub corporate_identity_jwt: Option<String>,
    /// Move-only trusted-proxy evidence captured at the Enforce upgrade boundary.
    pub canonical_transport_evidence: Mutex<Option<SealedTransportEvidence>>,
    /// Capability-specific canonical session grants installed only by Enforce AUTH.
    pub(crate) canonical_authorization: RwLock<Option<CanonicalWebsocketSession>>,
    /// Current NIP-42 authentication state.
    pub auth_state: RwLock<AuthState>,
    /// Verified optional S5 scope. It is inert until a finalized direct
    /// binding-status lease activates the Enforce-only outbox lane.
    pub(crate) status_scope: RwLock<Option<ClientBindingScopeV1>>,
    /// Active subscriptions keyed by subscription ID.
    pub subscriptions: ConnectionSubscriptions,
    /// Sender for outbound data messages (EVENT, NOTICE, OK, etc.).
    pub send_tx: mpsc::Sender<WsMessage>,
    /// Dedicated ordered writer for bootstrap and crash-durable status frames.
    pub(crate) status_writer: StatusWriter,
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
    /// Sole connection-local owner of the current scoped-AUTH status task.
    #[cfg(not(test))]
    pub(crate) client_binding_status_task: tokio::sync::Mutex<Option<ClientBindingStatusTask>>,
    /// Configurable slow-client grace limit (from `Config::slow_client_grace_limit`).
    pub grace_limit: u8,
}

pub(crate) struct ClientBindingStatusTask {
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

// Unit-test ConnectionState literals predate the production-only owner field
// and live in unrelated modules. Keep their ephemeral owner slots here so the
// status lease never expands those modules' path ownership.
#[cfg(test)]
static TEST_CLIENT_BINDING_STATUS_TASKS: std::sync::LazyLock<
    dashmap::DashMap<Uuid, Arc<Mutex<Option<ClientBindingStatusTask>>>>,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

#[cfg(test)]
fn test_client_binding_status_slot(conn_id: Uuid) -> Arc<Mutex<Option<ClientBindingStatusTask>>> {
    Arc::clone(
        TEST_CLIENT_BINDING_STATUS_TASKS
            .entry(conn_id)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .value(),
    )
}

impl ConnectionState {
    async fn cancel_and_join_client_binding_status_task(task: ClientBindingStatusTask) -> bool {
        let ClientBindingStatusTask { cancel, mut join } = task;
        cancel.cancel();
        match tokio::time::timeout(STATUS_TASK_JOIN_TIMEOUT, &mut join).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) => false,
            Err(_) => {
                join.abort();
                let _ = join.await;
                false
            }
        }
    }

    /// Cancel and join the prior AUTH scope before replacement bootstrap.
    pub(crate) async fn clear_client_binding_status_task(&self) -> bool {
        #[cfg(not(test))]
        let previous = self.client_binding_status_task.lock().await.take();
        #[cfg(test)]
        let previous = {
            let slot = test_client_binding_status_slot(self.conn_id);
            let previous = slot.lock().await.take();
            previous
        };
        if let Some(previous) = previous {
            if !Self::cancel_and_join_client_binding_status_task(previous).await {
                self.cancel.cancel();
                return false;
            }
        }
        #[cfg(test)]
        TEST_CLIENT_BINDING_STATUS_TASKS.remove(&self.conn_id);
        !self.cancel.is_cancelled()
    }

    /// Install the sole status task. Any concurrently installed scope is
    /// cancelled and joined before this replacement becomes owned.
    pub(crate) async fn replace_client_binding_status_task(
        &self,
        cancel: CancellationToken,
        join: tokio::task::JoinHandle<()>,
    ) -> bool {
        #[cfg(not(test))]
        let mut installed = self.client_binding_status_task.lock().await;
        #[cfg(test)]
        let test_slot = test_client_binding_status_slot(self.conn_id);
        #[cfg(test)]
        let mut installed = test_slot.lock().await;
        if let Some(previous) = installed.take() {
            if !Self::cancel_and_join_client_binding_status_task(previous).await {
                drop(installed);
                let _ = Self::cancel_and_join_client_binding_status_task(ClientBindingStatusTask {
                    cancel,
                    join,
                })
                .await;
                self.cancel.cancel();
                return false;
            }
        }
        if self.cancel.is_cancelled() {
            drop(installed);
            let _ = Self::cancel_and_join_client_binding_status_task(ClientBindingStatusTask {
                cancel,
                join,
            })
            .await;
            return false;
        }
        *installed = Some(ClientBindingStatusTask { cancel, join });
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

/// Exact same-connection current-status delivery boundary. It owns no durable state and
/// treats any queue ambiguity as terminal for the socket.
#[derive(Clone)]
pub struct WebSocketStatusChannel {
    send_tx: mpsc::Sender<WsMessage>,
    cancel: CancellationToken,
}

impl WebSocketStatusChannel {
    /// Bind the producer to one connection's data queue and close token.
    pub fn from_connection(connection: &ConnectionState) -> Self {
        Self {
            send_tx: connection.send_tx.clone(),
            cancel: connection.cancel.clone(),
        }
    }

    /// Deliver the unchanged validated bootstrap before a status session can
    /// be constructed. The scope must come from the already verified AUTH
    /// event and must pin the NIP-11 relay signer.
    pub async fn deliver_bootstrap(
        &self,
        scope: ClientBindingScopeV1,
        authorization: &CurrentStatusAuthorization,
        relay_keys: &Keys,
        issued_at: DateTime<Utc>,
    ) -> Result<UnchangedBootstrapDelivery, StatusSessionError> {
        if scope.relay_signer() != relay_keys.public_key() {
            self.cancel.cancel();
            return Err(StatusSessionError::ContractUnavailable);
        }
        let (domain, author) = authorization.domain_author();
        let issued_at = u64::try_from(issued_at.timestamp()).map_err(|_| {
            self.cancel.cancel();
            StatusSessionError::ContractUnavailable
        })?;
        let bootstrap = ClientBindingBootstrapInputV1::new(
            domain,
            author,
            scope.connection_epoch().clone(),
            issued_at,
        )
        .map_err(|_| {
            self.cancel.cancel();
            StatusSessionError::ContractUnavailable
        })?
        .sign_with_relay_keys(relay_keys)
        .map_err(|_| {
            self.cancel.cancel();
            StatusSessionError::ContractUnavailable
        })?;
        if self
            .send_tx
            .try_send(WsMessage::Text(
                RelayMessage::event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap).into(),
            ))
            .is_err()
        {
            self.cancel.cancel();
            return Err(StatusSessionError::DeliveryFailed);
        }
        Ok(UnchangedBootstrapDelivery::delivered())
    }
}

#[async_trait::async_trait]
impl CurrentStatusSink<ConnectionLocalStatusContract> for WebSocketStatusChannel {
    async fn send_current(&self, current: &nostr::Event) -> Result<(), StatusSessionError> {
        self.send_status(current).await
    }

    async fn send_withdrawal(&self, withdrawal: &nostr::Event) -> Result<(), StatusSessionError> {
        self.send_status(withdrawal).await
    }

    async fn close(&self) {
        self.cancel.cancel();
    }
}

impl WebSocketStatusChannel {
    async fn send_status(&self, event: &nostr::Event) -> Result<(), StatusSessionError> {
        use buzz_core::client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID;

        if self
            .send_tx
            .try_send(WsMessage::Text(
                RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, event).into(),
            ))
            .is_err()
        {
            self.cancel.cancel();
            return Err(StatusSessionError::DeliveryFailed);
        }
        Ok(())
    }
}

/// Entry point for a new WebSocket connection.
///
/// Acquires a connection semaphore permit, sends the NIP-42 AUTH challenge,
/// then drives the send, heartbeat, and receive loops until the connection closes.
pub async fn handle_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    corporate_identity_jwt: Option<String>,
    canonical_transport_evidence: Option<SealedTransportEvidence>,
) {
    let conn_id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let community_id = tenant.community();
    let registry = Arc::clone(&state.community_connections);
    let check_state = Arc::clone(&state);
    let run_state = Arc::clone(&state);
    run_registered_community_connection(
        &registry,
        conn_id,
        community_id,
        cancel.clone(),
        move || async move { check_state.db.is_community_active(community_id).await },
        move || {
            handle_active_connection(
                socket,
                run_state,
                tenant,
                conn_id,
                cancel,
                corporate_identity_jwt,
                canonical_transport_evidence,
            )
        },
    )
    .await;
}

async fn handle_active_connection(
    socket: WebSocket,
    state: Arc<AppState>,
    tenant: TenantContext,
    conn_id: Uuid,
    cancel: CancellationToken,
    corporate_identity_jwt: Option<String>,
    canonical_transport_evidence: Option<SealedTransportEvidence>,
) {
    let permit = match state.conn_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!("Connection limit reached");
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
    let (status_tx, status_rx) = mpsc::channel::<StatusWrite>(8);

    let backpressure_count = Arc::new(AtomicU8::new(0));
    let subscriptions = Arc::new(Mutex::new(HashMap::new()));

    let conn = Arc::new(ConnectionState {
        conn_id,
        tenant,
        corporate_identity_jwt,
        canonical_transport_evidence: Mutex::new(canonical_transport_evidence),
        canonical_authorization: RwLock::new(None),
        auth_state: RwLock::new(AuthState::Pending {
            challenge: challenge.clone(),
        }),
        status_scope: RwLock::new(None),
        subscriptions: Arc::clone(&subscriptions),
        send_tx: tx.clone(),
        status_writer: StatusWriter::new(status_tx),
        ctrl_tx: ctrl_tx.clone(),
        cancel: cancel.clone(),
        backpressure_count: Arc::clone(&backpressure_count),
        #[cfg(not(test))]
        client_binding_status_task: tokio::sync::Mutex::new(None),
        grace_limit: state.config.slow_client_grace_limit,
    });

    info!(conn_id = %conn_id, "WebSocket connection established");
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
        status_rx,
        send_cancel,
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

    if !conn.clear_client_binding_status_task().await {
        warn!(conn_id = %conn.conn_id, "current-binding status task did not stop cleanly");
    }
    cancel.cancel();
    let _ = send_task.await;
    let _ = heartbeat_task.await;
    let _ = auth_timeout_task.await;

    for removed in state.sub_registry.remove_connection(conn.conn_id) {
        state
            .pubsub
            .release_topic(&conn.tenant, topic_for_subscription(removed.channel_id))
            .await;
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
    info!(conn_id = %conn_id, "WebSocket connection closed");

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
    status_rx: mpsc::Receiver<StatusWrite>,
    cancel: CancellationToken,
) {
    send_loop_inner_with_status(ws_send, data_rx, ctrl_rx, restart_rx, status_rx, cancel).await;
}

#[cfg(test)]
async fn send_loop_inner<S>(
    ws_send: S,
    data_rx: mpsc::Receiver<WsMessage>,
    ctrl_rx: mpsc::Receiver<WsMessage>,
    restart_rx: mpsc::Receiver<RestartClose>,
    cancel: CancellationToken,
) where
    S: Sink<WsMessage> + Unpin,
{
    let (_status_tx, status_rx) = mpsc::channel(1);
    send_loop_inner_with_status(ws_send, data_rx, ctrl_rx, restart_rx, status_rx, cancel).await;
}

async fn send_loop_inner_with_status<S>(
    mut ws_send: S,
    mut data_rx: mpsc::Receiver<WsMessage>,
    mut ctrl_rx: mpsc::Receiver<WsMessage>,
    mut restart_rx: mpsc::Receiver<RestartClose>,
    mut status_rx: mpsc::Receiver<StatusWrite>,
    cancel: CancellationToken,
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
            // Biased: restart > status > cancel > ordinary control > data. A restart
            // command owns shutdown delivery and must flush its 1012 before
            // cancellation can fall back to an unacknowledged close. A status
            // command already accepted by its dedicated channel remains ordered
            // ahead of a concurrent close even if its waiter was dropped.
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
            Some(status) = status_rx.recv() => {
                let result = flush_status_write(
                    &mut ws_send,
                    &mut data_rx,
                    &status,
                ).await;
                let ack = result.map(|()| StatusWriteAck {
                    identity: status.identity,
                });
                let succeeded = ack.is_ok();
                let _ = status.flushed.send(ack);
                if !succeeded {
                    break;
                }
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
                let _ = ws_send.send(WsMessage::Close(None)).await;
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

async fn flush_status_write<S>(
    ws_send: &mut S,
    data_rx: &mut mpsc::Receiver<WsMessage>,
    status: &StatusWrite,
) -> Result<(), ()>
where
    S: Sink<WsMessage> + Unpin,
{
    tokio::time::timeout_at(status.deadline, async {
        if status.drain_data_first {
            while let Ok(message) = data_rx.try_recv() {
                ws_send.feed(message).await.map_err(|_| ())?;
            }
            ws_send.flush().await.map_err(|_| ())?;
        }
        if let Some(identity) = status.identity {
            let actual_wire_digest: [u8; 32] = sha2::Sha256::digest(status.text.as_bytes()).into();
            if actual_wire_digest != identity.wire_digest {
                return Err(());
            }
        }
        ws_send
            .feed(WsMessage::Text(status.text.clone().into()))
            .await
            .map_err(|_| ())?;
        ws_send.flush().await.map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
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
            let span = tracing::info_span!("ws.req", conn_id = %conn.conn_id, sub_id = %sub_id);
            tokio::spawn(
                async move {
                    handlers::req::handle_req(sub_id, filters, conn, state).await;
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

    if state.config.nip_fi_mode == buzz_auth::NipFiMode::Enforce {
        let ingress = match msg {
            ClientMessage::Event(_) => {
                crate::authorization_runtime::ProtectedIngress::WebSocketEvent
            }
            ClientMessage::Req { .. } => {
                crate::authorization_runtime::ProtectedIngress::WebSocketQuery
            }
            ClientMessage::Count { .. } => {
                crate::authorization_runtime::ProtectedIngress::WebSocketCount
            }
            _ => return true,
        };
        let session_ingress = if is_event {
            // EVENT uses the read-only canonical session only as its AUTH
            // origin. The handler independently prepares and commits the
            // exact MessagesWrite mutation before any event effect.
            crate::authorization_runtime::ProtectedIngress::WebSocketQuery
        } else {
            ingress
        };
        let admitted = {
            let authorization = conn.canonical_authorization.read().await;
            authorization.as_ref().is_some_and(|session| {
                session
                    .authorization(session_ingress)
                    .is_some_and(|authorization| {
                        crate::protected_ingress::session_authorizes(
                            state,
                            session_ingress,
                            authorization,
                            conn.tenant.community(),
                            pubkey,
                        )
                    })
            })
        };
        if !admitted {
            let sub_id = match msg {
                ClientMessage::Req { sub_id, .. } | ClientMessage::Count { sub_id, .. } => {
                    Some(sub_id.as_str())
                }
                _ => None,
            };
            conn.send(request_rejection_message(
                sub_id,
                "restricted: canonical session authorization denied",
            ));
            return false;
        }
        if is_event {
            // Enforce EVENT owns quota/replay and application mutation inside
            // the canonical PostgreSQL transaction. Legacy Redis admission
            // here would mutate before the exact event can be authorized.
            return true;
        }
    }

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
    use crate::authorization_runtime::CurrentStatusContract;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use buzz_core::client_binding_bootstrap::{
        CLIENT_BINDING_SCOPE_TAG, CLIENT_BINDING_STATUS_SUB_ID,
    };
    use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
    use nostr::{EventBuilder, Kind, Tag};

    #[test]
    fn authenticated_context_carries_only_the_verifier_peer_key() {
        let keys = Keys::generate();
        let authorization = AuthContext {
            pubkey: keys.public_key(),
            scopes: Vec::new(),
            channel_ids: None,
            auth_method: buzz_auth::AuthMethod::Nip42,
            agent_owner_pubkey: None,
        };
        let peer = AuthenticatedClientPeer::for_test([7; 32]);

        let context = AuthenticatedConnectionContext::new(authorization, Some(peer));

        assert_eq!(context.pubkey, keys.public_key());
        assert_eq!(
            context
                .authenticated_client_peer()
                .map(AuthenticatedClientPeer::admission_key),
            Some(&[7; 32])
        );
        let (authorization, carried_peer) = context.into_parts();
        assert_eq!(authorization.pubkey, keys.public_key());
        assert_eq!(
            carried_peer
                .as_ref()
                .map(AuthenticatedClientPeer::admission_key),
            Some(&[7; 32])
        );
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

    #[tokio::test]
    async fn reserved_status_channel_delivers_bootstrap_before_current_and_closes_on_ambiguity() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let now = Utc::now();
        let domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let evidence = CanonicalCurrentBindingEvidence::new(
            domain,
            author.public_key(),
            Uuid::from_u128(2),
            3,
            4,
            5,
            6,
            AuthorizationLeaseFence::from_bytes([7; 32]).unwrap(),
            now,
            now + chrono::Duration::seconds(240),
        )
        .unwrap();
        let authorization = CurrentStatusAuthorization::from_test_parts(
            &evidence,
            now + chrono::Duration::seconds(240),
        );
        let scope_event = EventBuilder::new(Kind::Custom(22242), "")
            .tags([Tag::parse(vec![
                CLIENT_BINDING_SCOPE_TAG.to_owned(),
                "1".to_owned(),
                "11111111-1111-4111-8111-111111111111".to_owned(),
                relay.public_key().to_hex(),
            ])
            .unwrap()])
            .sign_with_keys(&author)
            .unwrap();
        let scope = ClientBindingScopeV1::from_verified_auth_event(&scope_event).unwrap();
        let (send_tx, mut send_rx) = mpsc::channel(2);
        let cancel = CancellationToken::new();
        let channel = WebSocketStatusChannel {
            send_tx,
            cancel: cancel.clone(),
        };

        let bootstrap = channel
            .deliver_bootstrap(scope, &authorization, &relay, now)
            .await
            .unwrap();
        let contract =
            ConnectionLocalStatusContract::new(relay.clone(), relay.public_key()).unwrap();
        let current = contract.current(&evidence, 1).unwrap();
        channel.send_current(&current).await.unwrap();

        let first = send_rx.recv().await.unwrap();
        let second = send_rx.recv().await.unwrap();
        let subscription_id = |message: WsMessage| match message {
            WsMessage::Text(text) => serde_json::from_str::<serde_json::Value>(text.as_str())
                .unwrap()
                .as_array()
                .unwrap()[1]
                .as_str()
                .unwrap()
                .to_owned(),
            other => panic!("expected text delivery, got {other:?}"),
        };
        assert_eq!(subscription_id(first), CLIENT_BINDING_BOOTSTRAP_SUB_ID);
        assert_eq!(subscription_id(second), CLIENT_BINDING_STATUS_SUB_ID);
        let _bootstrap_proof = bootstrap;
        assert!(!cancel.is_cancelled());

        let (full_tx, _full_rx) = mpsc::channel(1);
        full_tx
            .try_send(WsMessage::Text("occupied".into()))
            .unwrap();
        let full_cancel = CancellationToken::new();
        let full = WebSocketStatusChannel {
            send_tx: full_tx,
            cancel: full_cancel.clone(),
        };
        assert_eq!(
            full.send_current(&current).await,
            Err(StatusSessionError::DeliveryFailed)
        );
        assert!(full_cancel.is_cancelled());
    }

    #[tokio::test]
    async fn auth_replacement_and_teardown_join_connection_owned_withdrawal() {
        let (send_tx, _send_rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (status_tx, _status_rx) = mpsc::channel(1);
        let socket_cancel = CancellationToken::new();
        let conn = ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: TenantContext::resolved(
                CommunityId::from_uuid(Uuid::from_u128(1)),
                "status.test",
            ),
            corporate_identity_jwt: None,
            canonical_transport_evidence: tokio::sync::Mutex::new(None),
            canonical_authorization: RwLock::new(None),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: "challenge".to_owned(),
            }),
            status_scope: RwLock::new(None),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            status_writer: StatusWriter::new(status_tx),
            ctrl_tx,
            cancel: socket_cancel.clone(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        };

        let first_cancel = socket_cancel.child_token();
        let first_observed = first_cancel.clone();
        let first_finished = Arc::new(AtomicBool::new(false));
        let first_finished_task = Arc::clone(&first_finished);
        let first_join = tokio::spawn(async move {
            first_observed.cancelled().await;
            first_finished_task.store(true, Ordering::SeqCst);
        });
        assert!(
            conn.replace_client_binding_status_task(first_cancel.clone(), first_join,)
                .await
        );

        let replacement_cancel = socket_cancel.child_token();
        let replacement_observed = replacement_cancel.clone();
        let replacement_finished = Arc::new(AtomicBool::new(false));
        let replacement_finished_task = Arc::clone(&replacement_finished);
        let (withdrawal_tx, mut withdrawal_rx) = mpsc::channel(1);
        let replacement_join = tokio::spawn(async move {
            replacement_observed.cancelled().await;
            withdrawal_tx.send("withdrawn").await.unwrap();
            replacement_finished_task.store(true, Ordering::SeqCst);
        });
        assert!(
            conn.replace_client_binding_status_task(replacement_cancel.clone(), replacement_join,)
                .await
        );
        assert!(first_cancel.is_cancelled());
        assert!(first_finished.load(Ordering::SeqCst));
        assert!(!replacement_cancel.is_cancelled());

        assert!(conn.clear_client_binding_status_task().await);
        assert!(replacement_cancel.is_cancelled());
        assert!(replacement_finished.load(Ordering::SeqCst));
        assert_eq!(withdrawal_rx.try_recv().unwrap(), "withdrawn");
        assert!(!socket_cancel.is_cancelled());
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

    fn text_payloads(messages: &[WsMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|msg| match msg {
                WsMessage::Text(text) => text.to_string(),
                other => panic!("unexpected websocket message in test: {other:?}"),
            })
            .collect()
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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, CancellationToken::new()).await;

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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, CancellationToken::new()).await;

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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, CancellationToken::new()).await;

        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 2);
        assert_eq!(
            text_payloads(&state.messages),
            vec!["control", "data-0", "data-1"]
        );
    }

    #[tokio::test]
    async fn status_writer_acknowledges_only_the_exact_physically_flushed_frame() {
        let (data_tx, data_rx) = mpsc::channel(MAX_WS_SEND_BATCH + 2);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let (status_tx, status_rx) = mpsc::channel(1);
        let writer = StatusWriter::new(status_tx);
        let cancel = CancellationToken::new();
        for index in 0..=MAX_WS_SEND_BATCH {
            data_tx
                .send(WsMessage::Text(format!("AUTH-OK-{index}").into()))
                .await
                .expect("queue auth acknowledgement");
        }
        let (sink, state) = MockSink::new(None);
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            send_loop_inner_with_status(sink, data_rx, ctrl_rx, restart_rx, status_rx, task_cancel)
                .await;
        });
        let identity = StatusWriteIdentity {
            delivery_id: Uuid::new_v4(),
            claim_id: Uuid::new_v4(),
            payload_digest: [7; 32],
            wire_digest: sha2::Sha256::digest(b"STATUS").into(),
        };
        let ack = writer
            .write(
                "STATUS".to_owned(),
                Some(identity),
                true,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await
            .expect("physical status flush");
        assert_eq!(ack.identity, Some(identity));
        {
            let state = state.lock().expect("mock sink poisoned");
            assert_eq!(state.flush_count, 2);
            let payloads = text_payloads(&state.messages);
            assert_eq!(payloads.len(), MAX_WS_SEND_BATCH + 2);
            assert_eq!(payloads.first().map(String::as_str), Some("AUTH-OK-0"));
            assert_eq!(payloads.last().map(String::as_str), Some("STATUS"));
        }
        cancel.cancel();
        task.await.expect("writer task");
    }

    #[tokio::test]
    async fn status_writer_never_acknowledges_queueing_when_flush_fails() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let (status_tx, status_rx) = mpsc::channel(1);
        let writer = StatusWriter::new(status_tx);
        let (sink, state) = MockSink::new(Some(1));
        let task = tokio::spawn(async move {
            send_loop_inner_with_status(
                sink,
                data_rx,
                ctrl_rx,
                restart_rx,
                status_rx,
                CancellationToken::new(),
            )
            .await;
        });
        let identity = StatusWriteIdentity {
            delivery_id: Uuid::new_v4(),
            claim_id: Uuid::new_v4(),
            payload_digest: [8; 32],
            wire_digest: sha2::Sha256::digest(b"STATUS").into(),
        };
        assert!(writer
            .write(
                "STATUS".to_owned(),
                Some(identity),
                false,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await
            .is_err());
        task.await.expect("writer task");
        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1);
        assert_eq!(text_payloads(&state.messages), vec!["STATUS"]);
    }

    #[tokio::test]
    async fn status_writer_rejects_an_ack_identity_bound_to_other_wire_bytes() {
        let (_data_tx, data_rx) = mpsc::channel(1);
        let (_ctrl_tx, ctrl_rx) = mpsc::channel(1);
        let (_restart_tx, restart_rx) = mpsc::channel(1);
        let (status_tx, status_rx) = mpsc::channel(1);
        let writer = StatusWriter::new(status_tx);
        let (sink, state) = MockSink::new(None);
        let task = tokio::spawn(async move {
            send_loop_inner_with_status(
                sink,
                data_rx,
                ctrl_rx,
                restart_rx,
                status_rx,
                CancellationToken::new(),
            )
            .await;
        });
        let identity = StatusWriteIdentity {
            delivery_id: Uuid::new_v4(),
            claim_id: Uuid::new_v4(),
            payload_digest: [9; 32],
            wire_digest: sha2::Sha256::digest(b"OTHER").into(),
        };
        assert!(writer
            .write(
                "STATUS".to_owned(),
                Some(identity),
                false,
                tokio::time::Instant::now() + Duration::from_secs(2),
            )
            .await
            .is_err());
        task.await.expect("writer task");
        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 0);
        assert!(state.messages.is_empty());
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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, CancellationToken::new()).await;

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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, CancellationToken::new()).await;

        assert_eq!(flushed_rx.await, Ok(false));
        let state = state.lock().expect("mock sink poisoned");
        assert_eq!(state.flush_count, 1);
        assert_eq!(state.messages.len(), 1, "no fallback close is appended");
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
        send_loop_inner(sink, data_rx, ctrl_rx, restart_rx, cancel).await;

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
            matches!(state.messages[1], WsMessage::Close(_)),
            "Close is sent only after the reason frame is flushed"
        );
    }
}
