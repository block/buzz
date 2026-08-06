use std::fmt;

use buzz_core::CommunityId;
use nostr::PublicKey;
use uuid::Uuid;

use crate::Scope;

#[cfg(test)]
use super::transport_accepts_proof;
use super::AuthContextError;

/// Cryptographic proof used to authenticate the Nostr actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// NIP-42 challenge/response over WebSocket.
    Nip42,
    /// NIP-98 signed HTTP request.
    Nip98,
    /// Blossom upload authorization.
    Blossom,
}

/// Entry point that produced the authorization context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthTransport {
    /// Relay WebSocket protocol.
    RelayWebSocket,
    /// HTTP relay bridge.
    HttpBridge,
    /// Git-over-HTTP endpoint.
    Git,
    /// Media upload endpoint.
    MediaUpload,
    /// Authenticated media download endpoint, including `GET` and `HEAD`.
    MediaDownload,
    /// Huddle audio WebSocket.
    Audio,
}

/// Transport profile used to deliver a federated assertion.
///
/// This records how the assertion reached its verifier. It is intentionally
/// independent of [`AuthTransport`]; each authentication adapter must verify
/// the delivery profile before constructing authorization evidence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AssertionTransport {
    /// A trusted proxy stripped inbound copies and injected the assertion.
    TrustedProxy,
    /// The client attached the assertion to the authorized request.
    ClientAttached,
}

impl fmt::Debug for AssertionTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AssertionTransport")
            .field(&"[redacted]")
            .finish()
    }
}

/// Expiry of a validated federated assertion, expressed as Unix seconds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssertionExpiry(u64);

impl fmt::Debug for AssertionExpiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AssertionExpiry")
            .field(&"[redacted]")
            .finish()
    }
}

impl AssertionExpiry {
    /// Build a non-zero assertion expiry.
    pub const fn new(unix_seconds: u64) -> Result<Self, AuthContextError> {
        if unix_seconds == 0 {
            return Err(AuthContextError::InvalidAssertionExpiry);
        }
        Ok(Self(unix_seconds))
    }

    /// Expiry as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    /// Returns `true` when the assertion is no longer valid at `now`.
    pub const fn is_expired_at(self, now_unix_seconds: u64) -> bool {
        self.0 <= now_unix_seconds
    }
}

/// Earliest valid time from a validated federated assertion, as Unix seconds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssertionNotBefore(u64);

impl fmt::Debug for AssertionNotBefore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AssertionNotBefore")
            .field(&"[redacted]")
            .finish()
    }
}

impl AssertionNotBefore {
    /// Preserve a validated `nbf` timestamp for finalization checks.
    pub const fn new(unix_seconds: u64) -> Self {
        Self(unix_seconds)
    }

    /// Earliest valid time as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    /// Returns `true` while the assertion is not yet valid at `now`.
    pub const fn is_not_yet_valid_at(self, now_unix_seconds: u64) -> bool {
        self.0 > now_unix_seconds
    }
}

/// Expiry imposed by a separately verified delegation proof.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DelegationExpiry(u64);

impl fmt::Debug for DelegationExpiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DelegationExpiry")
            .field(&"[redacted]")
            .finish()
    }
}

impl DelegationExpiry {
    /// Build a non-zero delegation expiry.
    pub const fn new(unix_seconds: u64) -> Result<Self, AuthContextError> {
        if unix_seconds == 0 {
            return Err(AuthContextError::InvalidDelegationExpiry);
        }
        Ok(Self(unix_seconds))
    }

    /// Expiry as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    /// Returns `true` when the delegation is no longer valid at `now`.
    pub const fn is_expired_at(self, now_unix_seconds: u64) -> bool {
        self.0 <= now_unix_seconds
    }
}

/// Freshness bound imposed by current community or enterprise admission.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AdmissionExpiry(u64);

impl fmt::Debug for AdmissionExpiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AdmissionExpiry")
            .field(&"[redacted]")
            .finish()
    }
}

impl AdmissionExpiry {
    /// Build a non-zero admission freshness bound.
    pub const fn new(unix_seconds: u64) -> Result<Self, AuthContextError> {
        if unix_seconds == 0 {
            return Err(AuthContextError::InvalidAdmissionExpiry);
        }
        Ok(Self(unix_seconds))
    }

    /// Freshness bound as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }

    /// Returns `true` when admission is no longer current at `now`.
    pub const fn is_expired_at(self, now_unix_seconds: u64) -> bool {
        self.0 <= now_unix_seconds
    }
}

/// Server-verified Nostr authority for a request or connection.
#[derive(PartialEq, Eq)]
pub struct NostrAuthority {
    actor_pubkey: PublicKey,
    proof_method: AuthMethod,
    verified_delegation: Option<VerifiedTransportDelegation>,
}

impl fmt::Debug for NostrAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NostrAuthority")
            .field("actor_pubkey", &"[redacted]")
            .field("proof_method", &self.proof_method)
            .field("verified_delegation", &"[redacted]")
            .finish()
    }
}

impl NostrAuthority {
    pub(super) fn new(proof: VerifiedNostrProof) -> Self {
        Self {
            actor_pubkey: proof.actor_pubkey,
            proof_method: proof.proof_method,
            verified_delegation: proof.verified_delegation,
        }
    }

    /// Authenticated Nostr actor.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Proof method used to authenticate the actor.
    pub const fn proof_method(&self) -> AuthMethod {
        self.proof_method
    }

    /// Cryptographically verified owner for a delegated Nostr actor.
    pub const fn verified_owner_pubkey(&self) -> Option<PublicKey> {
        match &self.verified_delegation {
            Some(delegation) => Some(delegation.owner_pubkey()),
            None => None,
        }
    }

    /// Cryptographically verified owner-to-actor delegation, when present.
    pub const fn verified_delegation(&self) -> Option<&VerifiedTransportDelegation> {
        self.verified_delegation.as_ref()
    }
}

/// Stable identity-provider principal.
///
/// Equality uses the exact validated issuer and subject bytes. Construction
/// does not trim, case-fold, parse, or otherwise normalize either value; the
/// assertion verifier owns canonical validation before crossing this boundary.
/// Neither value is suitable for public events or general-purpose logs.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FederatedPrincipal {
    issuer: String,
    subject: String,
}

impl FederatedPrincipal {
    /// Build an issuer-qualified principal from validated assertion claims.
    pub fn new(
        issuer: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<Self, AuthContextError> {
        let issuer = issuer.into();
        let subject = subject.into();
        if issuer.is_empty() {
            return Err(AuthContextError::EmptyIssuer);
        }
        if subject.is_empty() {
            return Err(AuthContextError::EmptySubject);
        }
        Ok(Self { issuer, subject })
    }

    /// Validated identity-provider issuer.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Stable, non-reassignable subject within the issuer namespace.
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

impl fmt::Debug for FederatedPrincipal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FederatedPrincipal")
            .field("issuer", &"[redacted]")
            .field("subject", &"[redacted]")
            .finish()
    }
}

/// Identity-provider attestation of the Nostr key proven by this request.
///
/// The assertion verifier may construct this evidence only after requiring the
/// configured key claim, parsing it successfully, and proving that it names
/// the exact Nostr key. Absence or mismatch must fail rather than silently
/// falling back to another enrollment mode. The evidence is intentionally
/// move-only and has no default or deserialization path.
#[derive(PartialEq, Eq)]
pub struct VerifiedKeyAttestation {
    pubkey: PublicKey,
}

impl VerifiedKeyAttestation {
    #[cfg(test)]
    pub(crate) const fn new(pubkey: PublicKey) -> Self {
        Self { pubkey }
    }

    /// Nostr key named by the verified assertion claim.
    pub const fn pubkey(&self) -> PublicKey {
        self.pubkey
    }
}

impl fmt::Debug for VerifiedKeyAttestation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedKeyAttestation")
            .field("pubkey", &"[redacted]")
            .finish()
    }
}

/// Federated assertion accepted by the configured assertion verifier.
///
/// The verifier must enforce an allowed algorithm and key, require correctly
/// typed `exp`, `iss`, and `aud` claims, validate the issuer and audience, and
/// reject a malformed `nbf` before constructing this evidence. Finalization
/// independently enforces the preserved `nbf` against server time. Raw
/// assertion claims cannot construct it from outside `buzz-auth`. Private
/// identity attributes and public display labels are deliberately excluded.
/// The evidence is intentionally move-only and has no default or
/// deserialization path.
#[derive(PartialEq, Eq)]
pub struct VerifiedFederatedAssertion {
    authorization_domain: CommunityId,
    authorized_transport: AuthTransport,
    principal: FederatedPrincipal,
    key_attestation: Option<VerifiedKeyAttestation>,
    transport: AssertionTransport,
    not_before: Option<AssertionNotBefore>,
    expires_at: AssertionExpiry,
}

impl VerifiedFederatedAssertion {
    #[cfg(test)]
    pub(crate) const fn new(
        authorization_domain: CommunityId,
        authorized_transport: AuthTransport,
        principal: FederatedPrincipal,
        key_attestation: Option<VerifiedKeyAttestation>,
        transport: AssertionTransport,
        not_before: Option<AssertionNotBefore>,
        expires_at: AssertionExpiry,
    ) -> Self {
        Self {
            authorization_domain,
            authorized_transport,
            principal,
            key_attestation,
            transport,
            not_before,
            expires_at,
        }
    }

    /// Authorization domain for which the assertion was verified.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Transport whose request or connection the verifier authorized.
    pub const fn authorized_transport(&self) -> AuthTransport {
        self.authorized_transport
    }

    /// Issuer-qualified principal from the verified assertion.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Nostr key attested by the verified assertion claim, when present.
    pub const fn key_attestation(&self) -> Option<&VerifiedKeyAttestation> {
        self.key_attestation.as_ref()
    }

    /// Verified assertion delivery profile.
    pub const fn transport(&self) -> AssertionTransport {
        self.transport
    }

    /// Earliest valid time preserved from the verified assertion, when present.
    pub const fn not_before(&self) -> Option<AssertionNotBefore> {
        self.not_before
    }

    /// Upper time bound carried by the verified assertion.
    pub const fn expires_at(&self) -> AssertionExpiry {
        self.expires_at
    }
}

impl fmt::Debug for VerifiedFederatedAssertion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedFederatedAssertion")
            .field("authorization_domain", &"[redacted]")
            .field("authorized_transport", &self.authorized_transport)
            .field("principal", &self.principal)
            .field("key_attestation", &"[redacted]")
            .field("transport", &"[redacted]")
            .field("not_before", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .finish()
    }
}

/// Current admission for the owner of a delegated Nostr actor.
///
/// This evidence is independent of a federated assertion: a delegated request
/// need not possess the owner's token. A provider adapter will construct it
/// only after confirming that the bound owner is currently admitted in the
/// same authorization domain. Construction remains crate-private so only the
/// validated provider finalizer can turn a current capability decision into
/// this move-only evidence.
#[derive(PartialEq, Eq)]
pub struct VerifiedOwnerAdmission {
    authorization_domain: CommunityId,
    principal: FederatedPrincipal,
    fresh_until: AdmissionExpiry,
}

impl VerifiedOwnerAdmission {
    // Consumed by the provider finalizer in the stacked capability contract.
    #[allow(dead_code)]
    pub(crate) const fn new(
        authorization_domain: CommunityId,
        principal: FederatedPrincipal,
        fresh_until: AdmissionExpiry,
    ) -> Self {
        Self {
            authorization_domain,
            principal,
            fresh_until,
        }
    }

    /// Authorization domain for which owner admission was resolved.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Issuer-qualified owner admitted by the provider.
    pub const fn principal(&self) -> &FederatedPrincipal {
        &self.principal
    }

    /// Upper bound after which provider admission must be resolved again.
    pub const fn fresh_until(&self) -> AdmissionExpiry {
        self.fresh_until
    }
}

impl fmt::Debug for VerifiedOwnerAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedOwnerAdmission")
            .field("authorization_domain", &"[redacted]")
            .field("principal", &self.principal)
            .field("fresh_until", &"[redacted]")
            .finish()
    }
}

/// Capability represented by a verified owner-to-delegate proof.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DelegationCapability {
    /// Authorizes the complete request or connection represented by the context.
    TransportWide,
}

impl fmt::Debug for DelegationCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DelegationCapability")
            .field(&"[redacted]")
            .finish()
    }
}

/// Transport-wide delegation from a bound owner to the authenticated key.
///
/// A verifier may construct this only after proving the capability authorizes
/// the complete target request or connection. Time-only constraints may be
/// reduced to [`DelegationExpiry`], but operation-, event-kind-, or
/// request-specific constraints must not be discarded or promoted into this
/// transport-wide evidence. This move-only evidence has no default or
/// deserialization path.
#[derive(PartialEq, Eq)]
pub struct VerifiedTransportDelegation {
    owner_pubkey: PublicKey,
    delegate_pubkey: PublicKey,
    capability: DelegationCapability,
    expires_at: Option<DelegationExpiry>,
}

impl fmt::Debug for VerifiedTransportDelegation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedTransportDelegation")
            .field("owner_pubkey", &"[redacted]")
            .field("delegate_pubkey", &"[redacted]")
            .field("capability", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .finish()
    }
}

impl VerifiedTransportDelegation {
    /// Build transport-wide evidence after validating both keys and confirming
    /// that no narrower capability constraint is being discarded.
    #[cfg(test)]
    pub(crate) fn new_unrestricted(
        owner_pubkey: PublicKey,
        delegate_pubkey: PublicKey,
        expires_at: Option<DelegationExpiry>,
    ) -> Result<Self, AuthContextError> {
        if owner_pubkey == delegate_pubkey {
            return Err(AuthContextError::SelfDelegation);
        }
        Ok(Self {
            owner_pubkey,
            delegate_pubkey,
            capability: DelegationCapability::TransportWide,
            expires_at,
        })
    }

    /// Bound owner that authorized the delegate.
    pub const fn owner_pubkey(&self) -> PublicKey {
        self.owner_pubkey
    }

    /// Authenticated delegate key.
    pub const fn delegate_pubkey(&self) -> PublicKey {
        self.delegate_pubkey
    }

    /// Verified capability scope.
    pub const fn capability(&self) -> DelegationCapability {
        self.capability
    }

    /// Optional upper bound imposed by the delegation proof.
    pub const fn expires_at(&self) -> Option<DelegationExpiry> {
        self.expires_at
    }
}

/// Cryptographically verified Nostr proof for one request or connection.
///
/// Transport verifiers inside `buzz-auth` produce this evidence after checking
/// the signature and transport-specific binding. Raw request keys and claimed
/// proof methods cannot construct it in relay call sites. Conditional
/// delegation may be attached only when it has been fully evaluated for the
/// target operation or safely reduced to transport-wide evidence. The evidence
/// is intentionally move-only and has no default or deserialization path.
#[derive(PartialEq, Eq)]
pub struct VerifiedNostrProof {
    authorization_domain: CommunityId,
    authorized_transport: AuthTransport,
    actor_pubkey: PublicKey,
    proof_method: AuthMethod,
    verified_delegation: Option<VerifiedTransportDelegation>,
}

impl VerifiedNostrProof {
    #[cfg(test)]
    pub(crate) fn new(
        authorization_domain: CommunityId,
        authorized_transport: AuthTransport,
        actor_pubkey: PublicKey,
        proof_method: AuthMethod,
        verified_delegation: Option<VerifiedTransportDelegation>,
    ) -> Result<Self, AuthContextError> {
        if !transport_accepts_proof(authorized_transport, proof_method) {
            return Err(AuthContextError::TransportProofMismatch);
        }
        if verified_delegation
            .as_ref()
            .is_some_and(|delegation| delegation.delegate_pubkey() != actor_pubkey)
        {
            return Err(AuthContextError::DelegateKeyMismatch);
        }
        Ok(Self {
            authorization_domain,
            authorized_transport,
            actor_pubkey,
            proof_method,
            verified_delegation,
        })
    }

    /// Authorization domain for which the Nostr proof was verified.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Transport whose request or connection the proof authorized.
    pub const fn authorized_transport(&self) -> AuthTransport {
        self.authorized_transport
    }

    /// Authenticated Nostr actor.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Cryptographic proof method accepted by the verifier.
    pub const fn proof_method(&self) -> AuthMethod {
        self.proof_method
    }

    /// Verified owner-to-actor delegation, when present.
    pub const fn verified_delegation(&self) -> Option<&VerifiedTransportDelegation> {
        self.verified_delegation.as_ref()
    }
}

impl fmt::Debug for VerifiedNostrProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedNostrProof")
            .field("authorization_domain", &"[redacted]")
            .field("authorized_transport", &self.authorized_transport)
            .field("actor_pubkey", &"[redacted]")
            .field("proof_method", &self.proof_method)
            .field("verified_delegation", &"[redacted]")
            .finish()
    }
}

/// Successful community admission and permissions for one decision.
///
/// An authorization adapter may construct this value only after membership,
/// invite, moderation, or equivalent community policy has allowed the actor.
/// Durable identity enrollment and public assertion publication must not occur
/// before this evidence exists; future binding adapters should require a borrow
/// of it before committing either side effect. Raw request scopes and channel
/// identifiers cannot construct this value in relay call sites. The resolution
/// is intentionally move-only and has no default or deserialization path.
#[derive(PartialEq, Eq)]
pub struct AuthorizedCommunityAccess {
    authorization_domain: CommunityId,
    scopes: Vec<Scope>,
    channel_ids: Option<Vec<Uuid>>,
}

impl AuthorizedCommunityAccess {
    #[cfg(test)]
    pub(crate) const fn new(
        authorization_domain: CommunityId,
        scopes: Vec<Scope>,
        channel_ids: Option<Vec<Uuid>>,
    ) -> Self {
        Self {
            authorization_domain,
            scopes,
            channel_ids,
        }
    }

    /// Authorization domain for which the permissions were resolved.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Permission scopes resolved for the decision.
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Optional channel restriction resolved for the decision.
    pub fn channel_ids(&self) -> Option<&[Uuid]> {
        self.channel_ids.as_deref()
    }

    /// Consume verified admission into the final immutable permissions.
    pub(super) fn into_permissions(self) -> (Vec<Scope>, Option<Vec<Uuid>>) {
        (self.scopes, self.channel_ids)
    }
}

impl fmt::Debug for AuthorizedCommunityAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedCommunityAccess")
            .field("authorization_domain", &"[redacted]")
            .field("scopes", &"[redacted]")
            .field("channel_ids", &"[redacted]")
            .finish()
    }
}
