use std::fmt;

use thiserror::Error;

/// Stable reason for an allowed authorization decision.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationReason {
    /// Only the configured Nostr proof was required.
    NostrOnly,
    /// An existing direct federated binding matched.
    ///
    /// Enrollment policy governs creation of new bindings. Resolution of an
    /// existing active binding, including future lease checks, is a separate
    /// lifecycle decision.
    ExistingBinding,
    /// A direct binding was created under attested-key enrollment.
    EnrolledAttestedKey,
    /// A direct binding was created under trust-on-first-use enrollment.
    EnrolledTofu,
    /// A verified delegate derived authority from a bound owner.
    DelegatedOwnerBinding,
}

impl fmt::Debug for AuthorizationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuthorizationReason")
            .field(&"[redacted]")
            .finish()
    }
}

impl AuthorizationReason {
    /// Stable audit and metric code for this decision.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NostrOnly => "authorization_allow_001",
            Self::ExistingBinding => "authorization_allow_002",
            Self::EnrolledAttestedKey => "authorization_allow_003",
            Self::EnrolledTofu => "authorization_allow_004",
            Self::DelegatedOwnerBinding => "authorization_allow_005",
        }
    }
}

/// Invalid authorization-context construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthContextError {
    /// Issuer was empty.
    #[error("federated principal issuer must not be empty")]
    EmptyIssuer,
    /// Subject was empty.
    #[error("federated principal subject must not be empty")]
    EmptySubject,
    /// Binding version was zero.
    #[error("identity binding version must be greater than zero")]
    InvalidBindingVersion,
    /// Binding identifier was the nil UUID.
    #[error("identity binding identifier must not be nil")]
    InvalidBindingId,
    /// Binding expiry was not a valid Unix timestamp.
    #[error("identity binding expiry must be greater than zero")]
    InvalidBindingExpiry,
    /// Enrollment-policy identifier was nil.
    #[error("federated enrollment-policy identifier must not be nil")]
    InvalidFederatedPolicyId,
    /// Enrollment-policy epoch was zero.
    #[error("federated enrollment-policy epoch must be greater than zero")]
    InvalidFederatedPolicyEpoch,
    /// Enrollment-policy correlation identifier was nil.
    #[error("federated enrollment-policy correlation must not be nil")]
    InvalidFederatedPolicyCorrelation,
    /// Enrollment-policy effective interval was empty or reversed.
    #[error("federated enrollment-policy effective interval is invalid")]
    InvalidFederatedPolicyInterval,
    /// Assertion expiry was not a valid Unix timestamp.
    #[error("federated assertion expiry must be greater than zero")]
    InvalidAssertionExpiry,
    /// Delegation expiry was not a valid Unix timestamp.
    #[error("delegation expiry must be greater than zero")]
    InvalidDelegationExpiry,
    /// Admission expiry was not a valid Unix timestamp.
    #[error("admission expiry must be greater than zero")]
    InvalidAdmissionExpiry,
    /// Assertion had expired when authorization was evaluated.
    #[error("federated assertion has expired")]
    AssertionExpired,
    /// Binding was no longer authorization-eligible when evaluated.
    #[error("identity binding has expired")]
    BindingExpired,
    /// Enrollment policy was resolved for another authorization decision.
    #[error("federated enrollment policy does not match the authorization decision")]
    FederatedPolicyCorrelationMismatch,
    /// Enrollment policy was used before its effective interval.
    #[error("federated enrollment policy is not yet effective")]
    FederatedPolicyNotYetEffective,
    /// Enrollment policy was used after its effective interval.
    #[error("federated enrollment policy has expired")]
    FederatedPolicyExpired,
    /// Assertion was used before its validated not-before bound.
    #[error("federated assertion is not yet valid")]
    AssertionNotYetValid,
    /// Required key-attestation evidence was absent.
    #[error("verified key attestation is required for this enrollment result")]
    KeyAttestationRequired,
    /// Key-attestation evidence named a different Nostr actor.
    #[error("verified key attestation does not match the authenticated Nostr key")]
    KeyAttestationMismatch,
    /// Owner admission was no longer current when authorization was evaluated.
    #[error("owner admission is no longer current")]
    OwnerAdmissionExpired,
    /// Resolved policy required federated identity, but none was supplied.
    #[error("federated identity is required by the resolved authorization policy")]
    FederatedIdentityRequired,
    /// Federated authorization was supplied for a domain that does not use it.
    #[error("federated authorization does not match the resolved authorization policy")]
    UnexpectedFederatedAuthorization,
    /// Delegation had expired when authorization was evaluated.
    #[error("verified delegation has expired")]
    DelegationExpired,
    /// Owner and delegate were the same key.
    #[error("delegation owner and delegate must be different keys")]
    SelfDelegation,
    /// Direct authorization reason did not match its enrollment policy or source.
    #[error("federated authorization reason does not match binding provenance")]
    InvalidAuthorizationReason,
    /// Binding belonged to a different server-resolved authorization domain.
    #[error("federated binding does not belong to the authorization domain")]
    BindingDomainMismatch,
    /// Nostr proof was verified for a different authorization domain.
    #[error("Nostr proof does not belong to the authorization domain")]
    NostrProofDomainMismatch,
    /// Federated policy was resolved for a different authorization domain.
    #[error("federated policy does not belong to the authorization domain")]
    PolicyDomainMismatch,
    /// Community admission was resolved for a different authorization domain.
    #[error("community admission does not belong to the authorization domain")]
    CommunityAccessDomainMismatch,
    /// Assertion was verified for a different authorization domain.
    #[error("federated assertion does not belong to the authorization domain")]
    AssertionDomainMismatch,
    /// Assertion was verified for a different transport.
    #[error("federated assertion does not match the authorization transport")]
    AssertionTransportMismatch,
    /// Owner admission was resolved for a different authorization domain.
    #[error("owner admission does not belong to the authorization domain")]
    OwnerAdmissionDomainMismatch,
    /// Owner admission represented a different bound principal.
    #[error("owner admission principal does not match the active binding")]
    OwnerAdmissionPrincipalMismatch,
    /// Validated assertion principal did not match the active binding.
    #[error("federated assertion principal does not match the active binding")]
    AssertionPrincipalMismatch,
    /// Proof method was not valid for the transport being authorized.
    #[error("Nostr proof method does not match authorization transport")]
    TransportProofMismatch,
    /// Direct federated authorization was attached to a delegated Nostr actor.
    #[error("direct federated authorization cannot include a delegated Nostr owner")]
    DirectAuthorizationHasOwner,
    /// Direct binding key did not match the authenticated actor.
    #[error("direct federated binding does not match the authenticated Nostr key")]
    DirectBindingKeyMismatch,
    /// Delegated authorization named a different actor.
    #[error("delegated federated authorization does not match the authenticated Nostr key")]
    DelegateKeyMismatch,
    /// Delegated federated authorization lacked verified Nostr delegation.
    #[error("delegated federated authorization requires verified Nostr delegation")]
    DelegationRequired,
    /// Delegated authorization did not match the verified Nostr owner.
    #[error("delegated federated authorization does not match the verified Nostr owner")]
    DelegatedOwnerMismatch,
    /// Delegated owner evidence did not resolve an already-active binding.
    #[error("delegated federated authorization requires an existing active binding")]
    DelegatedBindingNotExistingActive,
}

impl AuthContextError {
    /// Stable audit and metric code for this rejected finalization.
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyIssuer => "federated_principal_empty_issuer",
            Self::EmptySubject => "federated_principal_empty_subject",
            Self::InvalidBindingVersion => "federated_binding_invalid_version",
            Self::InvalidBindingId => "federated_binding_invalid_id",
            Self::InvalidBindingExpiry => "federated_binding_invalid_expiry",
            Self::InvalidFederatedPolicyId => "federated_policy_invalid_id",
            Self::InvalidFederatedPolicyEpoch => "federated_policy_invalid_epoch",
            Self::InvalidFederatedPolicyCorrelation => "federated_policy_invalid_correlation",
            Self::InvalidFederatedPolicyInterval => "federated_policy_invalid_interval",
            Self::InvalidAssertionExpiry => "federated_assertion_invalid_expiry",
            Self::InvalidDelegationExpiry => "delegation_invalid_expiry",
            Self::InvalidAdmissionExpiry => "owner_admission_invalid_expiry",
            Self::AssertionExpired => "federated_assertion_expired",
            Self::BindingExpired => "federated_binding_expired",
            Self::FederatedPolicyCorrelationMismatch => "federated_policy_correlation_mismatch",
            Self::FederatedPolicyNotYetEffective => "federated_policy_not_yet_effective",
            Self::FederatedPolicyExpired => "federated_policy_expired",
            Self::AssertionNotYetValid => "federated_assertion_not_yet_valid",
            Self::KeyAttestationRequired => "federated_key_attestation_required",
            Self::KeyAttestationMismatch => "federated_key_attestation_mismatch",
            Self::OwnerAdmissionExpired => "owner_admission_expired",
            Self::FederatedIdentityRequired => "federated_identity_required",
            Self::UnexpectedFederatedAuthorization => "federated_authorization_unexpected",
            Self::DelegationExpired => "delegation_expired",
            Self::SelfDelegation => "delegation_self_reference",
            Self::InvalidAuthorizationReason => "federated_binding_invalid_reason",
            Self::BindingDomainMismatch => "federated_binding_domain_mismatch",
            Self::NostrProofDomainMismatch => "nostr_proof_domain_mismatch",
            Self::PolicyDomainMismatch => "federated_policy_domain_mismatch",
            Self::CommunityAccessDomainMismatch => "community_access_domain_mismatch",
            Self::AssertionDomainMismatch => "federated_assertion_domain_mismatch",
            Self::AssertionTransportMismatch => "federated_assertion_transport_mismatch",
            Self::OwnerAdmissionDomainMismatch => "owner_admission_domain_mismatch",
            Self::OwnerAdmissionPrincipalMismatch => "owner_admission_principal_mismatch",
            Self::AssertionPrincipalMismatch => "federated_assertion_principal_mismatch",
            Self::TransportProofMismatch => "nostr_transport_proof_mismatch",
            Self::DirectAuthorizationHasOwner => "federated_direct_has_owner",
            Self::DirectBindingKeyMismatch => "federated_direct_key_mismatch",
            Self::DelegateKeyMismatch => "federated_delegate_key_mismatch",
            Self::DelegationRequired => "federated_delegation_required",
            Self::DelegatedOwnerMismatch => "federated_delegated_owner_mismatch",
            Self::DelegatedBindingNotExistingActive => {
                "federated_delegated_binding_not_existing_active"
            }
        }
    }
}
