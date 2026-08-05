//! Relay adapter for exact-domain authorization policy and finalization.
//!
//! A request never selects its provider profile or activation mode. The relay
//! resolves one immutable policy from the row-zero [`TenantContext`] domain.
//! Missing and duplicate domain configuration fail closed without a global or
//! Nostr-only fallback.

use std::{collections::HashMap, fmt, sync::Arc};

use buzz_auth::{
    resolve_authorization, AccessLeasePolicy, AuthorizationFinalizer, AuthorizationOutcome,
    AuthorizationProfileId, AuthorizationProvider, AuthorizationRequest, BindingLeaseBound,
    CapabilitySet, DecisionSource, EnrollmentMode, FederatedAuthorization, FederatedPrincipal,
    FinalizationError, LeaseVersion, PolicyVersion, ProviderAuthorizationClock,
    ProviderContractError, ProviderTimeout, ResolvedFederatedPolicy, SharedAuthorizationClock,
    VerificationOnlyDisposition, VerificationStatusPolicy, VerifiedFederatedAssertion,
    VerifiedNostrProof, VersionedBindingRef,
};
use buzz_core::{tenant::TenantContext, CommunityId};
use thiserror::Error;
use uuid::Uuid;

/// Server-owned activation mode for one exact authorization domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationMode {
    /// Do not evaluate federated identity or provider policy.
    Off,
    /// Evaluate read-only provider policy without binding or authority changes.
    Shadow,
    /// Produce a short-lived display-only result after full direct finalization.
    VerifyOnly,
    /// Issue bounded access leases after full direct or delegated finalization.
    Enforce,
    /// Keep every protected surface active while denying all protected access.
    DenyProtected,
}

impl AuthorizationMode {
    /// Whether this mode may evaluate the configured admission provider.
    pub const fn evaluates_provider(self) -> bool {
        matches!(self, Self::Shadow | Self::VerifyOnly | Self::Enforce)
    }

    /// Whether this mode keeps the protected-surface inventory authoritative.
    pub const fn protects_surfaces(self) -> bool {
        matches!(self, Self::Enforce | Self::DenyProtected)
    }
}

/// Immutable server configuration for one exact authorization domain.
#[derive(Clone)]
pub struct DomainAuthorizationPolicy {
    authorization_domain: CommunityId,
    profile_id: AuthorizationProfileId,
    provider: Arc<dyn AuthorizationProvider>,
    enrollment_mode: EnrollmentMode,
    mode: AuthorizationMode,
    provider_timeout: ProviderTimeout,
    access_lease_policy: AccessLeasePolicy,
    verification_status_policy: VerificationStatusPolicy,
}

impl DomainAuthorizationPolicy {
    /// Build policy exclusively from trusted server configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn from_server_configuration(
        authorization_domain: CommunityId,
        profile_id: impl Into<String>,
        provider: Arc<dyn AuthorizationProvider>,
        enrollment_mode: EnrollmentMode,
        mode: AuthorizationMode,
        provider_timeout: ProviderTimeout,
        access_lease_policy: AccessLeasePolicy,
        verification_status_policy: VerificationStatusPolicy,
    ) -> Result<Self, ProviderContractError> {
        Ok(Self {
            authorization_domain,
            profile_id: AuthorizationProfileId::from_server_configuration(profile_id)?,
            provider,
            enrollment_mode,
            mode,
            provider_timeout,
            access_lease_policy,
            verification_status_policy,
        })
    }

    /// Exact server-owned authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact server-owned activation mode.
    pub const fn mode(&self) -> AuthorizationMode {
        self.mode
    }

    /// Server-resolved provider profile.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        &self.profile_id
    }

    /// Server-resolved binding enrollment mode.
    pub const fn enrollment_mode(&self) -> EnrollmentMode {
        self.enrollment_mode
    }
}

impl fmt::Debug for DomainAuthorizationPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DomainAuthorizationPolicy")
            .field("authorization_domain", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("provider", &"[configured]")
            .field("enrollment_mode", &"[redacted]")
            .field("mode", &self.mode)
            .field("provider_timeout", &"[redacted]")
            .field("access_lease_policy", &"[redacted]")
            .field("verification_status_policy", &"[redacted]")
            .finish()
    }
}

/// Immutable exact-domain provider selector.
#[derive(Clone)]
pub struct DomainProviderSelector {
    policies: HashMap<CommunityId, DomainAuthorizationPolicy>,
}

impl DomainProviderSelector {
    /// Build an exact-domain selector, rejecting every ambiguous duplicate.
    pub fn new(
        policies: impl IntoIterator<Item = DomainAuthorizationPolicy>,
    ) -> Result<Self, DomainPolicyError> {
        let mut by_domain = HashMap::new();
        for policy in policies {
            let domain = policy.authorization_domain;
            if by_domain.insert(domain, policy).is_some() {
                return Err(DomainPolicyError::AmbiguousDomainPolicy);
            }
        }
        Ok(Self {
            policies: by_domain,
        })
    }

    /// Resolve policy only from the row-zero server tenant.
    ///
    /// No default provider exists. A federated authorization attempt for an
    /// unconfigured domain is denied as missing policy.
    pub fn resolve(
        &self,
        tenant: &TenantContext,
    ) -> Result<&DomainAuthorizationPolicy, DomainPolicyError> {
        self.policies
            .get(&tenant.community())
            .ok_or(DomainPolicyError::MissingDomainPolicy)
    }
}

impl fmt::Debug for DomainProviderSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DomainProviderSelector")
            .field("policies", &"[redacted]")
            .finish()
    }
}

/// Fail-closed exact-domain policy resolution error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DomainPolicyError {
    /// More than one authorization policy named the same exact domain.
    #[error("federated authorization policy is ambiguous for this domain")]
    AmbiguousDomainPolicy,
    /// No federated policy was configured for this exact domain.
    #[error("federated authorization policy is missing for this domain")]
    MissingDomainPolicy,
}

/// Provider-neutral relay finalizer with immutable policy and injected time.
#[derive(Clone)]
pub struct RelayAuthorizationFinalizer {
    selector: DomainProviderSelector,
    finalizer: AuthorizationFinalizer,
    clock: SharedAuthorizationClock,
    runtime_binding: Uuid,
}

struct RelayProviderClock<'a>(&'a dyn buzz_auth::AuthorizationClock);

impl ProviderAuthorizationClock for RelayProviderClock<'_> {
    fn now_unix_seconds(&self) -> Option<u64> {
        self.0.now().ok().map(|value| value.unix_seconds())
    }
}

impl RelayAuthorizationFinalizer {
    /// Build a runtime that shares one central clock across evaluation and finalization.
    pub fn new(selector: DomainProviderSelector, clock: SharedAuthorizationClock) -> Self {
        Self {
            selector,
            finalizer: AuthorizationFinalizer::new(Arc::clone(&clock)),
            clock,
            runtime_binding: Uuid::new_v4(),
        }
    }

    /// Evaluate current direct provider admission for a server-resolved domain.
    ///
    /// Off mode performs no provider call. All other modes preserve provider
    /// deny and unavailable outcomes without falling back to another policy.
    #[allow(clippy::too_many_arguments)]
    pub async fn evaluate_direct(
        &self,
        tenant: &TenantContext,
        proof: &VerifiedNostrProof,
        assertion: &VerifiedFederatedAssertion,
        federated_policy: ResolvedFederatedPolicy,
        requested_capabilities: CapabilitySet,
        correlation_id: Uuid,
    ) -> Result<AuthorizationOutcome, RelayFinalizationError> {
        let policy = self.selector.resolve(tenant)?;
        if !policy.mode.evaluates_provider() {
            return Err(RelayFinalizationError::ModeDoesNotEvaluate);
        }
        let now = self.finalizer.now()?;
        let request = AuthorizationRequest::direct(
            proof,
            assertion,
            federated_policy,
            requested_capabilities,
            correlation_id,
            now.unix_seconds(),
        )?;
        Ok(resolve_authorization(
            policy.provider.as_ref(),
            &request,
            &RelayProviderClock(self.clock.as_ref()),
            policy.provider_timeout,
            self.runtime_binding,
        )
        .await)
    }

    /// Evaluate current delegated-owner provider admission for an exact domain.
    pub async fn evaluate_delegated(
        &self,
        tenant: &TenantContext,
        proof: &VerifiedNostrProof,
        owner: &VersionedBindingRef,
        federated_policy: ResolvedFederatedPolicy,
        requested_capabilities: CapabilitySet,
        correlation_id: Uuid,
    ) -> Result<AuthorizationOutcome, RelayFinalizationError> {
        let policy = self.selector.resolve(tenant)?;
        if !policy.mode.evaluates_provider() {
            return Err(RelayFinalizationError::ModeDoesNotEvaluate);
        }
        let now = self.finalizer.now()?;
        let request = AuthorizationRequest::delegated_from_active_binding(
            proof,
            owner,
            federated_policy,
            requested_capabilities,
            correlation_id,
            now.unix_seconds(),
        )?;
        Ok(resolve_authorization(
            policy.provider.as_ref(),
            &request,
            &RelayProviderClock(self.clock.as_ref()),
            policy.provider_timeout,
            self.runtime_binding,
        )
        .await)
    }

    /// Finalize one validated allow snapshot according to server-owned mode.
    ///
    /// Off and shadow modes cannot finalize a binding, status, or access
    /// context. Verify-only returns a distinct display type; enforce is the
    /// only branch capable of returning an access context with a lease.
    pub fn finalize_allowed(
        &self,
        input: buzz_auth::AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        snapshot: Box<buzz_auth::CapabilitySnapshot>,
        binding_bound: BindingLeaseBound,
        lease_version: LeaseVersion,
    ) -> Result<RuntimeAuthorizationDisposition, RelayFinalizationError> {
        let policy = self.selector.resolve(input.tenant())?;
        match policy.mode {
            AuthorizationMode::Off
            | AuthorizationMode::Shadow
            | AuthorizationMode::DenyProtected => Err(RelayFinalizationError::ModeDoesNotFinalize),
            AuthorizationMode::VerifyOnly => self
                .finalizer
                .finalize_verification_only(
                    input,
                    federated_policy,
                    authorization,
                    snapshot,
                    &policy.profile_id,
                    binding_bound,
                    policy.verification_status_policy,
                )
                .map(RuntimeAuthorizationDisposition::VerificationOnly)
                .map_err(Into::into),
            AuthorizationMode::Enforce => self
                .finalizer
                .finalize_access(
                    input,
                    federated_policy,
                    authorization,
                    snapshot,
                    &policy.profile_id,
                    binding_bound,
                    policy.access_lease_policy,
                    lease_version,
                )
                .map(|context| RuntimeAuthorizationDisposition::Access(Box::new(context)))
                .map_err(Into::into),
        }
    }

    /// Finalize the same current direct evidence into a short-lived,
    /// display-only status. The caller separately proves the presentation
    /// gate; this method cannot issue access or a lease and performs no writes.
    pub fn finalize_client_status(
        &self,
        input: buzz_auth::AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        snapshot: Box<buzz_auth::CapabilitySnapshot>,
        binding_bound: BindingLeaseBound,
    ) -> Result<VerificationOnlyDisposition, RelayFinalizationError> {
        let policy = self.selector.resolve(input.tenant())?;
        if !matches!(
            policy.mode,
            AuthorizationMode::VerifyOnly | AuthorizationMode::Enforce
        ) {
            return Err(RelayFinalizationError::ModeDoesNotFinalize);
        }
        self.finalizer
            .finalize_verification_only(
                input,
                federated_policy,
                authorization,
                snapshot,
                &policy.profile_id,
                binding_bound,
                policy.verification_status_policy,
            )
            .map_err(Into::into)
    }

    /// Finalize direct first-enrollment evidence without creating a binding.
    pub fn finalize_enrollment(
        &self,
        tenant: &TenantContext,
        proof: &VerifiedNostrProof,
        assertion: &VerifiedFederatedAssertion,
        federated_policy: ResolvedFederatedPolicy,
        snapshot: Box<buzz_auth::CapabilitySnapshot>,
        correlation_id: Uuid,
    ) -> Result<EnrollmentDisposition, RelayFinalizationError> {
        let policy = self.selector.resolve(tenant)?;
        if policy.mode != AuthorizationMode::Enforce {
            return Err(RelayFinalizationError::ModeDoesNotFinalize);
        }
        if policy.enrollment_mode != EnrollmentMode::AttestedKey {
            return Err(RelayFinalizationError::EnrollmentModeUnsupported);
        }
        let now = self.finalizer.now()?;
        let key = assertion
            .key_attestation()
            .ok_or(RelayFinalizationError::EnrollmentEvidenceMismatch)?;
        if proof.verified_delegation().is_some()
            || proof.authorization_domain() != tenant.community()
            || federated_policy.authorization_domain() != tenant.community()
            || !snapshot.is_bound_to_federated_policy(&federated_policy)
            || assertion.authorization_domain() != tenant.community()
            || snapshot.authorization_domain() != tenant.community()
            || proof.authorized_transport() != assertion.authorized_transport()
            || snapshot.transport() != proof.authorized_transport()
            || snapshot.actor_pubkey() != proof.actor_pubkey()
            || key.pubkey() != proof.actor_pubkey()
            || snapshot.owner_pubkey().is_some()
            || snapshot.binding_id().is_some()
            || snapshot.binding_version().is_some()
            || snapshot.proof_method() != proof.proof_method()
            || snapshot.principal() != assertion.principal()
            || snapshot.profile_id() != &policy.profile_id
            || snapshot.decision_source() != DecisionSource::DirectAssertion
            || snapshot.correlation_id() != correlation_id
            || !snapshot
                .capabilities()
                .contains(buzz_auth::AuthorizationCapability::InviteClaim)
            || snapshot.issued_at() > now.unix_seconds()
            || snapshot.fresh_until() <= now.unix_seconds()
            || snapshot.effective_until() <= now.unix_seconds()
            || assertion
                .not_before()
                .is_some_and(|bound| bound.is_not_yet_valid_at(now.unix_seconds()))
            || assertion.expires_at().is_expired_at(now.unix_seconds())
        {
            return Err(RelayFinalizationError::EnrollmentEvidenceMismatch);
        }
        let application_until = now
            .unix_seconds()
            .checked_add(policy.access_lease_policy.application_limit().seconds())
            .ok_or(RelayFinalizationError::EnrollmentEvidenceMismatch)?;
        let expires_at = snapshot
            .effective_until()
            .min(assertion.expires_at().unix_seconds())
            .min(application_until)
            .saturating_sub(policy.access_lease_policy.clock_skew().seconds());
        if expires_at <= now.unix_seconds() {
            return Err(RelayFinalizationError::EnrollmentEvidenceMismatch);
        }
        Ok(EnrollmentDisposition {
            authorization_domain: tenant.community(),
            actor_pubkey: proof.actor_pubkey(),
            principal: assertion.principal().clone(),
            profile_id: policy.profile_id.clone(),
            policy_version: snapshot.policy_version().clone(),
            correlation_id,
            expires_at,
        })
    }
}

/// Direct provider decision sealed for atomic first enrollment.
#[must_use]
pub struct EnrollmentDisposition {
    authorization_domain: CommunityId,
    actor_pubkey: nostr::PublicKey,
    principal: FederatedPrincipal,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
    correlation_id: Uuid,
    expires_at: u64,
}

impl EnrollmentDisposition {
    /// Exact server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }
    /// Direct actor whose key is attested by the assertion.
    pub const fn actor_pubkey(&self) -> nostr::PublicKey {
        self.actor_pubkey
    }
    /// Literal issuer-qualified principal staged for enrollment.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }
    /// Server-selected authorization profile.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        &self.profile_id
    }
    /// Provider policy version that authorized the enrollment.
    pub const fn policy_version(&self) -> &PolicyVersion {
        &self.policy_version
    }
    /// Exact decision correlation identifier.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }
    /// Earliest authoritative expiry bound for the enrollment transaction.
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

impl fmt::Debug for EnrollmentDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnrollmentDisposition")
            .field("evidence", &"[redacted]")
            .finish()
    }
}

impl fmt::Debug for RelayAuthorizationFinalizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayAuthorizationFinalizer")
            .field("selector", &self.selector)
            .field("finalizer", &self.finalizer)
            .finish()
    }
}

/// Typed result of server-mode finalization.
#[must_use]
pub enum RuntimeAuthorizationDisposition {
    /// Enforcing authority carrying a bounded access lease.
    Access(Box<buzz_auth::AuthContext>),
    /// Display-only verification carrying no access authority.
    VerificationOnly(VerificationOnlyDisposition),
}

impl fmt::Debug for RuntimeAuthorizationDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access(_) => formatter
                .debug_tuple("Access")
                .field(&"[redacted]")
                .finish(),
            Self::VerificationOnly(_) => formatter
                .debug_tuple("VerificationOnly")
                .field(&"[redacted]")
                .finish(),
        }
    }
}

/// Fail-closed relay finalization adapter error.
#[derive(Debug, Error)]
pub enum RelayFinalizationError {
    /// Exact-domain server policy could not be resolved.
    #[error(transparent)]
    DomainPolicy(#[from] DomainPolicyError),
    /// Central authorization time was unavailable.
    #[error(transparent)]
    Clock(#[from] buzz_auth::AuthorizationClockError),
    /// Provider request evidence was inconsistent.
    #[error(transparent)]
    ProviderContract(#[from] ProviderContractError),
    /// Full provider/binding finalization failed.
    #[error(transparent)]
    Finalization(#[from] FinalizationError),
    /// A non-evaluating mode was asked to evaluate federated policy.
    #[error("server-resolved authorization mode does not evaluate federated policy")]
    ModeDoesNotEvaluate,
    /// A non-finalizing mode was asked to create status or authority.
    #[error("server-resolved authorization mode does not permit finalization")]
    ModeDoesNotFinalize,
    /// First enrollment was configured for a non-attested mode.
    #[error("server-resolved authorization policy does not permit direct enrollment")]
    EnrollmentModeUnsupported,
    /// Direct assertion, provider, proof, or expiry evidence did not match.
    #[error("direct enrollment evidence is inconsistent or stale")]
    EnrollmentEvidenceMismatch,
}

#[cfg(test)]
mod tests {
    use std::{future::ready, time::Duration};

    use buzz_auth::{
        ApplicationLeaseLimit, AuthorizationClockSkew, AuthorizationDenial,
        AuthorizationDenialReason, AuthorizationProviderFuture, ProviderDecision,
    };
    use uuid::Uuid;

    use super::*;

    struct DenyProvider;

    impl AuthorizationProvider for DenyProvider {
        fn profile_id(&self) -> buzz_auth::AuthorizationProfileId {
            buzz_auth::AuthorizationProfileId::from_server_configuration(
                "profile.synthetic-deny.example",
            )
            .expect("synthetic profile is valid")
        }

        fn authorize<'a>(
            &'a self,
            _request: &'a AuthorizationRequest,
        ) -> AuthorizationProviderFuture<'a> {
            Box::pin(ready(ProviderDecision::Deny(AuthorizationDenial::new(
                AuthorizationDenialReason::ProviderDenied,
            ))))
        }
    }

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn policy(domain: CommunityId) -> DomainAuthorizationPolicy {
        let application_limit =
            ApplicationLeaseLimit::from_seconds(300).expect("synthetic limit is valid");
        let skew = AuthorizationClockSkew::from_seconds(5).expect("synthetic skew is valid");
        DomainAuthorizationPolicy::from_server_configuration(
            domain,
            "synthetic-provider.example",
            Arc::new(DenyProvider),
            EnrollmentMode::Provisioned,
            AuthorizationMode::Enforce,
            ProviderTimeout::new(Duration::from_secs(1)).expect("synthetic timeout is valid"),
            AccessLeasePolicy::new(application_limit, skew),
            VerificationStatusPolicy::new(application_limit, skew),
        )
        .expect("synthetic policy is valid")
    }

    #[test]
    fn duplicate_domain_policy_is_rejected_as_ambiguous() {
        let exact_domain = domain(1);
        let result = DomainProviderSelector::new([policy(exact_domain), policy(exact_domain)]);
        assert!(matches!(
            result,
            Err(DomainPolicyError::AmbiguousDomainPolicy)
        ));
    }

    #[test]
    fn missing_domain_has_no_default_provider_fallback() {
        let configured = domain(1);
        let missing = domain(2);
        let selector = DomainProviderSelector::new([policy(configured)])
            .expect("one exact policy is unambiguous");
        let tenant = TenantContext::resolved(missing, "missing.authorization.example");
        assert!(matches!(
            selector.resolve(&tenant),
            Err(DomainPolicyError::MissingDomainPolicy)
        ));
    }

    #[test]
    fn exact_server_tenant_selects_its_policy() {
        let configured = domain(1);
        let selector = DomainProviderSelector::new([policy(configured)])
            .expect("one exact policy is unambiguous");
        let tenant = TenantContext::resolved(configured, "configured.authorization.example");
        let resolved = selector
            .resolve(&tenant)
            .expect("exact domain is configured");
        assert_eq!(resolved.authorization_domain(), configured);
        assert_eq!(resolved.mode(), AuthorizationMode::Enforce);
    }
}
