//! Provider-neutral relay client binding status.
//!
//! This module is a one-way presentation adapter. It consumes a display-only
//! [`VerificationOnlyDisposition`](buzz_auth::VerificationOnlyDisposition) or
//! an opaque withdrawal request and returns one relay-signed
//! ephemeral event. The event has no route, ordinary ingest, event storage,
//! pub/sub, membership, capability, access-context, or lease integration. A
//! dedicated delivery trait exists behind a typed presentation
//! permit that only complete external RFC/client gate evidence can construct;
//! the stock binary supplies none and therefore remains disabled by default.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::{
    AuthorizationProfileId, BindingVersion, PolicyVersion, VerificationOnlyDisposition,
};
use buzz_core::{
    client_binding_bootstrap::CLIENT_BINDING_STATUS_SUB_ID,
    client_binding_status::{
        ClientBindingStatusBuildError, ClientBindingStatusError, ClientBindingStatusInputV1,
        MAX_CLIENT_BINDING_STATUS_LABEL_BYTES,
    },
    CommunityId,
};
use hmac::{Hmac, KeyInit, Mac};
use nostr::{Event, Keys, PublicKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const POLICY_REVISION_DOMAIN_SEPARATOR: &[u8] = b"buzz-client-status-policy-v1";

/// Dedicated secret for unlinkable provider-neutral client-status revisions.
///
/// Deployments must inject a purpose-specific random value. Reusing provider
/// assertion keys, relay signing keys, or any public identifier would make the
/// status revision linkable across trust domains.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientStatusPrivacyKey([u8; 32]);

impl ClientStatusPrivacyKey {
    /// Construct a client-status-only privacy key from 32 secret bytes.
    pub const fn from_secret(secret: [u8; 32]) -> Self {
        Self(secret)
    }
}

impl fmt::Debug for ClientStatusPrivacyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ClientStatusPrivacyKey")
            .field(&"[redacted]")
            .finish()
    }
}

/// Exact scope used to obtain a durable client-status revision.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientStatusRevisionScope {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
}

impl ClientStatusRevisionScope {
    /// Server-resolved authorization domain.
    pub const fn authorization_domain(self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact event-author key.
    pub const fn event_author_pubkey(self) -> PublicKey {
        self.event_author_pubkey
    }
}

impl fmt::Debug for ClientStatusRevisionScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientStatusRevisionScope")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .finish()
    }
}

/// Opaque proof of the exact current status delivered to one connection.
///
/// Callers cannot manufacture a different scope or revision. A withdrawal
/// must present this receipt so it always supersedes the status users saw.
pub struct ClientStatusIssuanceReceipt {
    scope: ClientStatusRevisionScope,
    connection_id: Uuid,
    revision: u64,
    issuance_fingerprint: [u8; 32],
}

impl ClientStatusIssuanceReceipt {
    /// Exact authenticated connection that received the current status.
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// Revision that a withdrawal must supersede.
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

impl fmt::Debug for ClientStatusIssuanceReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientStatusIssuanceReceipt")
            .field("scope", &self.scope)
            .field("connection_id", &"[redacted]")
            .field("revision", &"[redacted]")
            .finish()
    }
}

/// Revision and durable floor read atomically from an injected persistence seam.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DurableClientStatusRevision {
    revision: u64,
    floor: u64,
}

impl DurableClientStatusRevision {
    /// Validate a revision/floor pair returned by durable state.
    pub const fn from_durable_state(
        revision: u64,
        floor: u64,
    ) -> Result<Self, ClientStatusRevisionError> {
        if revision == 0 {
            return Err(ClientStatusRevisionError::ZeroRevision);
        }
        if revision < floor {
            return Err(ClientStatusRevisionError::BelowDurableFloor);
        }
        Ok(Self { revision, floor })
    }

    /// Current monotonic status revision.
    pub const fn revision(self) -> u64 {
        self.revision
    }

    /// Lowest revision allowed by durable reconciliation state.
    pub const fn floor(self) -> u64 {
        self.floor
    }
}

impl fmt::Debug for DurableClientStatusRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableClientStatusRevision")
            .field("revision", &"[redacted]")
            .field("floor", &"[redacted]")
            .finish()
    }
}

/// Read-only durable revision source supplied by the invalidation/reconciliation lane.
///
/// Implementations must never synthesize a process-local fallback. `None`
/// withholds status, and restore/restart must not return a revision below its
/// persisted floor.
#[async_trait]
pub trait DurableClientStatusRevisionSource: Send + Sync {
    /// Atomically revalidate an exact active/fresh binding and return its
    /// current revision. `None` withholds output.
    async fn current_revision_for(
        &self,
        requirement: &ClientStatusCurrentRequirement<'_>,
        issuance_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision>;

    /// Allocate a durable revision for the exact delivered current issuance.
    async fn withdrawal_revision_for(
        &self,
        receipt: &ClientStatusIssuanceReceipt,
        withdrawal_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision>;
}

mod postgres;
pub use postgres::PostgresClientStatusRevisionSource;

/// Exact private state that must still be active at the signing boundary.
pub struct ClientStatusCurrentRequirement<'a> {
    scope: ClientStatusRevisionScope,
    binding_id: Uuid,
    binding_version: BindingVersion,
    profile_id: &'a AuthorizationProfileId,
    policy_version: &'a PolicyVersion,
    evaluation_generation: u64,
    fresh_until: u64,
}

impl ClientStatusCurrentRequirement<'_> {
    /// Exact public status scope.
    pub const fn scope(&self) -> ClientStatusRevisionScope {
        self.scope
    }

    /// Stable binding identifier required to remain active.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact binding version required to remain active.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Exact provider profile required to remain current.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        self.profile_id
    }

    /// Exact provider policy version required to remain current.
    pub const fn policy_version(&self) -> &PolicyVersion {
        self.policy_version
    }

    /// Invalidation generation captured before provider evaluation.
    pub const fn evaluation_generation(&self) -> u64 {
        self.evaluation_generation
    }

    /// Absolute status freshness boundary.
    pub const fn fresh_until(&self) -> u64 {
        self.fresh_until
    }
}

impl fmt::Debug for ClientStatusCurrentRequirement<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientStatusCurrentRequirement")
            .field("scope", &self.scope)
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("evaluation_generation", &"[redacted]")
            .field("fresh_until", &"[redacted]")
            .finish()
    }
}

/// Pseudonymous policy revision safe for the client-status wire contract.
///
/// The provider profile and opaque policy string are authenticated under a
/// dedicated privacy key with length framing. The raw profile, issuer,
/// audience, claim names, and policy value never enter the event, and equal
/// provider values are unlinkable across deployments with distinct keys.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderNeutralPolicyRevision(String);

impl ProviderNeutralPolicyRevision {
    /// Derive a provider-neutral revision from current server/provider evidence.
    pub fn derive(
        privacy_key: &ClientStatusPrivacyKey,
        profile: &AuthorizationProfileId,
        policy: &PolicyVersion,
    ) -> Result<Self, ClientStatusPrivacyError> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&privacy_key.0)
            .map_err(|_| ClientStatusPrivacyError::InvalidKeyMaterial)?;
        mac.update(POLICY_REVISION_DOMAIN_SEPARATOR);
        update_length_framed(&mut mac, profile.as_str().as_bytes());
        update_length_framed(&mut mac, policy.as_str().as_bytes());
        Ok(Self(hex::encode(mac.finalize().into_bytes())))
    }

    /// Lowercase hex digest carried in the signed status.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderNeutralPolicyRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProviderNeutralPolicyRevision")
            .field(&"[redacted]")
            .finish()
    }
}

fn update_length_framed(mac: &mut Hmac<Sha256>, value: &[u8]) {
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
}

/// Optional display label loaded only from privacy-approved server configuration.
///
/// There is intentionally no constructor from assertion claims, provider
/// decisions, binding `display_name`, or mutable Nostr profiles.
#[derive(Clone, PartialEq, Eq)]
pub struct PrivacyApprovedClientStatusLabel(String);

impl PrivacyApprovedClientStatusLabel {
    /// Load a non-empty, bounded label from approved server configuration.
    pub fn from_server_configuration(
        value: impl Into<String>,
    ) -> Result<Self, PrivacyApprovedClientStatusLabelError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_CLIENT_BINDING_STATUS_LABEL_BYTES
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(PrivacyApprovedClientStatusLabelError::InvalidLabel);
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PrivacyApprovedClientStatusLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PrivacyApprovedClientStatusLabel")
            .field(&"[redacted]")
            .finish()
    }
}

/// Authoritative, presentation-only evidence used to sign one status.
///
/// Construction from verification-only finalization is preferred for current
/// display. Invalidation/reconciliation code may reuse only its exact scope
/// and freshness window to issue an opaque withdrawal. Private binding and
/// policy evidence remains server-side and is never serialized into the event.
pub struct AuthoritativeClientStatusEvidence {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    binding_id: Uuid,
    binding_version: BindingVersion,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
    policy_revision: ProviderNeutralPolicyRevision,
    issuance_id: Uuid,
    evaluation_generation: u64,
    issued_at: u64,
    fresh_until: u64,
}

impl AuthoritativeClientStatusEvidence {
    /// Derive current display evidence from full verification-only finalization.
    pub fn from_verification_only(
        disposition: &VerificationOnlyDisposition,
        privacy_key: &ClientStatusPrivacyKey,
        evaluation_generation: u64,
    ) -> Result<Self, ClientStatusPrivacyError> {
        Ok(Self {
            authorization_domain: disposition.authorization_domain(),
            event_author_pubkey: disposition.actor_pubkey(),
            binding_id: disposition.binding_id(),
            binding_version: disposition.binding_version(),
            profile_id: disposition.profile_id().clone(),
            policy_version: disposition.policy_version().clone(),
            policy_revision: ProviderNeutralPolicyRevision::derive(
                privacy_key,
                disposition.profile_id(),
                disposition.policy_version(),
            )?,
            issuance_id: disposition.correlation_id(),
            evaluation_generation,
            issued_at: disposition.issued_at(),
            fresh_until: disposition.expires_at(),
        })
    }

    /// Consume separately authoritative runtime/lifecycle evidence.
    ///
    /// Callers must use server-resolved domain/key state and centrally injected
    /// time. This constructor validates representation through the core
    /// contract during issuance; it performs no persistence or lifecycle read.
    #[allow(clippy::too_many_arguments)]
    pub const fn from_authoritative_runtime(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
        binding_id: Uuid,
        binding_version: BindingVersion,
        profile_id: AuthorizationProfileId,
        policy_version: PolicyVersion,
        policy_revision: ProviderNeutralPolicyRevision,
        issuance_id: Uuid,
        evaluation_generation: u64,
        issued_at: u64,
        fresh_until: u64,
    ) -> Self {
        Self {
            authorization_domain,
            event_author_pubkey,
            binding_id,
            binding_version,
            profile_id,
            policy_version,
            policy_revision,
            issuance_id,
            evaluation_generation,
            issued_at,
            fresh_until,
        }
    }

    fn revision_scope(&self) -> ClientStatusRevisionScope {
        ClientStatusRevisionScope {
            authorization_domain: self.authorization_domain,
            event_author_pubkey: self.event_author_pubkey,
        }
    }

    fn current_requirement(&self) -> ClientStatusCurrentRequirement<'_> {
        ClientStatusCurrentRequirement {
            scope: self.revision_scope(),
            binding_id: self.binding_id,
            binding_version: self.binding_version,
            profile_id: &self.profile_id,
            policy_version: &self.policy_version,
            evaluation_generation: self.evaluation_generation,
            fresh_until: self.fresh_until,
        }
    }
}

impl fmt::Debug for AuthoritativeClientStatusEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthoritativeClientStatusEvidence")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("policy_revision", &self.policy_revision)
            .field("issuance_id", &"[redacted]")
            .field("evaluation_generation", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .field("fresh_until", &"[redacted]")
            .finish()
    }
}

/// Relay signer for display-only client binding status.
pub struct RelayClientBindingStatusIssuer<'a> {
    relay_keys: &'a Keys,
    revisions: &'a dyn DurableClientStatusRevisionSource,
    privacy_key: &'a ClientStatusPrivacyKey,
}

impl<'a> RelayClientBindingStatusIssuer<'a> {
    /// Bind a relay signing key and externally durable revision source.
    pub const fn new(
        relay_keys: &'a Keys,
        revisions: &'a dyn DurableClientStatusRevisionSource,
        privacy_key: &'a ClientStatusPrivacyKey,
    ) -> Self {
        Self {
            relay_keys,
            revisions,
            privacy_key,
        }
    }

    /// Sign a generic withdrawal without exposing its lifecycle cause.
    pub async fn issue_withdrawn(
        &self,
        evidence: &AuthoritativeClientStatusEvidence,
        receipt: &ClientStatusIssuanceReceipt,
    ) -> Result<Event, RelayClientStatusError> {
        if receipt.scope != evidence.revision_scope() {
            return Err(RelayClientStatusError::IssuanceReceiptMismatch);
        }
        let withdrawal_fingerprint = withdrawal_fingerprint(receipt);
        let revision = self
            .revisions
            .withdrawal_revision_for(receipt, withdrawal_fingerprint)
            .await
            .ok_or(RelayClientStatusError::RevisionUnavailable)?;
        if revision.revision() <= receipt.revision {
            return Err(RelayClientStatusError::RevisionDidNotAdvance);
        }
        ClientBindingStatusInputV1::withdrawn(
            evidence.authorization_domain,
            evidence.event_author_pubkey,
            revision.revision(),
            evidence.issued_at,
            evidence.fresh_until,
        )?
        .sign_with_relay_keys(self.relay_keys)
        .map_err(Into::into)
    }

    async fn issue_current(
        &self,
        evidence: &AuthoritativeClientStatusEvidence,
        label: Option<&PrivacyApprovedClientStatusLabel>,
    ) -> Result<(Event, u64), RelayClientStatusError> {
        let issuance_fingerprint = current_issuance_fingerprint(evidence, label);
        let revision = self
            .revisions
            .current_revision_for(&evidence.current_requirement(), issuance_fingerprint)
            .await
            .ok_or(RelayClientStatusError::RevisionUnavailable)?;
        let input = ClientBindingStatusInputV1::current(
            evidence.authorization_domain,
            evidence.event_author_pubkey,
            evidence.binding_version.get(),
            evidence.policy_revision.as_str(),
            revision.revision(),
            evidence.issued_at,
            evidence.fresh_until,
            label.map(|value| value.as_str().to_string()),
        )?;
        let event = input
            .sign_with_relay_keys(self.relay_keys)
            .map_err(RelayClientStatusError::from)?;
        Ok((event, revision.revision()))
    }
}

fn current_issuance_fingerprint(
    evidence: &AuthoritativeClientStatusEvidence,
    label: Option<&PrivacyApprovedClientStatusLabel>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"buzz-client-status-current-issuance-v2");
    digest.update(evidence.authorization_domain.as_uuid().as_bytes());
    digest.update(evidence.event_author_pubkey.to_bytes());
    digest.update(evidence.binding_id.as_bytes());
    digest.update(evidence.binding_version.get().to_be_bytes());
    let profile = evidence.profile_id.as_str().as_bytes();
    digest.update((profile.len() as u64).to_be_bytes());
    digest.update(profile);
    let policy = evidence.policy_version.as_str().as_bytes();
    digest.update((policy.len() as u64).to_be_bytes());
    digest.update(policy);
    digest.update(evidence.issuance_id.as_bytes());
    digest.update(evidence.evaluation_generation.to_be_bytes());
    digest.update(evidence.issued_at.to_be_bytes());
    digest.update(evidence.fresh_until.to_be_bytes());
    if let Some(label) = label {
        digest.update([1]);
        let label = label.as_str().as_bytes();
        digest.update((label.len() as u64).to_be_bytes());
        digest.update(label);
    } else {
        digest.update([0]);
    }
    digest.finalize().into()
}

fn withdrawal_fingerprint(receipt: &ClientStatusIssuanceReceipt) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"buzz-client-status-withdrawal-v1");
    digest.update(receipt.scope.authorization_domain().as_uuid().as_bytes());
    digest.update(receipt.scope.event_author_pubkey().to_bytes());
    // Every authenticated connection showing this author shares one durable
    // withdrawal allocation. The connection remains a delivery target only.
    digest.update(receipt.revision.to_be_bytes());
    digest.update(receipt.issuance_fingerprint);
    digest.finalize().into()
}

impl fmt::Debug for RelayClientBindingStatusIssuer<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayClientBindingStatusIssuer")
            .field("relay_keys", &"[redacted]")
            .field("revisions", &"[injected]")
            .field("privacy_key", &"[redacted]")
            .finish()
    }
}

/// Opaque permission to expose a status on the dedicated authenticated path.
///
/// This type has no unchecked constructor. Complete external gate evidence is
/// required; ordinary event ingest and pub/sub are never valid substitutes.
pub struct ClientStatusPresentationPermit {
    _private: (),
}

/// Deployment-owned evidence that the RFC presentation gate and exact client
/// contract have both been approved for one reviewed revision.
pub trait CompleteClientStatusPresentationApproval: Send + Sync {
    /// Exact lowercase Git revision reviewed by every presentation gate.
    fn reviewed_implementation_revision(&self) -> &str;

    /// Whether the applicable RFC presentation/privacy gate passed.
    fn presentation_gate_passed(&self) -> bool;

    /// Whether the dedicated client transport contract passed end to end.
    fn dedicated_client_contract_passed(&self) -> bool;
}

impl ClientStatusPresentationPermit {
    /// Construct the otherwise unavailable permit from complete external gate
    /// evidence. There is intentionally no environment/boolean constructor.
    pub fn from_complete_stack(
        approval: &dyn CompleteClientStatusPresentationApproval,
    ) -> Result<Self, ClientStatusPresentationGateError> {
        let revision = approval.reviewed_implementation_revision();
        if revision.len() != 40
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !approval.presentation_gate_passed()
            || !approval.dedicated_client_contract_passed()
        {
            return Err(ClientStatusPresentationGateError::Incomplete);
        }
        Ok(Self { _private: () })
    }
}

impl fmt::Debug for ClientStatusPresentationPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientStatusPresentationPermit")
            .finish_non_exhaustive()
    }
}

/// One relay-authenticated status targeted to an exact connection scope.
///
/// The transport implementation must deliver only to the authenticated
/// connection for `authorization_domain` and `event_author_pubkey`. It must
/// not route through event ingestion, storage, subscriptions, or pub/sub.
pub struct DedicatedClientStatusDelivery<'a> {
    event: &'a Event,
    relay_pubkey: PublicKey,
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    connection_id: Uuid,
}

impl DedicatedClientStatusDelivery<'_> {
    /// Relay-signed ephemeral status event.
    pub const fn event(&self) -> &Event {
        self.event
    }

    /// Relay key against which the transport must authenticate the event.
    pub const fn relay_pubkey(&self) -> PublicKey {
        self.relay_pubkey
    }

    /// Server-resolved authorization domain of the target connection.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact authenticated event-author key of the target connection.
    pub const fn event_author_pubkey(&self) -> PublicKey {
        self.event_author_pubkey
    }

    /// Exact server-owned authenticated connection target.
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }
}

impl fmt::Debug for DedicatedClientStatusDelivery<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DedicatedClientStatusDelivery")
            .field("event", &"[redacted]")
            .field("relay_pubkey", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("connection_id", &"[redacted]")
            .finish()
    }
}

/// Dedicated relay-authenticated status channel.
///
/// Implementations become reachable only behind the separately approved RFC
/// presentation gate. This trait must never be implemented by ordinary event
/// ingest, persistence, subscription, or pub/sub paths.
pub trait DedicatedClientStatusTransport: Send + Sync {
    /// Deliver one status only to its exact authenticated connection scope.
    fn deliver(
        &self,
        delivery: DedicatedClientStatusDelivery<'_>,
    ) -> Result<(), DedicatedClientStatusTransportError>;
}

/// Result of one current-status delivery attempt.
///
/// The durable receipt is retained even when the transport reports failure,
/// because the effect may have become visible before its acknowledgement was
/// lost. Callers must register the receipt for invalidation reconciliation.
pub struct ClientStatusDeliveryAttempt {
    event: Event,
    receipt: ClientStatusIssuanceReceipt,
    delivery_error: Option<DedicatedClientStatusTransportError>,
}

impl ClientStatusDeliveryAttempt {
    /// Relay-signed current status attempted on the dedicated connection.
    pub const fn event(&self) -> &Event {
        &self.event
    }

    /// Durable receipt that must remain withdrawable after any delivery result.
    pub const fn receipt(&self) -> &ClientStatusIssuanceReceipt {
        &self.receipt
    }

    /// Dedicated transport result; failure can be ambiguous after visibility.
    pub const fn delivery_error(&self) -> Option<DedicatedClientStatusTransportError> {
        self.delivery_error
    }

    /// Consume the attempt while retaining its exact withdrawal receipt.
    pub fn into_receipt(self) -> ClientStatusIssuanceReceipt {
        self.receipt
    }
}

/// Dedicated exact-connection transport backed by the live connection
/// manager. It bypasses event ingest, storage, subscriptions, and pub/sub.
pub struct ConnectionManagerClientStatusTransport {
    connections: Arc<crate::state::ConnectionManager>,
}

impl ConnectionManagerClientStatusTransport {
    /// Bind the server-owned connection registry.
    pub fn new(connections: Arc<crate::state::ConnectionManager>) -> Self {
        Self { connections }
    }
}

impl DedicatedClientStatusTransport for ConnectionManagerClientStatusTransport {
    fn deliver(
        &self,
        delivery: DedicatedClientStatusDelivery<'_>,
    ) -> Result<(), DedicatedClientStatusTransportError> {
        if self
            .connections
            .community_for_conn(delivery.connection_id())
            != Some(delivery.authorization_domain())
            || self
                .connections
                .pubkey_for(delivery.connection_id())
                .as_deref()
                != Some(delivery.event_author_pubkey().as_bytes())
        {
            return Err(DedicatedClientStatusTransportError::Unavailable);
        }
        let frame =
            crate::protocol::RelayMessage::event(CLIENT_BINDING_STATUS_SUB_ID, delivery.event());
        self.connections
            .send_to(delivery.connection_id(), frame)
            .then_some(())
            .ok_or(DedicatedClientStatusTransportError::Unavailable)
    }
}

/// Installed, opt-in production presentation runtime.
///
/// Construction requires the complete external gate. The stock OSS binary
/// never creates this value, so presentation remains disabled by default.
pub struct ProductionClientStatusRuntime {
    permit: Arc<ClientStatusPresentationPermit>,
    privacy_key: ClientStatusPrivacyKey,
    transport: Arc<dyn DedicatedClientStatusTransport>,
}

impl ProductionClientStatusRuntime {
    /// Bind approved presentation evidence to a dedicated transport.
    pub fn new(
        permit: ClientStatusPresentationPermit,
        privacy_key: ClientStatusPrivacyKey,
        transport: Arc<dyn DedicatedClientStatusTransport>,
    ) -> Self {
        Self {
            permit: Arc::new(permit),
            privacy_key,
            transport,
        }
    }

    /// Evaluate, issue, and register withdrawal for one authenticated direct
    /// connection. Failure withholds presentation and never changes access.
    pub async fn present_after_auth(
        self: &Arc<Self>,
        state: Arc<crate::state::AppState>,
        proof: Arc<buzz_auth::VerifiedNostrProof>,
        assertion: Arc<buzz_auth::VerifiedFederatedAssertion>,
        connection_id: Uuid,
        connection_cancellation: CancellationToken,
    ) -> Result<(), ClientStatusRuntimeError> {
        let protected = state
            .protected_transport()
            .ok_or(ClientStatusRuntimeError::ProtectedRuntimeUnavailable)?;
        // Presentation invalidation must withdraw the indicator without
        // becoming connection authority in VerifyOnly. Enforce retains its
        // independent protected-session cancellation fence.
        let presentation_cancellation = CancellationToken::new();
        let session_target = state
            .conn_manager
            .authorization_session_target(connection_id)
            .ok_or(super::transport::ProtectedTransportError::InvalidSessionId)?;
        let request = super::transport::ProtectedOperationRequest::new_with_cancellation(
            proof,
            Some(assertion),
            buzz_auth::AuthorizationCapability::CommunityRead,
            Uuid::new_v4(),
            "client.status.current",
            Some(session_target),
            Some(presentation_cancellation.clone()),
        )?;
        let Some(resolution) = protected.present_status(&request).await? else {
            return Ok(());
        };
        let (disposition, observer, evaluation_generation) = resolution.into_parts();
        observer
            .observe_current()
            .map_err(|_| ClientStatusRuntimeError::StatusStale)?;
        let evidence = AuthoritativeClientStatusEvidence::from_verification_only(
            &disposition,
            &self.privacy_key,
            evaluation_generation,
        )?;
        let restore = state
            .restore_protection()
            .cloned()
            .ok_or(ClientStatusRuntimeError::ProtectedRuntimeUnavailable)?;
        let revisions = PostgresClientStatusRevisionSource::new(state.db.clone(), restore);
        let issuer = RelayClientBindingStatusIssuer::new(
            &state.relay_keypair,
            &revisions,
            &self.privacy_key,
        );
        let attempt = issuer
            .deliver_verification_only(
                &self.permit,
                &disposition,
                evaluation_generation,
                None,
                connection_id,
                self.transport.as_ref(),
            )
            .await?;
        let delivery_failed = attempt.delivery_error().is_some();
        let receipt = attempt.into_receipt();
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let now = nostr::Timestamp::now().as_secs();
            let expiry_delay = Duration::from_secs(disposition.expires_at().saturating_sub(now));
            let invalidated = tokio::select! {
                _ = connection_cancellation.cancelled() => false,
                _ = presentation_cancellation.cancelled() => true,
                _ = tokio::time::sleep(expiry_delay) => {
                    // Status freshness is exclusive and clients clear locally at
                    // this bound. Dropping the observer prevents any extension.
                    false
                }
            };
            if invalidated {
                if let Some(restore) = state.restore_protection().cloned() {
                    let revisions =
                        PostgresClientStatusRevisionSource::new(state.db.clone(), restore);
                    let issuer = RelayClientBindingStatusIssuer::new(
                        &state.relay_keypair,
                        &revisions,
                        &runtime.privacy_key,
                    );
                    let _ = issuer
                        .deliver_withdrawn_after_invalidation(
                            &runtime.permit,
                            &evidence,
                            &receipt,
                            runtime.transport.as_ref(),
                        )
                        .await;
                }
            }
            drop(observer);
        });
        if delivery_failed {
            return Err(ClientStatusRuntimeError::DeliveryUnavailable);
        }
        Ok(())
    }
}

impl fmt::Debug for ClientStatusDeliveryAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientStatusDeliveryAttempt")
            .field("event", &"[redacted]")
            .field("receipt", &self.receipt)
            .field("delivery_error", &self.delivery_error)
            .finish()
    }
}

impl RelayClientBindingStatusIssuer<'_> {
    /// Issue and deliver verification-only status through the typed gate.
    ///
    /// Returning the delivery future preserves the event-storage-agnostic
    /// public contract while durable revision allocation remains an injected
    /// asynchronous implementation detail.
    pub fn issue_verification_only<'a>(
        &'a self,
        permit: &'a ClientStatusPresentationPermit,
        disposition: &'a VerificationOnlyDisposition,
        evaluation_generation: u64,
        label: Option<&'a PrivacyApprovedClientStatusLabel>,
        connection_id: Uuid,
        transport: &'a dyn DedicatedClientStatusTransport,
    ) -> impl std::future::Future<Output = Result<ClientStatusDeliveryAttempt, RelayClientStatusError>>
           + 'a {
        self.deliver_verification_only(
            permit,
            disposition,
            evaluation_generation,
            label,
            connection_id,
            transport,
        )
    }

    async fn deliver_current(
        &self,
        evidence: AuthoritativeClientStatusEvidence,
        label: Option<&PrivacyApprovedClientStatusLabel>,
        connection_id: Uuid,
        transport: &dyn DedicatedClientStatusTransport,
    ) -> Result<ClientStatusDeliveryAttempt, RelayClientStatusError> {
        let issuance_fingerprint = current_issuance_fingerprint(&evidence, label);
        let (event, revision) = self.issue_current(&evidence, label).await?;
        let delivery_error = transport
            .deliver(DedicatedClientStatusDelivery {
                event: &event,
                relay_pubkey: self.relay_keys.public_key(),
                authorization_domain: evidence.authorization_domain,
                event_author_pubkey: evidence.event_author_pubkey,
                connection_id,
            })
            .err();
        Ok(ClientStatusDeliveryAttempt {
            event,
            receipt: ClientStatusIssuanceReceipt {
                scope: evidence.revision_scope(),
                connection_id,
                revision,
                issuance_fingerprint,
            },
            delivery_error,
        })
    }

    /// Issue and deliver a verification-only status on the dedicated path.
    ///
    /// The production runtime can reach this only after injected complete
    /// presentation approval constructs `permit`. It never creates a client
    /// route or weakens verification-only authorization semantics.
    pub async fn deliver_verification_only(
        &self,
        _permit: &ClientStatusPresentationPermit,
        disposition: &VerificationOnlyDisposition,
        evaluation_generation: u64,
        label: Option<&PrivacyApprovedClientStatusLabel>,
        connection_id: Uuid,
        transport: &dyn DedicatedClientStatusTransport,
    ) -> Result<ClientStatusDeliveryAttempt, RelayClientStatusError> {
        let evidence = AuthoritativeClientStatusEvidence::from_verification_only(
            disposition,
            self.privacy_key,
            evaluation_generation,
        )?;
        self.deliver_current(evidence, label, connection_id, transport)
            .await
    }

    /// Issue an opaque, strictly newer withdrawal after invalidation and
    /// deliver it only to the exact authenticated connection. Without an
    /// externally approved and installed presentation runtime, this path
    /// remains unreachable.
    pub async fn deliver_withdrawn_after_invalidation(
        &self,
        _permit: &ClientStatusPresentationPermit,
        evidence: &AuthoritativeClientStatusEvidence,
        receipt: &ClientStatusIssuanceReceipt,
        transport: &dyn DedicatedClientStatusTransport,
    ) -> Result<Event, RelayClientStatusError> {
        if receipt.connection_id.is_nil() {
            return Err(RelayClientStatusError::IssuanceReceiptMismatch);
        }
        let event = self.issue_withdrawn(evidence, receipt).await?;
        transport.deliver(DedicatedClientStatusDelivery {
            event: &event,
            relay_pubkey: self.relay_keys.public_key(),
            authorization_domain: evidence.authorization_domain,
            event_author_pubkey: evidence.event_author_pubkey,
            connection_id: receipt.connection_id,
        })?;
        Ok(event)
    }
}

/// Opaque dedicated-transport failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DedicatedClientStatusTransportError {
    /// Exact authenticated connection delivery was unavailable.
    #[error("dedicated client-status transport is unavailable")]
    Unavailable,
}

/// Incomplete client-presentation approval evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientStatusPresentationGateError {
    /// Revision, RFC gate, or dedicated client contract was incomplete.
    #[error("client-status presentation approval is incomplete")]
    Incomplete,
}

/// Fail-closed production client-status result. These failures withhold only
/// presentation and never alter access policy.
#[derive(Debug, Error)]
pub enum ClientStatusRuntimeError {
    /// The protected/restore runtime was not completely installed.
    #[error("client-status protected runtime is unavailable")]
    ProtectedRuntimeUnavailable,
    /// Dedicated delivery failed or became ambiguous.
    #[error("client-status delivery is unavailable")]
    DeliveryUnavailable,
    /// Current authority changed after evaluation and before delivery.
    #[error("client-status authority is stale")]
    StatusStale,
    /// Protected status evaluation failed.
    #[error(transparent)]
    Protected(#[from] super::transport::ProtectedTransportError),
    /// Status construction, revision allocation, or signing failed.
    #[error(transparent)]
    Status(#[from] RelayClientStatusError),
    /// Privacy transform initialization failed.
    #[error(transparent)]
    Privacy(#[from] ClientStatusPrivacyError),
}

/// Invalid durable revision/floor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientStatusRevisionError {
    /// Revision zero cannot be issued.
    #[error("durable client-status revision must be positive")]
    ZeroRevision,
    /// Reconciliation returned a revision below its durable floor.
    #[error("durable client-status revision is below its floor")]
    BelowDurableFloor,
}

/// Invalid privacy-approved label configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PrivacyApprovedClientStatusLabelError {
    /// The configured label was empty, unsafe, or exceeded its public bound.
    #[error("privacy-approved client-status label is invalid")]
    InvalidLabel,
}

/// Failure to initialize the keyed client-status privacy transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientStatusPrivacyError {
    /// The injected privacy key could not initialize the HMAC primitive.
    #[error("client-status privacy key material is invalid")]
    InvalidKeyMaterial,
}

/// Fail-closed status issuance error.
#[derive(Debug, Error)]
pub enum RelayClientStatusError {
    /// Durable status revision/floor state was unavailable.
    #[error("durable client-status revision is unavailable")]
    RevisionUnavailable,
    /// A withdrawal revision did not supersede the last current status.
    #[error("durable client-status withdrawal revision did not advance")]
    RevisionDidNotAdvance,
    /// Withdrawal did not name the exact delivered current status and connection.
    #[error("client-status issuance receipt does not match the withdrawal scope")]
    IssuanceReceiptMismatch,
    /// The provider-neutral revision could not be derived safely.
    #[error(transparent)]
    Privacy(#[from] ClientStatusPrivacyError),
    /// Dedicated relay-authenticated delivery failed.
    #[error(transparent)]
    DedicatedTransport(#[from] DedicatedClientStatusTransportError),
    /// Core status representation was invalid.
    #[error(transparent)]
    InvalidStatus(#[from] ClientBindingStatusError),
    /// Event construction or signing failed.
    #[error(transparent)]
    Build(#[from] ClientBindingStatusBuildError),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU8;
    use std::sync::Mutex;

    use buzz_auth::{AuthorizationProfileId, PolicyVersion};
    use buzz_core::client_binding_status::{
        validate_client_binding_status_event, ClientBindingStatusDisposition,
    };
    use uuid::Uuid;

    use super::*;

    const ISSUED_AT: u64 = 1_800_000_000;
    type ObservedCurrentRequirement = (Uuid, u64, String, String, u64);
    type ObservedDedicatedDelivery = (nostr::EventId, CommunityId, PublicKey, PublicKey, Uuid);

    struct SyntheticRevisions {
        value: Option<DurableClientStatusRevision>,
        seen: Mutex<Vec<ClientStatusRevisionScope>>,
        requirements: Mutex<Vec<ObservedCurrentRequirement>>,
    }

    struct SyntheticDedicatedTransport {
        deliveries: Mutex<Vec<ObservedDedicatedDelivery>>,
    }

    struct FailingDedicatedTransport;

    struct SyntheticPresentationApproval {
        revision: &'static str,
        presentation: bool,
        client: bool,
    }

    impl CompleteClientStatusPresentationApproval for SyntheticPresentationApproval {
        fn reviewed_implementation_revision(&self) -> &str {
            self.revision
        }

        fn presentation_gate_passed(&self) -> bool {
            self.presentation
        }

        fn dedicated_client_contract_passed(&self) -> bool {
            self.client
        }
    }

    impl DedicatedClientStatusTransport for FailingDedicatedTransport {
        fn deliver(
            &self,
            _delivery: DedicatedClientStatusDelivery<'_>,
        ) -> Result<(), DedicatedClientStatusTransportError> {
            Err(DedicatedClientStatusTransportError::Unavailable)
        }
    }

    impl DedicatedClientStatusTransport for SyntheticDedicatedTransport {
        fn deliver(
            &self,
            delivery: DedicatedClientStatusDelivery<'_>,
        ) -> Result<(), DedicatedClientStatusTransportError> {
            self.deliveries.lock().expect("synthetic lock").push((
                delivery.event().id,
                delivery.authorization_domain(),
                delivery.event_author_pubkey(),
                delivery.relay_pubkey(),
                delivery.connection_id(),
            ));
            Ok(())
        }
    }

    #[async_trait]
    impl DurableClientStatusRevisionSource for SyntheticRevisions {
        async fn current_revision_for(
            &self,
            requirement: &ClientStatusCurrentRequirement<'_>,
            _issuance_fingerprint: [u8; 32],
        ) -> Option<DurableClientStatusRevision> {
            let scope = requirement.scope();
            self.seen.lock().expect("synthetic lock").push(scope);
            self.requirements.lock().expect("synthetic lock").push((
                requirement.binding_id(),
                requirement.binding_version().get(),
                requirement.profile_id().as_str().to_owned(),
                requirement.policy_version().as_str().to_owned(),
                requirement.fresh_until(),
            ));
            self.value
        }

        async fn withdrawal_revision_for(
            &self,
            receipt: &ClientStatusIssuanceReceipt,
            _withdrawal_fingerprint: [u8; 32],
        ) -> Option<DurableClientStatusRevision> {
            self.seen
                .lock()
                .expect("synthetic lock")
                .push(receipt.scope);
            self.value
        }
    }

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(7))
    }

    fn privacy_key() -> ClientStatusPrivacyKey {
        ClientStatusPrivacyKey::from_secret([0x51; 32])
    }

    fn policy_revision() -> ProviderNeutralPolicyRevision {
        let profile = AuthorizationProfileId::from_server_configuration("synthetic-profile")
            .expect("synthetic profile is valid");
        let policy =
            PolicyVersion::new("private-provider-policy-value").expect("synthetic policy is valid");
        ProviderNeutralPolicyRevision::derive(&privacy_key(), &profile, &policy)
            .expect("synthetic privacy key derives a revision")
    }

    fn evidence(author: PublicKey) -> AuthoritativeClientStatusEvidence {
        AuthoritativeClientStatusEvidence::from_authoritative_runtime(
            domain(),
            author,
            Uuid::from_u128(0x900),
            BindingVersion::new(9).expect("synthetic binding version is valid"),
            AuthorizationProfileId::from_server_configuration("synthetic-profile")
                .expect("synthetic profile is valid"),
            PolicyVersion::new("private-provider-policy-value").expect("synthetic policy is valid"),
            policy_revision(),
            Uuid::from_u128(0x901),
            1,
            ISSUED_AT,
            ISSUED_AT + 120,
        )
    }

    fn receipt(author: PublicKey, revision: u64) -> ClientStatusIssuanceReceipt {
        ClientStatusIssuanceReceipt {
            scope: ClientStatusRevisionScope {
                authorization_domain: domain(),
                event_author_pubkey: author,
            },
            connection_id: Uuid::from_u128(0x902),
            revision,
            issuance_fingerprint: [0x45; 32],
        }
    }

    fn revisions(value: Option<DurableClientStatusRevision>) -> SyntheticRevisions {
        SyntheticRevisions {
            value,
            seen: Mutex::new(Vec::new()),
            requirements: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn durable_revision_validates_floor() {
        assert_eq!(
            DurableClientStatusRevision::from_durable_state(0, 0),
            Err(ClientStatusRevisionError::ZeroRevision)
        );
        assert_eq!(
            DurableClientStatusRevision::from_durable_state(8, 9),
            Err(ClientStatusRevisionError::BelowDurableFloor)
        );
        let revision = DurableClientStatusRevision::from_durable_state(9, 9)
            .expect("revision at its floor is valid");
        assert_eq!(revision.revision(), 9);
        assert_eq!(revision.floor(), 9);
    }

    #[test]
    fn presentation_permit_requires_one_exact_complete_revision() {
        let complete = SyntheticPresentationApproval {
            revision: "0123456789abcdef0123456789abcdef01234567",
            presentation: true,
            client: true,
        };
        assert!(ClientStatusPresentationPermit::from_complete_stack(&complete).is_ok());

        for incomplete in [
            SyntheticPresentationApproval {
                revision: "not-a-revision",
                presentation: true,
                client: true,
            },
            SyntheticPresentationApproval {
                revision: "0123456789abcdef0123456789abcdef01234567",
                presentation: false,
                client: true,
            },
            SyntheticPresentationApproval {
                revision: "0123456789abcdef0123456789abcdef01234567",
                presentation: true,
                client: false,
            },
        ] {
            assert!(matches!(
                ClientStatusPresentationPermit::from_complete_stack(&incomplete),
                Err(ClientStatusPresentationGateError::Incomplete)
            ));
        }
    }

    #[tokio::test]
    async fn withdrawal_uses_exact_scope_and_omits_current_state() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(12, 11)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let issuance = receipt(author.public_key(), 11);
        let event = issuer
            .issue_withdrawn(&evidence(author.public_key()), &issuance)
            .await
            .expect("withdrawal event signs");
        let status = validate_client_binding_status_event(
            &event,
            &relay.public_key(),
            domain(),
            &author.public_key(),
            ISSUED_AT,
        )
        .expect("signed status validates");

        assert_eq!(status.status_revision(), 12);
        assert_eq!(status.binding_version(), None);
        assert_eq!(status.policy_version(), None);
        assert!(!event.content.contains("private-provider-policy-value"));
        assert!(!event.content.contains("synthetic-profile"));
        assert_eq!(
            status.disposition(),
            ClientBindingStatusDisposition::Withdrawn
        );

        let seen = source.seen.lock().expect("synthetic lock");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].authorization_domain(), domain());
        assert_eq!(seen[0].event_author_pubkey(), author.public_key());
    }

    #[tokio::test]
    async fn missing_durable_revision_withholds_all_output() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(None);
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let issuance = receipt(author.public_key(), 1);
        assert!(matches!(
            issuer
                .issue_withdrawn(&evidence(author.public_key()), &issuance)
                .await,
            Err(RelayClientStatusError::RevisionUnavailable)
        ));
    }

    #[tokio::test]
    async fn current_issuance_revalidates_exact_private_binding_state() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(12, 12)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        issuer
            .issue_current(&evidence(author.public_key()), None)
            .await
            .expect("exact current binding signs");

        let requirements = source.requirements.lock().expect("synthetic lock");
        assert_eq!(requirements.len(), 1);
        let requirement = &requirements[0];
        assert_eq!(requirement.0, Uuid::from_u128(0x900));
        assert_eq!(requirement.1, 9);
        assert_eq!(requirement.2, "synthetic-profile");
        assert_eq!(requirement.3, "private-provider-policy-value");
        assert_eq!(requirement.4, ISSUED_AT + 120);
    }

    #[tokio::test]
    async fn withdrawal_must_strictly_advance() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(12, 12)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let issuance = receipt(author.public_key(), 12);
        assert!(matches!(
            issuer
                .issue_withdrawn(&evidence(author.public_key()), &issuance)
                .await,
            Err(RelayClientStatusError::RevisionDidNotAdvance)
        ));
    }

    #[test]
    fn privacy_label_is_configuration_only_and_current_only() {
        assert!(PrivacyApprovedClientStatusLabel::from_server_configuration("").is_err());
        assert!(PrivacyApprovedClientStatusLabel::from_server_configuration(" private").is_err());
        let label = PrivacyApprovedClientStatusLabel::from_server_configuration(
            "Privacy Approved Enterprise",
        )
        .expect("synthetic configured label is valid");
        assert_eq!(label.as_str(), "Privacy Approved Enterprise");
        assert_eq!(
            format!("{label:?}"),
            "PrivacyApprovedClientStatusLabel(\"[redacted]\")"
        );
    }

    #[test]
    fn policy_revision_is_keyed_and_unlinkable_across_privacy_keys() {
        let profile = AuthorizationProfileId::from_server_configuration("synthetic-profile")
            .expect("synthetic profile is valid");
        let policy =
            PolicyVersion::new("private-provider-policy-value").expect("synthetic policy is valid");
        let first = ProviderNeutralPolicyRevision::derive(
            &ClientStatusPrivacyKey::from_secret([0x11; 32]),
            &profile,
            &policy,
        )
        .expect("first synthetic key derives a revision");
        let second = ProviderNeutralPolicyRevision::derive(
            &ClientStatusPrivacyKey::from_secret([0x22; 32]),
            &profile,
            &policy,
        )
        .expect("second synthetic key derives a revision");

        assert_eq!(first.as_str().len(), 64);
        assert_eq!(second.as_str().len(), 64);
        assert_ne!(first, second);
        for revision in [first.as_str(), second.as_str()] {
            assert!(!revision.contains(profile.as_str()));
            assert!(!revision.contains(policy.as_str()));
        }
        assert_eq!(
            format!("{:?}", ClientStatusPrivacyKey::from_secret([0x33; 32])),
            "ClientStatusPrivacyKey(\"[redacted]\")"
        );
    }

    #[tokio::test]
    async fn dedicated_transport_contract_is_exact_scope_and_test_only_permitted() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(14, 14)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let issuance = receipt(author.public_key(), 13);
        let event = issuer
            .issue_withdrawn(&evidence(author.public_key()), &issuance)
            .await
            .expect("synthetic gated status signs");
        let permit = ClientStatusPresentationPermit { _private: () };
        let connection_id = Uuid::new_v4();
        let transport = SyntheticDedicatedTransport {
            deliveries: Mutex::new(Vec::new()),
        };

        transport
            .deliver(DedicatedClientStatusDelivery {
                event: &event,
                relay_pubkey: relay.public_key(),
                authorization_domain: domain(),
                event_author_pubkey: author.public_key(),
                connection_id,
            })
            .expect("synthetic dedicated delivery succeeds");

        let deliveries = transport.deliveries.lock().expect("synthetic lock");
        assert_eq!(
            deliveries.as_slice(),
            &[(
                event.id,
                domain(),
                author.public_key(),
                relay.public_key(),
                connection_id,
            )]
        );
        assert!(format!("{permit:?}").starts_with("ClientStatusPresentationPermit"));
    }

    #[tokio::test]
    async fn production_transport_targets_only_the_exact_authenticated_connection() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(14, 14)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let event = issuer
            .issue_withdrawn(
                &evidence(author.public_key()),
                &receipt(author.public_key(), 13),
            )
            .await
            .expect("synthetic withdrawal signs");

        let connections = Arc::new(crate::state::ConnectionManager::new());
        let connection_id = Uuid::new_v4();
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let (ctrl_tx, _ctrl_rx) = tokio::sync::mpsc::channel(2);
        connections.register(
            connection_id,
            tx,
            ctrl_tx,
            CancellationToken::new(),
            domain(),
            Arc::new(AtomicU8::new(0)),
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            3,
        );
        connections
            .set_authenticated_pubkey(connection_id, author.public_key().to_bytes().to_vec());
        let transport = ConnectionManagerClientStatusTransport::new(Arc::clone(&connections));
        transport
            .deliver(DedicatedClientStatusDelivery {
                event: &event,
                relay_pubkey: relay.public_key(),
                authorization_domain: domain(),
                event_author_pubkey: author.public_key(),
                connection_id,
            })
            .expect("exact authenticated connection accepts status");
        let outbound = rx.recv().await.expect("dedicated frame is queued");
        let axum::extract::ws::Message::Text(frame) = outbound.message else {
            panic!("dedicated status must be a text frame");
        };
        assert!(frame.as_str().contains("__buzz_client_binding_status_v1__"));
        assert!(frame.as_str().contains(&event.id.to_string()));

        for (wrong_domain, wrong_author) in [
            (
                CommunityId::from_uuid(Uuid::from_u128(8)),
                author.public_key(),
            ),
            (domain(), Keys::generate().public_key()),
        ] {
            assert_eq!(
                transport.deliver(DedicatedClientStatusDelivery {
                    event: &event,
                    relay_pubkey: relay.public_key(),
                    authorization_domain: wrong_domain,
                    event_author_pubkey: wrong_author,
                    connection_id,
                }),
                Err(DedicatedClientStatusTransportError::Unavailable)
            );
        }
        assert!(rx.try_recv().is_err(), "wrong scopes emit no frame");
    }

    #[tokio::test]
    async fn ambiguous_current_delivery_retains_withdrawable_receipt() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(21, 21)
                .expect("synthetic revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let connection_id = Uuid::new_v4();
        let attempt = issuer
            .deliver_current(
                evidence(author.public_key()),
                None,
                connection_id,
                &FailingDedicatedTransport,
            )
            .await
            .expect("durable allocation and signing succeed");

        assert_eq!(attempt.receipt().connection_id(), connection_id);
        assert_eq!(attempt.receipt().revision(), 21);
        assert_eq!(
            attempt.delivery_error(),
            Some(DedicatedClientStatusTransportError::Unavailable)
        );
        assert_eq!(attempt.event().pubkey, relay.public_key());
    }

    #[tokio::test]
    async fn invalidation_withdraws_every_exact_connection_receipt() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let source = revisions(Some(
            DurableClientStatusRevision::from_durable_state(31, 31)
                .expect("synthetic withdrawal revision is valid"),
        ));
        let privacy_key = privacy_key();
        let issuer = RelayClientBindingStatusIssuer::new(&relay, &source, &privacy_key);
        let permit = ClientStatusPresentationPermit { _private: () };
        let first_connection = Uuid::new_v4();
        let second_connection = Uuid::new_v4();
        let mut first = receipt(author.public_key(), 30);
        first.connection_id = first_connection;
        let mut second = receipt(author.public_key(), 30);
        second.connection_id = second_connection;
        let transport = SyntheticDedicatedTransport {
            deliveries: Mutex::new(Vec::new()),
        };

        issuer
            .deliver_withdrawn_after_invalidation(
                &permit,
                &evidence(author.public_key()),
                &first,
                &transport,
            )
            .await
            .expect("first exact connection withdrawal");
        issuer
            .deliver_withdrawn_after_invalidation(
                &permit,
                &evidence(author.public_key()),
                &second,
                &transport,
            )
            .await
            .expect("second exact connection withdrawal");

        let targets = transport
            .deliveries
            .lock()
            .expect("synthetic lock")
            .iter()
            .map(|delivery| delivery.4)
            .collect::<Vec<_>>();
        assert_eq!(targets, vec![first_connection, second_connection]);
    }

    #[test]
    fn current_issuance_fingerprint_frames_variable_fields() {
        let author = Keys::generate().public_key();
        let mut first = evidence(author);
        first.profile_id =
            AuthorizationProfileId::from_server_configuration("a").expect("first profile");
        first.policy_version = PolicyVersion::new("bc").expect("first policy");
        let mut second = evidence(author);
        second.profile_id =
            AuthorizationProfileId::from_server_configuration("ab").expect("second profile");
        second.policy_version = PolicyVersion::new("c").expect("second policy");

        assert_ne!(
            current_issuance_fingerprint(&first, None),
            current_issuance_fingerprint(&second, None)
        );
    }

    #[test]
    fn no_public_api_can_sign_current_status_without_a_receipt() {
        let source = include_str!("status.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production section exists");
        assert!(!production.contains("pub async fn issue_verification_only"));
        assert!(!production.contains("pub async fn issue_current"));
    }

    #[test]
    fn production_module_has_no_authority_or_delivery_dependency() {
        let source = include_str!("status.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production section exists");
        for forbidden in [
            "AuthContext",
            "AuthorizationLease",
            "CapabilitySet",
            "buzz_pubsub",
            "handlers::",
            "KIND_USER_TRUSTED_ASSERTION",
            "corporate_identity",
        ] {
            assert!(
                !production.contains(forbidden),
                "status adapter gained forbidden dependency {forbidden}"
            );
        }
    }
}
