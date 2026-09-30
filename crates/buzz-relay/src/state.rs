//! Shared application state — Arc-wrapped, shared across all connections.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, Utf8Bytes as WsUtf8Bytes};
use dashmap::DashMap;
use futures_util::future::join_all;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use buzz_audit::AuditService;
use buzz_auth::{AuthService, CommandReplayGuard, Nip98ReplayGuard};
use buzz_core::tenant::TenantContext;
use buzz_core::CommunityId;
use buzz_db::Db;
use buzz_media::MediaStorage;
use buzz_pubsub::cache_invalidation::CacheInvalidation;
use buzz_pubsub::conn_control::ConnControl;
use buzz_pubsub::rate_limiter::RedisRateLimiter;
use buzz_pubsub::{PubSubManager, RedisCommandReplayGuard, RedisNip98ReplayGuard};
use buzz_search::SearchService;
use buzz_workflow::WorkflowEngine;
use deadpool_redis;

use crate::audio::AudioRoomManager;
use crate::config::Config;
use crate::connection::{ConnectionSubscriptions, RestartClose};
use crate::subscription::SubscriptionRegistry;

pub(crate) type ScopedPubkeyKey = (CommunityId, [u8; 32]);

/// Why a community-bound socket is being asked to stop.
///
/// Only deletion is externally attributed today. Ordinary lifecycle exits keep
/// using cancellation alone and therefore retain the existing bare-close
/// behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommunityDisconnectReason {
    CommunityDeleted,
    /// NIP-FI: the connection's proven pubkey was added to the deny set.
    AuthorizationDenied,
    /// The authenticated pubkey lost access to the community (e.g. a ban).
    AccessRevoked,
}

impl CommunityDisconnectReason {
    pub(crate) fn close_message(self) -> WsMessage {
        match self {
            Self::CommunityDeleted => WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::POLICY,
                reason: WsUtf8Bytes::from_static("community deleted"),
            })),
            Self::AuthorizationDenied => WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::POLICY,
                reason: WsUtf8Bytes::from_static("authorization denied"),
            })),
            Self::AccessRevoked => WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::POLICY,
                reason: WsUtf8Bytes::from_static("access revoked"),
            })),
        }
    }
}

/// NIP-42-proven key and the NIP-FI issuer a socket was admitted under.
#[derive(Clone)]
struct ProvenIdentity {
    pubkey: Vec<u8>,
    nip_fi_issuer: Option<String>,
}

impl ProvenIdentity {
    fn matches(&self, issuer: &str, pubkey: &[u8]) -> bool {
        self.pubkey == pubkey && self.nip_fi_issuer.as_deref() == Some(issuer)
    }
}

/// Per-socket lifecycle controls shared by the registry and the writer.
#[derive(Clone)]
pub(crate) struct CommunityConnectionControl {
    cancel: CancellationToken,
    reason_tx: watch::Sender<Option<CommunityDisconnectReason>>,
    /// NIP-42-proven pubkey plus the NIP-FI issuer the session was admitted
    /// under; matched by the registry's `disconnect_nip_fi` scan.  [FI-TRACE-DENY-SET]
    proven_identity: Arc<std::sync::RwLock<Option<ProvenIdentity>>>,
    /// Serializes NIP-FI denial writers across reason-win + terminal enqueue,
    /// and holds the audio terminal-frame sender (root connections leave it `None`).
    terminal_frame_tx: Arc<std::sync::Mutex<Option<mpsc::Sender<WsMessage>>>>,
    /// Pubkey proven by this socket's auth, set once auth succeeds. Sockets
    /// whose pubkey is tracked by [`ConnectionManager`] leave it unset.
    pubkey: Arc<std::sync::OnceLock<[u8; 32]>>,
}

impl CommunityConnectionControl {
    pub(crate) fn new(cancel: CancellationToken) -> Self {
        let (reason_tx, _reason_rx) = watch::channel(None);
        Self {
            cancel,
            reason_tx,
            proven_identity: Arc::new(std::sync::RwLock::new(None)),
            terminal_frame_tx: Arc::new(std::sync::Mutex::new(None)),
            pubkey: Arc::default(),
        }
    }

    /// Records the authenticated pubkey so pubkey-scoped disconnects reach this socket.
    pub(crate) fn bind_pubkey(&self, pubkey: [u8; 32]) {
        let _ = self.pubkey.set(pubkey);
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub(crate) fn disconnect_reason(&self) -> watch::Receiver<Option<CommunityDisconnectReason>> {
        self.reason_tx.subscribe()
    }

    /// Records the NIP-42-proven pubkey and admitting NIP-FI issuer for this
    /// connection so the registry can close it via `disconnect_nip_fi`.
    pub(crate) fn set_proven_identity(&self, pubkey: Vec<u8>, nip_fi_issuer: Option<String>) {
        if let Ok(mut slot) = self.proven_identity.write() {
            *slot = Some(ProvenIdentity {
                pubkey,
                nip_fi_issuer,
            });
        }
    }

    /// Registers the audio terminal-frame sender so `disconnect_nip_fi` can
    /// enqueue the denial payload before cancelling.
    ///
    /// Called by `handle_active_audio_connection` immediately after the terminal
    /// channel is created (before any `check_cancel!` or `send_loop`).  The
    /// sender is optional — root relay connections leave this unset and rely on
    /// the separate `ctrl_tx` path in `ConnectionManager::disconnect_nip_fi`.
    pub(crate) fn set_terminal_frame_sender(&self, tx: mpsc::Sender<WsMessage>) {
        let mut slot = self
            .terminal_frame_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(tx);
    }

    /// First-writer-wins `authorization_denied` transition shared by every
    /// NIP-FI denial writer: under the transition lock, publishes
    /// `AuthorizationDenied` only if no reason is set yet, and only the winner
    /// enqueues `route`'s denial frame — on `frame_tx`, else on the sender
    /// registered by `set_terminal_frame_sender`.  Every writer therefore
    /// yields the same single frame plus the reason's 1008 close.
    /// Does not cancel.  [FI-TRACE-DENIAL-ORACLE]
    fn publish_authorization_denied(
        &self,
        route: crate::nip_fi_session::NipFiWsRoute,
        frame_tx: Option<&mpsc::Sender<WsMessage>>,
    ) {
        let slot = self
            .terminal_frame_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let won = self.reason_tx.send_if_modified(|current| match current {
            None => {
                *current = Some(CommunityDisconnectReason::AuthorizationDenied);
                true
            }
            Some(_) => false,
        });
        if !won {
            return;
        }
        if let Some(tx) = frame_tx.or(slot.as_ref()) {
            let _ = tx.try_send(crate::nip_fi_session::denial_frame(
                route,
                buzz_auth::DenialClass::AuthorizationDenied,
            ));
        }
    }

    /// `authorization_denied` decided on the socket itself (key mismatch,
    /// deny-set hit, ban/allowlist/membership refusal, lease expiry): the
    /// shared first-writer-wins transition on `frame_tx`.  Does not cancel;
    /// the caller cancels after this returns.  [FI-TRACE-DENY-SET]
    pub(crate) fn deny_authorization(
        &self,
        frame_tx: &mpsc::Sender<WsMessage>,
        route: crate::nip_fi_session::NipFiWsRoute,
    ) {
        self.publish_authorization_denied(route, Some(frame_tx));
    }

    /// Denial transition for `ConnectionManager::disconnect_nip_fi`: the shared
    /// transition on the root connection's `terminal_ctrl_tx` (drained first
    /// by `send_loop` on cancel), then cancels.
    pub(crate) fn manager_disconnect_nip_fi(&self, frame_tx: &mpsc::Sender<WsMessage>) {
        self.publish_authorization_denied(
            crate::nip_fi_session::NipFiWsRoute::Root,
            Some(frame_tx),
        );
        self.cancel.cancel();
    }

    /// Audio-route denial transition, used by the registry scan and the audio
    /// handler's own denials: the shared transition on the sender registered
    /// by `set_terminal_frame_sender`, then cancels.
    pub(crate) fn disconnect_nip_fi(&self) {
        self.publish_authorization_denied(crate::nip_fi_session::NipFiWsRoute::Audio, None);
        self.cancel.cancel();
    }

    fn disconnect_community(&self) {
        self.reason_tx
            .send_replace(Some(CommunityDisconnectReason::CommunityDeleted));
        self.cancel.cancel();
    }

    fn revoke_access(&self) {
        self.reason_tx
            .send_replace(Some(CommunityDisconnectReason::AccessRevoked));
        self.cancel.cancel();
    }
}

/// Leaves headroom under the process-wide drain deadline for a stalled writer.
const RESTART_CLOSE_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
type SlidingWindowCounter = (u32, Instant);
type ScopedRateLimiter = DashMap<ScopedPubkeyKey, SlidingWindowCounter>;

/// Per-connection entry in the connection manager.
struct ConnEntry {
    tx: mpsc::Sender<WsMessage>,
    /// Control-frame sender, drained ahead of data and before cancel wins in
    /// the send loop. Used to deliver a ban-disconnect frame that must reach
    /// the client before the socket is closed (see [`ConnectionManager::disconnect_pubkey`]).
    ctrl_tx: mpsc::Sender<WsMessage>,
    /// Dedicated one-slot sender for the terminal NIP-FI denial frame, drained
    /// first by `send_loop` on cancel.
    terminal_ctrl_tx: mpsc::Sender<WsMessage>,
    restart_tx: Option<mpsc::Sender<RestartClose>>,
    cancel: CancellationToken,
    /// Community resolved from the connection host at handshake. This is the
    /// receiver-side tenant label fan-out must compare against the event label.
    community_id: CommunityId,
    /// Shared with `ConnectionState` — both direct sends and fan-out
    /// broadcasts track the same consecutive-full counter.
    backpressure_count: Arc<AtomicU8>,
    subscriptions: ConnectionSubscriptions,
    authenticated_pubkey: Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    /// NIP-FI issuer the session was admitted under.  Written while holding
    /// the `authenticated_pubkey` write lock and read under its read lock, so
    /// `disconnect_nip_fi` never sees the pubkey without its issuer.
    nip_fi_issuer: std::sync::RwLock<Option<String>>,
    grace_limit: u8,
    /// Lifecycle control used by `disconnect_nip_fi` for the denial transition.
    community_control: CommunityConnectionControl,
}

/// Community-scoped lifecycle registry shared by every long-lived socket type.
///
/// A handler registers before durable active-state revalidation. Archival after
/// registration cancels the token; archival before registration is observed by
/// the revalidation. The returned guard removes the entry on every exit path.
pub struct CommunityConnectionRegistry {
    connections: Arc<DashMap<Uuid, (CommunityId, CommunityConnectionControl)>>,
}

impl Default for CommunityConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommunityConnectionRegistry {
    /// Creates an empty lifecycle registry.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
        }
    }

    /// Registers one socket and returns a guard that deregisters it on drop.
    pub(crate) fn register(
        &self,
        connection_id: Uuid,
        community_id: CommunityId,
        control: CommunityConnectionControl,
    ) -> CommunityConnectionGuard {
        self.connections
            .insert(connection_id, (community_id, control));
        CommunityConnectionGuard {
            connection_id,
            connections: Arc::clone(&self.connections),
        }
    }

    /// Disconnects every socket type currently bound to `community_id` and
    /// attributes the close to community deletion.
    pub fn disconnect_community(&self, community_id: CommunityId) -> usize {
        let mut closed = 0;
        for entry in self.connections.iter() {
            if entry.value().0 == community_id {
                entry.value().1.disconnect_community();
                closed += 1;
            }
        }
        closed
    }

    /// Disconnects every registered socket admitted under NIP-FI `issuer` whose
    /// proven pubkey matches `pubkey`, across all communities.  A same-key
    /// socket admitted under a different issuer (or with no assertion) is not
    /// matched: this issuer's deny entry cannot block it.  [FI-TRACE-DENY-SET]
    ///
    /// Called by the admin disconnect route (`api/nip_fi.rs`) and the cross-pod
    /// apply closure (`apply_nip_fi_disconnect`), each alongside
    /// `ConnectionManager::disconnect_nip_fi` for Nostr relay connections.
    /// A match fires `AuthorizationDenied`, which the send loop turns into a 1008
    /// close frame before the socket shuts down.  Sockets not yet registered with
    /// a proven identity are not matched; admission registers the identity first
    /// and then runs the deny-set check, so such a socket is rejected there.
    ///
    /// Returns the number of connections closed.
    pub fn disconnect_nip_fi(&self, issuer: &str, pubkey: &[u8]) -> usize {
        let mut closed = 0;
        for entry in self.connections.iter() {
            let matches = entry
                .value()
                .1
                .proven_identity
                .read()
                .ok()
                .and_then(|v| v.as_ref().map(|id| id.matches(issuer, pubkey)))
                .unwrap_or(false);
            if matches {
                entry.value().1.disconnect_nip_fi();
                closed += 1;
            }
        }
        closed
    }

    /// Disconnects every socket bound to `pubkey` in `community`, attributing
    /// the close to revoked access. Fenced to `community` like
    /// [`ConnectionManager::disconnect_pubkey`].
    pub fn disconnect_pubkey(&self, community_id: CommunityId, pubkey: &[u8]) -> usize {
        let mut closed = 0;
        for entry in self.connections.iter() {
            let (community, control) = entry.value();
            if *community == community_id && control.pubkey.get().map(|k| &k[..]) == Some(pubkey) {
                control.revoke_access();
                closed += 1;
            }
        }
        closed
    }

    /// Returns the distinct communities with live sockets on this pod.
    pub fn bound_communities(&self) -> HashSet<CommunityId> {
        self.connections
            .iter()
            .map(|entry| entry.value().0)
            .collect()
    }
}

/// Removes a socket lifecycle registration on every handler exit path.
pub struct CommunityConnectionGuard {
    connection_id: Uuid,
    connections: Arc<DashMap<Uuid, (CommunityId, CommunityConnectionControl)>>,
}

impl Drop for CommunityConnectionGuard {
    fn drop(&mut self) {
        self.connections.remove(&self.connection_id);
    }
}

/// Message reported when the one-time Redis bootstrap gate rejects startup.
///
/// Bounded and stable so operators and the boot regression test can match on
/// it without parsing the underlying driver error.
pub const REDIS_BOOTSTRAP_FAILURE: &str = "Redis command path unavailable at startup";

/// Budget for the one-time bootstrap PING. A refused port answers immediately;
/// this only bounds a blackholed address, where hanging forever would be worse
/// than exiting.
const REDIS_BOOTSTRAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Keep the Redis driver's per-command timeout outside the relay-owned startup
/// budget so [`REDIS_BOOTSTRAP_TIMEOUT`] remains the one authoritative bound.
const REDIS_BOOTSTRAP_DRIVER_TIMEOUT: std::time::Duration =
    REDIS_BOOTSTRAP_TIMEOUT.saturating_mul(2);

/// Proves once, during startup, that the Redis command path this pod will serve
/// from can actually be reached.
///
/// `deadpool_redis` pools dial lazily and `PubSubManager::new` only allocates
/// channels, so without this nothing in boot ever opened a command connection:
/// a relay came up against a dead Redis, bound its health listener, and — since
/// readiness reports local lifecycle only — advertised ready forever. Binding
/// that listener is a one-way latch, so the check has to happen before it, and
/// it is deliberately a *startup* gate: once serving, a Redis blip is a
/// dependency failure and must never change readiness.
pub async fn verify_redis_command_path(pool: &deadpool_redis::Pool) -> anyhow::Result<()> {
    let ping = async {
        let connection = pool
            .get()
            .await
            .map_err(|error| anyhow::anyhow!("{REDIS_BOOTSTRAP_FAILURE}: {error}"))?;
        // This one-shot connection is removed from the pool so extending its
        // driver timeout cannot leak into normal serving traffic. The relay's
        // outer timeout below must bound both lazy checkout and PING.
        let mut connection = deadpool_redis::Connection::take(connection);
        connection.set_response_timeout(REDIS_BOOTSTRAP_DRIVER_TIMEOUT);
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map_err(|error| anyhow::anyhow!("{REDIS_BOOTSTRAP_FAILURE}: {error}"))
    };

    match tokio::time::timeout(REDIS_BOOTSTRAP_TIMEOUT, ping).await {
        Err(_) => Err(anyhow::anyhow!(
            "{REDIS_BOOTSTRAP_FAILURE}: no response within {REDIS_BOOTSTRAP_TIMEOUT:?}"
        )),
        Ok(result) => result.map(|_| ()),
    }
}

/// Bounded outcome of the durable community-active check run when a socket is
/// admitted.
///
/// `outcome` is the only dimension. Community, tenant, connection, and error
/// text are request-controlled and deliberately absent from the label set.
#[derive(Debug, Clone, Copy)]
enum AdmissionOutcome {
    Active,
    Inactive,
    CheckError,
}

impl AdmissionOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::CheckError => "check_error",
        }
    }
}

fn record_admission_check(outcome: AdmissionOutcome) {
    metrics::counter!(
        "buzz_community_admission_checks_total",
        "outcome" => outcome.label(),
    )
    .increment(1);
}

/// Registers a socket, durably revalidates its community, then runs it.
///
/// The ordering is the archival admission invariant: archive-before-query is
/// observed by the query, while archive-after-registration sees the token.
///
/// Admission is fail-closed: only an affirmative `Ok(true)` may serve. Both
/// `Ok(false)` and a lookup `Err` cancel, because neither proves this tenant is
/// currently admitted, and `docs/multi-tenant-relay.md` I5
/// (`Inv_AdmissionFence`) grants capability only to an actor *currently*
/// admitted to that community. The two are still told apart in telemetry
/// (`buzz_community_admission_checks_total{outcome}`) so an operator can
/// separate archival from database pressure.
///
/// # Cancellation safety
///
/// `check_active()` is awaited inside a `select!` against the registration's
/// cancellation token. If the token fires while the DB check is in flight
/// (e.g., a stalled DB holds an expired socket open), the check is abandoned,
/// `on_not_run()` is called for terminal-frame drain (if any), and the socket
/// is dropped without ever invoking `run`. This ensures a community deletion
/// or NIP-FI expiry that fires during bootstrap terminates the socket promptly
/// rather than waiting for a stalled DB. [Fix 3 / Carl 3 / F3]
pub(crate) async fn run_registered_community_connection<
    Check,
    CheckFuture,
    Run,
    RunFuture,
    OnNotRun,
    OnNotRunFuture,
>(
    registry: &CommunityConnectionRegistry,
    connection_id: Uuid,
    community_id: CommunityId,
    control: CommunityConnectionControl,
    check_active: Check,
    run: Run,
    on_not_run: OnNotRun,
) where
    Check: FnOnce() -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, buzz_db::DbError>>,
    Run: FnOnce(CommunityConnectionControl) -> RunFuture,
    RunFuture: Future<Output = ()>,
    OnNotRun: FnOnce() -> OnNotRunFuture,
    OnNotRunFuture: Future<Output = ()>,
{
    let cancel = control.cancel.clone();
    let _guard = registry.register(connection_id, community_id, control.clone());

    // Race the DB check against the cancellation token so a stalled DB cannot
    // hold an already-expired or already-deleted socket alive indefinitely.
    // Cancellation winning is not an admission outcome, so it records no
    // `buzz_community_admission_checks_total` sample.
    let check_result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            // Cancellation won — do NOT invoke run; drain terminal frames and
            // close the socket via the caller-supplied on_not_run path so a
            // queued NIP-FI denial is delivered even when bootstrap stalls.
            on_not_run().await;
            return;
        }
        result = check_active() => result,
    };

    match check_result {
        Ok(true) => record_admission_check(AdmissionOutcome::Active),
        Ok(false) => {
            record_admission_check(AdmissionOutcome::Inactive);
            cancel.cancel();
            on_not_run().await;
            return;
        }
        Err(error) => {
            // A lookup failure is not an answer, so it cannot authorize one.
            // Admitting here would begin serving AUTH and REQ for a tenant
            // whose lifecycle is unknown, and the adjacent host-binding seam
            // already refuses on exactly this evidence (see
            // `router::nip11_or_ws_handler`). The client sees an ordinary dial
            // failure and retries.
            record_admission_check(AdmissionOutcome::CheckError);
            tracing::warn!(
                %community_id,
                %error,
                "community active check failed; refusing the socket"
            );
            cancel.cancel();
            on_not_run().await;
            return;
        }
    }
    if cancel.is_cancelled() {
        on_not_run().await;
        return;
    }
    run(control).await;
    cancel.cancel();
}

async fn revalidate_registered_communities<Check, CheckFuture>(
    registry: &CommunityConnectionRegistry,
    mut check_active: Check,
) -> (usize, Vec<(CommunityId, buzz_db::DbError)>)
where
    Check: FnMut(CommunityId) -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, buzz_db::DbError>>,
{
    let communities = registry.bound_communities();
    let mut closed = 0;
    let mut failures = Vec::new();
    for community_id in communities {
        match check_active(community_id).await {
            Ok(false) => closed += registry.disconnect_community(community_id),
            Ok(true) => {}
            Err(error) => failures.push((community_id, error)),
        }
    }
    (closed, failures)
}

/// Tracks active Nostr WebSocket connections and provides message routing by connection ID.
pub struct ConnectionManager {
    connections: DashMap<Uuid, ConnEntry>,
    /// Sticky drain flag set by [`Self::drain_all`]. Registrations that land
    /// after the drain snapshot self-signal, so no upgrade-vs-shutdown
    /// interleaving can produce a connection that misses the restart close.
    draining: AtomicBool,
}

impl ConnectionManager {
    /// Creates a new, empty connection manager.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            draining: AtomicBool::new(false),
        }
    }

    /// Registers a connection with its outbound sender, cancellation token,
    /// server-resolved community, shared backpressure counter, mutable
    /// subscription map, and grace limit.
    // Each argument is a distinct per-connection attribute stored verbatim in
    // `ConnEntry`; a params struct would only relocate the same fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register(
        &self,
        conn_id: Uuid,
        tx: mpsc::Sender<WsMessage>,
        ctrl_tx: mpsc::Sender<WsMessage>,
        terminal_ctrl_tx: mpsc::Sender<WsMessage>,
        restart_tx: Option<mpsc::Sender<RestartClose>>,
        cancel: CancellationToken,
        community_id: CommunityId,
        backpressure_count: Arc<AtomicU8>,
        subscriptions: ConnectionSubscriptions,
        grace_limit: u8,
        community_control: CommunityConnectionControl,
    ) {
        let drain_ctrl_tx = ctrl_tx.clone();
        let drain_cancel = cancel.clone();
        self.connections.insert(
            conn_id,
            ConnEntry {
                tx,
                ctrl_tx,
                terminal_ctrl_tx,
                restart_tx,
                cancel,
                community_id,
                backpressure_count,
                subscriptions,
                authenticated_pubkey: Arc::new(std::sync::RwLock::new(None)),
                nip_fi_issuer: std::sync::RwLock::new(None),
                grace_limit,
                community_control,
            },
        );
        // Insert-then-check pairs with drain_all's store-then-iterate: either
        // the drain iteration sees this entry, or this check sees the flag.
        // A registration that raced past the snapshot self-signals here, so
        // no connection can outlive graceful shutdown unclosed. A client that
        // arrives mid-shutdown should be closed at once, so the self-signal
        // always uses the immediate control-frame + cancel path regardless of
        // whether jittered drain is enabled — jitter smears the sockets that
        // were already established, not late arrivals.
        if self.draining.load(Ordering::SeqCst) {
            let _ = drain_ctrl_tx.try_send(Self::restart_close_frame());
            drain_cancel.cancel();
        }
    }

    /// Removes a connection from the registry.
    pub fn deregister(&self, conn_id: Uuid) {
        self.connections.remove(&conn_id);
    }

    /// Record the authenticated pubkey for a connection after NIP-42 succeeds.
    pub fn set_authenticated_pubkey(&self, conn_id: Uuid, pubkey_bytes: Vec<u8>) {
        self.set_authenticated_identity(conn_id, pubkey_bytes, None);
    }

    /// Record the authenticated pubkey and the NIP-FI issuer it was admitted
    /// under as one unit: the issuer is written while the pubkey write lock is
    /// held, so a concurrent `disconnect_nip_fi` scan that sees the pubkey also
    /// sees its issuer.  [FI-TRACE-DENY-SET]
    pub(crate) fn set_authenticated_identity(
        &self,
        conn_id: Uuid,
        pubkey_bytes: Vec<u8>,
        nip_fi_issuer: Option<String>,
    ) {
        if let Some(entry) = self.connections.get(&conn_id) {
            if let Ok(mut slot) = entry.authenticated_pubkey.write() {
                if let Ok(mut issuer_slot) = entry.nip_fi_issuer.write() {
                    *issuer_slot = nip_fi_issuer;
                }
                *slot = Some(pubkey_bytes);
            }
        }
    }

    /// Return live connection IDs authenticated as `pubkey_bytes` in one community.
    ///
    /// The same Nostr key may be connected to multiple communities at once.
    /// Callers use this for tenant-visible cleanup such as presence clearing and
    /// subscription eviction, so a connection in B must not keep A's derived
    /// state alive.
    pub fn connection_ids_for_pubkey_in_community(
        &self,
        community_id: CommunityId,
        pubkey_bytes: &[u8],
    ) -> Vec<Uuid> {
        self.connections
            .iter()
            .filter_map(|entry| {
                let matches = entry.community_id == community_id
                    && entry
                        .authenticated_pubkey
                        .read()
                        .ok()
                        .and_then(|value| {
                            value
                                .as_ref()
                                .map(|stored| stored.as_slice() == pubkey_bytes)
                        })
                        .unwrap_or(false);
                matches.then_some(*entry.key())
            })
            .collect()
    }

    /// Return the authenticated pubkey recorded for a connection, if any.
    pub fn pubkey_for_conn(&self, conn_id: Uuid) -> Option<Vec<u8>> {
        self.connections
            .get(&conn_id)
            .and_then(|entry| entry.authenticated_pubkey.read().ok()?.clone())
    }

    /// Cancel a single connection by ID. A no-op if the connection is not
    /// registered (already deregistered or never known).
    pub(crate) fn cancel_conn(&self, conn_id: Uuid) {
        if let Some(entry) = self.connections.get(&conn_id) {
            entry.cancel.cancel();
        }
    }

    /// Disconnect every live connection authenticated as `pubkey` **in
    /// `community`**, delivering a final `OK false` frame carrying `reason`
    /// before closing.
    ///
    /// Used for live ban enforcement (COMMUNITY_MODERATION_PLAN.md §0 decision
    /// 4): a ban must take effect immediately on existing sessions, not just at
    /// the next auth. The frame is sent on the control channel, which the send
    /// loop drains ahead of both queued data and the biased cancel branch, so
    /// the client learns *why* it was dropped. `event_id` labels the `OK` (the
    /// ban has no triggering client event, so a synthetic all-zero id is used).
    ///
    /// The `community` filter is the tenant fence: one pod holds sockets for
    /// many communities, and the same pubkey may be live in several. A ban in
    /// community A must close only A's sockets, never a session the member holds
    /// in community B ("authority stays inside the tenant fence").
    ///
    /// Returns the number of connections closed. This is the pod-local half of
    /// live enforcement; cross-pod fan-out publishes the same intent over Redis.
    pub fn disconnect_pubkey(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        event_id: &str,
        reason: &str,
    ) -> usize {
        let frame = crate::protocol::RelayMessage::ok(event_id, false, reason);
        let mut closed = 0usize;
        for conn_id in self.connection_ids_for_pubkey_in_community(community, pubkey) {
            if let Some(entry) = self.connections.get(&conn_id) {
                if entry.community_id != community {
                    continue;
                }
                // Best-effort delivery: a full control buffer still gets the
                // close via cancel below, just without the reason frame.
                let _ = entry
                    .ctrl_tx
                    .try_send(WsMessage::Text(frame.clone().into()));
                entry.cancel.cancel();
                closed += 1;
            }
        }
        closed
    }

    /// Close all live connections admitted under NIP-FI `issuer` whose proven
    /// pubkey equals `pubkey`, **across all communities**.
    ///
    /// Used by the NIP-FI admin disconnect API: the deny entry is keyed by
    /// `(issuer, pubkey)` and spans every community this relay serves under
    /// that issuer, so the scan is issuer-scoped but not community-fenced.  A
    /// same-key connection admitted under a different issuer (or with no
    /// assertion) is never matched.  [FI-TRACE-DENY-SET]
    ///
    /// Enqueues the `authorization_denied` NOTICE on the dedicated
    /// `terminal_ctrl_tx` channel before cancelling; the send loop drains that
    /// channel first on cancel, so the denial is delivered even when the
    /// ordinary control buffer is full.
    ///
    /// Returns the number of connections closed.
    pub fn disconnect_nip_fi(&self, issuer: &str, pubkey: &[u8]) -> usize {
        let mut closed = 0usize;
        for entry in self.connections.iter() {
            let matches = entry
                .authenticated_pubkey
                .read()
                .ok()
                .map(|stored| {
                    stored.as_deref() == Some(pubkey)
                        && entry
                            .nip_fi_issuer
                            .read()
                            .is_ok_and(|iss| iss.as_deref() == Some(issuer))
                })
                .unwrap_or(false);
            if matches {
                // Winner-only enqueue on the terminal channel, then cancel.
                entry
                    .community_control
                    .manager_disconnect_nip_fi(&entry.terminal_ctrl_tx);
                closed += 1;
            }
        }
        closed
    }

    /// Closes every live connection with a `1012 Service Restart` close frame.
    ///
    /// This is the original, all-at-once drain, retained as the default path
    /// (`BUZZ_DRAIN_JITTER_MS` unset or `0`). It is synchronous and returns as
    /// soon as every close is queued and every connection cancelled, so the
    /// caller's hard-drain timeout backstops delivery unchanged.
    ///
    /// Called when graceful shutdown starts draining. Without this, upgraded
    /// WebSocket connections outlive the axum listener drain: clients ride the
    /// dying pod until the forced exit and then learn about the restart from a
    /// TCP reset (or, on an abrupt kill, from up to 60s of stall-watchdog
    /// silence). The explicit close frame tells them to reconnect immediately
    /// — and that the disconnect is a restart, not a policy action.
    ///
    /// Uses the "queue frame on ctrl, then cancel" idiom (see
    /// [`ConnectionManager::disconnect_pubkey`]): the send loop drains queued
    /// control frames — including this close — before its cancel branch closes
    /// the socket. Best-effort: a full control buffer still gets the close via
    /// cancel, just without the restart code.
    ///
    /// Returns the number of connections signalled.
    pub fn drain_all(&self) -> usize {
        // Store-then-iterate pairs with register's insert-then-check: a
        // registration that misses this iteration observes the flag and
        // self-signals instead. The flag is sticky — drain is one-way.
        self.draining.store(true, Ordering::SeqCst);
        let frame = Self::restart_close_frame();
        let mut closed = 0usize;
        for entry in self.connections.iter() {
            let _ = entry.ctrl_tx.try_send(frame.clone());
            entry.cancel.cancel();
            closed += 1;
        }
        closed
    }

    /// Closes every live connection with a `1012 Service Restart` frame,
    /// spreading closes across `[1, jitter_ms]`.
    ///
    /// This is the jittered drain, used only when `BUZZ_DRAIN_JITTER_MS > 0`.
    /// It is kept deliberately separate from [`Self::drain_all`] so that the
    /// default (jitter-off) shutdown path is byte-for-byte the previously
    /// shipped behavior; the new close-acknowledgement machinery only runs when
    /// jitter is explicitly enabled. Once the jittered path is proven in
    /// production for all cases, the two can be unified and the old one dropped.
    ///
    /// A pod under a rolling deploy can hold thousands of WebSocket sessions.
    /// Closing them simultaneously ([`Self::drain_all`]) makes every client
    /// reconnect at the same moment — a thundering herd that drives the DB
    /// pool-timeout bursts observed on each roll. Delaying each connection's
    /// close by an independent uniform random offset in `[1, jitter_ms]`
    /// smears the reconnects across the window while keeping the well-attributed
    /// 1012 close.
    ///
    /// Each delayed close is delivered over the connection's dedicated
    /// [`RestartClose`] channel: the writer flushes the 1012 frame and
    /// acknowledges the flush, so drain waits for confirmed delivery (up to
    /// [`RESTART_CLOSE_ACK_TIMEOUT`]) rather than assuming it. If the channel is
    /// full/closed or the ack times out, drain falls back to cancellation.
    ///
    /// The sticky drain flag is set before the first await, preserving
    /// [`Self::drain_all`]'s shutdown-boundary race guarantee: a registration
    /// that lands after the snapshot self-signals immediately (no jitter — a
    /// client arriving mid-shutdown should be closed at once). The returned
    /// future owns every delayed close, so the caller must await it before the
    /// relay runtime is allowed to stop.
    ///
    /// Returns the number of connections signalled.
    pub async fn drain_all_jittered(&self, jitter_ms: u64) -> usize {
        // Store-then-snapshot pairs with register's insert-then-check: either
        // the snapshot captures a registration, or it observes the sticky flag
        // and self-signals immediately.
        self.draining.store(true, Ordering::SeqCst);
        let jitter_ms = jitter_ms.max(1);
        let pending: Vec<_> = self
            .connections
            .iter()
            .map(|entry| {
                let ctrl_tx = entry.ctrl_tx.clone();
                let restart_tx = entry.restart_tx.clone();
                let cancel = entry.cancel.clone();
                let delay_ms = 1 + rand::random::<u64>() % jitter_ms;
                async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    let Some(restart_tx) = restart_tx else {
                        // Unit-only registrations do not own a writer task.
                        let _ = ctrl_tx.try_send(Self::restart_close_frame());
                        cancel.cancel();
                        return;
                    };
                    let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel();
                    if restart_tx
                        .try_send(RestartClose {
                            flushed: flushed_tx,
                        })
                        .is_err()
                    {
                        cancel.cancel();
                        return;
                    }
                    let flushed = tokio::time::timeout(RESTART_CLOSE_ACK_TIMEOUT, flushed_rx).await;
                    if !matches!(flushed, Ok(Ok(true))) {
                        cancel.cancel();
                    }
                }
            })
            .collect();
        let count = pending.len();
        join_all(pending).await;
        count
    }

    /// The WS close frame announcing a graceful restart: 1012 Service Restart.
    fn restart_close_frame() -> WsMessage {
        WsMessage::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::RESTART,
            reason: axum::extract::ws::Utf8Bytes::from_static("relay restarting"),
        }))
    }

    /// Return the server-resolved community that the connection's host bound to.
    pub fn community_for_conn(&self, conn_id: Uuid) -> Option<CommunityId> {
        self.connections
            .get(&conn_id)
            .map(|entry| entry.community_id)
    }

    /// Return the subscription map for a connection, if it is still live.
    pub fn subscriptions_for(&self, conn_id: Uuid) -> Option<ConnectionSubscriptions> {
        self.connections
            .get(&conn_id)
            .map(|entry| Arc::clone(&entry.subscriptions))
    }

    /// Snapshot the number of live WebSocket connections per community.
    ///
    /// Returns a map from community UUID to connection count. Used by the
    /// usage poller; snapshotting avoids per-community gauge drift from
    /// mismatched inc/dec across async boundaries.
    pub fn per_community_ws_connections(&self) -> HashMap<CommunityId, u64> {
        let mut counts: HashMap<CommunityId, u64> = HashMap::new();
        for entry in self.connections.iter() {
            *counts.entry(entry.community_id).or_default() += 1;
        }
        counts
    }

    /// Snapshot the number of distinct authenticated pubkeys online per community.
    ///
    /// A pubkey connected to multiple pods will be counted once per pod — the
    /// dashboard sums across pods, so per-pod partial counts are correct.
    /// A pubkey connected twice on the same pod is counted once (distinct set).
    pub fn per_community_users_online(&self) -> HashMap<CommunityId, u64> {
        // community_id → set of pubkey bytes
        let mut seen: HashMap<CommunityId, HashSet<Vec<u8>>> = HashMap::new();
        for entry in self.connections.iter() {
            if let Ok(lock) = entry.authenticated_pubkey.read() {
                if let Some(pk) = lock.as_ref() {
                    seen.entry(entry.community_id)
                        .or_default()
                        .insert(pk.clone());
                }
            }
        }
        seen.into_iter()
            .map(|(cid, set)| (cid, set.len() as u64))
            .collect()
    }

    /// Return the authenticated pubkey for a connection, if any.
    pub fn pubkey_for(&self, conn_id: Uuid) -> Option<Vec<u8>> {
        self.connections
            .get(&conn_id)
            .and_then(|entry| entry.authenticated_pubkey.read().ok()?.clone())
    }

    /// Sends a text message to the given connection.
    ///
    /// Returns `false` if the connection is gone or the buffer is full.
    /// On sustained backpressure (>grace_limit consecutive full buffers),
    /// cancels the connection. Transient stalls get a warning only.
    pub fn send_to(&self, conn_id: Uuid, msg: String) -> bool {
        self.try_send_ws_message(conn_id, WsMessage::Text(msg.into()))
    }

    /// Sends an already-serialized UTF-8 text payload to the given connection.
    ///
    /// The shared `Bytes` payload is cloned into the outbound WS message without
    /// copying the frame body. Callers must only pass valid UTF-8 bytes.
    pub fn send_to_text_bytes(&self, conn_id: Uuid, msg: Arc<Bytes>) -> bool {
        let text = WsUtf8Bytes::try_from(Bytes::clone(msg.as_ref()))
            .expect("relay fan-out frames are serialized UTF-8 JSON");
        self.try_send_ws_message(conn_id, WsMessage::Text(text))
    }

    fn try_send_ws_message(&self, conn_id: Uuid, msg: WsMessage) -> bool {
        if let Some(entry) = self.connections.get(&conn_id) {
            let conn = entry.value();
            match conn.tx.try_send(msg) {
                Ok(_) => {
                    conn.backpressure_count.store(0, Ordering::Relaxed);
                    true
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    let count = conn.backpressure_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= conn.grace_limit {
                        tracing::warn!(conn_id = %conn_id, count, "fan-out: sustained backpressure — cancelling slow client");
                        metrics::counter!("buzz_ws_backpressure_disconnects_total").increment(1);
                        conn.cancel.cancel();
                    } else {
                        tracing::warn!(conn_id = %conn_id, count, grace = conn.grace_limit, "fan-out: send buffer full — grace {count}/{}", conn.grace_limit);
                    }
                    false
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(conn_id = %conn_id, "fan-out: send channel closed");
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared application state, cloned cheaply via inner `Arc` fields.
#[derive(Clone)]
pub struct AppState {
    /// Relay configuration.
    pub config: Arc<Config>,
    /// Database connection pool.
    pub db: Db,
    /// Redis pool for readiness health checks.
    pub redis_pool: deadpool_redis::Pool,
    /// Audit event service, absent when audit logging is disabled.
    pub audit: Option<Arc<AuditService>>,
    /// Pub/sub manager for broadcasting events to subscribers.
    pub pubsub: Arc<PubSubManager>,
    /// Authentication service.
    pub auth: Arc<AuthService>,
    /// Full-text search service.
    pub search: Arc<SearchService>,
    /// Registry of active client subscriptions.
    pub sub_registry: Arc<SubscriptionRegistry>,
    /// Registry of active WebSocket connections.
    pub conn_manager: Arc<ConnectionManager>,
    /// Lifecycle cancellation for every long-lived socket, including huddle audio.
    pub community_connections: Arc<CommunityConnectionRegistry>,
    /// Stops only the periodic lifecycle revalidator during graceful shutdown.
    pub community_revalidator_cancel: CancellationToken,
    /// Test/telemetry counter for archive disconnect publication attempts.
    pub community_disconnect_publish_attempts: Arc<AtomicU64>,
    /// Semaphore limiting total concurrent connections.
    pub conn_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent message handler tasks.
    pub handler_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent git subprocess operations across
    /// the whole relay. Bounds resource use; **not** writer
    /// serialization — that's the CAS at the manifest pointer (spec
    /// §Push step 7, `Inv_NoFork`).
    pub git_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent media upload parsing/transcoding work.
    pub media_upload_semaphore: Arc<Semaphore>,

    /// Workflow engine for background processing.
    pub workflow_engine: Arc<WorkflowEngine>,
    /// Relay signing keypair — used to sign system messages (kind 40099).
    pub relay_keypair: nostr::Keys,
    /// Process-local generation advertised for non-mesh huddle liveness.
    ///
    /// A fresh value on every relay start lets desktop clients retire persisted
    /// admissions when an in-memory audio room is recreated at the same roster
    /// revision after a restart. Mesh rooms use their Redis-fenced generation.
    pub huddle_liveness_generation: Uuid,

    /// Recently-published event IDs for local-echo deduplication, keyed by
    /// `(community_id, event_id)`. Events fanned out in-process are added here;
    /// the Redis subscriber consumer skips them to avoid double delivery.
    ///
    /// The community is part of the key because the same Nostr event id can
    /// legitimately exist in two communities (channel-less events, and
    /// same-channel-UUID/same-`h` events across tenants). Keying on the bare id
    /// would let a local publish in community A suppress delivery of a distinct
    /// event with the same id arriving via Redis for community B — a
    /// cross-community non-interference violation. Entries expire after 60
    /// seconds via moka's TTL eviction — bounded regardless of subscriber health.
    pub local_event_ids: Arc<moka::sync::Cache<(CommunityId, [u8; 32]), ()>>,
    /// Membership cache: (community_id, channel_id, pubkey_bytes) → is_member.
    /// Short TTL (10s) — membership changes are rare but must propagate.
    #[allow(clippy::type_complexity)]
    pub membership_cache: Arc<moka::sync::Cache<(CommunityId, Uuid, Vec<u8>), bool>>,
    /// Accessible channel IDs cache: (community_id, pubkey_bytes) → channel UUIDs.
    /// Short TTL (10s) — invalidated on membership or channel visibility changes.
    #[allow(clippy::type_complexity)]
    pub accessible_channels_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>), Vec<Uuid>>>,
    /// Per-community channel visibility string, used to gate the private-channel fan-out
    /// access check so open channels stay zero-cost. Invalidated on a flip.
    pub channel_visibility_cache: Arc<moka::sync::Cache<(CommunityId, Uuid), String>>,

    /// Bounded channel for audit logging, absent when audit logging is disabled.
    pub audit_tx: Option<mpsc::Sender<buzz_audit::NewAuditEntry>>,
    /// Media storage client (S3/MinIO).
    pub media_storage: Arc<MediaStorage>,
    /// Cached worker snapshot and storage metric emission bookkeeping. See
    /// `storage_sweep` module docs; shared with the usage-metrics tick via
    /// `Arc` the same way other cross-tick poller state lives on `AppState`.
    pub storage_sweep: Arc<tokio::sync::Mutex<crate::storage_sweep::StorageSweepState>>,
    /// Git object-store backend (content-addressed packs/manifests plus
    /// CAS-guarded manifest pointer). This is the durable git source of truth;
    /// see `api::git::store` and `docs/git-on-object-storage.md`.
    pub git_store: crate::api::git::store::GitStore,
    /// Process-local, byte-bounded cache of immutable Git pack/index pairs.
    /// Object storage remains authoritative; this only avoids repeated reads
    /// and index generation for content-addressed packs.
    pub git_pack_cache: Arc<crate::api::git::pack_cache::GitPackCache>,
    /// Audio relay room manager — tracks active huddle audio rooms.
    pub audio_rooms: Arc<AudioRoomManager>,
    /// Set to `true` on SIGTERM — readiness probe returns 503.
    pub shutting_down: Arc<AtomicBool>,
    /// Cached shared-dependency evaluation behind the diagnostic `/_status`
    /// endpoint, owned by [`crate::readiness::run_dependency_sampler`]. Never
    /// consulted by a Kubernetes probe, and never evaluated by a request.
    pub(crate) dependency_diagnostics: Arc<crate::readiness::DependencyDiagnostics>,
    /// Stops only the periodic dependency sampler during graceful shutdown.
    pub dependency_sampler_cancel: CancellationToken,
    /// Stops only the completion-epoch publisher during graceful shutdown.
    pub dependency_completion_publisher_cancel: CancellationToken,
    /// Last completed read-only partition audit, for diagnostics only, never probes.
    pub partition_audit: Arc<std::sync::RwLock<Option<buzz_db::partition::PartitionAudit>>>,
    /// Process start time — used by `/_status` endpoint.
    pub started_at: Instant,
    /// Shared, community-scoped NIP-98 replay prevention.
    ///
    /// Correctness boundary for stateless workers: every pod must consult the
    /// same Redis `SET NX EX` seen-set, keyed by resolved community. Do not
    /// replace this with process-local caching; replay freshness must survive
    /// cross-pod routing.
    pub nip98_replay: Arc<dyn Nip98ReplayGuard>,
    /// Shared HTTP client for relay-proxied GIF provider requests. Reusing the
    /// connection pool avoids a fresh TLS handshake for every search/share.
    pub gif_http_client: reqwest::Client,
    /// Shared Redis-backed admission limits for ordinary HTTP and WebSocket work.
    pub admission_rate_limiter: Arc<RedisRateLimiter>,

    /// Per-agent sliding-window rate limiter for observer frames (kind 24200).
    /// Key: (community_id, agent pubkey bytes). Value: (count, window_start).
    /// 100 events/sec per agent — prevents relay/DB pressure from bursty telemetry.
    pub observer_rate_limiter: Arc<ScopedRateLimiter>,
    /// Per-uploader sliding-window rate limiter for media upload starts.
    /// Key: (community_id, uploader pubkey bytes). Value: (count, window_start).
    pub media_upload_rate_limiter: Arc<ScopedRateLimiter>,
    /// Per-claimer fixed-window rate limiter for invite claim attempts
    /// (`POST /api/invites/claim`). Entries expire after the claim window and
    /// the cache has a hard capacity because pre-membership callers can cheaply
    /// generate fresh Nostr keys.
    pub invite_claim_rate_limiter:
        Arc<moka::sync::Cache<ScopedPubkeyKey, Arc<std::sync::atomic::AtomicU32>>>,
    /// Current in-flight media uploads per (community, uploader pubkey).
    pub media_uploads_in_flight: Arc<DashMap<ScopedPubkeyKey, u32>>,
    /// Cache for observer agent-owner authorization (kind 24200).
    /// Key: (community_id, agent_pubkey_bytes, owner_pubkey_bytes). Value: is_owner.
    /// `agent_owner_pubkey` is immutable inside one community, so a long TTL
    /// (5 min) is safe once the community label is part of the key.
    /// Prevents repeated DB lookups from bursty observer traffic.
    #[allow(clippy::type_complexity)]
    pub observer_owner_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>, Vec<u8>), bool>>,
    /// Cache for the `author_type` metric label on the ingest path.
    /// Key: (community_id, author pubkey bytes). Value: is_agent
    /// (`users.agent_owner_pubkey IS NOT NULL`). The mapping is
    /// first-write-wins and set during auth before an agent's first event,
    /// so a short TTL only bounds staleness for the rare backfill race.
    pub author_type_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>), bool>>,

    /// Runtime conformance tracer. Production binds [`crate::conformance::NoopTracer`]
    /// (zero cost). Conformance tests bind [`crate::conformance::JsonlTracer`] to
    /// record traces for replay against `docs/spec/MultiTenantRelay.tla`.
    /// See `crates/buzz-conformance/` and `crate::conformance` for the
    /// schema, emitter helpers, and the independent checker.
    pub tracer: Arc<dyn buzz_conformance::Tracer>,

    /// Inter-relay mesh handle, set once by `main.rs` after `mesh_boot` (never
    /// a constructor parameter, so `AppState::new` call sites are untouched).
    /// `None`/unset ⇒ mesh-off / single-instance: consumers must behave
    /// byte-identically to a relay without the mesh. Access via
    /// [`AppState::mesh`].
    pub mesh: Arc<std::sync::OnceLock<crate::mesh_boot::MeshHandle>>,

    /// NIP-FI federated-identity assertion verifier, shared across all HTTP
    /// ingress and WebSocket upgrade checks.
    ///
    /// `None` when `config.nip_fi.mode` is `Off`. When present, the verifier
    /// is the single offline authority for assertion validation on every
    /// protected HTTP surface and WebSocket upgrade. The backing `ProductionJwksSource` is also
    /// shared and performs bounded periodic JWKS refresh internally.
    ///
    /// The field uses `dyn VerifyAssertion` (type erasure) so that
    /// integration tests can inject a `StaticIssuerKeySource`-backed verifier
    /// without requiring a live JWKS fetch.  Production code always stores a
    /// `FederatedAssertionVerifier<Arc<ProductionJwksSource>>` here; the type
    /// erased form costs one vtable dispatch per request, which is negligible
    /// relative to the JWT crypto.
    pub nip_fi_verifier: Option<Arc<dyn buzz_auth::VerifyAssertion>>,

    /// The shared JWKS source backing `nip_fi_verifier`, exposed so `main.rs`
    /// can warm it at startup and drive the background refresh loop.
    /// `None` iff `nip_fi_verifier` is `None`.
    pub nip_fi_jwks_source: Option<Arc<buzz_auth::ProductionJwksSource>>,

    // ── NIP-FI command API (S4) ────────────────────────────────────────────
    /// Shared in-memory deny set for NIP-FI.  Absent when mode is `Off`.
    ///
    /// Written by the admin disconnect endpoint; read at WS admission (S4 item
    /// 4) and HTTP admission (`nip_fi_http`): one `Arc`, never a second map.
    pub nip_fi_deny_map: Option<Arc<buzz_auth::NipFiDenyMap>>,

    /// Command JWT verifier for the NIP-FI admin disconnect endpoint.
    ///
    /// `None` when mode is `Off` (no command API is reachable).  When
    /// `Some`, the verifier owns a reference to `nip_fi_deny_map` so the
    /// atomic jti-reservation + deny-entry insertion happens inside `verify()`.
    pub nip_fi_command_verifier:
        Option<Arc<buzz_auth::CommandVerifier<Arc<buzz_auth::ProductionJwksSource>>>>,
    /// Shared NIP-FI command `(iss, jti)` replay claim — the cross-pod fence
    /// on top of the verifier's per-pod reservation.  Redis `SET NX EX`, like
    /// `nip98_replay`; callers fail closed on error.
    pub nip_fi_command_replay: Arc<dyn CommandReplayGuard>,
    /// Detached cross-pod NIP-FI disconnect publishes.  Spawning through it
    /// lets tests wait for every publish to finish; nothing waits on it in
    /// production.
    pub nip_fi_publish_tasks: tokio_util::task::TaskTracker,
}

impl AppState {
    /// Constructs `AppState` from its component services.
    ///
    /// Returns `(state, audit_shutdown)`. The caller should call
    /// `audit_shutdown.drain().await` during graceful shutdown so queued
    /// audit entries are flushed before the process exits.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        db: Db,
        redis_pool: deadpool_redis::Pool,
        audit: impl Into<Option<AuditService>>,
        pubsub: Arc<PubSubManager>,
        auth: AuthService,
        search: SearchService,
        workflow_engine: Arc<WorkflowEngine>,
        relay_keypair: nostr::Keys,
        media_storage: MediaStorage,
    ) -> (Self, AuditShutdownHandle) {
        let max_connections = config.max_connections;
        let max_concurrent_handlers = config.max_concurrent_handlers;
        let search_arc = Arc::new(search);

        let audit_arc = audit.into().map(Arc::new);
        let (audit_tx, mut audit_rx) = mpsc::channel::<buzz_audit::NewAuditEntry>(1000);
        let audit_for_worker = audit_arc.clone();
        let audit_cancel = CancellationToken::new();
        let audit_cancel_worker = audit_cancel.clone();
        let audit_worker_handle = tokio::spawn(async move {
            let Some(audit_for_worker) = audit_for_worker else {
                audit_cancel_worker.cancelled().await;
                return;
            };
            // Normal operation: process entries as they arrive.
            loop {
                tokio::select! {
                    entry = audit_rx.recv() => {
                        match entry {
                            Some(entry) => log_audit_entry(&audit_for_worker, entry).await,
                            None => break, // channel closed
                        }
                    }
                    _ = audit_cancel_worker.cancelled() => {
                        // Close the receiver: rejects future sends and lets us
                        // drain everything already buffered without a race.
                        audit_rx.close();
                        break;
                    }
                }
            }
            // Drain: recv() returns buffered entries, then None once empty.
            let mut drained = 0u32;
            while let Some(entry) = audit_rx.recv().await {
                log_audit_entry(&audit_for_worker, entry).await;
                drained += 1;
            }
            if drained > 0 {
                tracing::info!(drained, "audit worker flushed remaining entries");
            }
            tracing::warn!("audit log worker exited (expected on shutdown)");
        });

        let git_max_concurrent_ops = config.git_max_concurrent_ops;
        let media_max_concurrent_uploads = config.media_max_concurrent_uploads;
        let git_store = crate::api::git::store::GitStore::new(
            &config.media.s3_endpoint,
            &config.media.s3_access_key,
            &config.media.s3_secret_key,
            &config.media.s3_bucket,
            &config.media.s3_region,
            config.media.s3_addressing_style,
        )
        .expect("media storage was already constructed with this S3 config");
        let git_pack_cache = Arc::new(
            crate::api::git::pack_cache::GitPackCache::new(
                &config.git_pack_cache_path,
                config.git_pack_cache_max_bytes,
                config.git_pack_cache_max_concurrent_populations,
            )
            .expect("git pack cache path must be available"),
        );
        let nip98_replay: Arc<dyn Nip98ReplayGuard> =
            Arc::new(RedisNip98ReplayGuard::new(redis_pool.clone()));
        let nip_fi_command_replay: Arc<dyn CommandReplayGuard> =
            Arc::new(RedisCommandReplayGuard::new(redis_pool.clone()));
        let gif_http_client = crate::api::gifs::build_gif_http_client();
        let admission_rate_limiter = Arc::new(RedisRateLimiter::new(redis_pool.clone()));
        let audit_enabled = audit_arc.is_some();
        // Build NIP-FI components before moving config into the state Arc.
        let (nip_fi_verifier, nip_fi_jwks_source) = build_nip_fi_components(&config);
        let state = Self {
            config: Arc::new(config),
            db,
            redis_pool,
            audit: audit_arc,
            pubsub,
            auth: Arc::new(auth),
            search: search_arc,
            sub_registry: Arc::new(SubscriptionRegistry::new()),
            conn_manager: Arc::new(ConnectionManager::new()),
            community_connections: Arc::new(CommunityConnectionRegistry::new()),
            community_revalidator_cancel: CancellationToken::new(),
            community_disconnect_publish_attempts: Arc::new(AtomicU64::new(0)),
            conn_semaphore: Arc::new(Semaphore::new(max_connections)),
            handler_semaphore: Arc::new(Semaphore::new(max_concurrent_handlers)),
            git_semaphore: Arc::new(Semaphore::new(git_max_concurrent_ops)),
            media_upload_semaphore: Arc::new(Semaphore::new(media_max_concurrent_uploads)),
            workflow_engine,
            relay_keypair,
            huddle_liveness_generation: Uuid::new_v4(),

            local_event_ids: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(60))
                    .build(),
            ),
            membership_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            accessible_channels_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            channel_visibility_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            audit_tx: audit_enabled.then_some(audit_tx),
            media_storage: Arc::new(media_storage),
            storage_sweep: Arc::new(tokio::sync::Mutex::new(
                crate::storage_sweep::StorageSweepState::default(),
            )),
            git_store,
            git_pack_cache,
            audio_rooms: Arc::new(AudioRoomManager::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
            dependency_diagnostics: Arc::new(crate::readiness::DependencyDiagnostics::default()),
            dependency_sampler_cancel: CancellationToken::new(),
            dependency_completion_publisher_cancel: CancellationToken::new(),
            partition_audit: Arc::new(std::sync::RwLock::new(None)),
            started_at: Instant::now(),
            nip98_replay,
            gif_http_client,
            admission_rate_limiter,
            observer_rate_limiter: Arc::new(DashMap::new()),
            media_upload_rate_limiter: Arc::new(DashMap::new()),
            invite_claim_rate_limiter: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(crate::api::invites::CLAIM_RATE_CACHE_CAPACITY)
                    .time_to_live(crate::api::invites::CLAIM_RATE_WINDOW)
                    .build(),
            ),
            media_uploads_in_flight: Arc::new(DashMap::new()),
            observer_owner_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(1_000)
                    .time_to_live(std::time::Duration::from_secs(300))
                    .build(),
            ),
            author_type_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(300))
                    .build(),
            ),
            // Default to NoopTracer: production builds pay zero cost.
            // Conformance tests overwrite this with a JsonlTracer after
            // construction (see test helpers in
            // `crates/buzz-test-client` once those land).
            tracer: Arc::new(crate::conformance::NoopTracer),
            mesh: Arc::new(std::sync::OnceLock::new()),
            nip_fi_verifier,
            nip_fi_jwks_source,
            // NIP-FI deny map and command verifier are initialized lazily by
            // `build_nip_fi_command_components` in `api::nip_fi`, called from
            // `main.rs` after startup validation.  `None` is safe before that
            // call: the endpoint returns 503 when the verifier is absent.
            nip_fi_deny_map: None,
            nip_fi_command_verifier: None,
            nip_fi_command_replay,
            nip_fi_publish_tasks: tokio_util::task::TaskTracker::new(),
        };
        (
            state,
            AuditShutdownHandle {
                cancel: audit_cancel,
                handle: audit_worker_handle,
            },
        )
    }

    /// Withdraws this pod from routing. The lifecycle flag is authoritative for
    /// `/_readiness`; the private probe publishes its sampled observation to
    /// the readiness gauge on its next request.
    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn set_dependency_evaluator(
        &mut self,
        evaluator: Arc<dyn crate::readiness::DependencyEvaluator>,
    ) {
        self.dependency_diagnostics = Arc::new(
            crate::readiness::DependencyDiagnostics::with_evaluator(evaluator),
        );
    }

    /// Inter-relay mesh handle. `None` ⇒ mesh-off / single-instance: callers
    /// must no-op to today's behavior. Set once by `main.rs` after boot.
    pub fn mesh(&self) -> Option<&crate::mesh_boot::MeshHandle> {
        self.mesh.get()
    }

    /// Publish a completed partition audit for cached diagnostics.
    pub fn record_partition_audit(&self, audit: buzz_db::partition::PartitionAudit) {
        match self.partition_audit.write() {
            Ok(mut cached) => *cached = Some(audit),
            Err(poisoned) => *poisoned.into_inner() = Some(audit),
        }
    }

    /// Snapshot the last completed audit without accessing the database.
    ///
    /// Periodic refresh failures retain the last-known-good audit by design;
    /// operators should alert on staleness of
    /// `buzz_partition_audit_last_success_timestamp_seconds`.
    pub fn partition_audit_snapshot(&self) -> Option<buzz_db::partition::PartitionAudit> {
        let cached = match self.partition_audit.read() {
            Ok(cached) => cached,
            Err(poisoned) => poisoned.into_inner(),
        };
        cached.clone()
    }

    /// Record an event ID as locally-published for dedup, scoped to the
    /// community it was fanned out in. Called before Redis publish so the
    /// multi-node consumer can skip the echo for *this* community only — a
    /// same-id event in another community is a distinct delivery and must not
    /// be suppressed.
    pub fn mark_local_event(&self, community: CommunityId, event_id: &nostr::EventId) {
        self.local_event_ids
            .insert((community, event_id.to_bytes()), ());
    }

    /// Check channel membership with a 10-second cache. Falls back to DB on miss.
    pub async fn is_member_cached(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<bool, buzz_db::DbError> {
        let key = (community_id, channel_id, pubkey.to_vec());
        if let Some(cached) = self.membership_cache.get(&key) {
            metrics::counter!("buzz_membership_cache_hits_total").increment(1);
            return Ok(cached);
        }
        metrics::counter!("buzz_membership_cache_misses_total").increment(1);
        let result = self.db.is_member(community_id, channel_id, pubkey).await?;
        self.membership_cache.insert(key, result);
        Ok(result)
    }

    /// Invalidate caches after a membership change (add/remove member).
    ///
    /// Drops the local moka entries AND fire-and-forget publishes the same drop
    /// to every other pod over Redis (see [`apply_cache_invalidation`]). The
    /// publish is spawned, not awaited: the local drop is already done, and a
    /// dropped publish is backstopped by the REQ denial-path DB confirmation.
    pub fn invalidate_membership(&self, tenant: &TenantContext, channel_id: Uuid, pubkey: &[u8]) {
        self.invalidate_membership_local(tenant.community(), channel_id, pubkey);
        self.spawn_cache_invalidation(
            tenant,
            CacheInvalidation::Membership {
                channel_id,
                pubkey: pubkey.to_vec(),
            },
        );
    }

    /// Local-only membership drop. The cross-pod consumer calls this directly so
    /// applying a received drop never re-publishes it.
    pub(crate) fn invalidate_membership_local(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) {
        self.membership_cache
            .invalidate(&(community_id, channel_id, pubkey.to_vec()));
        self.accessible_channels_cache
            .invalidate(&(community_id, pubkey.to_vec()));
    }

    /// Invalidate all users' accessible-channels cache (e.g. new open channel created).
    pub fn invalidate_all_accessible_channels(&self, tenant: &TenantContext) {
        self.invalidate_all_accessible_channels_local(tenant.community());
        self.spawn_cache_invalidation(tenant, CacheInvalidation::AccessibleAll);
    }

    /// Local-only accessible-channels drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_all_accessible_channels_local(&self, community_id: CommunityId) {
        if let Err(error) = self
            .accessible_channels_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            // AppState enables invalidation closures at construction time. If
            // that invariant ever regresses, prefer over-invalidating to
            // serving stale access state.
            tracing::error!(
                ?error,
                "community-scoped accessible-channel invalidation unavailable; falling back to full invalidation"
            );
            self.accessible_channels_cache.invalidate_all();
        }
    }

    /// Invalidate the cached visibility for a single channel (e.g. after a flip).
    pub fn invalidate_channel_visibility(&self, tenant: &TenantContext, channel_id: Uuid) {
        self.invalidate_channel_visibility_local(tenant.community(), channel_id);
        self.spawn_cache_invalidation(tenant, CacheInvalidation::Visibility { channel_id });
    }

    /// Local-only visibility drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_channel_visibility_local(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) {
        self.channel_visibility_cache
            .invalidate(&(community_id, channel_id));
    }

    /// Invalidate all caches after a channel is deleted.
    ///
    /// Channel deletion is a rare admin operation, but it is still tenant-local:
    /// a deletion in A must not flush B's cache entries. Predicate invalidation
    /// keeps the safety property that stale `is_member=true` entries for the
    /// deleted channel are removed without turning the cache drop into a
    /// cross-community signal.
    pub fn invalidate_channel_deleted(&self, tenant: &TenantContext) {
        self.invalidate_channel_deleted_local(tenant.community());
        self.spawn_cache_invalidation(tenant, CacheInvalidation::ChannelDeleted);
    }

    /// Local-only channel-deleted drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_channel_deleted_local(&self, community_id: CommunityId) {
        if let Err(error) =
            self.membership_cache
                .invalidate_entries_if(move |(entry_community, _, _), _| {
                    *entry_community == community_id
                })
        {
            tracing::error!(
                ?error,
                "community-scoped membership invalidation unavailable; falling back to full invalidation"
            );
            self.membership_cache.invalidate_all();
        }
        if let Err(error) = self
            .accessible_channels_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            tracing::error!(
                ?error,
                "community-scoped accessible-channel invalidation unavailable; falling back to full invalidation"
            );
            self.accessible_channels_cache.invalidate_all();
        }
        if let Err(error) = self
            .channel_visibility_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            tracing::error!(
                ?error,
                "community-scoped visibility invalidation unavailable; falling back to full invalidation"
            );
            self.channel_visibility_cache.invalidate_all();
        }
    }

    /// Fire-and-forget publish of a cache-key drop to all other pods. Failures
    /// are logged and swallowed — the REQ denial-path DB confirmation is the
    /// backstop, so a missed publish degrades to a <=10s TTL wait, never a leak.
    fn spawn_cache_invalidation(&self, tenant: &TenantContext, invalidation: CacheInvalidation) {
        let pubsub = Arc::clone(&self.pubsub);
        let tenant = tenant.clone();
        tokio::spawn(async move {
            if let Err(e) = pubsub
                .publish_cache_invalidation(&tenant, &invalidation)
                .await
            {
                tracing::warn!("Failed to publish cache invalidation {invalidation:?}: {e}");
            }
        });
    }

    /// Apply a cache-key drop received from another pod. Calls the local-only
    /// drop variants so a received drop is never re-published (no fan-out loop).
    pub fn apply_cache_invalidation(
        &self,
        community_id: CommunityId,
        invalidation: CacheInvalidation,
    ) {
        match invalidation {
            CacheInvalidation::Membership { channel_id, pubkey } => {
                self.invalidate_membership_local(community_id, channel_id, &pubkey);
            }
            CacheInvalidation::AccessibleAll => {
                self.invalidate_all_accessible_channels_local(community_id);
            }
            CacheInvalidation::Visibility { channel_id } => {
                self.invalidate_channel_visibility_local(community_id, channel_id);
            }
            CacheInvalidation::ChannelDeleted => {
                self.invalidate_channel_deleted_local(community_id);
            }
        }
    }

    /// Close everything `pubkey` has open in `community` on this pod: root
    /// sockets (with a final `OK false` carrying `reason`) and audio sockets.
    ///
    /// The pod-local half of [`Self::disconnect_pubkey_clusterwide`], and what
    /// the conn-control subscriber runs for a remote pod's publish.
    pub fn disconnect_pubkey_local(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        event_id: &str,
        reason: &str,
    ) -> usize {
        self.conn_manager
            .disconnect_pubkey(community, pubkey, event_id, reason)
            + self
                .community_connections
                .disconnect_pubkey(community, pubkey)
    }

    /// Enforce a live ban cluster-wide: close this pod's sockets for `pubkey`
    /// now (fenced to `tenant`'s community) and fan the same disconnect out to
    /// every other pod over the conn-control Redis channel.
    ///
    /// This is the single entry point for live ban enforcement (decision 4:
    /// "a ban takes effect immediately, everywhere, including live sessions").
    /// Callers must not invoke the pod-local [`Self::disconnect_pubkey_local`]
    /// directly — doing so closes sockets only on the pod that processed the
    /// ban and silently drops the cluster-wide half. Pairing both halves here
    /// makes that mistake unrepresentable.
    ///
    /// Returns the number of sockets closed on *this* pod only — remote pods
    /// close asynchronously and do not report back, so callers must not treat
    /// the count as cluster-wide truth. The cross-pod publish is fire-and-forget
    /// (mirrors [`Self::spawn_cache_invalidation`]): the DB ban row is the
    /// durable backstop, so a dropped publish still refuses the banned member's
    /// next auth and next write.
    pub fn disconnect_pubkey_clusterwide(
        &self,
        tenant: &TenantContext,
        pubkey: &[u8],
        event_id: &str,
        reason: &str,
    ) -> usize {
        let closed = self.disconnect_pubkey_local(tenant.community(), pubkey, event_id, reason);

        // The banning pod re-receives its own publish through the subscriber and
        // no-ops (its local sockets are already closed above) — intentional; do
        // not add origin-suppression, it buys nothing.
        let pubsub = Arc::clone(&self.pubsub);
        let tenant = tenant.clone();
        let command = ConnControl::DisconnectPubkey {
            pubkey: pubkey.to_vec(),
            event_id: event_id.to_string(),
            reason: reason.to_string(),
        };
        // This pre-existing ban path may remain fire-and-forget because the
        // durable ban row rejects the member again at auth. Community archival
        // is different: its API awaits publication and live sockets also have a
        // periodic durable-state revalidation backstop below.
        tokio::spawn(async move {
            if let Err(e) = pubsub.publish_conn_control(&tenant, &command).await {
                tracing::warn!("Failed to publish conn-control disconnect: {e}");
            }
        });

        closed
    }

    /// Disconnect a community locally and publish the command to every relay pod.
    ///
    /// Publication is awaited so the archive API can distinguish durable state
    /// from propagation completion and offer a retryable response on failure.
    pub async fn disconnect_community_clusterwide(
        &self,
        tenant: &TenantContext,
    ) -> Result<usize, buzz_pubsub::PubSubError> {
        let closed = self
            .community_connections
            .disconnect_community(tenant.community());
        self.community_disconnect_publish_attempts
            .fetch_add(1, Ordering::Relaxed);
        self.pubsub
            .publish_conn_control(tenant, &ConnControl::DisconnectCommunity)
            .await?;
        Ok(closed)
    }

    /// Revalidate all communities with live sockets and cancel inactive ones.
    ///
    /// This is the durable backstop for Redis pub/sub's lossy offline-subscriber
    /// semantics: a pod that missed a successful publish eventually observes the
    /// archived row directly.
    pub async fn revalidate_live_communities(&self) -> usize {
        let (closed, failures) =
            revalidate_registered_communities(&self.community_connections, |community_id| {
                self.db.is_community_active_for_maintenance(community_id)
            })
            .await;
        for (community_id, error) in failures {
            tracing::warn!(%community_id, %error, "community lifecycle revalidation failed; retaining its sockets until next tick");
        }
        closed
    }

    /// Get accessible channel IDs with a 10-second cache. Falls back to DB on miss.
    pub async fn get_accessible_channel_ids_cached(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>, buzz_db::DbError> {
        let key = (community_id, pubkey.to_vec());
        if let Some(cached) = self.accessible_channels_cache.get(&key) {
            metrics::counter!("buzz_accessible_channels_cache_hits_total").increment(1);
            return Ok(cached);
        }
        metrics::counter!("buzz_accessible_channels_cache_misses_total").increment(1);
        let result = self
            .db
            .get_accessible_channel_ids(community_id, pubkey)
            .await?;
        self.accessible_channels_cache.insert(key, result.clone());
        Ok(result)
    }

    /// Channel visibility string. Caches only `private` (10s); never caches a
    /// non-private value.
    ///
    /// The fan-out access gate fails open on a non-private result, so a stale
    /// cached `open` on another node would mask the filter for the whole TTL
    /// after an open->private flip (no cross-node cache invalidation). Caching
    /// only `private` keeps the cache fail-safe: the worst stale entry is an
    /// over-restrictive `private` (drops non-members on a now-open channel for
    /// <=10s), never a leak.
    ///
    /// `prefetched` lets a caller that already holds the channel row for this
    /// request (ingest's once-per-request fetch, E1 §4.8) reuse it instead of
    /// re-SELECTing. The gate is unchanged: a cached `private` still wins over
    /// the prefetched row (the cache is fail-safe by design), and a `private`
    /// read from the row still populates the cache. With `Some(row)` this
    /// method performs no DB I/O and cannot error.
    pub async fn channel_visibility_cached(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        prefetched: Option<&buzz_db::channel::ChannelRecord>,
    ) -> Result<String, buzz_db::DbError> {
        if let Some(cached) = self
            .channel_visibility_cache
            .get(&(community_id, channel_id))
        {
            return Ok(cached);
        }
        let visibility = match prefetched {
            Some(row) => row.visibility.clone(),
            None => {
                self.db
                    .get_channel(community_id, channel_id)
                    .await?
                    .visibility
            }
        };
        if visibility == "private" {
            self.channel_visibility_cache
                .insert((community_id, channel_id), visibility.clone());
        }
        Ok(visibility)
    }
}

/// A channel-visibility read resolved at ingest and threaded through to
/// fan-out within the same request (E1 phase-2, §4.8 phase-2 addendum).
///
/// The community and channel ids the visibility was resolved under travel
/// with the value so it can never be consulted for a different channel or
/// community's fan-out (channel UUIDs collide across communities —
/// `Inv_LabelPropagation`). Consumers must treat a missing/mismatched bundle
/// as "no threaded visibility" and fall back to a fresh fail-closed lookup —
/// never as "assume open".
#[derive(Debug, Clone)]
pub struct ThreadedChannelVisibility {
    /// Community the visibility was resolved under (server-resolved tenant).
    pub community_id: CommunityId,
    /// Channel the visibility was resolved for.
    pub channel_id: Uuid,
    /// The visibility string read at ingest (`"open"` / `"private"` / ...).
    pub visibility: String,
}

/// Handle for graceful audit worker shutdown.
///
/// Signals the worker to stop accepting new entries, drain its buffer,
/// and exit. Independent of `Arc<AppState>` lifetime — works even when
/// background tasks (reaper, pubsub, health) still hold state clones.
pub struct AuditShutdownHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl AuditShutdownHandle {
    /// Signal the audit worker to drain and wait up to `timeout` for it to finish.
    pub async fn drain(self, timeout: std::time::Duration) {
        self.cancel.cancel();
        match tokio::time::timeout(timeout, self.handle).await {
            Ok(Ok(())) => tracing::info!("Audit worker drained cleanly"),
            Ok(Err(e)) => tracing::error!("Audit worker panicked: {e}"),
            Err(_) => tracing::error!(
                ?timeout,
                "Audit worker did not drain in time — exiting anyway"
            ),
        }
    }
}

/// Construct the NIP-FI assertion verifier + JWKS source from `config.nip_fi`.
///
/// Returns `(None, None)` when the mode is `Off` or `DenyProtected` (the
/// verifier is never consulted there; admission always returns 503). In
/// `Enforce` mode, constructs a `ProductionJwksSource` (shared via `Arc`)
/// and a `FederatedAssertionVerifier` over a clone of that `Arc`.
/// The source starts empty; HTTP admission returns `authorization_unavailable`
/// (503) until the startup warm in `main.rs` succeeds. [FI-TRACE-DEPENDENCY-FAIL-CLOSED]
type NipFiComponents = (
    Option<Arc<dyn buzz_auth::VerifyAssertion>>,
    Option<Arc<buzz_auth::ProductionJwksSource>>,
);

fn build_nip_fi_components(config: &crate::config::Config) -> NipFiComponents {
    use buzz_auth::{FederatedAssertionVerifier, HttpJwksFetcher, NipFiMode, ProductionJwksSource};

    if matches!(
        config.nip_fi.mode,
        NipFiMode::Off | NipFiMode::DenyProtected
    ) {
        // Off: no enforcement. DenyProtected: verifier never consulted (always 503).
        return (None, None);
    }

    let source =
        match ProductionJwksSource::new(config.nip_fi.jwks_configs.clone(), HttpJwksFetcher::new())
        {
            Some(s) => Arc::new(s),
            None => {
                tracing::error!(
                    "nip-fi: ProductionJwksSource construction returned None despite \
                     passing startup validation — HTTP enforcement unavailable"
                );
                return (None, None);
            }
        };

    let verifier: Arc<dyn buzz_auth::VerifyAssertion> = Arc::new(FederatedAssertionVerifier::new(
        config.nip_fi.registry.clone(),
        Arc::clone(&source),
    ));

    (Some(verifier), Some(source))
}

/// Log a single audit entry with metrics. Extracted so the normal loop
/// and the post-cancel drain share the same logic.
async fn log_audit_entry(audit: &buzz_audit::AuditService, entry: buzz_audit::NewAuditEntry) {
    let t = std::time::Instant::now();
    let mut retry_delay_ms = 50u64;
    let mut retries = 0u64;
    loop {
        match audit.log(entry.clone()).await {
            Ok(_) => {
                metrics::histogram!("buzz_audit_log_seconds").record(t.elapsed().as_secs_f64());
                return;
            }
            Err(buzz_audit::AuditError::Database(sqlx::Error::Database(database_error)))
                if database_error.code().as_deref() == Some("55P03") =>
            {
                retries += 1;
                metrics::counter!("buzz_audit_log_lock_retries_total").increment(1);
                tracing::warn!(
                    retries,
                    retry_delay_ms,
                    "Audit advisory lock timed out; preserving entry for retry"
                );
                tokio::time::sleep(std::time::Duration::from_millis(retry_delay_ms)).await;
                retry_delay_ms = (retry_delay_ms * 2).min(1_000);
            }
            Err(error) => {
                metrics::counter!("buzz_audit_log_errors_total").increment(1);
                tracing::error!("Audit log failed: {error}");
                return;
            }
        }
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("relay_url", &self.config.relay_url)
            .field("max_connections", &self.config.max_connections)
            .finish()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::connection::{AuthState, ConnectionState};
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    /// Helper: create a ConnectionManager with one registered connection.
    /// Returns (manager, conn_id, receiver, ctrl_receiver, cancel,
    /// shared_backpressure_count).
    fn setup_conn(
        buffer_size: usize,
    ) -> (
        ConnectionManager,
        Uuid,
        mpsc::Receiver<WsMessage>,
        mpsc::Receiver<WsMessage>,
        CancellationToken,
        Arc<AtomicU8>,
    ) {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(buffer_size);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(buffer_size);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );
        (mgr, conn_id, rx, ctrl_rx, cancel, bp)
    }

    // ── NIP-FI S4 disconnect/deny witnesses ──
    #[test]
    fn conn_manager_disconnect_nip_fi_ignores_unproven_connection() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let pubkey = vec![0xabu8; 32];

        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let reason_rx = control.disconnect_reason();

        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            control,
        );
        // No set_authenticated_pubkey — simulates pre-NIP-42 state.

        let closed = mgr.disconnect_nip_fi("test-issuer", &pubkey);

        assert_eq!(closed, 0, "unproven connection must not be closed");
        assert!(!cancel.is_cancelled(), "unproven connection must stay live");
        assert_eq!(
            *reason_rx.borrow(),
            None,
            "reason must remain None for untouched connection",
        );
    }

    // ── Issuer scope: a deny entry keyed (A, K) closes only sessions admitted
    // under A.  A same-key session admitted under B (or with no assertion) is
    // untouched — no cancel, no frame.  [FI-TRACE-DENY-SET]
    #[test]
    fn conn_manager_disconnect_nip_fi_is_issuer_scoped() {
        let mgr = ConnectionManager::new();
        let key = vec![0xabu8; 32];
        let register = |issuer: Option<&str>| {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let (terminal_ctrl_tx, terminal_ctrl_rx) = mpsc::channel(1);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                terminal_ctrl_tx,
                None,
                cancel.clone(),
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
                CommunityConnectionControl::new(cancel.clone()),
            );
            mgr.set_authenticated_identity(conn_id, key.clone(), issuer.map(str::to_owned));
            (cancel, terminal_ctrl_rx)
        };
        let (cancel_a, mut frames_a) = register(Some("https://issuer-a.example"));
        let (cancel_b, mut frames_b) = register(Some("https://issuer-b.example"));
        let (cancel_none, mut frames_none) = register(None);

        assert_eq!(mgr.disconnect_nip_fi("https://issuer-a.example", &key), 1);

        assert!(cancel_a.is_cancelled(), "A session must be closed");
        assert!(
            frames_a.try_recv().is_ok(),
            "A session must receive the denial frame"
        );
        assert!(!cancel_b.is_cancelled(), "B session must survive an A deny");
        assert!(
            frames_b.try_recv().is_err(),
            "B session must not get a frame"
        );
        assert!(
            !cancel_none.is_cancelled(),
            "no-assertion session must survive"
        );
        assert!(
            frames_none.try_recv().is_err(),
            "no-assertion session must not get a frame"
        );
    }

    // ── F10: ConnectionManager::disconnect_nip_fi sets AuthorizationDenied ────
    //
    // When the deny-API closes an active root-WS connection via
    // `disconnect_nip_fi`, the `nip_fi_reason_tx` inside `CommunityConnectionControl`
    // must be set to `AuthorizationDenied` before the cancellation fires.
    // The send loop reads this reason via `disconnect_reason.borrow()` and
    // emits a 1008 POLICY close frame instead of a bare Close(None).
    // [FI-TRACE-CLOSE-CODE]
    #[test]
    fn conn_manager_disconnect_nip_fi_sets_authorization_denied_reason() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let pubkey = vec![0xabu8; 32];

        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let reason_rx = control.disconnect_reason();

        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            terminal_ctrl_tx,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            control,
        );
        mgr.set_authenticated_identity(conn_id, pubkey.clone(), Some("test-issuer".to_owned()));

        let closed = mgr.disconnect_nip_fi("test-issuer", &pubkey);

        assert_eq!(closed, 1, "one matching connection must be closed");
        assert!(cancel.is_cancelled(), "connection token must be cancelled");
        assert_eq!(
            *reason_rx.borrow(),
            Some(CommunityDisconnectReason::AuthorizationDenied),
            "reason must be AuthorizationDenied so the send loop emits 1008 POLICY",
        );
        // Winner-only enqueue: the denial frame is enqueued on terminal_ctrl_tx.
        let frame = terminal_ctrl_rx
            .try_recv()
            .expect("denial frame must be enqueued");
        let WsMessage::Text(text) = frame else {
            panic!("expected Text frame, got {:?}", frame);
        };
        assert!(
            text.contains("authorization denied"),
            "denial frame must contain 'authorization denied'; got: {text}"
        );
    }

    #[test]
    fn nip_fi_disconnect_audio_is_issuer_scoped() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xce));
        let key = vec![0x42u8; 32];
        let register = |issuer: Option<&str>| {
            let cancel = CancellationToken::new();
            let (terminal_tx, terminal_rx) = mpsc::channel::<WsMessage>(1);
            let control = CommunityConnectionControl::new(cancel.clone());
            control.set_proven_identity(key.clone(), issuer.map(str::to_owned));
            control.set_terminal_frame_sender(terminal_tx);
            let guard = registry.register(Uuid::new_v4(), community, control);
            (cancel, terminal_rx, guard)
        };
        let (cancel_a, mut frames_a, _ga) = register(Some("https://issuer-a.example"));
        let (cancel_b, mut frames_b, _gb) = register(Some("https://issuer-b.example"));
        let (cancel_none, mut frames_none, _gn) = register(None);

        assert_eq!(
            registry.disconnect_nip_fi("https://issuer-a.example", &key),
            1
        );

        assert!(cancel_a.is_cancelled(), "A audio session must be closed");
        assert!(
            frames_a.try_recv().is_ok(),
            "A audio session must receive the denial frame"
        );
        assert!(
            !cancel_b.is_cancelled(),
            "B audio session must survive an A deny"
        );
        assert!(
            frames_b.try_recv().is_err(),
            "B audio session must not get a frame"
        );
        assert!(
            !cancel_none.is_cancelled(),
            "no-assertion audio session must survive"
        );
        assert!(
            frames_none.try_recv().is_err(),
            "no-assertion audio session must not get a frame"
        );
    }

    #[test]
    fn nip_fi_disconnect_closes_proven_audio_socket_and_sends_policy_close_reason() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xca));
        let target_pubkey = vec![0x42u8; 32];

        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let reason_rx = control.disconnect_reason();
        control.set_proven_identity(target_pubkey.clone(), Some("test-issuer".to_owned()));
        let _guard = registry.register(Uuid::new_v4(), community, control);

        assert_eq!(registry.disconnect_nip_fi("test-issuer", &target_pubkey), 1);
        assert!(cancel.is_cancelled(), "audio socket must be cancelled");
        assert_eq!(
            *reason_rx.borrow(),
            Some(CommunityDisconnectReason::AuthorizationDenied),
            "close reason must be AuthorizationDenied so send_loop sends 1008"
        );
    }

    #[test]
    fn nip_fi_disconnect_closes_target_audio_only_and_preserves_collocated_peer() {
        // Two audio sockets in the same community: only the target's is closed.
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xcd));
        let target_pubkey = vec![0x42u8; 32];
        let peer_pubkey = vec![0x55u8; 32];

        let target_cancel = CancellationToken::new();
        let target_control = CommunityConnectionControl::new(target_cancel.clone());
        target_control.set_proven_identity(target_pubkey.clone(), Some("test-issuer".to_owned()));
        let _target_guard = registry.register(Uuid::new_v4(), community, target_control);

        let peer_cancel = CancellationToken::new();
        let peer_control = CommunityConnectionControl::new(peer_cancel.clone());
        peer_control.set_proven_identity(peer_pubkey, Some("test-issuer".to_owned()));
        let _peer_guard = registry.register(Uuid::new_v4(), community, peer_control);

        assert_eq!(registry.disconnect_nip_fi("test-issuer", &target_pubkey), 1);
        assert!(
            target_cancel.is_cancelled(),
            "target audio socket must be cancelled"
        );
        assert!(
            !peer_cancel.is_cancelled(),
            "collocated peer must remain connected"
        );
    }

    #[test]
    fn nip_fi_disconnect_does_not_close_different_pubkey_audio_socket() {
        // A socket whose proven pubkey is different from the target must not
        // be closed — the scan must be key-exact.
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xcc));
        let target_pubkey = vec![0x42u8; 32];
        let other_pubkey = vec![0x99u8; 32];

        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        control.set_proven_identity(other_pubkey, Some("test-issuer".to_owned()));
        let _guard = registry.register(Uuid::new_v4(), community, control);

        assert_eq!(registry.disconnect_nip_fi("test-issuer", &target_pubkey), 0);
        assert!(
            !cancel.is_cancelled(),
            "different-key socket must not be touched"
        );
    }

    #[test]
    fn nip_fi_disconnect_does_not_close_unproven_audio_socket() {
        // A socket that registered but has not yet completed NIP-42 auth (no
        // proven pubkey) must not be touched by a targeted disconnect.
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xcb));
        let target_pubkey = vec![0x42u8; 32];

        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        // Intentionally skip set_proven_pubkey — simulates pre-auth state.
        let _guard = registry.register(Uuid::new_v4(), community, control);

        assert_eq!(registry.disconnect_nip_fi("test-issuer", &target_pubkey), 0);
        assert!(
            !cancel.is_cancelled(),
            "pre-auth socket must not be touched"
        );
    }

    #[test]
    fn community_disconnect_then_nip_fi_keeps_community_deleted_reason() {
        // CommunityDeleted fires first, AuthorizationDenied arrives second.
        // The slot must retain CommunityDeleted.
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        let reason_rx = control.disconnect_reason();

        // First writer: CommunityDeleted (via disconnect_community).
        control.disconnect_community();
        // Second writer: AuthorizationDenied — must be ignored (via disconnect_nip_fi).
        control.disconnect_nip_fi();

        assert_eq!(
            *reason_rx.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted),
            "CommunityDeleted (first writer) must not be clobbered by AuthorizationDenied"
        );
    }

    #[test]
    fn disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame() {
        // disconnect_community fires first → wins reason → no payload (community-deleted
        // path is intentionally payload-less).
        // disconnect_nip_fi fires second → loses reason → must NOT enqueue a denial
        // frame against the CommunityDeleted close.
        let (terminal_tx, mut terminal_rx) = tokio::sync::mpsc::channel(1);

        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());
        control.set_terminal_frame_sender(terminal_tx);

        // First writer: disconnect_community.
        control.disconnect_community();
        // Second writer: disconnect_nip_fi — loses reason slot.
        control.disconnect_nip_fi();

        // Reason slot retains CommunityDeleted.
        assert_eq!(
            *control.disconnect_reason().borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted),
            "CommunityDeleted must be retained when community wins reason"
        );

        // No frame queued — losing deny must not send an authorization_denied payload
        // against a community-deleted close.
        assert!(
            terminal_rx.try_recv().is_err(),
            "losing disconnect_nip_fi must not enqueue a denial frame when community wins reason"
        );
    }

    #[test]
    fn manager_disconnect_sets_reason_enqueues_frame_then_cancels() {
        // manager_disconnect_nip_fi on a fresh control sets AuthorizationDenied,
        // enqueues the Root denial frame, and cancels the token.
        let (terminal_tx, mut terminal_rx) = tokio::sync::mpsc::channel(1);
        let cancel = CancellationToken::new();
        let control = CommunityConnectionControl::new(cancel.clone());

        control.manager_disconnect_nip_fi(&terminal_tx);
        assert_eq!(
            *control.disconnect_reason().borrow(),
            Some(CommunityDisconnectReason::AuthorizationDenied),
            "manager_disconnect_nip_fi must set AuthorizationDenied"
        );
        let frame = terminal_rx
            .try_recv()
            .expect("manager_disconnect_nip_fi must enqueue a denial frame when it wins");
        let expected = crate::nip_fi_session::denial_frame(
            crate::nip_fi_session::NipFiWsRoute::Root,
            buzz_auth::DenialClass::AuthorizationDenied,
        );
        assert_eq!(
            frame, expected,
            "enqueued frame must be the Root denial frame"
        );
        assert!(
            cancel.is_cancelled(),
            "manager_disconnect_nip_fi must cancel the token"
        );
    }

    /// A relay state whose Redis is deliberately unreachable, so admission
    /// checks resolve to `AdmissionError::Unavailable` without any live
    /// infrastructure. Shared with `crate::rejection`'s tests.
    pub(crate) async fn test_state() -> Arc<AppState> {
        // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        build_test_state(config, pool).await
    }

    /// The same test state with an explicit database target. This lets handler
    /// tests deterministically exercise fail-closed database seams without
    /// depending on whether a developer has the normal test database running.
    pub(crate) async fn test_state_with_database_url(database_url: &str) -> Arc<AppState> {
        // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.database_url = database_url.to_owned();
        config.read_database_url = None;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(100))
            .connect_lazy(&config.database_url)
            .expect("lazy pg pool");
        build_test_state(config, pool).await
    }

    /// Build test state around a caller-owned writer pool. Production-path
    /// lifecycle tests use this to hold the sole connection as a deterministic
    /// barrier while AUTH waits in the real database acquisition path.
    pub(crate) async fn test_state_with_database_pool(pool: sqlx::PgPool) -> Arc<AppState> {
        // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.read_database_url = None;
        build_test_state(config, pool).await
    }

    async fn build_test_state(config: crate::config::Config, pool: sqlx::PgPool) -> Arc<AppState> {
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

    async fn audit_worker_retries_lock_timeout_until_original_entry_is_appended_once() {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let observer = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect observer pool");
        let application_name = format!("audit-retry-test-{}", Uuid::new_v4());
        let hook_application_name = application_name.clone();
        let audit_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |conn, _meta| {
                let application_name = hook_application_name.clone();
                Box::pin(async move {
                    sqlx::query(
                        "SELECT set_config('application_name', $1, false), \
                                set_config('lock_timeout', '100', false)",
                    )
                    .bind(application_name)
                    .execute(&mut *conn)
                    .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("connect audit pool");

        let community_id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(format!("audit-retry-{community_id}.example"))
            .execute(&observer)
            .await
            .expect("insert test community");
        let object_id = format!("audit-retry-object-{}", Uuid::new_v4());
        let entry = buzz_audit::NewAuditEntry {
            community_id: CommunityId::from_uuid(community_id),
            action: buzz_audit::AuditAction::EventCreated,
            actor_pubkey: Some(vec![0xab; 32]),
            object_id: Some(object_id.clone()),
            detail: serde_json::json!({"test": "lock-timeout-retry"}),
        };

        // Mirrors buzz_audit::service::AUDIT_LOCK_NAMESPACE.
        let lock_key = format!("buzz_audit:{community_id}");
        let mut holder = observer.acquire().await.expect("acquire lock holder");
        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *holder)
            .await
            .expect("hold community audit lock");

        let audit = Arc::new(AuditService::new(audit_pool));
        let worker = tokio::spawn({
            let audit = Arc::clone(&audit);
            async move { log_audit_entry(&audit, entry).await }
        });

        // Observe one timed-out advisory-lock attempt and then a second wait.
        // Releasing during the first wait would not prove that the worker
        // preserved and retried the original queue entry.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut saw_first_wait = false;
            let mut saw_retry_gap = false;
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (\
                         SELECT 1 FROM pg_stat_activity \
                         WHERE application_name = $1 \
                           AND query LIKE 'SELECT pg_advisory_lock%' \
                           AND wait_event = 'advisory'\
                     )",
                )
                .bind(&application_name)
                .fetch_one(&observer)
                .await
                .expect("inspect audit lock waiter");
                if waiting {
                    if saw_retry_gap {
                        break;
                    }
                    saw_first_wait = true;
                } else if saw_first_wait {
                    saw_retry_gap = true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("audit worker never retried after lock_timeout");

        sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *holder)
            .await
            .expect("release community audit lock");
        tokio::time::timeout(std::time::Duration::from_secs(3), worker)
            .await
            .expect("audit worker did not finish after lock release")
            .expect("audit worker task panicked");

        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE community_id = $1 AND object_id = $2",
        )
        .bind(community_id)
        .bind(&object_id)
        .fetch_one(&observer)
        .await
        .expect("count retried audit rows");
        assert_eq!(rows, 1, "the preserved entry must be appended exactly once");

        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND object_id = $2")
            .bind(community_id)
            .bind(&object_id)
            .execute(&observer)
            .await
            .expect("remove test audit row");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community_id)
            .execute(&observer)
            .await
            .expect("remove test community");
    }

    mod postgres_tests {
        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn audit_worker_retries_lock_timeout_until_original_entry_is_appended_once() {
            super::audit_worker_retries_lock_timeout_until_original_entry_is_appended_once().await;
        }
    }

    #[test]
    fn send_to_resets_grace_counter_on_success() {
        let (mgr, id, _rx, _ctrl_rx, _cancel, bp) = setup_conn(16);
        // Simulate prior backpressure.
        bp.store(2, Ordering::Relaxed);
        assert!(mgr.send_to(id, "hello".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            0,
            "successful send should reset counter"
        );
    }

    #[test]
    fn send_to_increments_grace_counter_on_full() {
        // Buffer size 1 — fill it, then the next send is Full.
        let (mgr, id, _rx, _ctrl_rx, cancel, bp) = setup_conn(1);
        assert!(mgr.send_to(id, "fill".into()));
        // Buffer is now full.
        assert!(!mgr.send_to(id, "overflow-1".into()));
        assert_eq!(bp.load(Ordering::Relaxed), 1, "first overflow → count=1");
        assert!(
            !cancel.is_cancelled(),
            "should not cancel on first overflow"
        );

        assert!(!mgr.send_to(id, "overflow-2".into()));
        assert_eq!(bp.load(Ordering::Relaxed), 2);
        assert!(
            !cancel.is_cancelled(),
            "should not cancel on second overflow"
        );
    }

    #[test]
    fn send_to_cancels_after_grace_limit() {
        let (mgr, id, _rx, _ctrl_rx, cancel, _bp) = setup_conn(1);
        assert!(mgr.send_to(id, "fill".into()));
        // Exhaust grace: 3 consecutive Full events (matches grace_limit=3 from setup_conn).
        for _ in 0..3u8 {
            mgr.send_to(id, "overflow".into());
        }
        assert!(
            cancel.is_cancelled(),
            "should cancel after grace_limit overflows"
        );
    }

    #[test]
    fn shared_counter_between_direct_and_fanout() {
        // Verify that ConnectionState::send() and ConnectionManager::send_to()
        // share the same backpressure counter via Arc<AtomicU8>.
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));

        let conn = ConnectionState {
            conn_id,
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: std::sync::Mutex::new(AuthState::Failed),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx: tx.clone(),
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::clone(&bp),
            grace_limit: 3,
            nip_fi_assertion: None,
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
            community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
        };

        let mgr = ConnectionManager::new();
        mgr.register(
            conn_id,
            tx,
            conn.ctrl_tx.clone(),
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::clone(&conn.subscriptions),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        // Fill the buffer via direct send.
        assert!(conn.send("fill".into()));
        // Overflow via fan-out.
        assert!(!mgr.send_to(conn_id, "overflow-fanout".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            1,
            "fan-out overflow increments shared counter"
        );
        // Overflow via direct send.
        assert!(!conn.send("overflow-direct".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            2,
            "direct overflow increments same counter"
        );
        // One more fan-out overflow → should cancel (3 consecutive).
        mgr.send_to(conn_id, "overflow-final".into());
        assert!(
            cancel.is_cancelled(),
            "shared counter reached limit via mixed path"
        );
    }

    #[tokio::test]
    async fn tracks_connections_by_authenticated_pubkey_within_community() {
        let mgr = ConnectionManager::new();
        let community_a = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let (tx_a, _rx_a) = mpsc::channel(1);
        let (ctrl_tx_a, _ctrl_rx_a) = mpsc::channel(1);
        let (tx_b, _rx_b) = mpsc::channel(1);
        let (ctrl_tx_b, _ctrl_rx_b) = mpsc::channel(1);
        mgr.register(
            conn_a,
            tx_a,
            ctrl_tx_a,
            tokio::sync::mpsc::channel(1).0,
            None,
            CancellationToken::new(),
            community_a,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(
                tokio_util::sync::CancellationToken::new(),
            ),
        );
        mgr.register(
            conn_b,
            tx_b,
            ctrl_tx_b,
            tokio::sync::mpsc::channel(1).0,
            None,
            CancellationToken::new(),
            community_b,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(
                tokio_util::sync::CancellationToken::new(),
            ),
        );

        let pubkey = vec![7u8; 32];
        mgr.set_authenticated_pubkey(conn_a, pubkey.clone());
        mgr.set_authenticated_pubkey(conn_b, pubkey.clone());

        assert_eq!(
            mgr.connection_ids_for_pubkey_in_community(community_a, &pubkey),
            vec![conn_a]
        );
        assert_eq!(
            mgr.connection_ids_for_pubkey_in_community(community_b, &pubkey),
            vec![conn_b]
        );
        assert!(mgr.subscriptions_for(conn_a).is_some());
        assert!(mgr.subscriptions_for(conn_b).is_some());
    }

    #[tokio::test]
    async fn pubkey_for_conn_returns_authenticated_pubkey() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        let subscriptions = Arc::new(Mutex::new(HashMap::new()));
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            bp,
            subscriptions,
            3,
            crate::state::CommunityConnectionControl::new(cancel),
        );

        assert_eq!(mgr.pubkey_for_conn(conn_id), None);
        let pubkey = vec![9u8; 32];
        mgr.set_authenticated_pubkey(conn_id, pubkey.clone());
        assert_eq!(mgr.pubkey_for_conn(conn_id), Some(pubkey));
        assert_eq!(mgr.pubkey_for_conn(Uuid::new_v4()), None);
    }

    #[tokio::test]
    async fn accessible_channel_invalidation_is_scoped_to_community() {
        let state = test_state().await;
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let pubkey = vec![7u8; 32];
        let channels_a = vec![Uuid::from_u128(1)];
        let channels_b = vec![Uuid::from_u128(2)];

        state
            .accessible_channels_cache
            .insert((community_a, pubkey.clone()), channels_a);
        state
            .accessible_channels_cache
            .insert((community_b, pubkey.clone()), channels_b.clone());

        state.invalidate_all_accessible_channels_local(community_a);

        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_a, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_b, pubkey.clone())),
            Some(channels_b),
            "A's cache drop must not evict B's accessible-channel entry"
        );
    }

    #[tokio::test]
    async fn channel_deleted_invalidation_is_scoped_to_community() {
        let state = test_state().await;
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let channel_id = Uuid::from_u128(1);
        let pubkey = vec![7u8; 32];

        for community in [community_a, community_b] {
            state
                .membership_cache
                .insert((community, channel_id, pubkey.clone()), true);
            state
                .accessible_channels_cache
                .insert((community, pubkey.clone()), vec![channel_id]);
            state
                .channel_visibility_cache
                .insert((community, channel_id), "private".to_string());
        }

        state.invalidate_channel_deleted_local(community_a);

        assert_eq!(
            state
                .membership_cache
                .get(&(community_a, channel_id, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_a, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .channel_visibility_cache
                .get(&(community_a, channel_id)),
            None
        );
        assert_eq!(
            state
                .membership_cache
                .get(&(community_b, channel_id, pubkey.clone())),
            Some(true)
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_b, pubkey.clone())),
            Some(vec![channel_id])
        );
        assert_eq!(
            state
                .channel_visibility_cache
                .get(&(community_b, channel_id)),
            Some("private".to_string()),
            "A's channel deletion must not evict B's cache entries"
        );
    }

    #[test]
    fn pubkey_disconnect_reaches_only_that_bound_pubkey_in_that_community() {
        let registry = CommunityConnectionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xb));
        let (target, other) = ([1u8; 32], [2u8; 32]);
        let bound = |community, pubkey: Option<[u8; 32]>| {
            let control = CommunityConnectionControl::new(CancellationToken::new());
            if let Some(pubkey) = pubkey {
                control.bind_pubkey(pubkey);
            }
            let guard = registry.register(Uuid::new_v4(), community, control.clone());
            (control, guard)
        };
        let (hit, _g1) = bound(community_a, Some(target));
        let (other_pubkey, _g2) = bound(community_a, Some(other));
        let (other_community, _g3) = bound(community_b, Some(target));
        let (unbound, _g4) = bound(community_a, None);

        assert_eq!(registry.disconnect_pubkey(community_a, &target), 1);
        assert!(hit.cancellation_token().is_cancelled());
        assert_eq!(
            *hit.disconnect_reason().borrow(),
            Some(CommunityDisconnectReason::AccessRevoked)
        );
        for untouched in [other_pubkey, other_community, unbound] {
            assert!(!untouched.cancellation_token().is_cancelled());
        }
    }

    #[test]
    fn community_lifecycle_disconnect_covers_socket_types_and_preserves_tenant_fence() {
        let registry = CommunityConnectionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xb));
        let ordinary_a = CancellationToken::new();
        let audio_a = CancellationToken::new();
        let ordinary_b = CancellationToken::new();
        let ordinary_a_control = CommunityConnectionControl::new(ordinary_a.clone());
        let audio_a_control = CommunityConnectionControl::new(audio_a.clone());
        let ordinary_b_control = CommunityConnectionControl::new(ordinary_b.clone());
        let ordinary_a_reason = ordinary_a_control.disconnect_reason();
        let audio_a_reason = audio_a_control.disconnect_reason();
        let ordinary_b_reason = ordinary_b_control.disconnect_reason();
        let _ordinary_a_guard = registry.register(Uuid::new_v4(), community_a, ordinary_a_control);
        let _audio_a_guard = registry.register(Uuid::new_v4(), community_a, audio_a_control);
        let _ordinary_b_guard = registry.register(Uuid::new_v4(), community_b, ordinary_b_control);

        assert_eq!(registry.disconnect_community(community_a), 2);
        assert!(ordinary_a.is_cancelled());
        assert!(audio_a.is_cancelled());
        assert!(!ordinary_b.is_cancelled());
        assert_eq!(
            *ordinary_a_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
        assert_eq!(
            *audio_a_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
        assert_eq!(*ordinary_b_reason.borrow(), None);
    }

    #[tokio::test]
    async fn register_then_revalidate_closes_both_archive_race_orderings() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xa));

        // Archive wins before durable revalidation: the check observes inactive
        // and the socket body never starts.
        let cancel_before = CancellationToken::new();
        let started_before = Arc::new(AtomicBool::new(false));
        let started_before_run = Arc::clone(&started_before);
        run_registered_community_connection(
            &registry,
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel_before.clone()),
            || async { Ok(false) },
            move |_| async move { started_before_run.store(true, Ordering::SeqCst) },
            || async {},
        )
        .await;
        assert!(cancel_before.is_cancelled());
        assert!(!started_before.load(Ordering::SeqCst));

        // Archive wins after registration but while revalidation is paused: its
        // sweep sees the token, and even an active query result cannot start the
        // socket body afterward.
        let cancel_during = CancellationToken::new();
        let registered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let registered_check = Arc::clone(&registered);
        let resume_check = Arc::clone(&resume);
        let started_during = Arc::new(AtomicBool::new(false));
        let started_during_run = Arc::clone(&started_during);
        let future = run_registered_community_connection(
            &registry,
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel_during.clone()),
            move || async move {
                registered_check.notify_one();
                resume_check.notified().await;
                Ok(true)
            },
            move |_| async move { started_during_run.store(true, Ordering::SeqCst) },
            || async {},
        );
        tokio::pin!(future);
        tokio::select! {
            _ = registered.notified() => {}
            _ = &mut future => panic!("revalidation should be paused"),
        }
        assert_eq!(registry.disconnect_community(community), 1);
        resume.notify_one();
        future.await;
        assert!(cancel_during.is_cancelled());
        assert!(!started_during.load(Ordering::SeqCst));
    }

    /// Fix 3 / Carl 3 / F3: when the cancellation token fires while
    /// `check_active` is in-flight (stalled DB scenario), the socket body must
    /// NOT start even if `check_active` would have returned `Ok(true)`.
    ///
    /// Mutation oracle: remove the `biased; _ = cancel.cancelled() =>` arm from
    /// the `select!` in `run_registered_community_connection` — the test still
    /// passes (the post-check `cancel.is_cancelled()` guard catches it).
    /// Replace the `select!` with the original `check_active().await` — the test
    /// PANICS: the check waits for resume, cancel fires during the wait, but
    /// without the select! the function only checks cancel _after_ the check
    /// returns, so the run closure _would_ still execute if cancel fired at
    /// exactly the wrong moment.
    ///
    /// Actually, to demonstrate the invariant uniquely, we need to show that
    /// cancellation-during-check terminates the connection without waiting for
    /// `check_active` to return. This test proves socket termination is prompt
    /// (the `run_registered_community_connection` future resolves before the
    /// check_active future is released) when cancel fires mid-check.
    #[tokio::test]
    async fn f3_cancellation_during_check_terminates_socket_without_waiting_for_check() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xf3));

        let cancel = CancellationToken::new();
        let started = Arc::new(AtomicBool::new(false));
        let started_run = Arc::clone(&started);

        // The check blocks forever — simulates a stalled DB.
        let release_check = Arc::new(tokio::sync::Notify::new());
        let release_check_clone = Arc::clone(&release_check);
        let check_reached = Arc::new(tokio::sync::Notify::new());
        let check_reached_clone = Arc::clone(&check_reached);

        let cancel_for_task = cancel.clone();
        let future = run_registered_community_connection(
            &registry,
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel.clone()),
            move || async move {
                check_reached_clone.notify_one();
                // Block until released — simulates stalled DB.
                release_check_clone.notified().await;
                Ok(true) // Would admit the socket if the select! weren't there.
            },
            move |_| async move { started_run.store(true, Ordering::SeqCst) },
            || async {},
        );

        tokio::pin!(future);

        // Wait for the check to start, then cancel the token.
        tokio::select! {
            _ = check_reached.notified() => {}
            _ = &mut future => panic!("future must not complete before check starts"),
        }

        // Fire cancellation while check_active is blocked.
        cancel_for_task.cancel();

        // The future must resolve promptly — it must NOT wait for release_check.
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut future)
            .await
            .expect("F3: run_registered_community_connection must resolve promptly on cancel, not wait for stalled check_active");

        // The socket body must never have started.
        assert!(
            !started.load(Ordering::SeqCst),
            "F3: socket body must not start when cancellation fires during check_active"
        );
        assert!(
            cancel.is_cancelled(),
            "F3: cancel token must be cancelled after bootstrap cancellation"
        );

        // Release the stalled check (cleanup) — the future is already done.
        release_check.notify_one();

        // Mutation oracle: comment out the `biased; _ = cancel.cancelled() =>` arm
        // from the select! in run_registered_community_connection. The timeout above
        // would expire (the function waits for the stalled check to return).
    }

    /// Reads one `buzz_community_admission_checks_total` series by exact label set.
    fn admission_counter(
        snapshot: &[(
            metrics_util::CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            metrics_util::debugging::DebugValue,
        )],
        outcome: &str,
    ) -> Option<u64> {
        snapshot.iter().find_map(|(key, _, _, value)| {
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<Vec<_>>();
            if key.key().name() != "buzz_community_admission_checks_total"
                || labels != [("outcome", outcome)]
            {
                return None;
            }
            match value {
                metrics_util::debugging::DebugValue::Counter(count) => Some(*count),
                _ => panic!("community admission checks must be a counter"),
            }
        })
    }

    /// Each deny arm (`Ok(false)`, `Err`) runs `on_not_run` exactly once, so
    /// a queued NIP-FI denial is drained; `Ok(true)` never runs it.
    ///
    /// Mutation oracle: delete `on_not_run().await` from either deny arm →
    /// that arm's count is 0 → RED.
    #[tokio::test]
    async fn on_not_run_runs_once_on_each_deny_arm_and_never_on_admit() {
        use std::sync::atomic::AtomicUsize;
        async fn not_run_count(check: Result<bool, buzz_db::DbError>) -> usize {
            let count = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&count);
            run_registered_community_connection(
                &CommunityConnectionRegistry::new(),
                Uuid::new_v4(),
                CommunityId::from_uuid(Uuid::from_u128(0xb)),
                CommunityConnectionControl::new(CancellationToken::new()),
                || async { check },
                |_| async {},
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                },
            )
            .await;
            count.load(Ordering::SeqCst)
        }
        assert_eq!(not_run_count(Ok(false)).await, 1, "inactive arm");
        assert_eq!(
            not_run_count(Err(buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut))).await,
            1,
            "check-error arm"
        );
        assert_eq!(not_run_count(Ok(true)).await, 0, "admitted socket");
    }

    /// Admission is fail-closed on both non-affirmative outcomes. A confirmed
    /// `Ok(false)` and a lookup `Err` are different diagnoses — the counter
    /// keeps them apart — but neither is proof of current admission, and
    /// `docs/multi-tenant-relay.md` I5 (`Inv_AdmissionFence`) grants read or
    /// membership capability only to an actor *currently* admitted to that
    /// community. Serving AUTH/REQ on an unproven tenant lifecycle is the
    /// failure this guards.
    #[test]
    fn neither_a_confirmed_inactive_community_nor_a_failed_lookup_admits_the_socket() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let (inactive_cancel, inactive_started, error_cancel, error_started, active_started) =
            metrics::with_local_recorder(&recorder, || {
                runtime.block_on(async {
                    let registry = CommunityConnectionRegistry::new();
                    let community = CommunityId::from_uuid(Uuid::from_u128(0xa));

                    let inactive_cancel = CancellationToken::new();
                    let inactive_started = Arc::new(AtomicBool::new(false));
                    let started = Arc::clone(&inactive_started);
                    run_registered_community_connection(
                        &registry,
                        Uuid::new_v4(),
                        community,
                        CommunityConnectionControl::new(inactive_cancel.clone()),
                        || async { Ok(false) },
                        move |_| async move { started.store(true, Ordering::SeqCst) },
                        || async {},
                    )
                    .await;

                    let error_cancel = CancellationToken::new();
                    let error_started = Arc::new(AtomicBool::new(false));
                    let started = Arc::clone(&error_started);
                    run_registered_community_connection(
                        &registry,
                        Uuid::new_v4(),
                        community,
                        CommunityConnectionControl::new(error_cancel.clone()),
                        || async { Err(buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut)) },
                        move |_| async move { started.store(true, Ordering::SeqCst) },
                        || async {},
                    )
                    .await;

                    let active_started = Arc::new(AtomicBool::new(false));
                    let started = Arc::clone(&active_started);
                    run_registered_community_connection(
                        &registry,
                        Uuid::new_v4(),
                        community,
                        CommunityConnectionControl::new(CancellationToken::new()),
                        || async { Ok(true) },
                        move |_| async move { started.store(true, Ordering::SeqCst) },
                        || async {},
                    )
                    .await;

                    (
                        inactive_cancel,
                        inactive_started,
                        error_cancel,
                        error_started,
                        active_started,
                    )
                })
            });

        assert!(
            inactive_cancel.is_cancelled(),
            "a confirmed-inactive community must still cancel its socket"
        );
        assert!(
            !inactive_started.load(Ordering::SeqCst),
            "a confirmed-inactive community must never start the socket body"
        );
        assert!(
            error_cancel.is_cancelled(),
            "a failed active check must cancel its socket, not admit it"
        );
        assert!(
            !error_started.load(Ordering::SeqCst),
            "a failed active check must never start serving AUTH/REQ on an unproven tenant"
        );
        assert!(active_started.load(Ordering::SeqCst));

        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(admission_counter(&snapshot, "inactive"), Some(1));
        assert_eq!(admission_counter(&snapshot, "check_error"), Some(1));
        assert_eq!(admission_counter(&snapshot, "active"), Some(1));

        let label_sets = snapshot
            .iter()
            .filter(|(key, _, _, _)| key.key().name() == "buzz_community_admission_checks_total")
            .map(|(key, _, _, _)| {
                key.key()
                    .labels()
                    .map(|label| label.key().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(label_sets.len(), 3, "outcome is the only dimension");
        assert!(
            label_sets.iter().all(|labels| labels == &["outcome"]),
            "admission telemetry must never carry community, tenant, or error labels: {label_sets:?}"
        );
    }

    #[tokio::test]
    async fn revalidation_continues_after_one_community_lookup_failure() {
        let registry = CommunityConnectionRegistry::new();
        let archived_a = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let failed = CommunityId::from_uuid(Uuid::from_u128(0xb));
        let archived_c = CommunityId::from_uuid(Uuid::from_u128(0xc));
        let cancel_a = CancellationToken::new();
        let cancel_failed = CancellationToken::new();
        let cancel_c = CancellationToken::new();
        let _guard_a = registry.register(
            Uuid::new_v4(),
            archived_a,
            CommunityConnectionControl::new(cancel_a.clone()),
        );
        let _guard_failed = registry.register(
            Uuid::new_v4(),
            failed,
            CommunityConnectionControl::new(cancel_failed.clone()),
        );
        let _guard_c = registry.register(
            Uuid::new_v4(),
            archived_c,
            CommunityConnectionControl::new(cancel_c.clone()),
        );

        let (closed, failures) =
            revalidate_registered_communities(&registry, |community| async move {
                if community == failed {
                    Err(buzz_db::DbError::InvalidData(
                        "injected lookup failure".into(),
                    ))
                } else {
                    Ok(false)
                }
            })
            .await;

        assert_eq!(closed, 2);
        assert!(cancel_a.is_cancelled());
        assert!(!cancel_failed.is_cancelled());
        assert!(cancel_c.is_cancelled());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, failed);
        assert_eq!(
            registry.bound_communities(),
            HashSet::from([archived_a, failed, archived_c])
        );
    }

    #[test]
    fn community_lifecycle_guard_deregisters_on_early_return() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let cancel = CancellationToken::new();
        let guard = registry.register(
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel.clone()),
        );
        assert_eq!(registry.bound_communities(), HashSet::from([community]));

        drop(guard);

        assert!(registry.bound_communities().is_empty());
        assert_eq!(registry.disconnect_community(community), 0);
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn disconnect_pubkey_closes_matching_conns_with_reason() {
        let (mgr, id, _rx, mut ctrl_rx, cancel, _bp) = setup_conn(8);
        let pubkey = vec![3u8; 32];
        mgr.set_authenticated_pubkey(id, pubkey.clone());

        // setup_conn registers the connection under the nil community.
        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
        let closed = mgr.disconnect_pubkey(
            community,
            &pubkey,
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 1, "the one matching connection is closed");
        assert!(
            cancel.is_cancelled(),
            "connection is cancelled (socket close)"
        );
        // The reason frame is queued on the control channel ahead of the close.
        let frame = ctrl_rx.try_recv().expect("reason frame delivered");
        match frame {
            WsMessage::Text(t) => {
                assert!(t.as_str().contains("blocked: banned"), "carries the reason");
                assert!(t.as_str().contains("false"), "is an OK false frame");
            }
            other => panic!("expected text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disconnect_pubkey_ignores_non_matching_conns() {
        let (mgr, id, _rx, _ctrl_rx, cancel, _bp) = setup_conn(8);
        mgr.set_authenticated_pubkey(id, vec![1u8; 32]);

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());
        let closed = mgr.disconnect_pubkey(
            community,
            &[2u8; 32],
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 0, "no connection matches a different pubkey");
        assert!(!cancel.is_cancelled(), "unrelated connection stays live");
    }

    #[tokio::test]
    async fn disconnect_pubkey_is_fenced_to_the_banning_community() {
        // Same pubkey, two live sockets in two different communities on one pod.
        // A ban in community A must close only A's socket, never B's — the
        // tenant fence on live-disconnect fan-out (B1).
        let mgr = ConnectionManager::new();
        let pubkey = vec![7u8; 32];

        let community_a = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xa));
        let community_b = buzz_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xb));

        let register = |community| {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                tokio::sync::mpsc::channel(1).0,
                None,
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
                crate::state::CommunityConnectionControl::new(cancel.clone()),
            );
            mgr.set_authenticated_pubkey(conn_id, pubkey.clone());
            cancel
        };

        let cancel_a = register(community_a);
        let cancel_b = register(community_b);

        let closed = mgr.disconnect_pubkey(
            community_a,
            &pubkey,
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 1, "only the community-A socket is closed");
        assert!(cancel_a.is_cancelled(), "community-A session is closed");
        assert!(
            !cancel_b.is_cancelled(),
            "community-B session stays live — ban does not cross the tenant fence"
        );
    }

    #[tokio::test]
    async fn drain_all_jittered_waits_for_writer_acknowledgement_without_cancelling() {
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (restart_tx, mut restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            Some(restart_tx),
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        let drain_mgr = Arc::clone(&mgr);
        let drain = tokio::spawn(async move { drain_mgr.drain_all_jittered(1).await });
        let restart = restart_rx.recv().await.expect("restart command delivered");
        assert!(!drain.is_finished(), "drain waits for the writer flush");
        restart.flushed.send(true).expect("acknowledge flush");

        assert_eq!(drain.await.expect("drain task"), 1);
        assert!(
            !cancel.is_cancelled(),
            "successful flush does not use cancellation fallback"
        );
    }

    #[tokio::test]
    async fn drain_all_jittered_cancels_when_restart_channel_is_full_or_closed() {
        for keep_receiver in [true, false] {
            let mgr = ConnectionManager::new();
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let (restart_tx, restart_rx) = mpsc::channel(1);
            let (pending_tx, _pending_rx) = tokio::sync::oneshot::channel();
            if keep_receiver {
                restart_tx
                    .try_send(RestartClose {
                        flushed: pending_tx,
                    })
                    .expect("fill restart channel");
            } else {
                drop(restart_rx);
            }
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                tokio::sync::mpsc::channel(1).0,
                Some(restart_tx),
                cancel.clone(),
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
                crate::state::CommunityConnectionControl::new(cancel.clone()),
            );

            assert_eq!(mgr.drain_all_jittered(1).await, 1);
            assert!(
                cancel.is_cancelled(),
                "unavailable writer cancels as fallback"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_all_jittered_cancels_when_flush_ack_times_out() {
        // A writer that accepts the restart command but never acknowledges the
        // flush (e.g. wedged mid-send) must not stall the drain: after
        // RESTART_CLOSE_ACK_TIMEOUT the connection falls back to cancellation.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (restart_tx, mut restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            Some(restart_tx),
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        let drain_mgr = Arc::clone(&mgr);
        let drain = tokio::spawn(async move { drain_mgr.drain_all_jittered(1).await });
        // Take the restart command but hold the ack sender forever.
        let restart = restart_rx.recv().await.expect("restart command delivered");
        assert!(!drain.is_finished(), "drain waits on the ack timeout");
        // Advance past the 5s ack timeout under paused time.
        tokio::time::sleep(RESTART_CLOSE_ACK_TIMEOUT + std::time::Duration::from_millis(1)).await;

        assert_eq!(drain.await.expect("drain task"), 1);
        assert!(
            cancel.is_cancelled(),
            "an un-acknowledged flush falls back to cancellation"
        );
        drop(restart);
    }

    #[tokio::test]
    async fn drain_all_sends_restart_close_and_cancels_every_conn() {
        // Graceful shutdown must tell every live client to reconnect — across
        // all communities — with a 1012 restart close frame queued ahead of
        // the cancel-driven socket close.
        let mgr = ConnectionManager::new();

        let register = |community| {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, ctrl_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                tokio::sync::mpsc::channel(1).0,
                None,
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
                crate::state::CommunityConnectionControl::new(cancel.clone()),
            );
            (ctrl_rx, cancel)
        };

        let (mut ctrl_a, cancel_a) = register(buzz_core::tenant::CommunityId::from_uuid(
            Uuid::from_u128(0xa),
        ));
        let (mut ctrl_b, cancel_b) = register(buzz_core::tenant::CommunityId::from_uuid(
            Uuid::from_u128(0xb),
        ));

        let closed = mgr.drain_all();

        assert_eq!(closed, 2, "every connection is signalled, no tenant fence");
        assert!(cancel_a.is_cancelled(), "community-A session is cancelled");
        assert!(cancel_b.is_cancelled(), "community-B session is cancelled");

        for ctrl_rx in [&mut ctrl_a, &mut ctrl_b] {
            let frame = ctrl_rx.try_recv().expect("close frame delivered");
            match frame {
                WsMessage::Close(Some(close)) => {
                    assert_eq!(
                        close.code,
                        axum::extract::ws::close_code::RESTART,
                        "close code is 1012 Service Restart"
                    );
                    assert_eq!(close.reason.as_str(), "relay restarting");
                }
                other => panic!("expected a restart close frame, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn drain_all_full_control_buffer_still_cancels() {
        // Best-effort delivery: a wedged control channel must not block the
        // drain — the cancel still closes the socket, just without the frame.
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx.clone(),
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );
        // Wedge the 1-slot control channel.
        ctrl_tx
            .try_send(WsMessage::Text("wedge".into()))
            .expect("fill control channel");

        let closed = mgr.drain_all();

        assert_eq!(closed, 1);
        assert!(
            cancel.is_cancelled(),
            "cancel fires even when the close frame cannot be queued"
        );
        // Only the wedge frame is present — the close was dropped, not queued.
        assert!(matches!(
            ctrl_rx.try_recv().expect("wedge frame"),
            WsMessage::Text(_)
        ));
        assert!(ctrl_rx.try_recv().is_err(), "no second frame queued");
    }

    #[tokio::test]
    async fn register_after_drain_self_signals_restart_close_and_cancel() {
        // The shutdown-boundary race: an upgrade accepted before SIGTERM can
        // finish its async admission check and register AFTER drain_all's
        // one-shot snapshot. The sticky drain flag makes that interleaving
        // deterministic — register itself queues the 1012 and cancels, so no
        // late registration can ride out graceful shutdown unclosed.
        let mgr = ConnectionManager::new();

        // Drain with zero connections — sets the sticky flag.
        assert_eq!(mgr.drain_all(), 0);

        // Late registration lands after the snapshot.
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        assert!(
            cancel.is_cancelled(),
            "late registration is cancelled by the sticky drain flag"
        );
        match ctrl_rx.try_recv().expect("close frame delivered") {
            WsMessage::Close(Some(close)) => {
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::RESTART,
                    "late registration still gets the 1012 restart close"
                );
                assert_eq!(close.reason.as_str(), "relay restarting");
            }
            other => panic!("expected a restart close frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn drain_all_is_immediate() {
        // The default (jitter-off) drain queues the frame and cancels
        // synchronously — the frame is present the moment drain_all() returns.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        let closed = mgr.drain_all();

        assert_eq!(closed, 1);
        assert!(cancel.is_cancelled(), "default drain cancels synchronously");
        assert!(
            matches!(
                ctrl_rx
                    .try_recv()
                    .expect("close frame delivered synchronously"),
                WsMessage::Close(Some(_))
            ),
            "the restart close is queued before drain_all() returns"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drain_all_jittered_defers_close_until_within_jitter_window() {
        // With jitter, the close is deferred within the owned drain future.
        // The sticky drain flag is still set immediately, so a late
        // registration self-signals with no delay.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(cancel.clone()),
        );

        let jitter_ms = 20_000u64;
        // Poll the owned drain through its first await. Dropping this future
        // would drop the timers too; the shutdown path must retain and await it.
        let drain = mgr.drain_all_jittered(jitter_ms);
        tokio::pin!(drain);
        assert!(
            futures_util::poll!(&mut drain).is_pending(),
            "jittered drain remains pending while its timers are owned"
        );

        // Not closed yet — the delayed drain is parked on its timer.
        assert!(
            !cancel.is_cancelled(),
            "jittered close is deferred, not synchronous"
        );
        assert!(
            ctrl_rx.try_recv().is_err(),
            "no close frame queued before the delay elapses"
        );

        // A registration racing past the snapshot still self-signals at once,
        // regardless of jitter — clients arriving mid-shutdown are closed now.
        let late_id = Uuid::new_v4();
        let (late_tx, _late_rx) = mpsc::channel(8);
        let (late_ctrl_tx, mut late_ctrl_rx) = mpsc::channel(8);
        let late_cancel = CancellationToken::new();
        mgr.register(
            late_id,
            late_tx,
            late_ctrl_tx,
            tokio::sync::mpsc::channel(1).0,
            None,
            late_cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(late_cancel.clone()),
        );
        assert!(
            late_cancel.is_cancelled(),
            "late registration self-signals immediately, unaffected by jitter"
        );
        assert!(
            matches!(
                late_ctrl_rx.try_recv().expect("late close frame"),
                WsMessage::Close(Some(_))
            ),
            "late registration gets the restart close with no delay"
        );

        // Advance past the whole jitter window; awaiting the owned drain must
        // complete only after the deferred close has fired.
        tokio::time::advance(std::time::Duration::from_millis(jitter_ms + 1)).await;
        assert_eq!(drain.await, 1, "one captured connection drained");

        assert!(
            cancel.is_cancelled(),
            "the jittered connection is closed within the jitter window"
        );
        match ctrl_rx.try_recv().expect("deferred close frame delivered") {
            WsMessage::Close(Some(close)) => {
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::RESTART,
                    "jittered close is still 1012 Service Restart"
                );
                assert_eq!(close.reason.as_str(), "relay restarting");
            }
            other => panic!("expected a restart close frame, got {other:?}"),
        }
    }
}
