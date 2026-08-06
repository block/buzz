use std::fmt;

use buzz_core::CommunityId;
use nostr::PublicKey;
use uuid::Uuid;

use super::{AuthContextError, AuthorizationReason, FederatedPrincipal};

/// Policy used when no active binding exists for either principal or key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentMode {
    /// First use requires an assertion that attests the proven Nostr key.
    AttestedKey,
    /// Bindings must be created by an out-of-band administrative process.
    Provisioned,
    /// First use may bind the proven key without an asserted key claim.
    Tofu,
}

impl fmt::Debug for EnrollmentMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EnrollmentMode")
            .field(&"[redacted]")
            .finish()
    }
}

/// Federated-identity requirement resolved for one authorization domain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FederatedIdentityRequirement {
    /// Federated identity is not required for this domain.
    NotRequired,
    /// Federated identity is required under the supplied enrollment policy.
    Required(EnrollmentMode),
}

impl fmt::Debug for FederatedIdentityRequirement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FederatedIdentityRequirement")
            .field(&"[redacted]")
            .finish()
    }
}

/// Exact authoritative enrollment-policy lineage for one decision.
///
/// This stamp is not provider capability-policy evidence. It names the
/// server-owned federated enrollment policy that supplied the requirement and
/// its half-open effective interval. The constructor validates shape, while a
/// crate-owned authority adapter remains responsible for sourcing current policy state.
#[derive(Clone, PartialEq, Eq)]
pub struct FederatedPolicyStamp {
    authorization_domain: CommunityId,
    policy_id: Uuid,
    epoch: u64,
    correlation_id: Uuid,
    requirement: FederatedIdentityRequirement,
    effective_from: u64,
    effective_until: u64,
}

impl FederatedPolicyStamp {
    /// Validate lineage read from current authoritative policy state.
    ///
    /// This constructor enforces structural invariants only. Callers must not
    /// source any field from transport input, and the authority adapter must
    /// compare the epoch as an atomic precondition before enrollment.
    pub(crate) fn from_authoritative_state(
        authorization_domain: CommunityId,
        policy_id: Uuid,
        epoch: u64,
        correlation_id: Uuid,
        requirement: FederatedIdentityRequirement,
        effective_from: u64,
        effective_until: u64,
    ) -> Result<Self, AuthContextError> {
        if policy_id.is_nil() {
            return Err(AuthContextError::InvalidFederatedPolicyId);
        }
        if epoch == 0 {
            return Err(AuthContextError::InvalidFederatedPolicyEpoch);
        }
        if correlation_id.is_nil() {
            return Err(AuthContextError::InvalidFederatedPolicyCorrelation);
        }
        if effective_from >= effective_until {
            return Err(AuthContextError::InvalidFederatedPolicyInterval);
        }
        Ok(Self {
            authorization_domain,
            policy_id,
            epoch,
            correlation_id,
            requirement,
            effective_from,
            effective_until,
        })
    }

    /// Authorization domain whose enrollment policy was resolved.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Stable, non-nil identifier of the enrollment-policy namespace.
    pub const fn policy_id(&self) -> Uuid {
        self.policy_id
    }

    /// Positive monotonic epoch within the enrollment-policy namespace.
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Correlation identifier of the decision that resolved this policy.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Federated-identity requirement resolved at this epoch.
    pub const fn requirement(&self) -> FederatedIdentityRequirement {
        self.requirement
    }

    /// Inclusive start of the policy's effective interval.
    pub const fn effective_from(&self) -> u64 {
        self.effective_from
    }

    /// Exclusive end of the policy's effective interval.
    pub const fn effective_until(&self) -> u64 {
        self.effective_until
    }

    /// Whether the policy is not yet effective at trusted server time.
    pub const fn is_not_yet_effective_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds < self.effective_from
    }

    /// Whether the policy is expired at trusted server time.
    pub const fn is_expired_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.effective_until
    }
}

impl fmt::Debug for FederatedPolicyStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FederatedPolicyStamp")
            .field("authorization_domain", &"[redacted]")
            .field("policy_id", &"[redacted]")
            .field("epoch", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("requirement", &"[redacted]")
            .field("effective_from", &"[redacted]")
            .field("effective_until", &"[redacted]")
            .finish()
    }
}

/// Server-resolved federated-identity policy for an authorization decision.
///
/// A policy adapter must resolve the authorization domain's current
/// configuration before producing it; transport values are never authoritative
/// input. The evidence is intentionally move-only and has no default or
/// deserialization path.
#[derive(PartialEq, Eq)]
pub struct ResolvedFederatedPolicy {
    stamp: FederatedPolicyStamp,
}

impl fmt::Debug for ResolvedFederatedPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedFederatedPolicy")
            .field("stamp", &self.stamp)
            .finish()
    }
}

impl ResolvedFederatedPolicy {
    /// Seal structurally validated current policy lineage for finalization.
    pub(crate) const fn from_authoritative_resolution(stamp: FederatedPolicyStamp) -> Self {
        Self { stamp }
    }

    #[cfg(test)]
    pub(crate) fn not_required(authorization_domain: CommunityId) -> Self {
        Self::from_authoritative_resolution(
            FederatedPolicyStamp::from_authoritative_state(
                authorization_domain,
                Uuid::from_u128(40),
                1,
                Uuid::from_u128(2),
                FederatedIdentityRequirement::NotRequired,
                1,
                u64::MAX,
            )
            .expect("synthetic federated policy lineage is valid"),
        )
    }

    #[cfg(test)]
    pub(crate) fn required(
        authorization_domain: CommunityId,
        enrollment_mode: EnrollmentMode,
    ) -> Self {
        Self::from_authoritative_resolution(
            FederatedPolicyStamp::from_authoritative_state(
                authorization_domain,
                Uuid::from_u128(40),
                1,
                Uuid::from_u128(2),
                FederatedIdentityRequirement::Required(enrollment_mode),
                1,
                u64::MAX,
            )
            .expect("synthetic federated policy lineage is valid"),
        )
    }

    /// Authorization domain whose configuration was resolved.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.stamp.authorization_domain()
    }

    /// Resolved federated-identity requirement.
    pub const fn requirement(&self) -> FederatedIdentityRequirement {
        self.stamp.requirement()
    }

    /// Exact authoritative enrollment-policy lineage for this decision.
    pub const fn stamp(&self) -> &FederatedPolicyStamp {
        &self.stamp
    }

    #[allow(dead_code)]
    pub(crate) fn into_stamp(self) -> FederatedPolicyStamp {
        self.stamp
    }
}

/// Provenance recorded when a binding is created.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    /// The identity provider attested the proven Nostr key.
    AttestedKey,
    /// An operator provisioned the binding out of band.
    Provisioned,
    /// The binding was established by trust on first use.
    Tofu,
}

impl fmt::Debug for BindingSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingSource")
            .field(&"[redacted]")
            .finish()
    }
}

/// Monotonically increasing version of an identity-to-key binding.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingVersion(u64);

impl fmt::Debug for BindingVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingVersion")
            .field(&"[redacted]")
            .finish()
    }
}

impl BindingVersion {
    /// Initial version assigned to a newly created binding.
    pub const INITIAL: Self = Self(1);

    /// Build a non-zero binding version.
    pub const fn new(value: u64) -> Result<Self, AuthContextError> {
        if value == 0 {
            return Err(AuthContextError::InvalidBindingVersion);
        }
        Ok(Self(value))
    }

    /// Numeric binding version.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Optional authoritative expiry of a lifecycle-active identity binding.
///
/// Expiry makes the binding ineligible for authorization but does not remove it
/// from lifecycle state or turn it into retirement or revocation evidence.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BindingExpiry(u64);

impl fmt::Debug for BindingExpiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingExpiry")
            .field(&"[redacted]")
            .finish()
    }
}

impl BindingExpiry {
    /// Build a non-zero binding expiry.
    pub const fn new(unix_seconds: u64) -> Result<Self, AuthContextError> {
        if unix_seconds == 0 {
            return Err(AuthContextError::InvalidBindingExpiry);
        }
        Ok(Self(unix_seconds))
    }

    /// Expiry as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    /// Returns `true` when the binding is no longer authorization-eligible.
    pub const fn is_expired_at(self, now_unix_seconds: u64) -> bool {
        self.0 <= now_unix_seconds
    }
}

/// Stable reference to one active identity-to-key binding.
///
/// This reference is identity evidence. It is not an authorization lease and
/// does not by itself provide live-revocation
/// enforcement. Its optional authoritative expiry is a finalization and later
/// lease bound; expiry does not synthesize lifecycle state. An
/// authoritative binding adapter constructs this move-only value after checking
/// active lifecycle state; it has no default or deserialization path.
/// Production construction is available only through the crate-owned
/// authoritative-resolution finalizer. Pending, revoked, newly proposed, and
/// synthetic records must not cross that gate.
#[derive(PartialEq, Eq)]
pub struct VersionedBindingRef {
    authorization_domain: CommunityId,
    binding_id: Uuid,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    binding_version: BindingVersion,
    expires_at: Option<BindingExpiry>,
    source: BindingSource,
    resolution_reason: AuthorizationReason,
}

/// Structurally validated binding fields returned by authoritative state.
///
/// This is not authorization by itself. The crate-owned finalizer additionally
/// requires a typed lifecycle outcome proving that the binding was already
/// active or was atomically enrolled during this decision. It has no default or
/// deserialization path.
#[derive(PartialEq, Eq)]
pub(crate) struct AuthoritativeBindingEvidence {
    authorization_domain: CommunityId,
    binding_id: Uuid,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    binding_version: BindingVersion,
    expires_at: Option<BindingExpiry>,
    source: BindingSource,
}

impl AuthoritativeBindingEvidence {
    /// Validate typed fields read from authoritative binding state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<Self, AuthContextError> {
        if binding_id.is_nil() {
            return Err(AuthContextError::InvalidBindingId);
        }
        Ok(Self {
            authorization_domain,
            binding_id,
            principal,
            bound_pubkey,
            binding_version,
            expires_at,
            source,
        })
    }

    /// Server-resolved authorization domain that owns the binding.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Stable binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Issuer-qualified principal represented by the binding.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Nostr key owned by the binding.
    pub const fn bound_pubkey(&self) -> PublicKey {
        self.bound_pubkey
    }

    /// Current local binding version.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Optional authoritative temporal bound for authorization eligibility.
    pub const fn expires_at(&self) -> Option<BindingExpiry> {
        self.expires_at
    }

    /// Persisted provenance of the active binding.
    pub const fn source(&self) -> BindingSource {
        self.source
    }
}

impl fmt::Debug for AuthoritativeBindingEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthoritativeBindingEvidence")
            .field("authorization_domain", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("principal", &"[redacted]")
            .field("bound_pubkey", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .field("source", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingResolutionOutcome {
    ExistingActive,
    AtomicallyEnrolled,
}

/// Typed authoritative lifecycle result consumed by the crate-owned finalizer.
///
/// It carries no caller-selected authorization reason; the finalizer derives that reason
/// from the lifecycle outcome, persisted provenance, and current enrollment
/// policy.
#[derive(PartialEq, Eq)]
pub struct AuthoritativeBindingResolution {
    evidence: AuthoritativeBindingEvidence,
    outcome: BindingResolutionOutcome,
}

impl AuthoritativeBindingResolution {
    /// Record the authoritative result that the binding already existed.
    pub(crate) fn existing_active(evidence: AuthoritativeBindingEvidence) -> Self {
        Self {
            evidence,
            outcome: BindingResolutionOutcome::ExistingActive,
        }
    }

    /// Record the authoritative result that enrollment committed atomically.
    pub(crate) fn atomically_enrolled(evidence: AuthoritativeBindingEvidence) -> Self {
        Self {
            evidence,
            outcome: BindingResolutionOutcome::AtomicallyEnrolled,
        }
    }

    /// Whether authoritative storage resolved an already-active binding.
    pub const fn is_existing_active(&self) -> bool {
        matches!(self.outcome, BindingResolutionOutcome::ExistingActive)
    }

    /// Server-resolved authorization domain that owns the binding.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.evidence.authorization_domain()
    }

    /// Stable binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.evidence.binding_id()
    }

    /// Issuer-qualified principal represented by the binding.
    pub const fn principal(&self) -> &FederatedPrincipal {
        self.evidence.principal()
    }

    /// Nostr key owned by the binding.
    pub const fn bound_pubkey(&self) -> PublicKey {
        self.evidence.bound_pubkey()
    }

    /// Current local binding version.
    pub const fn binding_version(&self) -> BindingVersion {
        self.evidence.binding_version()
    }

    /// Optional authoritative temporal bound for authorization eligibility.
    pub const fn expires_at(&self) -> Option<BindingExpiry> {
        self.evidence.expires_at()
    }

    /// Persisted provenance of the active binding.
    pub const fn source(&self) -> BindingSource {
        self.evidence.source()
    }
}

impl fmt::Debug for AuthoritativeBindingResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuthoritativeBindingResolution")
            .field(&"[redacted]")
            .finish()
    }
}

impl fmt::Debug for VersionedBindingRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionedBindingRef")
            .field("authorization_domain", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("principal", &self.principal)
            .field("bound_pubkey", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .field("source", &"[redacted]")
            .field("resolution_reason", &"[redacted]")
            .finish()
    }
}

impl VersionedBindingRef {
    pub(super) fn from_authoritative_resolution(
        resolution: AuthoritativeBindingResolution,
        requirement: FederatedIdentityRequirement,
    ) -> Result<Self, AuthContextError> {
        let reason = match resolution.outcome {
            BindingResolutionOutcome::ExistingActive => AuthorizationReason::ExistingBinding,
            BindingResolutionOutcome::AtomicallyEnrolled => {
                match (requirement, resolution.evidence.source) {
                    (
                        FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey),
                        BindingSource::AttestedKey,
                    ) => AuthorizationReason::EnrolledAttestedKey,
                    (
                        FederatedIdentityRequirement::Required(EnrollmentMode::Tofu),
                        BindingSource::Tofu | BindingSource::AttestedKey,
                    ) => AuthorizationReason::EnrolledTofu,
                    _ => return Err(AuthContextError::InvalidAuthorizationReason),
                }
            }
        };
        Ok(Self::from_authoritative_evidence(
            resolution.evidence,
            reason,
        ))
    }

    pub(super) fn from_existing_authoritative_resolution(
        resolution: AuthoritativeBindingResolution,
    ) -> Result<Self, AuthContextError> {
        if !resolution.is_existing_active() {
            return Err(AuthContextError::DelegatedBindingNotExistingActive);
        }
        Ok(Self::from_authoritative_evidence(
            resolution.evidence,
            AuthorizationReason::ExistingBinding,
        ))
    }

    fn from_authoritative_evidence(
        evidence: AuthoritativeBindingEvidence,
        resolution_reason: AuthorizationReason,
    ) -> Self {
        Self {
            authorization_domain: evidence.authorization_domain,
            binding_id: evidence.binding_id,
            principal: evidence.principal,
            bound_pubkey: evidence.bound_pubkey,
            binding_version: evidence.binding_version,
            expires_at: evidence.expires_at,
            source: evidence.source,
            resolution_reason,
        }
    }

    /// Build a reference to a binding authoritatively resolved as already active.
    #[cfg(test)]
    pub(crate) fn new_existing_active_for_test(
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<Self, AuthContextError> {
        if binding_id.is_nil() {
            return Err(AuthContextError::InvalidBindingId);
        }
        Ok(Self {
            authorization_domain,
            binding_id,
            principal,
            bound_pubkey,
            binding_version,
            expires_at,
            source,
            resolution_reason: AuthorizationReason::ExistingBinding,
        })
    }

    /// Build a reference to a binding atomically enrolled in this decision.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_enrolled_active_for_test(
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
        reason: AuthorizationReason,
    ) -> Result<Self, AuthContextError> {
        if binding_id.is_nil() {
            return Err(AuthContextError::InvalidBindingId);
        }
        if !matches!(
            reason,
            AuthorizationReason::EnrolledAttestedKey | AuthorizationReason::EnrolledTofu
        ) {
            return Err(AuthContextError::InvalidAuthorizationReason);
        }
        Ok(Self {
            authorization_domain,
            binding_id,
            principal,
            bound_pubkey,
            binding_version,
            expires_at,
            source,
            resolution_reason: reason,
        })
    }

    /// Server-resolved authorization domain that owns the binding.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Stable binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Issuer-qualified principal represented by the binding.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Nostr key owned by the binding.
    pub const fn bound_pubkey(&self) -> PublicKey {
        self.bound_pubkey
    }

    /// Current binding version.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Optional authoritative temporal bound for authorization eligibility.
    pub const fn expires_at(&self) -> Option<BindingExpiry> {
        self.expires_at
    }

    /// Provenance of the active binding.
    pub const fn source(&self) -> BindingSource {
        self.source
    }

    /// Stable reason proven by the authoritative binding lifecycle result.
    pub(super) const fn authorization_reason(&self) -> AuthorizationReason {
        self.resolution_reason
    }
}
