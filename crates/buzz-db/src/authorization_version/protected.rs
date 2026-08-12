//! High-level DB-authoritative publication for protected immutable content.
//!
//! Content bytes are staged under their immutable digest before this API is
//! called. PostgreSQL then owns visibility: one transaction re-fences the
//! sealed route, advances the exact protected-object authority, and records
//! the canonical receipt, event, and version manifest. External pointers are
//! caches and must not be exposed without a final witness recheck.

use std::fmt;

use buzz_auth::{
    AuthorizationLeaseDependencySnapshot, FinalizedAuthContext, ProofTransport, RouteCapability,
};
use buzz_core::{AuthorizationLeaseFence, CommunityId};
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::{
    authorization_lease_fence, authorization_version_authority_epoch_component_key, bytes32,
    framed, from_database_version, load_manifest_connection, next_authority_fence,
    record_authorization_operation_version_delta_tx, to_database_version,
    AuthorizationOperationFence, AuthorizationOperationVersionDelta,
    AuthorizationProtectedObjectKind, AuthorizationVersionComponentKind,
};
use crate::{
    authorization_events::{
        record_authorization_event_tx, record_authorization_operation_receipt_tx,
        AuthorizationAuditFailureCode, AuthorizationEventActor, AuthorizationEventKind,
        AuthorizationEventOutcome, AuthorizationEventWriteError, AuthorizationOperationKind,
        AuthorizationOperationOutcome, AuthorizationOperationReceipt, AuthorizationReasonCode,
        AuthorizationReceiptWrite, NewAuthorizationEvent,
    },
    authorization_invalidation::AuthorizationInvalidationSelector,
    Db, DbError, Result,
};

/// Canonical protected object derived from an origin-sealed route.
#[derive(Clone, PartialEq, Eq)]
pub struct ProtectedPublicationCoordinate {
    community_id: CommunityId,
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
    target_fingerprint: [u8; 32],
}

impl ProtectedPublicationCoordinate {
    /// Derive the canonical repository coordinate for a Git/repository route.
    pub fn repository(context: &FinalizedAuthContext) -> Result<Self> {
        Self::from_context(context, AuthorizationProtectedObjectKind::Repository)
    }

    /// Derive the canonical immutable-media coordinate for a media route.
    pub fn media(context: &FinalizedAuthContext) -> Result<Self> {
        Self::from_context(context, AuthorizationProtectedObjectKind::Media)
    }

    /// Derive the canonical moderation-target coordinate.
    pub fn moderation_target(context: &FinalizedAuthContext) -> Result<Self> {
        Self::from_context(context, AuthorizationProtectedObjectKind::ModerationTarget)
    }

    fn from_context(
        context: &FinalizedAuthContext,
        object_kind: AuthorizationProtectedObjectKind,
    ) -> Result<Self> {
        if !capability_reads_object(context.capability(), object_kind) {
            return Err(DbError::InvalidData(
                "authorization route cannot address this protected object".to_owned(),
            ));
        }
        let (_, target_fingerprint, _) = context.lease().request_binding();
        if *target_fingerprint == [0; 32] {
            return Err(DbError::InvalidData(
                "authorization protected target is invalid".to_owned(),
            ));
        }
        let object_key = protected_object_key(
            context.authorization_domain(),
            object_kind,
            *target_fingerprint,
        );
        Ok(Self {
            community_id: context.authorization_domain(),
            object_kind,
            object_key,
            target_fingerprint: *target_fingerprint,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.community_id
    }

    /// Closed protected-object class.
    pub const fn object_kind(&self) -> AuthorizationProtectedObjectKind {
        self.object_kind
    }
}

impl fmt::Debug for ProtectedPublicationCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPublicationCoordinate")
            .field("object_kind", &self.object_kind)
            .field("coordinate", &"[REDACTED]")
            .finish()
    }
}

/// Server-generated identities for one publication attempt.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPublicationOperationIdentity {
    operation_id: Uuid,
    event_id: Uuid,
    attempt_id: Uuid,
}

/// Read-only restore coordinates sealed by a validated publication request.
///
/// Callers cannot construct or alter these coordinates; operation restore
/// consumes the exact request fingerprint computed by the database boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPublicationRestoreIdentity {
    authorization_domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
}

impl ProtectedPublicationRestoreIdentity {
    /// Exact server-resolved authorization domain.
    pub const fn authorization_domain(self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact server-generated operation receipt identity.
    pub const fn operation_id(self) -> Uuid {
        self.operation_id
    }

    /// Canonical fingerprint of the complete sealed publication request.
    pub const fn request_fingerprint(self) -> [u8; 32] {
        self.request_fingerprint
    }
}

impl fmt::Debug for ProtectedPublicationRestoreIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedPublicationRestoreIdentity([REDACTED])")
    }
}

impl ProtectedPublicationOperationIdentity {
    /// Validate separately generated operation, event, and attempt IDs.
    pub fn new(operation_id: Uuid, event_id: Uuid, attempt_id: Uuid) -> Result<Self> {
        if operation_id.is_nil() || event_id.is_nil() || attempt_id.is_nil() {
            return Err(DbError::InvalidData(
                "authorization protected publication identity is invalid".to_owned(),
            ));
        }
        Ok(Self {
            operation_id,
            event_id,
            attempt_id,
        })
    }

    /// Stable operation receipt identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }
}

impl fmt::Debug for ProtectedPublicationOperationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedPublicationOperationIdentity([REDACTED])")
    }
}

/// Sealed current DB publication authority returned to immutable fetchers.
#[derive(Clone)]
pub struct ProtectedPublicationWitness {
    coordinate: ProtectedPublicationCoordinate,
    dependency: ProtectedPublicationDependency,
    result_digest: [u8; 32],
    authority_epoch: u64,
    invalidation_generation: u64,
    fence: AuthorizationLeaseFence,
    authoritative_now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
}

impl ProtectedPublicationWitness {
    /// Exact immutable content digest committed by the canonical receipt.
    pub const fn result_digest(&self) -> [u8; 32] {
        self.result_digest
    }

    /// Current protected-object authority epoch.
    pub const fn authority_epoch(&self) -> u64 {
        self.authority_epoch
    }

    /// Current nonzero observable fence.
    pub const fn fence(&self) -> AuthorizationLeaseFence {
        self.fence
    }

    /// Domain generation included in the joined dependency snapshot.
    pub const fn invalidation_generation(&self) -> u64 {
        self.invalidation_generation
    }

    /// PostgreSQL time sampled by the query that sealed this snapshot.
    pub const fn authoritative_now(&self) -> DateTime<Utc> {
        self.authoritative_now
    }

    /// Persisted exclusive lease expiry for this publication authority.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Canonical protected object this witness seals.
    pub const fn coordinate(&self) -> &ProtectedPublicationCoordinate {
        &self.coordinate
    }

    /// Opaque dependency coordinate used by the live observer.
    pub const fn dependency(&self) -> &ProtectedPublicationDependency {
        &self.dependency
    }
}

impl PartialEq for ProtectedPublicationWitness {
    fn eq(&self, other: &Self) -> bool {
        self.coordinate == other.coordinate
            && self.dependency == other.dependency
            && self.result_digest == other.result_digest
            && self.authority_epoch == other.authority_epoch
            && self.invalidation_generation == other.invalidation_generation
            && self.fence == other.fence
            && self.expires_at == other.expires_at
            && self.operation_id == other.operation_id
            && self.request_fingerprint == other.request_fingerprint
    }
}

/// Opaque exact protected-object dependency coordinate.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProtectedPublicationDependency {
    community_id: CommunityId,
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
}

impl ProtectedPublicationDependency {
    fn from_coordinate(coordinate: &ProtectedPublicationCoordinate) -> Self {
        Self {
            community_id: coordinate.community_id,
            object_kind: coordinate.object_kind,
            object_key: coordinate.object_key,
        }
    }

    pub(crate) fn from_database_parts(
        community_id: CommunityId,
        object_kind: AuthorizationProtectedObjectKind,
        object_key: [u8; 32],
    ) -> Result<Self> {
        if community_id.as_uuid().is_nil() || object_key == [0; 32] {
            return Err(DbError::InvalidData(
                "authorization publication dependency is invalid".to_owned(),
            ));
        }
        Ok(Self {
            community_id,
            object_kind,
            object_key,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.community_id
    }

    pub(crate) const fn object_kind(&self) -> AuthorizationProtectedObjectKind {
        self.object_kind
    }

    pub(crate) const fn object_key(&self) -> [u8; 32] {
        self.object_key
    }
}

impl fmt::Debug for ProtectedPublicationDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedPublicationDependency([REDACTED])")
    }
}

/// PostgreSQL time and exact persisted expiry returned by a final recheck.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPublicationValidity {
    authoritative_now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl ProtectedPublicationValidity {
    /// PostgreSQL time sampled during the final recheck.
    pub const fn authoritative_now(self) -> DateTime<Utc> {
        self.authoritative_now
    }

    /// Persisted exclusive authority expiry.
    pub const fn expires_at(self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl fmt::Debug for ProtectedPublicationValidity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedPublicationValidity([REDACTED])")
    }
}

impl Eq for ProtectedPublicationWitness {}

impl fmt::Debug for ProtectedPublicationWitness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPublicationWitness")
            .field("object_kind", &self.coordinate.object_kind)
            .field("authority", &"[REDACTED]")
            .field("result_digest", &"[REDACTED]")
            .finish()
    }
}

/// One staged immutable publication and its exact expected parent.
pub struct ProtectedPublicationRequest {
    coordinate: ProtectedPublicationCoordinate,
    operation: ProtectedPublicationOperationIdentity,
    authority: ProtectedPublicationAuthority,
    staged_digest: [u8; 32],
    expected_parent_result_digest: Option<[u8; 32]>,
    request_fingerprint: [u8; 32],
}

impl ProtectedPublicationRequest {
    /// Bind a nonzero staged digest and optional expected parent to one sealed route.
    pub fn new(
        context: &FinalizedAuthContext,
        coordinate: ProtectedPublicationCoordinate,
        operation: ProtectedPublicationOperationIdentity,
        staged_digest: [u8; 32],
        expected_parent_result_digest: Option<[u8; 32]>,
    ) -> Result<Self> {
        if staged_digest == [0; 32]
            || expected_parent_result_digest == Some([0; 32])
            || coordinate.community_id != context.authorization_domain()
            || !capability_mutates_object(context.capability(), coordinate.object_kind)
        {
            return Err(DbError::InvalidData(
                "authorization protected publication request is invalid".to_owned(),
            ));
        }
        let authority = ProtectedPublicationAuthority::from_context(context)?;
        if authority.target_fingerprint != coordinate.target_fingerprint {
            return Err(DbError::InvalidData(
                "authorization protected publication target changed".to_owned(),
            ));
        }
        let request_fingerprint = publication_request_fingerprint(
            &coordinate,
            operation,
            &authority,
            staged_digest,
            expected_parent_result_digest,
        );
        Ok(Self {
            coordinate,
            operation,
            authority,
            staged_digest,
            expected_parent_result_digest,
            request_fingerprint,
        })
    }

    /// Exact read-only operation restore identity sealed with this request.
    pub const fn restore_identity(&self) -> ProtectedPublicationRestoreIdentity {
        ProtectedPublicationRestoreIdentity {
            authorization_domain: self.coordinate.community_id,
            operation_id: self.operation.operation_id,
            request_fingerprint: self.request_fingerprint,
        }
    }
}

impl fmt::Debug for ProtectedPublicationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPublicationRequest")
            .field("object_kind", &self.coordinate.object_kind)
            .field("operation", &self.operation)
            .field("payload", &"[REDACTED]")
            .finish()
    }
}

/// Whether a protected publication created state or exactly replayed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedPublicationDisposition {
    /// New publication authority was committed.
    Applied,
    /// The complete prior receipt, event, manifest, and authority replayed.
    ExactReplay,
}

/// Canonical committed result of a staged publication.
#[derive(Clone, PartialEq, Eq)]
pub struct ProtectedPublicationCommit {
    disposition: ProtectedPublicationDisposition,
    witness: ProtectedPublicationWitness,
}

impl ProtectedPublicationCommit {
    /// Applied versus exact replay.
    pub const fn disposition(&self) -> ProtectedPublicationDisposition {
        self.disposition
    }

    /// Exact immutable digest committed by the canonical receipt.
    pub const fn result_digest(&self) -> [u8; 32] {
        self.witness.result_digest
    }

    /// Current read witness for the committed publication.
    pub const fn witness(&self) -> &ProtectedPublicationWitness {
        &self.witness
    }

    /// Consume the result and retain its read witness.
    pub fn into_witness(self) -> ProtectedPublicationWitness {
        self.witness
    }
}

impl fmt::Debug for ProtectedPublicationCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPublicationCommit")
            .field("disposition", &self.disposition)
            .field("witness", &self.witness)
            .finish()
    }
}

/// Fail-closed publication outcome.
#[derive(Debug, Error)]
pub enum ProtectedPublicationError {
    /// The expected parent or operation intent no longer matches.
    #[error("protected publication conflicts with current authority")]
    Conflict,
    /// The sealed route or final dependency fence is stale.
    #[error("protected publication authorization is stale")]
    StaleAuthorization,
    /// Canonical durable authorization evidence could not be recorded.
    #[error("protected publication audit is unavailable")]
    AuditUnavailable,
    /// PostgreSQL contract or storage failure outside the canonical event write.
    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
struct ProtectedPublicationAuthority {
    community_id: CommunityId,
    correlation_id: Uuid,
    actor: AuthorizationEventActor,
    capability: RouteCapability,
    actor_pubkey: [u8; 32],
    owner_pubkey: Option<[u8; 32]>,
    binding_id: Uuid,
    binding_version: u64,
    relationship_id: Option<Uuid>,
    relationship_revision: Option<u64>,
    delegation_conditions_fingerprint: Option<[u8; 32]>,
    source_request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport: ProofTransport,
    transport_context_fingerprint: [u8; 32],
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl ProtectedPublicationAuthority {
    fn from_context(context: &FinalizedAuthContext) -> Result<Self> {
        let actor = AuthorizationEventActor::from_auth_context(context)?;
        let snapshot = context.lease().dependency_snapshot();
        Self::from_snapshot(context.correlation_id(), actor, snapshot)
    }

    fn from_snapshot(
        correlation_id: Uuid,
        actor: AuthorizationEventActor,
        snapshot: AuthorizationLeaseDependencySnapshot,
    ) -> Result<Self> {
        let (_, community_id) = snapshot.identity();
        let (capability, actor_pubkey, owner_pubkey) = snapshot.authority();
        let (binding_id, binding_version) = snapshot.binding();
        let (source_request_fingerprint, target_fingerprint, transport, transport_context) =
            snapshot.request_binding();
        let (policy_revision, invalidation_generation, authority_epoch) =
            snapshot.dependency_versions();
        let (issued_at, expires_at) = snapshot.time_bounds();
        let delegated = snapshot.delegated_relationship();
        if correlation_id.is_nil()
            || community_id.as_uuid().is_nil()
            || binding_id.is_nil()
            || binding_version == 0
            || policy_revision == 0
            || authority_epoch == 0
            || *source_request_fingerprint == [0; 32]
            || *target_fingerprint == [0; 32]
            || *transport_context == [0; 32]
            || issued_at >= expires_at
            || !actor.is_bound_to(community_id)
            || owner_pubkey.is_some() != delegated.is_some()
        {
            return Err(DbError::InvalidData(
                "authorization protected publication authority is invalid".to_owned(),
            ));
        }
        let (relationship_id, relationship_revision, delegation_conditions_fingerprint) =
            match delegated {
                Some((id, revision, conditions))
                    if !id.is_nil() && revision > 0 && *conditions != [0; 32] =>
                {
                    (Some(id), Some(revision), Some(*conditions))
                }
                Some(_) => {
                    return Err(DbError::InvalidData(
                        "authorization delegated publication authority is invalid".to_owned(),
                    ));
                }
                None => (None, None, None),
            };
        Ok(Self {
            community_id,
            correlation_id,
            actor,
            capability,
            actor_pubkey: actor_pubkey.to_bytes(),
            owner_pubkey: owner_pubkey.map(|key| key.to_bytes()),
            binding_id,
            binding_version,
            relationship_id,
            relationship_revision,
            delegation_conditions_fingerprint,
            source_request_fingerprint: *source_request_fingerprint,
            target_fingerprint: *target_fingerprint,
            transport,
            transport_context_fingerprint: *transport_context,
            policy_revision,
            invalidation_generation,
            authority_epoch,
            fence: snapshot.fence(),
            issued_at,
            expires_at,
        })
    }
}

impl fmt::Debug for ProtectedPublicationAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedPublicationAuthority([REDACTED])")
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn protected_publication_request_for_restore_test(
    community_id: CommunityId,
    actor_pubkey: [u8; 32],
    binding_id: Uuid,
    binding_version: u64,
    fence: AuthorizationLeaseFence,
    invalidation_generation: u64,
    authority_epoch: u64,
    operation: ProtectedPublicationOperationIdentity,
    staged_digest: [u8; 32],
    expected_parent_result_digest: Option<[u8; 32]>,
) -> Result<(ProtectedPublicationCoordinate, ProtectedPublicationRequest)> {
    let target_fingerprint = [21_u8; 32];
    let coordinate = ProtectedPublicationCoordinate {
        community_id,
        object_kind: AuthorizationProtectedObjectKind::Repository,
        object_key: protected_object_key(
            community_id,
            AuthorizationProtectedObjectKind::Repository,
            target_fingerprint,
        ),
        target_fingerprint,
    };
    let now = Utc::now();
    let authority = ProtectedPublicationAuthority {
        community_id,
        correlation_id: Uuid::new_v4(),
        actor: AuthorizationEventActor::test_direct(community_id),
        capability: RouteCapability::GitWrite,
        actor_pubkey,
        owner_pubkey: None,
        binding_id,
        binding_version,
        relationship_id: None,
        relationship_revision: None,
        delegation_conditions_fingerprint: None,
        source_request_fingerprint: [22_u8; 32],
        target_fingerprint,
        transport: ProofTransport::Nip42,
        transport_context_fingerprint: [23_u8; 32],
        policy_revision: 1,
        invalidation_generation,
        authority_epoch,
        fence,
        issued_at: now,
        expires_at: now + Duration::minutes(5),
    };
    let request_fingerprint = publication_request_fingerprint(
        &coordinate,
        operation,
        &authority,
        staged_digest,
        expected_parent_result_digest,
    );
    Ok((
        coordinate.clone(),
        ProtectedPublicationRequest {
            coordinate,
            operation,
            authority,
            staged_digest,
            expected_parent_result_digest,
            request_fingerprint,
        },
    ))
}

struct LockedPublicationState {
    current: Option<ProtectedPublicationWitness>,
    prior_issued_at: Option<DateTime<Utc>>,
}

enum PublicationTxError {
    Conflict,
    Stale,
    Audit(AuthorizationAuditFailureCode),
    Database(DbError),
}

impl From<DbError> for PublicationTxError {
    fn from(value: DbError) -> Self {
        Self::Database(value)
    }
}

impl From<sqlx::Error> for PublicationTxError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value.into())
    }
}

impl Db {
    /// Commit on the exact physical session whose restore fence was acquired
    /// before the external Pending reservation.
    pub(crate) async fn commit_staged_protected_publication_fenced(
        &self,
        fence: &mut AuthorizationOperationFence,
        request: &ProtectedPublicationRequest,
    ) -> std::result::Result<ProtectedPublicationCommit, ProtectedPublicationError> {
        let identity = request.restore_identity();
        let connection = fence
            .connection_for(
                identity.authorization_domain(),
                identity.operation_id(),
                identity.request_fingerprint(),
            )
            .map_err(ProtectedPublicationError::Database)?;
        let community_id = request.coordinate.community_id;
        let mut transaction = Transaction::begin(&mut *connection, None)
            .await
            .map_err(DbError::from)?;
        let result = commit_publication_tx(&mut transaction, request).await;
        match result {
            Ok(commit) => {
                if let Err(error) = transaction.commit().await {
                    let _ = self
                        .latch_authorization_event_failure(
                            community_id,
                            AuthorizationAuditFailureCode::StorageUnavailable,
                        )
                        .await;
                    return Err(ProtectedPublicationError::Database(error.into()));
                }
                Ok(commit)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                match error {
                    PublicationTxError::Conflict => Err(ProtectedPublicationError::Conflict),
                    PublicationTxError::Stale => Err(ProtectedPublicationError::StaleAuthorization),
                    PublicationTxError::Audit(failure) => {
                        let _ = self
                            .latch_authorization_event_failure(community_id, failure)
                            .await;
                        Err(ProtectedPublicationError::AuditUnavailable)
                    }
                    PublicationTxError::Database(error) => {
                        Err(ProtectedPublicationError::Database(error))
                    }
                }
            }
        }
    }

    /// Resolve the current canonical publication witness for one sealed coordinate.
    pub async fn protected_publication_witness(
        &self,
        coordinate: &ProtectedPublicationCoordinate,
    ) -> Result<Option<ProtectedPublicationWitness>> {
        load_witness_pool(self, coordinate).await
    }

    /// Final post-fetch recheck immediately before any refs, headers, ranges, or bytes.
    pub async fn recheck_protected_publication_witness(
        &self,
        witness: &ProtectedPublicationWitness,
    ) -> Result<bool> {
        Ok(self
            .recheck_protected_publication_validity(witness)
            .await?
            .is_some())
    }

    /// Final joined recheck with the PostgreSQL time sample needed for a
    /// conservative monotonic expiry deadline.
    pub async fn recheck_protected_publication_validity(
        &self,
        witness: &ProtectedPublicationWitness,
    ) -> Result<Option<ProtectedPublicationValidity>> {
        match load_witness_pool(self, &witness.coordinate).await? {
            Some(current) if current == *witness => Ok(Some(ProtectedPublicationValidity {
                authoritative_now: current.authoritative_now,
                expires_at: current.expires_at,
            })),
            _ => Ok(None),
        }
    }

    /// Recheck a freshly finalized route against one atomic PostgreSQL snapshot.
    ///
    /// This is the last allow fence before transport code may construct
    /// protected access. Publication commit performs the same checks again
    /// under its writer locks.
    pub async fn recheck_protected_publication_authorization(
        &self,
        context: &FinalizedAuthContext,
        coordinate: &ProtectedPublicationCoordinate,
    ) -> Result<bool> {
        let authority = ProtectedPublicationAuthority::from_context(context)?;
        if authority.community_id != coordinate.community_id
            || authority.target_fingerprint != coordinate.target_fingerprint
            || !capability_reads_object(authority.capability, coordinate.object_kind)
        {
            return Ok(false);
        }

        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *transaction)
            .await?;
        match refence_publication_lease(&mut transaction, &authority).await {
            Ok(()) => {}
            Err(PublicationTxError::Stale) => {
                transaction.rollback().await?;
                return Ok(false);
            }
            Err(PublicationTxError::Conflict | PublicationTxError::Audit(_)) => {
                transaction.rollback().await?;
                return Ok(false);
            }
            Err(PublicationTxError::Database(error)) => {
                transaction.rollback().await?;
                return Err(error);
            }
        }

        let row = sqlx::query(
            "SELECT epoch.authority_epoch AS epoch_authority_epoch, \
                    epoch.fence AS epoch_fence, \
                    protected.authority_epoch AS protected_authority_epoch, \
                    protected.fence AS protected_fence \
             FROM (VALUES (1)) AS seed(value) \
             LEFT JOIN authorization_authority_epochs epoch \
               ON epoch.community_id=$1 AND epoch.object_kind=$2 AND epoch.object_key=$3 \
             LEFT JOIN protected_object_authority protected \
               ON protected.community_id=$1 AND protected.object_kind=$2 \
              AND protected.object_key=$3",
        )
        .bind(coordinate.community_id.as_uuid())
        .bind(coordinate.object_kind as i16)
        .bind(coordinate.object_key.as_slice())
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;

        let epoch = row
            .try_get::<Option<i64>, _>("epoch_authority_epoch")?
            .map(from_database_version)
            .transpose()?;
        let epoch_fence = row
            .try_get::<Option<Vec<u8>>, _>("epoch_fence")?
            .map(|value| authorization_lease_fence(value, "authority fence"))
            .transpose()?;
        let protected = row
            .try_get::<Option<i64>, _>("protected_authority_epoch")?
            .map(from_database_version)
            .transpose()?;
        let protected_fence = row
            .try_get::<Option<Vec<u8>>, _>("protected_fence")?
            .map(|value| authorization_lease_fence(value, "protected fence"))
            .transpose()?;
        Ok(match (epoch, epoch_fence, protected, protected_fence) {
            (None, None, None, None) => authority.authority_epoch == 1,
            (Some(epoch), Some(epoch_fence), Some(protected), Some(protected_fence)) => {
                epoch == authority.authority_epoch
                    && protected == authority.authority_epoch
                    && epoch_fence == authority.fence
                    && protected_fence == authority.fence
            }
            _ => false,
        })
    }

    /// Atomically make one already-staged immutable publication authoritative.
    pub async fn commit_staged_protected_publication(
        &self,
        _request: ProtectedPublicationRequest,
    ) -> std::result::Result<ProtectedPublicationCommit, ProtectedPublicationError> {
        #[cfg(test)]
        {
            return self
                .commit_staged_protected_publication_unfenced_for_test(&_request)
                .await;
        }
        #[cfg(not(test))]
        Err(ProtectedPublicationError::Database(DbError::InvalidData(
            "protected publication requires an operation restore fence".to_owned(),
        )))
    }

    /// Borrowed publication commit/replay entrypoint for operation restore.
    ///
    /// Retaining the sealed request lets a caller reconcile an ambiguous
    /// database outcome and then invoke the exact PostgreSQL replay path to
    /// reconstruct the canonical committed witness and result digest.
    pub async fn commit_staged_protected_publication_ref(
        &self,
        _request: &ProtectedPublicationRequest,
    ) -> std::result::Result<ProtectedPublicationCommit, ProtectedPublicationError> {
        #[cfg(test)]
        {
            return self
                .commit_staged_protected_publication_unfenced_for_test(_request)
                .await;
        }
        #[cfg(not(test))]
        Err(ProtectedPublicationError::Database(DbError::InvalidData(
            "protected publication requires an operation restore fence".to_owned(),
        )))
    }

    #[cfg(test)]
    async fn commit_staged_protected_publication_unfenced_for_test(
        &self,
        request: &ProtectedPublicationRequest,
    ) -> std::result::Result<ProtectedPublicationCommit, ProtectedPublicationError> {
        let community_id = request.coordinate.community_id;
        let mut transaction = self.pool.begin().await.map_err(DbError::from)?;
        let result = commit_publication_tx(&mut transaction, request).await;
        match result {
            Ok(commit) => {
                if let Err(error) = transaction.commit().await {
                    let _ = self
                        .latch_authorization_event_failure(
                            community_id,
                            AuthorizationAuditFailureCode::StorageUnavailable,
                        )
                        .await;
                    return Err(ProtectedPublicationError::Database(error.into()));
                }
                Ok(commit)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                match error {
                    PublicationTxError::Conflict => Err(ProtectedPublicationError::Conflict),
                    PublicationTxError::Stale => Err(ProtectedPublicationError::StaleAuthorization),
                    PublicationTxError::Audit(failure) => {
                        let _ = self
                            .latch_authorization_event_failure(community_id, failure)
                            .await;
                        Err(ProtectedPublicationError::AuditUnavailable)
                    }
                    PublicationTxError::Database(error) => {
                        Err(ProtectedPublicationError::Database(error))
                    }
                }
            }
        }
    }
}

async fn commit_publication_tx(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ProtectedPublicationRequest,
) -> std::result::Result<ProtectedPublicationCommit, PublicationTxError> {
    acquire_publication_lock(transaction, &request.coordinate).await?;

    let receipt = publication_receipt(request)?;
    let event = publication_event(request)?;
    if record_authorization_operation_receipt_tx(transaction, &receipt).await?
        == AuthorizationReceiptWrite::ExactReplay
    {
        let locked = lock_publication_state(transaction, &request.coordinate).await?;
        return validate_publication_replay(transaction, request, &event, locked.current).await;
    }

    let locked = lock_publication_state(transaction, &request.coordinate).await?;
    refence_publication_lease(transaction, &request.authority).await?;
    if locked
        .current
        .as_ref()
        .map(ProtectedPublicationWitness::result_digest)
        != request.expected_parent_result_digest
    {
        return Err(PublicationTxError::Conflict);
    }

    let before_epoch = locked
        .current
        .as_ref()
        .map_or(0, ProtectedPublicationWitness::authority_epoch);
    if before_epoch == 0 && request.authority.authority_epoch != 1 {
        return Err(PublicationTxError::Stale);
    }
    if before_epoch > 0
        && (request.authority.authority_epoch != before_epoch
            || locked
                .current
                .as_ref()
                .map(ProtectedPublicationWitness::fence)
                != Some(request.authority.fence))
    {
        return Err(PublicationTxError::Stale);
    }
    let after_epoch = if before_epoch == 0 {
        1
    } else {
        before_epoch.checked_add(1).ok_or_else(|| {
            PublicationTxError::Database(DbError::InvalidData(
                "authorization protected publication epoch exhausted".to_owned(),
            ))
        })?
    };
    let next_fence = if before_epoch == 0 {
        request.authority.fence
    } else {
        next_authority_fence(
            request.coordinate.community_id,
            request.operation.operation_id,
            request.request_fingerprint,
            request.coordinate.object_kind,
            request.coordinate.object_key,
            before_epoch,
            request.authority.fence,
        )?
    };
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await?;
    if !lease_valid_at(&request.authority, now) {
        return Err(PublicationTxError::Stale);
    }
    let issued_at = locked.prior_issued_at.map_or(now, |prior| {
        std::cmp::max(now, prior + Duration::microseconds(1))
    });
    if request.authority.expires_at <= issued_at {
        return Err(PublicationTxError::Stale);
    }

    write_publication_authority(
        transaction,
        request,
        before_epoch,
        after_epoch,
        next_fence,
        issued_at,
    )
    .await?;
    let delta = AuthorizationOperationVersionDelta::new(
        AuthorizationVersionComponentKind::AuthorityEpoch,
        authorization_version_authority_epoch_component_key(
            request.coordinate.community_id,
            request.coordinate.object_kind,
            request.coordinate.object_key,
        ),
        before_epoch,
        after_epoch,
    )?;
    record_authorization_operation_version_delta_tx(
        transaction,
        request.coordinate.community_id,
        request.operation.operation_id,
        request.request_fingerprint,
        vec![delta],
    )
    .await?;
    match record_authorization_event_tx(transaction, &event).await {
        Ok(AuthorizationReceiptWrite::Inserted) => {}
        Ok(AuthorizationReceiptWrite::ExactReplay) => {
            return Err(PublicationTxError::Conflict);
        }
        Err(AuthorizationEventWriteError::CapacityUnavailable) => {
            return Err(PublicationTxError::Audit(
                AuthorizationAuditFailureCode::CapacityExhausted,
            ));
        }
        Err(AuthorizationEventWriteError::Database(_)) => {
            return Err(PublicationTxError::Audit(
                AuthorizationAuditFailureCode::StorageUnavailable,
            ));
        }
    }
    crate::authorization_invalidation::notify_protected_publication_advance_tx(
        transaction,
        &ProtectedPublicationDependency::from_coordinate(&request.coordinate),
        after_epoch,
    )
    .await?;
    let witness = ProtectedPublicationWitness {
        coordinate: request.coordinate.clone(),
        dependency: ProtectedPublicationDependency::from_coordinate(&request.coordinate),
        result_digest: request.staged_digest,
        authority_epoch: after_epoch,
        invalidation_generation: request.authority.invalidation_generation,
        fence: next_fence,
        authoritative_now: issued_at,
        expires_at: request.authority.expires_at,
        operation_id: request.operation.operation_id,
        request_fingerprint: request.request_fingerprint,
    };
    Ok(ProtectedPublicationCommit {
        disposition: ProtectedPublicationDisposition::Applied,
        witness,
    })
}

async fn validate_publication_replay(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ProtectedPublicationRequest,
    event: &NewAuthorizationEvent,
    current: Option<ProtectedPublicationWitness>,
) -> std::result::Result<ProtectedPublicationCommit, PublicationTxError> {
    match record_authorization_event_tx(transaction, event).await {
        Ok(AuthorizationReceiptWrite::ExactReplay) => {}
        Ok(AuthorizationReceiptWrite::Inserted) => return Err(PublicationTxError::Conflict),
        Err(AuthorizationEventWriteError::CapacityUnavailable) => {
            return Err(PublicationTxError::Audit(
                AuthorizationAuditFailureCode::CapacityExhausted,
            ));
        }
        Err(AuthorizationEventWriteError::Database(error)) => {
            return Err(PublicationTxError::Database(error));
        }
    }
    let manifest = load_manifest_connection(
        transaction,
        request.coordinate.community_id,
        request.operation.operation_id,
    )
    .await?
    .ok_or_else(|| {
        PublicationTxError::Database(DbError::InvalidData(
            "authorization protected publication replay lacks a manifest".to_owned(),
        ))
    })?;
    if manifest.request_fingerprint() != request.request_fingerprint
        || manifest.components().len() != 1
    {
        return Err(PublicationTxError::Conflict);
    }
    let component = &manifest.components()[0];
    let expected_key = authorization_version_authority_epoch_component_key(
        request.coordinate.community_id,
        request.coordinate.object_kind,
        request.coordinate.object_key,
    );
    if component.component_kind() != AuthorizationVersionComponentKind::AuthorityEpoch
        || component.component_key() != expected_key
    {
        return Err(PublicationTxError::Conflict);
    }
    let witness = current.ok_or(PublicationTxError::Conflict)?;
    if witness.operation_id != request.operation.operation_id
        || witness.request_fingerprint != request.request_fingerprint
        || witness.result_digest != request.staged_digest
        || witness.authority_epoch != component.after_version()
    {
        return Err(PublicationTxError::Conflict);
    }
    Ok(ProtectedPublicationCommit {
        disposition: ProtectedPublicationDisposition::ExactReplay,
        witness,
    })
}

fn publication_receipt(
    request: &ProtectedPublicationRequest,
) -> Result<AuthorizationOperationReceipt> {
    AuthorizationOperationReceipt::new(
        request.coordinate.community_id,
        request.operation.operation_id,
        request.request_fingerprint,
        AuthorizationOperationKind::ProtectedMutation,
        request.authority.actor.clone(),
        AuthorizationOperationOutcome::Applied,
        request.staged_digest,
    )
}

fn publication_event(request: &ProtectedPublicationRequest) -> Result<NewAuthorizationEvent> {
    NewAuthorizationEvent::new(
        request.coordinate.community_id,
        request.operation.event_id,
        AuthorizationEventKind::ProtectedAllowed,
        AuthorizationEventOutcome::Allowed,
        AuthorizationReasonCode::Current,
        request.authority.actor.clone(),
        Some(request.coordinate.object_key),
        request.operation.operation_id,
        Some(request.request_fingerprint),
        request.authority.correlation_id,
        request.operation.attempt_id,
    )
}

async fn acquire_publication_lock(
    transaction: &mut Transaction<'_, Postgres>,
    coordinate: &ProtectedPublicationCoordinate,
) -> Result<()> {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&coordinate.object_key[..8]);
    let mut domain_bytes = [0_u8; 8];
    domain_bytes.copy_from_slice(&coordinate.community_id.as_uuid().as_bytes()[..8]);
    let key = i64::from_be_bytes(bytes)
        ^ i64::from(coordinate.object_kind as i16)
        ^ i64::from_be_bytes(domain_bytes);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn lock_publication_state(
    transaction: &mut Transaction<'_, Postgres>,
    coordinate: &ProtectedPublicationCoordinate,
) -> Result<LockedPublicationState> {
    let epoch = sqlx::query(
        "SELECT authority_epoch,fence,operation_id,request_fingerprint \
         FROM authorization_authority_epochs \
         WHERE community_id=$1 AND object_kind=$2 AND object_key=$3 FOR UPDATE",
    )
    .bind(coordinate.community_id.as_uuid())
    .bind(coordinate.object_kind as i16)
    .bind(coordinate.object_key.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    let protected = sqlx::query(
        "SELECT protected.authority_epoch,protected.fence,protected.operation_id, \
                protected.request_fingerprint,protected.issued_at,protected.expires_at, \
                protected.invalidation_generation,clock_timestamp() AS database_now, \
                receipt.operation_kind,receipt.outcome_code,receipt.actor_fingerprint, \
                receipt.result_digest \
         FROM protected_object_authority protected \
         JOIN authorization_operation_receipts receipt \
           ON receipt.community_id=protected.community_id \
          AND receipt.operation_id=protected.operation_id \
          AND receipt.request_fingerprint=protected.request_fingerprint \
         WHERE protected.community_id=$1 AND protected.object_kind=$2 \
           AND protected.object_key=$3 FOR UPDATE OF protected",
    )
    .bind(coordinate.community_id.as_uuid())
    .bind(coordinate.object_kind as i16)
    .bind(coordinate.object_key.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    match (epoch, protected) {
        (None, None) => Ok(LockedPublicationState {
            current: None,
            prior_issued_at: None,
        }),
        (Some(epoch), Some(protected)) => {
            let witness = witness_from_rows(coordinate, &epoch, &protected)?;
            Ok(LockedPublicationState {
                current: Some(witness),
                prior_issued_at: Some(protected.try_get("issued_at")?),
            })
        }
        _ => Err(DbError::InvalidData(
            "authorization protected publication authority is partial".to_owned(),
        )),
    }
}

async fn refence_publication_lease(
    transaction: &mut Transaction<'_, Postgres>,
    authority: &ProtectedPublicationAuthority,
) -> std::result::Result<(), PublicationTxError> {
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await?;
    if !lease_valid_at(authority, now) {
        return Err(PublicationTxError::Stale);
    }
    let expected_author = authority.owner_pubkey.unwrap_or(authority.actor_pubkey);
    let binding = sqlx::query(
        "SELECT event_author_pubkey FROM identity_bindings \
         WHERE community_id=$1 AND binding_id=$2 AND binding_version=$3 \
           AND binding_state=1 AND (expires_at IS NULL OR expires_at > $4) FOR SHARE",
    )
    .bind(authority.community_id.as_uuid())
    .bind(authority.binding_id)
    .bind(to_database_version(authority.binding_version)?)
    .bind(now)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(binding) = binding else {
        return Err(PublicationTxError::Stale);
    };
    if bytes32(binding.try_get("event_author_pubkey")?, "binding author")? != expected_author {
        return Err(PublicationTxError::Stale);
    }
    let policy_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM identity_enrollment_policies \
         WHERE community_id=$1 AND policy_revision=$2 AND effective_at <= $3 \
           AND (expires_at IS NULL OR expires_at > $3))",
    )
    .bind(authority.community_id.as_uuid())
    .bind(to_database_version(authority.policy_revision)?)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await?;
    if !policy_exists {
        return Err(PublicationTxError::Stale);
    }
    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT current_generation FROM authorization_invalidation_domains \
         WHERE community_id=$1 FOR SHARE",
    )
    .bind(authority.community_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await?;
    if generation.map(from_database_version).transpose()? != Some(authority.invalidation_generation)
    {
        return Err(PublicationTxError::Stale);
    }
    let binding_selector = AuthorizationInvalidationSelector::binding(
        authority.binding_id,
        authority.binding_version,
    )?;
    let binding_floor: Option<i64> = sqlx::query_scalar(
        "SELECT binding_version_floor FROM authorization_invalidation_floors \
         WHERE community_id=$1 AND selector_kind=3 AND selector_fingerprint=$2 FOR SHARE",
    )
    .bind(authority.community_id.as_uuid())
    .bind(
        binding_selector
            .fingerprint(authority.community_id)
            .as_slice(),
    )
    .fetch_optional(&mut **transaction)
    .await?
    .flatten();
    if binding_floor
        .map(from_database_version)
        .transpose()?
        .is_some_and(|floor| floor >= authority.binding_version)
    {
        return Err(PublicationTxError::Stale);
    }
    match (
        authority.relationship_id,
        authority.relationship_revision,
        authority.delegation_conditions_fingerprint,
    ) {
        (Some(relationship_id), Some(revision), Some(_)) => {
            let selector = AuthorizationInvalidationSelector::delegated_relationship(
                relationship_id,
                revision,
            )?;
            let floor: Option<i64> = sqlx::query_scalar(
                "SELECT relationship_revision_floor FROM authorization_invalidation_floors \
                 WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2 FOR SHARE",
            )
            .bind(authority.community_id.as_uuid())
            .bind(selector.fingerprint(authority.community_id).as_slice())
            .fetch_optional(&mut **transaction)
            .await?
            .flatten();
            if floor
                .map(from_database_version)
                .transpose()?
                .is_some_and(|floor| floor >= revision)
            {
                return Err(PublicationTxError::Stale);
            }
        }
        (None, None, None) => {}
        _ => {
            return Err(PublicationTxError::Database(DbError::InvalidData(
                "authorization delegated publication authority is partial".to_owned(),
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_publication_authority(
    transaction: &mut Transaction<'_, Postgres>,
    request: &ProtectedPublicationRequest,
    before_epoch: u64,
    after_epoch: u64,
    fence: AuthorizationLeaseFence,
    issued_at: DateTime<Utc>,
) -> Result<()> {
    if before_epoch == 0 {
        sqlx::query(
            "INSERT INTO authorization_authority_epochs \
             (community_id,object_kind,object_key,authority_epoch,fence,operation_id, \
              request_fingerprint) VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(request.coordinate.community_id.as_uuid())
        .bind(request.coordinate.object_kind as i16)
        .bind(request.coordinate.object_key.as_slice())
        .bind(to_database_version(after_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .bind(request.operation.operation_id)
        .bind(request.request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
    } else {
        let updated = sqlx::query(
            "UPDATE authorization_authority_epochs SET authority_epoch=$6,fence=$7, \
               operation_id=$8,request_fingerprint=$9, \
               updated_at=GREATEST(clock_timestamp(),updated_at + INTERVAL '1 microsecond') \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3 \
               AND authority_epoch=$4 AND fence=$5",
        )
        .bind(request.coordinate.community_id.as_uuid())
        .bind(request.coordinate.object_kind as i16)
        .bind(request.coordinate.object_key.as_slice())
        .bind(to_database_version(before_epoch)?)
        .bind(request.authority.fence.as_bytes().as_slice())
        .bind(to_database_version(after_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .bind(request.operation.operation_id)
        .bind(request.request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization protected publication epoch changed".to_owned(),
            ));
        }
    }

    let capability = capability_code(request.authority.capability);
    let relationship_revision = request
        .authority
        .relationship_revision
        .map(to_database_version)
        .transpose()?;
    if before_epoch == 0 {
        sqlx::query(
            "INSERT INTO protected_object_authority \
             (community_id,object_kind,object_key,capability,actor_pubkey,owner_pubkey, \
              binding_id,binding_version,delegated_relationship_id, \
              delegated_relationship_revision,delegation_conditions_fingerprint, \
              policy_revision,invalidation_generation,authority_epoch,fence,issued_at,expires_at, \
              operation_id,request_fingerprint) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)",
        )
        .bind(request.coordinate.community_id.as_uuid())
        .bind(request.coordinate.object_kind as i16)
        .bind(request.coordinate.object_key.as_slice())
        .bind(capability)
        .bind(request.authority.actor_pubkey.as_slice())
        .bind(request.authority.owner_pubkey.map(|value| value.to_vec()))
        .bind(request.authority.binding_id)
        .bind(to_database_version(request.authority.binding_version)?)
        .bind(request.authority.relationship_id)
        .bind(relationship_revision)
        .bind(
            request
                .authority
                .delegation_conditions_fingerprint
                .map(|value| value.to_vec()),
        )
        .bind(to_database_version(request.authority.policy_revision)?)
        .bind(to_database_version(
            request.authority.invalidation_generation,
        )?)
        .bind(to_database_version(after_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .bind(issued_at)
        .bind(request.authority.expires_at)
        .bind(request.operation.operation_id)
        .bind(request.request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
    } else {
        let updated = sqlx::query(
            "UPDATE protected_object_authority SET capability=$6,actor_pubkey=$7,owner_pubkey=$8, \
               binding_id=$9,binding_version=$10,delegated_relationship_id=$11, \
               delegated_relationship_revision=$12,delegation_conditions_fingerprint=$13, \
               policy_revision=$14,invalidation_generation=$15,authority_epoch=$16,fence=$17, \
               issued_at=$18,expires_at=$19,operation_id=$20,request_fingerprint=$21 \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3 \
               AND authority_epoch=$4 AND fence=$5",
        )
        .bind(request.coordinate.community_id.as_uuid())
        .bind(request.coordinate.object_kind as i16)
        .bind(request.coordinate.object_key.as_slice())
        .bind(to_database_version(before_epoch)?)
        .bind(request.authority.fence.as_bytes().as_slice())
        .bind(capability)
        .bind(request.authority.actor_pubkey.as_slice())
        .bind(request.authority.owner_pubkey.map(|value| value.to_vec()))
        .bind(request.authority.binding_id)
        .bind(to_database_version(request.authority.binding_version)?)
        .bind(request.authority.relationship_id)
        .bind(relationship_revision)
        .bind(
            request
                .authority
                .delegation_conditions_fingerprint
                .map(|value| value.to_vec()),
        )
        .bind(to_database_version(request.authority.policy_revision)?)
        .bind(to_database_version(
            request.authority.invalidation_generation,
        )?)
        .bind(to_database_version(after_epoch)?)
        .bind(fence.as_bytes().as_slice())
        .bind(issued_at)
        .bind(request.authority.expires_at)
        .bind(request.operation.operation_id)
        .bind(request.request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization protected publication authority changed".to_owned(),
            ));
        }
    }
    Ok(())
}

async fn load_witness_pool(
    db: &Db,
    coordinate: &ProtectedPublicationCoordinate,
) -> Result<Option<ProtectedPublicationWitness>> {
    let rows = sqlx::query(
        "WITH observed AS MATERIALIZED (SELECT clock_timestamp() AS database_now) \
         SELECT epoch.authority_epoch AS epoch_authority_epoch,epoch.fence AS epoch_fence, \
                epoch.operation_id AS epoch_operation_id, \
                epoch.request_fingerprint AS epoch_request_fingerprint, \
                protected.authority_epoch,protected.fence,protected.operation_id, \
                protected.request_fingerprint,protected.issued_at,receipt.operation_kind, \
                receipt.outcome_code,receipt.actor_fingerprint,receipt.result_digest, \
                protected.invalidation_generation,protected.expires_at, \
                observed.database_now, \
                (SELECT invalidation.current_generation \
                   FROM authorization_invalidation_domains invalidation \
                  WHERE invalidation.community_id=protected.community_id) \
                    AS current_invalidation_generation, \
                EXISTS(SELECT 1 FROM identity_bindings binding \
                  WHERE binding.community_id=protected.community_id \
                    AND binding.binding_id=protected.binding_id \
                    AND binding.binding_version=protected.binding_version \
                    AND binding.binding_state=1 \
                    AND (binding.expires_at IS NULL \
                         OR binding.expires_at > observed.database_now) \
                    AND binding.event_author_pubkey= \
                        COALESCE(protected.owner_pubkey,protected.actor_pubkey)) AS binding_current, \
                EXISTS(SELECT 1 FROM identity_enrollment_policies policy \
                  WHERE policy.community_id=protected.community_id \
                    AND policy.policy_revision=protected.policy_revision \
                    AND policy.effective_at <= observed.database_now \
                    AND (policy.expires_at IS NULL \
                         OR policy.expires_at > observed.database_now)) \
                    AS policy_current \
         FROM authorization_authority_epochs epoch \
         FULL JOIN protected_object_authority protected \
           ON protected.community_id=epoch.community_id AND protected.object_kind=epoch.object_kind \
          AND protected.object_key=epoch.object_key \
         LEFT JOIN authorization_operation_receipts receipt \
           ON receipt.community_id=protected.community_id \
          AND receipt.operation_id=protected.operation_id \
          AND receipt.request_fingerprint=protected.request_fingerprint \
         CROSS JOIN observed \
         WHERE COALESCE(epoch.community_id,protected.community_id)=$1 \
           AND COALESCE(epoch.object_kind,protected.object_kind)=$2 \
           AND COALESCE(epoch.object_key,protected.object_key)=$3",
    )
    .bind(coordinate.community_id.as_uuid())
    .bind(coordinate.object_kind as i16)
    .bind(coordinate.object_key.as_slice())
    .fetch_all(&db.pool)
    .await?;
    match rows.len() {
        0 => Ok(None),
        1 => witness_from_joined_row(coordinate, &rows[0]),
        _ => Err(DbError::InvalidData(
            "authorization protected publication witness is not unique".to_owned(),
        )),
    }
}

fn witness_from_rows(
    coordinate: &ProtectedPublicationCoordinate,
    epoch: &sqlx::postgres::PgRow,
    protected: &sqlx::postgres::PgRow,
) -> Result<ProtectedPublicationWitness> {
    let authority_epoch = from_database_version(epoch.try_get("authority_epoch")?)?;
    let fence = authorization_lease_fence(epoch.try_get("fence")?, "authority fence")?;
    let operation_id: Uuid = epoch.try_get("operation_id")?;
    let request_fingerprint = bytes32(
        epoch.try_get("request_fingerprint")?,
        "authority request fingerprint",
    )?;
    if from_database_version(protected.try_get("authority_epoch")?)? != authority_epoch
        || authorization_lease_fence(protected.try_get("fence")?, "protected fence")? != fence
        || protected.try_get::<Uuid, _>("operation_id")? != operation_id
        || bytes32(
            protected.try_get("request_fingerprint")?,
            "protected request fingerprint",
        )? != request_fingerprint
        || protected.try_get::<i16, _>("operation_kind")?
            != AuthorizationOperationKind::ProtectedMutation as i16
        || protected.try_get::<i16, _>("outcome_code")?
            != AuthorizationOperationOutcome::Applied as i16
        || bytes32(protected.try_get("actor_fingerprint")?, "receipt actor")? == [0; 32]
    {
        return Err(DbError::InvalidData(
            "authorization protected publication witness does not match".to_owned(),
        ));
    }
    let result_digest = bytes32(protected.try_get("result_digest")?, "publication digest")?;
    if result_digest == [0; 32] {
        return Err(DbError::InvalidData(
            "authorization protected publication digest is invalid".to_owned(),
        ));
    }
    let invalidation_generation =
        from_database_version(protected.try_get("invalidation_generation")?)?;
    let authoritative_now: DateTime<Utc> = protected.try_get("database_now")?;
    let expires_at: DateTime<Utc> = protected.try_get("expires_at")?;
    Ok(ProtectedPublicationWitness {
        coordinate: coordinate.clone(),
        dependency: ProtectedPublicationDependency::from_coordinate(coordinate),
        result_digest,
        authority_epoch,
        invalidation_generation,
        fence,
        authoritative_now,
        expires_at,
        operation_id,
        request_fingerprint,
    })
}

fn witness_from_joined_row(
    coordinate: &ProtectedPublicationCoordinate,
    row: &sqlx::postgres::PgRow,
) -> Result<Option<ProtectedPublicationWitness>> {
    let authority_epoch: Option<i64> = row.try_get("epoch_authority_epoch")?;
    let protected_epoch: Option<i64> = row.try_get("authority_epoch")?;
    let (Some(authority_epoch), Some(protected_epoch)) = (authority_epoch, protected_epoch) else {
        return Err(DbError::InvalidData(
            "authorization protected publication witness is partial".to_owned(),
        ));
    };
    let epoch_fence: Option<Vec<u8>> = row.try_get("epoch_fence")?;
    let protected_fence: Option<Vec<u8>> = row.try_get("fence")?;
    let epoch_operation: Option<Uuid> = row.try_get("epoch_operation_id")?;
    let protected_operation: Option<Uuid> = row.try_get("operation_id")?;
    let epoch_request: Option<Vec<u8>> = row.try_get("epoch_request_fingerprint")?;
    let protected_request: Option<Vec<u8>> = row.try_get("request_fingerprint")?;
    let operation_kind: Option<i16> = row.try_get("operation_kind")?;
    let outcome_code: Option<i16> = row.try_get("outcome_code")?;
    let actor_fingerprint: Option<Vec<u8>> = row.try_get("actor_fingerprint")?;
    let result_digest: Option<Vec<u8>> = row.try_get("result_digest")?;
    let protected_generation: Option<i64> = row.try_get("invalidation_generation")?;
    let current_generation: Option<i64> = row.try_get("current_invalidation_generation")?;
    let expires_at: Option<DateTime<Utc>> = row.try_get("expires_at")?;
    let authoritative_now: DateTime<Utc> = row.try_get("database_now")?;
    let binding_current: bool = row.try_get("binding_current")?;
    let policy_current: bool = row.try_get("policy_current")?;
    let authority_epoch = from_database_version(authority_epoch)?;
    if from_database_version(protected_epoch)? != authority_epoch
        || epoch_fence.is_none()
        || epoch_fence != protected_fence
        || epoch_operation.is_none()
        || epoch_operation != protected_operation
        || epoch_request.is_none()
        || epoch_request != protected_request
        || operation_kind != Some(AuthorizationOperationKind::ProtectedMutation as i16)
        || outcome_code != Some(AuthorizationOperationOutcome::Applied as i16)
    {
        return Err(DbError::InvalidData(
            "authorization protected publication witness does not match".to_owned(),
        ));
    }
    let fence = authorization_lease_fence(
        epoch_fence.ok_or_else(|| {
            DbError::InvalidData("authorization publication fence is absent".to_owned())
        })?,
        "publication fence",
    )?;
    let operation_id = epoch_operation.ok_or_else(|| {
        DbError::InvalidData("authorization publication operation is absent".to_owned())
    })?;
    let request_fingerprint = bytes32(
        epoch_request.ok_or_else(|| {
            DbError::InvalidData("authorization publication request is absent".to_owned())
        })?,
        "publication request fingerprint",
    )?;
    if bytes32(
        actor_fingerprint.ok_or_else(|| {
            DbError::InvalidData("authorization publication actor is absent".to_owned())
        })?,
        "publication actor fingerprint",
    )? == [0; 32]
    {
        return Err(DbError::InvalidData(
            "authorization publication actor is invalid".to_owned(),
        ));
    }
    let result_digest = bytes32(
        result_digest.ok_or_else(|| {
            DbError::InvalidData("authorization publication digest is absent".to_owned())
        })?,
        "publication result digest",
    )?;
    if result_digest == [0; 32] {
        return Err(DbError::InvalidData(
            "authorization publication digest is invalid".to_owned(),
        ));
    }
    let invalidation_generation = protected_generation
        .map(from_database_version)
        .transpose()?
        .ok_or_else(|| {
            DbError::InvalidData(
                "authorization publication invalidation generation is absent".to_owned(),
            )
        })?;
    let current_generation = current_generation.map(from_database_version).transpose()?;
    let expires_at = expires_at.ok_or_else(|| {
        DbError::InvalidData("authorization publication expiry is absent".to_owned())
    })?;
    if current_generation != Some(invalidation_generation)
        || !binding_current
        || !policy_current
        || authoritative_now >= expires_at
    {
        return Ok(None);
    }
    Ok(Some(ProtectedPublicationWitness {
        coordinate: coordinate.clone(),
        dependency: ProtectedPublicationDependency::from_coordinate(coordinate),
        result_digest,
        authority_epoch,
        invalidation_generation,
        fence,
        authoritative_now,
        expires_at,
        operation_id,
        request_fingerprint,
    }))
}

fn lease_valid_at(authority: &ProtectedPublicationAuthority, now: DateTime<Utc>) -> bool {
    now >= authority.issued_at && now < authority.expires_at
}

pub(crate) fn protected_object_key(
    community_id: CommunityId,
    object_kind: AuthorizationProtectedObjectKind,
    target_fingerprint: [u8; 32],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:protected-publication-object:v1");
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, &(object_kind as i16).to_be_bytes());
    framed(&mut digest, &target_fingerprint);
    digest.finalize().into()
}

fn publication_request_fingerprint(
    coordinate: &ProtectedPublicationCoordinate,
    operation: ProtectedPublicationOperationIdentity,
    authority: &ProtectedPublicationAuthority,
    staged_digest: [u8; 32],
    expected_parent_result_digest: Option<[u8; 32]>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:protected-publication-request:v1");
    framed(&mut digest, coordinate.community_id.as_uuid().as_bytes());
    framed(&mut digest, &(coordinate.object_kind as i16).to_be_bytes());
    framed(&mut digest, &coordinate.object_key);
    framed(&mut digest, operation.operation_id.as_bytes());
    framed(&mut digest, operation.event_id.as_bytes());
    framed(&mut digest, operation.attempt_id.as_bytes());
    framed(&mut digest, authority.correlation_id.as_bytes());
    framed(&mut digest, &authority.source_request_fingerprint);
    framed(&mut digest, &staged_digest);
    framed(
        &mut digest,
        &capability_code(authority.capability).to_be_bytes(),
    );
    framed(
        &mut digest,
        &(proof_transport_code(authority.transport)).to_be_bytes(),
    );
    framed(&mut digest, &authority.transport_context_fingerprint);
    framed(&mut digest, &authority.actor_pubkey);
    match authority.owner_pubkey {
        Some(owner) => {
            framed(&mut digest, &[1]);
            framed(&mut digest, &owner);
        }
        None => framed(&mut digest, &[0]),
    }
    framed(&mut digest, authority.binding_id.as_bytes());
    framed(&mut digest, &authority.binding_version.to_be_bytes());
    match (
        authority.relationship_id,
        authority.relationship_revision,
        authority.delegation_conditions_fingerprint,
    ) {
        (Some(relationship_id), Some(revision), Some(conditions)) => {
            framed(&mut digest, &[1]);
            framed(&mut digest, relationship_id.as_bytes());
            framed(&mut digest, &revision.to_be_bytes());
            framed(&mut digest, &conditions);
        }
        _ => framed(&mut digest, &[0]),
    }
    framed(&mut digest, &authority.policy_revision.to_be_bytes());
    framed(
        &mut digest,
        &authority.invalidation_generation.to_be_bytes(),
    );
    framed(&mut digest, &authority.authority_epoch.to_be_bytes());
    framed(&mut digest, authority.fence.as_bytes());
    framed(&mut digest, &authority.issued_at.timestamp().to_be_bytes());
    framed(
        &mut digest,
        &authority.issued_at.timestamp_subsec_nanos().to_be_bytes(),
    );
    framed(&mut digest, &authority.expires_at.timestamp().to_be_bytes());
    framed(
        &mut digest,
        &authority.expires_at.timestamp_subsec_nanos().to_be_bytes(),
    );
    match expected_parent_result_digest {
        Some(parent_digest) => {
            framed(&mut digest, &[1]);
            framed(&mut digest, &parent_digest);
        }
        None => framed(&mut digest, &[0]),
    }
    digest.finalize().into()
}

fn capability_reads_object(
    capability: RouteCapability,
    object_kind: AuthorizationProtectedObjectKind,
) -> bool {
    object_kind.admits_read(capability)
}

fn capability_mutates_object(
    capability: RouteCapability,
    object_kind: AuthorizationProtectedObjectKind,
) -> bool {
    object_kind.admits_mutation(capability)
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

fn proof_transport_code(transport: ProofTransport) -> i16 {
    match transport {
        ProofTransport::Nip42 => 1,
        ProofTransport::Nip98 => 2,
        ProofTransport::GitSmartHttpSession => 3,
        ProofTransport::Blossom => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_auth::AuthorizationEventCapacityPolicy;
    use sqlx::PgPool;

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(1))
    }

    fn coordinate(kind: AuthorizationProtectedObjectKind) -> ProtectedPublicationCoordinate {
        let target_fingerprint = [7_u8; 32];
        ProtectedPublicationCoordinate {
            community_id: domain(),
            object_kind: kind,
            object_key: protected_object_key(domain(), kind, target_fingerprint),
            target_fingerprint,
        }
    }

    fn authority(capability: RouteCapability) -> ProtectedPublicationAuthority {
        ProtectedPublicationAuthority {
            community_id: domain(),
            correlation_id: Uuid::from_u128(2),
            actor: AuthorizationEventActor::test_direct(domain()),
            capability,
            actor_pubkey: [3; 32],
            owner_pubkey: None,
            binding_id: Uuid::from_u128(4),
            binding_version: 5,
            relationship_id: None,
            relationship_revision: None,
            delegation_conditions_fingerprint: None,
            source_request_fingerprint: [6; 32],
            target_fingerprint: [7; 32],
            transport: ProofTransport::Nip42,
            transport_context_fingerprint: [8; 32],
            policy_revision: 9,
            invalidation_generation: 10,
            authority_epoch: 11,
            fence: AuthorizationLeaseFence::from_bytes([12; 32]).expect("valid fence"),
            issued_at: DateTime::<Utc>::from_timestamp(1_000, 0).expect("valid time"),
            expires_at: DateTime::<Utc>::from_timestamp(2_000, 0).expect("valid time"),
        }
    }

    #[test]
    fn protected_coordinates_are_kind_separated_and_redacted() {
        let repository = coordinate(AuthorizationProtectedObjectKind::Repository);
        let media = coordinate(AuthorizationProtectedObjectKind::Media);
        assert_ne!(repository.object_key, media.object_key);
        assert!(!format!("{repository:?}").contains(&hex::encode(repository.object_key)));
    }

    #[test]
    fn publication_fingerprint_binds_parent_and_operation() {
        let coordinate = coordinate(AuthorizationProtectedObjectKind::Repository);
        let authority = authority(RouteCapability::GitWrite);
        let operation = ProtectedPublicationOperationIdentity::new(
            Uuid::from_u128(20),
            Uuid::from_u128(21),
            Uuid::from_u128(22),
        )
        .expect("valid identity");
        let absent =
            publication_request_fingerprint(&coordinate, operation, &authority, [23; 32], None);
        let parent = ProtectedPublicationWitness {
            coordinate: coordinate.clone(),
            dependency: ProtectedPublicationDependency::from_coordinate(&coordinate),
            result_digest: [24; 32],
            authority_epoch: 11,
            invalidation_generation: authority.invalidation_generation,
            fence: authority.fence,
            authoritative_now: authority.issued_at,
            expires_at: authority.expires_at,
            operation_id: Uuid::from_u128(25),
            request_fingerprint: [26; 32],
        };
        assert_ne!(
            absent,
            publication_request_fingerprint(
                &coordinate,
                operation,
                &authority,
                [23; 32],
                Some(parent.result_digest),
            )
        );
        let changed_event = ProtectedPublicationOperationIdentity::new(
            operation.operation_id,
            Uuid::from_u128(99),
            operation.attempt_id,
        )
        .expect("valid changed event identity");
        assert_ne!(
            absent,
            publication_request_fingerprint(&coordinate, changed_event, &authority, [23; 32], None,)
        );
    }

    #[test]
    fn publication_restore_identity_is_sealed_exact_and_redacted() {
        let coordinate = coordinate(AuthorizationProtectedObjectKind::Repository);
        let authority = authority(RouteCapability::GitWrite);
        let operation = ProtectedPublicationOperationIdentity::new(
            Uuid::from_u128(20),
            Uuid::from_u128(21),
            Uuid::from_u128(22),
        )
        .expect("valid identity");
        let request_fingerprint =
            publication_request_fingerprint(&coordinate, operation, &authority, [23; 32], None);
        let request = ProtectedPublicationRequest {
            coordinate,
            operation,
            authority,
            staged_digest: [23; 32],
            expected_parent_result_digest: None,
            request_fingerprint,
        };
        let identity = request.restore_identity();
        assert_eq!(identity.authorization_domain(), domain());
        assert_eq!(identity.operation_id(), operation.operation_id());
        assert_eq!(identity.request_fingerprint(), request_fingerprint);
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains(&operation.operation_id().to_string()));
        assert!(!rendered.contains(&hex::encode(request_fingerprint)));
    }

    #[test]
    fn write_capabilities_are_closed_by_object_kind() {
        assert!(capability_mutates_object(
            RouteCapability::GitWrite,
            AuthorizationProtectedObjectKind::Repository
        ));
        assert!(!capability_mutates_object(
            RouteCapability::GitRead,
            AuthorizationProtectedObjectKind::Repository
        ));
        assert!(!capability_mutates_object(
            RouteCapability::MediaWrite,
            AuthorizationProtectedObjectKind::Repository
        ));
        assert!(capability_mutates_object(
            RouteCapability::MediaWrite,
            AuthorizationProtectedObjectKind::Media
        ));
        assert_eq!(proof_transport_code(ProofTransport::Nip42), 1);
        assert_eq!(proof_transport_code(ProofTransport::Nip98), 2);
        assert_eq!(proof_transport_code(ProofTransport::GitSmartHttpSession), 3);
        assert_eq!(proof_transport_code(ProofTransport::Blossom), 4);
    }

    fn request(
        coordinate: ProtectedPublicationCoordinate,
        operation: ProtectedPublicationOperationIdentity,
        authority: ProtectedPublicationAuthority,
        staged_digest: [u8; 32],
        expected_parent_result_digest: Option<[u8; 32]>,
    ) -> ProtectedPublicationRequest {
        let request_fingerprint = publication_request_fingerprint(
            &coordinate,
            operation,
            &authority,
            staged_digest,
            expected_parent_result_digest,
        );
        ProtectedPublicationRequest {
            coordinate,
            operation,
            authority,
            staged_digest,
            expected_parent_result_digest,
            request_fingerprint,
        }
    }

    async fn overwrite_publication_expiry_for_test(
        pool: &PgPool,
        community_id: Uuid,
        object_kind: AuthorizationProtectedObjectKind,
        object_key: &[u8; 32],
        offset_microseconds: i64,
    ) {
        let mut connection = pool.acquire().await.expect("expiry test connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *connection)
            .await
            .expect("disable authority immutability trigger");
        sqlx::query(
            "UPDATE protected_object_authority \
             SET expires_at=clock_timestamp() + ($4 * INTERVAL '1 microsecond') \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3",
        )
        .bind(community_id)
        .bind(object_kind as i16)
        .bind(object_key.as_slice())
        .bind(offset_microseconds)
        .execute(&mut *connection)
        .await
        .expect("overwrite publication expiry for boundary test");
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *connection)
            .await
            .expect("restore authority immutability trigger");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn protected_publication_is_atomic_replayable_and_finally_rechecked() {
        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| crate::authorization_version::loopback_test_database_url());
        let admin = PgPool::connect(&admin_url).await.expect("connect admin");
        let name = format!("s5_protected_publication_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let scratch_url = format!("{}/{}", &admin_url[..split], name);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&scratch_url)
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");

        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community_uuid)
            .bind(format!("publication-{}.example", community_uuid.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,1,$2,clock_timestamp() - INTERVAL '1 second')",
        )
        .bind(community_uuid)
        .bind(vec![1_u8; 32])
        .execute(&pool)
        .await
        .expect("insert policy");
        let binding_id = Uuid::new_v4();
        let actor_pubkey = [2_u8; 32];
        let mut seed = pool.acquire().await.expect("seed connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *seed)
            .await
            .expect("disable seed triggers");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,'issuer','subject',$3,$4,1,1,1,1,$5,$6,$7,$8) \
             RETURNING binding_version",
        )
        .bind(community_uuid)
        .bind(binding_id)
        .bind(vec![3_u8; 32])
        .bind(actor_pubkey.as_slice())
        .bind(vec![4_u8; 32])
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![5_u8; 32])
        .fetch_one(&mut *seed)
        .await
        .expect("insert binding");
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *seed)
            .await
            .expect("restore seed triggers");
        drop(seed);
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(community_uuid)
        .execute(&pool)
        .await
        .expect("activate invalidation");
        let db = Db::from_pool(pool.clone());
        db.install_authorization_event_capacity(
            community,
            AuthorizationEventCapacityPolicy::new(100, 1 << 20, 16 << 10).expect("valid capacity"),
        )
        .await
        .expect("install event capacity");
        let mut live_notices = db
            .subscribe_authorization_invalidations()
            .await
            .expect("install ready independent listener");

        let target_fingerprint = [6_u8; 32];
        let repository = ProtectedPublicationCoordinate {
            community_id: community,
            object_kind: AuthorizationProtectedObjectKind::Repository,
            object_key: protected_object_key(
                community,
                AuthorizationProtectedObjectKind::Repository,
                target_fingerprint,
            ),
            target_fingerprint,
        };
        let initial_fence = AuthorizationLeaseFence::from_bytes([7_u8; 32]).expect("valid fence");
        let issued_at = Utc::now() - Duration::seconds(1);
        let expires_at = Utc::now() + Duration::minutes(10);
        let initial_authority = ProtectedPublicationAuthority {
            community_id: community,
            correlation_id: Uuid::new_v4(),
            actor: AuthorizationEventActor::test_direct(community),
            capability: RouteCapability::GitWrite,
            actor_pubkey,
            owner_pubkey: None,
            binding_id,
            binding_version: u64::try_from(binding_version).expect("positive binding version"),
            relationship_id: None,
            relationship_revision: None,
            delegation_conditions_fingerprint: None,
            source_request_fingerprint: [8_u8; 32],
            target_fingerprint,
            transport: ProofTransport::Nip42,
            transport_context_fingerprint: [9_u8; 32],
            policy_revision: 1,
            invalidation_generation: 0,
            authority_epoch: 1,
            fence: initial_fence,
            issued_at,
            expires_at,
        };
        let first_operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("valid first operation");
        let first_digest = [10_u8; 32];
        let first = db
            .commit_staged_protected_publication(request(
                repository.clone(),
                first_operation,
                initial_authority.clone(),
                first_digest,
                None,
            ))
            .await
            .expect("commit first publication");
        assert_eq!(
            first.disposition(),
            ProtectedPublicationDisposition::Applied
        );
        assert_eq!(first.result_digest(), first_digest);
        assert_eq!(first.witness().authority_epoch(), 1);
        assert_eq!(first.witness().fence(), initial_fence);
        assert!(matches!(
            live_notices
                .recv()
                .await
                .expect("receive first publication advance"),
            crate::authorization_invalidation::AuthorizationInvalidationNotice::ProtectedPublicationAdvanced {
                authority_epoch: 1,
                ..
            }
        ));
        assert!(db
            .recheck_protected_publication_witness(first.witness())
            .await
            .expect("recheck first witness"));
        let first_manifest = db
            .authorization_operation_version_delta(
                community,
                first_operation.operation_id(),
                first.witness().request_fingerprint,
            )
            .await
            .expect("load first manifest");
        assert_eq!(first_manifest.components().len(), 1);
        assert_eq!(first_manifest.components()[0].before_version(), 0);
        assert_eq!(first_manifest.components()[0].after_version(), 1);

        let replay = db
            .commit_staged_protected_publication(request(
                repository.clone(),
                first_operation,
                initial_authority.clone(),
                first_digest,
                None,
            ))
            .await
            .expect("exactly replay first publication");
        assert_eq!(
            replay.disposition(),
            ProtectedPublicationDisposition::ExactReplay
        );
        assert_eq!(replay.result_digest(), first_digest);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), live_notices.recv())
                .await
                .is_err(),
            "exact publication replay emits no live notice"
        );
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_events WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(first_operation.operation_id())
        .fetch_one(&pool)
        .await
        .expect("count replay events");
        assert_eq!(event_count, 1);

        let conflict_operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("valid conflict operation");
        let mut next_authority = initial_authority.clone();
        next_authority.correlation_id = Uuid::new_v4();
        next_authority.authority_epoch = first.witness().authority_epoch();
        next_authority.fence = first.witness().fence();
        next_authority.source_request_fingerprint = [11_u8; 32];
        assert!(matches!(
            db.commit_staged_protected_publication(request(
                repository.clone(),
                conflict_operation,
                next_authority.clone(),
                [12_u8; 32],
                Some([99_u8; 32]),
            ))
            .await,
            Err(ProtectedPublicationError::Conflict)
        ));
        let conflict_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(conflict_operation.operation_id())
        .fetch_one(&pool)
        .await
        .expect("count conflict receipts");
        assert_eq!(conflict_receipts, 0, "parent conflict must mutate nothing");

        let second_operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("valid second operation");
        let second_digest = [13_u8; 32];
        let second = db
            .commit_staged_protected_publication(request(
                repository.clone(),
                second_operation,
                next_authority,
                second_digest,
                Some(first_digest),
            ))
            .await
            .expect("commit replacement publication");
        assert_eq!(second.result_digest(), second_digest);
        assert_eq!(second.witness().authority_epoch(), 2);
        assert!(matches!(
            live_notices
                .recv()
                .await
                .expect("receive replacement publication advance"),
            crate::authorization_invalidation::AuthorizationInvalidationNotice::ProtectedPublicationAdvanced {
                authority_epoch: 2,
                ..
            }
        ));
        assert!(!db
            .recheck_protected_publication_witness(first.witness())
            .await
            .expect("old witness is stale"));
        assert!(db
            .recheck_protected_publication_witness(second.witness())
            .await
            .expect("new witness is current"));
        let resolved = db
            .protected_publication_witness(&repository)
            .await
            .expect("resolve current witness")
            .expect("publication exists");
        assert_eq!(resolved, *second.witness());

        overwrite_publication_expiry_for_test(
            &pool,
            community_uuid,
            AuthorizationProtectedObjectKind::Repository,
            &repository.object_key,
            60_000_000,
        )
        .await;
        let future_expiry = db
            .protected_publication_witness(&repository)
            .await
            .expect("load before expiry")
            .expect("future authority remains current");
        assert!(future_expiry.authoritative_now() < future_expiry.expires_at());

        overwrite_publication_expiry_for_test(
            &pool,
            community_uuid,
            AuthorizationProtectedObjectKind::Repository,
            &repository.object_key,
            0,
        )
        .await;
        assert!(db
            .protected_publication_witness(&repository)
            .await
            .expect("load at exclusive expiry")
            .is_none());
        assert!(!db
            .recheck_protected_publication_witness(&future_expiry)
            .await
            .expect("expired final recheck is closed"));

        overwrite_publication_expiry_for_test(
            &pool,
            community_uuid,
            AuthorizationProtectedObjectKind::Repository,
            &repository.object_key,
            -1,
        )
        .await;
        assert!(db
            .protected_publication_witness(&repository)
            .await
            .expect("load after expiry")
            .is_none());
        let media_target = [14_u8; 32];
        let media = ProtectedPublicationCoordinate {
            community_id: community,
            object_kind: AuthorizationProtectedObjectKind::Media,
            object_key: protected_object_key(
                community,
                AuthorizationProtectedObjectKind::Media,
                media_target,
            ),
            target_fingerprint: media_target,
        };
        let media_operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("valid media operation");
        let mut media_authority = initial_authority;
        media_authority.capability = RouteCapability::MediaWrite;
        media_authority.target_fingerprint = media_target;
        media_authority.correlation_id = Uuid::new_v4();
        media_authority.source_request_fingerprint = [15_u8; 32];
        media_authority.fence =
            AuthorizationLeaseFence::from_bytes([16_u8; 32]).expect("valid media fence");
        let media_digest = [17_u8; 32];
        let media_applied = db
            .commit_staged_protected_publication(request(
                media.clone(),
                media_operation,
                media_authority.clone(),
                media_digest,
                None,
            ))
            .await
            .expect("commit media publication");
        let media_replay = db
            .commit_staged_protected_publication(request(
                media,
                media_operation,
                media_authority,
                media_digest,
                None,
            ))
            .await
            .expect("replay media publication");
        assert_eq!(
            media_replay.disposition(),
            ProtectedPublicationDisposition::ExactReplay
        );
        assert_eq!(media_replay.result_digest(), media_applied.result_digest());

        db.latch_authorization_event_failure(
            community,
            AuthorizationAuditFailureCode::StorageUnavailable,
        )
        .await
        .expect("latch audit unavailable");
        let moderation_target = [18_u8; 32];
        let moderation = ProtectedPublicationCoordinate {
            community_id: community,
            object_kind: AuthorizationProtectedObjectKind::ModerationTarget,
            object_key: protected_object_key(
                community,
                AuthorizationProtectedObjectKind::ModerationTarget,
                moderation_target,
            ),
            target_fingerprint: moderation_target,
        };
        let audit_failure_operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("valid audit failure operation");
        let mut denied_authority = authority(RouteCapability::Moderation);
        denied_authority.community_id = community;
        denied_authority.correlation_id = Uuid::new_v4();
        denied_authority.actor = AuthorizationEventActor::test_direct(community);
        denied_authority.actor_pubkey = actor_pubkey;
        denied_authority.binding_id = binding_id;
        denied_authority.binding_version =
            u64::try_from(binding_version).expect("positive binding version");
        denied_authority.target_fingerprint = moderation_target;
        denied_authority.policy_revision = 1;
        denied_authority.invalidation_generation = 0;
        denied_authority.authority_epoch = 1;
        denied_authority.fence =
            AuthorizationLeaseFence::from_bytes([19_u8; 32]).expect("valid moderation fence");
        denied_authority.issued_at = Utc::now() - Duration::seconds(1);
        denied_authority.expires_at = Utc::now() + Duration::minutes(10);
        assert!(matches!(
            db.commit_staged_protected_publication(request(
                moderation.clone(),
                audit_failure_operation,
                denied_authority,
                [20_u8; 32],
                None,
            ))
            .await,
            Err(ProtectedPublicationError::AuditUnavailable)
        ));
        let failed_receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(audit_failure_operation.operation_id())
        .fetch_one(&pool)
        .await
        .expect("count failed audit receipts");
        assert_eq!(failed_receipts, 0);
        assert!(db
            .protected_publication_witness(&moderation)
            .await
            .expect("resolve failed publication")
            .is_none());

        sqlx::query(
            "UPDATE authorization_invalidation_domains \
             SET current_generation=1,updated_at=clock_timestamp() WHERE community_id=$1",
        )
        .bind(community_uuid)
        .execute(&pool)
        .await
        .expect("advance invalidation after publication");
        assert!(!db
            .recheck_protected_publication_witness(second.witness())
            .await
            .expect("invalidation makes the publication witness stale"));

        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }
}
