//! Immutable provider-free authorization events and capacity controls.
//!
//! V1 deliberately has no claim, export, acknowledgement, retry, pruning, or
//! compaction workflow. Inserts consume an explicitly configured immutable
//! per-domain budget; exhaustion aborts the surrounding authorization
//! transaction and is latched unhealthy by the caller.

use std::{collections::HashSet, fmt};

use buzz_auth::{
    operator_lifecycle::{VerifiedOperatorLifecycleGrant, VerifiedProvisionBindingIntent},
    AuthorizationEventCapacityPolicy, FinalizedAuthContext, PreparedAuthorization, ProofTransport,
    VerifiedFederatedAssertion, VerifiedNostrProof,
};
use buzz_core::{CanonicalCurrentBindingEvidence, CommunityId};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::{Db, DbError, Result};

const MAX_EVENT_PAGE: u16 = 1_000;

/// Closed protected surfaces sharing the bounded denial bucket family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum ProtectedDenialSurface {
    /// Invite claim admission.
    InviteClaim = 2,
    /// Moderation command admission.
    Moderation = 3,
}

/// Closed protected actions represented by denial evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum ProtectedDenialAction {
    /// Claim one relay invite.
    InviteClaim = 1,
    /// Apply one moderation command.
    ModerationCommand = 2,
}

/// Coarse durable denial reasons. These values are operational evidence, not
/// public error detail, and intentionally retain no request coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum ProtectedDenialReason {
    /// Required command proof or assertion was absent.
    MissingProof = 1,
    /// A supplied command proof or assertion was malformed or unverifiable.
    InvalidProof = 2,
    /// The configured admission mode closed the protected surface.
    ModeDenied = 3,
    /// Current authority or policy did not permit the operation.
    AuthorizationDenied = 4,
    /// The protected resource was expired, exhausted, banned, or revoked.
    ResourceDenied = 5,
    /// Commit-time policy or authority state superseded preparation.
    PolicySuperseded = 6,
    /// Replay or intent identity conflicted with retained state.
    ReplayConflict = 7,
    /// A required audit, database, or verifier dependency was unavailable.
    DependencyUnavailable = 8,
}

/// Closed operation kinds stored in the single canonical receipt table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationOperationKind {
    /// Direct enrollment.
    Enroll = 1,
    /// Separate provisioning.
    Provision = 2,
    /// Retire an exact binding generation.
    Retire = 3,
    /// Disable a principal.
    Disable = 4,
    /// Revoke authority.
    Revoke = 5,
    /// Rotate to a successor binding.
    Rotate = 6,
    /// Recover a retired principal.
    Recover = 7,
    /// Enable a disabled principal.
    Enable = 8,
    /// Local admission loss.
    AdmissionLoss = 9,
    /// Authenticated operator action.
    Operator = 10,
    /// Protected mutation.
    ProtectedMutation = 11,
    /// Invalidation advance.
    Invalidation = 12,
    /// Client-status revision.
    StatusRevision = 13,
}

/// Persisted result classification for one operation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationOperationOutcome {
    /// State changed successfully.
    Applied = 1,
    /// Closed denial with durable evidence.
    Denied = 2,
    /// Successful semantic no-op.
    NoOp = 3,
}

/// Canonical operation receipt supplied by a transaction owner.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationOperationReceipt {
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    operation_kind: AuthorizationOperationKind,
    actor: AuthorizationEventActor,
    outcome: AuthorizationOperationOutcome,
    result_digest: [u8; 32],
}

impl AuthorizationOperationReceipt {
    /// Validate one receipt without persisting it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        operation_kind: AuthorizationOperationKind,
        actor: AuthorizationEventActor,
        outcome: AuthorizationOperationOutcome,
        result_digest: [u8; 32],
    ) -> Result<Self> {
        if community_id.as_uuid().is_nil()
            || operation_id.is_nil()
            || request_fingerprint == [0; 32]
            || actor.fingerprint().is_none()
            || !actor.is_bound_to(community_id)
            || result_digest == [0; 32]
        {
            return Err(DbError::InvalidData(
                "authorization operation receipt is invalid".to_owned(),
            ));
        }
        Ok(Self {
            community_id,
            operation_id,
            request_fingerprint,
            operation_kind,
            actor,
            outcome,
            result_digest,
        })
    }
}

impl fmt::Debug for AuthorizationOperationReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationOperationReceipt")
            .field("community_id", &"[REDACTED]")
            .field("operation_id", &"[REDACTED]")
            .field("request_fingerprint", &"[REDACTED]")
            .field("operation_kind", &self.operation_kind)
            .field("actor", &self.actor)
            .field("outcome", &self.outcome)
            .field("result_digest", &"[REDACTED]")
            .finish()
    }
}

/// Whether a receipt insert created state or replayed an identical receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationReceiptWrite {
    /// New receipt inserted.
    Inserted,
    /// Byte-for-byte semantic replay.
    ExactReplay,
}

/// Canonical authorization event kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationEventKind {
    /// Binding enrolled.
    Enrolled = 1,
    /// Authority revoked.
    Revoked = 2,
    /// Binding rotated.
    Rotated = 3,
    /// Principal recovered.
    Recovered = 4,
    /// Principal enabled.
    PrincipalEnabled = 5,
    /// Binding retired.
    Retired = 6,
    /// Principal disabled.
    PrincipalDisabled = 7,
    /// Local authority admission lost.
    AdmissionLost = 8,
    /// Operator action denied.
    OperatorDenied = 9,
    /// Protected operation allowed.
    ProtectedAllowed = 10,
    /// Protected operation denied.
    ProtectedDenied = 11,
    /// Current-binding status published.
    StatusPublished = 12,
    /// Current-binding status withdrawn.
    StatusWithdrawn = 13,
    /// Invalidation generation advanced.
    InvalidationAdvanced = 14,
}

impl AuthorizationEventKind {
    fn from_database(value: i16) -> Result<Self> {
        match value {
            1 => Ok(Self::Enrolled),
            2 => Ok(Self::Revoked),
            3 => Ok(Self::Rotated),
            4 => Ok(Self::Recovered),
            5 => Ok(Self::PrincipalEnabled),
            6 => Ok(Self::Retired),
            7 => Ok(Self::PrincipalDisabled),
            8 => Ok(Self::AdmissionLost),
            9 => Ok(Self::OperatorDenied),
            10 => Ok(Self::ProtectedAllowed),
            11 => Ok(Self::ProtectedDenied),
            12 => Ok(Self::StatusPublished),
            13 => Ok(Self::StatusWithdrawn),
            14 => Ok(Self::InvalidationAdvanced),
            _ => Err(DbError::InvalidData(
                "authorization event kind is invalid".to_owned(),
            )),
        }
    }
}

/// Stable canonical event outcome code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationEventOutcome {
    /// Allowed or applied.
    Allowed = 1,
    /// Denied closed.
    Denied = 2,
    /// No-op.
    NoOp = 3,
    /// Failed closed because durable evidence was unavailable.
    AuditUnavailable = 4,
    /// Withdrawn.
    Withdrawn = 5,
}

/// Stable redaction-safe event reason code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationReasonCode {
    /// Successful current policy.
    Current = 1,
    /// Missing credential or evidence.
    Missing = 2,
    /// Invalid credential or evidence.
    Invalid = 3,
    /// Unauthenticated attempt.
    Unauthenticated = 4,
    /// Stale authority.
    StaleAuthority = 5,
    /// Cross-domain evidence.
    CrossDomain = 6,
    /// Binding mismatch.
    BindingMismatch = 7,
    /// Delegation mismatch.
    DelegationMismatch = 8,
    /// Policy denial.
    PolicyDenied = 9,
    /// Invalidation fence changed.
    Invalidated = 10,
    /// Capacity exhausted.
    CapacityExhausted = 11,
    /// Storage unavailable.
    StorageUnavailable = 12,
    /// Proof expired.
    Expired = 13,
    /// Exact replay.
    Replay = 14,
    /// Intent conflict.
    IntentConflict = 15,
    /// Explicit withdrawal.
    Withdrawn = 16,
    /// A replay identity was rejected rather than accepted as an exact replay.
    ReplayRejected = 17,
}

/// Pseudonymous actor classification derived only from sealed or
/// authoritatively rechecked evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
enum AuthorizationActorKind {
    /// Authenticated direct actor.
    Direct = 1,
    /// Authenticated delegate.
    Delegate = 2,
    /// Authenticated operator.
    Operator = 3,
    /// Credential-free unresolved attempt.
    Unresolved = 4,
}

/// Exact local authority coordinate whose loss was rechecked in PostgreSQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalAdmissionLossCause {
    /// Exact active binding generation.
    Binding {
        /// Stable binding identifier.
        binding_id: Uuid,
        /// Immutable binding generation.
        binding_version: u64,
    },
    /// Exact active local policy revision.
    Policy {
        /// Positive policy revision.
        policy_revision: u64,
    },
    /// Exact delegated relationship revision already installed in local
    /// protected authority.
    DelegatedRelationship {
        /// Stable relationship identifier.
        relationship_id: Uuid,
        /// Positive relationship revision.
        relationship_revision: u64,
    },
}

/// Database-rechecked loss coordinate retained inside an opaque actor proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationAuthorityLossTarget {
    /// Every protected object owned by one exact binding generation.
    Binding(Uuid, u64),
    /// Every protected object admitted by one exact policy revision.
    Policy(u64),
    /// Every protected object admitted by one exact delegated relationship.
    DelegatedRelationship(Uuid, u64),
}

/// Opaque authenticated event actor.
///
/// Callers cannot choose an actor kind or fingerprint. Direct/delegated route
/// actors are derived from a sealed [`FinalizedAuthContext`]; local admission-loss
/// actors are minted only after an authoritative PostgreSQL recheck.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationEventActor {
    community_id: Option<CommunityId>,
    kind: AuthorizationActorKind,
    fingerprint: Option<[u8; 32]>,
    authority_loss_target: Option<AuthorizationAuthorityLossTarget>,
}

impl AuthorizationEventActor {
    /// Derive direct/delegated attribution from one sealed prepared route.
    pub(crate) fn from_prepared_authorization(prepared: &PreparedAuthorization) -> Result<Self> {
        let snapshot = prepared.recheck_request().lease_dependencies();
        let (_, domain) = snapshot.identity();
        let (_, actor, owner) = snapshot.authority();
        let relationship = snapshot
            .delegated_relationship()
            .map(|(id, revision, _)| (id, revision));
        actor_from_route_coordinates(
            domain,
            actor.to_bytes(),
            owner.map(|value| value.to_bytes()),
            snapshot.binding(),
            relationship,
        )
    }

    /// Derive operator attribution only from one origin-sealed lifecycle
    /// grant. Callers cannot select the actor class or fingerprint.
    pub fn from_verified_operator_grant(grant: &VerifiedOperatorLifecycleGrant) -> Result<Self> {
        if grant.authorization_domain().as_uuid().is_nil()
            || grant.operation_id().is_nil()
            || grant.intent_digest() == [0; 32]
            || grant.expected_revision() == 0
            || grant.approval_policy_digest() == [0; 32]
            || grant.signer_attribution() == [0; 32]
            || grant.approval_attribution() == Some([0; 32])
            || grant.request_attribution() == [0; 32]
            || grant.issued_at().timestamp_micros() <= 0
            || grant.expires_at() <= grant.issued_at()
        {
            return Err(DbError::InvalidData(
                "verified operator grant is invalid".to_owned(),
            ));
        }
        let action = (grant.action() as i16).to_be_bytes();
        let expected_revision = grant.expected_revision().to_be_bytes();
        Ok(local_actor(
            AuthorizationActorKind::Operator,
            grant.authorization_domain(),
            &[
                &action,
                grant.operation_id().as_bytes(),
                &grant.intent_digest(),
                &expected_revision,
                &grant.approval_policy_digest(),
                &grant.signer_attribution(),
                &grant.approval_attribution().unwrap_or([0; 32]),
                &grant.request_attribution(),
            ],
            None,
        ))
    }

    /// Derive provisioning attribution only from one verifier-sealed
    /// provision intent. This remains distinct from lifecycle authority.
    pub fn from_verified_provision_intent(intent: &VerifiedProvisionBindingIntent) -> Result<Self> {
        if intent.authorization_domain().as_uuid().is_nil()
            || intent.operation_id().is_nil()
            || intent.policy_revision() == 0
            || intent.policy_digest() == [0; 32]
            || intent.selected_event_author_pubkey() == [0; 32]
            || intent.intent_digest() == [0; 32]
            || intent.approval_policy_digest() == [0; 32]
            || intent.signer_attribution() == [0; 32]
            || intent.approval_attribution() == Some([0; 32])
            || intent.request_attribution() == [0; 32]
            || intent.issued_at().timestamp_micros() <= 0
            || intent.expires_at() <= intent.issued_at()
        {
            return Err(DbError::InvalidData(
                "verified provision intent is invalid".to_owned(),
            ));
        }
        let policy_revision = intent.policy_revision().to_be_bytes();
        Ok(local_actor(
            AuthorizationActorKind::Operator,
            intent.authorization_domain(),
            &[
                intent.operation_id().as_bytes(),
                &policy_revision,
                &intent.policy_digest(),
                &intent.selected_event_author_pubkey(),
                &intent.intent_digest(),
                &intent.approval_policy_digest(),
                &intent.signer_attribution(),
                &intent.approval_attribution().unwrap_or([0; 32]),
                &intent.request_attribution(),
            ],
            None,
        ))
    }

    /// Derive direct/delegated attribution from the complete sealed route.
    pub fn from_auth_context(context: &FinalizedAuthContext) -> Result<Self> {
        let lease = context.lease();
        let (request_fingerprint, _, _) = lease.request_binding();
        if lease.authorization_domain() != context.authorization_domain()
            || lease.actor_pubkey() != context.actor_pubkey()
            || lease.owner_pubkey() != context.owner_pubkey()
            || lease.binding() != context.binding()
            || lease.capability() != context.capability()
            || request_fingerprint != context.request_fingerprint()
        {
            return Err(DbError::InvalidData(
                "authorization actor route coordinates do not match".to_owned(),
            ));
        }
        actor_from_route_coordinates(
            context.authorization_domain(),
            context.actor_pubkey().to_bytes(),
            context.owner_pubkey().map(|value| value.to_bytes()),
            context.binding(),
            lease.delegated_relationship(),
        )
    }

    /// Derive enrollment attribution only from one origin-sealed direct
    /// assertion/proof pair. No binding exists yet, so this deliberately uses
    /// the verifier-bound principal, actor, and complete request coordinates
    /// rather than accepting a caller-selected actor label or fingerprint.
    pub fn from_verified_direct_enrollment(
        assertion: &VerifiedFederatedAssertion,
        proof: &VerifiedNostrProof,
    ) -> Result<Self> {
        let principal = assertion.principal_storage_key();
        let assertion_fingerprint = proof.bound_assertion_fingerprint().ok_or_else(|| {
            DbError::InvalidData("direct enrollment proof is not bound to an assertion".to_owned())
        })?;
        if assertion.authorization_domain() != proof.authorization_domain()
            || proof.delegation_conditions_fingerprint().is_some()
            || assertion_fingerprint == &[0; 32]
        {
            return Err(DbError::InvalidData(
                "direct enrollment evidence coordinates do not match".to_owned(),
            ));
        }
        let transport = proof_transport_code(proof.transport());
        Ok(local_actor(
            AuthorizationActorKind::Direct,
            assertion.authorization_domain(),
            &[
                principal.issuer().as_bytes(),
                principal.subject().as_bytes(),
                &proof.actor_pubkey().to_bytes(),
                assertion_fingerprint,
                proof.request_fingerprint(),
                proof.target_fingerprint(),
                proof.transport_context_fingerprint(),
                &transport.to_be_bytes(),
            ],
            None,
        ))
    }

    /// Credential-free actor used only by the dedicated pre-authentication
    /// denial lane, which cannot own a canonical operation receipt.
    pub const fn unresolved_authentication_denial() -> Self {
        Self {
            community_id: None,
            kind: AuthorizationActorKind::Unresolved,
            fingerprint: None,
            authority_loss_target: None,
        }
    }

    const fn kind(&self) -> AuthorizationActorKind {
        self.kind
    }

    pub(crate) const fn fingerprint(&self) -> Option<[u8; 32]> {
        self.fingerprint
    }

    /// Whether the sealed actor belongs to the exact server-owned domain.
    pub fn is_bound_to(&self, community_id: CommunityId) -> bool {
        matches!(self.community_id, Some(bound) if bound == community_id)
    }

    /// Whether this actor may own a canonical operation receipt.
    pub const fn is_authenticated(&self) -> bool {
        self.fingerprint.is_some() && !matches!(self.kind, AuthorizationActorKind::Unresolved)
    }

    /// Exact database-rechecked coordinate available to the coupled authority
    /// refence seam.
    pub const fn authority_loss_target(&self) -> Option<AuthorizationAuthorityLossTarget> {
        self.authority_loss_target
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

impl fmt::Debug for AuthorizationEventActor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationEventActor([REDACTED])")
    }
}

/// Recheck a local admission-loss cause and mint its non-forgeable event actor.
pub async fn resolve_local_admission_loss_actor_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    cause: LocalAdmissionLossCause,
) -> Result<AuthorizationEventActor> {
    if community_id.as_uuid().is_nil() {
        return Err(DbError::InvalidData(
            "authorization admission-loss actor domain is invalid".to_owned(),
        ));
    }
    match cause {
        LocalAdmissionLossCause::Binding {
            binding_id,
            binding_version,
        } => {
            if binding_id.is_nil() || binding_version == 0 || binding_version > i64::MAX as u64 {
                return Err(DbError::InvalidData(
                    "authorization admission-loss binding actor is invalid".to_owned(),
                ));
            }
            let actor: Vec<u8> = sqlx::query_scalar(
                "SELECT event_author_pubkey FROM identity_bindings \
                 WHERE community_id=$1 AND binding_id=$2 AND binding_version=$3 \
                   AND binding_state=1 AND (expires_at IS NULL OR expires_at > clock_timestamp())",
            )
            .bind(community_id.as_uuid())
            .bind(binding_id)
            .bind(i64::try_from(binding_version).map_err(|_| {
                DbError::InvalidData("authorization binding version is out of range".to_owned())
            })?)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| DbError::NotFound("active authorization binding".to_owned()))?;
            let actor = bytes32(actor, "local binding actor")?;
            Ok(local_actor(
                AuthorizationActorKind::Direct,
                community_id,
                &[
                    binding_id.as_bytes(),
                    &binding_version.to_be_bytes(),
                    &actor,
                ],
                Some(AuthorizationAuthorityLossTarget::Binding(
                    binding_id,
                    binding_version,
                )),
            ))
        }
        LocalAdmissionLossCause::Policy { policy_revision } => {
            if policy_revision == 0 || policy_revision > i64::MAX as u64 {
                return Err(DbError::InvalidData(
                    "authorization admission-loss policy actor is invalid".to_owned(),
                ));
            }
            let policy_digest: Vec<u8> = sqlx::query_scalar(
                "SELECT policy_digest FROM identity_enrollment_policies \
                 WHERE community_id=$1 AND policy_revision=$2 \
                   AND effective_at <= clock_timestamp() \
                   AND (expires_at IS NULL OR expires_at > clock_timestamp())",
            )
            .bind(community_id.as_uuid())
            .bind(i64::try_from(policy_revision).map_err(|_| {
                DbError::InvalidData("authorization policy revision is out of range".to_owned())
            })?)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| DbError::NotFound("active authorization policy".to_owned()))?;
            let policy_digest = bytes32(policy_digest, "local policy digest")?;
            Ok(local_actor(
                AuthorizationActorKind::Direct,
                community_id,
                &[&policy_revision.to_be_bytes(), &policy_digest],
                Some(AuthorizationAuthorityLossTarget::Policy(policy_revision)),
            ))
        }
        LocalAdmissionLossCause::DelegatedRelationship {
            relationship_id,
            relationship_revision,
        } => {
            if relationship_id.is_nil()
                || relationship_revision == 0
                || relationship_revision > i64::MAX as u64
            {
                return Err(DbError::InvalidData(
                    "authorization admission-loss delegation actor is invalid".to_owned(),
                ));
            }
            let rows = sqlx::query(
                "SELECT DISTINCT p.actor_pubkey,p.owner_pubkey,p.binding_id,p.binding_version \
                 FROM protected_object_authority p \
                 JOIN identity_bindings b ON b.community_id=p.community_id \
                   AND b.binding_id=p.binding_id AND b.binding_version=p.binding_version \
                 WHERE p.community_id=$1 AND p.delegated_relationship_id=$2 \
                   AND p.delegated_relationship_revision=$3 AND p.expires_at > clock_timestamp() \
                   AND b.binding_state=1 AND (b.expires_at IS NULL OR b.expires_at > clock_timestamp()) \
                 ORDER BY p.actor_pubkey,p.owner_pubkey,p.binding_id,p.binding_version LIMIT 2",
            )
            .bind(community_id.as_uuid())
            .bind(relationship_id)
            .bind(i64::try_from(relationship_revision).map_err(|_| {
                DbError::InvalidData(
                    "authorization relationship revision is out of range".to_owned(),
                )
            })?)
            .fetch_all(&mut **transaction)
            .await?;
            if rows.len() != 1 {
                return Err(DbError::InvalidData(
                    "authorization delegated authority is missing or ambiguous".to_owned(),
                ));
            }
            let row = &rows[0];
            let actor = bytes32(row.try_get("actor_pubkey")?, "delegated actor")?;
            let owner = bytes32(
                row.try_get::<Option<Vec<u8>>, _>("owner_pubkey")?
                    .ok_or_else(|| {
                        DbError::InvalidData("authorization delegated owner is missing".to_owned())
                    })?,
                "delegated owner",
            )?;
            let binding_id: Uuid = row.try_get("binding_id")?;
            let binding_version = u64::try_from(row.try_get::<i64, _>("binding_version")?)
                .map_err(|_| {
                    DbError::InvalidData(
                        "authorization delegated binding version is invalid".to_owned(),
                    )
                })?;
            Ok(local_actor(
                AuthorizationActorKind::Delegate,
                community_id,
                &[
                    relationship_id.as_bytes(),
                    &relationship_revision.to_be_bytes(),
                    &actor,
                    &owner,
                    binding_id.as_bytes(),
                    &binding_version.to_be_bytes(),
                ],
                Some(AuthorizationAuthorityLossTarget::DelegatedRelationship(
                    relationship_id,
                    relationship_revision,
                )),
            ))
        }
    }
}

fn actor_from_route_coordinates(
    community_id: CommunityId,
    actor: [u8; 32],
    owner: Option<[u8; 32]>,
    binding: (Uuid, u64),
    relationship: Option<(Uuid, u64)>,
) -> Result<AuthorizationEventActor> {
    let kind = match (owner, relationship) {
        (None, None) => AuthorizationActorKind::Direct,
        (Some(_), Some(_)) => AuthorizationActorKind::Delegate,
        _ => {
            return Err(DbError::InvalidData(
                "authorization actor route is partially delegated".to_owned(),
            ));
        }
    };
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-event-actor:route:v1");
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, &(kind as i16).to_be_bytes());
    framed(&mut digest, &actor);
    append_hash_optional(&mut digest, owner.as_ref().map(<[u8; 32]>::as_slice));
    framed(&mut digest, binding.0.as_bytes());
    framed(&mut digest, &binding.1.to_be_bytes());
    match relationship {
        Some((id, revision)) => {
            framed(&mut digest, id.as_bytes());
            framed(&mut digest, &revision.to_be_bytes());
        }
        None => framed(&mut digest, &[]),
    }
    Ok(AuthorizationEventActor {
        community_id: Some(community_id),
        kind,
        fingerprint: Some(digest.finalize().into()),
        authority_loss_target: None,
    })
}

fn local_actor(
    kind: AuthorizationActorKind,
    community_id: CommunityId,
    parts: &[&[u8]],
    authority_loss_target: Option<AuthorizationAuthorityLossTarget>,
) -> AuthorizationEventActor {
    let mut digest = Sha256::new();
    framed(
        &mut digest,
        b"buzz:authorization-event-actor:local-authority:v1",
    );
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, &(kind as i16).to_be_bytes());
    for part in parts {
        framed(&mut digest, part);
    }
    AuthorizationEventActor {
        community_id: Some(community_id),
        kind,
        fingerprint: Some(digest.finalize().into()),
        authority_loss_target,
    }
}

/// Mint status-publication attribution only after the complete evidence tuple
/// has been rechecked inside the allocation transaction.
pub async fn resolve_current_binding_event_actor_tx(
    transaction: &mut Transaction<'_, Postgres>,
    evidence: &CanonicalCurrentBindingEvidence,
) -> Result<AuthorizationEventActor> {
    let mut object_key_digest = Sha256::new();
    object_key_digest.update(b"buzz:client-binding-status-authority:v1");
    object_key_digest.update((16_u64).to_be_bytes());
    object_key_digest.update(evidence.authorization_domain().as_uuid().as_bytes());
    object_key_digest.update((32_u64).to_be_bytes());
    object_key_digest.update(evidence.event_author_pubkey().to_bytes());
    let object_key: [u8; 32] = object_key_digest.finalize().into();
    sqlx::query(
        "SELECT 1 FROM identity_bindings b \
         JOIN identity_enrollment_policies p \
           ON p.community_id=b.community_id AND p.policy_revision=b.policy_revision \
         JOIN authorization_invalidation_domains d ON d.community_id=b.community_id \
         JOIN authorization_authority_epochs a \
           ON a.community_id=b.community_id AND a.object_kind=7 AND a.object_key=$10 \
         WHERE b.community_id=$1 AND b.binding_id=$2 AND b.binding_version=$3 \
           AND b.event_author_pubkey=$4 AND b.policy_revision=$5 \
           AND d.current_generation=$6 AND a.authority_epoch=$7 AND a.fence=$8 \
           AND b.binding_state=1 AND $9 > clock_timestamp() \
           AND $11 <= clock_timestamp() \
           AND (b.expires_at IS NULL OR b.expires_at > clock_timestamp()) \
           AND p.effective_at <= clock_timestamp() \
           AND (p.expires_at IS NULL OR p.expires_at > clock_timestamp()) \
         FOR SHARE OF b,p,d,a",
    )
    .bind(evidence.authorization_domain().as_uuid())
    .bind(evidence.binding_id())
    .bind(i64::try_from(evidence.binding_version()).map_err(|_| {
        DbError::InvalidData("authorization binding version is out of range".to_owned())
    })?)
    .bind(evidence.event_author_pubkey().to_bytes().as_slice())
    .bind(i64::try_from(evidence.policy_revision()).map_err(|_| {
        DbError::InvalidData("authorization policy revision is out of range".to_owned())
    })?)
    .bind(
        i64::try_from(evidence.invalidation_generation()).map_err(|_| {
            DbError::InvalidData("authorization invalidation generation is out of range".to_owned())
        })?,
    )
    .bind(i64::try_from(evidence.authority_epoch()).map_err(|_| {
        DbError::InvalidData("authorization authority epoch is out of range".to_owned())
    })?)
    .bind(evidence.fence().as_bytes().as_slice())
    .bind(evidence.fresh_until())
    .bind(object_key.as_slice())
    .bind(evidence.observed_at())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| DbError::NotFound("current binding event actor".to_owned()))?;
    current_binding_event_actor(evidence)
}

/// Derive redaction-safe attribution from already rechecked binding evidence.
pub fn current_binding_event_actor(
    evidence: &CanonicalCurrentBindingEvidence,
) -> Result<AuthorizationEventActor> {
    Ok(local_actor(
        AuthorizationActorKind::Direct,
        evidence.authorization_domain(),
        &[
            evidence.binding_id().as_bytes(),
            &evidence.binding_version().to_be_bytes(),
            &evidence.event_author_pubkey().to_bytes(),
        ],
        None,
    ))
}

/// Mint withdrawal attribution only from the exact current durable status
/// receipt that is about to be superseded.
pub async fn resolve_status_withdrawal_event_actor_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event_author_pubkey: [u8; 32],
    supersedes_revision: u64,
) -> Result<AuthorizationEventActor> {
    let current = sqlx::query(
        "SELECT revision,disposition FROM client_status_revisions \
         WHERE community_id=$1 AND event_author_pubkey=$2 \
         ORDER BY revision DESC LIMIT 1 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(event_author_pubkey.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(current) = current else {
        return Err(DbError::NotFound(
            "current status withdrawal actor".to_owned(),
        ));
    };
    if current.try_get::<i16, _>("disposition")? != 1
        || u64::try_from(current.try_get::<i64, _>("revision")?).map_err(|_| {
            DbError::InvalidData("authorization status revision is invalid".to_owned())
        })? != supersedes_revision
    {
        return Err(DbError::InvalidData(
            "current status withdrawal actor changed".to_owned(),
        ));
    }
    status_withdrawal_event_actor(community_id, event_author_pubkey, supersedes_revision)
}

/// Derive withdrawal attribution from one exact durable status revision.
pub fn status_withdrawal_event_actor(
    community_id: CommunityId,
    event_author_pubkey: [u8; 32],
    supersedes_revision: u64,
) -> Result<AuthorizationEventActor> {
    if community_id.as_uuid().is_nil() || supersedes_revision == 0 {
        return Err(DbError::InvalidData(
            "authorization status withdrawal actor is invalid".to_owned(),
        ));
    }
    Ok(local_actor(
        AuthorizationActorKind::Direct,
        community_id,
        &[&event_author_pubkey, &supersedes_revision.to_be_bytes()],
        None,
    ))
}

fn append_hash_optional(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            framed(digest, &[1]);
            framed(digest, value);
        }
        None => framed(digest, &[0]),
    }
}

/// Validated immutable authorization event ready for insertion.
#[derive(Clone, PartialEq, Eq)]
pub struct NewAuthorizationEvent {
    community_id: CommunityId,
    event_id: Uuid,
    event_kind: AuthorizationEventKind,
    outcome: AuthorizationEventOutcome,
    reason: AuthorizationReasonCode,
    actor: AuthorizationEventActor,
    subject_fingerprint: Option<[u8; 32]>,
    operation_id: Uuid,
    request_fingerprint: Option<[u8; 32]>,
    correlation_id: Uuid,
    attempt_id: Uuid,
}

impl NewAuthorizationEvent {
    /// Validate a redaction-safe canonical event envelope.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        community_id: CommunityId,
        event_id: Uuid,
        event_kind: AuthorizationEventKind,
        outcome: AuthorizationEventOutcome,
        reason: AuthorizationReasonCode,
        actor: AuthorizationEventActor,
        subject_fingerprint: Option<[u8; 32]>,
        operation_id: Uuid,
        request_fingerprint: Option<[u8; 32]>,
        correlation_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<Self> {
        let unresolved = actor.kind() == AuthorizationActorKind::Unresolved;
        if community_id.as_uuid().is_nil()
            || event_id.is_nil()
            || operation_id.is_nil()
            || correlation_id.is_nil()
            || attempt_id.is_nil()
            || (!unresolved && !actor.is_bound_to(community_id))
            || (unresolved && actor.community_id.is_some())
            || !valid_event_semantics(event_kind, outcome, reason, unresolved)
            || (unresolved
                && (event_kind != AuthorizationEventKind::OperatorDenied
                    || request_fingerprint.is_some()
                    || actor.fingerprint().is_some()
                    || subject_fingerprint.is_some()))
            || (!unresolved
                && (request_fingerprint.is_none()
                    || request_fingerprint == Some([0; 32])
                    || actor.fingerprint().is_none()))
        {
            return Err(DbError::InvalidData(
                "authorization event envelope is invalid".to_owned(),
            ));
        }
        Ok(Self {
            community_id,
            event_id,
            event_kind,
            outcome,
            reason,
            actor,
            subject_fingerprint,
            operation_id,
            request_fingerprint,
            correlation_id,
            attempt_id,
        })
    }
}

fn valid_event_semantics(
    event_kind: AuthorizationEventKind,
    outcome: AuthorizationEventOutcome,
    reason: AuthorizationReasonCode,
    unresolved: bool,
) -> bool {
    use AuthorizationEventKind as Kind;
    use AuthorizationEventOutcome as Outcome;
    use AuthorizationReasonCode as Reason;

    if unresolved {
        return event_kind == Kind::OperatorDenied
            && outcome == Outcome::Denied
            && matches!(
                reason,
                Reason::Missing | Reason::Invalid | Reason::Unauthenticated
            );
    }
    match event_kind {
        Kind::Enrolled
        | Kind::Revoked
        | Kind::Rotated
        | Kind::Recovered
        | Kind::PrincipalEnabled
        | Kind::Retired
        | Kind::PrincipalDisabled => {
            matches!(outcome, Outcome::Allowed | Outcome::NoOp)
                && matches!(reason, Reason::Current | Reason::Replay)
        }
        Kind::AdmissionLost => {
            matches!(outcome, Outcome::Allowed | Outcome::NoOp)
                && matches!(reason, Reason::Invalidated | Reason::Replay)
        }
        Kind::OperatorDenied | Kind::ProtectedDenied => {
            matches!(outcome, Outcome::Denied | Outcome::AuditUnavailable)
                && !matches!(reason, Reason::Current | Reason::Replay | Reason::Withdrawn)
        }
        Kind::ProtectedAllowed => outcome == Outcome::Allowed && reason == Reason::Current,
        Kind::StatusPublished => outcome == Outcome::Allowed && reason == Reason::Current,
        Kind::StatusWithdrawn => outcome == Outcome::Withdrawn && reason == Reason::Withdrawn,
        Kind::InvalidationAdvanced => outcome == Outcome::Allowed && reason == Reason::Invalidated,
    }
}

impl fmt::Debug for NewAuthorizationEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewAuthorizationEvent")
            .field("event_kind", &self.event_kind)
            .field("outcome", &self.outcome)
            .field("reason", &self.reason)
            .field("actor", &self.actor)
            .field("event_id", &"[REDACTED]")
            .field("operation_id", &"[REDACTED]")
            .finish()
    }
}

/// One immutable event returned from a bounded domain page.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationEventRecord {
    /// Stable event identity.
    pub event_id: Uuid,
    /// Canonical event class.
    pub event_kind: AuthorizationEventKind,
    /// Stable event outcome.
    pub outcome_code: i16,
    /// Stable reason code.
    pub reason_code: i16,
    /// Authoritative acceptance time.
    pub accepted_at: DateTime<Utc>,
    /// Canonical redaction-safe envelope.
    pub canonical_envelope: Vec<u8>,
    /// Digest of the exact envelope.
    pub envelope_digest: [u8; 32],
}

impl fmt::Debug for AuthorizationEventRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationEventRecord")
            .field("event_id", &"[REDACTED]")
            .field("event_kind", &self.event_kind)
            .field("outcome_code", &self.outcome_code)
            .field("reason_code", &self.reason_code)
            .field("accepted_at", &"[REDACTED]")
            .field("canonical_envelope", &"[REDACTED]")
            .field("envelope_digest", &"[REDACTED]")
            .finish()
    }
}

/// Stable keyset cursor for descending event pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizationEventPageCursor {
    /// Acceptance time of the final row on the prior page.
    pub accepted_at: DateTime<Utc>,
    /// Stable event ID breaking timestamp ties.
    pub event_id: Uuid,
}

/// One bounded page and its continuation cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationEventPage {
    /// Events ordered newest-first.
    pub events: Vec<AuthorizationEventRecord>,
    /// Cursor for the next older page.
    pub next: Option<AuthorizationEventPageCursor>,
}

/// Durable immutable-capacity health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationEventCapacityHealth {
    /// Whether new events may be accepted.
    pub healthy: bool,
    /// Retained event count.
    pub retained_event_count: u64,
    /// Retained canonical envelope bytes.
    pub retained_envelope_bytes: u64,
    /// Sticky failure code, when unhealthy.
    pub failure_code: Option<AuthorizationAuditFailureCode>,
    /// Monotonic failure generation.
    pub failure_generation: u64,
    /// Last generation recovered by an exact health probe.
    pub recovery_generation: u64,
    /// Events retained exclusively for restrictive/security evidence.
    pub restrictive_reserve_events: u64,
    /// Canonical bytes retained exclusively for restrictive/security evidence.
    pub restrictive_reserve_bytes: u64,
}

/// Sticky authorization-audit failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum AuthorizationAuditFailureCode {
    /// Configured immutable capacity was exhausted.
    CapacityExhausted = 1,
    /// PostgreSQL audit persistence was unavailable.
    StorageUnavailable = 2,
    /// Canonical envelope contract failed.
    InvalidEnvelope = 3,
}

impl AuthorizationAuditFailureCode {
    fn from_database(value: i16) -> Result<Self> {
        match value {
            1 => Ok(Self::CapacityExhausted),
            2 => Ok(Self::StorageUnavailable),
            3 => Ok(Self::InvalidEnvelope),
            _ => Err(DbError::InvalidData(
                "authorization audit failure code is invalid".to_owned(),
            )),
        }
    }
}

/// Typed classification of an event insert failure.
#[derive(Debug, Error)]
pub enum AuthorizationEventWriteError {
    /// Immutable audit capacity is absent, unhealthy, or exhausted.
    #[error("authorization event capacity is unavailable")]
    CapacityUnavailable,
    /// PostgreSQL or contract failure outside capacity enforcement.
    #[error(transparent)]
    Database(#[from] DbError),
}

/// Insert or exact-replay the single canonical operation receipt.
pub async fn record_authorization_operation_receipt_tx(
    transaction: &mut Transaction<'_, Postgres>,
    receipt: &AuthorizationOperationReceipt,
) -> Result<AuthorizationReceiptWrite> {
    if let Some(row) = sqlx::query(
        "SELECT request_fingerprint, operation_kind, actor_fingerprint, outcome_code, result_digest \
         FROM authorization_operation_receipts WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(receipt.community_id.as_uuid())
    .bind(receipt.operation_id)
    .fetch_optional(&mut **transaction)
    .await?
    {
        let exact = bytes32(row.try_get("request_fingerprint")?, "request fingerprint")?
            == receipt.request_fingerprint
            && row.try_get::<i16, _>("operation_kind")? == receipt.operation_kind as i16
            && bytes32(row.try_get("actor_fingerprint")?, "actor fingerprint")?
                == receipt.actor.fingerprint().ok_or_else(|| {
                    DbError::InvalidData(
                        "authorization operation actor is unauthenticated".to_owned(),
                    )
                })?
            && row.try_get::<i16, _>("outcome_code")? == receipt.outcome as i16
            && bytes32(row.try_get("result_digest")?, "result digest")?
                == receipt.result_digest;
        if exact {
            return Ok(AuthorizationReceiptWrite::ExactReplay);
        }
        return Err(DbError::InvalidData(
            "authorization operation intent conflicts with prior receipt".to_owned(),
        ));
    }

    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, request_fingerprint, operation_kind, actor_fingerprint, \
          outcome_code, result_digest) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(receipt.community_id.as_uuid())
    .bind(receipt.operation_id)
    .bind(receipt.request_fingerprint.as_slice())
    .bind(receipt.operation_kind as i16)
    .bind(
        receipt
            .actor
            .fingerprint()
            .ok_or_else(|| {
                DbError::InvalidData("authorization operation actor is unauthenticated".to_owned())
            })?
            .as_slice(),
    )
    .bind(receipt.outcome as i16)
    .bind(receipt.result_digest.as_slice())
    .execute(&mut **transaction)
    .await?;
    Ok(AuthorizationReceiptWrite::Inserted)
}

/// Insert or exact-replay one canonical event in a caller-owned transaction.
pub async fn record_authorization_event_tx(
    transaction: &mut Transaction<'_, Postgres>,
    event: &NewAuthorizationEvent,
) -> std::result::Result<AuthorizationReceiptWrite, AuthorizationEventWriteError> {
    if let Some(row) = sqlx::query(
        "SELECT event_id, outcome_code, reason_code, actor_kind, actor_fingerprint, \
                subject_fingerprint, request_fingerprint, correlation_id, occurred_at, \
                canonical_envelope, envelope_digest \
         FROM authorization_events \
         WHERE community_id=$1 AND operation_id=$2 AND event_kind=$3 AND attempt_id=$4",
    )
    .bind(event.community_id.as_uuid())
    .bind(event.operation_id)
    .bind(event.event_kind as i16)
    .bind(event.attempt_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| AuthorizationEventWriteError::Database(error.into()))?
    {
        let occurred_at: DateTime<Utc> = row.try_get("occurred_at").map_err(DbError::from)?;
        let canonical_envelope: Vec<u8> =
            row.try_get("canonical_envelope").map_err(DbError::from)?;
        let envelope_digest = bytes32(
            row.try_get("envelope_digest").map_err(DbError::from)?,
            "event envelope digest",
        )?;
        let expected_envelope = canonical_event_envelope(event, occurred_at);
        let calculated_envelope_digest: [u8; 32] = Sha256::digest(&canonical_envelope).into();
        let exact = row.try_get::<Uuid, _>("event_id").map_err(DbError::from)? == event.event_id
            && row
                .try_get::<i16, _>("outcome_code")
                .map_err(DbError::from)?
                == event.outcome as i16
            && row
                .try_get::<i16, _>("reason_code")
                .map_err(DbError::from)?
                == event.reason as i16
            && row.try_get::<i16, _>("actor_kind").map_err(DbError::from)?
                == event.actor.kind() as i16
            && optional_bytes32(
                row.try_get("actor_fingerprint").map_err(DbError::from)?,
                "actor fingerprint",
            )? == event.actor.fingerprint()
            && optional_bytes32(
                row.try_get("subject_fingerprint").map_err(DbError::from)?,
                "subject fingerprint",
            )? == event.subject_fingerprint
            && optional_bytes32(
                row.try_get("request_fingerprint").map_err(DbError::from)?,
                "request fingerprint",
            )? == event.request_fingerprint
            && row
                .try_get::<Uuid, _>("correlation_id")
                .map_err(DbError::from)?
                == event.correlation_id
            && canonical_envelope == expected_envelope
            && envelope_digest == calculated_envelope_digest;
        if exact {
            return Ok(AuthorizationReceiptWrite::ExactReplay);
        }
        return Err(AuthorizationEventWriteError::Database(
            DbError::InvalidData(
                "authorization event attempt conflicts with prior evidence".to_owned(),
            ),
        ));
    }

    let occurred_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| AuthorizationEventWriteError::Database(error.into()))?;
    let canonical_envelope = canonical_event_envelope(event, occurred_at);
    let envelope_digest: [u8; 32] = Sha256::digest(&canonical_envelope).into();
    let result = sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,actor_fingerprint, \
          subject_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
          occurred_at,canonical_envelope,envelope_digest) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(event.community_id.as_uuid())
    .bind(event.event_id)
    .bind(event.event_kind as i16)
    .bind(event.outcome as i16)
    .bind(event.reason as i16)
    .bind(event.actor.kind() as i16)
    .bind(event.actor.fingerprint().map(|value| value.to_vec()))
    .bind(event.subject_fingerprint.map(|value| value.to_vec()))
    .bind(event.operation_id)
    .bind(event.request_fingerprint.map(|value| value.to_vec()))
    .bind(event.correlation_id)
    .bind(event.attempt_id)
    .bind(occurred_at)
    .bind(&canonical_envelope)
    .bind(envelope_digest.as_slice())
    .execute(&mut **transaction)
    .await;
    match result {
        Ok(_) => Ok(AuthorizationReceiptWrite::Inserted),
        Err(error) if is_capacity_constraint(&error) => {
            Err(AuthorizationEventWriteError::CapacityUnavailable)
        }
        Err(error) => Err(AuthorizationEventWriteError::Database(error.into())),
    }
}

impl Db {
    /// Atomically install and reconcile the complete enabled-domain capacity
    /// set before production authorization becomes observable.
    ///
    /// The entire input is validated before a connection is acquired. Every
    /// row is then locked and required to have the exact immutable policy and
    /// a healthy, internally consistent retained-capacity state. Any conflict
    /// or unhealthy row rolls back newly inserted rows from this call.
    pub async fn reconcile_authorization_event_capacities(
        &self,
        capacities: &[(CommunityId, AuthorizationEventCapacityPolicy)],
    ) -> Result<()> {
        validate_authorization_event_capacities(capacities)?;
        if capacities.is_empty() {
            return Ok(());
        }

        let mut transaction = self.pool.begin().await?;
        if let Err(error) =
            reconcile_authorization_event_capacities_tx(&mut transaction, capacities).await
        {
            transaction.rollback().await?;
            return Err(error);
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Install the immutable capacity policy for one enabled domain.
    ///
    /// Exact replay is accepted; changed limits require offline migration.
    pub async fn install_authorization_event_capacity(
        &self,
        community_id: CommunityId,
        policy: AuthorizationEventCapacityPolicy,
    ) -> Result<()> {
        self.reconcile_authorization_event_capacities(&[(community_id, policy)])
            .await
    }

    /// Latch one domain's audit capacity unhealthy at a new generation.
    pub async fn latch_authorization_event_failure(
        &self,
        community_id: CommunityId,
        failure: AuthorizationAuditFailureCode,
    ) -> Result<()> {
        let generation: i64 =
            sqlx::query_scalar("SELECT authorization_event_capacity_report_failure_v2($1,$2)")
                .bind(community_id.as_uuid())
                .bind(failure as i16)
                .fetch_one(&self.pool)
                .await?;
        if generation <= 0 {
            return Err(DbError::InvalidData(
                "authorization audit failure generation is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    /// Recover health only if no later failure generation raced the probe.
    pub async fn recover_authorization_event_capacity(
        &self,
        community_id: CommunityId,
        expected_failure_generation: u64,
    ) -> Result<()> {
        let expected = i64::try_from(expected_failure_generation).map_err(|_| {
            DbError::InvalidData("authorization audit recovery generation is invalid".to_owned())
        })?;
        if expected == 0 {
            return Err(DbError::InvalidData(
                "authorization audit recovery generation is invalid".to_owned(),
            ));
        }
        let recovered: bool =
            sqlx::query_scalar("SELECT authorization_event_capacity_recover_v2($1,$2)")
                .bind(community_id.as_uuid())
                .bind(expected)
                .fetch_one(&self.pool)
                .await?;
        if recovered {
            Ok(())
        } else {
            Err(DbError::InvalidData(
                "authorization audit recovery generation is stale".to_owned(),
            ))
        }
    }

    /// Increment one fixed-cardinality credential-free operator denial bucket.
    pub async fn record_operator_denial_bucket(
        &self,
        community_id: CommunityId,
        denial_class: u8,
        action_kind: u8,
    ) -> Result<u64> {
        if community_id.as_uuid().is_nil()
            || !(1..=8).contains(&denial_class)
            || !(1..=8).contains(&action_kind)
        {
            return Err(DbError::InvalidData(
                "operator denial bucket coordinate is invalid".to_owned(),
            ));
        }
        let count: i64 =
            sqlx::query_scalar("SELECT authorization_operator_denial_bucket_record_v1($1,$2,$3)")
                .bind(community_id.as_uuid())
                .bind(i16::from(denial_class))
                .bind(i16::from(action_kind))
                .fetch_one(&self.pool)
                .await?;
        nonnegative(count).and_then(|value| {
            if value == 0 {
                Err(DbError::InvalidData(
                    "operator denial bucket count is invalid".to_owned(),
                ))
            } else {
                Ok(value)
            }
        })
    }

    /// Increment one fixed-cardinality, redacted protected-denial bucket.
    ///
    /// This uses an independent autocommit statement, so a denial remains
    /// observable after an admission transaction rolls back. The bucket stores
    /// only closed coordinates, a bounded count, and authoritative timestamps.
    pub async fn record_protected_denial_bucket(
        &self,
        community_id: CommunityId,
        surface: ProtectedDenialSurface,
        reason: ProtectedDenialReason,
        action: ProtectedDenialAction,
    ) -> Result<u64> {
        if community_id.as_uuid().is_nil() {
            return Err(DbError::InvalidData(
                "protected denial bucket coordinate is invalid".to_owned(),
            ));
        }
        let count: i64 =
            sqlx::query_scalar("SELECT authorization_denial_bucket_record_v1($1,$2,$3,$4)")
                .bind(community_id.as_uuid())
                .bind(surface as i16)
                .bind(reason as i16)
                .bind(action as i16)
                .fetch_one(&self.pool)
                .await?;
        nonnegative(count).and_then(|value| {
            if value == 0 {
                Err(DbError::InvalidData(
                    "protected denial bucket count is invalid".to_owned(),
                ))
            } else {
                Ok(value)
            }
        })
    }

    /// Read immutable-capacity health for readiness and operator controls.
    pub async fn authorization_event_capacity_health(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationEventCapacityHealth> {
        let row = sqlx::query(
            "SELECT retained_event_count,retained_envelope_bytes,health_state,failure_code, \
                    failure_generation,recovery_generation,restrictive_reserve_events, \
                    restrictive_reserve_bytes \
             FROM authorization_event_capacity WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::NotFound("authorization event capacity policy".to_owned()))?;
        let health_state: i16 = row.try_get("health_state")?;
        let failure_code: Option<i16> = row.try_get("failure_code")?;
        Ok(AuthorizationEventCapacityHealth {
            healthy: health_state == 1,
            retained_event_count: nonnegative(row.try_get("retained_event_count")?)?,
            retained_envelope_bytes: nonnegative(row.try_get("retained_envelope_bytes")?)?,
            failure_code: failure_code
                .map(AuthorizationAuditFailureCode::from_database)
                .transpose()?,
            failure_generation: nonnegative(row.try_get("failure_generation")?)?,
            recovery_generation: nonnegative(row.try_get("recovery_generation")?)?,
            restrictive_reserve_events: nonnegative(row.try_get("restrictive_reserve_events")?)?,
            restrictive_reserve_bytes: nonnegative(row.try_get("restrictive_reserve_bytes")?)?,
        })
    }

    /// Read one bounded, domain-scoped immutable event page.
    pub async fn authorization_event_page(
        &self,
        community_id: CommunityId,
        cursor: Option<AuthorizationEventPageCursor>,
        limit: u16,
    ) -> Result<AuthorizationEventPage> {
        if community_id.as_uuid().is_nil() || limit == 0 || limit > MAX_EVENT_PAGE {
            return Err(DbError::InvalidData(
                "authorization event page bounds are invalid".to_owned(),
            ));
        }
        let rows = sqlx::query(
            "SELECT event_id,event_kind,outcome_code,reason_code,accepted_at, \
                    canonical_envelope,envelope_digest \
             FROM authorization_events \
             WHERE community_id=$1 \
               AND ($2::timestamptz IS NULL OR (accepted_at,event_id) < ($2,$3)) \
             ORDER BY accepted_at DESC,event_id DESC LIMIT $4",
        )
        .bind(community_id.as_uuid())
        .bind(cursor.map(|value| value.accepted_at))
        .bind(cursor.map(|value| value.event_id))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(AuthorizationEventRecord {
                event_id: row.try_get("event_id")?,
                event_kind: AuthorizationEventKind::from_database(row.try_get("event_kind")?)?,
                outcome_code: row.try_get("outcome_code")?,
                reason_code: row.try_get("reason_code")?,
                accepted_at: row.try_get("accepted_at")?,
                canonical_envelope: row.try_get("canonical_envelope")?,
                envelope_digest: bytes32(row.try_get("envelope_digest")?, "envelope digest")?,
            });
        }
        let next = events.last().map(|event| AuthorizationEventPageCursor {
            accepted_at: event.accepted_at,
            event_id: event.event_id,
        });
        Ok(AuthorizationEventPage { events, next })
    }
}

/// Validate a complete set of server-owned per-domain audit capacity policies.
pub fn validate_authorization_event_capacities(
    capacities: &[(CommunityId, AuthorizationEventCapacityPolicy)],
) -> Result<()> {
    let mut domains = HashSet::with_capacity(capacities.len());
    for (community_id, _) in capacities {
        if community_id.as_uuid().is_nil() || !domains.insert(*community_id) {
            return Err(DbError::InvalidData(
                "authorization event capacity domain set is invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

/// Reconcile validated capacity policies inside the caller's configuration transaction.
pub async fn reconcile_authorization_event_capacities_tx(
    transaction: &mut Transaction<'_, Postgres>,
    capacities: &[(CommunityId, AuthorizationEventCapacityPolicy)],
) -> Result<()> {
    for (community_id, policy) in capacities {
        let max_events = i64::try_from(policy.max_events_per_domain()).map_err(|_| {
            DbError::InvalidData("authorization event capacity is out of range".to_owned())
        })?;
        let max_bytes = i64::try_from(policy.max_bytes_per_domain()).map_err(|_| {
            DbError::InvalidData("authorization byte capacity is out of range".to_owned())
        })?;
        let max_envelope = i32::try_from(policy.max_envelope_bytes()).map_err(|_| {
            DbError::InvalidData("authorization envelope capacity is out of range".to_owned())
        })?;
        let reserve_events = if max_events > 1 {
            (max_events / 8).clamp(1, 64)
        } else {
            0
        };
        let reserve_bytes = if max_bytes > i64::from(max_envelope) {
            (max_bytes / 8).clamp(i64::from(max_envelope), 262_144)
        } else {
            0
        };
        sqlx::query(
            "INSERT INTO authorization_event_capacity \
             (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes, \
              restrictive_reserve_events,restrictive_reserve_bytes) \
             VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .bind(max_events)
        .bind(max_bytes)
        .bind(max_envelope)
        .bind(reserve_events)
        .bind(reserve_bytes)
        .execute(&mut **transaction)
        .await?;

        let row = sqlx::query(
            "SELECT max_events_per_domain,max_bytes_per_domain,max_envelope_bytes, \
                    retained_event_count,retained_envelope_bytes,health_state,failure_code \
             FROM authorization_event_capacity WHERE community_id=$1 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await?;
        let exact_policy = row.try_get::<i64, _>("max_events_per_domain")?
            == i64::try_from(policy.max_events_per_domain()).unwrap_or(i64::MAX)
            && row.try_get::<i64, _>("max_bytes_per_domain")?
                == i64::try_from(policy.max_bytes_per_domain()).unwrap_or(i64::MAX)
            && row.try_get::<i32, _>("max_envelope_bytes")?
                == i32::try_from(policy.max_envelope_bytes()).unwrap_or(i32::MAX);
        let retained_events: i64 = row.try_get("retained_event_count")?;
        let retained_bytes: i64 = row.try_get("retained_envelope_bytes")?;
        let healthy = row.try_get::<i16, _>("health_state")? == 1
            && row.try_get::<Option<i16>, _>("failure_code")?.is_none()
            && retained_events >= 0
            && retained_events <= i64::try_from(policy.max_events_per_domain()).unwrap_or(i64::MAX)
            && retained_bytes >= 0
            && retained_bytes <= i64::try_from(policy.max_bytes_per_domain()).unwrap_or(i64::MAX);
        if !exact_policy || !healthy {
            return Err(DbError::InvalidData(
                if exact_policy {
                    "authorization event capacity is unhealthy"
                } else {
                    "authorization event capacity conflicts with installed policy"
                }
                .to_owned(),
            ));
        }
    }
    Ok(())
}

fn is_capacity_constraint(error: &sqlx::Error) -> bool {
    error.as_database_error().is_some_and(|database| {
        matches!(
            database.constraint(),
            Some(
                "authorization_event_capacity_policy_required"
                    | "authorization_event_capacity_health"
                    | "authorization_event_capacity_exhausted"
                    | "authorization_events_envelope_size"
            )
        )
    })
}

/// Encode the versioned, bounded, credential-free canonical event envelope.
pub fn canonical_event_envelope(
    event: &NewAuthorizationEvent,
    occurred_at: DateTime<Utc>,
) -> Vec<u8> {
    let mut envelope = Vec::with_capacity(288);
    envelope.extend_from_slice(b"buzz:authorization-event:v1");
    envelope.extend_from_slice(event.community_id.as_uuid().as_bytes());
    envelope.extend_from_slice(event.event_id.as_bytes());
    envelope.extend_from_slice(&(event.event_kind as i16).to_be_bytes());
    envelope.extend_from_slice(&(event.outcome as i16).to_be_bytes());
    envelope.extend_from_slice(&(event.reason as i16).to_be_bytes());
    envelope.extend_from_slice(&(event.actor.kind() as i16).to_be_bytes());
    append_optional_bytes32(&mut envelope, event.actor.fingerprint());
    append_optional_bytes32(&mut envelope, event.subject_fingerprint);
    envelope.extend_from_slice(event.operation_id.as_bytes());
    append_optional_bytes32(&mut envelope, event.request_fingerprint);
    envelope.extend_from_slice(event.correlation_id.as_bytes());
    envelope.extend_from_slice(event.attempt_id.as_bytes());
    envelope.extend_from_slice(&occurred_at.timestamp().to_be_bytes());
    envelope.extend_from_slice(&occurred_at.timestamp_subsec_nanos().to_be_bytes());
    envelope
}

fn append_optional_bytes32(envelope: &mut Vec<u8>, value: Option<[u8; 32]>) {
    match value {
        Some(value) => {
            envelope.push(1);
            envelope.extend_from_slice(&value);
        }
        None => envelope.push(0),
    }
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn bytes32(value: Vec<u8>, name: &str) -> Result<[u8; 32]> {
    value
        .try_into()
        .map_err(|_| DbError::InvalidData(format!("authorization {name} is malformed")))
}

fn optional_bytes32(value: Option<Vec<u8>>, name: &str) -> Result<Option<[u8; 32]>> {
    value.map(|value| bytes32(value, name)).transpose()
}

fn nonnegative(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("authorization capacity is negative".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(1))
    }

    fn direct_actor() -> AuthorizationEventActor {
        actor_from_route_coordinates(domain(), [3; 32], None, (Uuid::from_u128(4), 5), None)
            .expect("valid sealed-route shape")
    }

    #[test]
    fn unresolved_events_cannot_carry_identity_or_receipt_evidence() {
        let common = || {
            NewAuthorizationEvent::new(
                domain(),
                Uuid::from_u128(2),
                AuthorizationEventKind::OperatorDenied,
                AuthorizationEventOutcome::Denied,
                AuthorizationReasonCode::Unauthenticated,
                AuthorizationEventActor::unresolved_authentication_denial(),
                None,
                Uuid::from_u128(3),
                None,
                Uuid::from_u128(4),
                Uuid::from_u128(5),
            )
        };
        assert!(common().is_ok());
        let mut event = common().expect("valid unresolved event");
        event.actor.fingerprint = Some([9; 32]);
        assert!(NewAuthorizationEvent::new(
            event.community_id,
            event.event_id,
            event.event_kind,
            event.outcome,
            event.reason,
            event.actor,
            None,
            event.operation_id,
            None,
            event.correlation_id,
            event.attempt_id,
        )
        .is_err());
    }

    #[test]
    fn event_and_receipt_debug_are_redacted() {
        let receipt = AuthorizationOperationReceipt::new(
            domain(),
            Uuid::from_u128(2),
            [3; 32],
            AuthorizationOperationKind::ProtectedMutation,
            direct_actor(),
            AuthorizationOperationOutcome::Applied,
            [5; 32],
        )
        .expect("valid receipt");
        let debug = format!("{receipt:?}");
        assert!(!debug.contains(&hex::encode([3; 32])));
        assert!(!debug.contains(&hex::encode([4; 32])));
        assert!(!debug.contains(&hex::encode([5; 32])));
    }

    #[test]
    fn page_limits_are_bounded() {
        assert_eq!(MAX_EVENT_PAGE, 1_000);
        assert_eq!(AuthorizationAuditFailureCode::CapacityExhausted as i16, 1);
        assert_eq!(AuthorizationReasonCode::ReplayRejected as i16, 17);
    }

    #[test]
    fn canonical_event_encoder_is_stable_and_binds_typed_fields() {
        let event = NewAuthorizationEvent::new(
            domain(),
            Uuid::from_u128(2),
            AuthorizationEventKind::ProtectedAllowed,
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Current,
            direct_actor(),
            Some([4; 32]),
            Uuid::from_u128(5),
            Some([6; 32]),
            Uuid::from_u128(7),
            Uuid::from_u128(8),
        )
        .expect("valid event");
        let occurred_at = DateTime::from_timestamp(9, 10).expect("valid timestamp");
        let first = canonical_event_envelope(&event, occurred_at);
        let second = canonical_event_envelope(&event, occurred_at);
        assert_eq!(first, second);
        assert!(first.len() <= 16_384);

        let changed = NewAuthorizationEvent::new(
            domain(),
            Uuid::from_u128(2),
            AuthorizationEventKind::ProtectedDenied,
            AuthorizationEventOutcome::Denied,
            AuthorizationReasonCode::PolicyDenied,
            direct_actor(),
            Some([4; 32]),
            Uuid::from_u128(5),
            Some([6; 32]),
            Uuid::from_u128(7),
            Uuid::from_u128(8),
        )
        .expect("valid changed event");
        assert_ne!(first, canonical_event_envelope(&changed, occurred_at));
    }

    #[test]
    fn actor_route_shape_is_closed_and_debug_is_redacted() {
        assert!(actor_from_route_coordinates(
            domain(),
            [1; 32],
            Some([2; 32]),
            (Uuid::from_u128(3), 4),
            None,
        )
        .is_err());
        let actor = direct_actor();
        assert_eq!(actor.kind(), AuthorizationActorKind::Direct);
        assert!(!format!("{actor:?}").contains(&hex::encode([3; 32])));
    }

    #[test]
    fn authenticated_actor_cannot_cross_domains() {
        let actor = direct_actor();
        let other = CommunityId::from_uuid(Uuid::from_u128(99));
        assert!(!actor.is_bound_to(other));
        assert!(AuthorizationOperationReceipt::new(
            other,
            Uuid::from_u128(20),
            [21; 32],
            AuthorizationOperationKind::ProtectedMutation,
            actor.clone(),
            AuthorizationOperationOutcome::Applied,
            [22; 32],
        )
        .is_err());
        assert!(NewAuthorizationEvent::new(
            other,
            Uuid::from_u128(23),
            AuthorizationEventKind::ProtectedAllowed,
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Current,
            actor,
            None,
            Uuid::from_u128(20),
            Some([21; 32]),
            Uuid::from_u128(24),
            Uuid::from_u128(25),
        )
        .is_err());
    }

    #[test]
    fn canonical_event_semantics_reject_contradictions() {
        let base = |kind, outcome, reason| {
            NewAuthorizationEvent::new(
                domain(),
                Uuid::from_u128(30),
                kind,
                outcome,
                reason,
                direct_actor(),
                None,
                Uuid::from_u128(31),
                Some([32; 32]),
                Uuid::from_u128(33),
                Uuid::from_u128(34),
            )
        };
        assert!(base(
            AuthorizationEventKind::StatusPublished,
            AuthorizationEventOutcome::Denied,
            AuthorizationReasonCode::Withdrawn,
        )
        .is_err());
        assert!(base(
            AuthorizationEventKind::InvalidationAdvanced,
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Current,
        )
        .is_err());
        assert!(base(
            AuthorizationEventKind::ProtectedAllowed,
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Current,
        )
        .is_ok());
    }
}
