//! Versioned, transport-neutral authorization context.
//!
//! Authentication adapters produce this context after verifying Nostr proof.
//! Federated identity is optional, but when present it remains distinct from
//! the Nostr authority that signed the request. Raw assertions and mutable
//! display claims never enter this type.

use std::fmt;

use buzz_core::{tenant::TenantContext, CommunityId};
use nostr::PublicKey;
use uuid::Uuid;

use crate::Scope;

pub(crate) mod authority;
mod binding;
mod evidence;
mod reason;

pub use authority::{
    resolve_current_federated_policy, AuthorityAdapterError, AuthorityAdapterFuture,
    BindingResolutionRequest, CurrentPolicyRequest, CurrentPolicyResolutionSink,
    DirectBindingResolutionSink, ExistingBindingResolutionSink, FederatedAuthorityAdapter,
};
pub(crate) use binding::AuthoritativeBindingEvidence;
pub use binding::{
    AuthoritativeBindingResolution, BindingExpiry, BindingSource, BindingVersion, EnrollmentMode,
    FederatedIdentityRequirement, FederatedPolicyStamp, ResolvedFederatedPolicy,
    VersionedBindingRef,
};
pub use evidence::{
    AdmissionExpiry, AssertionExpiry, AssertionNotBefore, AssertionTransport, AuthMethod,
    AuthTransport, AuthorizedCommunityAccess, DelegationCapability, DelegationExpiry,
    FederatedPrincipal, NostrAuthority, VerifiedFederatedAssertion, VerifiedKeyAttestation,
    VerifiedNostrProof, VerifiedOwnerAdmission, VerifiedTransportDelegation,
};
pub use reason::{AuthContextError, AuthorizationReason};

/// Version of the authorization-context contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthContextVersion {
    /// Initial shared authorization-context contract.
    V1,
}

/// Federated authorization attached to a Nostr-authenticated actor.
#[derive(PartialEq, Eq)]
pub enum FederatedAuthorization {
    /// This deployment does not require federated identity.
    ///
    /// An independently verified Nostr owner may still be present in the
    /// [`NostrAuthority`] without acquiring a federated binding.
    NotRequired,
    /// The actor directly owns the active federated binding.
    Direct {
        /// Active identity-to-key binding.
        ///
        /// The binding carries its authoritative lifecycle result, so a caller
        /// cannot relabel a binding enrolled in this decision as pre-existing.
        binding: VersionedBindingRef,
        /// Current assertion accepted by the configured verifier.
        assertion: VerifiedFederatedAssertion,
    },
    /// The actor is delegated by the owner of an active federated binding.
    Delegated {
        /// Owner's active binding.
        owner: VersionedBindingRef,
        /// Current admission resolved for the bound owner.
        admission: VerifiedOwnerAdmission,
    },
}

impl fmt::Debug for FederatedAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("FederatedAuthorization")
            .field(&"[redacted]")
            .finish()
    }
}

/// Authoritative result consumed by the production finalizer.
///
/// Unlike [`FederatedAuthorization`], this input cannot contain a raw
/// [`VersionedBindingRef`] or a caller-selected authorization reason.
#[derive(PartialEq, Eq)]
pub enum AuthoritativeFederatedResolution {
    /// This domain's current policy does not require federated identity.
    NotRequired,
    /// Direct authority backed by an existing or atomically enrolled binding.
    Direct {
        /// Typed authoritative lifecycle result.
        binding: AuthoritativeBindingResolution,
        /// Current verified assertion for the authenticated actor.
        assertion: VerifiedFederatedAssertion,
    },
    /// Delegated authority backed by an already-active owner binding.
    Delegated {
        /// Typed authoritative result for the existing owner binding.
        owner: AuthoritativeBindingResolution,
        /// Current admission resolved for the owner.
        admission: VerifiedOwnerAdmission,
    },
}

impl fmt::Debug for AuthoritativeFederatedResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuthoritativeFederatedResolution")
            .field(&"[redacted]")
            .finish()
    }
}

impl AuthoritativeFederatedResolution {
    #[allow(dead_code)]
    pub(crate) const fn principal(&self) -> Option<&FederatedPrincipal> {
        match self {
            Self::NotRequired => None,
            Self::Direct { assertion, .. } => Some(assertion.principal()),
            Self::Delegated { admission, .. } => Some(admission.principal()),
        }
    }
}

/// Initial shared authorization-context contract.
#[derive(PartialEq, Eq)]
pub struct AuthContextV1 {
    tenant: TenantContext,
    correlation_id: Uuid,
    transport: AuthTransport,
    nostr: NostrAuthority,
    federated_policy: ResolvedFederatedPolicy,
    federated: FederatedAuthorization,
    scopes: Vec<Scope>,
    channel_ids: Option<Vec<Uuid>>,
}

/// Server-verified inputs consumed by the V1 authorization finalizer.
#[derive(PartialEq, Eq)]
pub struct AuthContextInput {
    tenant: TenantContext,
    correlation_id: Uuid,
    nostr_proof: VerifiedNostrProof,
    community_access: AuthorizedCommunityAccess,
}

/// Opaque proof that a validated capability snapshot was consumed.
///
/// Only the crate-owned provider finalizer can construct this value. It keeps
/// the low-level context finalizer public for a stacked contract while making
/// it impossible for downstream code to bypass capability authorization.
pub struct CapabilityFinalizationSeal {
    _private: (),
}

impl CapabilityFinalizationSeal {
    #[allow(dead_code)]
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for CapabilityFinalizationSeal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CapabilityFinalizationSeal")
            .field(&"[redacted]")
            .finish()
    }
}

impl AuthContextInput {
    /// Collect evidence after cryptographic authentication and community
    /// admission have both succeeded.
    pub fn new(
        tenant: TenantContext,
        correlation_id: Uuid,
        nostr_proof: VerifiedNostrProof,
        community_access: AuthorizedCommunityAccess,
    ) -> Self {
        Self {
            tenant,
            correlation_id,
            nostr_proof,
            community_access,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn authorization_domain(&self) -> CommunityId {
        self.tenant.community()
    }

    #[allow(dead_code)]
    pub(crate) const fn nostr_proof_authorization_domain(&self) -> CommunityId {
        self.nostr_proof.authorization_domain()
    }

    #[allow(dead_code)]
    pub(crate) const fn community_access_authorization_domain(&self) -> CommunityId {
        self.community_access.authorization_domain()
    }

    #[allow(dead_code)]
    pub(crate) const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    #[allow(dead_code)]
    pub(crate) const fn transport(&self) -> AuthTransport {
        self.nostr_proof.authorized_transport()
    }

    #[allow(dead_code)]
    pub(crate) const fn proof_method(&self) -> AuthMethod {
        self.nostr_proof.proof_method()
    }

    #[allow(dead_code)]
    pub(crate) const fn actor_pubkey(&self) -> PublicKey {
        self.nostr_proof.actor_pubkey()
    }

    #[allow(dead_code)]
    pub(crate) const fn verified_owner_pubkey(&self) -> Option<PublicKey> {
        match self.nostr_proof.verified_delegation() {
            Some(delegation) => Some(delegation.owner_pubkey()),
            None => None,
        }
    }
}

impl fmt::Debug for AuthContextV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthContextV1")
            .field("authorization_domain", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .field("transport", &self.transport)
            .field("nostr", &self.nostr)
            .field("federated_policy", &self.federated_policy)
            .field("federated", &self.federated)
            .field("scopes", &"[redacted]")
            .field("channel_ids", &"[redacted]")
            .finish()
    }
}

/// Versioned result of successful request or connection authorization.
///
/// This security-boundary type intentionally has no default or deserialization
/// path. Persisted or transported data must be re-verified and finalized rather
/// than decoded directly into an authorized context.
#[derive(PartialEq, Eq)]
pub enum AuthContext {
    /// Initial shared authorization-context contract.
    V1(AuthContextV1),
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V1(context) => formatter.debug_tuple("V1").field(context).finish(),
        }
    }
}

impl AuthContext {
    /// Finalize an immutable V1 context from authoritative binding evidence.
    ///
    /// A crate-owned authority adapter must resolve the enrollment-policy stamp
    /// and binding lifecycle state from current authoritative storage, use the
    /// policy epoch as a conditional precondition for any atomic enrollment,
    /// and pass the resulting opaque lifecycle outcome here. The finalizer
    /// derives the authorization reason; transport code cannot select it or
    /// construct authoritative policy and binding outcomes.
    ///
    /// The opaque seal ensures a production caller first consumed the validated
    /// capability decision supplied by the provider contract.
    pub fn finalize_authoritative_v1(
        _capability: CapabilityFinalizationSeal,
        input: AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        resolution: AuthoritativeFederatedResolution,
        now_unix_seconds: u64,
    ) -> Result<Self, AuthContextError> {
        validate_federated_policy_stamp(&input, &federated_policy, now_unix_seconds)?;
        let authorization = match resolution {
            AuthoritativeFederatedResolution::NotRequired => FederatedAuthorization::NotRequired,
            AuthoritativeFederatedResolution::Direct { binding, assertion } => {
                FederatedAuthorization::Direct {
                    binding: VersionedBindingRef::from_authoritative_resolution(
                        binding,
                        federated_policy.requirement(),
                    )?,
                    assertion,
                }
            }
            AuthoritativeFederatedResolution::Delegated { owner, admission } => {
                FederatedAuthorization::Delegated {
                    owner: VersionedBindingRef::from_existing_authoritative_resolution(owner)?,
                    admission,
                }
            }
        };
        Self::finalize_v1(input, federated_policy, authorization, now_unix_seconds)
    }

    /// Validate all authorization evidence and finalize an immutable V1 context.
    ///
    /// Adapters must preserve this phase order: cryptographic proof and
    /// read-only assertion validation; community admission and capability
    /// resolution; atomic binding/enrollment; then finalization. A denial before
    /// admission must not create or refresh a binding, claim membership, or
    /// publish a public identity assertion.
    ///
    /// `now_unix_seconds` must come from the server clock for the authorization
    /// decision being finalized.
    pub(crate) fn finalize_v1(
        input: AuthContextInput,
        federated_policy: ResolvedFederatedPolicy,
        authorization: FederatedAuthorization,
        now_unix_seconds: u64,
    ) -> Result<Self, AuthContextError> {
        let authorization_domain = input.tenant.community();
        let transport = input.nostr_proof.authorized_transport();
        if input.nostr_proof.authorization_domain() != authorization_domain {
            return Err(AuthContextError::NostrProofDomainMismatch);
        }
        validate_federated_policy_stamp(&input, &federated_policy, now_unix_seconds)?;
        if input.community_access.authorization_domain() != authorization_domain {
            return Err(AuthContextError::CommunityAccessDomainMismatch);
        }
        if !transport_accepts_proof(transport, input.nostr_proof.proof_method()) {
            return Err(AuthContextError::TransportProofMismatch);
        }
        validate_federated_authorization(
            authorization_domain,
            transport,
            &input.nostr_proof,
            &federated_policy,
            &authorization,
            now_unix_seconds,
        )?;
        let nostr = NostrAuthority::new(input.nostr_proof);
        let (scopes, channel_ids) = input.community_access.into_permissions();
        Ok(Self::V1(AuthContextV1 {
            tenant: input.tenant,
            correlation_id: input.correlation_id,
            transport,
            nostr,
            federated_policy,
            federated: authorization,
            scopes,
            channel_ids,
        }))
    }

    /// Contract version represented by this context.
    pub const fn version(&self) -> AuthContextVersion {
        match self {
            Self::V1(_) => AuthContextVersion::V1,
        }
    }

    /// Server-resolved tenant for the request or connection.
    pub const fn tenant(&self) -> &TenantContext {
        match self {
            Self::V1(context) => &context.tenant,
        }
    }

    /// Request or connection correlation identifier.
    pub const fn correlation_id(&self) -> Uuid {
        match self {
            Self::V1(context) => context.correlation_id,
        }
    }

    /// Transport that established this authorization context.
    pub const fn transport(&self) -> AuthTransport {
        match self {
            Self::V1(context) => context.transport,
        }
    }

    /// Verified Nostr authority.
    pub const fn nostr(&self) -> &NostrAuthority {
        match self {
            Self::V1(context) => &context.nostr,
        }
    }

    /// Authenticated Nostr actor.
    pub const fn pubkey(&self) -> PublicKey {
        self.nostr().actor_pubkey()
    }

    /// Proof method used to authenticate the Nostr actor.
    pub const fn auth_method(&self) -> AuthMethod {
        self.nostr().proof_method()
    }

    /// Cryptographically verified owner for a delegated Nostr actor.
    pub const fn agent_owner_pubkey(&self) -> Option<PublicKey> {
        self.nostr().verified_owner_pubkey()
    }

    /// Federated authorization associated with the Nostr actor.
    pub const fn federated_authorization(&self) -> &FederatedAuthorization {
        match self {
            Self::V1(context) => &context.federated,
        }
    }

    /// Federated-identity policy resolved for this authorization decision.
    pub const fn federated_policy(&self) -> &ResolvedFederatedPolicy {
        match self {
            Self::V1(context) => &context.federated_policy,
        }
    }

    /// Stable reason for the successful authorization decision.
    pub const fn authorization_reason(&self) -> AuthorizationReason {
        match self.federated_authorization() {
            FederatedAuthorization::NotRequired => AuthorizationReason::NostrOnly,
            FederatedAuthorization::Direct { binding, .. } => binding.authorization_reason(),
            FederatedAuthorization::Delegated { .. } => AuthorizationReason::DelegatedOwnerBinding,
        }
    }

    /// Permission scopes granted to the context.
    pub fn scopes(&self) -> &[Scope] {
        match self {
            Self::V1(context) => &context.scopes,
        }
    }

    /// Optional channel restriction.
    pub fn channel_ids(&self) -> Option<&[Uuid]> {
        match self {
            Self::V1(context) => context.channel_ids.as_deref(),
        }
    }

    /// Returns `true` if this context includes the given scope.
    pub fn has_scope(&self, scope: &Scope) -> bool {
        self.scopes().contains(scope)
    }
}

fn validate_federated_policy_stamp(
    input: &AuthContextInput,
    federated_policy: &ResolvedFederatedPolicy,
    now_unix_seconds: u64,
) -> Result<(), AuthContextError> {
    if federated_policy.authorization_domain() != input.tenant.community() {
        return Err(AuthContextError::PolicyDomainMismatch);
    }
    if federated_policy.stamp().correlation_id() != input.correlation_id {
        return Err(AuthContextError::FederatedPolicyCorrelationMismatch);
    }
    if federated_policy
        .stamp()
        .is_not_yet_effective_at(now_unix_seconds)
    {
        return Err(AuthContextError::FederatedPolicyNotYetEffective);
    }
    if federated_policy.stamp().is_expired_at(now_unix_seconds) {
        return Err(AuthContextError::FederatedPolicyExpired);
    }
    Ok(())
}

pub(super) const fn transport_accepts_proof(
    transport: AuthTransport,
    proof_method: AuthMethod,
) -> bool {
    match transport {
        AuthTransport::RelayWebSocket | AuthTransport::Audio => {
            matches!(proof_method, AuthMethod::Nip42)
        }
        AuthTransport::HttpBridge | AuthTransport::Git => matches!(proof_method, AuthMethod::Nip98),
        AuthTransport::MediaUpload => {
            matches!(proof_method, AuthMethod::Nip98 | AuthMethod::Blossom)
        }
        AuthTransport::MediaDownload => matches!(proof_method, AuthMethod::Nip98),
    }
}

#[cfg(test)]
mod tests;

fn validate_federated_authorization(
    authorization_domain: CommunityId,
    authorized_transport: AuthTransport,
    nostr_proof: &VerifiedNostrProof,
    federated_policy: &ResolvedFederatedPolicy,
    authorization: &FederatedAuthorization,
    now_unix_seconds: u64,
) -> Result<(), AuthContextError> {
    match (federated_policy.requirement(), authorization) {
        (FederatedIdentityRequirement::Required(_), FederatedAuthorization::NotRequired) => {
            return Err(AuthContextError::FederatedIdentityRequired);
        }
        (FederatedIdentityRequirement::NotRequired, FederatedAuthorization::NotRequired) => {}
        (FederatedIdentityRequirement::NotRequired, _) => {
            return Err(AuthContextError::UnexpectedFederatedAuthorization);
        }
        (FederatedIdentityRequirement::Required(_), _) => {}
    }

    let actor_pubkey = nostr_proof.actor_pubkey();
    let verified_delegation = nostr_proof.verified_delegation();
    match authorization {
        FederatedAuthorization::NotRequired => {}
        FederatedAuthorization::Direct { binding, assertion } => {
            if binding.authorization_domain() != authorization_domain {
                return Err(AuthContextError::BindingDomainMismatch);
            }
            if assertion.authorization_domain() != authorization_domain {
                return Err(AuthContextError::AssertionDomainMismatch);
            }
            if assertion.authorized_transport() != authorized_transport {
                return Err(AuthContextError::AssertionTransportMismatch);
            }
            if assertion.principal() != binding.principal() {
                return Err(AuthContextError::AssertionPrincipalMismatch);
            }
            if verified_delegation.is_some() {
                return Err(AuthContextError::DirectAuthorizationHasOwner);
            }
            if binding.bound_pubkey() != actor_pubkey {
                return Err(AuthContextError::DirectBindingKeyMismatch);
            }
            validate_binding_time(binding, now_unix_seconds)?;
            validate_assertion_time(assertion, now_unix_seconds)?;
            let FederatedIdentityRequirement::Required(enrollment_mode) =
                federated_policy.requirement()
            else {
                return Err(AuthContextError::UnexpectedFederatedAuthorization);
            };
            let reason = binding.authorization_reason();
            if !direct_reason_is_valid(reason, enrollment_mode, binding.source()) {
                return Err(AuthContextError::InvalidAuthorizationReason);
            }
            validate_enrollment_key_attestation(reason, binding.source(), assertion, actor_pubkey)?;
        }
        FederatedAuthorization::Delegated { owner, admission } => {
            if owner.authorization_domain() != authorization_domain {
                return Err(AuthContextError::BindingDomainMismatch);
            }
            if admission.authorization_domain() != authorization_domain {
                return Err(AuthContextError::OwnerAdmissionDomainMismatch);
            }
            if admission.principal() != owner.principal() {
                return Err(AuthContextError::OwnerAdmissionPrincipalMismatch);
            }
            let Some(delegation) = verified_delegation else {
                return Err(AuthContextError::DelegationRequired);
            };
            if delegation.owner_pubkey() != owner.bound_pubkey() {
                return Err(AuthContextError::DelegatedOwnerMismatch);
            }
            validate_binding_time(owner, now_unix_seconds)?;
            if admission.fresh_until().is_expired_at(now_unix_seconds) {
                return Err(AuthContextError::OwnerAdmissionExpired);
            }
            if delegation
                .expires_at()
                .is_some_and(|expiry| expiry.is_expired_at(now_unix_seconds))
            {
                return Err(AuthContextError::DelegationExpired);
            }
        }
    }
    Ok(())
}

fn validate_binding_time(
    binding: &VersionedBindingRef,
    now_unix_seconds: u64,
) -> Result<(), AuthContextError> {
    if binding
        .expires_at()
        .is_some_and(|expiry| expiry.is_expired_at(now_unix_seconds))
    {
        return Err(AuthContextError::BindingExpired);
    }
    Ok(())
}

fn validate_assertion_time(
    assertion: &VerifiedFederatedAssertion,
    now_unix_seconds: u64,
) -> Result<(), AuthContextError> {
    if assertion
        .not_before()
        .is_some_and(|not_before| not_before.is_not_yet_valid_at(now_unix_seconds))
    {
        return Err(AuthContextError::AssertionNotYetValid);
    }
    if assertion.expires_at().is_expired_at(now_unix_seconds) {
        return Err(AuthContextError::AssertionExpired);
    }
    Ok(())
}

const fn direct_reason_is_valid(
    reason: AuthorizationReason,
    enrollment_mode: EnrollmentMode,
    binding_source: BindingSource,
) -> bool {
    match reason {
        AuthorizationReason::ExistingBinding => true,
        AuthorizationReason::EnrolledAttestedKey => {
            matches!(enrollment_mode, EnrollmentMode::AttestedKey)
                && matches!(binding_source, BindingSource::AttestedKey)
        }
        AuthorizationReason::EnrolledTofu => {
            matches!(enrollment_mode, EnrollmentMode::Tofu)
                && matches!(
                    binding_source,
                    // An attested-key binding is stronger provenance than
                    // TOFU. The decision reason records the enrollment policy
                    // while the stored source remains truthful and is never
                    // downgraded to TOFU.
                    BindingSource::Tofu | BindingSource::AttestedKey
                )
        }
        AuthorizationReason::NostrOnly | AuthorizationReason::DelegatedOwnerBinding => false,
    }
}

fn validate_enrollment_key_attestation(
    reason: AuthorizationReason,
    binding_source: BindingSource,
    assertion: &VerifiedFederatedAssertion,
    actor_pubkey: PublicKey,
) -> Result<(), AuthContextError> {
    let requires_attestation = matches!(reason, AuthorizationReason::EnrolledAttestedKey)
        || (matches!(reason, AuthorizationReason::EnrolledTofu)
            && matches!(binding_source, BindingSource::AttestedKey));
    if let Some(attestation) = assertion.key_attestation() {
        if attestation.pubkey() != actor_pubkey {
            return Err(AuthContextError::KeyAttestationMismatch);
        }
        return Ok(());
    }
    if requires_attestation {
        return Err(AuthContextError::KeyAttestationRequired);
    }
    Ok(())
}
