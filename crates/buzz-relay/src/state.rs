//! Shared application state — Arc-wrapped, shared across all connections.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, Utf8Bytes as WsUtf8Bytes};
use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use buzz_audit::AuditService;
use buzz_auth::{AuthService, Nip98ReplayGuard};
use buzz_core::tenant::TenantContext;
use buzz_core::CommunityId;
use buzz_db::Db;
use buzz_media::MediaStorage;
use buzz_pubsub::cache_invalidation::CacheInvalidation;
use buzz_pubsub::conn_control::ConnControl;
use buzz_pubsub::{PubSubManager, RedisNip98ReplayGuard};
use buzz_search::SearchService;
use buzz_workflow::WorkflowEngine;
use deadpool_redis;

use crate::audio::AudioRoomManager;
use crate::config::Config;
use crate::connection::ConnectionSubscriptions;
use crate::subscription::SubscriptionRegistry;

pub(crate) type ScopedPubkeyKey = (CommunityId, [u8; 32]);
type SlidingWindowCounter = (u32, Instant);
type ScopedRateLimiter = DashMap<ScopedPubkeyKey, SlidingWindowCounter>;

/// Per-connection entry in the connection manager.
struct ConnEntry {
    tx: mpsc::Sender<WsMessage>,
    /// Control-frame sender, drained ahead of data and before cancel wins in
    /// the send loop. Used to deliver a ban-disconnect frame that must reach
    /// the client before the socket is closed (see [`ConnectionManager::disconnect_pubkey`]).
    ctrl_tx: mpsc::Sender<WsMessage>,
    cancel: CancellationToken,
    /// Community resolved from the connection host at handshake. This is the
    /// receiver-side tenant label fan-out must compare against the event label.
    community_id: CommunityId,
    /// Shared with `ConnectionState` — both direct sends and fan-out
    /// broadcasts track the same consecutive-full counter.
    backpressure_count: Arc<AtomicU8>,
    subscriptions: ConnectionSubscriptions,
    authenticated_pubkey: Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    grace_limit: u8,
}

/// Tracks active WebSocket connections and provides message routing by connection ID.
pub struct ConnectionManager {
    connections: DashMap<Uuid, ConnEntry>,
}

impl ConnectionManager {
    /// Creates a new, empty connection manager.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Registers a connection with its outbound sender, cancellation token,
    /// server-resolved community, shared backpressure counter, mutable
    /// subscription map, and grace limit.
    // Each argument is a distinct per-connection attribute stored verbatim in
    // `ConnEntry`; a params struct would only relocate the same fields.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &self,
        conn_id: Uuid,
        tx: mpsc::Sender<WsMessage>,
        ctrl_tx: mpsc::Sender<WsMessage>,
        cancel: CancellationToken,
        community_id: CommunityId,
        backpressure_count: Arc<AtomicU8>,
        subscriptions: ConnectionSubscriptions,
        grace_limit: u8,
    ) {
        self.connections.insert(
            conn_id,
            ConnEntry {
                tx,
                ctrl_tx,
                cancel,
                community_id,
                backpressure_count,
                subscriptions,
                authenticated_pubkey: Arc::new(std::sync::RwLock::new(None)),
                grace_limit,
            },
        );
    }

    /// Removes a connection from the registry.
    pub fn deregister(&self, conn_id: Uuid) {
        self.connections.remove(&conn_id);
    }

    /// Record the authenticated pubkey for a connection after NIP-42 succeeds.
    pub fn set_authenticated_pubkey(&self, conn_id: Uuid, pubkey_bytes: Vec<u8>) {
        if let Some(entry) = self.connections.get(&conn_id) {
            if let Ok(mut slot) = entry.authenticated_pubkey.write() {
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
    /// Audit event service.
    pub audit: Arc<AuditService>,
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

    /// Bounded channel for audit logging — backpressure instead of unbounded spawns.
    /// Uses .send().await (blocks caller if full) because audit entries must not be lost.
    pub audit_tx: mpsc::Sender<buzz_audit::NewAuditEntry>,
    /// Media storage client (S3/MinIO).
    pub media_storage: Arc<MediaStorage>,
    /// Git object-store backend (content-addressed packs/manifests plus
    /// CAS-guarded manifest pointer). This is the durable git source of truth;
    /// see `api::git::store` and `docs/git-on-object-storage.md`.
    pub git_store: crate::api::git::store::GitStore,
    /// Audio relay room manager — tracks active huddle audio rooms.
    pub audio_rooms: Arc<AudioRoomManager>,
    /// Set to `true` on SIGTERM — readiness probe returns 503.
    pub shutting_down: Arc<AtomicBool>,
    /// Process start time — used by `/_status` endpoint.
    pub started_at: Instant,
    /// Shared, community-scoped NIP-98 replay prevention.
    ///
    /// Correctness boundary for stateless workers: every pod must consult the
    /// same Redis `SET NX EX` seen-set, keyed by resolved community. Do not
    /// replace this with process-local caching; replay freshness must survive
    /// cross-pod routing.
    pub nip98_replay: Arc<dyn Nip98ReplayGuard>,

    /// Per-agent sliding-window rate limiter for observer frames (kind 24200).
    /// Key: (community_id, agent pubkey bytes). Value: (count, window_start).
    /// 100 events/sec per agent — prevents relay/DB pressure from bursty telemetry.
    pub observer_rate_limiter: Arc<ScopedRateLimiter>,
    /// Per-requester sliding-window rate limiter for mesh connect requests
    /// (kind 24621). Key: (community_id, requester pubkey bytes). Value: (count,
    /// window_start). Bounds the 1→2 call-me-now amplification: a member is
    /// trusted, but a buggy desktop loop shouldn't make the relay sign+fan
    /// unboundedly. 20/sec is far above any real interactive use.
    pub mesh_connect_rate_limiter: Arc<ScopedRateLimiter>,
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
    /// Key: (agent_pubkey_bytes, owner_pubkey_bytes). Value: is_owner.
    /// agent_owner_pubkey is immutable so a long TTL (5 min) is safe.
    /// Prevents repeated DB lookups from bursty observer traffic.
    #[allow(clippy::type_complexity)]
    pub observer_owner_cache: Arc<moka::sync::Cache<(Vec<u8>, Vec<u8>), bool>>,

    /// Runtime conformance tracer. Production binds [`crate::conformance::NoopTracer`]
    /// (zero cost). Conformance tests bind [`crate::conformance::JsonlTracer`] to
    /// record traces for replay against `docs/spec/MultiTenantRelay.tla`.
    /// See `crates/buzz-conformance/` and `crate::conformance` for the
    /// schema, emitter helpers, and the independent checker.
    pub tracer: Arc<dyn buzz_conformance::Tracer>,
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
        audit: AuditService,
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

        let audit_arc = Arc::new(audit);
        let (audit_tx, mut audit_rx) = mpsc::channel::<buzz_audit::NewAuditEntry>(1000);
        let audit_for_worker = Arc::clone(&audit_arc);
        let audit_cancel = CancellationToken::new();
        let audit_cancel_worker = audit_cancel.clone();
        let audit_worker_handle = tokio::spawn(async move {
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
        )
        .expect("media storage was already constructed with this S3 config");
        let nip98_replay: Arc<dyn Nip98ReplayGuard> =
            Arc::new(RedisNip98ReplayGuard::new(redis_pool.clone()));
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
            conn_semaphore: Arc::new(Semaphore::new(max_connections)),
            handler_semaphore: Arc::new(Semaphore::new(max_concurrent_handlers)),
            git_semaphore: Arc::new(Semaphore::new(git_max_concurrent_ops)),
            media_upload_semaphore: Arc::new(Semaphore::new(media_max_concurrent_uploads)),
            workflow_engine,
            relay_keypair,

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
            audit_tx,
            media_storage: Arc::new(media_storage),
            git_store,
            audio_rooms: Arc::new(AudioRoomManager::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            nip98_replay,
            observer_rate_limiter: Arc::new(DashMap::new()),
            mesh_connect_rate_limiter: Arc::new(DashMap::new()),
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
            // Default to NoopTracer: production builds pay zero cost.
            // Conformance tests overwrite this with a JsonlTracer after
            // construction (see test helpers in
            // `crates/buzz-test-client` once those land).
            tracer: Arc::new(crate::conformance::NoopTracer),
        };
        (
            state,
            AuditShutdownHandle {
                cancel: audit_cancel,
                handle: audit_worker_handle,
            },
        )
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

    /// Enforce a live ban cluster-wide: close this pod's sockets for `pubkey`
    /// now (fenced to `tenant`'s community) and fan the same disconnect out to
    /// every other pod over the conn-control Redis channel.
    ///
    /// This is the single entry point for live ban enforcement (decision 4:
    /// "a ban takes effect immediately, everywhere, including live sessions").
    /// Callers must not invoke the pod-local `conn_manager.disconnect_pubkey`
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
        let closed =
            self.conn_manager
                .disconnect_pubkey(tenant.community(), pubkey, event_id, reason);

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
        tokio::spawn(async move {
            if let Err(e) = pubsub.publish_conn_control(&tenant, &command).await {
                tracing::warn!("Failed to publish conn-control disconnect: {e}");
            }
        });

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

/// Log a single audit entry with metrics. Extracted so the normal loop
/// and the post-cancel drain share the same logic.
async fn log_audit_entry(audit: &buzz_audit::AuditService, entry: buzz_audit::NewAuditEntry) {
    let t = std::time::Instant::now();
    if let Err(e) = audit.log(entry).await {
        metrics::counter!("buzz_audit_log_errors_total").increment(1);
        tracing::error!("Audit log failed: {e}");
    } else {
        metrics::histogram!("buzz_audit_log_seconds").record(t.elapsed().as_secs_f64());
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
mod tests {
    use super::*;
    use crate::connection::{AuthState, ConnectionState};
    use std::collections::HashMap;
    use tokio::sync::{Mutex, RwLock};

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
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        (mgr, conn_id, rx, ctrl_rx, cancel, bp)
    }

    async fn test_state() -> Arc<AppState> {
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
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));

        let conn = ConnectionState {
            conn_id,
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Failed),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx: tx.clone(),
            ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::clone(&bp),
            grace_limit: 3,
        };

        let mgr = ConnectionManager::new();
        mgr.register(
            conn_id,
            tx,
            conn.ctrl_tx.clone(),
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::clone(&conn.subscriptions),
            3,
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
            CancellationToken::new(),
            community_a,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        mgr.register(
            conn_b,
            tx_b,
            ctrl_tx_b,
            CancellationToken::new(),
            community_b,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
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
            cancel,
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            bp,
            subscriptions,
            3,
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
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
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
}
