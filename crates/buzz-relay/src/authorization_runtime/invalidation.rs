//! Durable authorization invalidation, reconciliation, and use fences.
//!
//! Postgres is authoritative. Redis messages only accelerate reconciliation;
//! startup, polling, lag recovery, and every error path fail closed.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::{
    AuthContext, FederatedAuthorization, FederatedIdentityRequirement, VerificationOnlyDisposition,
};
use buzz_core::{CommunityId, TenantContext};
use buzz_db::authorization_invalidation::{
    AuthorizationInvalidationEntry, AuthorizationInvalidationFloor,
    AuthorizationInvalidationReceipt, AuthorizationInvalidationRequest,
    AuthorizationInvalidationResult, AuthorizationInvalidationSnapshot, AuthorizationSelector,
    AuthorizationSelectorKind,
};
use buzz_db::{Db, DbError};
use buzz_pubsub::authorization_invalidation::{
    AuthorizationInvalidationHint, ScopedAuthorizationInvalidationHint,
    AUTHORIZATION_INVALIDATION_WIRE_VERSION,
};
use buzz_pubsub::PubSubManager;
use dashmap::DashMap;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Default interval between durable reconciliation reads.
pub const DEFAULT_INVALIDATION_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Default maximum age of a successful authority read.
pub const DEFAULT_INVALIDATION_MAX_STALENESS: Duration = Duration::from_secs(15);

/// Runtime polling and freshness bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizationInvalidationConfig {
    poll_interval: Duration,
    max_staleness: Duration,
}

impl AuthorizationInvalidationConfig {
    /// Build non-zero polling and staleness bounds.
    pub fn new(
        poll_interval: Duration,
        max_staleness: Duration,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        if poll_interval.is_zero() || max_staleness.is_zero() {
            return Err(AuthorizationInvalidationRuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            poll_interval,
            max_staleness,
        })
    }

    /// Durable reconciliation interval.
    pub const fn poll_interval(self) -> Duration {
        self.poll_interval
    }

    /// Maximum permitted age of a successful writer-database read.
    pub const fn max_staleness(self) -> Duration {
        self.max_staleness
    }
}

impl Default for AuthorizationInvalidationConfig {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_INVALIDATION_POLL_INTERVAL,
            max_staleness: DEFAULT_INVALIDATION_MAX_STALENESS,
        }
    }
}

/// Fail-closed runtime failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AuthorizationInvalidationRuntimeError {
    /// Configuration contained a zero bound.
    #[error("authorization invalidation configuration is invalid")]
    InvalidConfiguration,
    /// Lease dependencies could not be represented exactly.
    #[error("authorization invalidation dependencies are invalid")]
    InvalidDependencies,
    /// Admission-loss selectors or idempotency evidence were invalid.
    #[error("authorization admission-loss event is invalid")]
    InvalidAdmissionLoss,
    /// No successful startup/recovery snapshot is installed for the domain.
    #[error("authorization invalidation domain is not ready")]
    NotReady,
    /// Durable authority was not read within the configured freshness bound.
    #[error("authorization invalidation authority is stale")]
    Stale,
    /// A matching selector floor invalidated the observed authority.
    #[error("authorization authority was invalidated")]
    Invalidated,
    /// Durable generation or floor state moved backwards.
    #[error("authorization invalidation authority regressed")]
    AuthorityRegressed,
    /// The writer-database read failed.
    #[error("authorization invalidation authority is unavailable")]
    AuthorityUnavailable,
    /// Redis hint publication failed after durable commit.
    #[error("authorization invalidation hint publication failed")]
    HintUnavailable,
    /// The runtime no longer exists.
    #[error("authorization invalidation runtime stopped")]
    RuntimeStopped,
}

/// Provider-neutral description of reversible admission loss.
///
/// The event carries only selector material and an idempotency ID. Its exact
/// authorization domain is supplied separately by a server-resolved
/// [`TenantContext`] when the event is committed.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationAdmissionLoss {
    request: AuthorizationInvalidationRequest,
}

impl AuthorizationAdmissionLoss {
    /// Build one bounded event for exact principal, Nostr-key, or
    /// delegated-owner selectors.
    pub fn new(
        event_id: Uuid,
        selectors: Vec<AuthorizationSelector>,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        let entries = selectors
            .into_iter()
            .map(AuthorizationInvalidationEntry::admission_loss_fence)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidAdmissionLoss)?;
        let request = AuthorizationInvalidationRequest::new(event_id, entries)
            .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidAdmissionLoss)?;
        Ok(Self { request })
    }

    /// Idempotency identifier retained by the durable invalidation authority.
    pub const fn event_id(&self) -> Uuid {
        self.request.event_id()
    }
}

impl fmt::Debug for AuthorizationAdmissionLoss {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationAdmissionLoss")
            .field("event_id", &"[redacted]")
            .field("selector_count", &self.request.entries().len())
            .finish()
    }
}

/// Generation captured immediately before provider evaluation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationEvaluationFence {
    community_id: CommunityId,
    generation: u64,
}

impl AuthorizationEvaluationFence {
    /// Server-resolved authorization domain.
    pub const fn community_id(self) -> CommunityId {
        self.community_id
    }

    /// Durable generation captured before evaluation.
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

impl fmt::Debug for AuthorizationEvaluationFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationEvaluationFence")
            .field("community_id", &"[redacted]")
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Dependency {
    kind: AuthorizationSelectorKind,
    fingerprint: [u8; 32],
    binding_version: Option<u64>,
}

/// Exact selector dependencies represented by one finalized lease and session.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationDependencies {
    community_id: CommunityId,
    selectors: Vec<Dependency>,
}

impl AuthorizationDependencies {
    /// Derive exact invalidation dependencies for a display-only current
    /// direct binding. This registration can withdraw presentation but cannot
    /// authorize access or create a lease.
    pub fn from_verification_only(
        session_id: Uuid,
        disposition: &VerificationOnlyDisposition,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        Self::from_selectors(
            disposition.authorization_domain(),
            vec![
                AuthorizationSelector::nostr_key(disposition.actor_pubkey().to_bytes()),
                AuthorizationSelector::binding(
                    disposition.binding_id(),
                    disposition.binding_version().get(),
                )
                .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
                AuthorizationSelector::session(session_id)
                    .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
                AuthorizationSelector::domain(),
                AuthorizationSelector::policy_version(disposition.policy_version().as_str())
                    .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
            ],
        )
    }

    /// Derive the pre-binding dependencies for one staged direct enrollment.
    /// The transaction revalidates these selectors before it may create the
    /// first binding or membership row.
    pub fn from_enrollment(
        session_id: Uuid,
        disposition: &super::finalization::EnrollmentDisposition,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        let actor = disposition.actor_pubkey().to_bytes();
        Self::from_selectors(
            disposition.authorization_domain(),
            vec![
                AuthorizationSelector::principal(
                    disposition.principal().issuer(),
                    disposition.principal().subject(),
                )
                .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
                AuthorizationSelector::nostr_key(actor),
                AuthorizationSelector::session(session_id)
                    .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
                AuthorizationSelector::domain(),
                AuthorizationSelector::policy_version(disposition.policy_version().as_str())
                    .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
            ],
        )
    }

    /// Derive every invalidation dependency from one finalized enforcing
    /// context and a server-owned non-nil session ID.
    ///
    /// The principal is read from the same active binding that the finalizer
    /// bound to the lease. It is never accepted as an independent argument.
    pub fn from_context(
        session_id: Uuid,
        context: &AuthContext,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        let lease = context
            .authorization_lease()
            .ok_or(AuthorizationInvalidationRuntimeError::InvalidDependencies)?;
        let (binding, delegated_owner) = match context.federated_authorization() {
            FederatedAuthorization::Direct { binding, .. } => {
                if lease.owner_pubkey().is_some()
                    || binding.bound_pubkey() != context.pubkey()
                    || context.agent_owner_pubkey().is_some()
                {
                    return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
                }
                (binding, None)
            }
            FederatedAuthorization::Delegated { owner, .. } => {
                let owner_key = owner.bound_pubkey();
                if lease.owner_pubkey() != Some(owner_key)
                    || context.agent_owner_pubkey() != Some(owner_key)
                {
                    return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
                }
                (owner, Some(owner_key))
            }
            FederatedAuthorization::NotRequired => {
                return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
            }
        };
        if !matches!(
            context.federated_policy().requirement(),
            FederatedIdentityRequirement::Required(_)
        ) || context.tenant().community() != lease.authorization_domain()
            || context.federated_policy().authorization_domain() != lease.authorization_domain()
            || context.transport() != lease.transport()
            || context.pubkey() != lease.actor_pubkey()
            || context.correlation_id() != lease.correlation_id()
            || binding.authorization_domain() != lease.authorization_domain()
            || binding.binding_id() != lease.binding_id()
            || binding.binding_version() != lease.binding_version()
        {
            return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
        }
        let actor = lease.actor_pubkey().to_bytes();
        let mut selectors = vec![
            AuthorizationSelector::principal(
                binding.principal().issuer(),
                binding.principal().subject(),
            )
            .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
            AuthorizationSelector::nostr_key(actor),
            AuthorizationSelector::binding(lease.binding_id(), lease.binding_version().get())
                .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
            AuthorizationSelector::session(session_id)
                .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
            AuthorizationSelector::domain(),
            AuthorizationSelector::policy_version(lease.policy_version().as_str())
                .map_err(|_| AuthorizationInvalidationRuntimeError::InvalidDependencies)?,
        ];
        if let Some(owner) = delegated_owner {
            selectors.extend(delegated_owner_selectors(owner.to_bytes()));
        }
        Self::from_selectors(lease.authorization_domain(), selectors)
    }

    fn from_selectors(
        community_id: CommunityId,
        selectors: Vec<AuthorizationSelector>,
    ) -> Result<Self, AuthorizationInvalidationRuntimeError> {
        if selectors.is_empty() {
            return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
        }
        let mut dependencies = selectors
            .into_iter()
            .map(|selector| Dependency {
                kind: selector.kind(),
                fingerprint: selector.fingerprint(),
                binding_version: selector.binding_version_floor(),
            })
            .collect::<Vec<_>>();
        dependencies.sort_by_key(|dependency| (dependency.kind, dependency.fingerprint));
        dependencies.dedup();
        Ok(Self {
            community_id,
            selectors: dependencies,
        })
    }

    /// Server-resolved domain represented by the dependencies.
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Exact durable selector set carried into a PostgreSQL commit fence.
    pub fn commit_dependencies(
        &self,
    ) -> Result<Vec<super::executor::CommitDependency>, super::executor::AuthorizationExecutionError>
    {
        self.selectors
            .iter()
            .map(|dependency| {
                super::executor::CommitDependency::from_trusted_runtime(
                    dependency.kind,
                    dependency.fingerprint,
                    dependency.binding_version,
                )
            })
            .collect()
    }
}

impl fmt::Debug for AuthorizationDependencies {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationDependencies")
            .field("community_id", &"[redacted]")
            .field("selector_count", &self.selectors.len())
            .finish()
    }
}

type FloorKey = (AuthorizationSelectorKind, [u8; 32]);

#[derive(Default)]
struct CachedDomain {
    ready: bool,
    generation: u64,
    floors: BTreeMap<FloorKey, AuthorizationInvalidationFloor>,
    last_success: Option<Instant>,
}

#[derive(Default)]
struct DomainState {
    cached: RwLock<CachedDomain>,
    reconcile_lock: Mutex<()>,
}

#[derive(Clone)]
struct Registration {
    fence: AuthorizationEvaluationFence,
    dependencies: AuthorizationDependencies,
    cancellation: Option<CancellationToken>,
}

#[async_trait]
trait InvalidationStore: Send + Sync {
    async fn apply(
        &self,
        community_id: CommunityId,
        request: &AuthorizationInvalidationRequest,
    ) -> Result<AuthorizationInvalidationResult, DbError>;

    async fn snapshot(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationInvalidationSnapshot, DbError>;

    async fn delta(
        &self,
        community_id: CommunityId,
        after_generation: u64,
    ) -> Result<AuthorizationInvalidationSnapshot, DbError>;
}

struct DbInvalidationStore(Db);

#[async_trait]
impl InvalidationStore for DbInvalidationStore {
    async fn apply(
        &self,
        community_id: CommunityId,
        request: &AuthorizationInvalidationRequest,
    ) -> Result<AuthorizationInvalidationResult, DbError> {
        self.0
            .apply_authorization_invalidation(community_id, request)
            .await
    }

    async fn snapshot(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationInvalidationSnapshot, DbError> {
        self.0
            .authorization_invalidation_snapshot(community_id)
            .await
    }

    async fn delta(
        &self,
        community_id: CommunityId,
        after_generation: u64,
    ) -> Result<AuthorizationInvalidationSnapshot, DbError> {
        self.0
            .authorization_invalidation_delta(community_id, after_generation)
            .await
    }
}

struct RuntimeInner {
    store: Arc<dyn InvalidationStore>,
    pubsub: Option<Arc<PubSubManager>>,
    config: AuthorizationInvalidationConfig,
    domains: DashMap<CommunityId, Arc<DomainState>>,
    registrations: DashMap<Uuid, Registration>,
    shutdown: CancellationToken,
    healthy: AtomicBool,
    restore: Option<Arc<super::restore::RestoreProtectionRuntime>>,
}

/// Cloneable fail-closed invalidation and reconciliation runtime.
#[derive(Clone)]
pub struct AuthorizationInvalidationRuntime {
    inner: Arc<RuntimeInner>,
}

impl fmt::Debug for AuthorizationInvalidationRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationRuntime")
            .field("domains", &self.inner.domains.len())
            .field("registrations", &self.inner.registrations.len())
            .finish()
    }
}

impl AuthorizationInvalidationRuntime {
    /// Build a runtime backed by the writer database and provider-neutral Redis hints.
    pub fn new(
        db: Db,
        pubsub: Arc<PubSubManager>,
        config: AuthorizationInvalidationConfig,
    ) -> Self {
        Self::with_store(Arc::new(DbInvalidationStore(db)), Some(pubsub), config)
    }

    /// Build a production runtime whose invalidation commits participate in
    /// the independent restore witness protocol.
    pub fn new_with_restore(
        db: Db,
        pubsub: Arc<PubSubManager>,
        config: AuthorizationInvalidationConfig,
        restore: Arc<super::restore::RestoreProtectionRuntime>,
    ) -> Self {
        Self::with_store_and_restore(
            Arc::new(DbInvalidationStore(db)),
            Some(pubsub),
            config,
            Some(restore),
        )
    }

    fn with_store(
        store: Arc<dyn InvalidationStore>,
        pubsub: Option<Arc<PubSubManager>>,
        config: AuthorizationInvalidationConfig,
    ) -> Self {
        Self::with_store_and_restore(store, pubsub, config, None)
    }

    fn with_store_and_restore(
        store: Arc<dyn InvalidationStore>,
        pubsub: Option<Arc<PubSubManager>>,
        config: AuthorizationInvalidationConfig,
        restore: Option<Arc<super::restore::RestoreProtectionRuntime>>,
    ) -> Self {
        Self {
            inner: Arc::new(RuntimeInner {
                store,
                pubsub,
                config,
                domains: DashMap::new(),
                registrations: DashMap::new(),
                shutdown: CancellationToken::new(),
                healthy: AtomicBool::new(true),
                restore,
            }),
        }
    }

    fn domain_state(&self, community_id: CommunityId) -> Arc<DomainState> {
        self.inner
            .domains
            .entry(community_id)
            .or_insert_with(|| Arc::new(DomainState::default()))
            .clone()
    }

    /// Install full durable snapshots before marking protected domains ready.
    pub async fn initialize_domains(
        &self,
        domains: impl IntoIterator<Item = CommunityId>,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        for community_id in domains {
            self.reconcile(community_id, true).await?;
        }
        Ok(())
    }

    /// Whether a domain has a fresh successful authority snapshot.
    pub fn is_ready(&self, community_id: CommunityId) -> bool {
        self.inner.check_ready(community_id).is_ok()
    }

    /// Capture the durable generation immediately before provider evaluation.
    /// A new domain is synchronously bootstrapped from the writer database.
    pub async fn capture_before_evaluation(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationEvaluationFence, AuthorizationInvalidationRuntimeError> {
        if !self.inner.domains.contains_key(&community_id) {
            self.reconcile(community_id, true).await?;
        }
        let generation = self.inner.check_ready(community_id)?;
        Ok(AuthorizationEvaluationFence {
            community_id,
            generation,
        })
    }

    /// Recheck a read-only evaluation fence without registering authority.
    /// This is the mutation-free Shadow/VerifyOnly path.
    pub fn recheck_evaluation(
        &self,
        fence: AuthorizationEvaluationFence,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let current = self.inner.check_ready(fence.community_id)?;
        if current != fence.generation {
            return Err(AuthorizationInvalidationRuntimeError::Invalidated);
        }
        Ok(())
    }

    /// Register finalized authority and immediately recheck the pre-evaluation
    /// fence, closing the race between provider evaluation and registration.
    pub fn observe_authority(
        &self,
        fence: AuthorizationEvaluationFence,
        dependencies: AuthorizationDependencies,
        cancellation: Option<CancellationToken>,
    ) -> Result<AuthorizationObserver, AuthorizationInvalidationRuntimeError> {
        if fence.community_id != dependencies.community_id {
            return Err(AuthorizationInvalidationRuntimeError::InvalidDependencies);
        }
        self.inner.check_observation(fence, &dependencies)?;
        let registration_id = Uuid::new_v4();
        self.inner.registrations.insert(
            registration_id,
            Registration {
                fence,
                dependencies: dependencies.clone(),
                cancellation,
            },
        );
        if let Err(error) = self.inner.check_observation(fence, &dependencies) {
            if let Some((_, registration)) = self.inner.registrations.remove(&registration_id) {
                if let Some(token) = registration.cancellation {
                    token.cancel();
                }
            }
            return Err(error);
        }
        Ok(AuthorizationObserver {
            inner: Arc::downgrade(&self.inner),
            registration_id,
            fence,
            dependencies,
        })
    }

    /// Force a full writer-database reconcile for one domain.
    pub async fn reconcile_domain(
        &self,
        community_id: CommunityId,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        self.reconcile(community_id, true).await
    }

    /// Reconcile local state and publish a hint after an already-durable commit.
    pub async fn publish_committed(
        &self,
        receipt: AuthorizationInvalidationReceipt,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        self.reconcile(receipt.community_id, false).await?;
        let Some(pubsub) = &self.inner.pubsub else {
            return Ok(());
        };
        pubsub
            .publish_authorization_invalidation(
                receipt.community_id,
                AuthorizationInvalidationHint::current(receipt.generation),
            )
            .await
            .map_err(|_| AuthorizationInvalidationRuntimeError::HintUnavailable)?;
        Ok(())
    }

    /// Durably fence reversible admission loss, reconcile locally, and then
    /// advertise the committed generation to other nodes.
    ///
    /// The tenant is the row-zero server-resolved domain boundary. No domain
    /// value is accepted from the provider event itself.
    pub async fn apply_admission_loss(
        &self,
        tenant: &TenantContext,
        event: &AuthorizationAdmissionLoss,
    ) -> Result<AuthorizationInvalidationReceipt, AuthorizationInvalidationRuntimeError> {
        let revocation_started = Instant::now();
        self.inner.check_health()?;
        let community_id = tenant.community();
        let request_fingerprint =
            buzz_db::authorization_invalidation::authorization_invalidation_request_fingerprint(
                community_id,
                &event.request,
            );
        let restore = match &self.inner.restore {
            Some(runtime) => Some(
                runtime
                    .begin(community_id, event.request.event_id(), request_fingerprint)
                    .await
                    .map_err(|_| AuthorizationInvalidationRuntimeError::AuthorityUnavailable)?,
            ),
            None => None,
        };
        let result = match self.inner.store.apply(community_id, &event.request).await {
            Ok(result) => result,
            Err(error) => {
                if let Some(restore) = restore {
                    let _ = restore.abort().await;
                }
                tracing::warn!(%error, "authorization admission-loss commit failed closed");
                self.inner.mark_failed(community_id);
                return Err(AuthorizationInvalidationRuntimeError::AuthorityUnavailable);
            }
        };
        if let Some(restore) = restore {
            // Both variants prove the exact durable receipt. `AlreadyApplied`
            // is a replay, not a rollback, so it must advance the Pending
            // witness to Committed rather than attempting to abort it.
            if let Err(error) = restore
                .commit_invalidation(result.receipt().generation)
                .await
            {
                tracing::warn!(%error, "authorization invalidation witness failed closed");
                self.inner.mark_failed(community_id);
                return Err(AuthorizationInvalidationRuntimeError::AuthorityUnavailable);
            }
        }
        let receipt = result.receipt();
        self.publish_committed(receipt).await?;
        // This is a conservative upper bound: the local cancellation happens
        // during `publish_committed`'s reconcile, before the optional hint is
        // published. No selector, principal, token, or private claim is a label.
        metrics::histogram!("buzz_authorization_revocation_to_enforcement_seconds")
            .record(revocation_started.elapsed().as_secs_f64());
        Ok(receipt)
    }

    /// Poll every known domain once. Lost Redis hints converge here.
    pub async fn poll_once(&self) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let domains = self
            .inner
            .domains
            .iter()
            .map(|entry| *entry.key())
            .collect::<Vec<_>>();
        let mut first_error = None;
        for community_id in domains {
            if let Err(error) = self.reconcile(community_id, false).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Run periodic reconciliation and optional Redis-hint consumption until shutdown.
    /// Durable polling remains active when Redis is absent.
    pub async fn run(&self) {
        let mut hints = self
            .inner
            .pubsub
            .as_ref()
            .map(|pubsub| pubsub.subscribe_authorization_invalidations());
        let mut interval = tokio::time::interval(self.inner.config.poll_interval());
        loop {
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                _ = interval.tick() => {
                    if let Err(error) = self.poll_once().await {
                        tracing::warn!(%error, "authorization invalidation poll failed closed");
                    }
                }
                received = next_hint(&mut hints) => match received {
                    Ok(scoped) => {
                        if let Err(error) = self.handle_hint(scoped).await {
                            tracing::warn!(%error, "authorization invalidation hint reconcile failed closed");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Err(error) = self.reconcile_all(true).await {
                            tracing::warn!(%error, "authorization invalidation lag recovery failed closed");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        self.inner.mark_all_failed();
                        return;
                    }
                }
            }
        }
    }

    /// Stop the runtime loop. Existing observers fail closed once the runtime drops.
    pub fn shutdown(&self) {
        self.inner.shutdown.cancel();
    }

    /// Immediately make every installed domain unavailable.
    ///
    /// Production worker supervision calls this whenever a required worker
    /// exits, including a panic. Existing and future Enforce observations then
    /// fail closed instead of continuing with an unsupervised cache.
    pub fn fail_closed(&self) {
        self.inner.healthy.store(false, Ordering::SeqCst);
        self.inner.mark_all_failed();
    }

    async fn handle_hint(
        &self,
        scoped: ScopedAuthorizationInvalidationHint,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let full = scoped.hint.wire_version != AUTHORIZATION_INVALIDATION_WIRE_VERSION;
        if !full {
            let current = self
                .inner
                .generation(scoped.community_id)
                .unwrap_or_default();
            if scoped.hint.generation <= current {
                return Ok(());
            }
        }
        self.reconcile(scoped.community_id, full).await
    }

    async fn reconcile_all(&self, full: bool) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let domains = self
            .inner
            .domains
            .iter()
            .map(|entry| *entry.key())
            .collect::<Vec<_>>();
        let mut first_error = None;
        for community_id in domains {
            if let Err(error) = self.reconcile(community_id, full).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn reconcile(
        &self,
        community_id: CommunityId,
        full: bool,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        self.inner.check_health()?;
        let state = self.domain_state(community_id);
        let _guard = state.reconcile_lock.lock().await;
        self.inner.check_health()?;
        let current = self.inner.generation(community_id).unwrap_or_default();
        let result = if full {
            self.inner.store.snapshot(community_id).await
        } else {
            self.inner.store.delta(community_id, current).await
        };
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, "authorization invalidation writer read failed");
                self.inner.mark_failed(community_id);
                return Err(AuthorizationInvalidationRuntimeError::AuthorityUnavailable);
            }
        };
        self.inner.install_snapshot(snapshot, full)?;
        self.inner.cancel_invalid(community_id);
        Ok(())
    }
}

fn delegated_owner_selectors(owner: [u8; 32]) -> [AuthorizationSelector; 2] {
    [
        AuthorizationSelector::nostr_key(owner),
        AuthorizationSelector::delegated_owner(owner),
    ]
}

async fn next_hint(
    hints: &mut Option<tokio::sync::broadcast::Receiver<ScopedAuthorizationInvalidationHint>>,
) -> Result<ScopedAuthorizationInvalidationHint, tokio::sync::broadcast::error::RecvError> {
    match hints {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

impl RuntimeInner {
    fn check_health(&self) -> Result<(), AuthorizationInvalidationRuntimeError> {
        if self.healthy.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AuthorizationInvalidationRuntimeError::NotReady)
        }
    }

    fn generation(&self, community_id: CommunityId) -> Option<u64> {
        let state = self.domains.get(&community_id)?;
        state.cached.read().ok().map(|cached| cached.generation)
    }

    fn check_ready(
        &self,
        community_id: CommunityId,
    ) -> Result<u64, AuthorizationInvalidationRuntimeError> {
        self.check_health()?;
        let state = self
            .domains
            .get(&community_id)
            .ok_or(AuthorizationInvalidationRuntimeError::NotReady)?;
        let cached = state
            .cached
            .read()
            .map_err(|_| AuthorizationInvalidationRuntimeError::AuthorityUnavailable)?;
        if !cached.ready {
            return Err(AuthorizationInvalidationRuntimeError::NotReady);
        }
        let last_success = cached
            .last_success
            .ok_or(AuthorizationInvalidationRuntimeError::NotReady)?;
        if Instant::now().saturating_duration_since(last_success) > self.config.max_staleness() {
            return Err(AuthorizationInvalidationRuntimeError::Stale);
        }
        Ok(cached.generation)
    }

    fn check_observation(
        &self,
        fence: AuthorizationEvaluationFence,
        dependencies: &AuthorizationDependencies,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        self.check_health()?;
        let state = self
            .domains
            .get(&fence.community_id)
            .ok_or(AuthorizationInvalidationRuntimeError::NotReady)?;
        let cached = state
            .cached
            .read()
            .map_err(|_| AuthorizationInvalidationRuntimeError::AuthorityUnavailable)?;
        if !cached.ready {
            return Err(AuthorizationInvalidationRuntimeError::NotReady);
        }
        let last_success = cached
            .last_success
            .ok_or(AuthorizationInvalidationRuntimeError::NotReady)?;
        if Instant::now().saturating_duration_since(last_success) > self.config.max_staleness() {
            return Err(AuthorizationInvalidationRuntimeError::Stale);
        }
        if cached.generation < fence.generation {
            return Err(AuthorizationInvalidationRuntimeError::AuthorityRegressed);
        }
        for dependency in &dependencies.selectors {
            let key = (dependency.kind, dependency.fingerprint);
            let Some(floor) = cached.floors.get(&key) else {
                continue;
            };
            let binding_denied = match (floor.binding_version_floor, dependency.binding_version) {
                (Some(floor_version), Some(version)) => version <= floor_version,
                _ => false,
            };
            if floor.sticky_deny || binding_denied || floor.generation > fence.generation {
                return Err(AuthorizationInvalidationRuntimeError::Invalidated);
            }
        }
        Ok(())
    }

    fn install_snapshot(
        &self,
        snapshot: AuthorizationInvalidationSnapshot,
        full: bool,
    ) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let state = self
            .domains
            .get(&snapshot.community_id)
            .map(|entry| entry.clone())
            .ok_or(AuthorizationInvalidationRuntimeError::NotReady)?;
        let mut cached = state
            .cached
            .write()
            .map_err(|_| AuthorizationInvalidationRuntimeError::AuthorityUnavailable)?;
        if self.check_health().is_err() {
            cached.ready = false;
            return Err(AuthorizationInvalidationRuntimeError::NotReady);
        }
        if snapshot.generation < cached.generation
            || snapshot
                .floors
                .iter()
                .any(|floor| floor.generation > snapshot.generation)
        {
            cached.ready = false;
            return Err(AuthorizationInvalidationRuntimeError::AuthorityRegressed);
        }
        let incoming = snapshot
            .floors
            .into_iter()
            .map(|floor| ((floor.kind, floor.fingerprint), floor))
            .collect::<BTreeMap<_, _>>();
        if full
            && cached.ready
            && cached.floors.iter().any(|(key, existing)| {
                incoming.get(key).is_none_or(|next| {
                    next.generation < existing.generation
                        || (existing.sticky_deny && !next.sticky_deny)
                        || next.binding_version_floor < existing.binding_version_floor
                })
            })
        {
            cached.ready = false;
            return Err(AuthorizationInvalidationRuntimeError::AuthorityRegressed);
        }
        if full {
            cached.floors = incoming;
        } else {
            for (key, floor) in incoming {
                if cached.floors.get(&key).is_some_and(|existing| {
                    floor.generation < existing.generation
                        || (existing.sticky_deny && !floor.sticky_deny)
                        || floor.binding_version_floor < existing.binding_version_floor
                }) {
                    cached.ready = false;
                    return Err(AuthorizationInvalidationRuntimeError::AuthorityRegressed);
                }
                cached.floors.insert(key, floor);
            }
        }
        cached.generation = snapshot.generation;
        cached.last_success = Some(Instant::now());
        cached.ready = true;
        Ok(())
    }

    fn mark_failed(&self, community_id: CommunityId) {
        if let Some(state) = self.domains.get(&community_id) {
            if let Ok(mut cached) = state.cached.write() {
                cached.ready = false;
            }
        }
        self.cancel_invalid(community_id);
    }

    fn mark_all_failed(&self) {
        let domains = self
            .domains
            .iter()
            .map(|entry| *entry.key())
            .collect::<Vec<_>>();
        for community_id in domains {
            self.mark_failed(community_id);
        }
    }

    fn cancel_invalid(&self, community_id: CommunityId) {
        let registrations = self
            .registrations
            .iter()
            .filter(|entry| entry.value().fence.community_id == community_id)
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect::<Vec<_>>();
        for (registration_id, registration) in registrations {
            if self
                .check_observation(registration.fence, &registration.dependencies)
                .is_err()
            {
                if let Some((_, removed)) = self.registrations.remove(&registration_id) {
                    if let Some(token) = removed.cancellation {
                        token.cancel();
                    }
                }
            }
        }
    }
}

/// Registered authority observer used for both pre-use and pre-commit checks.
pub struct AuthorizationObserver {
    inner: Weak<RuntimeInner>,
    registration_id: Uuid,
    fence: AuthorizationEvaluationFence,
    dependencies: AuthorizationDependencies,
}

impl AuthorizationObserver {
    /// Fail closed unless the captured authority is still fresh and uninvalidated.
    /// Call immediately before each protected use and again before commit.
    pub fn recheck(&self) -> Result<(), AuthorizationInvalidationRuntimeError> {
        let inner = self
            .inner
            .upgrade()
            .ok_or(AuthorizationInvalidationRuntimeError::RuntimeStopped)?;
        inner.check_observation(self.fence, &self.dependencies)
    }

    /// Pre-evaluation fence retained by this observer.
    pub const fn fence(&self) -> AuthorizationEvaluationFence {
        self.fence
    }

    /// Seal the durable generation and dependencies for transaction-owned use.
    pub fn commit_fence(
        &self,
    ) -> Result<
        super::executor::AuthorizationCommitFence,
        super::executor::AuthorizationExecutionError,
    > {
        super::executor::AuthorizationCommitFence::from_trusted_runtime(
            self.fence.generation(),
            self.dependencies.commit_dependencies()?,
        )
    }
}

impl fmt::Debug for AuthorizationObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationObserver")
            .field("registration_id", &"[redacted]")
            .field("fence", &self.fence)
            .field("dependencies", &self.dependencies)
            .finish()
    }
}

impl Drop for AuthorizationObserver {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.registrations.remove(&self.registration_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[derive(Default)]
    struct FakeStore {
        snapshots: std::sync::Mutex<HashMap<CommunityId, AuthorizationInvalidationSnapshot>>,
        receipts: std::sync::Mutex<
            HashMap<
                (CommunityId, Uuid),
                (
                    AuthorizationInvalidationRequest,
                    AuthorizationInvalidationReceipt,
                ),
            >,
        >,
        fail: AtomicBool,
    }

    impl FakeStore {
        fn set(&self, snapshot: AuthorizationInvalidationSnapshot) {
            self.snapshots
                .lock()
                .expect("fake store lock")
                .insert(snapshot.community_id, snapshot);
        }
    }

    #[async_trait]
    impl InvalidationStore for FakeStore {
        async fn apply(
            &self,
            community_id: CommunityId,
            request: &AuthorizationInvalidationRequest,
        ) -> Result<AuthorizationInvalidationResult, DbError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(DbError::InvalidData("synthetic failure".into()));
            }
            let key = (community_id, request.event_id());
            let mut receipts = self.receipts.lock().expect("fake receipt lock");
            if let Some((stored, receipt)) = receipts.get(&key) {
                if stored != request {
                    return Err(DbError::InvalidData("synthetic event ID collision".into()));
                }
                return Ok(AuthorizationInvalidationResult::AlreadyApplied(*receipt));
            }

            let mut snapshots = self.snapshots.lock().expect("fake store lock");
            let snapshot =
                snapshots
                    .entry(community_id)
                    .or_insert(AuthorizationInvalidationSnapshot {
                        community_id,
                        generation: 0,
                        floors: Vec::new(),
                    });
            snapshot.generation = snapshot
                .generation
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("synthetic generation exhausted".into()))?;
            for entry in request.entries() {
                let selector = entry.selector();
                let binding_version_floor = selector.binding_version_floor();
                if let Some(floor) = snapshot.floors.iter_mut().find(|floor| {
                    floor.kind == selector.kind() && floor.fingerprint == selector.fingerprint()
                }) {
                    floor.generation = snapshot.generation;
                    floor.sticky_deny |= matches!(
                        entry.effect(),
                        buzz_db::authorization_invalidation::AuthorizationInvalidationEffect::StickyDeny
                    );
                    floor.binding_version_floor =
                        match (floor.binding_version_floor, binding_version_floor) {
                            (Some(current), Some(candidate)) => Some(current.max(candidate)),
                            (None, candidate) => candidate,
                            (current, None) => current,
                        };
                } else {
                    snapshot.floors.push(AuthorizationInvalidationFloor {
                        kind: selector.kind(),
                        fingerprint: selector.fingerprint(),
                        generation: snapshot.generation,
                        sticky_deny: matches!(
                            entry.effect(),
                            buzz_db::authorization_invalidation::AuthorizationInvalidationEffect::StickyDeny
                        ),
                        binding_version_floor,
                    });
                }
            }
            let receipt = AuthorizationInvalidationReceipt {
                community_id,
                event_id: request.event_id(),
                generation: snapshot.generation,
            };
            receipts.insert(key, (request.clone(), receipt));
            Ok(AuthorizationInvalidationResult::Applied(receipt))
        }

        async fn snapshot(
            &self,
            community_id: CommunityId,
        ) -> Result<AuthorizationInvalidationSnapshot, DbError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(DbError::InvalidData("synthetic failure".into()));
            }
            Ok(self
                .snapshots
                .lock()
                .expect("fake store lock")
                .get(&community_id)
                .cloned()
                .unwrap_or(AuthorizationInvalidationSnapshot {
                    community_id,
                    generation: 0,
                    floors: Vec::new(),
                }))
        }

        async fn delta(
            &self,
            community_id: CommunityId,
            after_generation: u64,
        ) -> Result<AuthorizationInvalidationSnapshot, DbError> {
            let mut snapshot = self.snapshot(community_id).await?;
            snapshot
                .floors
                .retain(|floor| floor.generation > after_generation);
            Ok(snapshot)
        }
    }

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn runtime(store: Arc<FakeStore>) -> AuthorizationInvalidationRuntime {
        AuthorizationInvalidationRuntime::with_store(
            store,
            None,
            AuthorizationInvalidationConfig::new(Duration::from_millis(5), Duration::from_secs(10))
                .expect("valid config"),
        )
    }

    fn dependencies(
        community_id: CommunityId,
        selector: AuthorizationSelector,
    ) -> AuthorizationDependencies {
        dependencies_for(community_id, vec![selector])
    }

    fn dependencies_for(
        community_id: CommunityId,
        selectors: Vec<AuthorizationSelector>,
    ) -> AuthorizationDependencies {
        let mut all = vec![AuthorizationSelector::domain()];
        all.extend(selectors);
        AuthorizationDependencies::from_selectors(community_id, all).expect("valid dependencies")
    }

    async fn observe(
        runtime: &AuthorizationInvalidationRuntime,
        community_id: CommunityId,
        selectors: Vec<AuthorizationSelector>,
        cancellation: CancellationToken,
    ) -> AuthorizationObserver {
        let fence = runtime
            .capture_before_evaluation(community_id)
            .await
            .expect("capture evaluation fence");
        runtime
            .observe_authority(
                fence,
                dependencies_for(community_id, selectors),
                Some(cancellation),
            )
            .expect("observe authority")
    }

    fn floor(
        selector: &AuthorizationSelector,
        generation: u64,
        sticky_deny: bool,
    ) -> AuthorizationInvalidationFloor {
        AuthorizationInvalidationFloor {
            kind: selector.kind(),
            fingerprint: selector.fingerprint(),
            generation,
            sticky_deny,
            binding_version_floor: selector.binding_version_floor(),
        }
    }

    #[tokio::test]
    async fn two_nodes_converge_after_lost_reordered_and_replayed_hints() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(1);
        let selector = AuthorizationSelector::session(Uuid::new_v4()).expect("valid session");
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 0,
            floors: Vec::new(),
        });
        let node_a = runtime(store.clone());
        let node_b = runtime(store.clone());
        node_a
            .initialize_domains([community_id])
            .await
            .expect("node A starts");
        node_b
            .initialize_domains([community_id])
            .await
            .expect("node B starts");
        let fence = node_b
            .capture_before_evaluation(community_id)
            .await
            .expect("capture fence");
        let cancellation = CancellationToken::new();
        let observer = node_b
            .observe_authority(
                fence,
                dependencies(community_id, selector.clone()),
                Some(cancellation.clone()),
            )
            .expect("observe authority");

        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 2,
            floors: vec![floor(&selector, 2, true)],
        });
        node_a.poll_once().await.expect("node A reconciles");
        assert!(
            observer.recheck().is_ok(),
            "lost hint has not reached node B yet"
        );
        node_b.poll_once().await.expect("poll heals lost hint");
        assert_eq!(
            observer.recheck(),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        );
        assert!(cancellation.is_cancelled());

        node_b
            .handle_hint(ScopedAuthorizationInvalidationHint {
                community_id,
                hint: AuthorizationInvalidationHint::current(1),
            })
            .await
            .expect("delayed hint is harmless");
        node_b
            .handle_hint(ScopedAuthorizationInvalidationHint {
                community_id,
                hint: AuthorizationInvalidationHint::current(2),
            })
            .await
            .expect("replayed hint is harmless");
        assert_eq!(node_b.inner.generation(community_id), Some(2));
    }

    #[tokio::test]
    async fn admission_loss_is_idempotent_and_cancels_two_nodes_before_readmission() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(7);
        let tenant = TenantContext::resolved(community_id, "admission-loss.example");
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 0,
            floors: Vec::new(),
        });
        let node_a = runtime(store.clone());
        let node_b = runtime(store.clone());
        node_a
            .initialize_domains([community_id])
            .await
            .expect("node A starts");
        node_b
            .initialize_domains([community_id])
            .await
            .expect("node B starts");

        let principal =
            AuthorizationSelector::principal("issuer.example", "subject").expect("principal");
        let actor = AuthorizationSelector::nostr_key([7_u8; 32]);
        let owner = AuthorizationSelector::delegated_owner([8_u8; 32]);
        let direct_dependencies = vec![principal.clone(), actor.clone()];
        let delegated_dependencies = vec![principal.clone(), owner.clone()];
        let a_direct_cancel = CancellationToken::new();
        let a_delegated_cancel = CancellationToken::new();
        let b_direct_cancel = CancellationToken::new();
        let b_delegated_cancel = CancellationToken::new();
        let a_direct = observe(
            &node_a,
            community_id,
            direct_dependencies.clone(),
            a_direct_cancel.clone(),
        )
        .await;
        let a_delegated = observe(
            &node_a,
            community_id,
            delegated_dependencies.clone(),
            a_delegated_cancel.clone(),
        )
        .await;
        let b_direct = observe(
            &node_b,
            community_id,
            direct_dependencies.clone(),
            b_direct_cancel.clone(),
        )
        .await;
        let b_delegated = observe(
            &node_b,
            community_id,
            delegated_dependencies.clone(),
            b_delegated_cancel.clone(),
        )
        .await;

        let event_id = Uuid::new_v4();
        let event = AuthorizationAdmissionLoss::new(
            event_id,
            vec![principal.clone(), actor.clone(), owner.clone()],
        )
        .expect("valid provider-neutral admission loss");
        let first = node_a
            .apply_admission_loss(&tenant, &event)
            .await
            .expect("admission loss commits");
        assert_eq!(first.event_id, event_id);
        assert_eq!(first.generation, 1);
        assert!(a_direct_cancel.is_cancelled());
        assert!(a_delegated_cancel.is_cancelled());
        assert_eq!(
            a_direct.recheck(),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        );
        assert_eq!(
            a_delegated.recheck(),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        );
        assert!(!b_direct_cancel.is_cancelled());
        assert!(!b_delegated_cancel.is_cancelled());

        let duplicate = node_a
            .apply_admission_loss(&tenant, &event)
            .await
            .expect("duplicate event reconciles idempotently");
        assert_eq!(duplicate, first);
        assert_eq!(store.snapshot(community_id).await.unwrap().generation, 1);

        node_b
            .handle_hint(ScopedAuthorizationInvalidationHint {
                community_id,
                hint: AuthorizationInvalidationHint::current(1),
            })
            .await
            .expect("current hint reconciles");
        node_b
            .handle_hint(ScopedAuthorizationInvalidationHint {
                community_id,
                hint: AuthorizationInvalidationHint::current(0),
            })
            .await
            .expect("reordered delayed hint is harmless");
        node_b
            .handle_hint(ScopedAuthorizationInvalidationHint {
                community_id,
                hint: AuthorizationInvalidationHint::current(1),
            })
            .await
            .expect("replayed hint is harmless");
        assert!(b_direct_cancel.is_cancelled());
        assert!(b_delegated_cancel.is_cancelled());
        assert_eq!(
            b_direct.recheck(),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        );
        assert_eq!(
            b_delegated.recheck(),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        );

        for (node, selectors) in [
            (&node_a, direct_dependencies),
            (&node_b, delegated_dependencies),
        ] {
            let fence = node
                .capture_before_evaluation(community_id)
                .await
                .expect("capture post-loss generation");
            assert_eq!(fence.generation(), 1);
            let fresh =
                node.observe_authority(fence, dependencies_for(community_id, selectors), None);
            assert!(
                fresh.is_ok(),
                "fresh admission at the committed generation is permitted"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres and S3-compatible object storage"]
    async fn restore_witnessed_admission_loss_replay_is_idempotent_and_fingerprint_bound() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("test migrations");
        let db = Db::from_pool(pool.clone());
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id.as_uuid())
            .bind(format!(
                "restore-invalidation-{}.example",
                Uuid::new_v4().simple()
            ))
            .execute(&pool)
            .await
            .expect("test community");

        let endpoint = std::env::var("BUZZ_S3_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_owned());
        let access_key =
            std::env::var("BUZZ_S3_ACCESS_KEY").unwrap_or_else(|_| "buzz_dev".to_owned());
        let secret_key =
            std::env::var("BUZZ_S3_SECRET_KEY").unwrap_or_else(|_| "buzz_dev_secret".to_owned());
        let bucket = std::env::var("BUZZ_S3_BUCKET").unwrap_or_else(|_| "buzz-git".to_owned());
        let store = crate::api::git::store::GitStore::new(
            &endpoint,
            &access_key,
            &secret_key,
            &bucket,
            "us-east-1",
            buzz_media::config::S3AddressingStyle::Path,
        )
        .expect("test object store");
        let bootstrap_id = Uuid::new_v4();
        super::super::restore::RestoreProtectionRuntime::provision_domain(
            &db,
            &store,
            community_id,
            bootstrap_id,
        )
        .await
        .expect("provision restore witness");
        let restore = super::super::restore::RestoreProtectionRuntime::initialize(
            db.clone(),
            store.clone(),
            [(community_id, bootstrap_id)],
        )
        .await
        .expect("initialize restore witness");
        let tenant = TenantContext::resolved(community_id, "restore-invalidation.example");
        let event_id = Uuid::new_v4();
        let event = AuthorizationAdmissionLoss::new(
            event_id,
            vec![
                AuthorizationSelector::principal("issuer.example", "subject-a").expect("principal"),
                AuthorizationSelector::nostr_key([41_u8; 32]),
            ],
        )
        .expect("valid admission loss");
        let fingerprint =
            buzz_db::authorization_invalidation::authorization_invalidation_request_fingerprint(
                community_id,
                &event.request,
            );

        // Replica A writes Pending and commits PostgreSQL. Replica B then
        // recovers the pending witness and replays the exact database effect
        // before A attempts its now-stale ETag CAS.
        let replica_a = restore
            .begin(community_id, event_id, fingerprint)
            .await
            .expect("replica A begins invalidation");
        let first = db
            .apply_authorization_invalidation(community_id, &event.request)
            .await
            .expect("database effect commits")
            .receipt();
        let restore_b = super::super::restore::RestoreProtectionRuntime::initialize(
            db.clone(),
            store.clone(),
            [(community_id, bootstrap_id)],
        )
        .await
        .expect("replica B recovers replica A pending witness");
        let replica_b = restore_b
            .begin(community_id, event_id, fingerprint)
            .await
            .expect("replica B recognizes exact durable replay");
        let replay = db
            .apply_authorization_invalidation(community_id, &event.request)
            .await
            .expect("replica B replay converges");
        assert!(!replay.committed_now());
        replica_b
            .commit_invalidation(first.generation)
            .await
            .expect("replica B observes committed witness");
        replica_a
            .commit_invalidation(first.generation)
            .await
            .expect("replica A stale CAS converges idempotently");

        let config =
            AuthorizationInvalidationConfig::new(Duration::from_millis(5), Duration::from_secs(10))
                .expect("valid config");
        let runtime = AuthorizationInvalidationRuntime::with_store_and_restore(
            Arc::new(DbInvalidationStore(db.clone())),
            None,
            config,
            Some(restore_b),
        );
        runtime
            .initialize_domains([community_id])
            .await
            .expect("initialize invalidation domain");
        let (duplicate_a, duplicate_b) = tokio::join!(
            runtime.apply_admission_loss(&tenant, &event),
            runtime.apply_admission_loss(&tenant, &event),
        );
        assert_eq!(duplicate_a.expect("first replay"), first);
        assert_eq!(duplicate_b.expect("concurrent replay"), first);
        assert_eq!(first.generation, 1);

        let restore_after_replay = super::super::restore::RestoreProtectionRuntime::initialize(
            db.clone(),
            store.clone(),
            [(community_id, bootstrap_id)],
        )
        .await
        .expect("replay leaves a committed checkpoint");
        let restarted = AuthorizationInvalidationRuntime::with_store_and_restore(
            Arc::new(DbInvalidationStore(db.clone())),
            None,
            AuthorizationInvalidationConfig::default(),
            Some(restore_after_replay),
        );
        restarted
            .initialize_domains([community_id])
            .await
            .expect("restart after replay");

        let conflicting = AuthorizationAdmissionLoss::new(
            event_id,
            vec![
                AuthorizationSelector::principal("issuer.example", "subject-b").expect("principal"),
            ],
        )
        .expect("valid conflicting request");
        assert_eq!(
            restarted.apply_admission_loss(&tenant, &conflicting).await,
            Err(AuthorizationInvalidationRuntimeError::AuthorityUnavailable)
        );
        let restore_after_conflict = super::super::restore::RestoreProtectionRuntime::initialize(
            db.clone(),
            store.clone(),
            [(community_id, bootstrap_id)],
        )
        .await
        .expect("fingerprint conflict cannot poison the checkpoint");
        let after_conflict = AuthorizationInvalidationRuntime::with_store_and_restore(
            Arc::new(DbInvalidationStore(db.clone())),
            None,
            AuthorizationInvalidationConfig::default(),
            Some(Arc::clone(&restore_after_conflict)),
        );
        after_conflict
            .initialize_domains([community_id])
            .await
            .expect("restart after conflict");
        let next = AuthorizationAdmissionLoss::new(
            Uuid::new_v4(),
            vec![AuthorizationSelector::nostr_key([42_u8; 32])],
        )
        .expect("valid next event");
        let next_receipt = after_conflict
            .apply_admission_loss(&tenant, &next)
            .await
            .expect("later valid event commits");
        assert_eq!(next_receipt.generation, 2);

        // The generic witness commit used by audio reconciliation must have
        // the same stale-CAS convergence property. Force a third durable
        // operation through two independent restore runtimes, but commit the
        // witness with `commit` rather than the invalidation specialization.
        let generic = AuthorizationAdmissionLoss::new(
            Uuid::new_v4(),
            vec![AuthorizationSelector::nostr_key([43_u8; 32])],
        )
        .expect("valid generic witness event");
        let generic_fingerprint =
            buzz_db::authorization_invalidation::authorization_invalidation_request_fingerprint(
                community_id,
                &generic.request,
            );
        let generic_a = restore_after_conflict
            .begin(
                community_id,
                generic.request.event_id(),
                generic_fingerprint,
            )
            .await
            .expect("generic replica A begins");
        db.apply_authorization_invalidation(community_id, &generic.request)
            .await
            .expect("generic database effect commits");
        let generic_restore_b = super::super::restore::RestoreProtectionRuntime::initialize(
            db.clone(),
            store.clone(),
            [(community_id, bootstrap_id)],
        )
        .await
        .expect("generic replica B recovers pending witness");
        let generic_b = generic_restore_b
            .begin(
                community_id,
                generic.request.event_id(),
                generic_fingerprint,
            )
            .await
            .expect("generic replica B recognizes durable replay");
        generic_b.commit().await.expect("replica B commits witness");
        generic_a
            .commit()
            .await
            .expect("replica A stale generic CAS converges");
    }

    #[tokio::test]
    async fn admission_loss_uses_only_the_server_resolved_tenant_domain() {
        let store = Arc::new(FakeStore::default());
        let selected_domain = domain(8);
        let other_domain = domain(9);
        for community_id in [selected_domain, other_domain] {
            store.set(AuthorizationInvalidationSnapshot {
                community_id,
                generation: 0,
                floors: Vec::new(),
            });
        }
        let runtime = runtime(store.clone());
        runtime
            .initialize_domains([selected_domain, other_domain])
            .await
            .expect("both domains start");
        let selector = AuthorizationSelector::nostr_key([9_u8; 32]);
        let other_cancel = CancellationToken::new();
        let other_observer = observe(
            &runtime,
            other_domain,
            vec![selector.clone()],
            other_cancel.clone(),
        )
        .await;
        let event = AuthorizationAdmissionLoss::new(Uuid::new_v4(), vec![selector])
            .expect("valid admission loss");
        let tenant = TenantContext::resolved(selected_domain, "selected.example");
        let receipt = runtime
            .apply_admission_loss(&tenant, &event)
            .await
            .expect("selected domain commits");

        assert_eq!(receipt.community_id, selected_domain);
        assert_eq!(store.snapshot(selected_domain).await.unwrap().generation, 1);
        assert_eq!(store.snapshot(other_domain).await.unwrap().generation, 0);
        assert!(!other_cancel.is_cancelled());
        assert!(other_observer.recheck().is_ok());
    }

    #[tokio::test]
    async fn restart_bootstraps_full_state_before_readiness() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(2);
        let selector = AuthorizationSelector::policy_version("old").expect("valid policy");
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 7,
            floors: vec![floor(&selector, 7, true)],
        });
        let restarted = runtime(store);
        assert!(!restarted.is_ready(community_id));
        let fence = restarted
            .capture_before_evaluation(community_id)
            .await
            .expect("startup snapshot loads");
        assert_eq!(fence.generation(), 7);
        assert!(matches!(
            restarted.observe_authority(fence, dependencies(community_id, selector), None,),
            Err(AuthorizationInvalidationRuntimeError::Invalidated)
        ));
    }

    #[tokio::test]
    async fn read_failure_denies_immediately_and_partition_heal_recovers() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(3);
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 0,
            floors: Vec::new(),
        });
        let node = runtime(store.clone());
        node.initialize_domains([community_id])
            .await
            .expect("starts ready");
        store.fail.store(true, Ordering::SeqCst);
        assert_eq!(
            node.poll_once().await,
            Err(AuthorizationInvalidationRuntimeError::AuthorityUnavailable)
        );
        assert!(!node.is_ready(community_id));
        store.fail.store(false, Ordering::SeqCst);
        node.poll_once().await.expect("partition heals");
        assert!(node.is_ready(community_id));
    }

    #[tokio::test]
    async fn durable_regression_never_restores_authority() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(4);
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 4,
            floors: Vec::new(),
        });
        let node = runtime(store.clone());
        node.initialize_domains([community_id])
            .await
            .expect("starts ready");
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 3,
            floors: Vec::new(),
        });
        assert_eq!(
            node.reconcile_domain(community_id).await,
            Err(AuthorizationInvalidationRuntimeError::AuthorityRegressed)
        );
        assert!(!node.is_ready(community_id));
    }

    #[test]
    fn debug_output_redacts_selector_material() {
        let community_id = domain(5);
        let private_policy = "private-policy-version";
        let dependencies = dependencies(
            community_id,
            AuthorizationSelector::policy_version(private_policy).expect("valid policy"),
        );
        assert!(!format!("{dependencies:?}").contains(private_policy));
    }

    #[test]
    fn admission_loss_rejects_unsafe_selectors_and_redacts_private_material() {
        let event_id = Uuid::new_v4();
        let private_issuer = "private-issuer.example";
        let event = AuthorizationAdmissionLoss::new(
            event_id,
            vec![
                AuthorizationSelector::principal(private_issuer, "private-subject")
                    .expect("valid principal"),
                AuthorizationSelector::nostr_key([3_u8; 32]),
                AuthorizationSelector::delegated_owner([4_u8; 32]),
            ],
        )
        .expect("safe admission-loss selectors");
        let debug = format!("{event:?}");
        assert!(!debug.contains(private_issuer));
        assert!(!debug.contains(&event_id.to_string()));

        for selector in [
            AuthorizationSelector::binding(Uuid::new_v4(), 1).expect("valid binding"),
            AuthorizationSelector::session(Uuid::new_v4()).expect("valid session"),
            AuthorizationSelector::domain(),
            AuthorizationSelector::policy_version("private-policy").expect("valid policy"),
        ] {
            assert_eq!(
                AuthorizationAdmissionLoss::new(Uuid::new_v4(), vec![selector]),
                Err(AuthorizationInvalidationRuntimeError::InvalidAdmissionLoss)
            );
        }
        assert_eq!(
            AuthorizationAdmissionLoss::new(Uuid::new_v4(), Vec::new()),
            Err(AuthorizationInvalidationRuntimeError::InvalidAdmissionLoss)
        );
        assert_eq!(
            AuthorizationAdmissionLoss::new(
                Uuid::nil(),
                vec![AuthorizationSelector::nostr_key([5_u8; 32])],
            ),
            Err(AuthorizationInvalidationRuntimeError::InvalidAdmissionLoss)
        );
    }

    #[test]
    fn delegated_owner_matches_generic_key_and_owner_selectors() {
        let owner = [7_u8; 32];
        let selectors = delegated_owner_selectors(owner);
        assert_eq!(selectors[0].kind(), AuthorizationSelectorKind::NostrKey);
        assert_eq!(
            selectors[1].kind(),
            AuthorizationSelectorKind::DelegatedOwner
        );
        assert_eq!(
            selectors[0].fingerprint(),
            AuthorizationSelector::nostr_key(owner).fingerprint()
        );
        assert_eq!(
            selectors[1].fingerprint(),
            AuthorizationSelector::delegated_owner(owner).fingerprint()
        );
    }

    #[tokio::test]
    async fn runtime_polls_durable_authority_without_redis() {
        let store = Arc::new(FakeStore::default());
        let community_id = domain(6);
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 0,
            floors: Vec::new(),
        });
        let node = runtime(store.clone());
        node.initialize_domains([community_id])
            .await
            .expect("starts ready");
        let running = node.clone();
        let task = tokio::spawn(async move { running.run().await });
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 1,
            floors: Vec::new(),
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while node.inner.generation(community_id) != Some(1) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("polling converges without Redis");
        node.fail_closed();
        assert!(!node.is_ready(community_id));
        store.set(AuthorizationInvalidationSnapshot {
            community_id,
            generation: 2,
            floors: Vec::new(),
        });
        assert_eq!(
            node.reconcile_domain(community_id).await,
            Err(AuthorizationInvalidationRuntimeError::NotReady)
        );
        assert!(!node.is_ready(community_id));
        node.shutdown();
        task.await.expect("runtime task exits");
    }

    #[test]
    fn public_constructor_has_no_caller_supplied_principal_slot() {
        fn assert_context_only_signature(
            _constructor: fn(
                Uuid,
                &AuthContext,
            ) -> Result<
                AuthorizationDependencies,
                AuthorizationInvalidationRuntimeError,
            >,
        ) {
        }

        assert_context_only_signature(AuthorizationDependencies::from_context);
    }
}
