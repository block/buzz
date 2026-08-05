//! Federated authorization finalization.
//!
//! This is the only crate-owned path from a validated provider capability
//! snapshot to an access lease. Display-only verification is returned as a
//! separate type that cannot be converted into an [`crate::AuthContext`] or
//! consumed by protected-operation lease validation.

use std::fmt;

use buzz_core::CommunityId;
use nostr::PublicKey;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    context::{
        validate_context_evidence, AuthContext, AuthContextError, AuthContextInput, BindingVersion,
        FederatedAuthorization, FederatedIdentityRequirement, ResolvedFederatedPolicy,
        VersionedBindingRef,
    },
    lease::{
        conservative_expiry, AccessLeasePolicy, AuthorizationClockError, AuthorizationLease,
        AuthorizationTime, BindingLeaseBound, LeaseIssueError, LeaseVersion,
        SharedAuthorizationClock, VerificationStatusPolicy,
    },
    provider::{AuthorizationProfileId, CapabilitySnapshot, DecisionSource, PolicyVersion},
};

/// Finalizer using one centrally injected clock for all authorization time.
#[derive(Clone)]
pub struct AuthorizationFinalizer {
    clock: SharedAuthorizationClock,
}

impl AuthorizationFinalizer {
    /// Create a finalizer backed by the supplied central authorization clock.
    pub fn new(clock: SharedAuthorizationClock) -> Self {
        Self { clock }
    }

    /// Read current time from the injected authorization clock.
    pub fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
        self.clock.now()
    }

    /// Finalize enforcing federated authority and issue one bounded lease.
    ///
    /// The provider snapshot must be the current allow decision for the exact
    /// domain, actor, transport, principal, profile, and correlation ID. The
    /// resulting lease expires at the earliest provider/identity bound,
    /// binding freshness bound, or configured application maximum, shortened
    /// by the explicit conservative clock skew.
    #[allow(clippy::too_many_arguments)]
    pub fn finalize_access(
        &self,
        input: AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        snapshot: Box<CapabilitySnapshot>,
        expected_profile: &AuthorizationProfileId,
        binding_bound: BindingLeaseBound,
        lease_policy: AccessLeasePolicy,
        lease_version: LeaseVersion,
    ) -> Result<AuthContext, FinalizationError> {
        let now = self.now()?;
        validate_context_evidence(
            &input,
            &federated_policy,
            &authorization,
            now.unix_seconds(),
        )?;
        let active_binding = validate_provider_evidence(
            &input,
            &federated_policy,
            &authorization,
            &snapshot,
            expected_profile,
            &binding_bound,
            now,
        )?;
        let lease = AuthorizationLease::issue(
            lease_version,
            snapshot.authorization_domain(),
            snapshot.transport(),
            snapshot.actor_pubkey(),
            snapshot.owner_pubkey(),
            active_binding,
            binding_bound,
            snapshot.profile_id().clone(),
            snapshot.policy_version().clone(),
            snapshot.capabilities().clone(),
            snapshot.effective_until(),
            now,
            lease_policy,
            snapshot.correlation_id(),
        )?;
        Ok(AuthContext::finalize_v1_with_lease(
            input,
            federated_policy,
            authorization,
            lease,
            now.unix_seconds(),
        )?)
    }

    /// Finalize display-only verification without issuing access authority.
    ///
    /// This path requires a direct active binding for the event-author key.
    /// The returned type carries no capability set, access lease, membership,
    /// or conversion into an authorized context.
    #[allow(clippy::too_many_arguments)]
    pub fn finalize_verification_only(
        &self,
        input: AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        snapshot: Box<CapabilitySnapshot>,
        expected_profile: &AuthorizationProfileId,
        binding_bound: BindingLeaseBound,
        status_policy: VerificationStatusPolicy,
    ) -> Result<VerificationOnlyDisposition, FinalizationError> {
        let now = self.now()?;
        validate_context_evidence(
            &input,
            &federated_policy,
            &authorization,
            now.unix_seconds(),
        )?;
        let active_binding = validate_provider_evidence(
            &input,
            &federated_policy,
            &authorization,
            &snapshot,
            expected_profile,
            &binding_bound,
            now,
        )?;
        if !matches!(authorization, FederatedAuthorization::Direct { .. }) {
            return Err(FinalizationError::VerificationRequiresDirectBinding);
        }
        let expires_at = conservative_expiry(
            now,
            snapshot.effective_until(),
            binding_bound.valid_until(),
            status_policy.application_limit(),
            status_policy.clock_skew(),
        )?;
        Ok(VerificationOnlyDisposition {
            authorization_domain: snapshot.authorization_domain(),
            actor_pubkey: snapshot.actor_pubkey(),
            binding_id: active_binding.binding_id(),
            binding_version: active_binding.binding_version(),
            profile_id: snapshot.profile_id().clone(),
            policy_version: snapshot.policy_version().clone(),
            correlation_id: snapshot.correlation_id(),
            issued_at: now.unix_seconds(),
            expires_at,
        })
    }
}

impl fmt::Debug for AuthorizationFinalizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationFinalizer")
            .field("clock", &"[injected]")
            .finish()
    }
}

/// Short-lived, display-only proof of a current direct binding.
///
/// This type is deliberately not an authorization context or lease. Protected
/// operations accept [`AuthorizationLease`], so a
/// verification-only result cannot grant access even if a caller retains it.
#[must_use]
#[derive(PartialEq, Eq)]
pub struct VerificationOnlyDisposition {
    authorization_domain: CommunityId,
    actor_pubkey: PublicKey,
    binding_id: Uuid,
    binding_version: BindingVersion,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
    correlation_id: Uuid,
    issued_at: u64,
    expires_at: u64,
}

impl VerificationOnlyDisposition {
    /// Exact authorization domain represented by the display status.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Event-author key whose direct active binding was verified.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Stable direct binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact active binding version.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Server-resolved provider profile.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        &self.profile_id
    }

    /// Current opaque provider policy version.
    pub const fn policy_version(&self) -> &PolicyVersion {
        &self.policy_version
    }

    /// Correlation identifier for the display-only decision.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Central issue time in Unix seconds.
    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }

    /// Conservative display-status expiry in Unix seconds.
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    /// Check display freshness using the supplied central authorization clock.
    ///
    /// This check only controls presentation and is not an access decision.
    pub fn is_current(
        &self,
        clock: &dyn crate::AuthorizationClock,
    ) -> Result<bool, AuthorizationClockError> {
        let now = clock.now()?;
        Ok(now.unix_seconds() >= self.issued_at && now.unix_seconds() < self.expires_at)
    }
}

impl fmt::Debug for VerificationOnlyDisposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerificationOnlyDisposition")
            .field("authorization_domain", &"[redacted]")
            .field("actor_pubkey", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .finish()
    }
}

fn validate_provider_evidence<'a>(
    input: &AuthContextInput,
    federated_policy: &ResolvedFederatedPolicy,
    authorization: &'a FederatedAuthorization,
    snapshot: &CapabilitySnapshot,
    expected_profile: &AuthorizationProfileId,
    binding_bound: &BindingLeaseBound,
    now: AuthorizationTime,
) -> Result<&'a VersionedBindingRef, FinalizationError> {
    if !matches!(
        federated_policy.requirement(),
        FederatedIdentityRequirement::Required(_)
    ) {
        return Err(FinalizationError::FederatedPolicyRequired);
    }
    let domain = input.tenant().community();
    let proof = input.nostr_proof();
    if federated_policy.authorization_domain() != domain
        || !snapshot.is_bound_to_federated_policy(federated_policy)
        || snapshot.authorization_domain() != domain
        || snapshot.transport() != proof.authorized_transport()
        || snapshot.actor_pubkey() != proof.actor_pubkey()
        || snapshot.proof_method() != proof.proof_method()
        || snapshot.correlation_id() != input.correlation_id()
        || snapshot.profile_id() != expected_profile
    {
        return Err(FinalizationError::ProviderEvidenceMismatch);
    }
    if snapshot.issued_at() > now.unix_seconds()
        || snapshot.fresh_until() <= now.unix_seconds()
        || snapshot.effective_until() <= now.unix_seconds()
    {
        return Err(FinalizationError::ProviderEvidenceStale);
    }
    let active_binding = authorization
        .active_binding()
        .ok_or(FinalizationError::FederatedAuthorizationRequired)?;
    if active_binding.binding_id() != binding_bound.binding_id()
        || active_binding.binding_version() != binding_bound.binding_version()
    {
        return Err(FinalizationError::BindingEvidenceMismatch);
    }
    match authorization {
        FederatedAuthorization::NotRequired => {
            return Err(FinalizationError::FederatedAuthorizationRequired);
        }
        FederatedAuthorization::Direct { binding, assertion } => {
            if snapshot.decision_source() != DecisionSource::DirectAssertion
                || snapshot.owner_pubkey().is_some()
                || snapshot.binding_id().is_some()
                || snapshot.binding_version().is_some()
                || snapshot.principal() != binding.principal()
                || snapshot.principal() != assertion.principal()
                || snapshot.effective_until() > assertion.expires_at().unix_seconds()
            {
                return Err(FinalizationError::ProviderEvidenceMismatch);
            }
        }
        FederatedAuthorization::Delegated { owner, admission } => {
            if !proof
                .verified_delegation()
                .is_some_and(|delegation| delegation.capability().is_transport_wide())
            {
                return Err(FinalizationError::UnsupportedDelegationScope);
            }
            if snapshot.decision_source() != DecisionSource::DelegatedOwnerBinding
                || snapshot.owner_pubkey() != Some(owner.bound_pubkey())
                || snapshot.binding_id() != Some(owner.binding_id())
                || snapshot.binding_version() != Some(owner.binding_version())
                || snapshot.principal() != owner.principal()
                || snapshot.principal() != admission.principal()
                || snapshot.fresh_until() != admission.fresh_until().unix_seconds()
            {
                return Err(FinalizationError::ProviderEvidenceMismatch);
            }
            if let Some(delegation_expiry) = proof
                .verified_delegation()
                .and_then(|delegation| delegation.expires_at())
            {
                if snapshot.effective_until() > delegation_expiry.unix_seconds() {
                    return Err(FinalizationError::ProviderEvidenceMismatch);
                }
            }
        }
    }
    Ok(active_binding)
}

/// Fail-closed federated finalization error.
#[derive(Debug, Error)]
pub enum FinalizationError {
    /// The central authorization clock failed.
    #[error(transparent)]
    Clock(#[from] AuthorizationClockError),
    /// Existing context evidence was invalid.
    #[error(transparent)]
    Context(#[from] AuthContextError),
    /// A bounded lease or display expiry could not be issued.
    #[error(transparent)]
    Lease(#[from] LeaseIssueError),
    /// The server-resolved policy did not require federated authorization.
    #[error("federated finalization requires server-resolved federated policy")]
    FederatedPolicyRequired,
    /// No direct or delegated active binding evidence was supplied.
    #[error("federated finalization requires active binding authorization")]
    FederatedAuthorizationRequired,
    /// The provider snapshot did not match the exact finalization evidence.
    #[error("provider decision does not match finalization evidence")]
    ProviderEvidenceMismatch,
    /// The provider snapshot was future-issued, stale, or expired.
    #[error("provider decision is no longer current")]
    ProviderEvidenceStale,
    /// Binding freshness evidence named another binding or version.
    #[error("binding freshness evidence does not match active binding")]
    BindingEvidenceMismatch,
    /// Display verification was attempted for delegated rather than direct authority.
    #[error("verification-only display requires the event author's direct active binding")]
    VerificationRequiresDirectBinding,
    /// Narrower delegation evidence reached the transport-wide finalizer.
    #[error("operation-bound delegation cannot be promoted to transport-wide authority")]
    UnsupportedDelegationScope,
}

impl FinalizationError {
    /// Stable audit and metric code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Clock(_) => "authorization_finalize_001",
            Self::Context(error) => error.code(),
            Self::Lease(error) => error.code(),
            Self::FederatedPolicyRequired => "authorization_finalize_002",
            Self::FederatedAuthorizationRequired => "authorization_finalize_003",
            Self::ProviderEvidenceMismatch => "authorization_finalize_004",
            Self::ProviderEvidenceStale => "authorization_finalize_005",
            Self::BindingEvidenceMismatch => "authorization_finalize_006",
            Self::VerificationRequiresDirectBinding => "authorization_finalize_007",
            Self::UnsupportedDelegationScope => "authorization_finalize_008",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use std::time::Duration;

    use nostr::Keys;

    use super::*;
    use crate::{
        context::{
            AssertionExpiry, AssertionTransport, AuthContextVersion, AuthMethod, AuthTransport,
            AuthoritativeBindingEvidence, AuthoritativeBindingResolution,
            AuthorizedCommunityAccess, BindingSource, DelegationExpiry, EnrollmentMode,
            FederatedPolicyStamp, VerifiedFederatedAssertion, VerifiedKeyAttestation,
            VerifiedNostrProof, VerifiedTransportDelegation,
        },
        lease::{
            ApplicationLeaseLimit, AuthorizationClock, AuthorizationClockSkew,
            AuthorizationLeaseValidator, LeaseRenewalAction, LeaseRenewalLeadTime,
            LeaseUseRequirement,
        },
        provider::{
            resolve_authorization, AuthorizationCapability, AuthorizationOutcome,
            AuthorizationProvider, AuthorizationProviderFuture, AuthorizationRequest,
            CapabilitySet, ProviderAllow, ProviderDecision, ProviderTimeout,
        },
        Scope,
    };

    struct FixedClock(AtomicU64);

    impl FixedClock {
        fn new(now: u64) -> Self {
            Self(AtomicU64::new(now))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl AuthorizationClock for FixedClock {
        fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
            Ok(AuthorizationTime::from_unix_seconds(
                self.0.load(Ordering::SeqCst),
            ))
        }
    }

    impl crate::provider::AuthorizationClock for FixedClock {
        fn now_unix_seconds(&self) -> Option<u64> {
            Some(self.0.load(Ordering::SeqCst))
        }
    }

    struct AllowProvider {
        issued_at: u64,
        fresh_until: u64,
    }

    impl AuthorizationProvider for AllowProvider {
        fn profile_id(&self) -> AuthorizationProfileId {
            profile()
        }

        fn authorize<'a>(
            &'a self,
            request: &'a AuthorizationRequest,
        ) -> AuthorizationProviderFuture<'a> {
            let allow = ProviderAllow::new(
                request.authorization_domain(),
                request.principal().clone(),
                self.profile_id(),
                request.requested_capabilities().clone(),
                PolicyVersion::new("policy.synthetic.example")
                    .expect("synthetic policy version is valid"),
                self.issued_at,
                self.fresh_until,
            )
            .expect("synthetic provider result is valid");
            Box::pin(std::future::ready(ProviderDecision::Allow(allow)))
        }
    }

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0x100))
    }

    fn profile() -> AuthorizationProfileId {
        AuthorizationProfileId::from_server_configuration("profile.synthetic.example")
            .expect("synthetic profile is valid")
    }

    fn required_policy(
        correlation_id: Uuid,
        enrollment_mode: EnrollmentMode,
    ) -> ResolvedFederatedPolicy {
        ResolvedFederatedPolicy::from_authoritative_resolution(
            FederatedPolicyStamp::from_authoritative_state(
                domain(),
                Uuid::from_u128(0x40),
                1,
                correlation_id,
                FederatedIdentityRequirement::Required(enrollment_mode),
                1,
                u64::MAX,
            )
            .expect("synthetic federated policy lineage is valid"),
        )
    }

    struct DirectFixture {
        input: AuthContextInput,
        policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        binding_bound: BindingLeaseBound,
        binding_id: Uuid,
        binding_version: BindingVersion,
        actor_pubkey: PublicKey,
    }

    fn direct_fixture(assertion_expiry: u64, binding_expiry: u64) -> DirectFixture {
        let actor = Keys::generate().public_key();
        let principal =
            crate::FederatedPrincipal::new("https://issuer.synthetic.example", "subject-synthetic")
                .expect("synthetic principal is valid");
        let proof = VerifiedNostrProof::new(
            domain(),
            AuthTransport::RelayWebSocket,
            actor,
            AuthMethod::Nip42,
            None,
        )
        .expect("synthetic proof is valid");
        let assertion = VerifiedFederatedAssertion::new(
            domain(),
            AuthTransport::RelayWebSocket,
            principal.clone(),
            Some(VerifiedKeyAttestation::new(actor)),
            AssertionTransport::TrustedProxy,
            None,
            AssertionExpiry::new(assertion_expiry).expect("synthetic expiry is valid"),
        );
        let binding_id = Uuid::from_u128(0x200);
        let binding_version = BindingVersion::new(7).expect("synthetic version is valid");
        let binding = VersionedBindingRef::new_existing_active_for_test(
            domain(),
            binding_id,
            principal,
            actor,
            binding_version,
            None,
            BindingSource::AttestedKey,
        )
        .expect("synthetic binding is valid");
        let binding_bound = BindingLeaseBound::new(&binding, binding_expiry)
            .expect("synthetic binding bound is valid");
        let tenant =
            buzz_core::tenant::TenantContext::resolved(domain(), "relay.synthetic.example");
        DirectFixture {
            input: AuthContextInput::new(
                tenant,
                Uuid::from_u128(0x300),
                proof,
                AuthorizedCommunityAccess::new(domain(), vec![Scope::MessagesRead], None),
            ),
            policy: required_policy(Uuid::from_u128(0x300), EnrollmentMode::AttestedKey),
            authorization: FederatedAuthorization::Direct { binding, assertion },
            binding_bound,
            binding_id,
            binding_version,
            actor_pubkey: actor,
        }
    }

    async fn direct_snapshot(
        fixture: &DirectFixture,
        clock: &FixedClock,
        provider_fresh_until: u64,
    ) -> Box<CapabilitySnapshot> {
        let FederatedAuthorization::Direct { assertion, .. } = &fixture.authorization else {
            panic!("direct fixture must contain direct authorization");
        };
        let request = AuthorizationRequest::direct(
            fixture.input.nostr_proof(),
            assertion,
            required_policy(fixture.input.correlation_id(), EnrollmentMode::AttestedKey),
            CapabilitySet::single(AuthorizationCapability::CommunityRead),
            fixture.input.correlation_id(),
            1_000,
        )
        .expect("synthetic request is valid");
        let outcome = resolve_authorization(
            &AllowProvider {
                issued_at: 999,
                fresh_until: provider_fresh_until,
            },
            &request,
            clock,
            ProviderTimeout::new(Duration::from_secs(1)).expect("synthetic timeout is valid"),
            Uuid::from_u128(0x600),
        )
        .await;
        match outcome {
            AuthorizationOutcome::Allow(snapshot) => snapshot,
            other => panic!("synthetic provider must allow, got {other:?}"),
        }
    }

    fn access_policy(limit_seconds: u64) -> AccessLeasePolicy {
        AccessLeasePolicy::new(
            ApplicationLeaseLimit::from_seconds(limit_seconds)
                .expect("synthetic application limit is valid"),
            AuthorizationClockSkew::from_seconds(5).expect("synthetic skew is valid"),
        )
    }

    #[tokio::test]
    async fn direct_lease_carries_binding_and_earliest_application_expiry() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context
            .authorization_lease()
            .expect("enforcing context carries a lease");
        assert_eq!(lease.binding_id(), fixture.binding_id);
        assert_eq!(lease.binding_version(), fixture.binding_version);
        assert_eq!(lease.expires_at(), 1_095);
        assert_eq!(
            lease.capabilities().as_slice(),
            &[AuthorizationCapability::CommunityRead]
        );
    }

    #[test]
    fn earliest_expiry_includes_provider_binding_application_and_skew() {
        let now = AuthorizationTime::from_unix_seconds(1_000);
        let limit = ApplicationLeaseLimit::from_seconds(500).expect("synthetic limit is valid");
        let skew = AuthorizationClockSkew::from_seconds(5).expect("synthetic skew is valid");
        assert_eq!(
            conservative_expiry(now, 1_100, 1_200, limit, skew),
            Ok(1_095)
        );
        assert_eq!(
            conservative_expiry(now, 1_300, 1_080, limit, skew),
            Ok(1_075)
        );
        let short_limit =
            ApplicationLeaseLimit::from_seconds(60).expect("synthetic limit is valid");
        assert_eq!(
            conservative_expiry(now, 1_300, 1_200, short_limit, skew),
            Ok(1_055)
        );
    }

    #[tokio::test]
    async fn operation_guard_is_per_capability_and_revalidates_at_commit() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let actor = fixture.actor_pubkey;
        let binding_id = fixture.binding_id;
        let binding_version = fixture.binding_version;
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        let validator = AuthorizationLeaseValidator::new(clock.clone());
        let requirement = LeaseUseRequirement {
            context_version: AuthContextVersion::V1,
            lease_version: LeaseVersion::INITIAL,
            authorization_domain: domain(),
            transport: AuthTransport::RelayWebSocket,
            actor_pubkey: actor,
            binding_id,
            binding_version,
            profile_id: lease.profile_id().clone(),
            policy_version: lease.policy_version().clone(),
            capability: AuthorizationCapability::CommunityRead,
        };
        let guard = context
            .operation_guard(&validator, requirement)
            .expect("exact capability creates an operation guard");
        guard
            .revalidate()
            .expect("guard remains valid before commit boundary");
        clock.set(lease.expires_at());
        assert!(matches!(
            guard.revalidate(),
            Err(crate::LeaseValidationError::Expired)
        ));
    }

    #[tokio::test]
    async fn same_version_different_binding_is_denied() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let actor = fixture.actor_pubkey;
        let binding_version = fixture.binding_version;
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        let requirement = LeaseUseRequirement {
            context_version: AuthContextVersion::V1,
            lease_version: LeaseVersion::INITIAL,
            authorization_domain: domain(),
            transport: AuthTransport::RelayWebSocket,
            actor_pubkey: actor,
            binding_id: Uuid::from_u128(0x201),
            binding_version,
            profile_id: lease.profile_id().clone(),
            policy_version: lease.policy_version().clone(),
            capability: AuthorizationCapability::CommunityRead,
        };

        assert_eq!(
            AuthorizationLeaseValidator::new(clock).authorize(lease, &requirement),
            Err(crate::LeaseValidationError::BindingIdMismatch)
        );
    }

    #[tokio::test]
    async fn same_policy_version_different_profile_is_denied() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let actor = fixture.actor_pubkey;
        let binding_id = fixture.binding_id;
        let binding_version = fixture.binding_version;
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        let requirement = LeaseUseRequirement {
            context_version: AuthContextVersion::V1,
            lease_version: LeaseVersion::INITIAL,
            authorization_domain: domain(),
            transport: AuthTransport::RelayWebSocket,
            actor_pubkey: actor,
            binding_id,
            binding_version,
            profile_id: AuthorizationProfileId::from_server_configuration(
                "other-profile.synthetic.example",
            )
            .expect("synthetic profile is valid"),
            policy_version: lease.policy_version().clone(),
            capability: AuthorizationCapability::CommunityRead,
        };

        assert_eq!(
            AuthorizationLeaseValidator::new(clock).authorize(lease, &requirement),
            Err(crate::LeaseValidationError::AuthorizationProfileMismatch)
        );
    }

    #[tokio::test]
    async fn typed_version_policy_and_capability_mismatches_fail_closed() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let actor = fixture.actor_pubkey;
        let binding_id = fixture.binding_id;
        let binding_version = fixture.binding_version;
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        let validator = AuthorizationLeaseValidator::new(clock);
        let mut requirement = LeaseUseRequirement {
            context_version: AuthContextVersion::V1,
            lease_version: LeaseVersion::new(2).expect("synthetic version is valid"),
            authorization_domain: domain(),
            transport: AuthTransport::RelayWebSocket,
            actor_pubkey: actor,
            binding_id,
            binding_version,
            profile_id: lease.profile_id().clone(),
            policy_version: lease.policy_version().clone(),
            capability: AuthorizationCapability::CommunityRead,
        };
        assert!(matches!(
            validator.authorize(lease, &requirement),
            Err(crate::LeaseValidationError::LeaseVersionMismatch)
        ));
        requirement.lease_version = LeaseVersion::INITIAL;
        requirement.policy_version =
            PolicyVersion::new("changed.synthetic.example").expect("synthetic version is valid");
        assert!(matches!(
            validator.authorize(lease, &requirement),
            Err(crate::LeaseValidationError::PolicyVersionMismatch)
        ));
        requirement.policy_version = lease.policy_version().clone();
        requirement.capability = AuthorizationCapability::CommunityWrite;
        assert!(matches!(
            validator.authorize(lease, &requirement),
            Err(crate::LeaseValidationError::MissingCapability)
        ));
    }

    #[tokio::test]
    async fn verification_only_is_short_lived_and_has_no_access_context() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let status = finalizer
            .finalize_verification_only(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                VerificationStatusPolicy::new(
                    ApplicationLeaseLimit::from_seconds(30)
                        .expect("synthetic status limit is valid"),
                    AuthorizationClockSkew::from_seconds(5).expect("synthetic skew is valid"),
                ),
            )
            .expect("complete current direct evidence produces display status");
        assert_eq!(status.expires_at(), 1_025);
        assert!(status
            .is_current(clock.as_ref())
            .expect("clock is available"));
    }

    #[tokio::test]
    async fn renewal_hooks_expose_renew_and_hard_expiry_boundaries() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let fixture = direct_fixture(1_500, 1_400);
        let snapshot = direct_snapshot(&fixture, clock.as_ref(), 1_300).await;
        let context = finalizer
            .finalize_access(
                fixture.input,
                fixture.policy,
                fixture.authorization,
                snapshot,
                &expected_profile,
                fixture.binding_bound,
                access_policy(100),
                LeaseVersion::INITIAL,
            )
            .expect("complete current evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        let schedule = lease.renewal_schedule(
            LeaseRenewalLeadTime::from_seconds(20).expect("synthetic lead is valid"),
        );
        let validator = AuthorizationLeaseValidator::new(clock.clone());
        assert_eq!(schedule.renew_at(), 1_075);
        assert_eq!(schedule.expires_at(), 1_095);
        assert_eq!(
            validator
                .renewal_action(schedule)
                .expect("clock is available"),
            LeaseRenewalAction::Current
        );
        clock.set(schedule.renew_at());
        assert_eq!(
            validator
                .renewal_action(schedule)
                .expect("clock is available"),
            LeaseRenewalAction::RenewNow
        );
        clock.set(schedule.expires_at());
        assert_eq!(
            validator
                .renewal_action(schedule)
                .expect("clock is available"),
            LeaseRenewalAction::Expired
        );
    }

    #[tokio::test]
    async fn delegated_lease_is_bounded_by_delegation_and_owner_binding() {
        let clock = Arc::new(FixedClock::new(1_000));
        let finalizer = AuthorizationFinalizer::new(clock.clone());
        let expected_profile = profile();
        let owner = Keys::generate().public_key();
        let delegate = Keys::generate().public_key();
        let principal = crate::FederatedPrincipal::new(
            "https://issuer.synthetic.example",
            "owner-subject-synthetic",
        )
        .expect("synthetic principal is valid");
        let proof = VerifiedNostrProof::new(
            domain(),
            AuthTransport::RelayWebSocket,
            delegate,
            AuthMethod::Nip42,
            Some(
                VerifiedTransportDelegation::new_unrestricted(
                    owner,
                    delegate,
                    Some(
                        DelegationExpiry::new(1_050).expect("synthetic delegation expiry is valid"),
                    ),
                )
                .expect("synthetic delegation is valid"),
            ),
        )
        .expect("synthetic proof is valid");
        let binding_id = Uuid::from_u128(0x400);
        let binding_version = BindingVersion::new(9).expect("synthetic version is valid");
        let owner_binding = VersionedBindingRef::new_existing_active_for_test(
            domain(),
            binding_id,
            principal.clone(),
            owner,
            binding_version,
            None,
            BindingSource::Provisioned,
        )
        .expect("synthetic owner binding is valid");
        let owner_resolution = AuthoritativeBindingResolution::existing_active(
            AuthoritativeBindingEvidence::new(
                domain(),
                binding_id,
                principal,
                owner,
                binding_version,
                None,
                BindingSource::Provisioned,
            )
            .expect("synthetic owner binding resolution is valid"),
        );
        let request = AuthorizationRequest::delegated(
            &proof,
            &owner_resolution,
            required_policy(Uuid::from_u128(0x500), EnrollmentMode::Provisioned),
            CapabilitySet::single(AuthorizationCapability::CommunityWrite),
            Uuid::from_u128(0x500),
            1_000,
        )
        .expect("synthetic delegated request is valid");
        let snapshot = match resolve_authorization(
            &AllowProvider {
                issued_at: 999,
                fresh_until: 1_300,
            },
            &request,
            clock.as_ref(),
            ProviderTimeout::new(Duration::from_secs(1)).expect("synthetic timeout is valid"),
            Uuid::from_u128(0x600),
        )
        .await
        {
            AuthorizationOutcome::Allow(snapshot) => snapshot,
            other => panic!("synthetic provider must allow, got {other:?}"),
        };
        assert_eq!(snapshot.effective_until(), 1_050);
        let binding_bound = BindingLeaseBound::new(&owner_binding, 1_400)
            .expect("synthetic binding bound is valid");
        let admission = snapshot
            .verified_owner_admission(&owner_binding)
            .expect("delegated snapshot matches the exact owner binding");
        let input = AuthContextInput::new(
            buzz_core::tenant::TenantContext::resolved(domain(), "relay.synthetic.example"),
            Uuid::from_u128(0x500),
            proof,
            AuthorizedCommunityAccess::new(domain(), vec![Scope::MessagesWrite], None),
        );
        let authorization = FederatedAuthorization::Delegated {
            owner: owner_binding,
            admission,
        };
        let context = finalizer
            .finalize_access(
                input,
                required_policy(Uuid::from_u128(0x500), EnrollmentMode::Provisioned),
                authorization,
                snapshot,
                &expected_profile,
                binding_bound,
                access_policy(500),
                LeaseVersion::INITIAL,
            )
            .expect("delegated evidence finalizes");
        let lease = context.authorization_lease().expect("lease is present");
        assert_eq!(lease.owner_pubkey(), Some(owner));
        assert_eq!(lease.binding_id(), binding_id);
        assert_eq!(lease.binding_version(), binding_version);
        assert_eq!(lease.expires_at(), 1_045);
    }
}
