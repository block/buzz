//! Closed deployment-operator lifecycle intent contract.
//!
//! This module contains no actor minting, database writer, restore fence,
//! lifecycle mutation, or caller-supplied fingerprint constructor. The relay
//! may parse bounded request coordinates into an opaque intent; only the
//! origin-sealed verifier grant and canonical database transition added by the
//! operation-specific restore boundary may authorize it.
//!
//! Raw intent fields and the internal transition plan are not public:
//!
//! ```compile_fail
//! use buzz_db::operator_lifecycle::OperatorLifecycleIntent;
//! let _ = OperatorLifecycleIntent {};
//! ```
//!
//! Exact binding coordinates are constructor-validated and remain opaque:
//!
//! ```compile_fail
//! use buzz_db::operator_lifecycle::ExactActiveOperatorTarget;
//! let _ = ExactActiveOperatorTarget {};
//! ```

use std::fmt;

use buzz_auth::operator_lifecycle::{OperatorLifecycleAction, VerifiedOperatorLifecycleGrant};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::authorization_events::{
    record_authorization_event_tx, record_authorization_operation_receipt_tx,
    AuthorizationEventActor, AuthorizationEventKind, AuthorizationEventOutcome,
    AuthorizationEventWriteError, AuthorizationOperationKind, AuthorizationOperationOutcome,
    AuthorizationOperationReceipt, AuthorizationReasonCode, AuthorizationReceiptWrite,
    NewAuthorizationEvent,
};
use crate::lifecycle_invalidation::{
    advance_lifecycle_invalidation_tx, LifecycleInvalidationError, LifecycleInvalidationNotice,
    LifecycleInvalidationSelector,
};
use crate::{Db, DbError};

const MAX_OPERATOR_PROVISION_BODY_BYTES: usize = 64 * 1024;

/// Closed failures shared by request parsing and the later canonical executor.
#[derive(Debug, thiserror::Error)]
pub enum OperatorLifecycleError {
    /// The bounded request coordinates were invalid.
    #[error("invalid operator lifecycle request")]
    InvalidRequest,
    /// The verifier grant or locked lifecycle state denied the transition.
    #[error("operator lifecycle authorization denied")]
    AuthorizationDenied,
    /// The operation id is already bound to different canonical semantics.
    #[error("operator lifecycle operation intent conflict")]
    IntentConflict,
    /// Required authorization audit evidence could not be committed.
    #[error("operator lifecycle audit unavailable")]
    AuditUnavailable,
    /// Stored immutable canonical evidence was inconsistent.
    #[error("operator lifecycle stored result is corrupt")]
    CorruptStoredResult,
    /// Ordinary database failure; the canonical transaction did not commit.
    #[error("operator lifecycle storage unavailable")]
    Storage(#[source] DbError),
}

/// Atomic result of the verified operator lifecycle transaction.
pub enum OperatorLifecycleCommitOutcome {
    /// The lifecycle transition, invalidation, receipt, and audit committed.
    Committed {
        /// Dispatchable only after this fresh transaction commits.
        invalidation: LifecycleInvalidationNotice,
    },
    /// The exact operation was committed previously; no post-commit action runs.
    ExactReplay,
}

impl fmt::Debug for OperatorLifecycleCommitOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Committed { .. } => "OperatorLifecycleCommitOutcome::Committed([REDACTED])",
            Self::ExactReplay => "OperatorLifecycleCommitOutcome::ExactReplay",
        })
    }
}

impl From<DbError> for OperatorLifecycleError {
    fn from(value: DbError) -> Self {
        Self::Storage(value)
    }
}

/// Redaction-safe operation identity parsed from the exact request body.
#[derive(Clone, Copy)]
pub struct OperatorLifecycleOperation {
    community_id: CommunityId,
    operation_id: Uuid,
    correlation_id: Uuid,
    attempt_id: Uuid,
}

impl OperatorLifecycleOperation {
    /// Validate the server-resolved domain and bounded operation identifiers.
    pub fn new(
        community_id: CommunityId,
        operation_id: Uuid,
        correlation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<Self, OperatorLifecycleError> {
        if community_id.as_uuid().is_nil()
            || operation_id.is_nil()
            || correlation_id.is_nil()
            || attempt_id.is_nil()
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(Self {
            community_id,
            operation_id,
            correlation_id,
            attempt_id,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.community_id
    }

    /// Stable operation identity used by canonical idempotency.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Redaction-safe correlation identity.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Unique delivery-attempt identity.
    pub const fn attempt_id(&self) -> Uuid {
        self.attempt_id
    }
}

impl fmt::Debug for OperatorLifecycleOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperatorLifecycleOperation([REDACTED])")
    }
}

/// Exact immutable active binding generation named by a request.
#[derive(Clone, Copy)]
pub struct ExactActiveOperatorTarget {
    binding_id: Uuid,
    binding_version: u64,
    expected_lifecycle_revision: u64,
    principal_fingerprint: [u8; 32],
    event_author_pubkey: [u8; 32],
}

impl ExactActiveOperatorTarget {
    /// Validate an exact active generation (whose lifecycle revision is one).
    pub fn new(
        binding_id: Uuid,
        binding_version: u64,
        expected_lifecycle_revision: u64,
        principal_fingerprint: [u8; 32],
        event_author_pubkey: [u8; 32],
    ) -> Result<Self, OperatorLifecycleError> {
        if binding_id.is_nil()
            || binding_version == 0
            || expected_lifecycle_revision != 1
            || principal_fingerprint == [0; 32]
            || event_author_pubkey == [0; 32]
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(Self {
            binding_id,
            binding_version,
            expected_lifecycle_revision,
            principal_fingerprint,
            event_author_pubkey,
        })
    }

    /// Exact stable binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact immutable binding version.
    pub const fn binding_version(&self) -> u64 {
        self.binding_version
    }

    /// Lifecycle revision required by the transition.
    pub const fn expected_lifecycle_revision(&self) -> u64 {
        self.expected_lifecycle_revision
    }

    /// Provider-free principal fingerprint.
    pub const fn principal_fingerprint(&self) -> [u8; 32] {
        self.principal_fingerprint
    }

    /// Exact event-author public key.
    pub const fn event_author_pubkey(&self) -> [u8; 32] {
        self.event_author_pubkey
    }
}

impl fmt::Debug for ExactActiveOperatorTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExactActiveOperatorTarget([REDACTED])")
    }
}

/// Exact immutable retired binding generation named by recovery.
#[derive(Clone, Copy)]
pub struct ExactRetiredOperatorTarget {
    binding_id: Uuid,
    binding_version: u64,
    expected_lifecycle_revision: u64,
    principal_fingerprint: [u8; 32],
    event_author_pubkey: [u8; 32],
}

impl ExactRetiredOperatorTarget {
    /// Validate an exact retired generation (whose lifecycle revision is two).
    pub fn new(
        binding_id: Uuid,
        binding_version: u64,
        expected_lifecycle_revision: u64,
        principal_fingerprint: [u8; 32],
        event_author_pubkey: [u8; 32],
    ) -> Result<Self, OperatorLifecycleError> {
        if binding_id.is_nil()
            || binding_version == 0
            || expected_lifecycle_revision != 2
            || principal_fingerprint == [0; 32]
            || event_author_pubkey == [0; 32]
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(Self {
            binding_id,
            binding_version,
            expected_lifecycle_revision,
            principal_fingerprint,
            event_author_pubkey,
        })
    }

    /// Exact stable binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact immutable binding version.
    pub const fn binding_version(&self) -> u64 {
        self.binding_version
    }

    /// Lifecycle revision required by recovery.
    pub const fn expected_lifecycle_revision(&self) -> u64 {
        self.expected_lifecycle_revision
    }

    /// Provider-free principal fingerprint.
    pub const fn principal_fingerprint(&self) -> [u8; 32] {
        self.principal_fingerprint
    }

    /// Exact retired event-author public key.
    pub const fn event_author_pubkey(&self) -> [u8; 32] {
        self.event_author_pubkey
    }
}

impl fmt::Debug for ExactRetiredOperatorTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExactRetiredOperatorTarget([REDACTED])")
    }
}

/// Exact immutable open pending-replacement fact named by recovery.
#[derive(Clone, Copy)]
pub struct ExpectedOperatorPendingReplacement {
    target: ExactRetiredOperatorTarget,
    fact_generation: u64,
}

impl ExpectedOperatorPendingReplacement {
    /// Bind a retired generation to its exact open pending-fact generation.
    pub fn new(
        target: ExactRetiredOperatorTarget,
        fact_generation: u64,
    ) -> Result<Self, OperatorLifecycleError> {
        if fact_generation == 0 {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(Self {
            target,
            fact_generation,
        })
    }

    /// Exact retired target bound to this pending fact.
    pub const fn target(&self) -> ExactRetiredOperatorTarget {
        self.target
    }

    /// Positive immutable pending-fact generation.
    pub const fn fact_generation(&self) -> u64 {
        self.fact_generation
    }
}

impl fmt::Debug for ExpectedOperatorPendingReplacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExpectedOperatorPendingReplacement([REDACTED])")
    }
}

/// Requested successor key and exclusive expiry bound by replacement proof.
#[derive(Clone, Copy)]
pub struct RequestedOperatorReplacement {
    event_author_pubkey: [u8; 32],
    expires_at: DateTime<Utc>,
}

impl RequestedOperatorReplacement {
    /// Validate bounded successor coordinates without asserting proof validity.
    pub fn new(
        event_author_pubkey: [u8; 32],
        expires_at: DateTime<Utc>,
    ) -> Result<Self, OperatorLifecycleError> {
        if event_author_pubkey == [0; 32] || expires_at.timestamp_micros() <= 0 {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(Self {
            event_author_pubkey,
            expires_at,
        })
    }

    /// Requested successor event-author public key.
    pub const fn event_author_pubkey(&self) -> [u8; 32] {
        self.event_author_pubkey
    }

    /// Exclusive successor-proof expiry.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl fmt::Debug for RequestedOperatorReplacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RequestedOperatorReplacement([REDACTED])")
    }
}

#[derive(Clone)]
pub(crate) enum OperatorLifecyclePlan {
    Retire {
        target: ExactActiveOperatorTarget,
    },
    Disable {
        principal_fingerprint: [u8; 32],
        expected_active_binding: Option<ExactActiveOperatorTarget>,
    },
    Revoke {
        event_author_pubkey: [u8; 32],
        expected_active_binding: Option<ExactActiveOperatorTarget>,
    },
    Rotate {
        target: ExactActiveOperatorTarget,
        replacement: RequestedOperatorReplacement,
    },
    Recover {
        pending: ExpectedOperatorPendingReplacement,
        replacement: RequestedOperatorReplacement,
    },
    Enable {
        principal_fingerprint: [u8; 32],
        disabled_fact_generation: u64,
        pending: ExpectedOperatorPendingReplacement,
        replacement: RequestedOperatorReplacement,
    },
}

/// Opaque bounded six-action intent accepted by the verified operator seam.
///
/// Constructors validate request shape only. They confer no authority: the
/// later operation-specific restore boundary must exact-match this digest and
/// every coordinate to an origin-sealed verifier grant and locked DB state.
#[derive(Clone)]
pub struct OperatorLifecycleIntent {
    operation: OperatorLifecycleOperation,
    expected_revision: u64,
    action: OperatorLifecycleAction,
    intent_digest: [u8; 32],
    plan: OperatorLifecyclePlan,
}

impl OperatorLifecycleIntent {
    /// Prepare exact active-pair retirement.
    pub fn retire(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        target: ExactActiveOperatorTarget,
    ) -> Result<Self, OperatorLifecycleError> {
        if expected_revision != target.expected_lifecycle_revision {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Retire,
            OperatorLifecyclePlan::Retire { target },
        )
    }

    /// Prepare exact principal disablement, with an optional active target.
    pub fn disable(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        principal_fingerprint: [u8; 32],
        expected_active_binding: Option<ExactActiveOperatorTarget>,
    ) -> Result<Self, OperatorLifecycleError> {
        if principal_fingerprint == [0; 32]
            || expected_active_binding.is_some_and(|target| {
                target.principal_fingerprint != principal_fingerprint
                    || target.expected_lifecycle_revision != expected_revision
            })
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Disable,
            OperatorLifecyclePlan::Disable {
                principal_fingerprint,
                expected_active_binding,
            },
        )
    }

    /// Prepare permanent event-author key revocation.
    pub fn revoke(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        event_author_pubkey: [u8; 32],
        expected_active_binding: Option<ExactActiveOperatorTarget>,
    ) -> Result<Self, OperatorLifecycleError> {
        if event_author_pubkey == [0; 32]
            || expected_active_binding.is_some_and(|target| {
                target.event_author_pubkey != event_author_pubkey
                    || target.expected_lifecycle_revision != expected_revision
            })
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Revoke,
            OperatorLifecyclePlan::Revoke {
                event_author_pubkey,
                expected_active_binding,
            },
        )
    }

    /// Prepare exact active-binding rotation.
    pub fn rotate(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        target: ExactActiveOperatorTarget,
        replacement: RequestedOperatorReplacement,
    ) -> Result<Self, OperatorLifecycleError> {
        if expected_revision != target.expected_lifecycle_revision
            || target.event_author_pubkey == replacement.event_author_pubkey
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Rotate,
            OperatorLifecyclePlan::Rotate {
                target,
                replacement,
            },
        )
    }

    /// Prepare exact pending-lineage recovery.
    pub fn recover(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        pending: ExpectedOperatorPendingReplacement,
        replacement: RequestedOperatorReplacement,
    ) -> Result<Self, OperatorLifecycleError> {
        if expected_revision != pending.fact_generation
            || pending.target.event_author_pubkey == replacement.event_author_pubkey
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Recover,
            OperatorLifecyclePlan::Recover {
                pending,
                replacement,
            },
        )
    }

    /// Prepare exact disabled-principal enablement from one immutable retired
    /// binding generation and its exact open pending-replacement fact.
    pub fn enable(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        principal_fingerprint: [u8; 32],
        disabled_fact_generation: u64,
        pending: ExpectedOperatorPendingReplacement,
        replacement: RequestedOperatorReplacement,
    ) -> Result<Self, OperatorLifecycleError> {
        if principal_fingerprint == [0; 32]
            || disabled_fact_generation != expected_revision
            || pending.target.principal_fingerprint != principal_fingerprint
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Self::seal(
            operation,
            expected_revision,
            OperatorLifecycleAction::Enable,
            OperatorLifecyclePlan::Enable {
                principal_fingerprint,
                disabled_fact_generation,
                pending,
                replacement,
            },
        )
    }

    fn seal(
        operation: OperatorLifecycleOperation,
        expected_revision: u64,
        action: OperatorLifecycleAction,
        plan: OperatorLifecyclePlan,
    ) -> Result<Self, OperatorLifecycleError> {
        if expected_revision == 0 {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        let intent_digest = operator_intent_digest(operation, expected_revision, action, &plan);
        Ok(Self {
            operation,
            expected_revision,
            action,
            intent_digest,
            plan,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.operation.community_id
    }

    /// Stable operation identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation.operation_id
    }

    /// Redaction-safe correlation identity.
    pub const fn correlation_id(&self) -> Uuid {
        self.operation.correlation_id
    }

    /// Unique delivery-attempt identity.
    pub const fn attempt_id(&self) -> Uuid {
        self.operation.attempt_id
    }

    /// Closed action from `buzz-auth`.
    pub const fn action(&self) -> OperatorLifecycleAction {
        self.action
    }

    /// Exact expected lifecycle revision or selector generation.
    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    /// Canonical semantic digest that every operator proof must bind.
    pub const fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    /// Requested successor key for replacement actions.
    pub const fn replacement_event_author_pubkey(&self) -> Option<[u8; 32]> {
        match &self.plan {
            OperatorLifecyclePlan::Rotate { replacement, .. }
            | OperatorLifecyclePlan::Recover { replacement, .. }
            | OperatorLifecyclePlan::Enable { replacement, .. } => {
                Some(replacement.event_author_pubkey)
            }
            OperatorLifecyclePlan::Retire { .. }
            | OperatorLifecyclePlan::Disable { .. }
            | OperatorLifecyclePlan::Revoke { .. } => None,
        }
    }

    /// Exclusive requested successor-proof expiry for replacement actions.
    pub const fn replacement_expires_at(&self) -> Option<DateTime<Utc>> {
        match &self.plan {
            OperatorLifecyclePlan::Rotate { replacement, .. }
            | OperatorLifecyclePlan::Recover { replacement, .. }
            | OperatorLifecyclePlan::Enable { replacement, .. } => Some(replacement.expires_at),
            OperatorLifecyclePlan::Retire { .. }
            | OperatorLifecyclePlan::Disable { .. }
            | OperatorLifecyclePlan::Revoke { .. } => None,
        }
    }

    /// Recompute the sole canonical intent digest from one exact binding row
    /// while the canonical lifecycle transaction holds its locks.
    #[allow(clippy::too_many_arguments)]
    pub fn matches_locked_retiring_binding(
        &self,
        action: OperatorLifecycleAction,
        binding_id: Uuid,
        binding_version: u64,
        principal_fingerprint: [u8; 32],
        event_author_pubkey: [u8; 32],
        lifecycle_revision: u64,
        replacement: Option<([u8; 32], DateTime<Utc>)>,
    ) -> bool {
        let target = ExactActiveOperatorTarget {
            binding_id,
            binding_version,
            expected_lifecycle_revision: lifecycle_revision,
            principal_fingerprint,
            event_author_pubkey,
        };
        let plan = match (action, replacement) {
            (OperatorLifecycleAction::Retire, None) => OperatorLifecyclePlan::Retire { target },
            (OperatorLifecycleAction::Disable, None) => OperatorLifecyclePlan::Disable {
                principal_fingerprint,
                expected_active_binding: Some(target),
            },
            (OperatorLifecycleAction::Revoke, None) => OperatorLifecyclePlan::Revoke {
                event_author_pubkey,
                expected_active_binding: Some(target),
            },
            (OperatorLifecycleAction::Rotate, Some((event_author_pubkey, expires_at))) => {
                OperatorLifecyclePlan::Rotate {
                    target,
                    replacement: RequestedOperatorReplacement {
                        event_author_pubkey,
                        expires_at,
                    },
                }
            }
            _ => return false,
        };
        self.action == action
            && self.expected_revision == lifecycle_revision
            && self.intent_digest
                == operator_intent_digest(self.operation, lifecycle_revision, action, &plan)
    }
}

impl fmt::Debug for OperatorLifecycleIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperatorLifecycleIntent")
            .field("action", &self.action)
            .field("coordinates", &"[REDACTED]")
            .finish()
    }
}

/// Closed credential-free authentication failures. They never authorize an
/// operation or occupy its canonical result receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum OperatorAuthenticationDenialReason {
    /// The Authorization header was absent.
    MissingCredential = 1,
    /// The credential envelope was malformed or ambiguous.
    InvalidCredential = 2,
    /// The configured verifier rejected a well-formed credential.
    Unauthenticated = 3,
}

/// Durable outcome of one credential-free operator denial attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorPreauthDenialWrite {
    /// The immutable redacted denial event was inserted.
    Inserted,
    /// The exact denial attempt was already present.
    ExactReplay,
    /// The independent pre-authentication budget is full. Canonical audit
    /// health and capacity are unaffected.
    CapacityExhausted,
}

/// A durable pre-authentication denial write failed after preparation.
#[derive(Debug, thiserror::Error)]
pub enum OperatorPreauthDenialPersistenceError {
    /// PostgreSQL or the dedicated audit dependency was unavailable.
    #[error("operator pre-authentication denial audit unavailable")]
    Unavailable,
    /// Persisted immutable evidence did not match the sealed attempt.
    #[error("operator pre-authentication denial evidence is corrupt")]
    CorruptStoredResult,
}

#[derive(Clone)]
pub(crate) struct PreparedOperatorAuthenticationDenial {
    community_id: CommunityId,
    operation_id: Uuid,
    correlation_id: Uuid,
    attempt_id: Uuid,
    semantic_fingerprint: [u8; 32],
    expected_revision: u64,
    action: i16,
}

impl PreparedOperatorAuthenticationDenial {
    pub(crate) fn from_lifecycle_intent(
        intent: &OperatorLifecycleIntent,
    ) -> Result<Self, OperatorLifecycleError> {
        let prepared = Self {
            community_id: intent.authorization_domain(),
            operation_id: intent.operation_id(),
            correlation_id: intent.correlation_id(),
            attempt_id: intent.attempt_id(),
            semantic_fingerprint: intent.intent_digest(),
            expected_revision: intent.expected_revision(),
            action: intent.action() as i16,
        };
        prepared.validate()?;
        Ok(prepared)
    }

    fn validate(&self) -> Result<(), OperatorLifecycleError> {
        if self.community_id.as_uuid().is_nil()
            || self.operation_id.is_nil()
            || self.correlation_id.is_nil()
            || self.attempt_id.is_nil()
            || self.semantic_fingerprint == [0; 32]
            || self.expected_revision == 0
            || !(1..=8).contains(&self.action)
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        Ok(())
    }
}

impl fmt::Debug for PreparedOperatorAuthenticationDenial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedOperatorAuthenticationDenial([REDACTED])")
    }
}

/// Opaque redaction-safe Provision authentication attempt. It contains no
/// issuer, subject, selected key, credential, or raw request body.
///
/// ```compile_fail
/// use buzz_db::operator_lifecycle::OperatorProvisionAuthenticationAttempt;
/// let _ = OperatorProvisionAuthenticationAttempt {};
/// ```
pub struct OperatorProvisionAuthenticationAttempt {
    db: Db,
    prepared: PreparedOperatorAuthenticationDenial,
}

impl OperatorProvisionAuthenticationAttempt {
    /// Seal one bounded raw Provision body into credential-free denial evidence.
    pub fn from_bounded_body(
        db: Db,
        authorization_domain: CommunityId,
        request_body: &[u8],
    ) -> Result<Self, OperatorLifecycleError> {
        if authorization_domain.as_uuid().is_nil()
            || request_body.is_empty()
            || request_body.len() > MAX_OPERATOR_PROVISION_BODY_BYTES
        {
            return Err(OperatorLifecycleError::InvalidRequest);
        }
        let semantic_fingerprint = operator_digest(
            b"buzz:operator-provision-authentication-attempt:v1",
            &[authorization_domain.as_uuid().as_bytes(), request_body],
        );
        let canonical = serde_json::from_slice::<ProvisionDenialWireCoordinates>(request_body)
            .ok()
            .filter(|wire| {
                wire.authorization_domain == *authorization_domain.as_uuid()
                    && !wire.operation_id.is_nil()
                    && wire.policy_revision > 0
                    && valid_private_coordinate(&wire.issuer)
                    && valid_private_coordinate(&wire.subject)
                    && valid_hex_32(&wire.policy_digest)
                    && valid_hex_32(&wire.selected_event_author_pubkey)
            });
        let prepared = PreparedOperatorAuthenticationDenial {
            community_id: authorization_domain,
            operation_id: canonical.as_ref().map_or_else(
                || denial_uuid(b"operation", semantic_fingerprint),
                |wire| wire.operation_id,
            ),
            correlation_id: denial_uuid(b"correlation", semantic_fingerprint),
            attempt_id: denial_uuid(b"attempt", semantic_fingerprint),
            semantic_fingerprint,
            expected_revision: canonical.as_ref().map_or(1, |wire| wire.policy_revision),
            action: 2,
        };
        prepared.validate()?;
        Ok(Self { db, prepared })
    }

    /// Persist one origin-classified credential denial in the independent
    /// bounded pre-authentication ledger. The attempt is consumed so no
    /// caller can relabel one sealed body as multiple denial attempts.
    pub async fn persist_denial(
        self,
        reason: OperatorAuthenticationDenialReason,
    ) -> Result<OperatorPreauthDenialWrite, OperatorPreauthDenialPersistenceError> {
        self.db
            .record_operator_authentication_denial(&self.prepared, reason)
            .await
    }
}

impl fmt::Debug for OperatorProvisionAuthenticationAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperatorProvisionAuthenticationAttempt([REDACTED])")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvisionDenialWireCoordinates {
    authorization_domain: Uuid,
    operation_id: Uuid,
    policy_revision: u64,
    policy_digest: String,
    issuer: String,
    subject: String,
    selected_event_author_pubkey: String,
}

fn valid_private_coordinate(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024
}

fn valid_hex_32(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.as_bytes() != b"0000000000000000000000000000000000000000000000000000000000000000"
}

fn denial_uuid(label: &[u8], fingerprint: [u8; 32]) -> Uuid {
    let digest = operator_digest(
        b"buzz:operator-authentication-denial-uuid:v1",
        &[label, &fingerprint],
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

impl Db {
    /// Persist denial evidence for a parsed lifecycle intent without granting it authority.
    pub async fn record_operator_lifecycle_authentication_denial(
        &self,
        intent: &OperatorLifecycleIntent,
        reason: OperatorAuthenticationDenialReason,
    ) -> Result<OperatorPreauthDenialWrite, OperatorPreauthDenialPersistenceError> {
        let prepared = PreparedOperatorAuthenticationDenial::from_lifecycle_intent(intent)
            .map_err(|_| OperatorPreauthDenialPersistenceError::CorruptStoredResult)?;
        self.record_operator_authentication_denial(&prepared, reason)
            .await
    }

    /// Increment one fixed-cardinality, credential-free denial bucket without
    /// creating or occupying a canonical lifecycle receipt.
    pub(crate) async fn record_operator_authentication_denial(
        &self,
        prepared: &PreparedOperatorAuthenticationDenial,
        reason: OperatorAuthenticationDenialReason,
    ) -> Result<OperatorPreauthDenialWrite, OperatorPreauthDenialPersistenceError> {
        prepared
            .validate()
            .map_err(|_| OperatorPreauthDenialPersistenceError::CorruptStoredResult)?;
        let denial_class = match reason {
            OperatorAuthenticationDenialReason::MissingCredential => 1_i16,
            OperatorAuthenticationDenialReason::InvalidCredential => 2_i16,
            OperatorAuthenticationDenialReason::Unauthenticated => 4_i16,
        };
        let retained_count: i64 =
            sqlx::query_scalar("SELECT authorization_operator_denial_bucket_record_v1($1,$2,$3)")
                .bind(prepared.community_id.as_uuid())
                .bind(denial_class)
                .bind(prepared.action)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| OperatorPreauthDenialPersistenceError::Unavailable)?;
        if retained_count <= 0 {
            return Err(OperatorPreauthDenialPersistenceError::CorruptStoredResult);
        }
        Ok(OperatorPreauthDenialWrite::Inserted)
    }

    /// Commit one verifier-sealed lifecycle transition and all canonical
    /// companions in a single PostgreSQL transaction.
    ///
    /// The returned invalidation notice is post-commit work and exists only for
    /// a fresh commit. Exact replay repeats no transition, audit, selector, or
    /// notification dispatch.
    pub async fn commit_verified_operator_lifecycle(
        &self,
        intent: &OperatorLifecycleIntent,
        grant: &VerifiedOperatorLifecycleGrant,
    ) -> Result<OperatorLifecycleCommitOutcome, OperatorLifecycleError> {
        validate_operator_grant(intent, grant)?;
        let domain = intent.authorization_domain();
        let request_fingerprint = grant.request_attribution();
        let actor = AuthorizationEventActor::from_verified_operator_grant(grant)
            .map_err(OperatorLifecycleError::Storage)?;
        let result_digest = operator_result_digest(intent, grant);
        let receipt = AuthorizationOperationReceipt::new(
            domain,
            intent.operation_id(),
            request_fingerprint,
            operation_kind(intent.action()),
            actor.clone(),
            AuthorizationOperationOutcome::Applied,
            result_digest,
        )
        .map_err(OperatorLifecycleError::Storage)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| OperatorLifecycleError::Storage(error.into()))?;
        sqlx::query("SELECT nip_fi_lock_authorization_operation_v1($1,$2)")
            .bind(domain.as_uuid())
            .bind(intent.operation_id())
            .execute(&mut *transaction)
            .await
            .map_err(operator_storage)?;
        let authoritative_now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(operator_storage)?;
        if authoritative_now < grant.issued_at() || authoritative_now >= grant.expires_at() {
            return Err(OperatorLifecycleError::AuthorizationDenied);
        }

        match record_authorization_operation_receipt_tx(&mut transaction, &receipt).await {
            Err(DbError::InvalidData(message))
                if message == "authorization operation intent conflicts with prior receipt" =>
            {
                return Err(OperatorLifecycleError::IntentConflict);
            }
            Err(error) => return Err(OperatorLifecycleError::Storage(error)),
            Ok(AuthorizationReceiptWrite::Inserted) => {}
            Ok(AuthorizationReceiptWrite::ExactReplay) => {
                verify_operator_replay(&mut transaction, intent, grant, result_digest).await?;
                transaction.rollback().await.map_err(operator_storage)?;
                return Ok(OperatorLifecycleCommitOutcome::ExactReplay);
            }
        }

        let transition = execute_operator_transition_tx(
            &mut transaction,
            intent,
            grant,
            request_fingerprint,
            authoritative_now,
        )
        .await?;
        let invalidation = advance_lifecycle_invalidation_tx(
            &mut transaction,
            domain,
            intent.operation_id(),
            request_fingerprint,
            transition.invalidation_selectors,
        )
        .await
        .map_err(map_invalidation_error)?;
        let event = NewAuthorizationEvent::new(
            domain,
            Uuid::new_v4(),
            event_kind(intent.action()),
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Current,
            actor,
            transition.subject_fingerprint,
            intent.operation_id(),
            Some(request_fingerprint),
            intent.correlation_id(),
            intent.attempt_id(),
        )
        .map_err(OperatorLifecycleError::Storage)?;
        match record_authorization_event_tx(&mut transaction, &event).await {
            Ok(AuthorizationReceiptWrite::Inserted) => {}
            Ok(AuthorizationReceiptWrite::ExactReplay) => {
                return Err(OperatorLifecycleError::CorruptStoredResult);
            }
            Err(AuthorizationEventWriteError::CapacityUnavailable) => {
                return Err(OperatorLifecycleError::AuditUnavailable);
            }
            Err(AuthorizationEventWriteError::Database(error)) => {
                return Err(OperatorLifecycleError::Storage(error));
            }
        }
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *transaction)
            .await
            .map_err(map_operator_commit_error)?;
        transaction
            .commit()
            .await
            .map_err(map_operator_commit_error)?;
        Ok(OperatorLifecycleCommitOutcome::Committed { invalidation })
    }
}

struct OperatorTransitionResult {
    subject_fingerprint: Option<[u8; 32]>,
    invalidation_selectors: Vec<LifecycleInvalidationSelector>,
}

fn validate_operator_grant(
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
) -> Result<(), OperatorLifecycleError> {
    if grant.authorization_domain() != intent.authorization_domain()
        || grant.action() != intent.action()
        || grant.operation_id() != intent.operation_id()
        || grant.intent_digest() != intent.intent_digest()
        || grant.expected_revision() != intent.expected_revision()
        || grant.request_attribution() == [0; 32]
        || grant.approval_policy_digest() == [0; 32]
        || grant.signer_attribution() == [0; 32]
        || grant.issued_at() >= grant.expires_at()
        || grant.replacement_event_author_pubkey() != intent.replacement_event_author_pubkey()
        || grant.replacement_expires_at() != intent.replacement_expires_at()
        || grant.replacement_event_author_pubkey().is_some()
            != grant.replacement_proof_attribution().is_some()
    {
        return Err(OperatorLifecycleError::AuthorizationDenied);
    }
    Ok(())
}

fn operation_kind(action: OperatorLifecycleAction) -> AuthorizationOperationKind {
    match action {
        OperatorLifecycleAction::Retire => AuthorizationOperationKind::Retire,
        OperatorLifecycleAction::Disable => AuthorizationOperationKind::Disable,
        OperatorLifecycleAction::Revoke => AuthorizationOperationKind::Revoke,
        OperatorLifecycleAction::Rotate => AuthorizationOperationKind::Rotate,
        OperatorLifecycleAction::Recover => AuthorizationOperationKind::Recover,
        OperatorLifecycleAction::Enable => AuthorizationOperationKind::Enable,
    }
}

fn event_kind(action: OperatorLifecycleAction) -> AuthorizationEventKind {
    match action {
        OperatorLifecycleAction::Retire => AuthorizationEventKind::Retired,
        OperatorLifecycleAction::Disable => AuthorizationEventKind::PrincipalDisabled,
        OperatorLifecycleAction::Revoke => AuthorizationEventKind::Revoked,
        OperatorLifecycleAction::Rotate => AuthorizationEventKind::Rotated,
        OperatorLifecycleAction::Recover => AuthorizationEventKind::Recovered,
        OperatorLifecycleAction::Enable => AuthorizationEventKind::PrincipalEnabled,
    }
}

fn operator_result_digest(
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-lifecycle-result:v1",
        &[
            intent.authorization_domain().as_uuid().as_bytes(),
            intent.operation_id().as_bytes(),
            &(intent.action() as i16).to_be_bytes(),
            &intent.expected_revision().to_be_bytes(),
            &intent.intent_digest(),
            &grant.request_attribution(),
        ],
    )
}

fn operator_storage(error: sqlx::Error) -> OperatorLifecycleError {
    OperatorLifecycleError::Storage(error.into())
}

fn map_operator_commit_error(error: sqlx::Error) -> OperatorLifecycleError {
    if matches!(
        error
            .as_database_error()
            .and_then(|database| database.constraint()),
        Some(
            "authorization_event_capacity_policy_required"
                | "authorization_event_capacity_health"
                | "authorization_event_capacity_exhausted"
                | "authorization_events_envelope_size"
        )
    ) {
        OperatorLifecycleError::AuditUnavailable
    } else {
        operator_storage(error)
    }
}

fn map_invalidation_error(error: LifecycleInvalidationError) -> OperatorLifecycleError {
    match error {
        LifecycleInvalidationError::InvalidInput => OperatorLifecycleError::InvalidRequest,
        LifecycleInvalidationError::DomainInactive => OperatorLifecycleError::AuthorizationDenied,
        LifecycleInvalidationError::StorageUnavailable => OperatorLifecycleError::Storage(
            DbError::InvalidData("operator lifecycle invalidation unavailable".to_owned()),
        ),
    }
}

struct LockedOperatorBinding {
    binding_id: Uuid,
    binding_version: u64,
    issuer: String,
    subject: String,
    principal_fingerprint: [u8; 32],
    event_author_pubkey: [u8; 32],
    binding_state: i16,
    lifecycle_revision: u64,
    binding_provenance: i16,
    policy_revision: u64,
    enrollment_evidence_digest: [u8; 32],
    expires_at: Option<DateTime<Utc>>,
}

async fn verify_operator_replay(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
    result_digest: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let exact: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM authorization_operation_receipts receipt
            JOIN identity_lifecycle_history history
              ON history.community_id=receipt.community_id
             AND history.operation_id=receipt.operation_id
             AND history.request_fingerprint=receipt.request_fingerprint
            JOIN authorization_events event
              ON event.community_id=receipt.community_id
             AND event.operation_id=receipt.operation_id
             AND event.request_fingerprint=receipt.request_fingerprint
            WHERE receipt.community_id=$1 AND receipt.operation_id=$2
              AND receipt.request_fingerprint=$3
              AND receipt.operation_kind=$4 AND receipt.outcome_code=1
              AND receipt.result_digest=$5
              AND history.transition_kind=$4 AND history.outcome_code=1
              AND history.transition_digest=$6
              AND event.event_kind=$7 AND event.outcome_code=1
              AND event.reason_code=1
        )
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(intent.operation_id())
    .bind(grant.request_attribution().as_slice())
    .bind(operation_kind(intent.action()) as i16)
    .bind(result_digest.as_slice())
    .bind(operator_transition_digest(intent, grant).as_slice())
    .bind(event_kind(intent.action()) as i16)
    .fetch_one(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    if exact {
        Ok(())
    } else {
        Err(OperatorLifecycleError::IntentConflict)
    }
}

async fn execute_operator_transition_tx(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
    request_fingerprint: [u8; 32],
    authoritative_now: DateTime<Utc>,
) -> Result<OperatorTransitionResult, OperatorLifecycleError> {
    match &intent.plan {
        OperatorLifecyclePlan::Retire { target } => {
            let old = lock_exact_active_binding(transaction, intent, *target, None).await?;
            let history_id = Uuid::new_v4();
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                Some(&old),
                None,
                request_fingerprint,
            )
            .await?;
            retire_binding(transaction, intent.authorization_domain(), &old, history_id).await?;
            insert_retired_pair_selector(
                transaction,
                intent,
                &old,
                history_id,
                request_fingerprint,
            )
            .await?;
            insert_pending_selector(transaction, intent, &old, history_id, request_fingerprint)
                .await?;
            Ok(operator_transition_result(intent, &old))
        }
        OperatorLifecyclePlan::Disable {
            principal_fingerprint,
            expected_active_binding,
        } => {
            lock_lifecycle_coordinates(
                transaction,
                intent.authorization_domain(),
                Some(*principal_fingerprint),
                expected_active_binding.map(|target| target.event_author_pubkey),
            )
            .await?;
            let old = match expected_active_binding {
                Some(target) => Some(
                    lock_exact_active_binding(transaction, intent, *target, Some(false)).await?,
                ),
                None => {
                    ensure_no_active_principal(
                        transaction,
                        intent.authorization_domain(),
                        *principal_fingerprint,
                    )
                    .await?;
                    None
                }
            };
            let no_active_generation = if old.is_none() {
                let generation = next_selector_generation(
                    transaction,
                    intent.authorization_domain(),
                    2,
                    *principal_fingerprint,
                )
                .await?;
                if generation != intent.expected_revision() {
                    return Err(OperatorLifecycleError::AuthorizationDenied);
                }
                Some(generation)
            } else {
                None
            };
            let history_id = Uuid::new_v4();
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                old.as_ref(),
                None,
                request_fingerprint,
            )
            .await?;
            if let Some(old) = old.as_ref() {
                retire_binding(transaction, intent.authorization_domain(), old, history_id).await?;
                insert_retired_pair_selector(
                    transaction,
                    intent,
                    old,
                    history_id,
                    request_fingerprint,
                )
                .await?;
                insert_pending_selector(transaction, intent, old, history_id, request_fingerprint)
                    .await?;
            }
            insert_principal_selector(
                transaction,
                intent,
                *principal_fingerprint,
                no_active_generation,
                history_id,
                request_fingerprint,
            )
            .await?;
            Ok(match old.as_ref() {
                Some(old) => operator_transition_result(intent, old),
                None => OperatorTransitionResult {
                    subject_fingerprint: Some(*principal_fingerprint),
                    invalidation_selectors: vec![LifecycleInvalidationSelector::Principal(
                        *principal_fingerprint,
                    )],
                },
            })
        }
        OperatorLifecyclePlan::Revoke {
            event_author_pubkey,
            expected_active_binding,
        } => {
            lock_lifecycle_coordinates(
                transaction,
                intent.authorization_domain(),
                expected_active_binding.map(|target| target.principal_fingerprint),
                Some(*event_author_pubkey),
            )
            .await?;
            let old = match expected_active_binding {
                Some(target) => Some(
                    lock_exact_active_binding(transaction, intent, *target, Some(false)).await?,
                ),
                None => {
                    ensure_no_active_event_author(
                        transaction,
                        intent.authorization_domain(),
                        *event_author_pubkey,
                    )
                    .await?;
                    None
                }
            };
            if old.is_none() && intent.expected_revision() != 1 {
                return Err(OperatorLifecycleError::AuthorizationDenied);
            }
            let history_id = Uuid::new_v4();
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                old.as_ref(),
                None,
                request_fingerprint,
            )
            .await?;
            if let Some(old) = old.as_ref() {
                retire_binding(transaction, intent.authorization_domain(), old, history_id).await?;
                insert_retired_pair_selector(
                    transaction,
                    intent,
                    old,
                    history_id,
                    request_fingerprint,
                )
                .await?;
                insert_pending_selector(transaction, intent, old, history_id, request_fingerprint)
                    .await?;
            }
            insert_event_author_selector(
                transaction,
                intent,
                *event_author_pubkey,
                history_id,
                request_fingerprint,
            )
            .await?;
            Ok(match old.as_ref() {
                Some(old) => operator_transition_result(intent, old),
                None => OperatorTransitionResult {
                    subject_fingerprint: None,
                    invalidation_selectors: vec![LifecycleInvalidationSelector::EventAuthor(
                        event_author_selector_fingerprint(
                            intent.authorization_domain(),
                            *event_author_pubkey,
                        ),
                    )],
                },
            })
        }
        OperatorLifecyclePlan::Rotate {
            target,
            replacement,
        } => {
            let old = lock_exact_active_binding(transaction, intent, *target, None).await?;
            validate_replacement(grant, replacement, authoritative_now)?;
            lock_lifecycle_coordinates(
                transaction,
                intent.authorization_domain(),
                Some(old.principal_fingerprint),
                Some(replacement.event_author_pubkey),
            )
            .await?;
            let history_id = Uuid::new_v4();
            // The active-principal uniqueness constraint is immediate. Retire
            // the locked predecessor first so the successor can reuse its
            // principal coordinate; every later write remains in this same
            // transaction, so any failure restores the predecessor.
            retire_binding(transaction, intent.authorization_domain(), &old, history_id).await?;
            let successor = insert_successor_binding(
                transaction,
                intent,
                &old,
                replacement,
                history_id,
                request_fingerprint,
                authoritative_now,
            )
            .await?;
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                Some(&old),
                Some(successor),
                request_fingerprint,
            )
            .await?;
            insert_retired_pair_selector(
                transaction,
                intent,
                &old,
                history_id,
                request_fingerprint,
            )
            .await?;
            Ok(operator_transition_result(intent, &old))
        }
        OperatorLifecyclePlan::Recover {
            pending,
            replacement,
        } => {
            let old = lock_exact_retired_binding(transaction, intent, pending.target).await?;
            validate_replacement(grant, replacement, authoritative_now)?;
            let pending_selector =
                lock_pending_selector(transaction, intent, *pending, &old).await?;
            lock_lifecycle_coordinates(
                transaction,
                intent.authorization_domain(),
                Some(old.principal_fingerprint),
                Some(replacement.event_author_pubkey),
            )
            .await?;
            let history_id = Uuid::new_v4();
            let successor = insert_successor_binding(
                transaction,
                intent,
                &old,
                replacement,
                history_id,
                request_fingerprint,
                authoritative_now,
            )
            .await?;
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                Some(&old),
                Some(successor),
                request_fingerprint,
            )
            .await?;
            consume_selector(
                transaction,
                intent,
                pending_selector,
                4,
                history_id,
                successor,
                request_fingerprint,
            )
            .await?;
            Ok(operator_transition_result(intent, &old))
        }
        OperatorLifecyclePlan::Enable {
            principal_fingerprint,
            disabled_fact_generation,
            pending,
            replacement,
        } => {
            let old = lock_exact_retired_binding(transaction, intent, pending.target).await?;
            if old.principal_fingerprint != *principal_fingerprint {
                return Err(OperatorLifecycleError::AuthorizationDenied);
            }
            validate_replacement(grant, replacement, authoritative_now)?;
            let pending_selector =
                lock_pending_selector(transaction, intent, *pending, &old).await?;
            let disabled_selector = lock_principal_selector(
                transaction,
                intent,
                *principal_fingerprint,
                *disabled_fact_generation,
            )
            .await?;
            lock_lifecycle_coordinates(
                transaction,
                intent.authorization_domain(),
                Some(old.principal_fingerprint),
                Some(replacement.event_author_pubkey),
            )
            .await?;
            let history_id = Uuid::new_v4();
            let successor = insert_successor_binding(
                transaction,
                intent,
                &old,
                replacement,
                history_id,
                request_fingerprint,
                authoritative_now,
            )
            .await?;
            insert_lifecycle_history(
                transaction,
                intent,
                grant,
                history_id,
                Some(&old),
                Some(successor),
                request_fingerprint,
            )
            .await?;
            consume_selector(
                transaction,
                intent,
                disabled_selector,
                2,
                history_id,
                successor,
                request_fingerprint,
            )
            .await?;
            consume_selector(
                transaction,
                intent,
                pending_selector,
                4,
                history_id,
                successor,
                request_fingerprint,
            )
            .await?;
            Ok(operator_transition_result(intent, &old))
        }
    }
}

async fn lock_lifecycle_coordinates(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    principal_fingerprint: Option<[u8; 32]>,
    event_author_pubkey: Option<[u8; 32]>,
) -> Result<(), OperatorLifecycleError> {
    sqlx::query("SELECT identity_lifecycle_lock_coordinates_v1($1,$2,$3)")
        .bind(domain.as_uuid())
        .bind(principal_fingerprint.map(|value| value.to_vec()))
        .bind(event_author_pubkey.map(|value| value.to_vec()))
        .execute(&mut **transaction)
        .await
        .map_err(operator_storage)?;
    Ok(())
}

async fn lock_exact_active_binding(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    target: ExactActiveOperatorTarget,
    acquire_coordinates: Option<bool>,
) -> Result<LockedOperatorBinding, OperatorLifecycleError> {
    if acquire_coordinates != Some(false) {
        lock_lifecycle_coordinates(
            transaction,
            intent.authorization_domain(),
            Some(target.principal_fingerprint),
            Some(target.event_author_pubkey),
        )
        .await?;
    }
    let binding = lock_binding(
        transaction,
        intent.authorization_domain(),
        target.binding_id,
        target.binding_version,
    )
    .await?;
    if binding.binding_state != 1
        || binding.lifecycle_revision != target.expected_lifecycle_revision
        || binding.principal_fingerprint != target.principal_fingerprint
        || binding.event_author_pubkey != target.event_author_pubkey
    {
        return Err(OperatorLifecycleError::AuthorizationDenied);
    }
    Ok(binding)
}

async fn lock_exact_retired_binding(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    target: ExactRetiredOperatorTarget,
) -> Result<LockedOperatorBinding, OperatorLifecycleError> {
    lock_lifecycle_coordinates(
        transaction,
        intent.authorization_domain(),
        Some(target.principal_fingerprint),
        Some(target.event_author_pubkey),
    )
    .await?;
    let binding = lock_binding(
        transaction,
        intent.authorization_domain(),
        target.binding_id,
        target.binding_version,
    )
    .await?;
    if binding.binding_state != 2
        || binding.lifecycle_revision != target.expected_lifecycle_revision
        || binding.principal_fingerprint != target.principal_fingerprint
        || binding.event_author_pubkey != target.event_author_pubkey
    {
        return Err(OperatorLifecycleError::AuthorizationDenied);
    }
    Ok(binding)
}

async fn lock_binding(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    binding_id: Uuid,
    binding_version: u64,
) -> Result<LockedOperatorBinding, OperatorLifecycleError> {
    let binding_version =
        i64::try_from(binding_version).map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let row = sqlx::query(
        r#"
        SELECT binding_id,binding_version,issuer,subject,principal_fingerprint,
               event_author_pubkey,binding_state,lifecycle_revision,
               binding_provenance,policy_revision,enrollment_evidence_digest,expires_at
        FROM identity_bindings
        WHERE community_id=$1 AND binding_id=$2 AND binding_version=$3
        FOR UPDATE
        "#,
    )
    .bind(domain.as_uuid())
    .bind(binding_id)
    .bind(binding_version)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(operator_storage)?
    .ok_or(OperatorLifecycleError::AuthorizationDenied)?;
    Ok(LockedOperatorBinding {
        binding_id: row.try_get("binding_id").map_err(operator_storage_row)?,
        binding_version: positive_operator_i64(
            row.try_get("binding_version")
                .map_err(operator_storage_row)?,
        )?,
        issuer: row.try_get("issuer").map_err(operator_storage_row)?,
        subject: row.try_get("subject").map_err(operator_storage_row)?,
        principal_fingerprint: operator_bytes32(
            row.try_get("principal_fingerprint")
                .map_err(operator_storage_row)?,
        )?,
        event_author_pubkey: operator_bytes32(
            row.try_get("event_author_pubkey")
                .map_err(operator_storage_row)?,
        )?,
        binding_state: row.try_get("binding_state").map_err(operator_storage_row)?,
        lifecycle_revision: positive_operator_i64(
            row.try_get("lifecycle_revision")
                .map_err(operator_storage_row)?,
        )?,
        binding_provenance: row
            .try_get("binding_provenance")
            .map_err(operator_storage_row)?,
        policy_revision: positive_operator_i64(
            row.try_get("policy_revision")
                .map_err(operator_storage_row)?,
        )?,
        enrollment_evidence_digest: operator_bytes32(
            row.try_get("enrollment_evidence_digest")
                .map_err(operator_storage_row)?,
        )?,
        expires_at: row.try_get("expires_at").map_err(operator_storage_row)?,
    })
}

fn operator_storage_row(error: sqlx::Error) -> OperatorLifecycleError {
    operator_storage(error)
}

fn positive_operator_i64(value: i64) -> Result<u64, OperatorLifecycleError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(OperatorLifecycleError::CorruptStoredResult)
}

fn operator_bytes32(value: Vec<u8>) -> Result<[u8; 32], OperatorLifecycleError> {
    value
        .try_into()
        .map_err(|_| OperatorLifecycleError::CorruptStoredResult)
}

async fn ensure_no_active_principal(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    principal_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM identity_bindings \
         WHERE community_id=$1 AND principal_fingerprint=$2 AND binding_state=1)",
    )
    .bind(domain.as_uuid())
    .bind(principal_fingerprint.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    if present {
        Err(OperatorLifecycleError::AuthorizationDenied)
    } else {
        Ok(())
    }
}

async fn ensure_no_active_event_author(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    event_author_pubkey: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM identity_bindings \
         WHERE community_id=$1 AND event_author_pubkey=$2 AND binding_state=1)",
    )
    .bind(domain.as_uuid())
    .bind(event_author_pubkey.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    if present {
        Err(OperatorLifecycleError::AuthorizationDenied)
    } else {
        Ok(())
    }
}

async fn insert_lifecycle_history(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
    history_id: Uuid,
    old: Option<&LockedOperatorBinding>,
    successor: Option<(Uuid, u64)>,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let old_id = old.map(|binding| binding.binding_id);
    let old_version = old
        .map(|binding| i64::try_from(binding.binding_version))
        .transpose()
        .map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let old_revision = old
        .map(|binding| i64::try_from(binding.lifecycle_revision))
        .transpose()
        .map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let old_state = old.map(|binding| binding.binding_state);
    let (successor_id, successor_version) = successor
        .map(|(id, version)| {
            i64::try_from(version)
                .map(|version| (Some(id), Some(version)))
                .map_err(|_| OperatorLifecycleError::InvalidRequest)
        })
        .transpose()?
        .unwrap_or((None, None));
    sqlx::query(
        r#"
        INSERT INTO identity_lifecycle_history (
            community_id,history_id,transition_kind,outcome_code,
            old_binding_id,old_binding_version,old_prior_lifecycle_revision,
            old_prior_state,old_resulting_lifecycle_revision,old_resulting_state,
            successor_binding_id,successor_binding_version,
            successor_lifecycle_revision,successor_state,
            operation_id,request_fingerprint,transition_digest
        ) VALUES (
            $1,$2,$3,1,$4,$5,$6,$7,$8,$9,$10,$11,
            CASE WHEN $10::uuid IS NULL THEN NULL ELSE 1 END,
            CASE WHEN $10::uuid IS NULL THEN NULL ELSE 1 END,
            $12,$13,$14
        )
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(history_id)
    .bind(operation_kind(intent.action()) as i16)
    .bind(old_id)
    .bind(old_version)
    .bind(old_revision)
    .bind(old_state)
    .bind(old_revision.map(|revision| {
        if matches!(
            intent.action(),
            OperatorLifecycleAction::Recover | OperatorLifecycleAction::Enable
        ) {
            revision
        } else {
            2
        }
    }))
    .bind(old_state.map(|state| {
        if matches!(
            intent.action(),
            OperatorLifecycleAction::Recover | OperatorLifecycleAction::Enable
        ) {
            state
        } else {
            2
        }
    }))
    .bind(successor_id)
    .bind(successor_version)
    .bind(intent.operation_id())
    .bind(request_fingerprint.as_slice())
    .bind(operator_transition_digest(intent, grant).as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    Ok(())
}

async fn retire_binding(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    old: &LockedOperatorBinding,
    history_id: Uuid,
) -> Result<(), OperatorLifecycleError> {
    let updated = sqlx::query(
        "UPDATE identity_bindings SET binding_state=2,lifecycle_revision=2, \
         retirement_history_id=$4 WHERE community_id=$1 AND binding_id=$2 \
         AND binding_version=$3 AND binding_state=1 AND lifecycle_revision=1",
    )
    .bind(domain.as_uuid())
    .bind(old.binding_id)
    .bind(i64::try_from(old.binding_version).map_err(|_| OperatorLifecycleError::InvalidRequest)?)
    .bind(history_id)
    .execute(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    if updated.rows_affected() == 1 {
        Ok(())
    } else {
        Err(OperatorLifecycleError::AuthorizationDenied)
    }
}

fn validate_replacement(
    grant: &VerifiedOperatorLifecycleGrant,
    replacement: &RequestedOperatorReplacement,
    authoritative_now: DateTime<Utc>,
) -> Result<(), OperatorLifecycleError> {
    if grant.replacement_event_author_pubkey() != Some(replacement.event_author_pubkey)
        || grant.replacement_expires_at() != Some(replacement.expires_at)
        || grant.replacement_proof_attribution().is_none()
        || authoritative_now >= replacement.expires_at
    {
        Err(OperatorLifecycleError::AuthorizationDenied)
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_successor_binding(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    old: &LockedOperatorBinding,
    replacement: &RequestedOperatorReplacement,
    history_id: Uuid,
    request_fingerprint: [u8; 32],
    authoritative_now: DateTime<Utc>,
) -> Result<(Uuid, u64), OperatorLifecycleError> {
    let policy_expiry: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT expires_at FROM identity_enrollment_policies \
         WHERE community_id=$1 AND policy_revision=$2 FOR SHARE",
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(i64::try_from(old.policy_revision).map_err(|_| OperatorLifecycleError::InvalidRequest)?)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(operator_storage)?
    .ok_or(OperatorLifecycleError::AuthorizationDenied)?;
    let expires_at = [old.expires_at, policy_expiry, Some(replacement.expires_at)]
        .into_iter()
        .flatten()
        .min();
    if expires_at.is_some_and(|expires_at| authoritative_now >= expires_at) {
        return Err(OperatorLifecycleError::AuthorizationDenied);
    }
    let binding_id = Uuid::new_v4();
    let binding_version: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO identity_bindings (
            community_id,binding_id,issuer,subject,principal_fingerprint,
            event_author_pubkey,binding_state,lifecycle_revision,binding_provenance,
            policy_revision,enrollment_evidence_digest,expires_at,birth_history_id,
            creation_operation_id,creation_request_fingerprint
        ) VALUES ($1,$2,$3,$4,$5,$6,1,1,$7,$8,$9,$10,$11,$12,$13)
        RETURNING binding_version
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(binding_id)
    .bind(&old.issuer)
    .bind(&old.subject)
    .bind(old.principal_fingerprint.as_slice())
    .bind(replacement.event_author_pubkey.as_slice())
    .bind(old.binding_provenance)
    .bind(i64::try_from(old.policy_revision).map_err(|_| OperatorLifecycleError::InvalidRequest)?)
    .bind(old.enrollment_evidence_digest.as_slice())
    .bind(expires_at)
    .bind(history_id)
    .bind(intent.operation_id())
    .bind(request_fingerprint.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    Ok((binding_id, positive_operator_i64(binding_version)?))
}

fn operator_transition_digest(
    intent: &OperatorLifecycleIntent,
    grant: &VerifiedOperatorLifecycleGrant,
) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-lifecycle-transition:v1",
        &[
            &intent.intent_digest(),
            &grant.request_attribution(),
            &grant.approval_policy_digest(),
            &grant.signer_attribution(),
            &grant.approval_attribution().unwrap_or([0; 32]),
            &grant.replacement_proof_attribution().unwrap_or([0; 32]),
        ],
    )
}

fn operator_transition_result(
    intent: &OperatorLifecycleIntent,
    old: &LockedOperatorBinding,
) -> OperatorTransitionResult {
    OperatorTransitionResult {
        subject_fingerprint: Some(old.principal_fingerprint),
        invalidation_selectors: vec![
            LifecycleInvalidationSelector::Principal(old.principal_fingerprint),
            LifecycleInvalidationSelector::EventAuthor(event_author_selector_fingerprint(
                intent.authorization_domain(),
                old.event_author_pubkey,
            )),
            LifecycleInvalidationSelector::Binding {
                fingerprint: binding_selector_fingerprint(
                    intent.authorization_domain(),
                    old.binding_id,
                    old.binding_version,
                ),
                version_floor: old.binding_version,
            },
        ],
    }
}

async fn insert_retired_pair_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    old: &LockedOperatorBinding,
    history_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    insert_selector(
        transaction,
        intent,
        1,
        selector_fingerprint(
            intent.authorization_domain(),
            1,
            old.principal_fingerprint,
            old.event_author_pubkey,
            old.binding_id,
            old.binding_version,
            1,
        ),
        1,
        Some(old.principal_fingerprint),
        Some(old.event_author_pubkey),
        Some((old.binding_id, old.binding_version)),
        history_id,
        request_fingerprint,
    )
    .await
}

async fn insert_pending_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    old: &LockedOperatorBinding,
    history_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let generation = next_selector_generation(
        transaction,
        intent.authorization_domain(),
        4,
        old.principal_fingerprint,
    )
    .await?;
    insert_selector(
        transaction,
        intent,
        4,
        selector_fingerprint(
            intent.authorization_domain(),
            4,
            old.principal_fingerprint,
            old.event_author_pubkey,
            old.binding_id,
            old.binding_version,
            generation,
        ),
        generation,
        Some(old.principal_fingerprint),
        Some(old.event_author_pubkey),
        Some((old.binding_id, old.binding_version)),
        history_id,
        request_fingerprint,
    )
    .await
}

async fn insert_principal_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    principal_fingerprint: [u8; 32],
    prevalidated_generation: Option<u64>,
    history_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let generation = match prevalidated_generation {
        Some(generation) => generation,
        None => {
            next_selector_generation(
                transaction,
                intent.authorization_domain(),
                2,
                principal_fingerprint,
            )
            .await?
        }
    };
    insert_selector(
        transaction,
        intent,
        2,
        selector_fingerprint(
            intent.authorization_domain(),
            2,
            principal_fingerprint,
            [0; 32],
            Uuid::nil(),
            0,
            generation,
        ),
        generation,
        Some(principal_fingerprint),
        None,
        None,
        history_id,
        request_fingerprint,
    )
    .await
}

async fn insert_event_author_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    event_author_pubkey: [u8; 32],
    history_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    insert_selector(
        transaction,
        intent,
        3,
        selector_fingerprint(
            intent.authorization_domain(),
            3,
            [0; 32],
            event_author_pubkey,
            Uuid::nil(),
            0,
            1,
        ),
        1,
        None,
        Some(event_author_pubkey),
        None,
        history_id,
        request_fingerprint,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    kind: i16,
    fingerprint: [u8; 32],
    generation: u64,
    principal_fingerprint: Option<[u8; 32]>,
    event_author_pubkey: Option<[u8; 32]>,
    binding: Option<(Uuid, u64)>,
    history_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let generation =
        i64::try_from(generation).map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let (binding_id, binding_version) = binding
        .map(|(id, version)| {
            i64::try_from(version)
                .map(|version| (Some(id), Some(version)))
                .map_err(|_| OperatorLifecycleError::InvalidRequest)
        })
        .transpose()?
        .unwrap_or((None, None));
    sqlx::query(
        r#"
        INSERT INTO identity_lifecycle_selectors (
            community_id,selector_id,selector_kind,selector_fingerprint,
            fact_generation,principal_fingerprint,event_author_pubkey,
            binding_id,binding_version,asserted_history_id,
            selected_by_operation_id,selected_by_request_fingerprint
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(Uuid::new_v4())
    .bind(kind)
    .bind(fingerprint.as_slice())
    .bind(generation)
    .bind(principal_fingerprint.map(|value| value.to_vec()))
    .bind(event_author_pubkey.map(|value| value.to_vec()))
    .bind(binding_id)
    .bind(binding_version)
    .bind(history_id)
    .bind(intent.operation_id())
    .bind(request_fingerprint.as_slice())
    .execute(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    Ok(())
}

async fn next_selector_generation(
    transaction: &mut Transaction<'_, Postgres>,
    domain: CommunityId,
    kind: i16,
    principal_fingerprint: [u8; 32],
) -> Result<u64, OperatorLifecycleError> {
    let maximum: Option<i64> = sqlx::query_scalar(
        "SELECT max(fact_generation) FROM identity_lifecycle_selectors \
         WHERE community_id=$1 AND selector_kind=$2 AND principal_fingerprint=$3",
    )
    .bind(domain.as_uuid())
    .bind(kind)
    .bind(principal_fingerprint.as_slice())
    .fetch_one(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    let next = maximum
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(OperatorLifecycleError::CorruptStoredResult)?;
    positive_operator_i64(next)
}

fn selector_fingerprint(
    domain: CommunityId,
    kind: i16,
    principal_fingerprint: [u8; 32],
    event_author_pubkey: [u8; 32],
    binding_id: Uuid,
    binding_version: u64,
    generation: u64,
) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-lifecycle-selector:v1",
        &[
            domain.as_uuid().as_bytes(),
            &kind.to_be_bytes(),
            &principal_fingerprint,
            &event_author_pubkey,
            binding_id.as_bytes(),
            &binding_version.to_be_bytes(),
            &generation.to_be_bytes(),
        ],
    )
}

fn event_author_selector_fingerprint(
    domain: CommunityId,
    event_author_pubkey: [u8; 32],
) -> [u8; 32] {
    operator_digest(
        b"buzz:authorization-invalidation-event-author:v1",
        &[domain.as_uuid().as_bytes(), &event_author_pubkey],
    )
}

fn binding_selector_fingerprint(
    domain: CommunityId,
    binding_id: Uuid,
    binding_version: u64,
) -> [u8; 32] {
    operator_digest(
        b"buzz:authorization-invalidation-binding:v1",
        &[
            domain.as_uuid().as_bytes(),
            binding_id.as_bytes(),
            &binding_version.to_be_bytes(),
        ],
    )
}

async fn lock_pending_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    pending: ExpectedOperatorPendingReplacement,
    old: &LockedOperatorBinding,
) -> Result<Uuid, OperatorLifecycleError> {
    let generation = i64::try_from(pending.fact_generation)
        .map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let selector: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT selector.selector_id
        FROM identity_lifecycle_selectors selector
        LEFT JOIN identity_lifecycle_selector_consumptions consumption
          ON consumption.community_id=selector.community_id
         AND consumption.selector_id=selector.selector_id
        WHERE selector.community_id=$1 AND selector.selector_kind=4
          AND selector.principal_fingerprint=$2
          AND selector.event_author_pubkey=$3
          AND selector.binding_id=$4 AND selector.binding_version=$5
          AND selector.fact_generation=$6
          AND consumption.selector_id IS NULL
        FOR UPDATE OF selector
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(old.principal_fingerprint.as_slice())
    .bind(old.event_author_pubkey.as_slice())
    .bind(old.binding_id)
    .bind(i64::try_from(old.binding_version).map_err(|_| OperatorLifecycleError::InvalidRequest)?)
    .bind(generation)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    selector.ok_or(OperatorLifecycleError::AuthorizationDenied)
}

async fn lock_principal_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    principal_fingerprint: [u8; 32],
    fact_generation: u64,
) -> Result<Uuid, OperatorLifecycleError> {
    let fact_generation =
        i64::try_from(fact_generation).map_err(|_| OperatorLifecycleError::InvalidRequest)?;
    let selector: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT selector.selector_id
        FROM identity_lifecycle_selectors selector
        LEFT JOIN identity_lifecycle_selector_consumptions consumption
          ON consumption.community_id=selector.community_id
         AND consumption.selector_id=selector.selector_id
        WHERE selector.community_id=$1 AND selector.selector_kind=2
          AND selector.principal_fingerprint=$2
          AND selector.fact_generation=$3
          AND consumption.selector_id IS NULL
        FOR UPDATE OF selector
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(principal_fingerprint.as_slice())
    .bind(fact_generation)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    selector.ok_or(OperatorLifecycleError::AuthorizationDenied)
}

#[allow(clippy::too_many_arguments)]
async fn consume_selector(
    transaction: &mut Transaction<'_, Postgres>,
    intent: &OperatorLifecycleIntent,
    selector_id: Uuid,
    selector_kind: i16,
    history_id: Uuid,
    successor: (Uuid, u64),
    request_fingerprint: [u8; 32],
) -> Result<(), OperatorLifecycleError> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO identity_lifecycle_selector_consumptions (
            community_id,selector_id,selector_kind,history_id,operation_id,
            request_fingerprint,successor_binding_id,successor_binding_version
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
        ON CONFLICT (community_id,selector_id) DO NOTHING
        "#,
    )
    .bind(intent.authorization_domain().as_uuid())
    .bind(selector_id)
    .bind(selector_kind)
    .bind(history_id)
    .bind(intent.operation_id())
    .bind(request_fingerprint.as_slice())
    .bind(successor.0)
    .bind(i64::try_from(successor.1).map_err(|_| OperatorLifecycleError::InvalidRequest)?)
    .execute(&mut **transaction)
    .await
    .map_err(operator_storage)?;
    if inserted.rows_affected() == 1 {
        Ok(())
    } else {
        Err(OperatorLifecycleError::AuthorizationDenied)
    }
}

fn operator_intent_digest(
    operation: OperatorLifecycleOperation,
    expected_revision: u64,
    action: OperatorLifecycleAction,
    plan: &OperatorLifecyclePlan,
) -> [u8; 32] {
    let plan_digest = match plan {
        OperatorLifecyclePlan::Retire { target } => operator_digest(
            b"buzz:operator-retire-intent:v1",
            &[&active_target_digest(*target)],
        ),
        OperatorLifecyclePlan::Disable {
            principal_fingerprint,
            expected_active_binding,
        } => operator_digest(
            b"buzz:operator-disable-intent:v1",
            &[
                principal_fingerprint,
                &expected_active_binding
                    .map(active_target_digest)
                    .unwrap_or([0; 32]),
            ],
        ),
        OperatorLifecyclePlan::Revoke {
            event_author_pubkey,
            expected_active_binding,
        } => operator_digest(
            b"buzz:operator-revoke-intent:v1",
            &[
                event_author_pubkey,
                &expected_active_binding
                    .map(active_target_digest)
                    .unwrap_or([0; 32]),
            ],
        ),
        OperatorLifecyclePlan::Rotate {
            target,
            replacement,
        } => operator_digest(
            b"buzz:operator-rotate-intent:v1",
            &[
                &active_target_digest(*target),
                &replacement_digest(replacement),
            ],
        ),
        OperatorLifecyclePlan::Recover {
            pending,
            replacement,
        } => operator_digest(
            b"buzz:operator-recover-intent:v1",
            &[&pending_digest(*pending), &replacement_digest(replacement)],
        ),
        OperatorLifecyclePlan::Enable {
            principal_fingerprint,
            disabled_fact_generation,
            pending,
            replacement,
        } => operator_digest(
            b"buzz:operator-enable-intent:v1",
            &[
                principal_fingerprint,
                &disabled_fact_generation.to_be_bytes(),
                &pending_digest(*pending),
                &replacement_digest(replacement),
            ],
        ),
    };
    operator_digest(
        b"buzz:operator-lifecycle-intent:v1",
        &[
            operation.community_id.as_uuid().as_bytes(),
            operation.operation_id.as_bytes(),
            &(action as i16).to_be_bytes(),
            &expected_revision.to_be_bytes(),
            &plan_digest,
        ],
    )
}

fn active_target_digest(target: ExactActiveOperatorTarget) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-active-target:v1",
        &[
            target.binding_id.as_bytes(),
            &target.binding_version.to_be_bytes(),
            &target.expected_lifecycle_revision.to_be_bytes(),
            &target.principal_fingerprint,
            &target.event_author_pubkey,
        ],
    )
}

fn retired_target_digest(target: ExactRetiredOperatorTarget) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-retired-target:v1",
        &[
            target.binding_id.as_bytes(),
            &target.binding_version.to_be_bytes(),
            &target.expected_lifecycle_revision.to_be_bytes(),
            &target.principal_fingerprint,
            &target.event_author_pubkey,
        ],
    )
}

fn pending_digest(pending: ExpectedOperatorPendingReplacement) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-pending-replacement:v1",
        &[
            &retired_target_digest(pending.target),
            &pending.fact_generation.to_be_bytes(),
        ],
    )
}

fn replacement_digest(replacement: &RequestedOperatorReplacement) -> [u8; 32] {
    operator_digest(
        b"buzz:operator-replacement:v1",
        &[
            &replacement.event_author_pubkey,
            &replacement.expires_at.timestamp_micros().to_be_bytes(),
        ],
    )
}

fn operator_digest(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use buzz_auth::{
        operator_lifecycle::{OperatorLifecycleVerifier, OperatorProofSetReplayGuard},
        AuthError,
    };
    use chrono::Duration;
    use nostr::{EventBuilder, EventId, Keys, Kind, Tag, Timestamp};

    use super::*;

    fn loopback_test_database_url() -> String {
        format!(
            "{}://{}:{}@{}:{}/{}",
            "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
        )
    }

    struct PermissiveOperatorGuard;

    impl OperatorProofSetReplayGuard for PermissiveOperatorGuard {
        fn try_admit_quota<'a>(
            &'a self,
            _quota_scope: &'a str,
            _signer_attribution: &'a [u8; 32],
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async { Ok(true) })
        }

        fn try_mark_all_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_ids: &'a [EventId],
            _ttl_secs: u64,
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async { Ok(true) })
        }
    }

    fn operation(domain: CommunityId, operation_id: Uuid) -> OperatorLifecycleOperation {
        OperatorLifecycleOperation::new(domain, operation_id, Uuid::new_v4(), Uuid::new_v4())
            .expect("operation")
    }

    fn active() -> ExactActiveOperatorTarget {
        ExactActiveOperatorTarget::new(Uuid::new_v4(), 7, 1, [11; 32], [12; 32])
            .expect("active target")
    }

    fn retired() -> ExactRetiredOperatorTarget {
        ExactRetiredOperatorTarget::new(Uuid::new_v4(), 7, 2, [11; 32], [12; 32])
            .expect("retired target")
    }

    fn replacement() -> RequestedOperatorReplacement {
        RequestedOperatorReplacement::new([13; 32], Utc::now() + Duration::minutes(5))
            .expect("replacement")
    }

    fn signed_operator_request(
        keys: &Keys,
        url: &str,
        body: &[u8],
        intent_digest: [u8; 32],
        created_at: Timestamp,
    ) -> String {
        let payload = hex::encode(<[u8; 32]>::from(Sha256::digest(body)));
        let intent = hex::encode(intent_digest);
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url]).expect("url tag"),
                Tag::parse(["method", "POST"]).expect("method tag"),
                Tag::parse(["payload", payload.as_str()]).expect("payload tag"),
                Tag::parse(["buzz-intent", intent.as_str()]).expect("intent tag"),
            ])
            .custom_created_at(created_at)
            .sign_with_keys(keys)
            .expect("sign operator request");
        serde_json::to_string(&event).expect("serialize operator request")
    }

    async fn seed_active_operator_binding(
        pool: &sqlx::PgPool,
        domain: CommunityId,
        actor_keys: &Keys,
        discriminator: u8,
    ) -> ExactActiveOperatorTarget {
        let binding_id = Uuid::new_v4();
        let enrollment_operation = Uuid::new_v4();
        let enrollment_request = [discriminator; 32];
        let enrollment_history = Uuid::new_v4();
        let principal_fingerprint = [discriminator.wrapping_add(1); 32];
        let event_author = actor_keys.public_key().to_bytes();
        let actor_fingerprint = [discriminator.wrapping_add(2); 32];
        let enrollment_result = [discriminator.wrapping_add(3); 32];
        let enrollment_envelope = vec![discriminator.wrapping_add(4); 64];
        let envelope_digest: [u8; 32] = Sha256::digest(&enrollment_envelope).into();
        let mut seed = pool.begin().await.expect("begin active binding seed");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,1,$4,1,$5)",
        )
        .bind(domain.as_uuid())
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind(actor_fingerprint.as_slice())
        .bind(enrollment_result.as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert active binding receipt");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,$3,$4,$5,$6,1,1,1,1,$7,$8,$9,$10) \
             RETURNING binding_version",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .bind(format!("issuer-{discriminator}"))
        .bind(format!("subject-{discriminator}"))
        .bind(principal_fingerprint.as_slice())
        .bind(event_author.as_slice())
        .bind([discriminator.wrapping_add(5); 32].as_slice())
        .bind(enrollment_history)
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .fetch_one(&mut *seed)
        .await
        .expect("insert active binding");
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
              successor_binding_version,successor_lifecycle_revision,successor_state, \
              operation_id,request_fingerprint,transition_digest) \
             VALUES ($1,$2,1,1,$3,$4,1,1,$5,$6,$7)",
        )
        .bind(domain.as_uuid())
        .bind(enrollment_history)
        .bind(binding_id)
        .bind(binding_version)
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind([discriminator.wrapping_add(6); 32].as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert active binding history");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
              actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
              occurred_at,canonical_envelope,envelope_digest) \
             VALUES ($1,$2,1,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(domain.as_uuid())
        .bind(Uuid::new_v4())
        .bind(actor_fingerprint.as_slice())
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(&enrollment_envelope)
        .bind(envelope_digest.as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert active binding audit");
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *seed)
            .await
            .expect("validate active binding seed");
        seed.commit().await.expect("commit active binding seed");
        ExactActiveOperatorTarget::new(
            binding_id,
            u64::try_from(binding_version).expect("positive binding version"),
            1,
            principal_fingerprint,
            event_author,
        )
        .expect("exact active seed target")
    }

    fn rotate_body(
        operation_id: Uuid,
        replacement: &Keys,
        replacement_expires_at: DateTime<Utc>,
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "operation": { "operation_id": operation_id },
            "expected_revision": 1,
            "replacement": {
                "event_author_pubkey": hex::encode(replacement.public_key().to_bytes()),
                "expires_at": replacement_expires_at,
            },
        }))
        .expect("serialize rotate body")
    }

    #[allow(clippy::too_many_arguments)]
    async fn verified_rotate_grant(
        verifier: &OperatorLifecycleVerifier,
        guard: &PermissiveOperatorGuard,
        signer: &Keys,
        replacement: &Keys,
        url: &str,
        body: &[u8],
        intent: &OperatorLifecycleIntent,
        replacement_expires_at: DateTime<Utc>,
        created_at: Timestamp,
    ) -> VerifiedOperatorLifecycleGrant {
        let primary =
            signed_operator_request(signer, url, body, intent.intent_digest(), created_at);
        let replacement_proof =
            signed_operator_request(replacement, url, body, intent.intent_digest(), created_at);
        verifier
            .verify_lifecycle(
                &primary,
                None,
                Some(&replacement_proof),
                url,
                body,
                guard,
                intent.authorization_domain(),
                OperatorLifecycleAction::Rotate,
                intent.operation_id(),
                intent.intent_digest(),
                1,
                Some(replacement.public_key().to_bytes()),
                Some(replacement_expires_at),
            )
            .await
            .expect("verify rotate grant")
    }

    #[test]
    fn all_six_actions_are_typed_and_redacted() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let active = active();
        let pending = ExpectedOperatorPendingReplacement::new(retired(), 3).expect("pending");
        let intents = [
            OperatorLifecycleIntent::retire(operation(domain, Uuid::new_v4()), 1, active),
            OperatorLifecycleIntent::disable(
                operation(domain, Uuid::new_v4()),
                1,
                [11; 32],
                Some(active),
            ),
            OperatorLifecycleIntent::revoke(
                operation(domain, Uuid::new_v4()),
                1,
                [12; 32],
                Some(active),
            ),
            OperatorLifecycleIntent::rotate(
                operation(domain, Uuid::new_v4()),
                1,
                active,
                replacement(),
            ),
            OperatorLifecycleIntent::recover(
                operation(domain, Uuid::new_v4()),
                3,
                pending,
                replacement(),
            ),
            OperatorLifecycleIntent::enable(
                operation(domain, Uuid::new_v4()),
                4,
                [11; 32],
                4,
                pending,
                replacement(),
            ),
        ];
        for intent in intents {
            let intent = intent.expect("typed intent");
            assert_ne!(intent.intent_digest(), [0; 32]);
            let debug = format!("{intent:?}");
            assert!(debug.contains("[REDACTED]"));
            assert!(!debug.contains(&intent.operation_id().to_string()));
        }
    }

    #[test]
    fn semantic_digest_ignores_delivery_ids_but_binds_operation_and_target() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let target = active();
        let first = OperatorLifecycleIntent::retire(operation(domain, operation_id), 1, target)
            .expect("first");
        let retry = OperatorLifecycleIntent::retire(operation(domain, operation_id), 1, target)
            .expect("retry");
        assert_eq!(first.intent_digest(), retry.intent_digest());

        let changed = OperatorLifecycleIntent::retire(operation(domain, operation_id), 1, active())
            .expect("changed");
        assert_ne!(first.intent_digest(), changed.intent_digest());
    }

    #[test]
    fn invalid_coordinates_fail_before_fingerprinting() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        assert!(OperatorLifecycleOperation::new(
            domain,
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::new_v4()
        )
        .is_err());
        assert!(ExactActiveOperatorTarget::new(Uuid::new_v4(), 1, 2, [1; 32], [2; 32]).is_err());
        assert!(ExactRetiredOperatorTarget::new(Uuid::new_v4(), 1, 1, [1; 32], [2; 32]).is_err());
        assert!(RequestedOperatorReplacement::new([0; 32], Utc::now()).is_err());
    }

    #[test]
    fn enable_requires_and_binds_exact_pending_lineage() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let pending = ExpectedOperatorPendingReplacement::new(retired(), 7).expect("pending");
        let intent = OperatorLifecycleIntent::enable(
            operation(domain, Uuid::new_v4()),
            4,
            [11; 32],
            4,
            pending,
            replacement(),
        )
        .expect("exact pending intent");
        assert_eq!(intent.action(), OperatorLifecycleAction::Enable);
        assert_ne!(intent.intent_digest(), [0; 32]);

        let changed = OperatorLifecycleIntent::enable(
            operation(domain, intent.operation_id()),
            4,
            [11; 32],
            4,
            ExpectedOperatorPendingReplacement::new(retired(), 8).expect("changed pending"),
            replacement(),
        )
        .expect("changed pending intent");
        assert_ne!(intent.intent_digest(), changed.intent_digest());
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn postgres_verified_retire_commits_transition_invalidation_receipt_and_audit() {
        use sqlx::postgres::PgPoolOptions;

        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect operator test admin");
        let database_name = format!("s3_operator_lifecycle_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("create disposable operator database");
        let split = admin_url.rfind('/').expect("database URL path");
        let database_url = format!("{}/{}", &admin_url[..split], database_name);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect disposable operator database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply canonical migrations");

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("operator-{}.example", domain.as_uuid().simple()))
            .execute(&pool)
            .await
            .expect("insert operator community");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,1,$2,transaction_timestamp()-interval '1 second')",
        )
        .bind(domain.as_uuid())
        .bind([31_u8; 32].as_slice())
        .execute(&pool)
        .await
        .expect("insert enrollment policy");
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
             VALUES ($1,32,65536,4096)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("install audit capacity");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("activate invalidation");

        let actor_keys = Keys::generate();
        let binding_id = Uuid::new_v4();
        let enrollment_operation = Uuid::new_v4();
        let enrollment_request = [32_u8; 32];
        let enrollment_history = Uuid::new_v4();
        let principal_fingerprint = [33_u8; 32];
        let event_author = actor_keys.public_key().to_bytes();
        let actor_fingerprint = [34_u8; 32];
        let enrollment_result = [35_u8; 32];
        let enrollment_envelope = vec![36_u8; 64];
        let enrollment_envelope_digest: [u8; 32] = Sha256::digest(&enrollment_envelope).into();
        let mut seed = pool.begin().await.expect("begin canonical enrollment seed");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,1,$4,1,$5)",
        )
        .bind(domain.as_uuid())
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind(actor_fingerprint.as_slice())
        .bind(enrollment_result.as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert canonical enrollment receipt");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,'issuer','subject',$3,$4,1,1,1,1,$5,$6,$7,$8) \
             RETURNING binding_version",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .bind(principal_fingerprint.as_slice())
        .bind(event_author.as_slice())
        .bind([38_u8; 32].as_slice())
        .bind(enrollment_history)
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .fetch_one(&mut *seed)
        .await
        .expect("insert active binding");
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
              successor_binding_version,successor_lifecycle_revision,successor_state, \
              operation_id,request_fingerprint,transition_digest) \
             VALUES ($1,$2,1,1,$3,$4,1,1,$5,$6,$7)",
        )
        .bind(domain.as_uuid())
        .bind(enrollment_history)
        .bind(binding_id)
        .bind(binding_version)
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind([37_u8; 32].as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert enrollment history");
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
              actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
              occurred_at,canonical_envelope,envelope_digest) \
             VALUES ($1,$2,1,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(domain.as_uuid())
        .bind(Uuid::new_v4())
        .bind(actor_fingerprint.as_slice())
        .bind(enrollment_operation)
        .bind(enrollment_request.as_slice())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(&enrollment_envelope)
        .bind(enrollment_envelope_digest.as_slice())
        .execute(&mut *seed)
        .await
        .expect("insert canonical enrollment audit");
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
            .execute(&mut *seed)
            .await
            .expect("validate enrollment seed");
        seed.commit().await.expect("commit enrollment seed");

        let operation_id = Uuid::new_v4();
        let operation =
            OperatorLifecycleOperation::new(domain, operation_id, Uuid::new_v4(), Uuid::new_v4())
                .expect("operator operation");
        let target = ExactActiveOperatorTarget::new(
            binding_id,
            u64::try_from(binding_version).expect("positive binding version"),
            1,
            principal_fingerprint,
            event_author,
        )
        .expect("exact active target");
        let intent = OperatorLifecycleIntent::retire(operation, 1, target).expect("retire intent");
        let body = serde_json::to_vec(&serde_json::json!({
            "operation": { "operation_id": operation_id },
            "expected_revision": 1,
        }))
        .expect("serialize operator body");
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("operator verifier");
        let guard = PermissiveOperatorGuard;
        let first_time = Timestamp::from(
            u64::try_from(Utc::now().timestamp() - 1).expect("positive signer time"),
        );
        let first_grant = verifier
            .verify_lifecycle(
                &signed_operator_request(&signer, &url, &body, intent.intent_digest(), first_time),
                None,
                None,
                &url,
                &body,
                &guard,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent.intent_digest(),
                1,
                None,
                None,
            )
            .await
            .expect("verify first operator grant");
        let db = Db::from_pool(pool.clone());
        let committed = db
            .commit_verified_operator_lifecycle(&intent, &first_grant)
            .await
            .expect("commit verified retire");
        let notice = match committed {
            OperatorLifecycleCommitOutcome::Committed { invalidation } => invalidation,
            OperatorLifecycleCommitOutcome::ExactReplay => panic!("first retire must commit"),
        };
        assert_eq!(notice.authorization_domain(), domain);
        assert_eq!(notice.operation_id(), operation_id);
        assert_eq!(notice.generation(), 1);

        let retry_grant = verifier
            .verify_lifecycle(
                &signed_operator_request(
                    &signer,
                    &url,
                    &body,
                    intent.intent_digest(),
                    Timestamp::now(),
                ),
                None,
                None,
                &url,
                &body,
                &guard,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent.intent_digest(),
                1,
                None,
                None,
            )
            .await
            .expect("verify fresh retry grant");
        assert_eq!(
            first_grant.request_attribution(),
            retry_grant.request_attribution()
        );
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&intent, &retry_grant)
                .await
                .expect("exact operator replay"),
            OperatorLifecycleCommitOutcome::ExactReplay
        ));
        let (state, history_count, event_count, generation): (i16, i64, i64, i64) =
            sqlx::query_as(
                "SELECT binding_state, \
                 (SELECT count(*) FROM identity_lifecycle_history WHERE community_id=$1 AND operation_id=$2), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1 AND operation_id=$2), \
                 (SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1) \
                 FROM identity_bindings WHERE community_id=$1 AND binding_id=$3",
            )
            .bind(domain.as_uuid())
            .bind(operation_id)
            .bind(binding_id)
            .fetch_one(&pool)
            .await
            .expect("read committed operator state");
        assert_eq!(
            (state, history_count, event_count, generation),
            (2, 1, 1, 1)
        );
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("drop disposable operator database");
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn postgres_verified_rotate_retires_before_successor_and_replays_atomically() {
        use sqlx::postgres::PgPoolOptions;

        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect rotate test admin");
        let database_name = format!("s3_operator_rotate_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("create disposable rotate database");
        let split = admin_url.rfind('/').expect("database URL path");
        let database_url = format!("{}/{}", &admin_url[..split], database_name);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect disposable rotate database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply canonical migrations");

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("rotate-{}.example", domain.as_uuid().simple()))
            .execute(&pool)
            .await
            .expect("insert rotate community");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,1,$2,transaction_timestamp()-interval '1 second')",
        )
        .bind(domain.as_uuid())
        .bind([61_u8; 32].as_slice())
        .execute(&pool)
        .await
        .expect("insert rotate enrollment policy");
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
             VALUES ($1,64,131072,4096)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("install rotate audit capacity");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("activate rotate invalidation");

        let old_keys = Keys::generate();
        let collision_keys = Keys::generate();
        let target = seed_active_operator_binding(&pool, domain, &old_keys, 62).await;
        let collision_target =
            seed_active_operator_binding(&pool, domain, &collision_keys, 72).await;
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("operator verifier");
        let guard = PermissiveOperatorGuard;
        let db = Db::from_pool(pool.clone());
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/rotate",
            domain.as_uuid()
        );

        let failed_operation_id = Uuid::new_v4();
        let failed_expiry = Utc::now() + Duration::minutes(5);
        let failed_intent = OperatorLifecycleIntent::rotate(
            operation(domain, failed_operation_id),
            1,
            target,
            RequestedOperatorReplacement::new(
                collision_keys.public_key().to_bytes(),
                failed_expiry,
            )
            .expect("conflicting rotate replacement"),
        )
        .expect("conflicting rotate intent");
        let failed_body = rotate_body(failed_operation_id, &collision_keys, failed_expiry);
        let failed_grant = verified_rotate_grant(
            &verifier,
            &guard,
            &signer,
            &collision_keys,
            &url,
            &failed_body,
            &failed_intent,
            failed_expiry,
            Timestamp::now(),
        )
        .await;
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&failed_intent, &failed_grant)
                .await,
            Err(OperatorLifecycleError::Storage(_))
        ));
        let (old_after_failure, collision_after_failure, failed_history, failed_receipt, failed_event, failed_generation):
            (i16, i16, i64, i64, i64, i64) = sqlx::query_as(
                "SELECT \
                 (SELECT binding_state FROM identity_bindings WHERE community_id=$1 AND binding_id=$2), \
                 (SELECT binding_state FROM identity_bindings WHERE community_id=$1 AND binding_id=$3), \
                 (SELECT count(*) FROM identity_lifecycle_history WHERE community_id=$1 AND operation_id=$4), \
                 (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1 AND operation_id=$4), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1 AND operation_id=$4), \
                 (SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1)",
            )
            .bind(domain.as_uuid())
            .bind(target.binding_id)
            .bind(collision_target.binding_id)
            .bind(failed_operation_id)
            .fetch_one(&pool)
            .await
            .expect("read failed rotate rollback");
        assert_eq!(
            (
                old_after_failure,
                collision_after_failure,
                failed_history,
                failed_receipt,
                failed_event,
                failed_generation,
            ),
            (1, 1, 0, 0, 0, 0)
        );

        let replacement_keys = Keys::generate();
        let operation_id = Uuid::new_v4();
        let replacement_expiry = Utc::now() + Duration::minutes(5);
        let intent = OperatorLifecycleIntent::rotate(
            operation(domain, operation_id),
            1,
            target,
            RequestedOperatorReplacement::new(
                replacement_keys.public_key().to_bytes(),
                replacement_expiry,
            )
            .expect("successful rotate replacement"),
        )
        .expect("successful rotate intent");
        let body = rotate_body(operation_id, &replacement_keys, replacement_expiry);
        let grant = verified_rotate_grant(
            &verifier,
            &guard,
            &signer,
            &replacement_keys,
            &url,
            &body,
            &intent,
            replacement_expiry,
            Timestamp::now(),
        )
        .await;
        let notice = match db
            .commit_verified_operator_lifecycle(&intent, &grant)
            .await
            .expect("commit verified rotate")
        {
            OperatorLifecycleCommitOutcome::Committed { invalidation } => invalidation,
            OperatorLifecycleCommitOutcome::ExactReplay => panic!("first rotate must commit"),
        };
        assert_eq!(notice.generation(), 1);

        let replay_grant = verified_rotate_grant(
            &verifier,
            &guard,
            &signer,
            &replacement_keys,
            &url,
            &body,
            &intent,
            replacement_expiry,
            Timestamp::now(),
        )
        .await;
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&intent, &replay_grant)
                .await
                .expect("exact rotate replay"),
            OperatorLifecycleCommitOutcome::ExactReplay
        ));

        let changed_keys = Keys::generate();
        let changed_expiry = Utc::now() + Duration::minutes(5);
        let changed_intent = OperatorLifecycleIntent::rotate(
            operation(domain, operation_id),
            1,
            target,
            RequestedOperatorReplacement::new(changed_keys.public_key().to_bytes(), changed_expiry)
                .expect("changed rotate replacement"),
        )
        .expect("changed rotate intent");
        let changed_body = rotate_body(operation_id, &changed_keys, changed_expiry);
        let changed_grant = verified_rotate_grant(
            &verifier,
            &guard,
            &signer,
            &changed_keys,
            &url,
            &changed_body,
            &changed_intent,
            changed_expiry,
            Timestamp::now(),
        )
        .await;
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&changed_intent, &changed_grant)
                .await,
            Err(OperatorLifecycleError::IntentConflict)
        ));

        let (old_state, successor_count, changed_count, history_count, receipt_count, event_count, generation):
            (i16, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
                "SELECT \
                 (SELECT binding_state FROM identity_bindings WHERE community_id=$1 AND binding_id=$2), \
                 (SELECT count(*) FROM identity_bindings WHERE community_id=$1 \
                   AND principal_fingerprint=$3 AND event_author_pubkey=$4 AND binding_state=1), \
                 (SELECT count(*) FROM identity_bindings WHERE community_id=$1 \
                   AND event_author_pubkey=$5), \
                 (SELECT count(*) FROM identity_lifecycle_history WHERE community_id=$1 AND operation_id=$6), \
                 (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1 AND operation_id=$6), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1 AND operation_id=$6), \
                 (SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1)",
            )
            .bind(domain.as_uuid())
            .bind(target.binding_id)
            .bind(target.principal_fingerprint.as_slice())
            .bind(replacement_keys.public_key().to_bytes().as_slice())
            .bind(changed_keys.public_key().to_bytes().as_slice())
            .bind(operation_id)
            .fetch_one(&pool)
            .await
            .expect("read committed rotate state");
        assert_eq!(
            (
                old_state,
                successor_count,
                changed_count,
                history_count,
                receipt_count,
                event_count,
                generation,
            ),
            (2, 1, 0, 1, 1, 1, 1)
        );

        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("drop disposable rotate database");
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn postgres_no_active_disable_and_revoke_reject_generation_mismatch_without_writes() {
        use sqlx::postgres::PgPoolOptions;

        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect generation test admin");
        let database_name = format!("s3_operator_generation_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("create disposable generation database");
        let split = admin_url.rfind('/').expect("database URL path");
        let database_url = format!("{}/{}", &admin_url[..split], database_name);
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&database_url)
            .await
            .expect("connect disposable generation database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("apply canonical migrations");

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("generation-{}.example", domain.as_uuid().simple()))
            .execute(&pool)
            .await
            .expect("insert generation community");
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
             VALUES ($1,16,32768,4096)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("install generation audit capacity");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(domain.as_uuid())
        .execute(&pool)
        .await
        .expect("activate generation invalidation");

        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("operator verifier");
        let guard = PermissiveOperatorGuard;
        let db = Db::from_pool(pool.clone());
        let principal = [91_u8; 32];
        let event_author = [92_u8; 32];
        let disable_operation = Uuid::new_v4();
        let disable = OperatorLifecycleIntent::disable(
            operation(domain, disable_operation),
            2,
            principal,
            None,
        )
        .expect("mismatched no-active disable");
        let disable_body = serde_json::to_vec(&serde_json::json!({
            "operation": { "operation_id": disable_operation },
            "expected_revision": 2,
        }))
        .expect("serialize disable mismatch");
        let disable_url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/disable",
            domain.as_uuid()
        );
        let disable_grant = verifier
            .verify_lifecycle(
                &signed_operator_request(
                    &signer,
                    &disable_url,
                    &disable_body,
                    disable.intent_digest(),
                    Timestamp::now(),
                ),
                None,
                None,
                &disable_url,
                &disable_body,
                &guard,
                domain,
                OperatorLifecycleAction::Disable,
                disable_operation,
                disable.intent_digest(),
                2,
                None,
                None,
            )
            .await
            .expect("verify disable mismatch grant");
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&disable, &disable_grant)
                .await,
            Err(OperatorLifecycleError::AuthorizationDenied)
        ));

        let revoke_operation = Uuid::new_v4();
        let revoke = OperatorLifecycleIntent::revoke(
            operation(domain, revoke_operation),
            2,
            event_author,
            None,
        )
        .expect("mismatched no-active revoke");
        let revoke_body = serde_json::to_vec(&serde_json::json!({
            "operation": { "operation_id": revoke_operation },
            "expected_revision": 2,
        }))
        .expect("serialize revoke mismatch");
        let revoke_url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/revoke",
            domain.as_uuid()
        );
        let revoke_grant = verifier
            .verify_lifecycle(
                &signed_operator_request(
                    &signer,
                    &revoke_url,
                    &revoke_body,
                    revoke.intent_digest(),
                    Timestamp::now(),
                ),
                None,
                None,
                &revoke_url,
                &revoke_body,
                &guard,
                domain,
                OperatorLifecycleAction::Revoke,
                revoke_operation,
                revoke.intent_digest(),
                2,
                None,
                None,
            )
            .await
            .expect("verify revoke mismatch grant");
        assert!(matches!(
            db.commit_verified_operator_lifecycle(&revoke, &revoke_grant)
                .await,
            Err(OperatorLifecycleError::AuthorizationDenied)
        ));

        let (selectors, histories, receipts, events, generation): (i64, i64, i64, i64, i64) =
            sqlx::query_as(
                "SELECT \
                 (SELECT count(*) FROM identity_lifecycle_selectors WHERE community_id=$1), \
                 (SELECT count(*) FROM identity_lifecycle_history WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1), \
                 (SELECT count(*) FROM authorization_events WHERE community_id=$1), \
                 (SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1)",
            )
            .bind(domain.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read generation mismatch rollback");
        assert_eq!(
            (selectors, histories, receipts, events, generation),
            (0, 0, 0, 0, 0)
        );

        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {database_name}"
        )))
        .execute(&admin)
        .await
        .expect("drop disposable generation database");
    }

    #[tokio::test]
    async fn provision_authentication_attempt_is_exact_and_redacted() {
        use sqlx::postgres::PgPoolOptions;

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let db = Db::from_pool(
            PgPoolOptions::new()
                .connect_lazy(&loopback_test_database_url())
                .expect("lazy test pool"),
        );
        let body = serde_json::to_vec(&serde_json::json!({
            "authorization_domain": domain.as_uuid(),
            "operation_id": Uuid::new_v4(),
            "policy_revision": 7,
            "policy_digest": hex::encode([21_u8; 32]),
            "issuer": "https://private-issuer.example",
            "subject": "private-subject-canary",
            "selected_event_author_pubkey": hex::encode([22_u8; 32]),
        }))
        .expect("serialize provision attempt");
        let attempt =
            OperatorProvisionAuthenticationAttempt::from_bounded_body(db.clone(), domain, &body)
                .expect("bounded attempt");
        let replay =
            OperatorProvisionAuthenticationAttempt::from_bounded_body(db.clone(), domain, &body)
                .expect("bounded replay");
        assert_eq!(
            attempt.prepared.semantic_fingerprint,
            replay.prepared.semantic_fingerprint
        );
        let debug = format!("{attempt:?}");
        assert!(!debug.contains("private-issuer"));
        assert!(!debug.contains("private-subject"));

        let mut changed: serde_json::Value =
            serde_json::from_slice(&body).expect("parse provision attempt");
        changed["selected_event_author_pubkey"] = serde_json::Value::String(hex::encode([23; 32]));
        let changed = serde_json::to_vec(&changed).expect("serialize changed attempt");
        assert_ne!(
            attempt.prepared.semantic_fingerprint,
            OperatorProvisionAuthenticationAttempt::from_bounded_body(db.clone(), domain, &changed)
                .expect("changed bounded attempt")
                .prepared
                .semantic_fingerprint
        );
        let other_domain = OperatorProvisionAuthenticationAttempt::from_bounded_body(
            db,
            CommunityId::from_uuid(Uuid::new_v4()),
            &body,
        )
        .expect("other-domain attempt");
        assert_ne!(
            attempt.prepared.semantic_fingerprint,
            other_domain.prepared.semantic_fingerprint
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn postgres_authentication_denials_aggregate_without_receipt_occupancy() {
        use buzz_auth::AuthorizationEventCapacityPolicy;
        use sqlx::postgres::PgPoolOptions;

        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin");
        let name = format!("s5_operator_denial_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let url = format!("{}/{}", &admin_url[..split], name);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");
        let db = Db::from_pool(pool.clone());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!(
                "operator-denial-{}.example",
                domain.as_uuid().simple()
            ))
            .execute(&pool)
            .await
            .expect("insert denial community");
        db.install_authorization_event_capacity(
            domain,
            AuthorizationEventCapacityPolicy::new(100, 1 << 20, 16 << 10).expect("valid capacity"),
        )
        .await
        .expect("atomically install canonical and pre-auth capacities");
        let lifecycle =
            OperatorLifecycleIntent::revoke(operation(domain, Uuid::new_v4()), 1, [31; 32], None)
                .expect("lifecycle denial intent");
        assert_eq!(
            db.record_operator_lifecycle_authentication_denial(
                &lifecycle,
                OperatorAuthenticationDenialReason::MissingCredential,
            )
            .await
            .expect("record lifecycle denial"),
            OperatorPreauthDenialWrite::Inserted
        );
        assert_eq!(
            db.record_operator_lifecycle_authentication_denial(
                &lifecycle,
                OperatorAuthenticationDenialReason::MissingCredential,
            )
            .await
            .expect("aggregate repeated lifecycle denial"),
            OperatorPreauthDenialWrite::Inserted
        );

        let provision_body = serde_json::to_vec(&serde_json::json!({
            "authorization_domain": domain.as_uuid(),
            "operation_id": Uuid::new_v4(),
            "policy_revision": 3,
            "policy_digest": hex::encode([32_u8; 32]),
            "issuer": "https://private-issuer.example",
            "subject": "private-subject-canary",
            "selected_event_author_pubkey": hex::encode([33_u8; 32]),
        }))
        .expect("serialize provision denial");
        let provision = OperatorProvisionAuthenticationAttempt::from_bounded_body(
            db.clone(),
            domain,
            &provision_body,
        )
        .expect("bounded provision denial");
        assert_eq!(
            provision
                .persist_denial(OperatorAuthenticationDenialReason::InvalidCredential)
                .await
                .expect("record provision denial"),
            OperatorPreauthDenialWrite::Inserted
        );
        let malformed_body = b"{\"authorization_domain\":";
        assert_eq!(
            OperatorProvisionAuthenticationAttempt::from_bounded_body(
                db.clone(),
                domain,
                malformed_body,
            )
            .expect("bounded malformed attempt")
            .persist_denial(OperatorAuthenticationDenialReason::InvalidCredential)
            .await
            .expect("record malformed bounded provision denial"),
            OperatorPreauthDenialWrite::Inserted
        );
        assert_eq!(
            OperatorProvisionAuthenticationAttempt::from_bounded_body(db, domain, malformed_body,)
                .expect("bounded malformed replay")
                .persist_denial(OperatorAuthenticationDenialReason::InvalidCredential)
                .await
                .expect("aggregate malformed bounded provision denial"),
            OperatorPreauthDenialWrite::Inserted
        );

        let (buckets, denials, events, receipts): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT count(*) FROM authorization_operator_denial_buckets \
                 WHERE community_id=$1), \
               (SELECT COALESCE(sum(denial_count),0)::bigint \
                  FROM authorization_operator_denial_buckets \
                 WHERE community_id=$1), \
               (SELECT count(*) FROM authorization_events WHERE community_id=$1), \
               (SELECT count(*) FROM authorization_operation_receipts WHERE community_id=$1)",
        )
        .bind(domain.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load denial cardinality");
        assert_eq!((buckets, denials, events, receipts), (2, 5, 0, 0));

        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("drop scratch database");
        admin.close().await;
    }
}
