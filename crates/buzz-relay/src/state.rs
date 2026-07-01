//! Shared application state — Arc-wrapped, shared across all connections.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::Message as WsMessage;
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
use buzz_pubsub::{PubSubManager, RedisNip98ReplayGuard};
use buzz_search::SearchService;
use buzz_workflow::WorkflowEngine;
use deadpool_redis;

use crate::audio::AudioRoomManager;
use crate::config::Config;
use crate::connection::ConnectionSubscriptions;
use crate::subscription::SubscriptionRegistry;

/// Per-connection entry in the connection manager.
struct ConnEntry {
    tx: mpsc::Sender<WsMessage>,
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

    /// Return all live connection IDs authenticated as `pubkey_bytes`.
    pub fn connection_ids_for_pubkey(&self, pubkey_bytes: &[u8]) -> Vec<Uuid> {
        self.connections
            .iter()
            .filter_map(|entry| {
                let matches = entry
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
        if let Some(entry) = self.connections.get(&conn_id) {
            let conn = entry.value();
            match conn.tx.try_send(WsMessage::Text(msg.into())) {
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
    /// Key: agent pubkey bytes (32). Value: (count, window_start).
    /// 100 events/sec per agent — prevents relay/DB pressure from bursty telemetry.
    pub observer_rate_limiter: Arc<DashMap<[u8; 32], (u32, Instant)>>,
    /// Per-requester sliding-window rate limiter for mesh connect requests
    /// (kind 24621). Key: requester pubkey bytes (32). Value: (count,
    /// window_start). Bounds the 1→2 call-me-now amplification: a member is
    /// trusted, but a buggy desktop loop shouldn't make the relay sign+fan
    /// unboundedly. 20/sec is far above any real interactive use.
    pub mesh_connect_rate_limiter: Arc<DashMap<[u8; 32], (u32, Instant)>>,
    /// Per-uploader sliding-window rate limiter for media upload starts.
    /// Key: uploader pubkey bytes (32). Value: (count, window_start).
    pub media_upload_rate_limiter: Arc<DashMap<[u8; 32], (u32, Instant)>>,
    /// Current in-flight media uploads per uploader pubkey.
    pub media_uploads_in_flight: Arc<DashMap<[u8; 32], u32>>,
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
                    .build(),
            ),
            accessible_channels_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .build(),
            ),
            channel_visibility_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
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
        self.invalidate_all_accessible_channels_local();
        self.spawn_cache_invalidation(tenant, CacheInvalidation::AccessibleAll);
    }

    /// Local-only accessible-channels drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_all_accessible_channels_local(&self) {
        self.accessible_channels_cache.invalidate_all();
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
    /// Channel deletion is a rare admin operation. We clear the entire membership
    /// cache because moka doesn't support prefix-based invalidation on composite
    /// keys, and stale `is_member=true` entries for a deleted channel would bypass
    /// the DB's `deleted_at IS NULL` guard.
    pub fn invalidate_channel_deleted(&self, tenant: &TenantContext) {
        self.invalidate_channel_deleted_local();
        self.spawn_cache_invalidation(tenant, CacheInvalidation::ChannelDeleted);
    }

    /// Local-only channel-deleted drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_channel_deleted_local(&self) {
        self.membership_cache.invalidate_all();
        self.accessible_channels_cache.invalidate_all();
        self.channel_visibility_cache.invalidate_all();
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
                self.invalidate_all_accessible_channels_local();
            }
            CacheInvalidation::Visibility { channel_id } => {
                self.invalidate_channel_visibility_local(community_id, channel_id);
            }
            CacheInvalidation::ChannelDeleted => {
                self.invalidate_channel_deleted_local();
            }
        }
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
    pub async fn channel_visibility_cached(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<String, buzz_db::DbError> {
        if let Some(cached) = self
            .channel_visibility_cache
            .get(&(community_id, channel_id))
        {
            return Ok(cached);
        }
        let visibility = self
            .db
            .get_channel(community_id, channel_id)
            .await?
            .visibility;
        if visibility == "private" {
            self.channel_visibility_cache
                .insert((community_id, channel_id), visibility.clone());
        }
        Ok(visibility)
    }
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
    /// Returns (manager, conn_id, receiver, cancel, shared_backpressure_count).
    fn setup_conn(
        buffer_size: usize,
    ) -> (
        ConnectionManager,
        Uuid,
        mpsc::Receiver<WsMessage>,
        CancellationToken,
        Arc<AtomicU8>,
    ) {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(buffer_size);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        mgr.register(
            conn_id,
            tx,
            cancel.clone(),
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        (mgr, conn_id, rx, cancel, bp)
    }

    #[test]
    fn send_to_resets_grace_counter_on_success() {
        let (mgr, id, _rx, _cancel, bp) = setup_conn(16);
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
        let (mgr, id, _rx, cancel, bp) = setup_conn(1);
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
        let (mgr, id, _rx, cancel, _bp) = setup_conn(1);
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
    async fn tracks_connections_by_authenticated_pubkey() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        let subscriptions = Arc::new(Mutex::new(HashMap::new()));
        mgr.register(
            conn_id,
            tx,
            cancel,
            buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            bp,
            Arc::clone(&subscriptions),
            3,
        );

        let pubkey = vec![7u8; 32];
        mgr.set_authenticated_pubkey(conn_id, pubkey.clone());

        assert_eq!(mgr.connection_ids_for_pubkey(&pubkey), vec![conn_id]);
        assert!(mgr.subscriptions_for(conn_id).is_some());
    }

    #[tokio::test]
    async fn pubkey_for_conn_returns_authenticated_pubkey() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        let subscriptions = Arc::new(Mutex::new(HashMap::new()));
        mgr.register(
            conn_id,
            tx,
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
}
