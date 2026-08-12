//! Canonical atomic final-admission boundary for every protected ingress.
//!
//! Preparation remains read-only. Implementations of
//! [`CanonicalAdmissionCommitter`] own the one transaction that performs the
//! final dependency recheck, optional enrollment, replay claim, receipt, and
//! required authorization-audit append. A failed commit must roll back all of
//! those effects together.

use std::{
    fmt,
    future::{ready, Future},
    pin::Pin,
    sync::{Arc, Mutex},
};

use buzz_auth::{
    AuthoritativeAuthorizationRecheck, AuthorizationError, AuthorizationFinalizationRechecker,
    AuthorizationFinalizer, AuthorizationInput, AuthorizationReason, DirectEnrollmentProposal,
    FinalizedAuthContext, LocalAuthorizationPolicy, LocalBindingResolution, PreparedAuthorization,
    PreparedAuthorizationRecheck, ProofTransport, RouteCapability, VerifiedFederatedAssertion,
    VerifiedNostrProof,
};
use buzz_core::CommunityId;
use chrono::{DateTime, Datelike, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::identity_enrollment::{
    actor_fingerprint, execute_authoritative_enrollment_tx, EnrollmentDisposition,
    IdentityEnrollmentError, PreparedDirectEnrollment,
};

/// Closed protected-object namespaces persisted by canonical admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionObjectKind {
    /// Domain-wide authority.
    Domain,
    /// One channel.
    Channel,
    /// One repository.
    Repository,
    /// One media object.
    Media,
    /// One moderation target.
    ModerationTarget,
    /// One audio session.
    AudioSession,
}

impl AdmissionObjectKind {
    /// Stable database code shared with `protected_object_authority`.
    pub const fn database_code(self) -> i16 {
        match self {
            Self::Domain => 1,
            Self::Channel => 2,
            Self::Repository => 3,
            Self::Media => 4,
            Self::ModerationTarget => 5,
            Self::AudioSession => 6,
        }
    }
}

/// Server-resolved protected object; clients cannot select this coordinate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmissionObject {
    kind: AdmissionObjectKind,
    key: [u8; 32],
}

impl AdmissionObject {
    /// Construct a non-sentinel server-owned object coordinate.
    pub fn new(kind: AdmissionObjectKind, key: [u8; 32]) -> Option<Self> {
        if key == [0; 32] {
            None
        } else {
            Some(Self { kind, key })
        }
    }

    /// Closed object namespace.
    pub const fn kind(self) -> AdmissionObjectKind {
        self.kind
    }

    /// Privacy-safe object key.
    pub const fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

impl fmt::Debug for AdmissionObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionObject([REDACTED])")
    }
}

/// Closed provider-free replay namespaces consumed by final admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionReplayClaimKind {
    /// Opaque digest of one validated trusted-transport nonce.
    TrustedProxyNonce,
}

impl AdmissionReplayClaimKind {
    /// Stable database code reserved across replay-claim migrations.
    pub const fn database_code(self) -> i16 {
        match self {
            Self::TrustedProxyNonce => 1,
        }
    }
}

/// Credential-free one-use replay coordinate sealed by a verifier.
///
/// The key is an opaque domain-separated digest, never the nonce, proof,
/// assertion, subject, issuer, email address, or display data. Chrono has no
/// infinity representation; the constructor additionally limits values to the
/// interoperable finite civil-time range. The canonical transaction must call
/// [`Self::validate_at`] using PostgreSQL transaction time before insertion.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmissionReplayClaim {
    kind: AdmissionReplayClaimKind,
    claim_key: [u8; 32],
    retain_until: DateTime<Utc>,
}

impl AdmissionReplayClaim {
    /// Seal a non-sentinel opaque replay claim and exclusive retention bound.
    pub fn new(
        kind: AdmissionReplayClaimKind,
        claim_key: [u8; 32],
        retain_until: DateTime<Utc>,
    ) -> Result<Self, AdmissionCommitError> {
        if claim_key == [0; 32] || !(1..=9999).contains(&retain_until.year()) {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            kind,
            claim_key,
            retain_until,
        })
    }

    /// Closed replay namespace.
    pub const fn kind(self) -> AdmissionReplayClaimKind {
        self.kind
    }

    /// Opaque domain-separated claim digest.
    pub const fn claim_key(&self) -> &[u8; 32] {
        &self.claim_key
    }

    /// Exclusive storage-retention deadline.
    pub const fn retain_until(self) -> DateTime<Utc> {
        self.retain_until
    }

    /// Reject a stale or non-future claim using authoritative transaction time.
    pub fn validate_at(self, authoritative_now: DateTime<Utc>) -> Result<(), AdmissionCommitError> {
        if authoritative_now >= self.retain_until {
            Err(AdmissionCommitError::ReplayRejected)
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for AdmissionReplayClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionReplayClaim([REDACTED])")
    }
}

enum AdmissionPreparation {
    Existing {
        prepared: Box<PreparedAuthorization>,
    },
    Enrollment {
        correlation_id: Uuid,
        capability: RouteCapability,
        evidence: Box<AdmissionEnrollmentEvidence>,
    },
}

/// Origin-sealed provider-free evidence required for one direct enrollment.
pub struct AdmissionEnrollmentEvidence {
    proposal: DirectEnrollmentProposal,
    enrollment_policy_digest: [u8; 32],
    assertion: VerifiedFederatedAssertion,
    proof: VerifiedNostrProof,
    policy: LocalAuthorizationPolicy,
}

impl AdmissionEnrollmentEvidence {
    /// Seal one proposal/assertion/proof/policy tuple for final admission.
    pub fn new(
        prepared: PreparedDirectEnrollment,
        assertion: VerifiedFederatedAssertion,
        proof: VerifiedNostrProof,
        policy: LocalAuthorizationPolicy,
    ) -> Result<Self, AdmissionCommitError> {
        let (proposal, enrollment_policy_digest) = prepared.into_parts();
        if proposal.authorization_domain() != assertion.authorization_domain()
            || proposal.authorization_domain() != proof.authorization_domain()
            || enrollment_policy_digest == [0; 32]
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            proposal,
            enrollment_policy_digest,
            assertion,
            proof,
            policy,
        })
    }
}

impl fmt::Debug for AdmissionEnrollmentEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionEnrollmentEvidence([REDACTED])")
    }
}

#[derive(Clone, Copy)]
enum AdmissionReceiptShape {
    ProtectedMutation,
    EnrollmentOrProtectedMutation,
}

impl AdmissionReceiptShape {
    fn from_preparation(preparation: &AdmissionPreparation) -> Self {
        match preparation {
            AdmissionPreparation::Existing { .. } => Self::ProtectedMutation,
            AdmissionPreparation::Enrollment { .. } => Self::EnrollmentOrProtectedMutation,
        }
    }

    fn matches(
        self,
        operation_kind: i16,
        receipt_outcome: i16,
        event_kind: Option<i16>,
        event_outcome: Option<i16>,
    ) -> bool {
        if receipt_outcome != 1 || event_outcome != Some(1) {
            return false;
        }
        match self {
            Self::ProtectedMutation => operation_kind == 11 && event_kind == Some(10),
            Self::EnrollmentOrProtectedMutation => {
                (operation_kind == 1 && event_kind == Some(1))
                    || (operation_kind == 11 && event_kind == Some(10))
            }
        }
    }
}

/// Complete sealed input to one canonical final-admission transaction.
///
/// Enrollment carries independently sealed assertion and proof values because
/// the proposal deliberately hides its credential-bearing internals. The
/// transaction uses the public storage coordinates from those clones, then
/// asks the sole authorization finalizer to compare the inserted row with the
/// proposal. A mismatch rolls the transaction back.
pub struct AdmissionCommitRequest {
    attempt_id: Uuid,
    object: AdmissionObject,
    replay_claim: Option<AdmissionReplayClaim>,
    application_effect: Option<Box<dyn AdmissionApplicationEffect>>,
    preparation: AdmissionPreparation,
}

impl AdmissionCommitRequest {
    /// Prepare the commit of an already-resolved authorization.
    pub fn existing(
        attempt_id: Uuid,
        object: AdmissionObject,
        prepared: PreparedAuthorization,
    ) -> Result<Self, AdmissionCommitError> {
        if attempt_id.is_nil() {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            attempt_id,
            object,
            replay_claim: None,
            application_effect: None,
            preparation: AdmissionPreparation::Existing {
                prepared: Box::new(prepared),
            },
        })
    }

    /// Prepare one transactionally created Attested or risk-labelled TOFU
    /// binding and its first committed authorization.
    pub fn enrollment(
        attempt_id: Uuid,
        correlation_id: Uuid,
        object: AdmissionObject,
        capability: RouteCapability,
        evidence: AdmissionEnrollmentEvidence,
    ) -> Result<Self, AdmissionCommitError> {
        if attempt_id.is_nil() || correlation_id.is_nil() {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            attempt_id,
            object,
            replay_claim: None,
            application_effect: None,
            preparation: AdmissionPreparation::Enrollment {
                correlation_id,
                capability,
                evidence: Box::new(evidence),
            },
        })
    }

    /// Deterministic server-owned logical idempotency identifier.
    ///
    /// The caller cannot select this value. It is derived from the complete
    /// domain-separated logical route/application intent, excluding the
    /// attempt-scoped replay claim. A retry with a fresh trusted-proxy nonce
    /// therefore reaches the first canonical result without repeating DML.
    pub fn operation_id(&self) -> Uuid {
        admission_operation_id(self.semantic_fingerprint())
    }

    /// Server-generated identifier for this bounded attempt.
    pub const fn attempt_id(&self) -> Uuid {
        self.attempt_id
    }

    /// Server-resolved protected object.
    pub const fn object(&self) -> AdmissionObject {
        self.object
    }

    /// Attach the optional verifier-sealed claim consumed by final admission.
    pub fn with_replay_claim(mut self, replay_claim: AdmissionReplayClaim) -> Self {
        self.replay_claim = Some(replay_claim);
        self
    }

    /// Optional credential-free claim to consume in the final transaction.
    pub const fn replay_claim(&self) -> Option<AdmissionReplayClaim> {
        self.replay_claim
    }

    /// Attach one credential-free application mutation to the final transaction.
    pub fn with_application_effect(
        mut self,
        application_effect: Box<dyn AdmissionApplicationEffect>,
    ) -> Result<Self, AdmissionCommitError> {
        if application_effect.intent_digest() == [0; 32] {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        self.application_effect = Some(application_effect);
        Ok(self)
    }

    /// Domain sealed by the independently verified evidence.
    pub fn authorization_domain(&self) -> CommunityId {
        match &self.preparation {
            AdmissionPreparation::Existing { prepared } => {
                prepared.recheck_request().lease_dependencies().identity().1
            }
            AdmissionPreparation::Enrollment { evidence, .. } => {
                evidence.proposal.authorization_domain()
            }
        }
    }

    /// Exact request fingerprint used by replay and receipt uniqueness.
    pub fn request_fingerprint(&self) -> [u8; 32] {
        match &self.preparation {
            AdmissionPreparation::Existing { prepared } => {
                let snapshot = prepared.recheck_request().lease_dependencies();
                *snapshot.request_binding().0
            }
            AdmissionPreparation::Enrollment { evidence, .. } => {
                *evidence.proof.request_fingerprint()
            }
        }
    }

    /// Exact provider-free transport binding sealed by the prepared witness.
    ///
    /// Trusted transport adapters must compare both values byte-for-byte with
    /// their independently sealed evidence before attaching a replay claim.
    pub fn transport_binding(&self) -> (ProofTransport, [u8; 32]) {
        match &self.preparation {
            AdmissionPreparation::Existing { prepared } => {
                let snapshot = prepared.recheck_request().lease_dependencies();
                let (_, _, transport, context) = snapshot.request_binding();
                (transport, *context)
            }
            AdmissionPreparation::Enrollment { evidence, .. } => (
                evidence.proof.transport(),
                *evidence.proof.transport_context_fingerprint(),
            ),
        }
    }

    /// Full canonical route/application intent persisted with the receipt.
    pub fn semantic_fingerprint(&self) -> [u8; 32] {
        let domain = self.authorization_domain();
        let request_fingerprint = self.request_fingerprint();
        let (
            capability,
            target_fingerprint,
            transport,
            transport_context,
            actor_pubkey,
            owner_pubkey,
            binding_id,
            binding_version,
            relationship_id,
            relationship_revision,
            relationship_conditions,
            enrollment_revision,
            enrollment_mode,
            enrollment_evidence,
            enrollment_policy_digest,
            preparation_kind,
        ) = match &self.preparation {
            AdmissionPreparation::Existing { prepared } => {
                let snapshot = prepared.recheck_request().lease_dependencies();
                let (capability, actor, owner) = snapshot.authority();
                let (binding_id, binding_version) = snapshot.binding();
                let (request, target, transport, context) = snapshot.request_binding();
                debug_assert_eq!(request, &request_fingerprint);
                let relationship = snapshot.delegated_relationship();
                (
                    capability,
                    *target,
                    transport,
                    *context,
                    actor.to_bytes(),
                    owner.map(|value| value.to_bytes()).unwrap_or([0; 32]),
                    binding_id,
                    binding_version,
                    relationship.map(|value| value.0).unwrap_or(Uuid::nil()),
                    relationship.map(|value| value.1).unwrap_or(0),
                    relationship.map(|value| *value.2).unwrap_or([0; 32]),
                    0,
                    0,
                    [0; 32],
                    [0; 32],
                    1_u8,
                )
            }
            AdmissionPreparation::Enrollment {
                capability,
                evidence,
                ..
            } => {
                let (revision, enrollment_evidence) = evidence.proposal.local_authority();
                (
                    *capability,
                    *evidence.proof.target_fingerprint(),
                    evidence.proof.transport(),
                    *evidence.proof.transport_context_fingerprint(),
                    evidence.proof.actor_pubkey().to_bytes(),
                    [0; 32],
                    Uuid::nil(),
                    0,
                    Uuid::nil(),
                    0,
                    [0; 32],
                    revision,
                    evidence.proposal.mode().database_code(),
                    *enrollment_evidence,
                    evidence.enrollment_policy_digest,
                    2_u8,
                )
            }
        };
        let application_schema = self
            .application_effect
            .as_ref()
            .map(|effect| effect.result_schema());
        let application_type = application_schema
            .map(|schema| *schema.type_key())
            .unwrap_or([0; 32]);
        let application_version = application_schema
            .map(AdmissionApplicationResultSchema::version)
            .unwrap_or(0);
        let application_intent = self
            .application_effect
            .as_ref()
            .map(|effect| effect.intent_digest())
            .unwrap_or([0; 32]);
        admission_framed_digest(
            b"buzz:canonical-admission-semantic-intent:v2",
            &[
                domain.as_uuid().as_bytes(),
                &request_fingerprint,
                &(capability_code(capability)).to_be_bytes(),
                &(self.object.kind().database_code()).to_be_bytes(),
                self.object.key(),
                &target_fingerprint,
                &(proof_transport_code(transport)).to_be_bytes(),
                &transport_context,
                &actor_pubkey,
                &owner_pubkey,
                binding_id.as_bytes(),
                &binding_version.to_be_bytes(),
                relationship_id.as_bytes(),
                &relationship_revision.to_be_bytes(),
                &relationship_conditions,
                &enrollment_revision.to_be_bytes(),
                &enrollment_mode.to_be_bytes(),
                &enrollment_evidence,
                &enrollment_policy_digest,
                &[preparation_kind],
                &application_type,
                &application_version.to_be_bytes(),
                &application_intent,
            ],
        )
    }
}

impl fmt::Debug for AdmissionCommitRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionCommitRequest([REDACTED])")
    }
}

/// Credential-free durable identity of a committed admission.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmissionCommitReceipt {
    authorization_domain: CommunityId,
    object: AdmissionObject,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    semantic_fingerprint: [u8; 32],
    result_digest: [u8; 32],
    application_result_digest: Option<[u8; 32]>,
    audit_event_id: Uuid,
}

/// Closed pair of canonical admission and application-result digests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionCommitDigests {
    result_digest: [u8; 32],
    application_result_digest: Option<[u8; 32]>,
}

impl AdmissionCommitDigests {
    /// Validate the complete admission envelope and optional typed-result binder.
    pub fn new(
        result_digest: [u8; 32],
        application_result_digest: Option<[u8; 32]>,
    ) -> Option<Self> {
        if result_digest == [0; 32] || application_result_digest == Some([0; 32]) {
            None
        } else {
            Some(Self {
                result_digest,
                application_result_digest,
            })
        }
    }
}

impl AdmissionCommitReceipt {
    /// Construct a receipt returned from authoritative storage.
    pub fn from_storage(
        authorization_domain: CommunityId,
        object: AdmissionObject,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        semantic_fingerprint: [u8; 32],
        digests: AdmissionCommitDigests,
        audit_event_id: Uuid,
    ) -> Option<Self> {
        if authorization_domain.as_uuid().is_nil()
            || operation_id.is_nil()
            || request_fingerprint == [0; 32]
            || semantic_fingerprint == [0; 32]
            || audit_event_id.is_nil()
        {
            None
        } else {
            Some(Self {
                authorization_domain,
                object,
                operation_id,
                request_fingerprint,
                semantic_fingerprint,
                result_digest: digests.result_digest,
                application_result_digest: digests.application_result_digest,
                audit_event_id,
            })
        }
    }

    /// Server-owned authorization domain committed by this admission.
    pub const fn authorization_domain(self) -> CommunityId {
        self.authorization_domain
    }

    /// Server-resolved object kind and fingerprint committed by this admission.
    pub const fn object(self) -> AdmissionObject {
        self.object
    }

    /// Server-generated idempotency identifier.
    pub const fn operation_id(self) -> Uuid {
        self.operation_id
    }

    /// Exact request fingerprint committed with the operation.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    /// Complete domain/object/application semantic fingerprint.
    pub const fn semantic_fingerprint(&self) -> &[u8; 32] {
        &self.semantic_fingerprint
    }

    /// Canonical result digest.
    pub const fn result_digest(&self) -> &[u8; 32] {
        &self.result_digest
    }

    /// Separately type/object/domain-bound application artifact digest.
    ///
    /// This is distinct from [`Self::result_digest`], which commits the whole
    /// admission envelope. Application outboxes compare this value to the
    /// context-derived digest before dispatching fresh post-commit work.
    pub const fn application_result_digest(&self) -> Option<&[u8; 32]> {
        self.application_result_digest.as_ref()
    }

    /// Required authorization-audit event.
    pub const fn audit_event_id(self) -> Uuid {
        self.audit_event_id
    }
}

impl fmt::Debug for AdmissionCommitReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionCommitReceipt([REDACTED])")
    }
}

/// Atomic canonical-admission result.
pub enum AdmissionCommitOutcome {
    /// This transaction committed every required authority effect.
    Committed {
        /// Finalized credential-free authorization context.
        authorization: Box<FinalizedAuthContext>,
        /// Durable receipt and audit identity.
        receipt: AdmissionCommitReceipt,
        /// Fresh closed application result, when application DML participated.
        application_result: Option<AdmissionApplicationResult>,
    },
    /// The exact operation and request were committed previously.
    ExactReplay {
        /// Existing durable receipt; no authority effect was repeated.
        receipt: AdmissionCommitReceipt,
        /// Exact typed result reconstructed from immutable canonical storage.
        application_result: Option<AdmissionApplicationResult>,
    },
}

impl fmt::Debug for AdmissionCommitOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Committed { .. } => "AdmissionCommitOutcome::Committed([REDACTED])",
            Self::ExactReplay { .. } => "AdmissionCommitOutcome::ExactReplay([REDACTED])",
        })
    }
}

/// Stable fail-closed final-admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionCommitError {
    /// A server-generated identifier or sealed coordinate was malformed.
    #[error("invalid canonical admission request")]
    InvalidRequest,
    /// Current evidence, binding, lifecycle, or policy denied admission.
    #[error("canonical admission denied")]
    AuthorizationDenied,
    /// The operation identifier was already bound to different semantics.
    #[error("canonical admission intent conflict")]
    IntentConflict,
    /// A proxy nonce or proof replay identity was already consumed.
    #[error("canonical admission replay rejected")]
    ReplayRejected,
    /// Required canonical audit evidence could not be retained.
    #[error("canonical authorization audit unavailable")]
    AuditUnavailable,
    /// Authoritative state or verifier state could not be rechecked.
    #[error("canonical admission dependency unavailable")]
    DependencyUnavailable,
}

/// Closed canonical invite-claim result shared by fresh commit and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalInviteClaimOutcome {
    /// The transaction inserted relay membership and consumed one invite use.
    Joined,
    /// Membership already existed and no invite use was consumed.
    AlreadyMember,
}

impl CanonicalInviteClaimOutcome {
    const fn database_code(self) -> i16 {
        match self {
            Self::Joined => 1,
            Self::AlreadyMember => 2,
        }
    }

    fn from_database(code: i16) -> Result<Self, AdmissionCommitError> {
        match code {
            1 => Ok(Self::Joined),
            2 => Ok(Self::AlreadyMember),
            _ => Err(AdmissionCommitError::DependencyUnavailable),
        }
    }
}

/// Maximum canonical application-result payload retained for exact replay.
pub const MAX_ADMISSION_APPLICATION_RESULT_PAYLOAD_BYTES: usize = 4_096;

const INVITE_CLAIM_RESULT_TYPE: [u8; 32] = [
    0x03, 0x05, 0x7a, 0x5b, 0xd9, 0x5c, 0x3d, 0x4f, 0x9a, 0x89, 0x42, 0x89, 0x33, 0x45, 0xf0, 0xc6,
    0x1e, 0x04, 0x37, 0xbc, 0x43, 0xd1, 0xe4, 0x71, 0x6d, 0xe6, 0xc2, 0xda, 0x2a, 0x39, 0x4e, 0xcd,
];

const PROTECTED_PUBLICATION_RESULT_TYPE: [u8; 32] = [
    0x07, 0xa2, 0x9a, 0x3f, 0x15, 0x45, 0xf0, 0x3b, 0x1b, 0xda, 0x4e, 0xee, 0x5a, 0x25, 0xf0, 0x45,
    0x3f, 0x58, 0x8d, 0x01, 0x81, 0xf8, 0x49, 0x68, 0x87, 0x94, 0x5f, 0xa6, 0xbf, 0x70, 0x0c, 0x9c,
];

const MODERATION_RESULT_TYPE: [u8; 32] = [
    0x1c, 0x3e, 0xdc, 0x9a, 0x25, 0xcd, 0xb1, 0x43, 0x5f, 0x78, 0x99, 0xa1, 0x8d, 0xf6, 0x70, 0x91,
    0x26, 0xf3, 0x20, 0x4f, 0x10, 0x0c, 0x74, 0x93, 0x63, 0x76, 0xf8, 0x14, 0x0d, 0x7b, 0x3f, 0x79,
];

/// Versioned type binding for one bounded canonical application result.
///
/// The type key is a server-owned, domain-separated 32-byte identifier. It is
/// never a provider name or client-controlled discriminator. Schema versions
/// are positive and capped by PostgreSQL `SMALLINT` so storage and callers use
/// one exact representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionApplicationResultSchema {
    type_key: [u8; 32],
    version: u16,
}

impl AdmissionApplicationResultSchema {
    /// Bind an application-owned nonzero type key and positive schema version.
    pub fn new(type_key: [u8; 32], version: u16) -> Option<Self> {
        if type_key == [0; 32] || version == 0 || version > i16::MAX as u16 {
            None
        } else {
            Some(Self { type_key, version })
        }
    }

    /// Stable schema for canonical relay invite results.
    pub const fn invite_claim() -> Self {
        Self {
            type_key: INVITE_CLAIM_RESULT_TYPE,
            version: 1,
        }
    }

    /// Stable schema for canonical protected-publication results.
    pub const fn protected_publication() -> Self {
        Self {
            type_key: PROTECTED_PUBLICATION_RESULT_TYPE,
            version: 1,
        }
    }

    /// Stable schema for canonical moderation results.
    pub const fn moderation() -> Self {
        Self {
            type_key: MODERATION_RESULT_TYPE,
            version: 1,
        }
    }

    /// Opaque server-owned application result type key.
    pub const fn type_key(&self) -> &[u8; 32] {
        &self.type_key
    }

    /// Positive payload schema version.
    pub const fn version(self) -> u16 {
        self.version
    }
}

/// Credential-free typed result returned byte-identically on exact replay.
///
/// Payloads are bounded to 4096 bytes. The canonical transaction persists the
/// type key, schema version, result code, and payload verbatim and covers all
/// four fields with the immutable receipt digest. Application adapters must
/// encode only stable server-derived output, never credentials or mutable
/// request attribution such as uploader names, source addresses, or hosts.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmissionApplicationResult {
    schema: AdmissionApplicationResultSchema,
    code: i16,
    payload: Box<[u8]>,
}

impl AdmissionApplicationResult {
    /// Seal a bounded versioned result for fresh return and durable replay.
    pub fn new(
        schema: AdmissionApplicationResultSchema,
        code: i16,
        payload: impl Into<Box<[u8]>>,
    ) -> Result<Self, AdmissionCommitError> {
        let payload = payload.into();
        if code <= 0 || payload.len() > MAX_ADMISSION_APPLICATION_RESULT_PAYLOAD_BYTES {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            schema,
            code,
            payload,
        })
    }

    /// Encode the closed relay invite result in its shared schema.
    pub fn invite_claim(result: CanonicalInviteClaimOutcome) -> Self {
        Self {
            schema: AdmissionApplicationResultSchema::invite_claim(),
            code: result.database_code(),
            payload: Box::default(),
        }
    }

    /// Decode the closed relay invite result, rejecting every other schema.
    pub fn decode_invite_claim(&self) -> Result<CanonicalInviteClaimOutcome, AdmissionCommitError> {
        if self.schema != AdmissionApplicationResultSchema::invite_claim()
            || !self.payload.is_empty()
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        CanonicalInviteClaimOutcome::from_database(self.code)
            .map_err(|_| AdmissionCommitError::InvalidRequest)
    }

    /// Exact type and version binding for application decoding.
    pub const fn schema(&self) -> AdmissionApplicationResultSchema {
        self.schema
    }

    /// Stable application-defined result code.
    pub const fn code(&self) -> i16 {
        self.code
    }

    /// Exact bounded result bytes persisted by canonical admission.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    fn from_database(
        type_key: Vec<u8>,
        version: i16,
        code: i16,
        payload: Vec<u8>,
    ) -> Result<Self, AdmissionCommitError> {
        let type_key: [u8; 32] = type_key
            .try_into()
            .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
        let version =
            u16::try_from(version).map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
        let schema = AdmissionApplicationResultSchema::new(type_key, version)
            .ok_or(AdmissionCommitError::DependencyUnavailable)?;
        Self::new(schema, code, payload).map_err(|_| AdmissionCommitError::DependencyUnavailable)
    }
}

impl fmt::Debug for AdmissionApplicationResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmissionApplicationResult")
            .field("schema", &self.schema)
            .field("code", &self.code)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

/// Privacy-safe result of a transaction-shareable application mutation.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmissionApplicationOutcome {
    result: AdmissionApplicationResult,
    effect_digest: [u8; 32],
}

impl AdmissionApplicationOutcome {
    /// Seal a typed invite result and non-sentinel application-effect digest.
    pub fn invite_claim(
        result: CanonicalInviteClaimOutcome,
        effect_digest: [u8; 32],
    ) -> Result<Self, AdmissionCommitError> {
        if effect_digest == [0; 32] {
            Err(AdmissionCommitError::InvalidRequest)
        } else {
            Ok(Self {
                result: AdmissionApplicationResult::invite_claim(result),
                effect_digest,
            })
        }
    }

    /// Seal an application-owned bounded result and effect digest.
    pub fn new(
        result: AdmissionApplicationResult,
        effect_digest: [u8; 32],
    ) -> Result<Self, AdmissionCommitError> {
        if effect_digest == [0; 32] {
            Err(AdmissionCommitError::InvalidRequest)
        } else {
            Ok(Self {
                result,
                effect_digest,
            })
        }
    }

    /// Closed credential-free application result.
    pub const fn result(&self) -> &AdmissionApplicationResult {
        &self.result
    }

    /// Canonical credential-free effect digest included in the receipt.
    pub const fn effect_digest(&self) -> &[u8; 32] {
        &self.effect_digest
    }
}

impl fmt::Debug for AdmissionApplicationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionApplicationOutcome([REDACTED])")
    }
}

/// Canonical provider-free coordinates supplied by the transaction owner.
///
/// Application effects cannot choose or capture these values. The committer
/// constructs this context after the final authorization recheck and passes it
/// into the same PostgreSQL transaction used for application DML, receipt,
/// typed result, audit, and authority persistence.
pub struct AdmissionApplicationContext<'a> {
    authorization: &'a FinalizedAuthContext,
    operation_id: Uuid,
    object: AdmissionObject,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
    authoritative_now: DateTime<Utc>,
}

impl<'a> AdmissionApplicationContext<'a> {
    fn new(
        authorization: &'a FinalizedAuthContext,
        operation_id: Uuid,
        object: AdmissionObject,
        semantic_fingerprint: [u8; 32],
        application_intent_digest: [u8; 32],
        authoritative_now: DateTime<Utc>,
    ) -> Result<Self, AdmissionCommitError> {
        if authorization.authorization_domain().as_uuid().is_nil()
            || operation_id.is_nil()
            || semantic_fingerprint == [0; 32]
            || application_intent_digest == [0; 32]
            || authorization.request_fingerprint() == &[0; 32]
            || authorization.lease().request_binding().1 != object.key()
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            authorization,
            operation_id,
            object,
            semantic_fingerprint,
            application_intent_digest,
            authoritative_now,
        })
    }

    /// Exact finalized provider-free authorization.
    pub const fn authorization(&self) -> &FinalizedAuthContext {
        self.authorization
    }

    /// Server-owned authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization.authorization_domain()
    }

    /// Stable server-derived logical operation identifier.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Server-resolved protected object.
    pub const fn object(&self) -> AdmissionObject {
        self.object
    }

    /// Canonical request intent fingerprint.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        self.authorization.request_fingerprint()
    }

    /// Complete domain/object/application semantic fingerprint.
    pub const fn semantic_fingerprint(&self) -> &[u8; 32] {
        &self.semantic_fingerprint
    }

    /// Effect-owned bounded intent digest bound into logical identity.
    pub const fn application_intent_digest(&self) -> &[u8; 32] {
        &self.application_intent_digest
    }

    /// Authoritative PostgreSQL transaction time.
    pub const fn authoritative_now(&self) -> DateTime<Utc> {
        self.authoritative_now
    }

    /// Bind one typed effect outcome to this canonical operation and object.
    pub fn canonical_result_digest(
        &self,
        outcome: &AdmissionApplicationOutcome,
    ) -> Result<[u8; 32], AdmissionCommitError> {
        canonical_application_result_digest(
            self.authorization_domain(),
            self.object,
            self.semantic_fingerprint,
            self.application_intent_digest,
            outcome.result(),
        )
    }
}

impl fmt::Debug for AdmissionApplicationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmissionApplicationContext([REDACTED])")
    }
}

/// Route-owned application DML executed inside canonical final admission.
///
/// Implementations may capture only bounded application intent. Canonical
/// operation/outbox coordinates come exclusively from the supplied context;
/// effects must not capture bearer credentials or substitute domain, object,
/// operation, request, or result-binding values.
pub trait AdmissionApplicationEffect: Send {
    /// Stable server-derived semantic digest used by logical operation identity.
    fn intent_digest(&self) -> [u8; 32];

    /// Exact result type and version produced and persisted by this effect.
    fn result_schema(&self) -> AdmissionApplicationResultSchema;

    /// Apply or idempotently stage the application mutation and return its digest.
    fn apply<'a, 'transaction>(
        &'a mut self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        context: &'a AdmissionApplicationContext<'a>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>>
                + Send
                + 'a,
        >,
    >;
}

/// Canonical relay-invite application mutation executed by final admission.
///
/// The bearer token is already reduced to its opaque fixed digest. Membership
/// identity comes only from the finalized authorization context, never from a
/// caller-supplied subject or public-key string.
pub struct CanonicalInviteClaimEffect {
    token_hash: [u8; 32],
    policy_version: Option<Box<str>>,
    intent_digest: [u8; 32],
}

impl CanonicalInviteClaimEffect {
    /// Bind one opaque invite digest and optional bounded policy version.
    pub fn new(
        token_hash: [u8; 32],
        policy_version: Option<&str>,
    ) -> Result<Self, AdmissionCommitError> {
        if token_hash == [0; 32]
            || policy_version.is_some_and(|version| {
                version.len() != 64 || !version.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        let policy_bytes = policy_version.map(str::as_bytes).unwrap_or_default();
        let intent_digest = admission_framed_digest(
            b"buzz:canonical-invite-claim-intent:v1",
            &[&token_hash, policy_bytes],
        );
        Ok(Self {
            token_hash,
            policy_version: policy_version.map(Into::into),
            intent_digest,
        })
    }
}

impl fmt::Debug for CanonicalInviteClaimEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CanonicalInviteClaimEffect([REDACTED])")
    }
}

impl AdmissionApplicationEffect for CanonicalInviteClaimEffect {
    fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    fn result_schema(&self) -> AdmissionApplicationResultSchema {
        AdmissionApplicationResultSchema::invite_claim()
    }

    fn apply<'a, 'transaction>(
        &'a mut self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        context: &'a AdmissionApplicationContext<'a>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if context.authorization().capability() != RouteCapability::InviteClaim
                || context.object().kind() != AdmissionObjectKind::Domain
            {
                return Err(AdmissionCommitError::AuthorizationDenied);
            }
            apply_canonical_invite_claim_tx(
                transaction,
                context.authorization_domain(),
                &context.authorization().actor_pubkey().to_hex(),
                self.token_hash,
                self.policy_version.as_deref(),
                self.intent_digest,
            )
            .await
        })
    }
}

async fn apply_canonical_invite_claim_tx(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    actor_pubkey: &str,
    token_hash: [u8; 32],
    policy_version: Option<&str>,
    intent_digest: [u8; 32],
) -> Result<AdmissionApplicationOutcome, AdmissionCommitError> {
    if domain.as_uuid().is_nil()
        || actor_pubkey.len() != 64
        || !actor_pubkey.bytes().all(|byte| byte.is_ascii_hexdigit())
        || token_hash == [0; 32]
        || intent_digest == [0; 32]
    {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    let authoritative_now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let invite = sqlx::query(
        r#"
        SELECT id, max_uses, use_count, expires_at
        FROM relay_invites
        WHERE community_id = $1 AND token_hash = $2
        FOR UPDATE
        "#,
    )
    .bind(domain.as_uuid())
    .bind(token_hash.as_slice())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?
    .ok_or(AdmissionCommitError::AuthorizationDenied)?;
    let invite_id: Uuid = invite
        .try_get("id")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let max_uses: Option<i32> = invite
        .try_get("max_uses")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let use_count: i32 = invite
        .try_get("use_count")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let expires_at: DateTime<Utc> = invite
        .try_get("expires_at")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if invite_id.is_nil()
        || use_count < 0
        || max_uses.is_some_and(|maximum| maximum <= 0 || use_count > maximum)
        || expires_at <= authoritative_now
    {
        return Err(AdmissionCommitError::AuthorizationDenied);
    }

    let existing = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
    )
    .bind(domain.as_uuid())
    .bind(actor_pubkey)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?
    .is_some();
    if existing {
        persist_invite_policy_acceptance(transaction, domain, actor_pubkey, policy_version).await?;
        return invite_application_outcome(
            intent_digest,
            CanonicalInviteClaimOutcome::AlreadyMember,
        );
    }
    if max_uses.is_some_and(|maximum| use_count >= maximum) {
        return Err(AdmissionCommitError::AuthorizationDenied);
    }

    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id,pubkey,role,added_by) \
         VALUES ($1,$2,'member','invite') ON CONFLICT (community_id,pubkey) DO NOTHING",
    )
    .bind(domain.as_uuid())
    .bind(actor_pubkey)
    .execute(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?
    .rows_affected();
    persist_invite_policy_acceptance(transaction, domain, actor_pubkey, policy_version).await?;
    if inserted == 0 {
        return invite_application_outcome(
            intent_digest,
            CanonicalInviteClaimOutcome::AlreadyMember,
        );
    }
    if inserted != 1 {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }
    let next_use_count = use_count
        .checked_add(1)
        .ok_or(AdmissionCommitError::DependencyUnavailable)?;
    let updated = sqlx::query(
        "UPDATE relay_invites SET use_count=$1 \
         WHERE community_id=$2 AND id=$3 AND use_count=$4",
    )
    .bind(next_use_count)
    .bind(domain.as_uuid())
    .bind(invite_id)
    .bind(use_count)
    .execute(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if updated.rows_affected() != 1 {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }
    invite_application_outcome(intent_digest, CanonicalInviteClaimOutcome::Joined)
}

async fn persist_invite_policy_acceptance(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    actor_pubkey: &str,
    policy_version: Option<&str>,
) -> Result<(), AdmissionCommitError> {
    if let Some(policy_version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id,pubkey,policy_version) \
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(domain.as_uuid())
        .bind(actor_pubkey)
        .bind(policy_version)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    }
    Ok(())
}

fn invite_application_outcome(
    intent_digest: [u8; 32],
    result: CanonicalInviteClaimOutcome,
) -> Result<AdmissionApplicationOutcome, AdmissionCommitError> {
    let effect_digest = admission_framed_digest(
        b"buzz:canonical-invite-claim-result:v1",
        &[&intent_digest, &result.database_code().to_be_bytes()],
    );
    AdmissionApplicationOutcome::invite_claim(result, effect_digest)
}

/// Transaction-bound final dependency reader configured by composition.
///
/// The implementation atomically reads the complete lease dependency tuple
/// through the supplied transaction and includes the current verifier policy
/// generation. It may not echo the prepared tuple without authoritative reads.
pub trait AdmissionFinalRechecker: Send + Sync {
    /// Observe exact current dependencies while all admission locks are held.
    fn authoritative_recheck<'a, 'transaction>(
        &'a self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        request: &'a PreparedAuthorizationRecheck,
        object: AdmissionObject,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AuthoritativeAuthorizationRecheck, AdmissionCommitError>>
                + Send
                + 'a,
        >,
    >;
}

/// PostgreSQL implementation of the canonical final-admission boundary.
pub struct PostgresCanonicalAdmissionCommitter {
    pool: PgPool,
    rechecker: Arc<dyn AdmissionFinalRechecker>,
}

impl PostgresCanonicalAdmissionCommitter {
    /// Bind canonical admission to the writer pool and configured rechecker.
    pub fn new(pool: PgPool, rechecker: Arc<dyn AdmissionFinalRechecker>) -> Self {
        Self { pool, rechecker }
    }

    async fn commit_inner(
        &self,
        request: AdmissionCommitRequest,
    ) -> Result<AdmissionCommitOutcome, AdmissionCommitError> {
        let domain = request.authorization_domain();
        let operation_id = request.operation_id();
        let semantic_fingerprint = request.semantic_fingerprint();
        let attempt_id = request.attempt_id;
        let object = request.object;
        let request_fingerprint = request.request_fingerprint();
        let receipt_shape = AdmissionReceiptShape::from_preparation(&request.preparation);
        let replay_claim = request.replay_claim;
        let expected_application_schema = request
            .application_effect
            .as_ref()
            .map(|effect| effect.result_schema());
        let application_intent_digest = request
            .application_effect
            .as_ref()
            .map(|effect| effect.intent_digest());
        let mut application_effect = request.application_effect;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;

        lock_operation(&mut transaction, domain, operation_id).await?;
        if let Some(receipt) = read_existing_receipt(
            &mut transaction,
            ExistingAdmissionLookup {
                domain,
                operation_id,
                request_fingerprint,
                semantic_fingerprint,
                object,
                receipt_shape,
                expected_application_schema,
            },
        )
        .await?
        {
            if let Some(claim) = replay_claim {
                let authoritative_now: DateTime<Utc> =
                    sqlx::query_scalar("SELECT transaction_timestamp()")
                        .fetch_one(&mut *transaction)
                        .await
                        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                claim_replay_identity_tx(&mut transaction, domain, claim, authoritative_now)
                    .await?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            } else {
                transaction
                    .rollback()
                    .await
                    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            }
            return Ok(AdmissionCommitOutcome::ExactReplay {
                receipt: receipt.0,
                application_result: receipt.1,
            });
        }

        let authoritative_now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
        if let Some(claim) = replay_claim {
            claim_replay_identity_tx(&mut transaction, domain, claim, authoritative_now).await?;
        }

        let (prepared, enrollment_disposition) = match request.preparation {
            AdmissionPreparation::Existing { prepared } => (*prepared, None),
            AdmissionPreparation::Enrollment {
                correlation_id,
                capability,
                evidence,
            } => {
                let AdmissionEnrollmentEvidence {
                    proposal,
                    enrollment_policy_digest,
                    assertion,
                    proof,
                    policy,
                } = *evidence;
                let enrollment = execute_authoritative_enrollment_tx(
                    &mut transaction,
                    operation_id,
                    request_fingerprint,
                    &proposal,
                    enrollment_policy_digest,
                    &assertion,
                    &proof,
                    authoritative_now,
                )
                .await
                .map_err(map_enrollment_error)?;
                let input = AuthorizationInput::new(domain, correlation_id, proof, capability)
                    .map_err(|_| AdmissionCommitError::AuthorizationDenied)?;
                let resolution = LocalBindingResolution::enrollment(proposal, enrollment.binding);
                let prepared =
                    AuthorizationFinalizer::prepare(input, resolution, policy, authoritative_now)
                        .map_err(|_| AdmissionCommitError::AuthorizationDenied)?;
                (prepared, Some(enrollment.disposition))
            }
        };

        let recheck_request = prepared.recheck_request();
        let observation = self
            .rechecker
            .authoritative_recheck(&mut transaction, &recheck_request, object)
            .await?;
        let one_shot = OneShotRechecker::new(observation);
        let witness = AuthorizationFinalizer::recheck(&prepared, &one_shot)
            .await
            .map_err(|_| AdmissionCommitError::AuthorizationDenied)?;
        let authorization = AuthorizationFinalizer::finalize(prepared, witness)
            .map_err(|_| AdmissionCommitError::AuthorizationDenied)?;

        let application_context = application_intent_digest
            .map(|intent| {
                AdmissionApplicationContext::new(
                    &authorization,
                    operation_id,
                    object,
                    semantic_fingerprint,
                    intent,
                    authoritative_now,
                )
            })
            .transpose()?;
        let application_outcome = match (application_effect.as_mut(), application_context.as_ref())
        {
            (Some(effect), Some(context)) => {
                let outcome = effect.apply(&mut transaction, context).await?;
                let _ = context.canonical_result_digest(&outcome)?;
                Some(outcome)
            }
            (None, None) => None,
            _ => return Err(AdmissionCommitError::DependencyUnavailable),
        };
        if application_outcome
            .as_ref()
            .map(|outcome| outcome.result().schema())
            != expected_application_schema
        {
            return Err(AdmissionCommitError::DependencyUnavailable);
        }

        let receipt = persist_committed_admission(
            &mut transaction,
            operation_id,
            attempt_id,
            object,
            request_fingerprint,
            semantic_fingerprint,
            &authorization,
            enrollment_disposition,
            application_intent_digest,
            application_outcome.as_ref(),
            authoritative_now,
        )
        .await?;
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *transaction)
            .await
            .map_err(map_commit_error)?;
        transaction.commit().await.map_err(map_commit_error)?;
        Ok(AdmissionCommitOutcome::Committed {
            authorization: Box::new(authorization),
            receipt,
            application_result: application_outcome.map(|outcome| outcome.result),
        })
    }
}

async fn claim_replay_identity_tx(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    claim: AdmissionReplayClaim,
    authoritative_now: DateTime<Utc>,
) -> Result<(), AdmissionCommitError> {
    claim.validate_at(authoritative_now)?;
    let claimed = sqlx::query(
        r#"
        INSERT INTO authorization_proxy_nonce_claims (
            authorization_domain, claim_kind, claim_key, committed_at, retain_until
        ) VALUES ($1, $2, $3, transaction_timestamp(), $4)
        ON CONFLICT (authorization_domain, claim_kind, claim_key) DO NOTHING
        "#,
    )
    .bind(domain.as_uuid())
    .bind(claim.kind().database_code())
    .bind(claim.claim_key().as_slice())
    .bind(claim.retain_until())
    .execute(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if claimed.rows_affected() == 1 {
        Ok(())
    } else {
        Err(AdmissionCommitError::ReplayRejected)
    }
}

impl fmt::Debug for PostgresCanonicalAdmissionCommitter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PostgresCanonicalAdmissionCommitter([REDACTED])")
    }
}

/// The sole mutation authority used by every protected ingress.
///
/// Implementations must be provider-free, fail closed, and atomic. They must
/// never retain raw credentials or accept a client-selected authorization
/// domain. The replay claim, optional enrollment, canonical receipt, required
/// audit event, and transaction-shareable application mutation must commit or
/// roll back together. A non-shareable application effect requires an equally
/// request-bound idempotent staging receipt before this method returns.
pub trait CanonicalAdmissionCommitter: Send + Sync {
    /// Recheck and consume one request exactly once.
    fn commit<'a>(
        &'a self,
        request: AdmissionCommitRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<AdmissionCommitOutcome, AdmissionCommitError>> + Send + 'a>,
    >;
}

impl CanonicalAdmissionCommitter for PostgresCanonicalAdmissionCommitter {
    fn commit<'a>(
        &'a self,
        request: AdmissionCommitRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<AdmissionCommitOutcome, AdmissionCommitError>> + Send + 'a>,
    > {
        Box::pin(self.commit_inner(request))
    }
}

struct OneShotRechecker {
    observation: Mutex<Option<AuthoritativeAuthorizationRecheck>>,
}

impl OneShotRechecker {
    fn new(observation: AuthoritativeAuthorizationRecheck) -> Self {
        Self {
            observation: Mutex::new(Some(observation)),
        }
    }
}

impl AuthorizationFinalizationRechecker for OneShotRechecker {
    fn recheck<'a>(
        &'a self,
        _request: &'a PreparedAuthorizationRecheck,
    ) -> impl Future<Output = Result<AuthoritativeAuthorizationRecheck, AuthorizationError>> + Send + 'a
    {
        ready(
            self.observation
                .lock()
                .map_err(|_| AuthorizationError::StaleRecheck)
                .and_then(|mut observation| {
                    observation.take().ok_or(AuthorizationError::StaleRecheck)
                }),
        )
    }
}

async fn lock_operation(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    operation_id: Uuid,
) -> Result<(), AdmissionCommitError> {
    sqlx::query("SELECT nip_fi_lock_authorization_operation_v1($1,$2)")
        .bind(domain.as_uuid())
        .bind(operation_id)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct ExistingAdmissionLookup {
    domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    semantic_fingerprint: [u8; 32],
    object: AdmissionObject,
    receipt_shape: AdmissionReceiptShape,
    expected_application_schema: Option<AdmissionApplicationResultSchema>,
}

async fn read_existing_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    lookup: ExistingAdmissionLookup,
) -> Result<
    Option<(AdmissionCommitReceipt, Option<AdmissionApplicationResult>)>,
    AdmissionCommitError,
> {
    let ExistingAdmissionLookup {
        domain,
        operation_id,
        request_fingerprint,
        semantic_fingerprint,
        object,
        receipt_shape,
        expected_application_schema,
    } = lookup;
    let row = sqlx::query(
        r#"
        SELECT receipt.request_fingerprint, receipt.result_digest,
               receipt.operation_kind, receipt.outcome_code,
               admission.semantic_fingerprint,
               admission.object_kind,
               admission.object_key,
               admission.application_type,
               admission.application_version,
               admission.application_code,
               admission.application_payload,
               admission.application_intent_digest,
               admission.application_effect_digest,
               admission.application_result_digest,
               event.event_id AS audit_event_id,
               event.event_kind AS audit_event_kind,
               event.outcome_code AS audit_outcome_code,
               event.matching_event_count
        FROM authorization_operation_receipts receipt
        LEFT JOIN LATERAL (
            SELECT (array_agg(event_id ORDER BY accepted_at, event_id))[1] AS event_id,
                   min(event_kind) AS event_kind,
                   min(outcome_code) AS outcome_code,
                   count(*) AS matching_event_count
            FROM authorization_events
            WHERE community_id = receipt.community_id
              AND operation_id = receipt.operation_id
              AND request_fingerprint = receipt.request_fingerprint
        ) event ON TRUE
        LEFT JOIN authorization_admission_results admission
          ON admission.community_id = receipt.community_id
         AND admission.operation_id = receipt.operation_id
         AND admission.request_fingerprint = receipt.request_fingerprint
        WHERE receipt.community_id = $1 AND receipt.operation_id = $2
        "#,
    )
    .bind(domain.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let stored_request = fixed_digest(&row, "request_fingerprint")?;
    if stored_request != request_fingerprint {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let stored_semantic = row
        .try_get::<Option<Vec<u8>>, _>("semantic_fingerprint")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?
        .ok_or(AdmissionCommitError::IntentConflict)?;
    if stored_semantic.as_slice() != semantic_fingerprint {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let stored_object_kind: i16 = row
        .try_get("object_kind")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_object_key = row
        .try_get::<Vec<u8>, _>("object_key")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if stored_object_kind != object.kind().database_code()
        || stored_object_key.as_slice() != object.key()
    {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let operation_kind: i16 = row
        .try_get("operation_kind")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let receipt_outcome: i16 = row
        .try_get("outcome_code")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let event_kind: Option<i16> = row
        .try_get("audit_event_kind")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let event_outcome: Option<i16> = row
        .try_get("audit_outcome_code")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let event_count: i64 = row
        .try_get("matching_event_count")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if event_count != 1
        || !receipt_shape.matches(operation_kind, receipt_outcome, event_kind, event_outcome)
    {
        return Err(AdmissionCommitError::IntentConflict);
    }
    let result_digest = fixed_digest(&row, "result_digest")?;
    let audit_event_id: Option<Uuid> = row
        .try_get("audit_event_id")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_type: Option<Vec<u8>> = row
        .try_get("application_type")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_version: Option<i16> = row
        .try_get("application_version")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_code: Option<i16> = row
        .try_get("application_code")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_payload: Option<Vec<u8>> = row
        .try_get("application_payload")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_intent: Option<Vec<u8>> = row
        .try_get("application_intent_digest")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_effect: Option<Vec<u8>> = row
        .try_get("application_effect_digest")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let stored_application_result_digest: Option<Vec<u8>> = row
        .try_get("application_result_digest")
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let (
        application_result,
        stored_application_intent,
        stored_application_effect,
        stored_application_result_digest,
    ) = match (
        expected_application_schema,
        stored_application_type,
        stored_application_version,
        stored_application_code,
        stored_application_payload,
        stored_application_intent,
        stored_application_effect,
        stored_application_result_digest,
    ) {
        (None, None, None, None, None, None, None, None) => (None, None, None, None),
        (
            Some(expected),
            Some(type_key),
            Some(version),
            Some(code),
            Some(payload),
            Some(intent),
            Some(effect),
            Some(result_digest),
        ) if type_key.as_slice() == expected.type_key()
            && i32::from(version) == i32::from(expected.version())
            && intent.len() == 32
            && intent.iter().any(|byte| *byte != 0)
            && effect.len() == 32
            && effect.iter().any(|byte| *byte != 0)
            && result_digest.len() == 32
            && result_digest.iter().any(|byte| *byte != 0) =>
        {
            let intent: [u8; 32] = intent
                .try_into()
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            let effect: [u8; 32] = effect
                .try_into()
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            let result_digest: [u8; 32] = result_digest
                .try_into()
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            (
                Some(AdmissionApplicationResult::from_database(
                    type_key, version, code, payload,
                )?),
                Some(intent),
                Some(effect),
                Some(result_digest),
            )
        }
        _ => return Err(AdmissionCommitError::IntentConflict),
    };
    let calculated_application_result_digest =
        match (stored_application_intent, application_result.as_ref()) {
            (Some(intent), Some(result)) => Some(canonical_application_result_digest(
                domain,
                object,
                semantic_fingerprint,
                intent,
                result,
            )?),
            (None, None) => None,
            _ => return Err(AdmissionCommitError::DependencyUnavailable),
        };
    if calculated_application_result_digest != stored_application_result_digest {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }
    let calculated_result_digest = canonical_admission_result_digest(
        domain,
        object,
        semantic_fingerprint,
        stored_application_intent,
        application_result.as_ref(),
        stored_application_effect,
    )
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if calculated_result_digest != result_digest {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }
    let receipt = AdmissionCommitReceipt::from_storage(
        domain,
        object,
        operation_id,
        stored_request,
        semantic_fingerprint,
        AdmissionCommitDigests::new(result_digest, stored_application_result_digest)
            .ok_or(AdmissionCommitError::DependencyUnavailable)?,
        audit_event_id.ok_or(AdmissionCommitError::DependencyUnavailable)?,
    )
    .ok_or(AdmissionCommitError::DependencyUnavailable)?;
    Ok(Some((receipt, application_result)))
}

#[allow(clippy::too_many_arguments)]
async fn persist_committed_admission(
    transaction: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    attempt_id: Uuid,
    object: AdmissionObject,
    request_fingerprint: [u8; 32],
    semantic_fingerprint: [u8; 32],
    authorization: &FinalizedAuthContext,
    enrollment_disposition: Option<EnrollmentDisposition>,
    application_intent_digest: Option<[u8; 32]>,
    application_outcome: Option<&AdmissionApplicationOutcome>,
    authoritative_now: DateTime<Utc>,
) -> Result<AdmissionCommitReceipt, AdmissionCommitError> {
    if authorization.authorization_domain().as_uuid().is_nil()
        || authorization.authorization_domain() != authorization.lease().authorization_domain()
        || authorization.request_fingerprint() != &request_fingerprint
        || authorization.lease().request_binding().1 != object.key()
        || semantic_fingerprint == [0; 32]
        || application_intent_digest.is_some() != application_outcome.is_some()
    {
        return Err(AdmissionCommitError::AuthorizationDenied);
    }
    let domain = authorization.authorization_domain();
    let actor_bytes = authorization.actor_pubkey().to_bytes();
    let actor_digest = actor_fingerprint(domain, &actor_bytes);
    let reason_code = reason_code(authorization.reason());
    let event_kind = if enrollment_disposition == Some(EnrollmentDisposition::Created) {
        1_i16
    } else {
        10_i16
    };
    let operation_kind = if enrollment_disposition == Some(EnrollmentDisposition::Created) {
        1_i16
    } else {
        11_i16
    };
    let actor_kind = if authorization.owner_pubkey().is_some() {
        2_i16
    } else {
        1_i16
    };
    let application_result_digest = match (application_intent_digest, application_outcome) {
        (Some(intent), Some(outcome)) => Some(canonical_application_result_digest(
            domain,
            object,
            semantic_fingerprint,
            intent,
            outcome.result(),
        )?),
        (None, None) => None,
        _ => return Err(AdmissionCommitError::InvalidRequest),
    };
    let result_digest = canonical_admission_result_digest(
        domain,
        object,
        semantic_fingerprint,
        application_intent_digest,
        application_outcome.map(AdmissionApplicationOutcome::result),
        application_outcome.map(|outcome| *outcome.effect_digest()),
    )?;
    let envelope = serde_json::to_vec(&serde_json::json!({
        "schema": 1,
        "event_kind": event_kind,
        "outcome": 1,
        "reason": reason_code,
        "operation_id": operation_id,
        "request": hex::encode(request_fingerprint),
        "actor": hex::encode(actor_digest),
        "object_kind": object.kind().database_code(),
        "object": hex::encode(object.key()),
        "binding_id": authorization.binding().0,
        "binding_version": authorization.binding().1,
        "capability": capability_code(authorization.capability()),
        "admission_intent": hex::encode(semantic_fingerprint),
        "application_type": application_outcome
            .map(|outcome| hex::encode(outcome.result().schema().type_key())),
        "application_version": application_outcome
            .map(|outcome| outcome.result().schema().version()),
        "application_code": application_outcome
            .map(|outcome| outcome.result().code()),
        "application_payload_digest": application_outcome
            .map(|outcome| hex::encode(Sha256::digest(outcome.result().payload()))),
        "application_result_digest": application_result_digest.map(hex::encode),
        "application_intent": application_intent_digest.map(hex::encode),
        "application_effect": application_outcome
            .map(|outcome| hex::encode(outcome.effect_digest())),
    }))
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    let audit_envelope_digest: [u8; 32] = Sha256::digest(&envelope).into();

    sqlx::query(
        r#"
        INSERT INTO authorization_operation_receipts (
            community_id, operation_id, request_fingerprint, operation_kind,
            actor_fingerprint, outcome_code, result_digest
        ) VALUES ($1, $2, $3, $4, $5, 1, $6)
        "#,
    )
    .bind(domain.as_uuid())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(operation_kind)
    .bind(actor_digest.as_slice())
    .bind(result_digest.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(map_receipt_error)?;

    let event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO authorization_events (
            community_id, event_id, event_kind, outcome_code, reason_code,
            actor_kind, actor_fingerprint, subject_fingerprint,
            operation_id, request_fingerprint, correlation_id, attempt_id,
            occurred_at, canonical_envelope, envelope_digest
        ) VALUES ($1, $2, $3, 1, $4, $5, $6, NULL, $7, $8, $9, $10, $11, $12, $13)
        "#,
    )
    .bind(domain.as_uuid())
    .bind(event_id)
    .bind(event_kind)
    .bind(reason_code)
    .bind(actor_kind)
    .bind(actor_digest.as_slice())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(authorization.correlation_id())
    .bind(attempt_id)
    .bind(authoritative_now)
    .bind(envelope.as_slice())
    .bind(audit_envelope_digest.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(map_audit_error)?;

    sqlx::query(
        r#"
        INSERT INTO authorization_admission_results (
            community_id, operation_id, request_fingerprint,
            semantic_fingerprint, object_kind, object_key, application_type,
            application_version, application_code, application_payload,
            application_intent_digest, application_effect_digest,
            application_result_digest
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        "#,
    )
    .bind(domain.as_uuid())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(semantic_fingerprint.as_slice())
    .bind(object.kind().database_code())
    .bind(object.key().as_slice())
    .bind(application_outcome.map(|outcome| outcome.result().schema().type_key().to_vec()))
    .bind(application_outcome.map(|outcome| i32::from(outcome.result().schema().version()) as i16))
    .bind(application_outcome.map(|outcome| outcome.result().code()))
    .bind(application_outcome.map(|outcome| outcome.result().payload().to_vec()))
    .bind(application_intent_digest.map(|digest| digest.to_vec()))
    .bind(application_outcome.map(|outcome| outcome.effect_digest().to_vec()))
    .bind(application_result_digest.map(|digest| digest.to_vec()))
    .execute(&mut **transaction)
    .await
    .map_err(map_receipt_error)?;

    persist_protected_authority(
        transaction,
        operation_id,
        object,
        request_fingerprint,
        authorization,
    )
    .await?;
    AdmissionCommitReceipt::from_storage(
        domain,
        object,
        operation_id,
        request_fingerprint,
        semantic_fingerprint,
        AdmissionCommitDigests::new(result_digest, application_result_digest)
            .ok_or(AdmissionCommitError::DependencyUnavailable)?,
        event_id,
    )
    .ok_or(AdmissionCommitError::DependencyUnavailable)
}

async fn persist_protected_authority(
    transaction: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    object: AdmissionObject,
    request_fingerprint: [u8; 32],
    authorization: &FinalizedAuthContext,
) -> Result<(), AdmissionCommitError> {
    let lease = authorization.lease();
    let domain = authorization.authorization_domain();
    let (policy_revision, invalidation_generation, authority_epoch) = lease.dependency_versions();
    let fence = lease.fence();
    let epoch_write = sqlx::query(
        r#"
        INSERT INTO authorization_authority_epochs (
            community_id, object_kind, object_key, authority_epoch, fence,
            operation_id, request_fingerprint
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (community_id, object_kind, object_key) DO UPDATE SET
            authority_epoch = EXCLUDED.authority_epoch,
            fence = EXCLUDED.fence,
            operation_id = EXCLUDED.operation_id,
            request_fingerprint = EXCLUDED.request_fingerprint,
            updated_at = GREATEST(
                transaction_timestamp(),
                authorization_authority_epochs.updated_at + INTERVAL '1 microsecond'
            )
        WHERE authorization_authority_epochs.authority_epoch < EXCLUDED.authority_epoch
        "#,
    )
    .bind(domain.as_uuid())
    .bind(object.kind().database_code())
    .bind(object.key().as_slice())
    .bind(to_i64(authority_epoch)?)
    .bind(fence.as_bytes().as_slice())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if epoch_write.rows_affected() == 0 {
        let exact_epoch: Option<bool> = sqlx::query_scalar(
            r#"
            SELECT authority_epoch = $4 AND fence = $5
            FROM authorization_authority_epochs
            WHERE community_id = $1 AND object_kind = $2 AND object_key = $3
            FOR UPDATE
            "#,
        )
        .bind(domain.as_uuid())
        .bind(object.kind().database_code())
        .bind(object.key().as_slice())
        .bind(to_i64(authority_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
        if exact_epoch != Some(true) {
            return Err(AdmissionCommitError::AuthorizationDenied);
        }
    } else if epoch_write.rows_affected() != 1 {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }

    let (binding_id, binding_version) = authorization.binding();
    let relationship = lease.delegated_relationship();
    let dependency_snapshot = lease.dependency_snapshot();
    let conditions = dependency_snapshot
        .delegated_relationship()
        .map(|(_, _, conditions)| conditions.to_vec());
    let actor_pubkey = authorization.actor_pubkey().to_bytes();
    let authority_write = sqlx::query(
        r#"
        INSERT INTO protected_object_authority (
            community_id, object_kind, object_key, capability, actor_pubkey,
            owner_pubkey, binding_id, binding_version,
            delegated_relationship_id, delegated_relationship_revision,
            delegation_conditions_fingerprint, policy_revision,
            invalidation_generation, authority_epoch, fence, issued_at,
            expires_at, operation_id, request_fingerprint
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
            $14, $15, $16, $17, $18, $19
        )
        ON CONFLICT (community_id, object_kind, object_key) DO UPDATE SET
            capability = EXCLUDED.capability,
            actor_pubkey = EXCLUDED.actor_pubkey,
            owner_pubkey = EXCLUDED.owner_pubkey,
            binding_id = EXCLUDED.binding_id,
            binding_version = EXCLUDED.binding_version,
            delegated_relationship_id = EXCLUDED.delegated_relationship_id,
            delegated_relationship_revision = EXCLUDED.delegated_relationship_revision,
            delegation_conditions_fingerprint = EXCLUDED.delegation_conditions_fingerprint,
            policy_revision = EXCLUDED.policy_revision,
            invalidation_generation = EXCLUDED.invalidation_generation,
            authority_epoch = EXCLUDED.authority_epoch,
            fence = EXCLUDED.fence,
            issued_at = EXCLUDED.issued_at,
            expires_at = EXCLUDED.expires_at,
            operation_id = EXCLUDED.operation_id,
            request_fingerprint = EXCLUDED.request_fingerprint
        WHERE protected_object_authority.authority_epoch < EXCLUDED.authority_epoch
        "#,
    )
    .bind(domain.as_uuid())
    .bind(object.kind().database_code())
    .bind(object.key().as_slice())
    .bind(capability_code(authorization.capability()))
    .bind(actor_pubkey.as_slice())
    .bind(
        authorization
            .owner_pubkey()
            .map(|key| key.to_bytes().to_vec()),
    )
    .bind(binding_id)
    .bind(to_i64(binding_version)?)
    .bind(relationship.map(|value| value.0))
    .bind(relationship.map(|value| to_i64(value.1)).transpose()?)
    .bind(conditions)
    .bind(to_i64(policy_revision)?)
    .bind(to_i64_allow_zero(invalidation_generation)?)
    .bind(to_i64(authority_epoch)?)
    .bind(fence.as_bytes().as_slice())
    .bind(lease.issued_at())
    .bind(lease.expires_at())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    if authority_write.rows_affected() == 0 {
        let exact_authority: Option<bool> = sqlx::query_scalar(
            r#"
            SELECT capability = $4
               AND actor_pubkey = $5
               AND owner_pubkey IS NOT DISTINCT FROM $6
               AND binding_id = $7
               AND binding_version = $8
               AND delegated_relationship_id IS NOT DISTINCT FROM $9
               AND delegated_relationship_revision IS NOT DISTINCT FROM $10
               AND delegation_conditions_fingerprint IS NOT DISTINCT FROM $11
               AND policy_revision = $12
               AND invalidation_generation = $13
               AND authority_epoch = $14
               AND fence = $15
            FROM protected_object_authority
            WHERE community_id = $1 AND object_kind = $2 AND object_key = $3
            FOR UPDATE
            "#,
        )
        .bind(domain.as_uuid())
        .bind(object.kind().database_code())
        .bind(object.key().as_slice())
        .bind(capability_code(authorization.capability()))
        .bind(actor_pubkey.as_slice())
        .bind(
            authorization
                .owner_pubkey()
                .map(|key| key.to_bytes().to_vec()),
        )
        .bind(binding_id)
        .bind(to_i64(binding_version)?)
        .bind(relationship.map(|value| value.0))
        .bind(relationship.map(|value| to_i64(value.1)).transpose()?)
        .bind(
            dependency_snapshot
                .delegated_relationship()
                .map(|(_, _, value)| value.to_vec()),
        )
        .bind(to_i64(policy_revision)?)
        .bind(to_i64_allow_zero(invalidation_generation)?)
        .bind(to_i64(authority_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
        if exact_authority != Some(true) {
            return Err(AdmissionCommitError::AuthorizationDenied);
        }
    } else if authority_write.rows_affected() != 1 {
        return Err(AdmissionCommitError::DependencyUnavailable);
    }
    Ok(())
}

fn reason_code(reason: AuthorizationReason) -> i16 {
    match reason {
        AuthorizationReason::ExistingBinding => 1,
        AuthorizationReason::EnrolledAttestedKey => 2,
        AuthorizationReason::EnrolledProvisioned => 3,
        AuthorizationReason::EnrolledRiskLabelledTofu => 4,
        AuthorizationReason::DelegatedOwnerBinding => 5,
    }
}

fn capability_code(capability: RouteCapability) -> i16 {
    match capability {
        RouteCapability::MessagesRead => 1,
        RouteCapability::MessagesWrite => 2,
        RouteCapability::ChannelsRead => 3,
        RouteCapability::ChannelsWrite => 4,
        RouteCapability::AdminChannels => 5,
        RouteCapability::UsersRead => 6,
        RouteCapability::UsersWrite => 7,
        RouteCapability::AdminUsers => 8,
        RouteCapability::JobsRead => 9,
        RouteCapability::JobsWrite => 10,
        RouteCapability::SubscriptionsRead => 11,
        RouteCapability::SubscriptionsWrite => 12,
        RouteCapability::FilesRead => 13,
        RouteCapability::FilesWrite => 14,
        RouteCapability::ReposRead => 15,
        RouteCapability::ReposWrite => 16,
        RouteCapability::GitRead => 17,
        RouteCapability::GitWrite => 18,
        RouteCapability::GitStream => 19,
        RouteCapability::MediaRead => 20,
        RouteCapability::MediaWrite => 21,
        RouteCapability::Moderation => 22,
        RouteCapability::AudioJoin => 23,
        RouteCapability::AudioMedia => 24,
        RouteCapability::Discovery => 25,
        RouteCapability::BindingStatus => 26,
        RouteCapability::Enrollment => 27,
        RouteCapability::InviteMint => 28,
        RouteCapability::InviteClaim => 29,
    }
}

const fn proof_transport_code(transport: ProofTransport) -> i16 {
    match transport {
        ProofTransport::Nip42 => 1,
        ProofTransport::Nip98 => 2,
        ProofTransport::GitSmartHttpSession => 3,
        ProofTransport::Blossom => 4,
    }
}

fn fixed_digest(
    row: &sqlx::postgres::PgRow,
    column: &str,
) -> Result<[u8; 32], AdmissionCommitError> {
    let bytes: Vec<u8> = row
        .try_get(column)
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
    bytes
        .try_into()
        .map_err(|_| AdmissionCommitError::DependencyUnavailable)
}

fn admission_framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

fn admission_operation_id(semantic_fingerprint: [u8; 32]) -> Uuid {
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&semantic_fingerprint[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn canonical_admission_result_digest(
    domain: CommunityId,
    object: AdmissionObject,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: Option<[u8; 32]>,
    application_result: Option<&AdmissionApplicationResult>,
    application_effect_digest: Option<[u8; 32]>,
) -> Result<[u8; 32], AdmissionCommitError> {
    let valid_shape = match (
        application_intent_digest,
        application_result,
        application_effect_digest,
    ) {
        (None, None, None) => true,
        (Some(intent), Some(_), Some(effect)) => intent != [0; 32] && effect != [0; 32],
        _ => false,
    };
    if semantic_fingerprint == [0; 32] || !valid_shape {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    let intent = application_intent_digest.unwrap_or([0; 32]);
    let effect = application_effect_digest.unwrap_or([0; 32]);
    let application_result_digest = match (application_intent_digest, application_result) {
        (Some(intent), Some(result)) => canonical_application_result_digest(
            domain,
            object,
            semantic_fingerprint,
            intent,
            result,
        )?,
        (None, None) => [0; 32],
        _ => return Err(AdmissionCommitError::InvalidRequest),
    };
    Ok(admission_framed_digest(
        b"buzz:canonical-admission-result:v2",
        &[
            &semantic_fingerprint,
            &intent,
            &effect,
            &application_result_digest,
        ],
    ))
}

fn canonical_application_result_digest(
    domain: CommunityId,
    object: AdmissionObject,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
    result: &AdmissionApplicationResult,
) -> Result<[u8; 32], AdmissionCommitError> {
    if domain.as_uuid().is_nil()
        || semantic_fingerprint == [0; 32]
        || application_intent_digest == [0; 32]
    {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    Ok(admission_framed_digest(
        b"buzz:canonical-application-result:v1",
        &[
            domain.as_uuid().as_bytes(),
            &object.kind().database_code().to_be_bytes(),
            object.key(),
            &semantic_fingerprint,
            &application_intent_digest,
            result.schema().type_key(),
            &result.schema().version().to_be_bytes(),
            &result.code().to_be_bytes(),
            result.payload(),
        ],
    ))
}

fn to_i64(value: u64) -> Result<i64, AdmissionCommitError> {
    i64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(AdmissionCommitError::DependencyUnavailable)
}

fn to_i64_allow_zero(value: u64) -> Result<i64, AdmissionCommitError> {
    i64::try_from(value).map_err(|_| AdmissionCommitError::DependencyUnavailable)
}

fn map_enrollment_error(error: IdentityEnrollmentError) -> AdmissionCommitError {
    match error {
        IdentityEnrollmentError::Denied | IdentityEnrollmentError::ProvisioningRequired => {
            AdmissionCommitError::AuthorizationDenied
        }
        IdentityEnrollmentError::InvalidInput => AdmissionCommitError::InvalidRequest,
        IdentityEnrollmentError::PolicyUnavailable
        | IdentityEnrollmentError::StorageUnavailable => {
            AdmissionCommitError::DependencyUnavailable
        }
    }
}

fn map_receipt_error(error: sqlx::Error) -> AdmissionCommitError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        AdmissionCommitError::IntentConflict
    } else {
        AdmissionCommitError::DependencyUnavailable
    }
}

fn map_audit_error(error: sqlx::Error) -> AdmissionCommitError {
    let constraint = error
        .as_database_error()
        .and_then(|database| database.constraint());
    if matches!(
        constraint,
        Some(
            "authorization_event_capacity_policy_required"
                | "authorization_event_capacity_health"
                | "authorization_event_capacity_exhausted"
                | "authorization_events_envelope_size"
        )
    ) {
        AdmissionCommitError::AuditUnavailable
    } else {
        AdmissionCommitError::DependencyUnavailable
    }
}

fn map_commit_error(error: sqlx::Error) -> AdmissionCommitError {
    let constraint = error
        .as_database_error()
        .and_then(|database| database.constraint());
    if matches!(
        constraint,
        Some(
            "authorization_event_capacity_policy_required"
                | "authorization_event_capacity_health"
                | "authorization_event_capacity_exhausted"
                | "authorization_events_envelope_size"
        )
    ) {
        AdmissionCommitError::AuditUnavailable
    } else {
        AdmissionCommitError::DependencyUnavailable
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        path::PathBuf,
        process::{Command, Stdio},
    };

    use buzz_auth::{
        nip42::verify_nip42_authorization_proof, CanonicalFederatedAssertionVerifier,
        CanonicalVerifierKeySet, CanonicalVerifierPolicy, VerifierKeyGeneration,
    };
    use buzz_core::AuthorizationLeaseFence;
    use chrono::TimeDelta;
    use nostr::{EventBuilder, Keys, RelayUrl};

    use super::*;

    fn loopback_test_database_url() -> String {
        format!(
            "{}://{}:{}@{}:{}/{}",
            "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
        )
    }

    fn base64_url(input: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let first = chunk[0];
            let second = chunk.get(1).copied().unwrap_or(0);
            let third = chunk.get(2).copied().unwrap_or(0);
            output.push(ALPHABET[(first >> 2) as usize] as char);
            output.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
            if chunk.len() > 1 {
                output.push(ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
            }
            if chunk.len() > 2 {
                output.push(ALPHABET[(third & 0x3f) as usize] as char);
            }
        }
        output
    }

    struct EphemeralRsaKey {
        path: PathBuf,
    }

    impl EphemeralRsaKey {
        fn generate(kid: &str, prefix: &str) -> (Self, serde_json::Value) {
            let key = Self {
                path: std::env::temp_dir().join(format!("{prefix}-{}.pem", Uuid::new_v4())),
            };
            let generated = Command::new("openssl")
                .args([
                    "genpkey",
                    "-algorithm",
                    "RSA",
                    "-pkeyopt",
                    "rsa_keygen_bits:2048",
                    "-pkeyopt",
                    "rsa_keygen_pubexp:65537",
                    "-out",
                ])
                .arg(&key.path)
                .output()
                .expect("generate ephemeral RSA test key");
            assert!(
                generated.status.success(),
                "OpenSSL RSA key generation failed: {}",
                String::from_utf8_lossy(&generated.stderr)
            );
            let modulus = Command::new("openssl")
                .args(["rsa", "-in"])
                .arg(&key.path)
                .args(["-noout", "-modulus"])
                .output()
                .expect("read ephemeral RSA test modulus");
            assert!(
                modulus.status.success(),
                "OpenSSL RSA modulus extraction failed: {}",
                String::from_utf8_lossy(&modulus.stderr)
            );
            let modulus = std::str::from_utf8(&modulus.stdout)
                .expect("UTF-8 RSA test modulus")
                .trim()
                .strip_prefix("Modulus=")
                .expect("OpenSSL RSA modulus prefix");
            let modulus = hex::decode(modulus).expect("hex RSA test modulus");
            let jwks = serde_json::json!({
                "keys": [{
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": kid,
                    "n": base64_url(&modulus),
                    "e": "AQAB",
                }],
            });
            (key, jwks)
        }
    }

    impl Drop for EphemeralRsaKey {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn signed_test_jwt(subject: &str, event_author: [u8; 32]) -> (String, serde_json::Value) {
        let issued_at = Utc::now().timestamp() - 1;
        let claims = serde_json::json!({
            "iss": "https://s3-verifier.test",
            "aud": "buzz-s3-test",
            "sub": subject,
            "event_author": hex::encode(event_author),
            "iat": issued_at,
            "nbf": issued_at,
            "exp": issued_at + 600,
        });
        let header = serde_json::json!({ "alg": "RS256", "kid": "s3-test", "typ": "JWT" });
        let signing_input = format!(
            "{}.{}",
            base64_url(&serde_json::to_vec(&header).expect("serialize JWT header")),
            base64_url(&serde_json::to_vec(&claims).expect("serialize JWT claims")),
        );
        let (key, jwks) = EphemeralRsaKey::generate("s3-test", "buzz-s3-jwt");
        let mut child = Command::new("openssl")
            .args(["dgst", "-sha256", "-sign"])
            .arg(&key.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn OpenSSL test signer");
        child
            .stdin
            .as_mut()
            .expect("OpenSSL signer stdin")
            .write_all(signing_input.as_bytes())
            .expect("write JWT signing input");
        let signed = child.wait_with_output().expect("wait for JWT signer");
        assert!(signed.status.success(), "OpenSSL JWT signer failed");
        (
            format!("{signing_input}.{}", base64_url(&signed.stdout)),
            jwks,
        )
    }

    fn verified_enrollment_evidence(
        domain: CommunityId,
        keys: &Keys,
        subject: &str,
        target: [u8; 32],
        request: [u8; 32],
        transport_context: [u8; 32],
    ) -> (VerifiedFederatedAssertion, VerifiedNostrProof) {
        let (token, jwks) = signed_test_jwt(subject, keys.public_key().to_bytes());
        let verifier = CanonicalFederatedAssertionVerifier::new(
            CanonicalVerifierPolicy::new(
                "https://s3-verifier.test".to_owned(),
                "buzz-s3-test".to_owned(),
                "sub".to_owned(),
                Some("event_author".to_owned()),
                5,
                600,
            )
            .expect("canonical verifier policy"),
        );
        let key_set = CanonicalVerifierKeySet::new(
            VerifierKeyGeneration::new(1).expect("positive verifier generation"),
            serde_json::from_value(jwks).expect("parse generated test JWKS"),
        );
        let assertion = verifier
            .verify(
                &token,
                &key_set,
                domain,
                ProofTransport::Nip42,
                target,
                request,
                transport_context,
            )
            .expect("verify test assertion");
        let challenge = hex::encode([81_u8; 32]);
        let relay = "wss://s3-admission.test";
        let event = EventBuilder::auth(
            challenge.clone(),
            RelayUrl::parse(relay).expect("test relay URL"),
        )
        .sign_with_keys(keys)
        .expect("sign NIP-42 test proof");
        let assertion_fingerprint: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let proof = verify_nip42_authorization_proof(
            &event,
            &challenge,
            relay,
            domain,
            request,
            target,
            transport_context,
            Some(assertion_fingerprint),
            None,
            Utc::now() + TimeDelta::minutes(5),
        )
        .expect("verify NIP-42 authorization proof");
        (assertion, proof)
    }

    struct TestPostgresAdmissionRechecker;

    impl AdmissionFinalRechecker for TestPostgresAdmissionRechecker {
        fn authoritative_recheck<'a, 'transaction>(
            &'a self,
            transaction: &'a mut Transaction<'transaction, Postgres>,
            request: &'a PreparedAuthorizationRecheck,
            object: AdmissionObject,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<AuthoritativeAuthorizationRecheck, AdmissionCommitError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let snapshot = request.lease_dependencies();
                let (_, domain) = snapshot.identity();
                let (binding_id, binding_version) = snapshot.binding();
                let (_, target, _, _) = snapshot.request_binding();
                let (policy_revision, invalidation_generation, _) = snapshot.dependency_versions();
                if target != object.key() {
                    return Err(AdmissionCommitError::AuthorizationDenied);
                }
                let binding_current: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM identity_bindings \
                     WHERE community_id=$1 AND binding_id=$2 AND binding_version=$3 \
                       AND binding_state=1 AND lifecycle_revision=1 \
                       AND (expires_at IS NULL OR transaction_timestamp() < expires_at))",
                )
                .bind(domain.as_uuid())
                .bind(binding_id)
                .bind(
                    i64::try_from(binding_version)
                        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?,
                )
                .fetch_one(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                let policy_current: Option<i64> = sqlx::query_scalar(
                    "SELECT max(policy_revision) FROM identity_enrollment_policies \
                     WHERE community_id=$1 AND effective_at <= transaction_timestamp() \
                       AND (expires_at IS NULL OR transaction_timestamp() < expires_at)",
                )
                .bind(domain.as_uuid())
                .fetch_one(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                let generation: Option<i64> = sqlx::query_scalar(
                    "SELECT current_generation FROM authorization_invalidation_domains \
                     WHERE community_id=$1 FOR SHARE",
                )
                .bind(domain.as_uuid())
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                if !binding_current
                    || policy_current != i64::try_from(policy_revision).ok()
                    || generation != i64::try_from(invalidation_generation).ok()
                {
                    return Err(AdmissionCommitError::AuthorizationDenied);
                }
                let authoritative_now: DateTime<Utc> =
                    sqlx::query_scalar("SELECT transaction_timestamp()")
                        .fetch_one(&mut **transaction)
                        .await
                        .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                Ok(AuthoritativeAuthorizationRecheck::from_authoritative_parts(
                    snapshot,
                    request.verifier_stamp(),
                    authoritative_now,
                ))
            })
        }
    }

    struct TestMarkerEffect {
        intent_digest: [u8; 32],
        fail_after_write: bool,
    }

    impl AdmissionApplicationEffect for TestMarkerEffect {
        fn intent_digest(&self) -> [u8; 32] {
            self.intent_digest
        }

        fn result_schema(&self) -> AdmissionApplicationResultSchema {
            AdmissionApplicationResultSchema::new([91; 32], 1).expect("test result schema")
        }

        fn apply<'a, 'transaction>(
            &'a mut self,
            transaction: &'a mut Transaction<'transaction, Postgres>,
            context: &'a AdmissionApplicationContext<'a>,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let result = AdmissionApplicationResult::new(
                    self.result_schema(),
                    1,
                    b"stable-marker".to_vec(),
                )?;
                let outcome = AdmissionApplicationOutcome::new(
                    result,
                    admission_framed_digest(
                        b"buzz:s3-test-marker-effect:v1",
                        &[context.operation_id().as_bytes(), &self.intent_digest],
                    ),
                )?;
                let result_digest = context.canonical_result_digest(&outcome)?;
                sqlx::query(
                    "INSERT INTO s3_admission_markers \
                     (community_id,operation_id,request_fingerprint,object_kind,object_key, \
                      result_digest) VALUES ($1,$2,$3,$4,$5,$6)",
                )
                .bind(context.authorization_domain().as_uuid())
                .bind(context.operation_id())
                .bind(context.request_fingerprint().as_slice())
                .bind(context.object().kind().database_code())
                .bind(context.object().key().as_slice())
                .bind(result_digest.as_slice())
                .execute(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
                if self.fail_after_write {
                    Err(AdmissionCommitError::DependencyUnavailable)
                } else {
                    Ok(outcome)
                }
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn enrollment_commit_request(
        pool: &PgPool,
        assertion: VerifiedFederatedAssertion,
        proof: VerifiedNostrProof,
        policy: LocalAuthorizationPolicy,
        object: AdmissionObject,
        replay_key: [u8; 32],
        application_effect: Box<dyn AdmissionApplicationEffect>,
    ) -> AdmissionCommitRequest {
        enrollment_commit_request_for_route(
            pool,
            assertion,
            proof,
            policy,
            object,
            RouteCapability::InviteClaim,
            replay_key,
            application_effect,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn enrollment_commit_request_for_route(
        pool: &PgPool,
        assertion: VerifiedFederatedAssertion,
        proof: VerifiedNostrProof,
        policy: LocalAuthorizationPolicy,
        object: AdmissionObject,
        capability: RouteCapability,
        replay_key: [u8; 32],
        application_effect: Box<dyn AdmissionApplicationEffect>,
    ) -> AdmissionCommitRequest {
        let prepared = crate::identity_enrollment::prepare_direct_enrollment(
            pool,
            assertion.clone(),
            proof.clone(),
            Utc::now(),
        )
        .await
        .expect("prepare direct enrollment");
        let evidence = AdmissionEnrollmentEvidence::new(prepared, assertion, proof, policy)
            .expect("seal admission enrollment evidence");
        AdmissionCommitRequest::enrollment(
            Uuid::new_v4(),
            Uuid::new_v4(),
            object,
            capability,
            evidence,
        )
        .expect("construct enrollment admission")
        .with_replay_claim(
            AdmissionReplayClaim::new(
                AdmissionReplayClaimKind::TrustedProxyNonce,
                replay_key,
                Utc::now() + TimeDelta::minutes(10),
            )
            .expect("fresh replay claim"),
        )
        .with_application_effect(application_effect)
        .expect("attach canonical application effect")
    }

    #[test]
    fn replay_claim_rejects_sentinels_and_non_interoperable_time() {
        let now = Utc::now();
        assert_eq!(
            AdmissionReplayClaim::new(
                AdmissionReplayClaimKind::TrustedProxyNonce,
                [0; 32],
                now + TimeDelta::minutes(1),
            ),
            Err(AdmissionCommitError::InvalidRequest)
        );
        assert_eq!(
            AdmissionReplayClaim::new(
                AdmissionReplayClaimKind::TrustedProxyNonce,
                [1; 32],
                DateTime::<Utc>::MAX_UTC,
            ),
            Err(AdmissionCommitError::InvalidRequest)
        );
    }

    #[test]
    fn replay_claim_uses_an_exclusive_authoritative_time_bound() {
        let now = Utc::now();
        let retain_until = now + TimeDelta::minutes(1);
        let claim = AdmissionReplayClaim::new(
            AdmissionReplayClaimKind::TrustedProxyNonce,
            [1; 32],
            retain_until,
        )
        .expect("valid replay claim");
        assert_eq!(claim.kind().database_code(), 1);
        assert_eq!(claim.claim_key(), &[1; 32]);
        assert!(claim.validate_at(now).is_ok());
        assert_eq!(
            claim.validate_at(retain_until),
            Err(AdmissionCommitError::ReplayRejected)
        );
        assert_eq!(
            claim.validate_at(retain_until + TimeDelta::microseconds(1)),
            Err(AdmissionCommitError::ReplayRejected)
        );
    }

    #[test]
    fn logical_operation_id_is_stable_domain_separated_and_server_formed() {
        let semantic = admission_framed_digest(
            b"buzz:test:admission-intent:v1",
            &[b"domain", b"request", b"capability", b"target"],
        );
        let operation_id = admission_operation_id(semantic);
        assert_eq!(operation_id, admission_operation_id(semantic));
        assert_ne!(
            operation_id,
            admission_operation_id(admission_framed_digest(
                b"buzz:test:admission-intent:v1",
                &[b"domain", b"request", b"capability", b"other-target"],
            ))
        );
        assert_eq!(operation_id.get_version_num(), 8);
        assert_eq!(operation_id.get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn invite_application_results_are_closed_and_replay_decodable() {
        for expected in [
            CanonicalInviteClaimOutcome::Joined,
            CanonicalInviteClaimOutcome::AlreadyMember,
        ] {
            let result = AdmissionApplicationResult::invite_claim(expected);
            assert_eq!(
                result.schema(),
                AdmissionApplicationResultSchema::invite_claim()
            );
            assert_eq!(result.decode_invite_claim(), Ok(expected));
            assert_eq!(
                AdmissionApplicationResult::from_database(
                    result.schema().type_key().to_vec(),
                    result.schema().version() as i16,
                    result.code(),
                    result.payload().to_vec(),
                ),
                Ok(result)
            );
        }
        assert_eq!(
            AdmissionApplicationResult::from_database(
                INVITE_CLAIM_RESULT_TYPE.to_vec(),
                1,
                3,
                Vec::new(),
            )
            .and_then(|result| result.decode_invite_claim()),
            Err(AdmissionCommitError::InvalidRequest)
        );
        assert_eq!(
            AdmissionApplicationResult::new(
                AdmissionApplicationResultSchema::invite_claim(),
                1,
                vec![0; MAX_ADMISSION_APPLICATION_RESULT_PAYLOAD_BYTES + 1],
            ),
            Err(AdmissionCommitError::InvalidRequest)
        );
        assert_eq!(AdmissionApplicationResultSchema::new([0; 32], 1), None);
        assert_eq!(AdmissionApplicationResultSchema::new([1; 32], 0), None);
        assert_eq!(
            AdmissionApplicationResult::from_database(vec![1; 31], 1, 1, Vec::new()),
            Err(AdmissionCommitError::DependencyUnavailable)
        );
    }

    #[test]
    fn bounded_application_result_is_type_version_and_semantic_bound() {
        let schema =
            AdmissionApplicationResultSchema::new([71; 32], 3).expect("nonzero application schema");
        let result = AdmissionApplicationResult::new(schema, 7, b"stable-result".to_vec())
            .expect("bounded application result");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let object =
            AdmissionObject::new(AdmissionObjectKind::Repository, [70; 32]).expect("test object");
        let first =
            canonical_application_result_digest(domain, object, [72; 32], [73; 32], &result)
                .expect("first result digest");
        let different_domain_or_object = canonical_application_result_digest(
            CommunityId::from_uuid(Uuid::new_v4()),
            object,
            [72; 32],
            [73; 32],
            &result,
        )
        .expect("second result digest");
        assert_ne!(first, different_domain_or_object);
        assert_eq!(result.schema(), schema);
        assert_eq!(result.code(), 7);
        assert_eq!(result.payload(), b"stable-result");
    }

    #[test]
    fn invite_effect_intent_is_bounded_and_redacted() {
        assert!(CanonicalInviteClaimEffect::new([0; 32], None).is_err());
        assert!(CanonicalInviteClaimEffect::new([1; 32], Some("")).is_err());
        assert!(CanonicalInviteClaimEffect::new([1; 32], Some(&"x".repeat(129))).is_err());
        let first_policy = "a1".repeat(32);
        let second_policy = "b2".repeat(32);
        let first =
            CanonicalInviteClaimEffect::new([1; 32], Some(&first_policy)).expect("bounded effect");
        let retry =
            CanonicalInviteClaimEffect::new([1; 32], Some(&first_policy)).expect("stable retry");
        let changed =
            CanonicalInviteClaimEffect::new([1; 32], Some(&second_policy)).expect("changed policy");
        assert_eq!(first.intent_digest(), retry.intent_digest());
        assert_ne!(first.intent_digest(), changed.intent_digest());
        assert_eq!(
            format!("{first:?}"),
            "CanonicalInviteClaimEffect([REDACTED])"
        );
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database and OpenSSL"]
    async fn postgres_committer_co_commits_and_replays_fresh_claim_with_typed_result() {
        use sqlx::postgres::PgPoolOptions;

        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admission test admin");
        let database_name = format!("s3_admission_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("create disposable admission database");
        let split = admin_url.rfind('/').expect("database URL path");
        let database_url = format!("{}/{}", &admin_url[..split], database_name);
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect(&database_url)
            .await
            .expect("connect disposable admission database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply canonical migrations");
        sqlx::query(
            "CREATE TABLE s3_admission_markers ( \
             community_id uuid NOT NULL, operation_id uuid NOT NULL PRIMARY KEY, \
             request_fingerprint bytea NOT NULL, object_kind smallint NOT NULL, \
             object_key bytea NOT NULL, result_digest bytea NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create transaction marker table");

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let target = [101_u8; 32];
        let request_fingerprint = [102_u8; 32];
        let transport_context = [103_u8; 32];
        let object =
            AdmissionObject::new(AdmissionObjectKind::Domain, target).expect("test object");
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("admission-{}.example", domain.as_uuid().simple()))
            .execute(&pool)
            .await
            .expect("insert admission community");
        // Use the replay-claim table installed by the registered migrations.
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,1,$2,transaction_timestamp()-interval '1 second')",
        )
        .bind(domain.as_uuid())
        .bind([104_u8; 32].as_slice())
        .execute(&pool)
        .await
        .expect("insert attested enrollment policy");
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes, \
              restrictive_reserve_events,restrictive_reserve_bytes) \
             VALUES ($1,64,262144,8192,8,32768)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("install admission audit capacity");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("activate admission invalidation");
        let invite_token = [105_u8; 32];
        let alternate_invite = [106_u8; 32];
        for token in [invite_token, alternate_invite] {
            sqlx::query(
                "INSERT INTO relay_invites \
                 (community_id,token_hash,max_uses,expires_at,created_by) \
                 VALUES ($1,$2,8,transaction_timestamp()+interval '1 hour','operator')",
            )
            .bind(domain.as_uuid())
            .bind(token.as_slice())
            .execute(&pool)
            .await
            .expect("insert relay invite");
        }

        let keys = Keys::generate();
        let (assertion, proof) = verified_enrollment_evidence(
            domain,
            &keys,
            "canonical-subject",
            target,
            request_fingerprint,
            transport_context,
        );
        let policy = LocalAuthorizationPolicy::from_database(
            domain,
            Uuid::new_v4(),
            1,
            0,
            2,
            AuthorizationLeaseFence::from_bytes([107; 32]).expect("authorization fence"),
            RouteCapability::InviteClaim,
            Utc::now() + TimeDelta::minutes(5),
            None,
            None,
        )
        .expect("local authorization policy");
        let committer = Arc::new(PostgresCanonicalAdmissionCommitter::new(
            pool.clone(),
            Arc::new(TestPostgresAdmissionRechecker),
        ));

        let cross_capability_policy = LocalAuthorizationPolicy::from_database(
            domain,
            Uuid::new_v4(),
            1,
            0,
            2,
            AuthorizationLeaseFence::from_bytes([107; 32]).expect("authorization fence"),
            RouteCapability::MessagesWrite,
            Utc::now() + TimeDelta::minutes(5),
            None,
            None,
        )
        .expect("cross-capability policy");
        let cross_capability = enrollment_commit_request_for_route(
            &pool,
            assertion.clone(),
            proof.clone(),
            cross_capability_policy,
            object,
            RouteCapability::MessagesWrite,
            [124; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, None)
                    .expect("cross-capability invite effect"),
            ),
        )
        .await;
        assert!(matches!(
            committer.commit(cross_capability).await,
            Err(AdmissionCommitError::AuthorizationDenied)
        ));

        let cross_object = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            AdmissionObject::new(AdmissionObjectKind::Repository, target)
                .expect("cross-object coordinate"),
            [125; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, None)
                    .expect("cross-object invite effect"),
            ),
        )
        .await;
        assert!(matches!(
            committer.commit(cross_object).await,
            Err(AdmissionCommitError::AuthorizationDenied)
        ));
        let (
            denied_bindings,
            denied_members,
            denied_invite_uses,
            denied_receipts,
            denied_events,
            denied_claims,
        ): (i64, i64, i32, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM identity_bindings WHERE community_id=$1), \
                 (SELECT count(*) FROM relay_members WHERE community_id=$1), \
                 (SELECT use_count FROM relay_invites WHERE community_id=$1 AND token_hash=$2), \
                 (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_proxy_nonce_claims \
                   WHERE authorization_domain=$1 AND claim_key IN ($3,$4))",
        )
        .bind(domain.as_uuid())
        .bind(invite_token.as_slice())
        .bind([124_u8; 32].as_slice())
        .bind([125_u8; 32].as_slice())
        .fetch_one(&pool)
        .await
        .expect("read denied invite effect side effects");
        assert_eq!(
            (
                denied_bindings,
                denied_members,
                denied_invite_uses,
                denied_receipts,
                denied_events,
                denied_claims,
            ),
            (0, 0, 0, 0, 0, 0)
        );

        sqlx::query(
            "CREATE FUNCTION s3_reject_policy_acceptance_v1() RETURNS trigger AS $$ \
             BEGIN RAISE EXCEPTION 'injected policy acceptance failure'; END; \
             $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .expect("install concrete invite DML failure function");
        sqlx::query(
            "CREATE TRIGGER s3_reject_policy_acceptance \
             BEFORE INSERT ON join_policy_acceptances FOR EACH ROW \
             EXECUTE FUNCTION s3_reject_policy_acceptance_v1()",
        )
        .execute(&pool)
        .await
        .expect("install concrete invite DML failure trigger");
        let policy_version = "ab".repeat(32);
        let concrete_failure = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [126; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, Some(&policy_version))
                    .expect("concrete failing invite effect"),
            ),
        )
        .await;
        assert!(matches!(
            committer.commit(concrete_failure).await,
            Err(AdmissionCommitError::DependencyUnavailable)
        ));
        let (
            failed_bindings,
            failed_members,
            failed_invite_uses,
            failed_receipts,
            failed_events,
            failed_claims,
            failed_acceptances,
        ): (i64, i64, i32, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM identity_bindings WHERE community_id=$1), \
                 (SELECT count(*) FROM relay_members WHERE community_id=$1), \
                 (SELECT use_count FROM relay_invites WHERE community_id=$1 AND token_hash=$2), \
                 (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_proxy_nonce_claims \
                   WHERE authorization_domain=$1 AND claim_key=$3), \
                 (SELECT count(*) FROM join_policy_acceptances WHERE community_id=$1)",
        )
        .bind(domain.as_uuid())
        .bind(invite_token.as_slice())
        .bind([126_u8; 32].as_slice())
        .fetch_one(&pool)
        .await
        .expect("read concrete invite DML rollback");
        assert_eq!(
            (
                failed_bindings,
                failed_members,
                failed_invite_uses,
                failed_receipts,
                failed_events,
                failed_claims,
                failed_acceptances,
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
        sqlx::query("DROP TRIGGER s3_reject_policy_acceptance ON join_policy_acceptances")
            .execute(&pool)
            .await
            .expect("remove concrete invite DML failure trigger");
        sqlx::query("DROP FUNCTION s3_reject_policy_acceptance_v1()")
            .execute(&pool)
            .await
            .expect("remove concrete invite DML failure function");

        let first_request = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [108; 32],
            Box::new(CanonicalInviteClaimEffect::new(invite_token, None).expect("invite effect")),
        )
        .await;
        assert_eq!(
            first_request.transport_binding(),
            (ProofTransport::Nip42, transport_context)
        );
        let first_operation = first_request.operation_id();
        let first = committer
            .commit(first_request)
            .await
            .expect("canonical admission commit");
        let (first_receipt, first_result) = match first {
            AdmissionCommitOutcome::Committed {
                receipt,
                application_result,
                ..
            } => (receipt, application_result.expect("fresh typed result")),
            AdmissionCommitOutcome::ExactReplay { .. } => panic!("first admission must commit"),
        };
        assert_eq!(
            first_result.decode_invite_claim(),
            Ok(CanonicalInviteClaimOutcome::Joined)
        );
        assert!(first_receipt.application_result_digest().is_some());

        let retry_request = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [109; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, None).expect("retry invite effect"),
            ),
        )
        .await;
        assert_eq!(retry_request.operation_id(), first_operation);
        let replay = committer
            .commit(retry_request)
            .await
            .expect("fresh-claim exact replay");
        let (replay_receipt, replay_result) = match replay {
            AdmissionCommitOutcome::ExactReplay {
                receipt,
                application_result,
            } => (receipt, application_result.expect("replayed typed result")),
            AdmissionCommitOutcome::Committed { .. } => panic!("retry must not repeat DML"),
        };
        assert_eq!(replay_result, first_result);
        assert_eq!(
            replay_result.decode_invite_claim(),
            Ok(CanonicalInviteClaimOutcome::Joined)
        );
        assert_eq!(
            replay_receipt.application_result_digest(),
            first_receipt.application_result_digest()
        );
        let replay_claim_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain=$1 AND claim_key IN ($2,$3)",
        )
        .bind(domain.as_uuid())
        .bind([108_u8; 32].as_slice())
        .bind([109_u8; 32].as_slice())
        .fetch_one(&pool)
        .await
        .expect("count exact replay claims");
        assert_eq!(
            replay_claim_count, 2,
            "fresh execution and fresh-claim replay must consume both claims"
        );

        let duplicate_claim = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [128; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, None)
                    .expect("duplicate-claim invite effect"),
            ),
        )
        .await
        .with_replay_claim(
            AdmissionReplayClaim::new(
                AdmissionReplayClaimKind::TrustedProxyNonce,
                [109; 32],
                Utc::now() + TimeDelta::minutes(10),
            )
            .expect("duplicate prior claim"),
        );
        assert_eq!(duplicate_claim.operation_id(), first_operation);
        assert!(matches!(
            committer.commit(duplicate_claim).await,
            Err(AdmissionCommitError::ReplayRejected)
        ));

        let stale_claim = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [127; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(invite_token, None)
                    .expect("stale-claim invite effect"),
            ),
        )
        .await
        .with_replay_claim(
            AdmissionReplayClaim::new(
                AdmissionReplayClaimKind::TrustedProxyNonce,
                [127; 32],
                Utc::now() - TimeDelta::seconds(1),
            )
            .expect("well-formed stale claim"),
        );
        assert_eq!(stale_claim.operation_id(), first_operation);
        assert!(matches!(
            committer.commit(stale_claim).await,
            Err(AdmissionCommitError::ReplayRejected)
        ));
        let stale_claim_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_proxy_nonce_claims \
             WHERE authorization_domain=$1 AND claim_key=$2",
        )
        .bind(domain.as_uuid())
        .bind([127_u8; 32].as_slice())
        .fetch_one(&pool)
        .await
        .expect("read rejected stale replay claim");
        assert_eq!(stale_claim_count, 0);

        let second_publication = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [110; 32],
            Box::new(TestMarkerEffect {
                intent_digest: [111; 32],
                fail_after_write: false,
            }),
        )
        .await;
        let second_operation = second_publication.operation_id();
        assert_ne!(second_operation, first_operation);
        assert!(matches!(
            committer
                .commit(second_publication)
                .await
                .expect("distinct same-epoch publication"),
            AdmissionCommitOutcome::Committed { .. }
        ));

        let mismatched_same_epoch_policy = LocalAuthorizationPolicy::from_database(
            domain,
            Uuid::new_v4(),
            1,
            0,
            2,
            AuthorizationLeaseFence::from_bytes([119; 32]).expect("mismatched fence"),
            RouteCapability::InviteClaim,
            Utc::now() + TimeDelta::minutes(5),
            None,
            None,
        )
        .expect("same-epoch mismatched policy");
        let mismatched_same_epoch = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            mismatched_same_epoch_policy,
            object,
            [120; 32],
            Box::new(TestMarkerEffect {
                intent_digest: [121; 32],
                fail_after_write: false,
            }),
        )
        .await;
        let mismatched_operation = mismatched_same_epoch.operation_id();
        assert!(matches!(
            committer.commit(mismatched_same_epoch).await,
            Err(AdmissionCommitError::AuthorizationDenied)
        ));

        let lower_epoch_policy = LocalAuthorizationPolicy::from_database(
            domain,
            Uuid::new_v4(),
            1,
            0,
            1,
            AuthorizationLeaseFence::from_bytes([107; 32]).expect("lower-epoch fence"),
            RouteCapability::InviteClaim,
            Utc::now() + TimeDelta::minutes(5),
            None,
            None,
        )
        .expect("lower-epoch policy");
        let lower_epoch = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            lower_epoch_policy,
            object,
            [122; 32],
            Box::new(TestMarkerEffect {
                intent_digest: [123; 32],
                fail_after_write: false,
            }),
        )
        .await;
        let lower_operation = lower_epoch.operation_id();
        assert!(matches!(
            committer.commit(lower_epoch).await,
            Err(AdmissionCommitError::AuthorizationDenied)
        ));
        let denied_authority_markers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM s3_admission_markers WHERE operation_id IN ($1,$2)",
        )
        .bind(mismatched_operation)
        .bind(lower_operation)
        .fetch_one(&pool)
        .await
        .expect("read denied same/lower epoch markers");
        assert_eq!(denied_authority_markers, 0);

        let failure_request = enrollment_commit_request(
            &pool,
            assertion.clone(),
            proof.clone(),
            policy.clone(),
            object,
            [112; 32],
            Box::new(TestMarkerEffect {
                intent_digest: [113; 32],
                fail_after_write: true,
            }),
        )
        .await;
        let failure_operation = failure_request.operation_id();
        assert!(matches!(
            committer.commit(failure_request).await,
            Err(AdmissionCommitError::DependencyUnavailable)
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM s3_admission_markers WHERE operation_id=$1",
            )
            .bind(failure_operation)
            .fetch_one(&pool)
            .await
            .expect("read rolled-back effect marker"),
            0
        );

        let failure_generation: i64 =
            sqlx::query_scalar("SELECT authorization_event_capacity_report_failure_v2($1,$2)")
                .bind(domain.as_uuid())
                .bind(1_i16)
                .fetch_one(&pool)
                .await
                .expect("latch audit failure");
        let audit_keys = Keys::generate();
        let audit_event_author = audit_keys.public_key().to_bytes();
        let (audit_assertion, audit_proof) = verified_enrollment_evidence(
            domain,
            &audit_keys,
            "audit-capacity-subject",
            target,
            request_fingerprint,
            transport_context,
        );
        let audit_request = enrollment_commit_request(
            &pool,
            audit_assertion,
            audit_proof,
            policy.clone(),
            object,
            [114; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(alternate_invite, None)
                    .expect("audit rollback invite effect"),
            ),
        )
        .await;
        let audit_operation = audit_request.operation_id();
        assert!(matches!(
            committer.commit(audit_request).await,
            Err(AdmissionCommitError::AuditUnavailable)
        ));
        let (
            audit_binding,
            audit_member,
            audit_invite_uses,
            audit_receipt,
            audit_event,
            audit_claim,
        ): (i64, i64, i32, i64, i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM identity_bindings \
               WHERE community_id=$2 AND event_author_pubkey=$4), \
             (SELECT count(*) FROM relay_members WHERE community_id=$2 AND pubkey=$5), \
             (SELECT use_count FROM relay_invites WHERE community_id=$2 AND token_hash=$6), \
             (SELECT count(*) FROM authorization_operation_receipts \
               WHERE community_id=$2 AND operation_id=$1), \
             (SELECT count(*) FROM authorization_events \
               WHERE community_id=$2 AND operation_id=$1), \
             (SELECT count(*) FROM authorization_proxy_nonce_claims \
               WHERE authorization_domain=$2 AND claim_key=$3)",
        )
        .bind(audit_operation)
        .bind(domain.as_uuid())
        .bind([114_u8; 32].as_slice())
        .bind(audit_event_author.as_slice())
        .bind(audit_keys.public_key().to_hex())
        .bind(alternate_invite.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read audit rollback state");
        assert_eq!(
            (
                audit_binding,
                audit_member,
                audit_invite_uses,
                audit_receipt,
                audit_event,
                audit_claim,
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT authorization_event_capacity_recover_v2($1,$2)",
        )
        .bind(domain.as_uuid())
        .bind(failure_generation)
        .fetch_one(&pool)
        .await
        .expect("recover admission audit capacity"));

        let other_keys = Keys::generate();
        let (other_assertion, other_proof) = verified_enrollment_evidence(
            domain,
            &other_keys,
            "canonical-subject",
            target,
            request_fingerprint,
            transport_context,
        );
        let partial_bijection_request = enrollment_commit_request(
            &pool,
            other_assertion,
            other_proof,
            policy.clone(),
            object,
            [116; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(alternate_invite, None)
                    .expect("alternate invite effect"),
            ),
        )
        .await;
        assert!(matches!(
            committer.commit(partial_bijection_request).await,
            Err(AdmissionCommitError::AuthorizationDenied)
        ));

        let stale_keys = Keys::generate();
        let (stale_assertion, stale_proof) = verified_enrollment_evidence(
            domain,
            &stale_keys,
            "stale-policy-subject",
            target,
            request_fingerprint,
            transport_context,
        );
        let stale_policy_request = enrollment_commit_request(
            &pool,
            stale_assertion.clone(),
            stale_proof.clone(),
            policy.clone(),
            object,
            [117; 32],
            Box::new(
                CanonicalInviteClaimEffect::new(alternate_invite, None)
                    .expect("stale policy invite effect"),
            ),
        )
        .await;
        let mut policy_update = pool.begin().await.expect("begin concurrent policy update");
        sqlx::query("SELECT id FROM communities WHERE id=$1 FOR UPDATE")
            .bind(domain.as_uuid())
            .execute(&mut *policy_update)
            .await
            .expect("lock domain before restrictive policy update");
        let concurrent_committer = Arc::clone(&committer);
        let mut stale_commit =
            tokio::spawn(async move { concurrent_committer.commit(stale_policy_request).await });
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut stale_commit)
                .await
                .is_err(),
            "admission must serialize behind the domain policy lock"
        );
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,2,2,$2,transaction_timestamp()-interval '1 microsecond')",
        )
        .bind(domain.as_uuid())
        .bind([118_u8; 32].as_slice())
        .execute(&mut *policy_update)
        .await
        .expect("install newer Provisioned policy");
        policy_update
            .commit()
            .await
            .expect("commit concurrent restrictive policy");
        assert!(matches!(
            stale_commit.await.expect("join serialized stale admission"),
            Err(AdmissionCommitError::AuthorizationDenied)
        ));
        assert!(matches!(
            crate::identity_enrollment::prepare_direct_enrollment(
                &pool,
                stale_assertion,
                stale_proof,
                Utc::now(),
            )
            .await,
            Err(IdentityEnrollmentError::ProvisioningRequired)
        ));

        let (binding_count, member_count, invite_uses, receipt_count, event_count, authority_count):
            (i64, i64, i32, i64, i64, i64) = sqlx::query_as(
            "SELECT \
             (SELECT count(*) FROM identity_bindings WHERE community_id=$1), \
             (SELECT count(*) FROM relay_members WHERE community_id=$1), \
             (SELECT use_count FROM relay_invites WHERE community_id=$1 AND token_hash=$2), \
             (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1 AND operation_id IN ($3,$4)), \
             (SELECT count(*) FROM authorization_events WHERE community_id=$1 AND operation_id IN ($3,$4)), \
             (SELECT count(*) FROM protected_object_authority WHERE community_id=$1 AND object_kind=1 AND object_key=$5)",
        )
        .bind(domain.as_uuid())
        .bind(invite_token.as_slice())
        .bind(first_operation)
        .bind(second_operation)
        .bind(target.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read canonical co-commit state");
        assert_eq!(
            (
                binding_count,
                member_count,
                invite_uses,
                receipt_count,
                event_count,
                authority_count,
            ),
            (1, 1, 1, 2, 2, 1)
        );
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("drop disposable admission database");
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn postgres_invite_effect_and_persisted_replay_are_atomic_and_typed() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("dedicated test database URL");
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect dedicated database");
        sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
            .execute(&pool)
            .await
            .expect("drop public schema");
        sqlx::query("CREATE SCHEMA public")
            .execute(&pool)
            .await
            .expect("create public schema");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply canonical migrations");

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("admission-{}.example", domain.as_uuid().simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        let actor = "11".repeat(32);
        let rollback_actor = "22".repeat(32);
        let capacity_actor = "33".repeat(32);
        let joined_token = [41_u8; 32];
        let rollback_token = [42_u8; 32];
        let capacity_token = [43_u8; 32];
        for (token, maximum) in [
            (joined_token, 2_i32),
            (rollback_token, 1_i32),
            (capacity_token, 1_i32),
        ] {
            sqlx::query(
                "INSERT INTO relay_invites \
                 (community_id,token_hash,max_uses,expires_at,created_by) \
                 VALUES ($1,$2,$3,transaction_timestamp()+INTERVAL '1 hour','operator')",
            )
            .bind(domain.as_uuid())
            .bind(token.as_slice())
            .bind(maximum)
            .execute(&pool)
            .await
            .expect("insert invite");
        }

        let policy_version = "a1".repeat(32);
        let effect = CanonicalInviteClaimEffect::new(joined_token, Some(&policy_version))
            .expect("invite effect");
        let mut joined_tx = pool.begin().await.expect("begin joined effect");
        let joined = apply_canonical_invite_claim_tx(
            &mut joined_tx,
            domain,
            &actor,
            joined_token,
            Some(&policy_version),
            effect.intent_digest(),
        )
        .await
        .expect("apply joined effect");
        assert_eq!(
            joined.result(),
            &AdmissionApplicationResult::invite_claim(CanonicalInviteClaimOutcome::Joined)
        );
        joined_tx.commit().await.expect("commit joined effect");

        let mut replay_tx = pool.begin().await.expect("begin already-member effect");
        let already = apply_canonical_invite_claim_tx(
            &mut replay_tx,
            domain,
            &actor,
            joined_token,
            Some(&policy_version),
            effect.intent_digest(),
        )
        .await
        .expect("apply already-member effect");
        assert_eq!(
            already.result(),
            &AdmissionApplicationResult::invite_claim(CanonicalInviteClaimOutcome::AlreadyMember)
        );
        replay_tx
            .commit()
            .await
            .expect("commit already-member effect");
        let joined_count: i32 = sqlx::query_scalar(
            "SELECT use_count FROM relay_invites WHERE community_id=$1 AND token_hash=$2",
        )
        .bind(domain.as_uuid())
        .bind(joined_token.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read joined use count");
        assert_eq!(joined_count, 1);

        let rollback_effect =
            CanonicalInviteClaimEffect::new(rollback_token, None).expect("rollback effect");
        let mut rollback_tx = pool.begin().await.expect("begin rollback effect");
        assert_eq!(
            apply_canonical_invite_claim_tx(
                &mut rollback_tx,
                domain,
                &rollback_actor,
                rollback_token,
                Some("invalid-policy-version"),
                rollback_effect.intent_digest(),
            )
            .await
            .expect_err("policy persistence failure must abort the effect"),
            AdmissionCommitError::DependencyUnavailable
        );
        rollback_tx
            .rollback()
            .await
            .expect("roll back failed effect");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM relay_members WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(domain.as_uuid())
            .bind(&rollback_actor)
            .fetch_one(&pool)
            .await
            .expect("read rolled-back membership"),
            0
        );

        let capacity_effect = CanonicalInviteClaimEffect::new(capacity_token, None)
            .expect("capacity rollback effect");
        let mut capacity_tx = pool.begin().await.expect("begin capacity rollback");
        apply_canonical_invite_claim_tx(
            &mut capacity_tx,
            domain,
            &capacity_actor,
            capacity_token,
            None,
            capacity_effect.intent_digest(),
        )
        .await
        .expect("stage capacity rollback effect");
        let capacity_error = sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,operation_id, \
              request_fingerprint,correlation_id,attempt_id,occurred_at,canonical_envelope,envelope_digest) \
             VALUES ($1,$2,9,2,9,4,$3,NULL,$4,$5,transaction_timestamp(),$6,$7)",
        )
        .bind(domain.as_uuid())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![9_u8; 32])
        .bind(vec![19_u8; 32])
        .execute(&mut *capacity_tx)
        .await
        .expect_err("missing audit capacity aborts the shared transaction");
        assert_eq!(
            capacity_error
                .as_database_error()
                .and_then(|error| error.constraint()),
            Some("authorization_event_capacity_policy_required")
        );
        capacity_tx
            .rollback()
            .await
            .expect("roll back capacity failure");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM relay_members WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(domain.as_uuid())
            .bind(&capacity_actor)
            .fetch_one(&pool)
            .await
            .expect("read capacity-rolled-back membership"),
            0
        );

        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
             VALUES ($1,8,8192,1024)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("install audit capacity");
        let operation_id = admission_operation_id([51; 32]);
        let request_fingerprint = [52_u8; 32];
        let semantic_fingerprint = [53_u8; 32];
        let application_intent = [56_u8; 32];
        let application_effect = [57_u8; 32];
        let object =
            AdmissionObject::new(AdmissionObjectKind::Domain, [58; 32]).expect("test object");
        let persisted_result =
            AdmissionApplicationResult::invite_claim(CanonicalInviteClaimOutcome::AlreadyMember);
        let persisted_result_digest = canonical_admission_result_digest(
            domain,
            object,
            semantic_fingerprint,
            Some(application_intent),
            Some(&persisted_result),
            Some(application_effect),
        )
        .expect("canonical persisted result digest");
        let persisted_application_result_digest = canonical_application_result_digest(
            domain,
            object,
            semantic_fingerprint,
            application_intent,
            &persisted_result,
        )
        .expect("canonical application result digest");
        let audit_envelope = vec![10_u8; 32];
        let audit_envelope_digest: [u8; 32] = Sha256::digest(&audit_envelope).into();
        let event_id = Uuid::new_v4();
        let mut persisted = pool.begin().await.expect("begin persisted replay fixture");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,11,$4,1,$5)",
        )
        .bind(domain.as_uuid())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind([54_u8; 32].as_slice())
        .bind(persisted_result_digest.as_slice())
        .execute(&mut *persisted)
        .await
        .expect("insert replay receipt");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,actor_fingerprint, \
              operation_id,request_fingerprint,correlation_id,attempt_id,occurred_at, \
              canonical_envelope,envelope_digest) \
             VALUES ($1,$2,10,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(domain.as_uuid())
        .bind(event_id)
        .bind([54_u8; 32].as_slice())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(&audit_envelope)
        .bind(audit_envelope_digest.as_slice())
        .execute(&mut *persisted)
        .await
        .expect("insert replay event");
        sqlx::query(
            "INSERT INTO authorization_admission_results \
             (community_id,operation_id,request_fingerprint,semantic_fingerprint,object_kind, \
              object_key,application_type,application_version,application_code,application_payload, \
              application_intent_digest,application_effect_digest,application_result_digest) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,1,2,$8,$9,$10,$11)",
        )
        .bind(domain.as_uuid())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(semantic_fingerprint.as_slice())
        .bind(object.kind().database_code())
        .bind(object.key().as_slice())
        .bind(INVITE_CLAIM_RESULT_TYPE.as_slice())
        .bind(Vec::<u8>::new())
        .bind(application_intent.as_slice())
        .bind(application_effect.as_slice())
        .bind(persisted_application_result_digest.as_slice())
        .execute(&mut *persisted)
        .await
        .expect("insert persisted typed result");
        persisted.commit().await.expect("commit replay fixture");
        sqlx::query(
            "UPDATE authorization_admission_results SET application_payload=$3 \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(operation_id)
        .bind(b"changed-result".as_slice())
        .execute(&pool)
        .await
        .expect_err("persisted application result is immutable");

        let mut read_tx = pool.begin().await.expect("begin replay read");
        let (replayed_receipt, replayed_result) = read_existing_receipt(
            &mut read_tx,
            ExistingAdmissionLookup {
                domain,
                operation_id,
                request_fingerprint,
                semantic_fingerprint,
                object,
                receipt_shape: AdmissionReceiptShape::ProtectedMutation,
                expected_application_schema: Some(AdmissionApplicationResultSchema::invite_claim()),
            },
        )
        .await
        .expect("read exact replay")
        .expect("persisted replay exists");
        assert_eq!(replayed_receipt.authorization_domain(), domain);
        assert_eq!(replayed_receipt.object(), object);
        assert_eq!(
            replayed_receipt.semantic_fingerprint(),
            &semantic_fingerprint
        );
        assert_eq!(
            replayed_result,
            Some(AdmissionApplicationResult::invite_claim(
                CanonicalInviteClaimOutcome::AlreadyMember
            ))
        );
        assert_eq!(
            read_existing_receipt(
                &mut read_tx,
                ExistingAdmissionLookup {
                    domain,
                    operation_id,
                    request_fingerprint,
                    semantic_fingerprint: [99; 32],
                    object,
                    receipt_shape: AdmissionReceiptShape::ProtectedMutation,
                    expected_application_schema: Some(
                        AdmissionApplicationResultSchema::invite_claim(),
                    ),
                },
            )
            .await,
            Err(AdmissionCommitError::IntentConflict)
        );
        assert_eq!(
            read_existing_receipt(
                &mut read_tx,
                ExistingAdmissionLookup {
                    domain,
                    operation_id,
                    request_fingerprint,
                    semantic_fingerprint,
                    object: AdmissionObject::new(AdmissionObjectKind::Repository, [59; 32])
                        .expect("other object"),
                    receipt_shape: AdmissionReceiptShape::ProtectedMutation,
                    expected_application_schema: Some(
                        AdmissionApplicationResultSchema::invite_claim(),
                    ),
                },
            )
            .await,
            Err(AdmissionCommitError::IntentConflict)
        );
        read_tx.rollback().await.expect("close replay read");
        pool.close().await;
    }
}
