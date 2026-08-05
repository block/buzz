//! Provider-neutral composition boundary for privileged lifecycle operations.
//!
//! The stock relay never constructs this runtime. A deployment must explicitly
//! supply an authenticator, a durable idempotent executor, and a trusted clock
//! before [`crate::api::operator::lifecycle_router`] can be built.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use axum::http::HeaderValue;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const MAX_CREDENTIAL_BYTES: usize = 8 * 1024;
const MAX_APPROVALS: usize = 4;
const MAX_LIST_LIMIT: u16 = 100;

/// Redaction-safe opaque reference to an actor, target, approval, or cursor.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpaqueOperatorReference([u8; 32]);

impl OpaqueOperatorReference {
    /// Construct a reference from an already-derived pseudonymous digest.
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// Return the pseudonymous digest for durable comparison and encoding.
    pub const fn digest(self) -> [u8; 32] {
        self.0
    }

    fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for OpaqueOperatorReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueOperatorReference([redacted])")
    }
}

impl Serialize for OpaqueOperatorReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for OpaqueOperatorReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() != 64 {
            return Err(D::Error::custom("operator reference must be 32-byte hex"));
        }
        let mut digest = [0; 32];
        hex::decode_to_slice(encoded, &mut digest)
            .map_err(|_| D::Error::custom("operator reference must be 32-byte hex"))?;
        if digest == [0; 32] {
            return Err(D::Error::custom("operator reference must be non-zero"));
        }
        Ok(Self(digest))
    }
}

/// Sensitive transport credential passed only to the installed authenticator.
pub struct OperatorCredential(Box<[u8]>);

impl OperatorCredential {
    /// Copy one bounded authorization header without parsing or logging it.
    pub fn from_authorization_header(value: &HeaderValue) -> Result<Self, OperatorRuntimeError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(OperatorRuntimeError::InvalidCredential);
        }
        Ok(Self(bytes.to_vec().into_boxed_slice()))
    }

    /// Expose the credential only at the explicit authentication boundary.
    pub fn expose_to_authenticator(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for OperatorCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperatorCredential([redacted])")
    }
}

impl Drop for OperatorCredential {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Closed lifecycle capabilities understood by the provider-neutral runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperatorCapability {
    /// Inspect active and historical lifecycle records.
    Inspect,
    /// Preview an exact lifecycle transition.
    Preview,
    /// Revoke an exact lifecycle target.
    Revoke,
    /// Rotate an exact binding to a proven replacement.
    Rotate,
}

/// Stable provider-neutral purpose for a privileged operation.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OperatorReasonCode {
    /// Routine account offboarding.
    Offboarding,
    /// Credential or key compromise containment.
    CompromiseContainment,
    /// Planned key rotation.
    PlannedRotation,
    /// Account recovery after independent verification.
    VerifiedRecovery,
    /// Data-integrity repair of an already committed effect.
    IntegrityRepair,
    /// Emergency deny-only local containment.
    EmergencyContainment,
    /// Retention archive of an inactive record.
    RetentionArchive,
}

impl OperatorReasonCode {
    /// Numeric representation frozen by the provider-neutral lifecycle contract.
    pub const fn discriminant(self) -> u16 {
        match self {
            Self::Offboarding => 1,
            Self::CompromiseContainment => 2,
            Self::PlannedRotation => 3,
            Self::VerifiedRecovery => 4,
            Self::IntegrityRepair => 5,
            Self::EmergencyContainment => 6,
            Self::RetentionArchive => 7,
        }
    }
}

/// Lifecycle operations exposed by the initial reachable operator surface.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OperatorAction {
    /// List active and historical lifecycle records.
    List,
    /// Preview an exact rotation.
    Preview,
    /// Revoke an exact target.
    Revoke,
    /// Rotate an exact binding.
    Rotate,
}

impl OperatorAction {
    /// Required authenticated capability.
    pub const fn capability(self) -> OperatorCapability {
        match self {
            Self::List => OperatorCapability::Inspect,
            Self::Preview => OperatorCapability::Preview,
            Self::Revoke => OperatorCapability::Revoke,
            Self::Rotate => OperatorCapability::Rotate,
        }
    }

    fn discriminant(self) -> u16 {
        match self {
            Self::List => 1,
            Self::Preview => 2,
            Self::Revoke => 3,
            Self::Rotate => 4,
        }
    }
}

/// Common stable identities and fences for one lifecycle invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorInvocationContext {
    domain_id: Uuid,
    operation_id: Uuid,
    correlation_id: Uuid,
    reason: OperatorReasonCode,
    expected_revision: u64,
    approval_references: Box<[OpaqueOperatorReference]>,
}

impl OperatorInvocationContext {
    /// Validate the domain, operation, correlation, revision, and approvals.
    pub fn new(
        domain_id: Uuid,
        operation_id: Uuid,
        correlation_id: Uuid,
        reason: OperatorReasonCode,
        expected_revision: u64,
        mut approval_references: Vec<OpaqueOperatorReference>,
    ) -> Result<Self, OperatorRuntimeError> {
        if domain_id.is_nil()
            || operation_id.is_nil()
            || correlation_id.is_nil()
            || expected_revision == 0
            || approval_references.len() > MAX_APPROVALS
            || approval_references.iter().any(|value| value.is_zero())
        {
            return Err(OperatorRuntimeError::InvalidRequest);
        }
        approval_references.sort_unstable_by_key(|value| value.digest());
        if approval_references
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(OperatorRuntimeError::InvalidRequest);
        }
        Ok(Self {
            domain_id,
            operation_id,
            correlation_id,
            reason,
            expected_revision,
            approval_references: approval_references.into_boxed_slice(),
        })
    }

    /// Server-resolved authorization domain.
    pub const fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    /// Stable idempotency identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Request-correlation identity, separate from operation identity.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Stable operator reason.
    pub const fn reason(&self) -> OperatorReasonCode {
        self.reason
    }

    /// Exact lifecycle revision fence supplied by the caller.
    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    /// Bounded opaque approval references.
    pub fn approval_references(&self) -> &[OpaqueOperatorReference] {
        &self.approval_references
    }
}

/// Action-specific bounded operator intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatorIntent {
    /// Bounded list with an optional opaque cursor.
    List {
        /// Maximum records to return.
        limit: u16,
        /// Opaque pagination cursor.
        after: Option<OpaqueOperatorReference>,
    },
    /// Preview one exact rotation.
    Preview {
        /// Existing binding reference.
        target: OpaqueOperatorReference,
        /// Proposed replacement reference.
        replacement: OpaqueOperatorReference,
    },
    /// Revoke one exact target.
    Revoke {
        /// Exact target reference.
        target: OpaqueOperatorReference,
    },
    /// Rotate one exact target to one exact replacement.
    Rotate {
        /// Existing binding reference.
        target: OpaqueOperatorReference,
        /// Proven replacement reference.
        replacement: OpaqueOperatorReference,
    },
}

impl OperatorIntent {
    /// Operation class represented by this intent.
    pub const fn action(&self) -> OperatorAction {
        match self {
            Self::List { .. } => OperatorAction::List,
            Self::Preview { .. } => OperatorAction::Preview,
            Self::Revoke { .. } => OperatorAction::Revoke,
            Self::Rotate { .. } => OperatorAction::Rotate,
        }
    }
}

/// Fully shaped, still-unauthenticated operator invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorInvocation {
    context: OperatorInvocationContext,
    intent: OperatorIntent,
    fingerprint: [u8; 32],
}

impl OperatorInvocation {
    /// Construct a bounded invocation and derive its stable semantic digest.
    pub fn new(
        context: OperatorInvocationContext,
        intent: OperatorIntent,
    ) -> Result<Self, OperatorRuntimeError> {
        match &intent {
            OperatorIntent::List { limit, after } => {
                if *limit == 0
                    || *limit > MAX_LIST_LIMIT
                    || after.is_some_and(OpaqueOperatorReference::is_zero)
                {
                    return Err(OperatorRuntimeError::InvalidRequest);
                }
            }
            OperatorIntent::Preview {
                target,
                replacement,
            }
            | OperatorIntent::Rotate {
                target,
                replacement,
            } => {
                if target.is_zero() || replacement.is_zero() || target == replacement {
                    return Err(OperatorRuntimeError::InvalidRequest);
                }
            }
            OperatorIntent::Revoke { target } if target.is_zero() => {
                return Err(OperatorRuntimeError::InvalidRequest);
            }
            OperatorIntent::Revoke { .. } => {}
        }
        let fingerprint = semantic_fingerprint(&context, &intent);
        Ok(Self {
            context,
            intent,
            fingerprint,
        })
    }

    /// Invocation context.
    pub const fn context(&self) -> &OperatorInvocationContext {
        &self.context
    }

    /// Action-specific intent.
    pub const fn intent(&self) -> &OperatorIntent {
        &self.intent
    }

    /// Stable semantic digest used for idempotency conflict detection.
    pub const fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
}

fn semantic_fingerprint(context: &OperatorInvocationContext, intent: &OperatorIntent) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"buzz-operator-runtime-intent-v1");
    hasher.update(context.domain_id.as_bytes());
    hasher.update(context.operation_id.as_bytes());
    hasher.update(intent.action().discriminant().to_be_bytes());
    hasher.update(context.reason.discriminant().to_be_bytes());
    hasher.update(context.expected_revision.to_be_bytes());
    for approval in context.approval_references.iter() {
        hasher.update(approval.digest());
    }
    match intent {
        OperatorIntent::List { limit, after } => {
            hasher.update(limit.to_be_bytes());
            if let Some(after) = after {
                hasher.update(after.digest());
            }
        }
        OperatorIntent::Preview {
            target,
            replacement,
        }
        | OperatorIntent::Rotate {
            target,
            replacement,
        } => {
            hasher.update(target.digest());
            hasher.update(replacement.digest());
        }
        OperatorIntent::Revoke { target } => hasher.update(target.digest()),
    }
    hasher.finalize().into()
}

/// Redaction-safe facts supplied to the installed authenticator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorAuthorizationRequest {
    domain_id: Uuid,
    operation_id: Uuid,
    capability: OperatorCapability,
    intent_fingerprint: [u8; 32],
    replacement_reference: Option<OpaqueOperatorReference>,
}

impl OperatorAuthorizationRequest {
    fn from_invocation(invocation: &OperatorInvocation) -> Self {
        let replacement_reference = match invocation.intent {
            OperatorIntent::Preview { replacement, .. }
            | OperatorIntent::Rotate { replacement, .. } => Some(replacement),
            _ => None,
        };
        Self {
            domain_id: invocation.context.domain_id,
            operation_id: invocation.context.operation_id,
            capability: invocation.intent.action().capability(),
            intent_fingerprint: invocation.fingerprint,
            replacement_reference,
        }
    }

    /// Server-resolved authorization domain.
    pub const fn domain_id(self) -> Uuid {
        self.domain_id
    }

    /// Stable operation identity.
    pub const fn operation_id(self) -> Uuid {
        self.operation_id
    }

    /// Required capability.
    pub const fn capability(self) -> OperatorCapability {
        self.capability
    }

    /// Stable semantic fingerprint.
    pub const fn intent_fingerprint(self) -> [u8; 32] {
        self.intent_fingerprint
    }

    /// Requested replacement reference when a preview or rotation needs fresh proof.
    pub const fn replacement_reference(self) -> Option<OpaqueOperatorReference> {
        self.replacement_reference
    }
}

/// Fresh replacement material supplied by an authenticated preview or rotation grant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GrantedOperatorReplacement {
    reference: OpaqueOperatorReference,
    public_key: [u8; 32],
    policy_digest: [u8; 32],
}

impl GrantedOperatorReplacement {
    /// Bind a proven replacement key and policy revision to an opaque reference.
    pub fn new(
        reference: OpaqueOperatorReference,
        public_key: [u8; 32],
        policy_digest: [u8; 32],
    ) -> Result<Self, OperatorRuntimeError> {
        if reference.is_zero() || public_key == [0; 32] || policy_digest == [0; 32] {
            return Err(OperatorRuntimeError::InvalidAuthority);
        }
        Ok(Self {
            reference,
            public_key,
            policy_digest,
        })
    }

    /// Opaque replacement identity bound into the requested intent.
    pub const fn reference(self) -> OpaqueOperatorReference {
        self.reference
    }

    /// Fresh public key proven by the authenticator.
    pub const fn public_key(self) -> [u8; 32] {
        self.public_key
    }

    /// Digest of the policy revision that authorized the replacement.
    pub const fn policy_digest(self) -> [u8; 32] {
        self.policy_digest
    }
}

impl fmt::Debug for GrantedOperatorReplacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GrantedOperatorReplacement([redacted])")
    }
}

/// Authenticated capability grant returned by a deployment-owned verifier.
pub trait GrantedOperatorCapability: Send + Sync {
    /// Single-use authority evidence identity.
    fn authority_evidence_id(&self) -> Uuid;
    /// Authorization domain bound by the grant.
    fn domain_id(&self) -> Uuid;
    /// Stable operation identity bound by the grant.
    fn operation_id(&self) -> Uuid;
    /// Exact semantic intent fingerprint bound by the grant.
    fn intent_fingerprint(&self) -> [u8; 32];
    /// Pseudonymous actor reference stored in durable evidence.
    fn actor_reference(&self) -> OpaqueOperatorReference;
    /// Pseudonymous credential/provenance reference stored in durable evidence.
    fn provenance_reference(&self) -> OpaqueOperatorReference;
    /// Single-use approval evidence identities, parallel to request approvals.
    fn approval_evidence_ids(&self) -> &[Uuid];
    /// Fresh replacement proof for an exact preview or rotate intent, if any.
    fn replacement(&self) -> Option<GrantedOperatorReplacement>;
    /// Exclusive trusted expiry in Unix seconds.
    fn expires_at_unix_seconds(&self) -> u64;
    /// Whether this grant permits the exact closed capability.
    fn permits(&self, capability: OperatorCapability) -> bool;
}

/// Deployment-provided credential authenticator and capability source.
#[async_trait]
pub trait OperatorAuthenticator: Send + Sync {
    /// Authenticate sensitive credential material and return an intent-bound grant.
    async fn authenticate(
        &self,
        credential: &OperatorCredential,
        request: OperatorAuthorizationRequest,
    ) -> Result<Box<dyn GrantedOperatorCapability>, OperatorRuntimeError>;
}

/// Trusted time source used to reject authority stale at invocation time.
pub trait OperatorClock: Send + Sync {
    /// Current Unix time in seconds.
    fn now_unix_seconds(&self) -> Result<u64, OperatorRuntimeError>;
}

/// Fully authenticated operation passed to the durable executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedOperatorOperation {
    invocation: OperatorInvocation,
    authority_evidence_id: Uuid,
    actor_reference: OpaqueOperatorReference,
    provenance_reference: OpaqueOperatorReference,
    approval_evidence_ids: Box<[Uuid]>,
    expires_at_unix_seconds: u64,
    replacement: Option<GrantedOperatorReplacement>,
}

impl AuthorizedOperatorOperation {
    /// Authenticated invocation.
    pub const fn invocation(&self) -> &OperatorInvocation {
        &self.invocation
    }

    /// Single-use authority evidence identity.
    pub const fn authority_evidence_id(&self) -> Uuid {
        self.authority_evidence_id
    }

    /// Pseudonymous actor reference.
    pub const fn actor_reference(&self) -> OpaqueOperatorReference {
        self.actor_reference
    }

    /// Pseudonymous credential/provenance reference.
    pub const fn provenance_reference(&self) -> OpaqueOperatorReference {
        self.provenance_reference
    }

    /// Single-use approval evidence identities.
    pub fn approval_evidence_ids(&self) -> &[Uuid] {
        &self.approval_evidence_ids
    }

    /// Exclusive trusted authority expiry.
    pub const fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    /// Fresh replacement material for a preview or rotate operation.
    pub const fn replacement(&self) -> Option<GrantedOperatorReplacement> {
        self.replacement
    }
}

/// Authenticated denial facts accepted by the independent durable recorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedOperatorDenial {
    invocation: OperatorInvocation,
    actor_reference: OpaqueOperatorReference,
    provenance_reference: OpaqueOperatorReference,
    reason: OperatorRuntimeError,
}

impl AuthenticatedOperatorDenial {
    /// Denied invocation.
    pub const fn invocation(&self) -> &OperatorInvocation {
        &self.invocation
    }

    /// Pseudonymous actor reference.
    pub const fn actor_reference(&self) -> OpaqueOperatorReference {
        self.actor_reference
    }

    /// Pseudonymous credential/provenance reference.
    pub const fn provenance_reference(&self) -> OpaqueOperatorReference {
        self.provenance_reference
    }

    /// Closed denial reason.
    pub const fn reason(&self) -> OperatorRuntimeError {
        self.reason
    }
}

/// Redacted lifecycle state returned by listing operations.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorRecordState {
    /// Currently active.
    Active,
    /// Revoked but retained for attribution.
    Revoked,
    /// Replaced by a later binding.
    Rotated,
    /// Archived while lineage remains retained.
    Archived,
}

/// One redacted active or historical lifecycle record.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OperatorRecord {
    /// Pseudonymous record reference.
    pub reference: OpaqueOperatorReference,
    /// Redacted lifecycle state.
    pub state: OperatorRecordState,
    /// Monotonic record revision.
    pub revision: u64,
}

/// Stable result status for an idempotent operator operation.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperatorOutcomeStatus {
    /// Read-only list completed.
    Listed,
    /// Read-only preview completed.
    Previewed,
    /// Exact target was revoked.
    Revoked,
    /// Exact target was rotated.
    Rotated,
}

/// Redacted result returned by the durable executor.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OperatorOutcome {
    operation_id: Uuid,
    correlation_id: Uuid,
    action: OperatorAction,
    status: OperatorOutcomeStatus,
    affected_count: u32,
    lifecycle_revision: u64,
    records: Vec<OperatorRecord>,
}

impl OperatorOutcome {
    /// Construct a bounded result for one committed or read-only operation.
    pub fn new(
        operation_id: Uuid,
        correlation_id: Uuid,
        action: OperatorAction,
        status: OperatorOutcomeStatus,
        affected_count: u32,
        lifecycle_revision: u64,
        records: Vec<OperatorRecord>,
    ) -> Result<Self, OperatorRuntimeError> {
        if operation_id.is_nil()
            || correlation_id.is_nil()
            || lifecycle_revision == 0
            || affected_count > u32::from(MAX_LIST_LIMIT)
            || records.len() > usize::from(MAX_LIST_LIMIT)
            || records
                .iter()
                .any(|record| record.reference.is_zero() || record.revision == 0)
            || !matches!(
                (action, status),
                (OperatorAction::List, OperatorOutcomeStatus::Listed)
                    | (OperatorAction::Preview, OperatorOutcomeStatus::Previewed)
                    | (OperatorAction::Revoke, OperatorOutcomeStatus::Revoked)
                    | (OperatorAction::Rotate, OperatorOutcomeStatus::Rotated)
            )
            || (action != OperatorAction::List && !records.is_empty())
        {
            return Err(OperatorRuntimeError::ExecutorContract);
        }
        Ok(Self {
            operation_id,
            correlation_id,
            action,
            status,
            affected_count,
            lifecycle_revision,
            records,
        })
    }

    /// Stable operation identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Separate request-correlation identity.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Completed action.
    pub const fn action(&self) -> OperatorAction {
        self.action
    }
}

/// Durable, atomically audited, idempotent lifecycle executor.
#[async_trait]
pub trait DurableOperatorExecutor: Send + Sync {
    /// Execute or replay one operation by `(domain, operation_id, fingerprint)`.
    ///
    /// The implementation must return the original result for an identical
    /// retry and reject an operation ID reused with a different fingerprint.
    /// Mutations, receipt, audit outbox, invalidation, and post-commit effects
    /// must share one database transaction.
    async fn execute_idempotent(
        &self,
        operation: AuthorizedOperatorOperation,
    ) -> Result<OperatorOutcome, OperatorRuntimeError>;

    /// Record one authenticated denial without performing a lifecycle mutation.
    async fn record_denial(
        &self,
        denial: AuthenticatedOperatorDenial,
    ) -> Result<(), OperatorRuntimeError>;
}

/// Explicit composition root for the disabled operator lifecycle surface.
pub struct OperatorRuntime {
    authenticator: Arc<dyn OperatorAuthenticator>,
    executor: Arc<dyn DurableOperatorExecutor>,
    clock: Arc<dyn OperatorClock>,
}

impl OperatorRuntime {
    /// Construct a runtime from complete deployment-provided dependencies.
    pub fn new(
        authenticator: Arc<dyn OperatorAuthenticator>,
        executor: Arc<dyn DurableOperatorExecutor>,
        clock: Arc<dyn OperatorClock>,
    ) -> Self {
        Self {
            authenticator,
            executor,
            clock,
        }
    }

    /// Authenticate, capability-check, and invoke the durable idempotent executor.
    pub async fn invoke(
        &self,
        credential: &OperatorCredential,
        invocation: OperatorInvocation,
    ) -> Result<OperatorOutcome, OperatorRuntimeError> {
        let request = OperatorAuthorizationRequest::from_invocation(&invocation);
        let grant = self.authenticator.authenticate(credential, request).await?;
        let required = invocation.intent.action().capability();
        let now = self.clock.now_unix_seconds()?;
        let actor_reference = grant.actor_reference();
        let provenance_reference = grant.provenance_reference();
        if actor_reference.is_zero()
            || provenance_reference.is_zero()
            || actor_reference == provenance_reference
        {
            tracing::warn!(
                reason = OperatorRuntimeError::InvalidAuthority.code(),
                "operator denial could not be durably attributed"
            );
            return Err(OperatorRuntimeError::InvalidAuthority);
        }
        let deny = |reason| AuthenticatedOperatorDenial {
            invocation: invocation.clone(),
            actor_reference,
            provenance_reference,
            reason,
        };
        if grant.domain_id() != invocation.context.domain_id {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::CrossDomain))
                .await;
        }
        if grant.operation_id() != invocation.context.operation_id
            || grant.intent_fingerprint() != invocation.fingerprint
        {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::InvalidAuthority))
                .await;
        }
        if grant.expires_at_unix_seconds() <= now {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::StaleAuthority))
                .await;
        }
        if !grant.permits(required) {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::MissingCapability))
                .await;
        }
        let authority_evidence_id = grant.authority_evidence_id();
        let approval_evidence_ids = grant.approval_evidence_ids();
        let approvals = invocation.context.approval_references();
        let mut approval_ids = approval_evidence_ids.to_vec();
        approval_ids.sort_unstable();
        let invalid_approval_ids = authority_evidence_id.is_nil()
            || approval_evidence_ids.len() != approvals.len()
            || approval_evidence_ids.iter().any(Uuid::is_nil)
            || approval_ids.windows(2).any(|pair| pair[0] == pair[1]);
        if invalid_approval_ids {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::InvalidAuthority))
                .await;
        }
        if matches!(
            invocation.intent.action(),
            OperatorAction::Revoke | OperatorAction::Rotate
        ) && approvals.is_empty()
        {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::MissingApproval))
                .await;
        }
        if approvals.contains(&actor_reference) {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::SelfApproval))
                .await;
        }
        let replacement = grant.replacement();
        let expected_replacement = match invocation.intent {
            OperatorIntent::Preview { replacement, .. }
            | OperatorIntent::Rotate { replacement, .. } => Some(replacement),
            _ => None,
        };
        if replacement.map(GrantedOperatorReplacement::reference) != expected_replacement {
            return self
                .reject_authenticated(deny(OperatorRuntimeError::InvalidAuthority))
                .await;
        }
        let expected_operation_id = invocation.context.operation_id;
        let expected_correlation_id = invocation.context.correlation_id;
        let expected_action = invocation.intent.action();
        let outcome = self
            .executor
            .execute_idempotent(AuthorizedOperatorOperation {
                invocation,
                authority_evidence_id,
                actor_reference,
                provenance_reference,
                approval_evidence_ids: approval_evidence_ids.to_vec().into_boxed_slice(),
                expires_at_unix_seconds: grant.expires_at_unix_seconds(),
                replacement,
            })
            .await?;
        if outcome.operation_id() != expected_operation_id
            || outcome.correlation_id() != expected_correlation_id
            || outcome.action() != expected_action
        {
            return Err(OperatorRuntimeError::ExecutorContract);
        }
        Ok(outcome)
    }

    async fn reject_authenticated(
        &self,
        denial: AuthenticatedOperatorDenial,
    ) -> Result<OperatorOutcome, OperatorRuntimeError> {
        let reason = denial.reason();
        if self.executor.record_denial(denial).await.is_err() {
            tracing::warn!(
                reason = reason.code(),
                "operator denial evidence unavailable; request remains denied"
            );
        }
        Err(reason)
    }
}

/// Closed, redaction-safe failure returned by the operator runtime.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum OperatorRuntimeError {
    /// Authorization header is absent.
    #[error("operator credential is required")]
    MissingCredential,
    /// Authorization header is empty or exceeds its bound.
    #[error("operator credential is invalid")]
    InvalidCredential,
    /// Request identifiers, bounds, or intent shape are invalid.
    #[error("operator request is invalid")]
    InvalidRequest,
    /// Credential authentication failed.
    #[error("operator authentication failed")]
    Unauthenticated,
    /// Authenticated evidence crosses the requested domain.
    #[error("operator authority crosses the requested domain")]
    CrossDomain,
    /// Authenticated authority is stale.
    #[error("operator authority is stale")]
    StaleAuthority,
    /// Authenticated authority lacks the exact capability.
    #[error("operator capability is missing")]
    MissingCapability,
    /// A mutating lifecycle operation lacks independent approval evidence.
    #[error("operator independent approval is required")]
    MissingApproval,
    /// The authenticated actor attempted to approve its own operation.
    #[error("operator approval is not independent")]
    SelfApproval,
    /// Single-use authority or approval evidence was already consumed.
    #[error("operator authority evidence was replayed")]
    ReplayedAuthority,
    /// Authenticated grant contains invalid evidence references.
    #[error("operator authority is invalid")]
    InvalidAuthority,
    /// Operation ID was replayed with a different semantic intent.
    #[error("operator operation conflicts with an existing intent")]
    IdempotencyConflict,
    /// Durable storage could not accept the operation or denial evidence.
    #[error("operator durable storage is unavailable")]
    StorageUnavailable,
    /// Executor returned an outcome inconsistent with the request.
    #[error("operator executor contract failed")]
    ExecutorContract,
}

impl OperatorRuntimeError {
    /// Stable client-safe error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingCredential => "operator_credential_required",
            Self::InvalidCredential => "operator_credential_invalid",
            Self::InvalidRequest => "operator_request_invalid",
            Self::Unauthenticated => "operator_authentication_failed",
            Self::CrossDomain => "operator_cross_domain_denied",
            Self::StaleAuthority => "operator_authority_stale",
            Self::MissingCapability => "operator_capability_missing",
            Self::MissingApproval => "operator_approval_missing",
            Self::SelfApproval => "operator_self_approval_denied",
            Self::ReplayedAuthority => "operator_authority_replayed",
            Self::InvalidAuthority => "operator_authority_invalid",
            Self::IdempotencyConflict => "operator_idempotency_conflict",
            Self::StorageUnavailable => "operator_storage_unavailable",
            Self::ExecutorContract => "operator_executor_contract_failed",
        }
    }
}
