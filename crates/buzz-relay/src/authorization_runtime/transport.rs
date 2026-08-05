//! Provider-neutral protected-transport authorization.
//!
//! Transport handlers request one portable capability at the point of use.
//! An injected resolver owns provider evaluation, binding/admission ordering,
//! and finalization. This module accepts only a finalized access context as
//! authority, binds it back to the exact operation, and retains a guard for
//! mandatory pre-commit or pre-emission revalidation.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use buzz_auth::{
    AuthContext, AuthTransport, AuthorizationCapability, AuthorizationLease,
    AuthorizationLeaseValidator, AuthorizationProfileId, BindingVersion, LeaseUseRequirement,
    LeaseValidationError, PolicyVersion, SharedAuthorizationClock, VerificationOnlyDisposition,
    VerifiedFederatedAssertion, VerifiedNostrProof, VerifiedProviderEvidence,
};
use buzz_core::CommunityId;
use buzz_db::authorization_invalidation::AuthorizationSessionTarget;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::finalization::{AuthorizationMode, EnrollmentDisposition};

/// Compatibility policy for the inherited corporate-identity lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyIdentityLane {
    /// Runtime absent or explicitly Off: run the inherited verifier and
    /// finalizer without changing pre-O4 behavior.
    Legacy,
    /// Shadow or VerifyOnly: cryptographic verification and read-only policy
    /// evaluation are allowed, but identity state and public projections are
    /// immutable. This lane can never enroll, reactivate, strengthen, update,
    /// or retire a binding.
    ObserveOnly,
    /// Enforce or DenyProtected: the protected runtime owns the surface and
    /// legacy projection must not run.
    ProtectedEnforce,
}

/// Resolve the centralized identity compatibility policy for one exact domain.
pub fn legacy_identity_lane(
    state: &crate::state::AppState,
    authorization_domain: CommunityId,
) -> LegacyIdentityLane {
    legacy_identity_lane_for_mode(
        state
            .protected_transport()
            .and_then(|runtime| runtime.mode_for_domain(authorization_domain)),
    )
}

/// Resolve compatibility behavior from an already selected exact-domain mode.
pub const fn legacy_identity_lane_for_mode(mode: Option<AuthorizationMode>) -> LegacyIdentityLane {
    match mode {
        None | Some(AuthorizationMode::Off) => LegacyIdentityLane::Legacy,
        Some(AuthorizationMode::Shadow) | Some(AuthorizationMode::VerifyOnly) => {
            LegacyIdentityLane::ObserveOnly
        }
        Some(AuthorizationMode::Enforce | AuthorizationMode::DenyProtected) => {
            LegacyIdentityLane::ProtectedEnforce
        }
    }
}

/// Server-owned activation for one exact authorization domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainTransportPolicy {
    authorization_domain: CommunityId,
    mode: AuthorizationMode,
}

/// Exact result produced by one installed provider-neutral evidence resolver.
pub enum VerifiedProviderEvidenceResolution {
    /// No verified evidence was available for this request.
    Absent,
    /// Exactly one sealed evidence object was available.
    One(Arc<VerifiedProviderEvidence>),
    /// More than one source or value was present; callers must deny.
    Ambiguous,
}

impl fmt::Debug for VerifiedProviderEvidenceResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Absent => "VerifiedProviderEvidenceResolution::Absent",
            Self::One(_) => "VerifiedProviderEvidenceResolution::One([redacted])",
            Self::Ambiguous => "VerifiedProviderEvidenceResolution::Ambiguous",
        })
    }
}

/// Redacted failure from the deployment-installed evidence resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("verified provider evidence resolution failed")]
pub struct VerifiedProviderEvidenceResolutionError;

/// Deployment adapter that supplies already-verified provider-neutral evidence.
///
/// The adapter receives only typed protected-request metadata. Raw headers,
/// tokens, and transport classifications cannot construct the returned sealed
/// value. Implementations must return [`VerifiedProviderEvidenceResolution::Ambiguous`]
/// when multiple or conflicting sources are observed.
pub trait VerifiedProviderEvidenceResolver: Send + Sync {
    /// Resolve zero, exactly one, or ambiguous evidence for one exact request.
    fn resolve(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<VerifiedProviderEvidenceResolution, VerifiedProviderEvidenceResolutionError>;
}

impl DomainTransportPolicy {
    /// Construct policy from immutable server configuration.
    pub const fn from_server_configuration(
        authorization_domain: CommunityId,
        mode: AuthorizationMode,
    ) -> Self {
        Self {
            authorization_domain,
            mode,
        }
    }

    /// Exact configured domain.
    pub const fn authorization_domain(self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact configured activation mode.
    pub const fn mode(self) -> AuthorizationMode {
        self.mode
    }
}

/// Exact protected operation presented to the configured resolver.
#[derive(Clone)]
pub struct ProtectedOperationRequest {
    verified_proof: Arc<VerifiedNostrProof>,
    capability: AuthorizationCapability,
    correlation_id: Uuid,
    session_target: Option<AuthorizationSessionTarget>,
    surface: &'static str,
    cancellation: Option<CancellationToken>,
    verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
    enrollment_assertion: Option<Arc<VerifiedFederatedAssertion>>,
    enrollment_requested: bool,
    provider_evidence: Option<Arc<VerifiedProviderEvidence>>,
}

impl ProtectedOperationRequest {
    /// Build one exact operation after transport authentication.
    pub fn new(
        verified_proof: Arc<VerifiedNostrProof>,
        verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
        capability: AuthorizationCapability,
        correlation_id: Uuid,
        surface: &'static str,
    ) -> Result<Self, ProtectedTransportError> {
        Self::new_with_cancellation(
            verified_proof,
            verified_assertion,
            capability,
            correlation_id,
            surface,
            None,
            None,
        )
    }

    pub(crate) fn new_with_cancellation(
        verified_proof: Arc<VerifiedNostrProof>,
        verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
        capability: AuthorizationCapability,
        correlation_id: Uuid,
        surface: &'static str,
        session_target: Option<AuthorizationSessionTarget>,
        cancellation: Option<CancellationToken>,
    ) -> Result<Self, ProtectedTransportError> {
        if correlation_id.is_nil() {
            return Err(ProtectedTransportError::InvalidCorrelationId);
        }
        if surface.is_empty() {
            return Err(ProtectedTransportError::InvalidSurface);
        }
        if !crate::protected_surface::protected_operation_matches(
            surface,
            verified_proof.authorized_transport(),
            capability,
        ) {
            return Err(ProtectedTransportError::SurfaceCapabilityMismatch);
        }
        Ok(Self {
            verified_proof,
            capability,
            correlation_id,
            session_target,
            surface,
            cancellation,
            verified_assertion,
            enrollment_assertion: None,
            enrollment_requested: false,
            provider_evidence: None,
        })
    }

    fn new_enrollment(
        verified_proof: Arc<VerifiedNostrProof>,
        assertion: Option<Arc<VerifiedFederatedAssertion>>,
        correlation_id: Uuid,
        surface: &'static str,
    ) -> Result<Self, ProtectedTransportError> {
        let mut request = Self::new(
            verified_proof,
            assertion.clone(),
            AuthorizationCapability::InviteClaim,
            correlation_id,
            surface,
        )?;
        request.enrollment_assertion = assertion;
        request.enrollment_requested = true;
        Ok(request)
    }

    /// Exact server-resolved authorization domain.
    pub fn authorization_domain(&self) -> CommunityId {
        self.verified_proof.authorization_domain()
    }

    /// Transport carrying this operation.
    pub fn transport(&self) -> AuthTransport {
        self.verified_proof.authorized_transport()
    }

    /// Authenticated actor.
    pub fn actor_pubkey(&self) -> nostr::PublicKey {
        self.verified_proof.actor_pubkey()
    }

    /// Verified Nostr owner for delegated authority, when present.
    pub fn owner_pubkey(&self) -> Option<nostr::PublicKey> {
        self.verified_proof
            .verified_delegation()
            .map(buzz_auth::VerifiedTransportDelegation::owner_pubkey)
    }

    /// Sealed verifier evidence for this exact transport request or session.
    pub fn verified_proof(&self) -> &Arc<VerifiedNostrProof> {
        &self.verified_proof
    }

    /// Direct verified assertion retained only for first-enrollment resolution.
    pub fn enrollment_assertion(&self) -> Option<&Arc<VerifiedFederatedAssertion>> {
        self.enrollment_assertion.as_ref()
    }

    /// Current direct assertion verified by the transport identity adapter.
    pub fn verified_assertion(&self) -> Option<&Arc<VerifiedFederatedAssertion>> {
        self.verified_assertion.as_ref()
    }

    /// Whether this request is the dedicated first-enrollment operation.
    pub const fn enrollment_requested(&self) -> bool {
        self.enrollment_requested
    }

    /// Exactly one provider-neutral evidence object resolved for this request.
    pub fn provider_evidence(&self) -> Option<&Arc<VerifiedProviderEvidence>> {
        self.provider_evidence.as_ref()
    }

    /// Exact dynamically selected capability.
    pub const fn capability(&self) -> AuthorizationCapability {
        self.capability
    }

    /// Correlation identifier for this decision.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Exact server-issued target for a long-lived transport session.
    pub const fn session_target(&self) -> Option<AuthorizationSessionTarget> {
        self.session_target
    }

    /// Stable low-cardinality surface name for resolver telemetry.
    pub const fn surface(&self) -> &'static str {
        self.surface
    }

    /// Server-owned cancellation for a leased session, when applicable.
    pub fn cancellation(&self) -> Option<CancellationToken> {
        self.cancellation.clone()
    }
}

impl fmt::Debug for ProtectedOperationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedOperationRequest")
            .field("authorization_domain", &"[redacted]")
            .field("transport", &self.transport())
            .field("actor_pubkey", &"[redacted]")
            .field("owner_pubkey", &"[redacted]")
            .field("capability", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("surface", &self.surface)
            .finish()
    }
}

/// Resolver failure that carries only a stable, non-sensitive code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("protected authorization resolver denied the operation ({code})")]
pub struct ProtectedResolutionError {
    code: &'static str,
}

impl ProtectedResolutionError {
    /// Construct a sanitized resolver denial.
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }

    /// Stable denial code suitable for metrics and protocol mapping.
    pub const fn code(self) -> &'static str {
        self.code
    }
}

/// Exact-domain resolver implemented by the authorization adapter.
///
/// Direct and delegated actors intentionally enter through the same method.
/// The request's verified owner is evidence, never a separate bypass path.
#[async_trait]
pub trait ProtectedAuthorizationResolver: Send + Sync {
    /// Resolve and fully finalize current authority for one operation.
    async fn resolve(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<ProtectedResolution, ProtectedResolutionError>;

    /// Run a read-only observation without producing authority or mutating
    /// identity, binding, membership, projection, or lease state.
    async fn observe(
        &self,
        _request: &ProtectedOperationRequest,
    ) -> Result<(), ProtectedResolutionError> {
        Err(ProtectedResolutionError::new(
            "observational_provider_unavailable",
        ))
    }

    /// Resolve display-only current binding status without granting access or
    /// mutating binding, membership, projection, or lease state.
    async fn present(
        &self,
        _request: &ProtectedOperationRequest,
    ) -> Result<ProtectedStatusResolution, ProtectedResolutionError> {
        Err(ProtectedResolutionError::new(
            "client_status_presentation_unavailable",
        ))
    }
}

/// Display-only result retained with its exact invalidation observer.
#[allow(dead_code)]
pub struct ProtectedStatusResolution {
    disposition: VerificationOnlyDisposition,
    observer: Arc<dyn LeaseCurrentStateObserver>,
    evaluation_generation: u64,
}

#[allow(dead_code)]
impl ProtectedStatusResolution {
    /// Couple one current status to the invalidation fence captured for it.
    pub fn new(
        disposition: VerificationOnlyDisposition,
        observer: Arc<dyn LeaseCurrentStateObserver>,
        evaluation_generation: u64,
    ) -> Self {
        Self {
            disposition,
            observer,
            evaluation_generation,
        }
    }

    /// Consume the display result for dedicated delivery and reconciliation.
    pub(crate) fn into_parts(
        self,
    ) -> (
        VerificationOnlyDisposition,
        Arc<dyn LeaseCurrentStateObserver>,
        u64,
    ) {
        (self.disposition, self.observer, self.evaluation_generation)
    }
}

impl fmt::Debug for ProtectedStatusResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedStatusResolution")
            .field("disposition", &"[redacted]")
            .field("observer", &"[registered]")
            .field("evaluation_generation", &"[redacted]")
            .finish()
    }
}

/// Finalized resolver output.
///
/// Fields are private so enforcing access cannot be constructed without the
/// exact per-decision observer registered during finalization.
pub struct ProtectedResolution {
    kind: ProtectedResolutionKind,
}

enum ProtectedResolutionKind {
    Access {
        context: Box<AuthContext>,
        observer: Arc<dyn LeaseCurrentStateObserver>,
    },
    VerificationOnly(VerificationOnlyDisposition),
    Enrollment {
        disposition: EnrollmentDisposition,
        observer: Arc<dyn LeaseCurrentStateObserver>,
    },
}

impl ProtectedResolution {
    /// Couple finalized access with its exact read-only invalidation observer.
    ///
    /// Enforcing access has no observer-free constructor:
    ///
    /// ```compile_fail
    /// use buzz_auth::AuthContext;
    /// use buzz_relay::authorization_runtime::transport::ProtectedResolution;
    ///
    /// fn invalid(context: Box<AuthContext>) -> ProtectedResolution {
    ///     ProtectedResolution::access(context)
    /// }
    /// ```
    pub fn access(context: Box<AuthContext>, observer: Arc<dyn LeaseCurrentStateObserver>) -> Self {
        Self {
            kind: ProtectedResolutionKind::Access { context, observer },
        }
    }

    /// Preserve a display-only result without manufacturing access state.
    pub fn verification_only(disposition: VerificationOnlyDisposition) -> Self {
        Self {
            kind: ProtectedResolutionKind::VerificationOnly(disposition),
        }
    }

    /// Couple staged direct enrollment with its exact invalidation observer.
    pub fn enrollment(
        disposition: EnrollmentDisposition,
        observer: Arc<dyn LeaseCurrentStateObserver>,
    ) -> Self {
        Self {
            kind: ProtectedResolutionKind::Enrollment {
                disposition,
                observer,
            },
        }
    }
}

impl fmt::Debug for ProtectedResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedResolution")
            .field("kind", &"[redacted]")
            .finish()
    }
}

/// Current trusted state for every dependency that can invalidate a lease.
#[derive(Clone, PartialEq, Eq)]
pub struct LeaseCurrentState {
    binding_version: BindingVersion,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
}

impl LeaseCurrentState {
    /// Construct state from the trusted finalization/invalidation runtime.
    pub const fn from_trusted_runtime(
        binding_version: BindingVersion,
        profile_id: AuthorizationProfileId,
        policy_version: PolicyVersion,
    ) -> Self {
        Self {
            binding_version,
            profile_id,
            policy_version,
        }
    }
}

impl fmt::Debug for LeaseCurrentState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeaseCurrentState")
            .field("binding_version", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .finish()
    }
}

/// Exact per-decision observer used by a retained lease guard.
///
/// Implementations retain the registered invalidation fence and session
/// dependencies for this exact finalization. They must check those dependencies
/// and read current binding/profile/policy state before returning. The observer
/// is intentionally read-only; transport adoption cannot publish invalidations.
pub trait LeaseCurrentStateObserver: Send + Sync {
    /// Validate the registered fence/dependencies and return current versions.
    fn observe_current(&self) -> Result<LeaseCurrentState, LeaseCurrentStateError>;

    /// Return the captured durable generation and exact selector dependencies
    /// required by a transaction-owned mutation commit.
    ///
    /// Read-only observers may retain the default fail-closed implementation.
    fn observe_commit_fence(
        &self,
    ) -> Result<super::executor::AuthorizationCommitFence, LeaseCurrentStateError> {
        Err(LeaseCurrentStateError::Unavailable)
    }
}

/// Fail-closed current-state result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeaseCurrentStateError {
    /// Binding, profile, or policy state changed.
    #[error("protected authorization state is stale")]
    Stale,
    /// Current state could not be read.
    #[error("protected authorization state is unavailable")]
    Unavailable,
}

/// Optional protected-transport runtime.
///
/// Domains absent from this map, and domains explicitly configured `Off`, use
/// legacy behavior. An enforcing domain never falls back after a resolver or
/// lease failure.
pub struct ProtectedTransportRuntime {
    domains: HashMap<CommunityId, AuthorizationMode>,
    resolver: Arc<dyn ProtectedAuthorizationResolver>,
    provider_evidence_resolver: Option<Arc<dyn VerifiedProviderEvidenceResolver>>,
    validator: AuthorizationLeaseValidator,
}

impl ProtectedTransportRuntime {
    /// Build an exact-domain runtime, rejecting duplicate configuration.
    pub fn new(
        policies: impl IntoIterator<Item = DomainTransportPolicy>,
        resolver: Arc<dyn ProtectedAuthorizationResolver>,
        clock: SharedAuthorizationClock,
    ) -> Result<Self, ProtectedTransportError> {
        let mut domains = HashMap::new();
        for policy in policies {
            if domains
                .insert(policy.authorization_domain, policy.mode)
                .is_some()
            {
                return Err(ProtectedTransportError::AmbiguousDomainPolicy);
            }
        }
        Ok(Self {
            domains,
            resolver,
            provider_evidence_resolver: None,
            validator: AuthorizationLeaseValidator::new(clock),
        })
    }

    /// Install exactly one immutable provider-neutral evidence resolver.
    pub fn with_provider_evidence_resolver(
        mut self,
        resolver: Arc<dyn VerifiedProviderEvidenceResolver>,
    ) -> Self {
        self.provider_evidence_resolver = Some(resolver);
        self
    }

    /// Whether the production composition installed the neutral evidence seam.
    pub const fn has_provider_evidence_resolver(&self) -> bool {
        self.provider_evidence_resolver.is_some()
    }

    fn resolve_provider_evidence(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<ProtectedOperationRequest, ProtectedTransportError> {
        let Some(resolver) = self.provider_evidence_resolver.as_ref() else {
            return Ok(request.clone());
        };
        let resolution = resolver
            .resolve(request)
            .map_err(|_| ProtectedTransportError::ProviderEvidenceUnavailable)?;
        match resolution {
            VerifiedProviderEvidenceResolution::Absent
                if request.verified_assertion().is_none()
                    && request.provider_evidence().is_none() =>
            {
                Err(ProtectedTransportError::Resolution(
                    ProtectedResolutionError::new("provider_evidence_missing"),
                ))
            }
            VerifiedProviderEvidenceResolution::Absent => Ok(request.clone()),
            VerifiedProviderEvidenceResolution::Ambiguous => {
                Err(ProtectedTransportError::AmbiguousProviderEvidence)
            }
            VerifiedProviderEvidenceResolution::One(evidence) => {
                if request.verified_assertion.is_some() || request.provider_evidence.is_some() {
                    return Err(ProtectedTransportError::ConflictingProviderEvidence);
                }
                let mut resolved = request.clone();
                resolved.provider_evidence = Some(evidence);
                Ok(resolved)
            }
        }
    }

    /// Return the exact configured mode, if this runtime owns the domain.
    pub fn mode_for_domain(&self, domain: CommunityId) -> Option<AuthorizationMode> {
        self.domains.get(&domain).copied()
    }

    /// Exact domains whose background effects must remain unavailable.
    pub fn enforcing_domains(&self) -> Vec<CommunityId> {
        self.domains
            .iter()
            .filter_map(|(domain, mode)| mode.protects_surfaces().then_some(*domain))
            .collect()
    }

    /// Resolve one protected operation or explicitly preserve legacy behavior.
    pub async fn authorize(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<ProtectedAuthorization, ProtectedTransportError> {
        let Some(mode) = self.mode_for_domain(request.authorization_domain()) else {
            return Ok(ProtectedAuthorization::Legacy);
        };
        match mode {
            AuthorizationMode::Off => Ok(ProtectedAuthorization::Legacy),
            // Transport adoption has no mutation-free decision-only resolver.
            // Preserve legacy access in both observational modes and leave
            // their status evaluation to the lane that owns that API.
            AuthorizationMode::Shadow | AuthorizationMode::VerifyOnly => {
                // Observation failures are telemetry only. These modes must
                // never alter the inherited access result.
                if let Ok(request) = self.resolve_provider_evidence(request) {
                    let _ = self.resolver.observe(&request).await;
                }
                Ok(ProtectedAuthorization::Legacy)
            }
            AuthorizationMode::DenyProtected => deny_protected_request(request),
            AuthorizationMode::Enforce => {
                let request = self.resolve_provider_evidence(request)?;
                let resolution = self
                    .resolver
                    .resolve(&request)
                    .await
                    .map_err(ProtectedTransportError::Resolution)?;
                let (context, observer) = match resolution.kind {
                    ProtectedResolutionKind::Access { context, observer } => (context, observer),
                    ProtectedResolutionKind::VerificationOnly(_status) => {
                        return Err(ProtectedTransportError::VerificationOnlyCannotGrant)
                    }
                    ProtectedResolutionKind::Enrollment { .. } => {
                        return Err(ProtectedTransportError::EnrollmentCannotGrantAccess)
                    }
                };
                let context: Arc<AuthContext> = Arc::from(context);
                let authority = ProtectedOperationAuthority {
                    context,
                    capability: request.capability(),
                    validator: self.validator.clone(),
                    observer,
                };
                authority.validate_exact_request(&request)?;
                authority.revalidate()?;
                Ok(ProtectedAuthorization::Access(authority))
            }
        }
    }

    /// Resolve a dedicated client presentation only in VerifyOnly or Enforce.
    /// Off and Shadow remain byte-for-byte non-presenting legacy behavior.
    pub async fn present_status(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<Option<ProtectedStatusResolution>, ProtectedTransportError> {
        match self.mode_for_domain(request.authorization_domain()) {
            Some(AuthorizationMode::VerifyOnly | AuthorizationMode::Enforce) => {
                let request = self.resolve_provider_evidence(request)?;
                self.resolver
                    .present(&request)
                    .await
                    .map(Some)
                    .map_err(ProtectedTransportError::Resolution)
            }
            Some(AuthorizationMode::DenyProtected) => Err(ProtectedTransportError::DenyProtected),
            None | Some(AuthorizationMode::Off | AuthorizationMode::Shadow) => Ok(None),
        }
    }

    /// Resolve direct first-enrollment evidence without requiring an active binding.
    pub async fn authorize_enrollment(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<ProtectedEnrollmentAuthorization, ProtectedTransportError> {
        let Some(mode) = self.mode_for_domain(request.authorization_domain()) else {
            return Ok(ProtectedEnrollmentAuthorization::Legacy);
        };
        match mode {
            AuthorizationMode::Off | AuthorizationMode::Shadow | AuthorizationMode::VerifyOnly => {
                Ok(ProtectedEnrollmentAuthorization::Legacy)
            }
            AuthorizationMode::DenyProtected => Err(ProtectedTransportError::DenyProtected),
            AuthorizationMode::Enforce => {
                let request = self.resolve_provider_evidence(request)?;
                if request.capability() != AuthorizationCapability::InviteClaim
                    || !request.enrollment_requested()
                    || request.owner_pubkey().is_some()
                {
                    return Err(ProtectedTransportError::EnrollmentEvidenceRequired);
                }
                let resolution = self
                    .resolver
                    .resolve(&request)
                    .await
                    .map_err(ProtectedTransportError::Resolution)?;
                let (disposition, observer) = match resolution.kind {
                    ProtectedResolutionKind::Enrollment {
                        disposition,
                        observer,
                    } => (disposition, observer),
                    _ => return Err(ProtectedTransportError::EnrollmentEvidenceRequired),
                };
                let authority = ProtectedEnrollmentAuthority {
                    disposition,
                    observer,
                    validator: self.validator.clone(),
                };
                authority.validate_exact_request(&request)?;
                authority.revalidate()?;
                Ok(ProtectedEnrollmentAuthorization::Enrollment(authority))
            }
        }
    }
}

/// Consult the optional runtime for one authenticated relay operation.
///
/// This is the common transport seam used by HTTP, WebSocket, Git, media, and
/// audio handlers. Runtime absence is the only implicit legacy case.
pub async fn authorize_if_configured(
    state: &crate::state::AppState,
    verified_proof: Arc<VerifiedNostrProof>,
    verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
    capability: AuthorizationCapability,
    correlation_id: Uuid,
    surface: &'static str,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    let Some(runtime) = state.protected_transport() else {
        return Ok(ProtectedAuthorization::Legacy);
    };
    let request = ProtectedOperationRequest::new(
        verified_proof,
        verified_assertion,
        capability,
        correlation_id,
        surface,
    )?;
    runtime.authorize(&request).await
}

/// Consult the runtime for a leased session and expose only its server-owned
/// cancellation token to the resolver's invalidation registration.
#[allow(clippy::too_many_arguments)]
pub async fn authorize_session_if_configured(
    state: &crate::state::AppState,
    verified_proof: Arc<VerifiedNostrProof>,
    verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
    capability: AuthorizationCapability,
    correlation_id: Uuid,
    surface: &'static str,
    session_id: Uuid,
    cancellation: CancellationToken,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    if state.protected_transport().is_none() {
        return Ok(ProtectedAuthorization::Legacy);
    }
    let session_target = state
        .conn_manager
        .authorization_session_target(session_id)
        .filter(|target| target.session_id() == session_id)
        .ok_or(ProtectedTransportError::InvalidSessionId)?;
    let authority = authorize_exact_session_if_configured(
        state,
        verified_proof,
        verified_assertion,
        capability,
        correlation_id,
        surface,
        session_target,
        cancellation,
    )
    .await?;
    state
        .conn_manager
        .retain_protected_session_authority(session_id, &authority);
    Ok(authority)
}

/// Consult the runtime for a server-issued session target that is not managed
/// by the ordinary relay connection registry (for example, protected audio).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn authorize_exact_session_if_configured(
    state: &crate::state::AppState,
    verified_proof: Arc<VerifiedNostrProof>,
    verified_assertion: Option<Arc<VerifiedFederatedAssertion>>,
    capability: AuthorizationCapability,
    correlation_id: Uuid,
    surface: &'static str,
    session_target: AuthorizationSessionTarget,
    cancellation: CancellationToken,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    let Some(runtime) = state.protected_transport() else {
        return Ok(ProtectedAuthorization::Legacy);
    };
    let request = ProtectedOperationRequest::new_with_cancellation(
        verified_proof,
        verified_assertion,
        capability,
        correlation_id,
        surface,
        Some(session_target),
        Some(cancellation),
    )?;
    runtime.authorize(&request).await
}

/// Resolve staged direct authority for atomic invite enrollment.
pub async fn authorize_enrollment_if_configured(
    state: &crate::state::AppState,
    verified_proof: Arc<VerifiedNostrProof>,
    assertion: Option<Arc<VerifiedFederatedAssertion>>,
    correlation_id: Uuid,
) -> Result<ProtectedEnrollmentAuthorization, ProtectedTransportError> {
    let Some(runtime) = state.protected_transport() else {
        return Ok(ProtectedEnrollmentAuthorization::Legacy);
    };
    let request = ProtectedOperationRequest::new_enrollment(
        verified_proof,
        assertion,
        correlation_id,
        "invite.claim",
    )?;
    runtime.authorize_enrollment(&request).await
}

/// Whether an enforcing domain can resolve provider-neutral verified evidence.
pub fn provider_evidence_resolver_is_installed(
    state: &crate::state::AppState,
    authorization_domain: CommunityId,
) -> bool {
    state.protected_transport().is_some_and(|runtime| {
        runtime.mode_for_domain(authorization_domain) == Some(AuthorizationMode::Enforce)
            && runtime.has_provider_evidence_resolver()
    })
}

/// Preserve legacy behavior only when an unwired surface cannot enter an
/// enforcing exact-domain runtime.
///
/// This helper never constructs a resolver request from caller-supplied key
/// primitives. It exists solely to make missing typed evidence fail closed on
/// enforcing domains while retaining the configured Off/Shadow/VerifyOnly
/// behavior for legacy-only authentication paths.
pub fn authorize_unwired_if_configured(
    state: &crate::state::AppState,
    authorization_domain: CommunityId,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    authorize_unwired_for_mode(
        state
            .protected_transport()
            .and_then(|runtime| runtime.mode_for_domain(authorization_domain)),
    )
}

/// Deny an unproved mutation in Enforce before any backend-visible work starts.
///
/// Alternate helpers such as secret-triggered workflow execution have no
/// request proof that a protected resolver can finalize. Until a transaction-
/// owning executor carries durable authority into their commit, they must be
/// unavailable in Enforce while preserving every legacy lane.
pub fn require_unwired_atomic_mutation_if_configured(
    state: &crate::state::AppState,
    authorization_domain: CommunityId,
) -> Result<(), ProtectedTransportError> {
    authorize_unwired_if_configured(state, authorization_domain)?.require_atomic_mutation()
}

fn authorize_unwired_for_mode(
    mode: Option<AuthorizationMode>,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    match mode {
        None
        | Some(AuthorizationMode::Off)
        | Some(AuthorizationMode::Shadow)
        | Some(AuthorizationMode::VerifyOnly) => Ok(ProtectedAuthorization::Legacy),
        Some(AuthorizationMode::Enforce) => Err(ProtectedTransportError::MissingVerifiedProof),
        Some(AuthorizationMode::DenyProtected) => Err(ProtectedTransportError::DenyProtected),
    }
}

fn deny_protected_request(
    request: &ProtectedOperationRequest,
) -> Result<ProtectedAuthorization, ProtectedTransportError> {
    if let Some(cancellation) = request.cancellation() {
        cancellation.cancel();
    }
    Err(ProtectedTransportError::DenyProtected)
}

impl fmt::Debug for ProtectedTransportRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedTransportRuntime")
            .field("domains", &"[redacted]")
            .field("resolver", &"[configured]")
            .field("provider_evidence_resolver", &"[configured]")
            .field("validator", &self.validator)
            .finish()
    }
}

/// Result of consulting the direct first-enrollment runtime.
#[must_use]
pub enum ProtectedEnrollmentAuthorization {
    /// Runtime absent or configured non-enforcing.
    Legacy,
    /// Enforcing staged enrollment with no binding or membership yet.
    Enrollment(ProtectedEnrollmentAuthority),
}

impl ProtectedEnrollmentAuthorization {
    /// Seal the enrollment decision for transaction-owned execution.
    pub fn seal_postgres_enrollment(
        &self,
        operation_id: super::executor::ProtectedOperationId,
        operation_kind: &'static str,
        request_fingerprint: [u8; 32],
    ) -> Result<Option<super::executor::SealedEnrollmentPermit>, ProtectedTransportError> {
        match self {
            Self::Legacy => Ok(None),
            Self::Enrollment(authority) => super::executor::SealedEnrollmentPermit::from_authority(
                authority,
                operation_id,
                operation_kind,
                request_fingerprint,
            )
            .map(Some),
        }
    }

    /// Whether this decision is enforcing.
    pub const fn is_enforcing(&self) -> bool {
        matches!(self, Self::Enrollment(_))
    }
}

/// Staged direct authority that cannot be consumed as ordinary access.
pub struct ProtectedEnrollmentAuthority {
    disposition: EnrollmentDisposition,
    observer: Arc<dyn LeaseCurrentStateObserver>,
    validator: AuthorizationLeaseValidator,
}

impl ProtectedEnrollmentAuthority {
    pub(super) const fn disposition(&self) -> &EnrollmentDisposition {
        &self.disposition
    }

    pub(super) fn observer(&self) -> &dyn LeaseCurrentStateObserver {
        self.observer.as_ref()
    }

    fn validate_exact_request(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<(), ProtectedTransportError> {
        if self.disposition.authorization_domain() != request.authorization_domain()
            || self.disposition.actor_pubkey() != request.actor_pubkey()
            || self.disposition.correlation_id() != request.correlation_id()
            || request.capability() != AuthorizationCapability::InviteClaim
        {
            return Err(ProtectedTransportError::FinalizedContextMismatch);
        }
        Ok(())
    }

    /// Recheck time and the captured invalidation fence before sealing.
    pub fn revalidate(&self) -> Result<(), ProtectedTransportError> {
        self.validator
            .seconds_until(self.disposition.expires_at())?;
        self.observer.observe_commit_fence()?;
        Ok(())
    }
}

impl fmt::Debug for ProtectedEnrollmentAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedEnrollmentAuthority([redacted])")
    }
}

/// Result of consulting the optional exact-domain runtime.
#[derive(Clone)]
#[must_use]
pub enum ProtectedAuthorization {
    /// Runtime absent for this exact domain or configured non-enforcing.
    Legacy,
    /// Enforcing access with a retained operation guard.
    Access(ProtectedOperationAuthority),
}

impl ProtectedAuthorization {
    /// Revalidate enforcing authority; legacy mode is a no-op.
    pub fn revalidate(&self) -> Result<(), ProtectedTransportError> {
        match self {
            Self::Legacy => Ok(()),
            Self::Access(authority) => authority.revalidate(),
        }
    }

    /// Release already-fetched protected data only after a final authority check.
    ///
    /// Callers place this at the last synchronous boundary before constructing
    /// a response, queue entry, or stream item. Keeping the fetched value owned
    /// by this method makes the post-fetch/pre-emission ordering explicit.
    pub fn release_fetched<T>(&self, value: T) -> Result<T, ProtectedTransportError> {
        self.revalidate()?;
        Ok(value)
    }

    /// Require an atomic mutation coordinator for an enforcing operation.
    ///
    /// Legacy lanes retain their exact behavior. Enforcing callers must select
    /// the transaction/CAS executor for their registered surface; an adjacent
    /// authorization check is never treated as an atomic fence.
    pub fn require_atomic_mutation(&self) -> Result<(), ProtectedTransportError> {
        require_atomic_mutation_executor(matches!(self, Self::Access(_)))
    }

    /// Seal an enforcing mutation for transaction-owned PostgreSQL execution.
    ///
    /// Legacy lanes return `None` and retain their existing mutation path.
    /// Enforcing lanes return a permit that cannot be constructed from request
    /// fields or an adjacent authorization check.
    pub fn seal_postgres_mutation(
        &self,
        operation_id: super::executor::ProtectedOperationId,
        operation_kind: &'static str,
        request_fingerprint: [u8; 32],
    ) -> Result<Option<super::executor::SealedOperationPermit>, ProtectedTransportError> {
        match self {
            Self::Legacy => Ok(None),
            Self::Access(authority) => super::executor::SealedOperationPermit::from_authority(
                authority,
                operation_id,
                operation_kind,
                request_fingerprint,
            )
            .map(Some),
        }
    }

    /// Seal opaque sender authority for one ephemeral event delivered across
    /// the trusted relay Redis plane. The resulting claim still requires a
    /// relay signature and writer-database revalidation on the receiving pod.
    pub(crate) fn seal_ephemeral_delivery(
        &self,
        event_id: [u8; 32],
    ) -> Result<Option<super::executor::EphemeralAuthorityClaim>, ProtectedTransportError> {
        match self {
            Self::Legacy => Ok(None),
            Self::Access(authority) => {
                super::executor::EphemeralAuthorityClaim::from_authority(authority, event_id)
                    .map(Some)
            }
        }
    }

    /// Whether this value carries enforcing authority rather than legacy mode.
    pub const fn is_enforcing(&self) -> bool {
        matches!(self, Self::Access(_))
    }

    /// Hard lease expiry for enforcing authority.
    pub fn expires_at(&self) -> Option<u64> {
        match self {
            Self::Legacy => None,
            Self::Access(authority) => Some(authority.expires_at()),
        }
    }

    /// Delay to hard expiry using the same injected clock as lease validation.
    pub fn expiry_delay(&self) -> Result<Option<std::time::Duration>, ProtectedTransportError> {
        match self {
            Self::Legacy => Ok(None),
            Self::Access(authority) => authority.expiry_delay().map(Some),
        }
    }
}

fn require_atomic_mutation_executor(enforcing: bool) -> Result<(), ProtectedTransportError> {
    if enforcing {
        Err(ProtectedTransportError::AtomicMutationFenceUnavailable)
    } else {
        Ok(())
    }
}

impl fmt::Debug for ProtectedAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => formatter.write_str("ProtectedAuthorization::Legacy"),
            Self::Access(_) => formatter.write_str("ProtectedAuthorization::Access([redacted])"),
        }
    }
}

/// Retained authority for request, stream, and pre-commit checkpoints.
#[derive(Clone)]
pub struct ProtectedOperationAuthority {
    context: Arc<AuthContext>,
    capability: AuthorizationCapability,
    validator: AuthorizationLeaseValidator,
    observer: Arc<dyn LeaseCurrentStateObserver>,
}

impl ProtectedOperationAuthority {
    pub(super) fn context(&self) -> &AuthContext {
        &self.context
    }

    pub(super) fn observer(&self) -> &dyn LeaseCurrentStateObserver {
        self.observer.as_ref()
    }

    fn lease(&self) -> Result<&AuthorizationLease, ProtectedTransportError> {
        self.context
            .authorization_lease()
            .ok_or(ProtectedTransportError::MissingAccessLease)
    }

    fn requirement(
        &self,
        current: &LeaseCurrentState,
    ) -> Result<LeaseUseRequirement, ProtectedTransportError> {
        let lease = self.lease()?;
        Ok(LeaseUseRequirement {
            context_version: lease.context_version(),
            lease_version: lease.lease_version(),
            authorization_domain: lease.authorization_domain(),
            transport: lease.transport(),
            actor_pubkey: lease.actor_pubkey(),
            binding_id: lease.binding_id(),
            binding_version: current.binding_version,
            profile_id: current.profile_id.clone(),
            policy_version: current.policy_version.clone(),
            capability: self.capability,
        })
    }

    fn validate_exact_request(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<(), ProtectedTransportError> {
        if self.context.tenant().community() != request.authorization_domain()
            || self.context.transport() != request.transport()
            || self.context.pubkey() != request.actor_pubkey()
            || self.context.agent_owner_pubkey() != request.owner_pubkey()
            || !exact_correlation_matches(self.context.correlation_id(), request.correlation_id())
        {
            return Err(ProtectedTransportError::FinalizedContextMismatch);
        }
        Ok(())
    }

    /// Revalidate current state, exact capability, typed versions, and expiry.
    pub fn revalidate(&self) -> Result<(), ProtectedTransportError> {
        let current = self.observer.observe_current()?;
        self.context
            .authorize_lease_use(&self.validator, &self.requirement(&current)?)?;
        Ok(())
    }

    /// Exact operation capability retained by this authority.
    pub const fn capability(&self) -> AuthorizationCapability {
        self.capability
    }

    /// Hard conservative expiry after which revalidation fails.
    pub fn expires_at(&self) -> u64 {
        self.context
            .authorization_lease()
            .map_or(0, AuthorizationLease::expires_at)
    }

    fn expiry_delay(&self) -> Result<std::time::Duration, ProtectedTransportError> {
        let expires_at = self.expires_at();
        let seconds = self.validator.seconds_until(expires_at)?;
        Ok(std::time::Duration::from_secs(seconds))
    }
}

fn exact_correlation_matches(finalized: Uuid, requested: Uuid) -> bool {
    finalized == requested
}

impl fmt::Debug for ProtectedOperationAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedOperationAuthority")
            .field("context", &"[redacted]")
            .field("capability", &"[redacted]")
            .field("validator", &self.validator)
            .field("observer", &"[configured]")
            .finish()
    }
}

/// Fail-closed protected-transport error.
#[derive(Debug, Error)]
pub enum ProtectedTransportError {
    /// Duplicate exact-domain activation is ambiguous.
    #[error("protected authorization policy is ambiguous for this domain")]
    AmbiguousDomainPolicy,
    /// Correlation IDs must be non-nil.
    #[error("protected authorization correlation id is invalid")]
    InvalidCorrelationId,
    /// Session IDs for long-lived transports must be non-nil.
    #[error("protected authorization session id is invalid")]
    InvalidSessionId,
    /// Surface labels must be stable non-empty server constants.
    #[error("protected authorization surface is invalid")]
    InvalidSurface,
    /// The surface, sealed proof transport, and capability do not match.
    #[error("protected authorization surface does not match proof and capability")]
    SurfaceCapabilityMismatch,
    /// An enforcing surface did not retain sealed verifier evidence.
    #[error("protected authorization requires verified transport evidence")]
    MissingVerifiedProof,
    /// The installed evidence adapter could not resolve the request.
    #[error("verified provider evidence is unavailable")]
    ProviderEvidenceUnavailable,
    /// Multiple provider-neutral evidence values or sources were present.
    #[error("verified provider evidence is ambiguous")]
    AmbiguousProviderEvidence,
    /// JWT-derived and provider-neutral evidence were both present.
    #[error("verified provider evidence sources conflict")]
    ConflictingProviderEvidence,
    /// The exact domain is in the explicit fail-safe protected-denial mode.
    #[error("protected authorization is unavailable in deny-protected mode")]
    DenyProtected,
    /// Resolver denied or could not evaluate current policy.
    #[error(transparent)]
    Resolution(#[from] ProtectedResolutionError),
    /// Display verification is deliberately not access authority.
    #[error("verification-only authorization cannot grant protected access")]
    VerificationOnlyCannotGrant,
    /// A staged enrollment result was presented as ordinary access.
    #[error("first-enrollment authorization cannot grant ordinary protected access")]
    EnrollmentCannotGrantAccess,
    /// Invite enrollment lacked matching direct assertion/provider evidence.
    #[error("protected invite enrollment requires direct verified evidence")]
    EnrollmentEvidenceRequired,
    /// Resolver returned a context for another exact operation.
    #[error("finalized authorization context does not match protected operation")]
    FinalizedContextMismatch,
    /// Enforcing disposition lacked its bounded lease.
    #[error("enforcing authorization context is missing its access lease")]
    MissingAccessLease,
    /// Current binding/profile/policy observation failed closed.
    #[error(transparent)]
    CurrentState(#[from] LeaseCurrentStateError),
    /// Typed lease validation failed.
    #[error(transparent)]
    Lease(#[from] LeaseValidationError),
    /// No transaction- or CAS-coupled mutation coordinator owns this commit.
    #[error("protected mutation requires an atomic authorization fence")]
    AtomicMutationFenceUnavailable,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use buzz_auth::{
        AuthorizationClock, AuthorizationClockError, AuthorizationTime, VerifiedEvidenceAdapter,
    };
    use nostr::{EventBuilder, Keys, RelayUrl};

    use super::*;

    struct UnavailableResolver;

    #[async_trait]
    impl ProtectedAuthorizationResolver for UnavailableResolver {
        async fn resolve(
            &self,
            _request: &ProtectedOperationRequest,
        ) -> Result<ProtectedResolution, ProtectedResolutionError> {
            Err(ProtectedResolutionError::new("synthetic_unavailable"))
        }
    }

    struct PanicResolver;

    #[async_trait]
    impl ProtectedAuthorizationResolver for PanicResolver {
        async fn resolve(
            &self,
            _request: &ProtectedOperationRequest,
        ) -> Result<ProtectedResolution, ProtectedResolutionError> {
            panic!("non-enforcing mode must not call a finalizing resolver")
        }
    }

    struct ObservingResolver(AtomicUsize);

    #[async_trait]
    impl ProtectedAuthorizationResolver for ObservingResolver {
        async fn resolve(
            &self,
            _request: &ProtectedOperationRequest,
        ) -> Result<ProtectedResolution, ProtectedResolutionError> {
            panic!("observational mode must not finalize authority")
        }

        async fn observe(
            &self,
            _request: &ProtectedOperationRequest,
        ) -> Result<(), ProtectedResolutionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ProtectedResolutionError::new("synthetic_denial"))
        }
    }

    struct FixedClock;

    impl AuthorizationClock for FixedClock {
        fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
            Ok(AuthorizationTime::from_unix_seconds(100))
        }
    }

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn runtime(
        policies: impl IntoIterator<Item = DomainTransportPolicy>,
    ) -> Result<ProtectedTransportRuntime, ProtectedTransportError> {
        ProtectedTransportRuntime::new(
            policies,
            Arc::new(UnavailableResolver),
            Arc::new(FixedClock),
        )
    }

    fn request(domain: CommunityId) -> ProtectedOperationRequest {
        let keys = Keys::generate();
        let challenge = "protected-request-test";
        let relay_url = "wss://relay.example";
        let event = EventBuilder::auth(challenge, RelayUrl::parse(relay_url).expect("relay URL"))
            .sign_with_keys(&keys)
            .expect("signed NIP-42 event");
        let proof = VerifiedEvidenceAdapter::new()
            .verify_nip42(
                domain,
                AuthTransport::RelayWebSocket,
                &event,
                challenge,
                relay_url,
                None,
            )
            .expect("verified NIP-42 proof");
        ProtectedOperationRequest::new(
            Arc::new(proof),
            None,
            AuthorizationCapability::CommunityRead,
            Uuid::new_v4(),
            "ws_req",
        )
        .expect("valid request")
    }

    #[test]
    fn request_retains_sealed_proof_and_rejects_surface_substitution() {
        let request = request(domain(1));
        let retained = Arc::clone(request.verified_proof());
        assert!(Arc::ptr_eq(&retained, request.verified_proof()));
        assert_eq!(request.authorization_domain(), domain(1));
        assert_eq!(request.transport(), AuthTransport::RelayWebSocket);

        assert!(matches!(
            ProtectedOperationRequest::new(
                retained,
                None,
                AuthorizationCapability::MediaWrite,
                Uuid::new_v4(),
                "media.upload",
            ),
            Err(ProtectedTransportError::SurfaceCapabilityMismatch)
        ));
    }

    #[test]
    fn finalized_context_cannot_be_replayed_across_correlations() {
        let finalized = Uuid::from_u128(1);
        assert!(exact_correlation_matches(finalized, finalized));
        assert!(!exact_correlation_matches(finalized, Uuid::from_u128(2)));
    }

    #[tokio::test]
    async fn absent_and_off_domains_preserve_legacy_behavior() {
        let configured = domain(1);
        let absent = domain(2);
        let runtime = runtime([DomainTransportPolicy::from_server_configuration(
            configured,
            AuthorizationMode::Off,
        )])
        .expect("runtime");
        assert!(matches!(
            runtime.authorize(&request(configured)).await,
            Ok(ProtectedAuthorization::Legacy)
        ));
        assert!(matches!(
            runtime.authorize(&request(absent)).await,
            Ok(ProtectedAuthorization::Legacy)
        ));
    }

    #[tokio::test]
    async fn enforcing_domain_never_falls_back_on_resolver_failure() {
        let configured = domain(1);
        let runtime = runtime([DomainTransportPolicy::from_server_configuration(
            configured,
            AuthorizationMode::Enforce,
        )])
        .expect("runtime");
        assert!(matches!(
            runtime.authorize(&request(configured)).await,
            Err(ProtectedTransportError::Resolution(_))
        ));
    }

    #[tokio::test]
    async fn verify_only_preserves_legacy_access_without_finalizing() {
        let configured = domain(1);
        let runtime = ProtectedTransportRuntime::new(
            [DomainTransportPolicy::from_server_configuration(
                configured,
                AuthorizationMode::VerifyOnly,
            )],
            Arc::new(PanicResolver),
            Arc::new(FixedClock),
        )
        .expect("runtime");
        assert!(matches!(
            runtime.authorize(&request(configured)).await,
            Ok(ProtectedAuthorization::Legacy)
        ));
    }

    #[tokio::test]
    async fn shadow_preserves_legacy_access_without_finalizing() {
        let configured = domain(1);
        let runtime = ProtectedTransportRuntime::new(
            [DomainTransportPolicy::from_server_configuration(
                configured,
                AuthorizationMode::Shadow,
            )],
            Arc::new(PanicResolver),
            Arc::new(FixedClock),
        )
        .expect("runtime");
        assert!(matches!(
            runtime.authorize(&request(configured)).await,
            Ok(ProtectedAuthorization::Legacy)
        ));
    }

    #[tokio::test]
    async fn observational_denial_is_read_only_and_never_changes_access() {
        let configured = domain(1);
        let resolver = Arc::new(ObservingResolver(AtomicUsize::new(0)));
        let runtime = ProtectedTransportRuntime::new(
            [DomainTransportPolicy::from_server_configuration(
                configured,
                AuthorizationMode::Shadow,
            )],
            resolver.clone(),
            Arc::new(FixedClock),
        )
        .expect("runtime");
        assert!(matches!(
            runtime.authorize(&request(configured)).await,
            Ok(ProtectedAuthorization::Legacy)
        ));
        assert_eq!(resolver.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn client_status_is_absent_in_off_and_shadow_and_fail_closed_when_unavailable() {
        let configured = domain(1);
        for mode in [AuthorizationMode::Off, AuthorizationMode::Shadow] {
            let runtime = runtime([DomainTransportPolicy::from_server_configuration(
                configured, mode,
            )])
            .expect("runtime");
            assert!(runtime
                .present_status(&request(configured))
                .await
                .expect("non-presenting mode")
                .is_none());
        }

        for mode in [AuthorizationMode::VerifyOnly, AuthorizationMode::Enforce] {
            let runtime = runtime([DomainTransportPolicy::from_server_configuration(
                configured, mode,
            )])
            .expect("runtime");
            assert!(matches!(
                runtime.present_status(&request(configured)).await,
                Err(ProtectedTransportError::Resolution(_))
            ));
        }
    }

    #[test]
    fn duplicate_exact_domain_policy_is_rejected() {
        let configured = domain(1);
        assert!(matches!(
            runtime([
                DomainTransportPolicy::from_server_configuration(
                    configured,
                    AuthorizationMode::Off,
                ),
                DomainTransportPolicy::from_server_configuration(
                    configured,
                    AuthorizationMode::Enforce,
                ),
            ]),
            Err(ProtectedTransportError::AmbiguousDomainPolicy)
        ));
    }

    #[test]
    fn atomic_mutations_are_unavailable_only_for_enforcing_authority() {
        assert!(require_atomic_mutation_executor(false).is_ok());
        assert!(matches!(
            require_atomic_mutation_executor(true),
            Err(ProtectedTransportError::AtomicMutationFenceUnavailable)
        ));
        assert!(ProtectedAuthorization::Legacy
            .require_atomic_mutation()
            .is_ok());
    }

    #[test]
    fn unwired_mutations_preserve_legacy_modes_and_deny_enforce() {
        for mode in [
            None,
            Some(AuthorizationMode::Off),
            Some(AuthorizationMode::Shadow),
            Some(AuthorizationMode::VerifyOnly),
        ] {
            assert!(matches!(
                authorize_unwired_for_mode(mode),
                Ok(ProtectedAuthorization::Legacy)
            ));
        }
        assert!(matches!(
            authorize_unwired_for_mode(Some(AuthorizationMode::Enforce)),
            Err(ProtectedTransportError::MissingVerifiedProof)
        ));
    }
}
