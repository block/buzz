use std::{fmt, future::Future, pin::Pin};

use buzz_core::CommunityId;
use nostr::PublicKey;
use uuid::Uuid;

use super::{
    AuthContextError, AuthoritativeBindingEvidence, AuthoritativeBindingResolution, BindingExpiry,
    BindingSource, BindingVersion, EnrollmentMode, FederatedIdentityRequirement,
    FederatedPolicyStamp, FederatedPrincipal, ResolvedFederatedPolicy,
};

/// Boxed asynchronous result returned by a federated authority adapter.
pub type AuthorityAdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Failure while invoking or validating a federated authority adapter.
#[derive(PartialEq, Eq)]
pub enum AuthorityAdapterError<E> {
    /// The storage adapter failed before producing authoritative state.
    Adapter(E),
    /// Adapter output violated the authorization contract.
    Contract(AuthContextError),
    /// Current policy no longer matches the atomic binding precondition.
    PolicyChanged,
}

impl<E> fmt::Debug for AuthorityAdapterError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::Adapter(_) => "Adapter",
            Self::Contract(_) => "Contract",
            Self::PolicyChanged => "PolicyChanged",
        };
        formatter
            .debug_struct("AuthorityAdapterError")
            .field("variant", &variant)
            .field("detail", &"[redacted]")
            .finish()
    }
}

impl<E> AuthorityAdapterError<E> {
    /// Wrap a storage-adapter failure.
    pub const fn adapter(error: E) -> Self {
        Self::Adapter(error)
    }

    /// Report that the policy identifier or epoch changed before binding resolution.
    pub const fn policy_changed() -> Self {
        Self::PolicyChanged
    }
}

impl<E> From<AuthContextError> for AuthorityAdapterError<E> {
    fn from(error: AuthContextError) -> Self {
        Self::Contract(error)
    }
}

/// Read-only request for the current enrollment policy of one authorization domain.
#[derive(Clone, PartialEq, Eq)]
pub struct CurrentPolicyRequest {
    authorization_domain: CommunityId,
    correlation_id: Uuid,
    observed_at: u64,
}

impl CurrentPolicyRequest {
    /// Server-resolved authorization domain to read.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Correlation identifier for the decision being assembled.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Trusted server time for the policy read.
    pub const fn observed_at(&self) -> u64 {
        self.observed_at
    }
}

impl fmt::Debug for CurrentPolicyRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentPolicyRequest")
            .field("authorization_domain", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("observed_at", &"[redacted]")
            .finish()
    }
}

/// Crate-owned capability for sealing one current policy read.
///
/// The adapter receives this value from [`resolve_current_federated_policy`];
/// downstream callers cannot construct it. Calling [`Self::resolved`] validates
/// the raw storage fields and returns an opaque policy value.
pub struct CurrentPolicyResolutionSink {
    request: CurrentPolicyRequest,
}

impl CurrentPolicyResolutionSink {
    /// Validate and seal current policy fields read by the adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn resolved(
        self,
        authorization_domain: CommunityId,
        policy_id: Uuid,
        epoch: u64,
        requirement: FederatedIdentityRequirement,
        effective_from: u64,
        effective_until: u64,
    ) -> Result<ResolvedFederatedPolicy, AuthContextError> {
        if authorization_domain != self.request.authorization_domain {
            return Err(AuthContextError::PolicyDomainMismatch);
        }
        let stamp = FederatedPolicyStamp::from_authoritative_state(
            authorization_domain,
            policy_id,
            epoch,
            self.request.correlation_id,
            requirement,
            effective_from,
            effective_until,
        )?;
        if stamp.is_not_yet_effective_at(self.request.observed_at) {
            return Err(AuthContextError::FederatedPolicyNotYetEffective);
        }
        if stamp.is_expired_at(self.request.observed_at) {
            return Err(AuthContextError::FederatedPolicyExpired);
        }
        Ok(ResolvedFederatedPolicy::from_authoritative_resolution(
            stamp,
        ))
    }
}

impl fmt::Debug for CurrentPolicyResolutionSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CurrentPolicyResolutionSink")
            .field(&"[redacted]")
            .finish()
    }
}

/// Atomic binding request tied to an exact current enrollment-policy epoch.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingResolutionRequest {
    authorization_domain: CommunityId,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    policy_id: Uuid,
    policy_epoch: u64,
    policy_requirement: FederatedIdentityRequirement,
    correlation_id: Uuid,
    key_attested: bool,
    effective_from: u64,
    effective_until: u64,
    observed_at: u64,
}

impl BindingResolutionRequest {
    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact issuer-qualified principal being resolved.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Authenticated Nostr key being resolved.
    pub const fn bound_pubkey(&self) -> PublicKey {
        self.bound_pubkey
    }

    /// Stable current enrollment-policy identifier.
    pub const fn policy_id(&self) -> Uuid {
        self.policy_id
    }

    /// Exact policy epoch that must still be current inside the binding transaction.
    pub const fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }

    /// Enrollment requirement at the expected policy epoch.
    pub const fn policy_requirement(&self) -> FederatedIdentityRequirement {
        self.policy_requirement
    }

    /// Correlation identifier for this decision.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Whether verifier-owned assertion evidence attested the exact bound key.
    pub const fn key_attested(&self) -> bool {
        self.key_attested
    }

    /// Inclusive joined assertion, capability, and policy validity bound.
    pub const fn effective_from(&self) -> u64 {
        self.effective_from
    }

    /// Exclusive joined assertion, capability, and policy validity bound.
    pub const fn effective_until(&self) -> u64 {
        self.effective_until
    }

    /// Trusted server time for binding eligibility.
    pub const fn observed_at(&self) -> u64 {
        self.observed_at
    }
}

impl fmt::Debug for BindingResolutionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BindingResolutionRequest")
            .field("authorization_domain", &"[redacted]")
            .field("principal", &"[redacted]")
            .field("bound_pubkey", &"[redacted]")
            .field("policy_id", &"[redacted]")
            .field("policy_epoch", &"[redacted]")
            .field("policy_requirement", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("key_attested", &"[redacted]")
            .field("effective_from", &"[redacted]")
            .field("effective_until", &"[redacted]")
            .field("observed_at", &"[redacted]")
            .finish()
    }
}

#[derive(Clone)]
struct BindingExpectation {
    request: BindingResolutionRequest,
}

impl BindingExpectation {
    #[allow(clippy::too_many_arguments)]
    fn evidence(
        self,
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<AuthoritativeBindingEvidence, AuthContextError> {
        if authorization_domain != self.request.authorization_domain {
            return Err(AuthContextError::BindingDomainMismatch);
        }
        if principal != self.request.principal {
            return Err(AuthContextError::AssertionPrincipalMismatch);
        }
        if bound_pubkey != self.request.bound_pubkey {
            return Err(AuthContextError::DirectBindingKeyMismatch);
        }
        if expires_at.is_some_and(|bound| bound.is_expired_at(self.request.observed_at)) {
            return Err(AuthContextError::BindingExpired);
        }
        AuthoritativeBindingEvidence::new(
            authorization_domain,
            binding_id,
            principal,
            bound_pubkey,
            binding_version,
            expires_at,
            source,
        )
    }
}

/// Crate-owned capability for sealing direct binding state.
///
/// Implementations may call [`Self::existing_active`] after a current active
/// read, or [`Self::atomically_enrolled`] only after enrollment commits in the
/// same transaction that compared the request's policy identifier and epoch.
pub struct DirectBindingResolutionSink {
    expected: BindingExpectation,
}

impl DirectBindingResolutionSink {
    /// Seal an already-active binding returned by authoritative storage.
    #[allow(clippy::too_many_arguments)]
    pub fn existing_active(
        self,
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<AuthoritativeBindingResolution, AuthContextError> {
        self.expected
            .evidence(
                authorization_domain,
                binding_id,
                principal,
                bound_pubkey,
                binding_version,
                expires_at,
                source,
            )
            .map(AuthoritativeBindingResolution::existing_active)
    }

    /// Seal a binding created under the request's atomic policy precondition.
    #[allow(clippy::too_many_arguments)]
    pub fn atomically_enrolled(
        self,
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<AuthoritativeBindingResolution, AuthContextError> {
        match self.expected.request.policy_requirement {
            FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey)
                if !self.expected.request.key_attested =>
            {
                return Err(AuthContextError::KeyAttestationRequired);
            }
            FederatedIdentityRequirement::Required(
                EnrollmentMode::AttestedKey | EnrollmentMode::Tofu,
            ) => {}
            FederatedIdentityRequirement::NotRequired
            | FederatedIdentityRequirement::Required(EnrollmentMode::Provisioned) => {
                return Err(AuthContextError::InvalidAuthorizationReason);
            }
        }
        self.expected
            .evidence(
                authorization_domain,
                binding_id,
                principal,
                bound_pubkey,
                binding_version,
                expires_at,
                source,
            )
            .map(AuthoritativeBindingResolution::atomically_enrolled)
    }
}

impl fmt::Debug for DirectBindingResolutionSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DirectBindingResolutionSink")
            .field(&"[redacted]")
            .finish()
    }
}

/// Crate-owned capability for sealing a read-only existing owner binding.
///
/// This sink intentionally has no enrollment method, so delegated-owner
/// resolution cannot create or relabel a binding.
pub struct ExistingBindingResolutionSink {
    expected: BindingExpectation,
}

impl ExistingBindingResolutionSink {
    /// Seal an already-active owner binding returned by authoritative storage.
    #[allow(clippy::too_many_arguments)]
    pub fn existing_active(
        self,
        authorization_domain: CommunityId,
        binding_id: Uuid,
        principal: FederatedPrincipal,
        bound_pubkey: PublicKey,
        binding_version: BindingVersion,
        expires_at: Option<BindingExpiry>,
        source: BindingSource,
    ) -> Result<AuthoritativeBindingResolution, AuthContextError> {
        self.expected
            .evidence(
                authorization_domain,
                binding_id,
                principal,
                bound_pubkey,
                binding_version,
                expires_at,
                source,
            )
            .map(AuthoritativeBindingResolution::existing_active)
    }
}

impl fmt::Debug for ExistingBindingResolutionSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExistingBindingResolutionSink")
            .field(&"[redacted]")
            .finish()
    }
}

/// Trusted cross-crate adapter for current policy and binding state.
///
/// The server must compose exactly one implementation backed by authoritative
/// storage; request and transport code must never select an implementation.
/// Binding methods must compare `policy_id` and `policy_epoch` and check
/// database time against `[effective_from, effective_until)` after lock
/// acquisition and immediately before commit, inside the same transaction as
/// the active read or enrollment. A mismatch or elapsed interval fails closed
/// without binding mutation; policy mismatch is
/// [`AuthorityAdapterError::PolicyChanged`].
pub trait FederatedAuthorityAdapter: Send + Sync {
    /// Storage-specific failure type.
    type Error;

    /// Read the domain's current enrollment policy and seal it with `sink`.
    fn resolve_current_policy<'a>(
        &'a self,
        request: CurrentPolicyRequest,
        sink: CurrentPolicyResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<ResolvedFederatedPolicy, AuthorityAdapterError<Self::Error>>,
    >;

    /// Resolve or atomically enroll a direct binding under the exact policy precondition.
    fn resolve_direct_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: DirectBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    >;

    /// Resolve an already-active owner binding without enrollment or mutation.
    fn resolve_existing_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: ExistingBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    >;
}

/// Resolve and seal the current policy for one authorization decision.
pub async fn resolve_current_federated_policy<A: FederatedAuthorityAdapter + ?Sized>(
    adapter: &A,
    authorization_domain: CommunityId,
    correlation_id: Uuid,
    now_unix_seconds: u64,
) -> Result<ResolvedFederatedPolicy, AuthorityAdapterError<A::Error>> {
    let request = CurrentPolicyRequest {
        authorization_domain,
        correlation_id,
        observed_at: now_unix_seconds,
    };
    let sink = CurrentPolicyResolutionSink {
        request: request.clone(),
    };
    let policy = adapter.resolve_current_policy(request, sink).await?;
    validate_returned_policy(
        &policy,
        authorization_domain,
        correlation_id,
        now_unix_seconds,
    )?;
    Ok(policy)
}

/// Resolve or atomically enroll a direct binding under an exact current policy.
#[allow(dead_code)]
// Keep the verifier-derived attestation bit and joined interval explicit at
// this sealed boundary so storage adapters cannot infer or widen either fact.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_direct_binding<A: FederatedAuthorityAdapter + ?Sized>(
    adapter: &A,
    policy: &ResolvedFederatedPolicy,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    key_attested: bool,
    effective_from: u64,
    effective_until: u64,
    now_unix_seconds: u64,
) -> Result<AuthoritativeBindingResolution, AuthorityAdapterError<A::Error>> {
    let request = binding_request(
        policy,
        principal,
        bound_pubkey,
        key_attested,
        effective_from,
        effective_until,
        now_unix_seconds,
    )?;
    let sink = DirectBindingResolutionSink {
        expected: BindingExpectation {
            request: request.clone(),
        },
    };
    let resolution = adapter
        .resolve_direct_binding(request.clone(), sink)
        .await?;
    validate_returned_binding(&resolution, &request, false)?;
    Ok(resolution)
}

/// Resolve an already-active owner binding under an exact current policy.
#[allow(dead_code)]
pub(crate) async fn resolve_existing_binding<A: FederatedAuthorityAdapter + ?Sized>(
    adapter: &A,
    policy: &ResolvedFederatedPolicy,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    effective_from: u64,
    effective_until: u64,
    now_unix_seconds: u64,
) -> Result<AuthoritativeBindingResolution, AuthorityAdapterError<A::Error>> {
    let request = binding_request(
        policy,
        principal,
        bound_pubkey,
        false,
        effective_from,
        effective_until,
        now_unix_seconds,
    )?;
    let sink = ExistingBindingResolutionSink {
        expected: BindingExpectation {
            request: request.clone(),
        },
    };
    let resolution = adapter
        .resolve_existing_binding(request.clone(), sink)
        .await?;
    validate_returned_binding(&resolution, &request, true)?;
    Ok(resolution)
}

fn binding_request<E>(
    policy: &ResolvedFederatedPolicy,
    principal: FederatedPrincipal,
    bound_pubkey: PublicKey,
    key_attested: bool,
    effective_from: u64,
    effective_until: u64,
    now_unix_seconds: u64,
) -> Result<BindingResolutionRequest, AuthorityAdapterError<E>> {
    if policy.stamp().is_not_yet_effective_at(now_unix_seconds) {
        return Err(AuthContextError::FederatedPolicyNotYetEffective.into());
    }
    if policy.stamp().is_expired_at(now_unix_seconds) {
        return Err(AuthContextError::FederatedPolicyExpired.into());
    }
    if effective_from < policy.stamp().effective_from()
        || effective_until > policy.stamp().effective_until()
        || effective_from >= effective_until
    {
        return Err(AuthContextError::InvalidFederatedPolicyInterval.into());
    }
    if now_unix_seconds < effective_from {
        return Err(AuthContextError::FederatedPolicyNotYetEffective.into());
    }
    if now_unix_seconds >= effective_until {
        return Err(AuthContextError::FederatedPolicyExpired.into());
    }
    Ok(BindingResolutionRequest {
        authorization_domain: policy.authorization_domain(),
        principal,
        bound_pubkey,
        policy_id: policy.stamp().policy_id(),
        policy_epoch: policy.stamp().epoch(),
        policy_requirement: policy.requirement(),
        correlation_id: policy.stamp().correlation_id(),
        key_attested,
        effective_from,
        effective_until,
        observed_at: now_unix_seconds,
    })
}

fn validate_returned_policy<E>(
    policy: &ResolvedFederatedPolicy,
    authorization_domain: CommunityId,
    correlation_id: Uuid,
    now_unix_seconds: u64,
) -> Result<(), AuthorityAdapterError<E>> {
    if policy.authorization_domain() != authorization_domain {
        return Err(AuthContextError::PolicyDomainMismatch.into());
    }
    if policy.stamp().correlation_id() != correlation_id {
        return Err(AuthContextError::FederatedPolicyCorrelationMismatch.into());
    }
    if policy.stamp().is_not_yet_effective_at(now_unix_seconds) {
        return Err(AuthContextError::FederatedPolicyNotYetEffective.into());
    }
    if policy.stamp().is_expired_at(now_unix_seconds) {
        return Err(AuthContextError::FederatedPolicyExpired.into());
    }
    Ok(())
}

fn validate_returned_binding<E>(
    resolution: &AuthoritativeBindingResolution,
    request: &BindingResolutionRequest,
    require_existing: bool,
) -> Result<(), AuthorityAdapterError<E>> {
    if require_existing && !resolution.is_existing_active() {
        return Err(AuthContextError::DelegatedBindingNotExistingActive.into());
    }
    if resolution.authorization_domain() != request.authorization_domain {
        return Err(AuthContextError::BindingDomainMismatch.into());
    }
    if resolution.principal() != &request.principal {
        return Err(AuthContextError::AssertionPrincipalMismatch.into());
    }
    if resolution.bound_pubkey() != request.bound_pubkey {
        return Err(AuthContextError::DirectBindingKeyMismatch.into());
    }
    if resolution
        .expires_at()
        .is_some_and(|bound| bound.is_expired_at(request.observed_at))
    {
        return Err(AuthContextError::BindingExpired.into());
    }
    Ok(())
}
