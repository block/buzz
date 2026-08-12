//! Ordered immutable startup and aggregate readiness.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use buzz_auth::{LocalBindingResolverCapability, RouteCapability, VerifierPolicyStamp};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    DelegationConfig, DynamicVerifier, InvalidationRegistry, ProviderFreeRuntimeConfig,
    ProviderFreeRuntimeMode, RouteAuthority, RuntimeAuthority, RuntimeAuthorizationError,
};

const SCHEMA_BIT: u16 = 1 << 0;
const DATABASE_BIT: u16 = 1 << 1;
const VERIFIER_BIT: u16 = 1 << 2;
const DISCOVERY_BIT: u16 = 1 << 3;
const RECONCILIATION_BIT: u16 = 1 << 4;
const STATE_BIT: u16 = 1 << 5;
const ROUTES_BIT: u16 = 1 << 6;
const STATUS_BIT: u16 = 1 << 7;
const ENFORCE_READY_BITS: u16 = SCHEMA_BIT
    | DATABASE_BIT
    | VERIFIER_BIT
    | DISCOVERY_BIT
    | RECONCILIATION_BIT
    | STATE_BIT
    | ROUTES_BIT
    | STATUS_BIT;
const MAX_STARTUP_OBSERVATION_AGE_SECONDS: i64 = 300;
const MAX_STARTUP_CLOCK_SKEW_SECONDS: i64 = 30;

/// Readiness dependency whose loss immediately denies protected operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessReason {
    /// Exact schema and migration lineage.
    Schema,
    /// Independent writer, listener, and witness roles.
    DatabaseRoles,
    /// Stable policy and current key generation.
    Verifier,
    /// Issuer-matched discovery state.
    Discovery,
    /// Restore and invalidation reconciliation.
    Reconciliation,
    /// One complete immutable runtime state.
    State,
    /// Complete closed route inventory.
    Routes,
    /// Connection-local current-status production contract.
    Status,
}

impl ReadinessReason {
    const fn bit(self) -> u16 {
        match self {
            Self::Schema => SCHEMA_BIT,
            Self::DatabaseRoles => DATABASE_BIT,
            Self::Verifier => VERIFIER_BIT,
            Self::Discovery => DISCOVERY_BIT,
            Self::Reconciliation => RECONCILIATION_BIT,
            Self::State => STATE_BIT,
            Self::Routes => ROUTES_BIT,
            Self::Status => STATUS_BIT,
        }
    }
}

/// Shared aggregate readiness. Health workers may only withdraw bits; recovery
/// is installed through a new proof-bearing immutable runtime generation.
pub struct AggregateReadiness {
    required: u16,
    healthy: AtomicU16,
}

impl AggregateReadiness {
    fn complete_enforce() -> Self {
        Self {
            required: ENFORCE_READY_BITS,
            healthy: AtomicU16::new(ENFORCE_READY_BITS),
        }
    }

    fn disabled() -> Self {
        Self {
            required: 0,
            healthy: AtomicU16::new(0),
        }
    }

    fn unavailable_enforce() -> Self {
        Self {
            required: ENFORCE_READY_BITS,
            healthy: AtomicU16::new(0),
        }
    }

    /// Whether every required startup and live dependency remains healthy.
    pub fn is_ready(&self) -> bool {
        self.healthy.load(Ordering::Acquire) & self.required == self.required
    }

    /// Permanently withdraw one dependency from this installed generation.
    /// Recovery constructs and atomically installs a freshly reconciled state;
    /// it never toggles a stale generation optimistic again.
    pub fn withdraw(&self, reason: ReadinessReason) {
        self.healthy.fetch_and(!reason.bit(), Ordering::AcqRel);
    }

    /// Current privacy-safe health mask for diagnostics.
    pub fn health_mask(&self) -> u16 {
        self.healthy.load(Ordering::Acquire)
    }
}

impl fmt::Debug for AggregateReadiness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AggregateReadiness")
            .field("ready", &self.is_ready())
            .finish_non_exhaustive()
    }
}

/// Exact schema and signed-seal witness produced after migrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaWitness {
    seal_tree: [u8; 20],
    schema_fingerprint: [u8; 32],
    migration_ordinal: u32,
}

impl SchemaWitness {
    /// Construct only after schema attachment and drift checks pass.
    pub fn new(
        seal_tree: [u8; 20],
        schema_fingerprint: [u8; 32],
        migration_ordinal: u32,
        fully_attached: bool,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if seal_tree == [0; 20]
            || schema_fingerprint == [0; 32]
            || migration_ordinal == 0
            || !fully_attached
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(Self {
            seal_tree,
            schema_fingerprint,
            migration_ordinal,
        })
    }

    /// Exact signed-seal tree identity.
    pub const fn seal_tree(&self) -> &[u8; 20] {
        &self.seal_tree
    }

    /// Fingerprint of the exact fully attached schema.
    pub const fn schema_fingerprint(&self) -> &[u8; 32] {
        &self.schema_fingerprint
    }

    /// Highest verified migration ordinal.
    pub const fn migration_ordinal(&self) -> u32 {
        self.migration_ordinal
    }
}

/// Non-substitutable writer/listener/witness role probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseRoleWitness {
    writer: Uuid,
    listener: Uuid,
    witness: Uuid,
    probed_at: DateTime<Utc>,
}

impl DatabaseRoleWitness {
    /// Validate three independently owned healthy role identities.
    pub fn new(
        writer: Uuid,
        listener: Uuid,
        witness: Uuid,
        probed_at: DateTime<Utc>,
        healthy: bool,
    ) -> Result<Self, RuntimeAuthorizationError> {
        let now = Utc::now();
        if writer.is_nil()
            || listener.is_nil()
            || witness.is_nil()
            || writer == listener
            || writer == witness
            || listener == witness
            || !healthy
            || !observation_is_recent(probed_at, now)
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(Self {
            writer,
            listener,
            witness,
            probed_at,
        })
    }

    /// Independent writer, listener, and witness role identities.
    pub const fn role_identities(&self) -> (Uuid, Uuid, Uuid) {
        (self.writer, self.listener, self.witness)
    }

    /// Authoritative role-probe observation time.
    pub const fn probed_at(&self) -> DateTime<Utc> {
        self.probed_at
    }
}

/// Canonical verifier/discovery observation with separate hard deadlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifierReadinessWitness {
    stamp: VerifierPolicyStamp,
    observed_at: DateTime<Utc>,
    verifier_fresh_until: DateTime<Utc>,
    discovery_fresh_until: DateTime<Utc>,
}

impl VerifierReadinessWitness {
    /// Validate non-stale verifier and issuer-matched discovery observations.
    pub fn new(
        stamp: VerifierPolicyStamp,
        observed_at: DateTime<Utc>,
        verifier_fresh_until: DateTime<Utc>,
        discovery_fresh_until: DateTime<Utc>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if !observation_is_recent(observed_at, Utc::now())
            || observed_at >= verifier_fresh_until
            || observed_at >= discovery_fresh_until
        {
            return Err(RuntimeAuthorizationError::StaleVerifier);
        }
        Ok(Self {
            stamp,
            observed_at,
            verifier_fresh_until,
            discovery_fresh_until,
        })
    }

    /// Stable policy and current rotating key generation.
    pub const fn stamp(&self) -> VerifierPolicyStamp {
        self.stamp
    }

    /// Observation time and independent hard deadlines.
    pub const fn deadlines(&self) -> (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) {
        (
            self.observed_at,
            self.verifier_fresh_until,
            self.discovery_fresh_until,
        )
    }
}

/// Complete restore/invalidation reconciliation witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreReconciliationWitness {
    digest: [u8; 32],
    observed_at: DateTime<Utc>,
}

impl RestoreReconciliationWitness {
    /// Validate a non-sentinel result only after every refusal check passes.
    pub fn new(
        digest: [u8; 32],
        observed_at: DateTime<Utc>,
        complete: bool,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if digest == [0; 32] || !complete || !observation_is_recent(observed_at, Utc::now()) {
            return Err(RuntimeAuthorizationError::RestoreRejected);
        }
        Ok(Self {
            digest,
            observed_at,
        })
    }

    /// Privacy-safe reconciliation digest and observation time.
    pub const fn observation(&self) -> (&[u8; 32], DateTime<Utc>) {
        (&self.digest, self.observed_at)
    }
}

/// Complete route inventory plus connection-local contract witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteInventoryWitness {
    route_fingerprint: [u8; 32],
    status_contract_fingerprint: [u8; 32],
}

/// Positive proof that the installed resolver implements the exact configured
/// owner-bound delegation surface. Absence is the only valid disabled state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationReadinessWitness {
    capabilities: BTreeSet<RouteCapability>,
    maximum_lifetime: Duration,
    resolver: LocalBindingResolverCapability,
    contract_fingerprint: [u8; 32],
}

impl DelegationReadinessWitness {
    /// Bind one nonempty finite capability set to the positive resolver
    /// capability and a stable implementation-contract fingerprint.
    pub fn new(
        capabilities: BTreeSet<RouteCapability>,
        maximum_lifetime: Duration,
        resolver: LocalBindingResolverCapability,
        contract_fingerprint: [u8; 32],
    ) -> Result<Self, RuntimeAuthorizationError> {
        if capabilities.is_empty()
            || maximum_lifetime.is_zero()
            || resolver != LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
            || contract_fingerprint == [0; 32]
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(Self {
            capabilities,
            maximum_lifetime,
            resolver,
            contract_fingerprint,
        })
    }
}

impl RouteInventoryWitness {
    /// Bind exact complete routes to the immutable typed current-binding contract.
    pub fn new(
        route_fingerprint: [u8; 32],
        status_contract_fingerprint: [u8; 32],
    ) -> Result<Self, RuntimeAuthorizationError> {
        if route_fingerprint == [0; 32] || status_contract_fingerprint == [0; 32] {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(Self {
            route_fingerprint,
            status_contract_fingerprint,
        })
    }
}

/// Concrete immutable objects installed together after reconciliation.
pub struct RuntimeStateComponents {
    /// Complete typed route authority.
    pub routes: Arc<RouteAuthority>,
    /// Single dynamic canonical verifier.
    pub verifier: Arc<DynamicVerifier>,
    /// Reconciled live invalidation registry.
    pub invalidation: Arc<InvalidationRegistry>,
    /// Canonical authorization authority.
    pub authority: Arc<RuntimeAuthority>,
    /// Exact reconciliation lineage installed with the invalidation registry.
    pub reconciliation_digest: [u8; 32],
    /// Exact current-binding typed-contract identity installed with route adapters.
    pub status_contract_fingerprint: [u8; 32],
    /// Positive owner-bound resolver proof. Configuration alone never creates
    /// delegation authority; disabled configuration requires this to be absent.
    pub delegation: Option<DelegationReadinessWitness>,
}

impl fmt::Debug for RuntimeStateComponents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeStateComponents([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupPhase {
    Schema,
    Database,
    Verifier,
    Reconciliation,
    State,
    Aggregate,
}

/// Ordered Enforce-mode builder. Calling phases out of order fails closed.
pub struct RuntimeStartup {
    config: ProviderFreeRuntimeConfig,
    phase: StartupPhase,
    schema: Option<SchemaWitness>,
    database: Option<DatabaseRoleWitness>,
    verifier_witness: Option<VerifierReadinessWitness>,
    reconciliation: Option<RestoreReconciliationWitness>,
    route_witness: Option<RouteInventoryWitness>,
    components: Option<RuntimeStateComponents>,
}

impl RuntimeStartup {
    /// Begin Enforce startup. Off and emergency denial use
    /// [`InstalledAuthorizationRuntime::disabled`].
    pub fn enforce(config: ProviderFreeRuntimeConfig) -> Result<Self, RuntimeAuthorizationError> {
        if config.mode() != ProviderFreeRuntimeMode::Enforce || config.enforce().is_none() {
            return Err(RuntimeAuthorizationError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            phase: StartupPhase::Schema,
            schema: None,
            database: None,
            verifier_witness: None,
            reconciliation: None,
            route_witness: None,
            components: None,
        })
    }

    /// Phase 1: accept exact migration/schema witness.
    pub fn migrations(&mut self, witness: SchemaWitness) -> Result<(), RuntimeAuthorizationError> {
        self.require_phase(StartupPhase::Schema)?;
        self.schema = Some(witness);
        self.phase = StartupPhase::Database;
        Ok(())
    }

    /// Phase 2: accept non-substitutable DB-role probes.
    pub fn database_roles(
        &mut self,
        witness: DatabaseRoleWitness,
    ) -> Result<(), RuntimeAuthorizationError> {
        self.require_phase(StartupPhase::Database)?;
        self.database = Some(witness);
        self.phase = StartupPhase::Verifier;
        Ok(())
    }

    /// Phase 3: accept canonical key generation and discovery deadlines.
    pub fn verifier(
        &mut self,
        witness: VerifierReadinessWitness,
    ) -> Result<(), RuntimeAuthorizationError> {
        self.require_phase(StartupPhase::Verifier)?;
        let expected_policy = self
            .config
            .enforce()
            .map(|config| config.verifier_policy().id())
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        if witness.stamp.policy_id() != expected_policy {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        self.verifier_witness = Some(witness);
        self.phase = StartupPhase::Reconciliation;
        Ok(())
    }

    /// Phase 4: accept complete restore/invalidation reconciliation.
    pub fn reconciliation(
        &mut self,
        witness: RestoreReconciliationWitness,
    ) -> Result<(), RuntimeAuthorizationError> {
        self.require_phase(StartupPhase::Reconciliation)?;
        self.reconciliation = Some(witness);
        self.phase = StartupPhase::State;
        Ok(())
    }

    /// Phase 5: install one immutable, internally matching runtime state.
    pub async fn install_state(
        &mut self,
        components: RuntimeStateComponents,
        routes: RouteInventoryWitness,
    ) -> Result<(), RuntimeAuthorizationError> {
        self.require_phase(StartupPhase::State)?;
        let verifier_witness = self
            .verifier_witness
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let reconciliation = self
            .reconciliation
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let database = self
            .database
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let delegation_matches = match (
            self.config.enforce().map(|config| config.delegation()),
            components.delegation.as_ref(),
        ) {
            (Some(DelegationConfig::Disabled), None) => true,
            (
                Some(DelegationConfig::Enabled {
                    capabilities,
                    maximum_lifetime,
                }),
                Some(witness),
            ) => {
                witness.capabilities == *capabilities
                    && witness.maximum_lifetime == *maximum_lifetime
                    && witness.resolver
                        == LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
                    && witness.contract_fingerprint != [0; 32]
            }
            _ => false,
        };
        let install_now = Utc::now();
        if !observation_is_recent(database.probed_at, install_now)
            || !observation_is_recent(reconciliation.observed_at, install_now)
            || !observation_is_recent(verifier_witness.observed_at, install_now)
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        let verifier_snapshot = components.verifier.current(install_now).await?;
        let discovery_matches = match self.config.enforce().map(|config| config.jwks_source()) {
            Some(super::JwksSourceConfig::DiscoveryUri(_)) => {
                verifier_snapshot.discovery_fresh_until()
                    == Some(verifier_witness.discovery_fresh_until)
            }
            Some(super::JwksSourceConfig::JwksUri(_)) => {
                verifier_snapshot.discovery_fresh_until().is_none()
                    && verifier_witness.discovery_fresh_until
                        == verifier_witness.verifier_fresh_until
            }
            None => false,
        };
        if components.routes.inventory_fingerprint() != &routes.route_fingerprint
            || !components.invalidation.is_ready()
            || components.authority.routes().inventory_fingerprint()
                != components.routes.inventory_fingerprint()
            || !Arc::ptr_eq(
                components.authority.invalidation(),
                &components.invalidation,
            )
            || verifier_snapshot.stamp() != verifier_witness.stamp
            || verifier_snapshot.fresh_until() != verifier_witness.verifier_fresh_until
            || install_now >= verifier_witness.verifier_fresh_until
            || install_now >= verifier_witness.discovery_fresh_until
            || !discovery_matches
            || components.reconciliation_digest != reconciliation.digest
            || components.status_contract_fingerprint != routes.status_contract_fingerprint
            || !delegation_matches
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        self.components = Some(components);
        self.route_witness = Some(routes);
        self.phase = StartupPhase::Aggregate;
        Ok(())
    }

    /// Phase 6: construct aggregate readiness. Routers, listeners, and
    /// background tasks may be exposed only after this succeeds.
    pub fn finish(self) -> Result<InstalledAuthorizationRuntime, RuntimeAuthorizationError> {
        if self.config.mode() != ProviderFreeRuntimeMode::Enforce
            || self.phase != StartupPhase::Aggregate
            || self.schema.is_none()
            || self.database.is_none()
            || self.verifier_witness.is_none()
            || self.reconciliation.is_none()
            || self.route_witness.is_none()
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        let verifier_witness = self
            .verifier_witness
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let database = self
            .database
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let reconciliation = self
            .reconciliation
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        let finish_now = Utc::now();
        if !observation_is_recent(database.probed_at, finish_now)
            || !observation_is_recent(reconciliation.observed_at, finish_now)
            || !observation_is_recent(verifier_witness.observed_at, finish_now)
            || finish_now >= verifier_witness.verifier_fresh_until
            || finish_now >= verifier_witness.discovery_fresh_until
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        let components = self
            .components
            .ok_or(RuntimeAuthorizationError::PartialState)?;
        if components.verifier.current_stamp() != Some(verifier_witness.stamp)
            || !components.invalidation.is_ready()
        {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(InstalledAuthorizationRuntime {
            mode: ProviderFreeRuntimeMode::Enforce,
            readiness: Arc::new(AggregateReadiness::complete_enforce()),
            routes: Some(components.routes),
            verifier: Some(components.verifier),
            trusted_proxy: None,
            invalidation: Some(components.invalidation),
            authority: Some(components.authority),
            status_contract_fingerprint: self
                .route_witness
                .map(|witness| witness.status_contract_fingerprint),
            verifier_fresh_until: Some(verifier_witness.verifier_fresh_until),
            discovery_fresh_until: Some(verifier_witness.discovery_fresh_until),
            verifier_stamp: Some(verifier_witness.stamp),
            follows_dynamic_verifier: false,
        })
    }

    fn require_phase(&self, expected: StartupPhase) -> Result<(), RuntimeAuthorizationError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(RuntimeAuthorizationError::PartialState)
        }
    }
}

impl fmt::Debug for RuntimeStartup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeStartup")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

/// Complete provider-free state stored once on `AppState`.
#[derive(Clone)]
pub struct InstalledAuthorizationRuntime {
    mode: ProviderFreeRuntimeMode,
    readiness: Arc<AggregateReadiness>,
    routes: Option<Arc<RouteAuthority>>,
    verifier: Option<Arc<DynamicVerifier>>,
    trusted_proxy: Option<Arc<buzz_auth::TrustedProxyProvenanceVerifier>>,
    invalidation: Option<Arc<InvalidationRegistry>>,
    authority: Option<Arc<RuntimeAuthority>>,
    status_contract_fingerprint: Option<[u8; 32]>,
    verifier_fresh_until: Option<DateTime<Utc>>,
    discovery_fresh_until: Option<DateTime<Utc>>,
    verifier_stamp: Option<VerifierPolicyStamp>,
    follows_dynamic_verifier: bool,
}

impl InstalledAuthorizationRuntime {
    /// Construct an assertion-only ready runtime for route-adapter tests.
    ///
    /// Production must use [`RuntimeStartup::finish`]; this helper deliberately
    /// installs no route, database, invalidation, or status authority.
    #[cfg(test)]
    pub(crate) fn for_canonical_assertion_test(
        verifier: Arc<DynamicVerifier>,
        snapshot: super::JwksSnapshot,
    ) -> Self {
        let routes = Arc::new(RouteAuthority::closed().expect("closed route inventory"));
        Self {
            mode: ProviderFreeRuntimeMode::Enforce,
            readiness: Arc::new(AggregateReadiness::complete_enforce()),
            routes: Some(routes),
            verifier: Some(verifier),
            trusted_proxy: None,
            invalidation: None,
            authority: None,
            status_contract_fingerprint: None,
            verifier_fresh_until: Some(snapshot.fresh_until()),
            discovery_fresh_until: Some(snapshot.fresh_until()),
            verifier_stamp: Some(snapshot.stamp()),
            follows_dynamic_verifier: false,
        }
    }

    /// Install the production handler-owned canonical authority.
    ///
    /// Startup fetches one current key generation and constructs the exact v2
    /// trusted-proxy verifier before listeners can open. Refresh is continuous;
    /// a failed or stale refresh is denied by `DynamicVerifier::verify` at the
    /// owning handler before application mutation.
    pub async fn production(
        config: &ProviderFreeRuntimeConfig,
    ) -> Result<Self, RuntimeAuthorizationError> {
        let enforce = config
            .enforce()
            .ok_or(RuntimeAuthorizationError::InvalidConfiguration)?;
        let refresh_policy = super::JwksRefreshPolicy::new(
            1024 * 1024,
            Duration::from_secs(10),
            enforce.lease_maximum().max(Duration::from_secs(300)),
        )?;
        let verifier = Arc::new(DynamicVerifier::new(
            enforce.verifier_policy().clone(),
            enforce.issuer().to_owned(),
            enforce.jwks_source().clone(),
            refresh_policy,
            Arc::new(super::ReqwestJwksDocumentLoader::new()?),
        )?);
        let snapshot = verifier.refresh(Utc::now()).await?;
        let trusted_proxy = Arc::new(enforce.trusted_proxy_verifier()?);
        let routes = Arc::new(RouteAuthority::closed()?);
        let refresh_verifier = Arc::clone(&verifier);
        tokio::spawn(async move {
            let interval = refresh_policy.fresh_lifetime / 2;
            loop {
                tokio::time::sleep(interval).await;
                if let Err(error) = refresh_verifier.refresh(Utc::now()).await {
                    tracing::warn!(
                        code = error.code(),
                        "canonical verifier refresh failed closed"
                    );
                }
            }
        });
        Ok(Self {
            mode: ProviderFreeRuntimeMode::Enforce,
            readiness: Arc::new(AggregateReadiness::complete_enforce()),
            routes: Some(routes),
            verifier: Some(verifier),
            trusted_proxy: Some(trusted_proxy),
            invalidation: None,
            authority: None,
            status_contract_fingerprint: None,
            verifier_fresh_until: Some(snapshot.fresh_until()),
            discovery_fresh_until: Some(
                snapshot
                    .discovery_fresh_until()
                    .unwrap_or(snapshot.fresh_until()),
            ),
            verifier_stamp: Some(snapshot.stamp()),
            follows_dynamic_verifier: true,
        })
    }

    /// Install complete Off or emergency-denial state without initializing
    /// verifier, database, restore, or status dependencies.
    pub fn disabled(config: &ProviderFreeRuntimeConfig) -> Result<Self, RuntimeAuthorizationError> {
        if config.mode() == ProviderFreeRuntimeMode::Enforce {
            return Err(RuntimeAuthorizationError::PartialState);
        }
        Ok(Self {
            mode: config.mode(),
            readiness: Arc::new(AggregateReadiness::disabled()),
            routes: None,
            verifier: None,
            trusted_proxy: None,
            invalidation: None,
            authority: None,
            status_contract_fingerprint: None,
            verifier_fresh_until: None,
            discovery_fresh_until: None,
            verifier_stamp: None,
            follows_dynamic_verifier: false,
        })
    }

    /// Construct the only safe pre-start state for a configured Enforce
    /// runtime. It carries no dependency objects, denies every protected
    /// operation, and can never become ready. Production must replace it with
    /// the result of [`RuntimeStartup::finish`] before serving.
    pub fn fail_closed(config: &ProviderFreeRuntimeConfig) -> Self {
        if config.mode() != ProviderFreeRuntimeMode::Enforce {
            return Self {
                mode: config.mode(),
                readiness: Arc::new(AggregateReadiness::disabled()),
                routes: None,
                verifier: None,
                trusted_proxy: None,
                invalidation: None,
                authority: None,
                status_contract_fingerprint: None,
                verifier_fresh_until: None,
                discovery_fresh_until: None,
                verifier_stamp: None,
                follows_dynamic_verifier: false,
            };
        }
        Self {
            mode: ProviderFreeRuntimeMode::Enforce,
            readiness: Arc::new(AggregateReadiness::unavailable_enforce()),
            routes: None,
            verifier: None,
            trusted_proxy: None,
            invalidation: None,
            authority: None,
            status_contract_fingerprint: None,
            verifier_fresh_until: None,
            discovery_fresh_until: None,
            verifier_stamp: None,
            follows_dynamic_verifier: false,
        }
    }

    /// Closed configured mode.
    pub const fn mode(&self) -> ProviderFreeRuntimeMode {
        self.mode
    }

    /// Aggregate readiness including dynamic dependency loss.
    pub fn is_ready(&self) -> bool {
        let now = Utc::now();
        self.readiness.is_ready()
            && (self.mode != ProviderFreeRuntimeMode::Enforce || self.routes.is_some())
            && (self.mode != ProviderFreeRuntimeMode::Enforce
                || (self.follows_dynamic_verifier
                    && self
                        .verifier
                        .as_ref()
                        .is_some_and(|verifier| verifier.has_current_snapshot(now))
                    && self.trusted_proxy.is_some())
                || (self
                    .verifier_fresh_until
                    .is_some_and(|deadline| now < deadline)
                    && self
                        .discovery_fresh_until
                        .is_some_and(|deadline| now < deadline)
                    && self
                        .verifier
                        .as_ref()
                        .is_some_and(|verifier| verifier.current_stamp() == self.verifier_stamp)))
            && self
                .invalidation
                .as_ref()
                .is_none_or(|registry| registry.is_ready())
    }

    /// Whether protected work must be denied before any adapter executes.
    pub fn denies_protected(&self) -> bool {
        self.mode != ProviderFreeRuntimeMode::Off
            && (self.mode == ProviderFreeRuntimeMode::DenyProtected || !self.is_ready())
    }

    /// Withdraw one live dependency immediately.
    pub fn withdraw_readiness(&self, reason: ReadinessReason) {
        self.readiness.withdraw(reason);
    }

    /// Complete route table, available only in healthy Enforce mode.
    pub fn routes(&self) -> Result<&Arc<RouteAuthority>, RuntimeAuthorizationError> {
        if self.mode != ProviderFreeRuntimeMode::Enforce || !self.is_ready() {
            return Err(RuntimeAuthorizationError::NotReady);
        }
        self.routes
            .as_ref()
            .ok_or(RuntimeAuthorizationError::PartialState)
    }

    /// Canonical admission authority, available only in healthy Enforce mode.
    pub fn authority(&self) -> Result<&Arc<RuntimeAuthority>, RuntimeAuthorizationError> {
        if self.mode != ProviderFreeRuntimeMode::Enforce || !self.is_ready() {
            return Err(RuntimeAuthorizationError::NotReady);
        }
        self.authority
            .as_ref()
            .ok_or(RuntimeAuthorizationError::PartialState)
    }

    /// Dynamic verifier, available only in healthy Enforce mode.
    pub fn verifier(&self) -> Result<&Arc<DynamicVerifier>, RuntimeAuthorizationError> {
        if self.mode != ProviderFreeRuntimeMode::Enforce || !self.is_ready() {
            return Err(RuntimeAuthorizationError::NotReady);
        }
        self.verifier
            .as_ref()
            .ok_or(RuntimeAuthorizationError::PartialState)
    }

    /// Trusted-proxy authority, available only in healthy production Enforce mode.
    pub fn trusted_proxy_verifier(
        &self,
    ) -> Result<&Arc<buzz_auth::TrustedProxyProvenanceVerifier>, RuntimeAuthorizationError> {
        if self.mode != ProviderFreeRuntimeMode::Enforce || !self.is_ready() {
            return Err(RuntimeAuthorizationError::NotReady);
        }
        self.trusted_proxy
            .as_ref()
            .ok_or(RuntimeAuthorizationError::PartialState)
    }

    /// Live invalidation registry, available only in healthy Enforce mode.
    pub fn invalidation(&self) -> Result<&Arc<InvalidationRegistry>, RuntimeAuthorizationError> {
        if self.mode != ProviderFreeRuntimeMode::Enforce || !self.is_ready() {
            return Err(RuntimeAuthorizationError::NotReady);
        }
        self.invalidation
            .as_ref()
            .ok_or(RuntimeAuthorizationError::PartialState)
    }

    /// Immutable current-binding contract fingerprint without exposing wire internals.
    pub const fn status_contract_fingerprint(&self) -> Option<&[u8; 32]> {
        self.status_contract_fingerprint.as_ref()
    }
}

fn observation_is_recent(observed_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    observed_at >= now - chrono::Duration::seconds(MAX_STARTUP_OBSERVATION_AGE_SECONDS)
        && observed_at <= now + chrono::Duration::seconds(MAX_STARTUP_CLOCK_SKEW_SECONDS)
}

impl fmt::Debug for InstalledAuthorizationRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstalledAuthorizationRuntime")
            .field("mode", &self.mode)
            .field("ready", &self.is_ready())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use buzz_auth::{CanonicalVerifierPolicy, VerifierKeyGeneration, VerifierPolicyStamp};
    use buzz_core::CommunityId;

    use super::*;
    use crate::authorization_runtime::{
        AdmissionCommitPort, JwksDocumentLoader, JwksRefreshPolicy, ProtectedIngress,
        ReconciledInvalidationState, RouteRule, RuntimeAdmissionRequest,
    };

    fn deny_config() -> ProviderFreeRuntimeConfig {
        ProviderFreeRuntimeConfig::from_optional_json(Some(r#"{"deny_protected":true}"#)).unwrap()
    }

    #[test]
    fn disabled_modes_initialize_no_partial_dependencies() {
        let off =
            InstalledAuthorizationRuntime::disabled(&ProviderFreeRuntimeConfig::off()).unwrap();
        assert!(off.is_ready());
        assert!(!off.denies_protected());
        assert!(off.routes.is_none());

        let deny = InstalledAuthorizationRuntime::disabled(&deny_config()).unwrap();
        assert!(deny.is_ready());
        assert!(deny.denies_protected());
        assert!(deny.verifier.is_none());

        let enforce = InstalledAuthorizationRuntime::fail_closed(&enforce_config());
        assert!(!enforce.is_ready());
        assert!(enforce.denies_protected());
        assert!(enforce.authority.is_none());
    }

    #[test]
    fn witnesses_reject_partial_or_substitutable_state() {
        assert!(SchemaWitness::new([1; 20], [2; 32], 30, false).is_err());
        let same = Uuid::from_u128(1);
        assert!(
            DatabaseRoleWitness::new(same, same, Uuid::from_u128(2), Utc::now(), true).is_err()
        );
        assert!(DatabaseRoleWitness::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Utc::now() - chrono::Duration::seconds(301),
            true,
        )
        .is_err());
        assert!(DelegationReadinessWitness::new(
            BTreeSet::from([RouteCapability::MessagesRead]),
            Duration::from_secs(60),
            LocalBindingResolverCapability::Direct,
            [9; 32],
        )
        .is_err());
        assert!(RestoreReconciliationWitness::new(
            [4; 32],
            Utc::now() - chrono::Duration::seconds(301),
            true,
        )
        .is_err());
        assert!(RouteInventoryWitness::new([1; 32], [0; 32]).is_err());
    }

    #[test]
    fn readiness_loss_is_sticky_for_installed_generation() {
        let readiness = AggregateReadiness::complete_enforce();
        assert!(readiness.is_ready());
        readiness.withdraw(ReadinessReason::Verifier);
        assert!(!readiness.is_ready());
        assert_eq!(readiness.health_mask() & VERIFIER_BIT, 0);
    }

    fn enforce_config() -> ProviderFreeRuntimeConfig {
        ProviderFreeRuntimeConfig::from_optional_json(Some(
            r#"{
                "issuer":"https://issuer.example",
                "audience":"buzz",
                "maximum_token_lifetime_seconds":300,
                "jwks":{"jwks_uri":"https://issuer.example/keys"},
                "lease":{"maximum_seconds":120},
                "policy_revision":1,
                "audit":{"max_events_per_domain":100,"max_bytes_per_domain":65536,"max_envelope_bytes":4096},
                "client_status_admission":{"max_presentations_per_domain":100,"max_presentations_per_actor":5,"max_presentations_per_peer":20},
                "transport":{"kind":"sealed_nostr_proof"},
                "enrollment":{"kind":"canonical_admission"},
                "restore":{"kind":"operation_manifest"}
            }"#,
        ))
        .unwrap()
    }

    fn verifier_witness(now: DateTime<Utc>) -> VerifierReadinessWitness {
        let policy = CanonicalVerifierPolicy::new(
            "https://issuer.example".to_owned(),
            "buzz".to_owned(),
            "sub".to_owned(),
            None,
            0,
            300,
        )
        .unwrap();
        let generation = VerifierKeyGeneration::new(1).unwrap();
        VerifierReadinessWitness::new(
            VerifierPolicyStamp::new(policy.id(), generation),
            now,
            now + chrono::Duration::minutes(5),
            now + chrono::Duration::minutes(5),
        )
        .unwrap()
    }

    #[test]
    fn startup_phases_cannot_be_reordered_or_skipped() {
        let now = Utc::now();
        let mut startup = RuntimeStartup::enforce(enforce_config()).unwrap();
        let database = DatabaseRoleWitness::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            now,
            true,
        )
        .unwrap();
        assert_eq!(
            startup.database_roles(database),
            Err(RuntimeAuthorizationError::PartialState)
        );
        startup
            .migrations(SchemaWitness::new([1; 20], [2; 32], 30, true).unwrap())
            .unwrap();
        assert_eq!(
            startup.verifier(verifier_witness(now)),
            Err(RuntimeAuthorizationError::PartialState)
        );
        startup.database_roles(database).unwrap();
        startup.verifier(verifier_witness(now)).unwrap();
        assert!(startup.finish().is_err());
    }

    struct Loader(tokio::sync::Mutex<VecDeque<Vec<u8>>>);

    #[async_trait::async_trait]
    impl JwksDocumentLoader for Loader {
        async fn load(
            &self,
            _source: &super::super::JwksSourceConfig,
            _expected_issuer: &str,
            _policy: JwksRefreshPolicy,
        ) -> Result<Vec<u8>, RuntimeAuthorizationError> {
            self.0
                .lock()
                .await
                .pop_front()
                .ok_or(RuntimeAuthorizationError::DependencyUnavailable)
        }
    }

    struct DenyCommitter;

    #[async_trait::async_trait]
    impl AdmissionCommitPort for DenyCommitter {
        async fn commit(
            &self,
            _request: RuntimeAdmissionRequest,
        ) -> Result<buzz_auth::FinalizedAuthContext, RuntimeAuthorizationError> {
            Err(RuntimeAuthorizationError::DependencyUnavailable)
        }
    }

    #[tokio::test]
    async fn complete_startup_installs_once_in_mandatory_order() {
        let now = Utc::now();
        let config = enforce_config();
        let enforce = config.enforce().unwrap();
        let verifier = Arc::new(
            DynamicVerifier::new(
                enforce.verifier_policy().clone(),
                enforce.issuer().to_owned(),
                enforce.jwks_source().clone(),
                JwksRefreshPolicy::new(
                    64 * 1024,
                    std::time::Duration::from_secs(2),
                    std::time::Duration::from_secs(300),
                )
                .unwrap(),
                Arc::new(Loader(tokio::sync::Mutex::new(VecDeque::from([
                    br#"{"keys":[{"kty":"RSA","kid":"one","n":"AQAB","e":"AQAB"}]}"#.to_vec(),
                    br#"{"keys":[{"kty":"RSA","kid":"two","n":"AQAB","e":"AQAB"}]}"#.to_vec(),
                ])))),
            )
            .unwrap(),
        );
        let snapshot = verifier.refresh(now).await.unwrap();
        let rules = ProtectedIngress::ALL.into_iter().map(|ingress| {
            RouteRule::protected(
                ingress,
                ingress.required_capability(),
                ingress.required_resource(),
                ingress.required_effect(),
                ingress.required_transport(),
            )
        });
        let routes = Arc::new(RouteAuthority::new(rules).unwrap());
        let invalidation = Arc::new(InvalidationRegistry::new());
        invalidation
            .reconcile(ReconciledInvalidationState {
                complete: true,
                domain_generations: vec![(CommunityId::from_uuid(Uuid::from_u128(1)), 1)],
                observed_at: now,
            })
            .unwrap();
        let authority = Arc::new(RuntimeAuthority::new(
            Arc::clone(&routes),
            Arc::new(DenyCommitter),
            Arc::clone(&invalidation),
        ));
        let route_fingerprint = *routes.inventory_fingerprint();

        let mut startup = RuntimeStartup::enforce(config).unwrap();
        startup
            .migrations(SchemaWitness::new([1; 20], [2; 32], 33, true).unwrap())
            .unwrap();
        startup
            .database_roles(
                DatabaseRoleWitness::new(
                    Uuid::from_u128(2),
                    Uuid::from_u128(3),
                    Uuid::from_u128(4),
                    now,
                    true,
                )
                .unwrap(),
            )
            .unwrap();
        startup
            .verifier(
                VerifierReadinessWitness::new(
                    snapshot.stamp(),
                    now,
                    snapshot.fresh_until(),
                    snapshot.fresh_until(),
                )
                .unwrap(),
            )
            .unwrap();
        startup
            .reconciliation(RestoreReconciliationWitness::new([4; 32], now, true).unwrap())
            .unwrap();
        startup
            .install_state(
                RuntimeStateComponents {
                    routes,
                    verifier: Arc::clone(&verifier),
                    invalidation,
                    authority,
                    reconciliation_digest: [4; 32],
                    status_contract_fingerprint: [5; 32],
                    delegation: None,
                },
                RouteInventoryWitness::new(route_fingerprint, [5; 32]).unwrap(),
            )
            .await
            .unwrap();
        let installed = startup.finish().unwrap();
        assert!(installed.is_ready());
        assert!(installed.routes().is_ok());
        assert_eq!(installed.status_contract_fingerprint(), Some(&[5; 32]));
        verifier
            .refresh(now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert!(!installed.is_ready());
        assert!(installed.routes().is_err());
    }
}
