//! Durable provider-free authorization invalidation state.
//!
//! PostgreSQL is authoritative. Callers capture the exact domain generation
//! and dependency floors before evaluation, then compare them again at the
//! final allow fence. Local lifecycle and delegation authority changes enter
//! through the same typed selector set; there is no provider event vocabulary.

use std::{collections::BTreeSet, fmt};

use buzz_core::CommunityId;
use sha2::{Digest, Sha256};
use sqlx::{
    postgres::{PgListener, PgPoolOptions},
    Postgres, Row, Transaction,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    authorization_events::{
        record_authorization_event_tx, record_authorization_operation_receipt_tx,
        AuthorizationEventActor, AuthorizationEventKind, AuthorizationEventOutcome,
        AuthorizationEventWriteError, AuthorizationOperationKind, AuthorizationOperationOutcome,
        AuthorizationOperationReceipt, AuthorizationReasonCode, AuthorizationReceiptWrite,
        NewAuthorizationEvent,
    },
    authorization_version::{
        authorization_version_delegated_relationship_component_key,
        authorization_version_invalidation_generation_component_key, load_manifest_connection,
        record_authorization_operation_version_delta_tx, AuthorizationAuthorityEpochAdvance,
        AuthorizationAuthorityObjectEvidence, AuthorizationOperationVersionDelta,
        AuthorizationOperationVersionDeltaManifest, AuthorizationProtectedObjectKind,
        AuthorizationVersionComponentKind, ProtectedPublicationDependency,
    },
    Db, DbError, Result,
};

/// Maximum exact selectors accepted by one invalidation operation.
pub const MAX_AUTHORIZATION_INVALIDATION_SELECTORS: usize = 64;

const AUTHORIZATION_INVALIDATION_CHANNEL: &str = "buzz_authorization_invalidation_v1";

/// Committed authorization dependency change delivered to live observers.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthorizationInvalidationNotice {
    /// The canonical domain invalidation generation advanced.
    DomainAdvanced {
        /// Server-resolved authorization domain.
        authorization_domain: CommunityId,
        /// Exact committed generation.
        generation: u64,
    },
    /// One protected publication authority epoch advanced.
    ProtectedPublicationAdvanced {
        /// Opaque exact protected-object dependency.
        dependency: ProtectedPublicationDependency,
        /// Exact committed authority epoch.
        authority_epoch: u64,
    },
}

impl fmt::Debug for AuthorizationInvalidationNotice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DomainAdvanced { generation, .. } => formatter
                .debug_struct("AuthorizationInvalidationNotice::DomainAdvanced")
                .field("generation", generation)
                .field("authorization_domain", &"[REDACTED]")
                .finish(),
            Self::ProtectedPublicationAdvanced {
                authority_epoch, ..
            } => formatter
                .debug_struct("AuthorizationInvalidationNotice::ProtectedPublicationAdvanced")
                .field("authority_epoch", authority_epoch)
                .field("dependency", &"[REDACTED]")
                .finish(),
        }
    }
}

/// Dedicated independent PostgreSQL subscription for live authorization loss.
pub struct AuthorizationInvalidationSubscription {
    listener: PgListener,
    failed: bool,
}

impl AuthorizationInvalidationSubscription {
    /// Receive one committed typed notice.
    ///
    /// A lost listener connection is returned as an error instead of being
    /// transparently reconnected across a notification gap.
    pub async fn recv(&mut self) -> Result<AuthorizationInvalidationNotice> {
        if self.failed {
            return Err(DbError::InvalidData(
                "authorization invalidation listener is unhealthy".to_owned(),
            ));
        }
        let notification = match self.listener.try_recv().await {
            Ok(Some(notification)) => notification,
            Ok(None) => {
                self.failed = true;
                return Err(DbError::InvalidData(
                    "authorization invalidation listener was lost".to_owned(),
                ));
            }
            Err(error) => {
                self.failed = true;
                return Err(error.into());
            }
        };
        if notification.channel() != AUTHORIZATION_INVALIDATION_CHANNEL {
            self.failed = true;
            return Err(DbError::InvalidData(
                "authorization invalidation channel is invalid".to_owned(),
            ));
        }
        match parse_authorization_invalidation_notice(notification.payload()) {
            Ok(notice) => Ok(notice),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }
}

impl fmt::Debug for AuthorizationInvalidationSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationInvalidationSubscription([REDACTED])")
    }
}

/// Closed invalidation selector classes from migration 0030.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i16)]
pub enum AuthorizationInvalidationSelectorKind {
    /// Exact local principal fingerprint.
    Principal = 1,
    /// Exact Nostr key.
    NostrKey = 2,
    /// Exact binding with invalid-through binding version.
    Binding = 3,
    /// Exact server-issued session target.
    Session = 4,
    /// Entire authorization domain.
    Domain = 5,
    /// Exact local configuration revision.
    ConfigurationRevision = 6,
    /// Exact delegated relationship with invalid-through revision.
    DelegatedRelationship = 7,
}

impl AuthorizationInvalidationSelectorKind {
    fn from_database(value: i16) -> Result<Self> {
        match value {
            1 => Ok(Self::Principal),
            2 => Ok(Self::NostrKey),
            3 => Ok(Self::Binding),
            4 => Ok(Self::Session),
            5 => Ok(Self::Domain),
            6 => Ok(Self::ConfigurationRevision),
            7 => Ok(Self::DelegatedRelationship),
            _ => Err(DbError::InvalidData(
                "authorization invalidation selector kind is invalid".to_owned(),
            )),
        }
    }
}

/// Exact server-issued session target resistant to UUID reuse.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthorizationSessionTarget {
    session_id: Uuid,
    issuance_fence: Uuid,
}

impl AuthorizationSessionTarget {
    /// Construct a non-reusable session target.
    pub fn new(session_id: Uuid, issuance_fence: Uuid) -> Result<Self> {
        if session_id.is_nil() || issuance_fence.is_nil() {
            return Err(DbError::InvalidData(
                "authorization session target is invalid".to_owned(),
            ));
        }
        Ok(Self {
            session_id,
            issuance_fence,
        })
    }
}

impl fmt::Debug for AuthorizationSessionTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationSessionTarget([REDACTED])")
    }
}

/// Typed local invalidation dependency.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthorizationInvalidationSelector {
    /// Already-derived exact principal fingerprint.
    Principal([u8; 32]),
    /// Exact Nostr key.
    NostrKey([u8; 32]),
    /// Exact binding generation floor.
    Binding {
        /// Stable binding ID.
        binding_id: Uuid,
        /// Highest invalid binding version.
        invalid_through: u64,
    },
    /// Exact server-issued session.
    Session(AuthorizationSessionTarget),
    /// Entire domain.
    Domain,
    /// Exact positive local configuration revision.
    ConfigurationRevision(u64),
    /// Exact delegated relationship revision floor.
    DelegatedRelationship {
        /// Stable verifier-defined relationship identity.
        relationship_id: Uuid,
        /// Highest invalid relationship revision.
        invalid_through: u64,
    },
}

impl AuthorizationInvalidationSelector {
    /// Validate a principal fingerprint selector.
    pub fn principal(fingerprint: [u8; 32]) -> Result<Self> {
        nonzero(fingerprint, "principal fingerprint")?;
        Ok(Self::Principal(fingerprint))
    }

    /// Validate an exact Nostr-key selector.
    pub fn nostr_key(key: [u8; 32]) -> Result<Self> {
        nonzero(key, "Nostr key")?;
        Ok(Self::NostrKey(key))
    }

    /// Validate a binding invalid-through selector.
    pub fn binding(binding_id: Uuid, invalid_through: u64) -> Result<Self> {
        if binding_id.is_nil() || invalid_through == 0 || invalid_through > i64::MAX as u64 {
            return Err(DbError::InvalidData(
                "authorization binding invalidation selector is invalid".to_owned(),
            ));
        }
        Ok(Self::Binding {
            binding_id,
            invalid_through,
        })
    }

    /// Select an exact session issuance.
    pub const fn session(target: AuthorizationSessionTarget) -> Self {
        Self::Session(target)
    }

    /// Select the entire server-resolved domain.
    pub const fn domain() -> Self {
        Self::Domain
    }

    /// Validate a local configuration revision selector.
    pub fn configuration_revision(revision: u64) -> Result<Self> {
        if revision == 0 || revision > i64::MAX as u64 {
            return Err(DbError::InvalidData(
                "authorization configuration revision is invalid".to_owned(),
            ));
        }
        Ok(Self::ConfigurationRevision(revision))
    }

    /// Validate an exact delegated-relationship revision selector.
    pub fn delegated_relationship(relationship_id: Uuid, invalid_through: u64) -> Result<Self> {
        if relationship_id.is_nil() || invalid_through == 0 || invalid_through > i64::MAX as u64 {
            return Err(DbError::InvalidData(
                "authorization delegated relationship selector is invalid".to_owned(),
            ));
        }
        Ok(Self::DelegatedRelationship {
            relationship_id,
            invalid_through,
        })
    }

    /// Closed selector class.
    pub const fn kind(&self) -> AuthorizationInvalidationSelectorKind {
        match self {
            Self::Principal(_) => AuthorizationInvalidationSelectorKind::Principal,
            Self::NostrKey(_) => AuthorizationInvalidationSelectorKind::NostrKey,
            Self::Binding { .. } => AuthorizationInvalidationSelectorKind::Binding,
            Self::Session(_) => AuthorizationInvalidationSelectorKind::Session,
            Self::Domain => AuthorizationInvalidationSelectorKind::Domain,
            Self::ConfigurationRevision(_) => {
                AuthorizationInvalidationSelectorKind::ConfigurationRevision
            }
            Self::DelegatedRelationship { .. } => {
                AuthorizationInvalidationSelectorKind::DelegatedRelationship
            }
        }
    }

    /// Domain-separated selector fingerprint.
    pub fn fingerprint(&self, community_id: CommunityId) -> [u8; 32] {
        if let Self::DelegatedRelationship {
            relationship_id, ..
        } = self
        {
            // Migration 0030 intentionally reuses selector_fingerprint as the
            // restore component coordinate for delegated relationships. All
            // readers and writers must therefore store this one canonical key.
            return authorization_version_delegated_relationship_component_key(
                community_id,
                *relationship_id,
            );
        }
        let mut digest = Sha256::new();
        framed(&mut digest, b"buzz:authorization-invalidation-selector:v1");
        framed(&mut digest, community_id.as_uuid().as_bytes());
        framed(&mut digest, &(self.kind() as i16).to_be_bytes());
        match self {
            Self::Principal(value) | Self::NostrKey(value) => framed(&mut digest, value),
            Self::Binding { binding_id, .. } => framed(&mut digest, binding_id.as_bytes()),
            Self::Session(target) => {
                framed(&mut digest, target.session_id.as_bytes());
                framed(&mut digest, target.issuance_fence.as_bytes());
            }
            Self::Domain => {}
            Self::ConfigurationRevision(revision) => {
                framed(&mut digest, &revision.to_be_bytes());
            }
            Self::DelegatedRelationship { .. } => {
                unreachable!("delegated relationship coordinates return before selector encoding")
            }
        }
        digest.finalize().into()
    }

    fn binding_floor(&self) -> Option<u64> {
        match self {
            Self::Binding {
                invalid_through, ..
            } => Some(*invalid_through),
            _ => None,
        }
    }

    fn relationship_floor(&self) -> Option<u64> {
        match self {
            Self::DelegatedRelationship {
                invalid_through, ..
            } => Some(*invalid_through),
            _ => None,
        }
    }
}

impl fmt::Debug for AuthorizationInvalidationSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationSelector")
            .field("kind", &self.kind())
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// One stored selector floor at an exact generation.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationInvalidationFloor {
    /// Closed selector class.
    pub kind: AuthorizationInvalidationSelectorKind,
    /// Opaque selector fingerprint.
    pub fingerprint: [u8; 32],
    /// Domain generation that last advanced this floor.
    pub generation: u64,
    /// Highest invalid binding version for a binding selector.
    pub binding_version_floor: Option<u64>,
    /// Highest invalid relationship revision for a relationship selector.
    pub relationship_revision_floor: Option<u64>,
}

impl fmt::Debug for AuthorizationInvalidationFloor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationFloor")
            .field("kind", &self.kind)
            .field("fingerprint", &"[REDACTED]")
            .field("generation", &"[REDACTED]")
            .field("binding_version_floor", &"[REDACTED]")
            .field("relationship_revision_floor", &"[REDACTED]")
            .finish()
    }
}

/// Exact invalidation state captured for an authorization evaluation.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationInvalidationSnapshot {
    community_id: CommunityId,
    generation: u64,
    floors: Vec<AuthorizationInvalidationFloor>,
}

impl AuthorizationInvalidationSnapshot {
    /// Server-resolved domain.
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Current durable domain generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exact requested floors, sorted by kind and fingerprint.
    pub fn floors(&self) -> &[AuthorizationInvalidationFloor] {
        &self.floors
    }

    /// Require byte-for-byte current state at a final fence.
    pub fn accepts_exact_recheck(&self, current: &Self) -> bool {
        self == current
    }
}

impl fmt::Debug for AuthorizationInvalidationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationSnapshot")
            .field("community_id", &"[REDACTED]")
            .field("generation", &"[REDACTED]")
            .field("floor_count", &self.floors.len())
            .finish()
    }
}

/// Complete caller input for one durable invalidation operation.
#[derive(Clone)]
pub(crate) struct AuthorizationInvalidationRequest {
    /// Server-resolved domain.
    pub(crate) community_id: CommunityId,
    /// Idempotency operation ID.
    pub(crate) operation_id: Uuid,
    /// Exact semantic request fingerprint.
    pub(crate) request_fingerprint: [u8; 32],
    /// Actor derived from origin-sealed or database-rechecked authority.
    pub(crate) actor: AuthorizationEventActor,
    /// Optional pseudonymous subject fingerprint.
    pub(crate) subject_fingerprint: Option<[u8; 32]>,
    /// Correlation ID.
    pub(crate) correlation_id: Uuid,
    /// Exact attempt ID for canonical evidence replay.
    pub(crate) attempt_id: Uuid,
    /// Stable canonical event ID.
    pub(crate) event_id: Uuid,
    /// Non-empty exact selectors.
    pub(crate) selectors: Vec<AuthorizationInvalidationSelector>,
}

impl fmt::Debug for AuthorizationInvalidationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationRequest")
            .field("community_id", &"[REDACTED]")
            .field("operation_id", &"[REDACTED]")
            .field("selector_count", &self.selectors.len())
            .finish()
    }
}

/// Result of one atomic invalidation application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthorizationInvalidationApplied {
    /// Durable domain generation after this operation.
    pub(crate) generation: u64,
    /// Whether the exact operation was already committed.
    pub(crate) replay: bool,
}

/// Exact generation advance returned to a caller-owned lifecycle transaction.
///
/// The contained delta is opaque outside `buzz-db`, preventing relay callers
/// from fabricating restore attribution.
pub(crate) struct AuthorizationInvalidationAdvance {
    generation: u64,
    authority_objects: Vec<AuthorizationAuthorityObjectEvidence>,
    deltas: Vec<AuthorizationOperationVersionDelta>,
}

impl AuthorizationInvalidationAdvance {
    /// New durable domain generation.
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exact object evidence refenced before invalidation locking.
    #[allow(dead_code)] // Consumed by canonical admission-loss integration.
    pub(crate) fn authority_objects(&self) -> &[AuthorizationAuthorityObjectEvidence] {
        &self.authority_objects
    }

    /// Consume all exact database-owned restore deltas.
    pub(crate) fn into_deltas(self) -> Vec<AuthorizationOperationVersionDelta> {
        self.deltas
    }
}

impl fmt::Debug for AuthorizationInvalidationAdvance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationInvalidationAdvance([REDACTED])")
    }
}

/// Advance invalidation for one exact local admission loss inside the
/// lifecycle transaction that owns the receipt, history, audit, and manifest.
#[allow(dead_code)] // Consumed by lifecycle integration.
pub(crate) async fn apply_admission_loss_invalidation_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    selectors: &[AuthorizationInvalidationSelector],
    authority_advance: AuthorizationAuthorityEpochAdvance,
) -> Result<AuthorizationInvalidationAdvance> {
    validate_selectors(community_id, selectors)?;
    if operation_id.is_nil()
        || request_fingerprint == [0; 32]
        || !authority_advance.matches_operation(community_id, operation_id, request_fingerprint)
        || !loss_target_matches_selectors(authority_advance.loss_target(), selectors)
    {
        return Err(DbError::InvalidData(
            "authorization admission-loss invalidation is incomplete".to_owned(),
        ));
    }
    let (authority_objects, mut authority_deltas) = authority_advance.into_parts();
    let before = lock_invalidation_generation_tx(transaction, community_id).await?;
    let invalidation_advance = advance_locked_invalidation_tx(
        transaction,
        community_id,
        operation_id,
        request_fingerprint,
        selectors,
        before,
    )
    .await?;
    authority_deltas.extend(invalidation_advance.deltas);
    Ok(AuthorizationInvalidationAdvance {
        generation: invalidation_advance.generation,
        authority_objects,
        deltas: authority_deltas,
    })
}

/// Closed application failure preserving audit-unavailable classification.
#[derive(Debug, Error)]
pub(crate) enum AuthorizationInvalidationApplyError {
    /// Canonical evidence could not be persisted; no invalidation committed.
    #[error("authorization invalidation audit is unavailable")]
    AuditUnavailable,
    /// Invalid input or PostgreSQL failure.
    #[error(transparent)]
    Database(#[from] DbError),
}

impl Db {
    /// Install a ready listener on a dedicated connection independent of the
    /// writer pool's capacity.
    pub async fn subscribe_authorization_invalidations(
        &self,
    ) -> Result<AuthorizationInvalidationSubscription> {
        let connect_options = self.pool.connect_options();
        let listener_pool = PgPoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect_with(connect_options.as_ref().clone())
            .await?;
        let mut listener = PgListener::connect_with(&listener_pool).await?;
        listener.eager_reconnect(false);
        listener.listen(AUTHORIZATION_INVALIDATION_CHANNEL).await?;
        Ok(AuthorizationInvalidationSubscription {
            listener,
            failed: false,
        })
    }

    /// Atomically advance one domain generation, selector floors, canonical
    /// receipt/event, and exact operation version manifest.
    #[allow(dead_code)] // Called by the verified mutation adapter.
    pub(crate) async fn apply_authorization_invalidation(
        &self,
        request: AuthorizationInvalidationRequest,
    ) -> std::result::Result<AuthorizationInvalidationApplied, AuthorizationInvalidationApplyError>
    {
        validate_request(&request)?;
        let mut transaction = self.pool.begin().await.map_err(DbError::from)?;

        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2)",
        )
        .bind(request.community_id.as_uuid())
        .bind(request.operation_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(DbError::from)?
        {
            let manifest = load_manifest_connection(
                &mut transaction,
                request.community_id,
                request.operation_id,
            )
            .await?
            .ok_or_else(|| {
                DbError::InvalidData(
                    "authorization invalidation replay lacks exact attribution".to_owned(),
                )
            })?;
            let exact_component = invalidation_manifest_matches_request(&manifest, &request);
            if !exact_component {
                return Err(DbError::InvalidData(
                    "authorization invalidation replay attribution is incomplete".to_owned(),
                )
                .into());
            }
            let generation = manifest
                .components()
                .iter()
                .find(|component| {
                    component.component_kind()
                        == AuthorizationVersionComponentKind::InvalidationGeneration
                })
                .map(AuthorizationOperationVersionDelta::after_version)
                .ok_or_else(|| {
                    DbError::InvalidData(
                        "authorization invalidation replay generation is missing".to_owned(),
                    )
                })?;
            let result_digest = invalidation_result_digest(
                request.community_id,
                request.operation_id,
                request.request_fingerprint,
                generation,
                &request.selectors,
            );
            let receipt = AuthorizationOperationReceipt::new(
                request.community_id,
                request.operation_id,
                request.request_fingerprint,
                AuthorizationOperationKind::Invalidation,
                request.actor.clone(),
                AuthorizationOperationOutcome::Applied,
                result_digest,
            )?;
            if record_authorization_operation_receipt_tx(&mut transaction, &receipt).await?
                != AuthorizationReceiptWrite::ExactReplay
            {
                return Err(DbError::InvalidData(
                    "authorization invalidation replay receipt is incomplete".to_owned(),
                )
                .into());
            }
            let event = NewAuthorizationEvent::new(
                request.community_id,
                request.event_id,
                AuthorizationEventKind::InvalidationAdvanced,
                AuthorizationEventOutcome::Allowed,
                AuthorizationReasonCode::Invalidated,
                request.actor.clone(),
                request.subject_fingerprint,
                request.operation_id,
                Some(request.request_fingerprint),
                request.correlation_id,
                request.attempt_id,
            )?;
            if !matches!(
                record_authorization_event_tx(&mut transaction, &event).await,
                Ok(AuthorizationReceiptWrite::ExactReplay)
            ) {
                return Err(DbError::InvalidData(
                    "authorization invalidation replay event is incomplete".to_owned(),
                )
                .into());
            }
            transaction.rollback().await.map_err(DbError::from)?;
            return Ok(AuthorizationInvalidationApplied {
                generation,
                replay: true,
            });
        }

        let authority_advance = crate::authorization_version::advance_admission_loss_authority_tx(
            &mut transaction,
            request.community_id,
            request.operation_id,
            request.request_fingerprint,
            &request.actor,
        )
        .await?;
        let advance = apply_admission_loss_invalidation_tx(
            &mut transaction,
            request.community_id,
            request.operation_id,
            request.request_fingerprint,
            &request.selectors,
            authority_advance,
        )
        .await?;
        let generation = advance.generation();

        let result_digest = invalidation_result_digest(
            request.community_id,
            request.operation_id,
            request.request_fingerprint,
            generation,
            &request.selectors,
        );
        let receipt = AuthorizationOperationReceipt::new(
            request.community_id,
            request.operation_id,
            request.request_fingerprint,
            AuthorizationOperationKind::Invalidation,
            request.actor.clone(),
            AuthorizationOperationOutcome::Applied,
            result_digest,
        )?;
        let receipt_write =
            record_authorization_operation_receipt_tx(&mut transaction, &receipt).await?;
        if receipt_write != AuthorizationReceiptWrite::Inserted {
            return Err(DbError::InvalidData(
                "authorization invalidation receipt raced with another writer".to_owned(),
            )
            .into());
        }

        record_authorization_operation_version_delta_tx(
            &mut transaction,
            request.community_id,
            request.operation_id,
            request.request_fingerprint,
            advance.into_deltas(),
        )
        .await?;

        let event = NewAuthorizationEvent::new(
            request.community_id,
            request.event_id,
            AuthorizationEventKind::InvalidationAdvanced,
            AuthorizationEventOutcome::Allowed,
            AuthorizationReasonCode::Invalidated,
            request.actor.clone(),
            request.subject_fingerprint,
            request.operation_id,
            Some(request.request_fingerprint),
            request.correlation_id,
            request.attempt_id,
        )?;
        match record_authorization_event_tx(&mut transaction, &event).await {
            Ok(AuthorizationReceiptWrite::Inserted) => {}
            Ok(AuthorizationReceiptWrite::ExactReplay) => {
                return Err(DbError::InvalidData(
                    "authorization invalidation event raced with another writer".to_owned(),
                )
                .into());
            }
            Err(AuthorizationEventWriteError::CapacityUnavailable) => {
                transaction.rollback().await.map_err(DbError::from)?;
                latch_invalidation_audit_failure(
                    self,
                    request.community_id,
                    crate::authorization_events::AuthorizationAuditFailureCode::CapacityExhausted,
                )
                .await;
                return Err(AuthorizationInvalidationApplyError::AuditUnavailable);
            }
            Err(AuthorizationEventWriteError::Database(_)) => {
                transaction.rollback().await.map_err(DbError::from)?;
                latch_invalidation_audit_failure(
                    self,
                    request.community_id,
                    crate::authorization_events::AuthorizationAuditFailureCode::StorageUnavailable,
                )
                .await;
                return Err(AuthorizationInvalidationApplyError::AuditUnavailable);
            }
        }

        transaction.commit().await.map_err(DbError::from)?;
        Ok(AuthorizationInvalidationApplied {
            generation,
            replay: false,
        })
    }

    /// Capture exact current invalidation state for a bounded selector set.
    pub async fn authorization_invalidation_snapshot(
        &self,
        community_id: CommunityId,
        selectors: &[AuthorizationInvalidationSelector],
    ) -> Result<AuthorizationInvalidationSnapshot> {
        validate_selectors(community_id, selectors)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let generation: i64 = sqlx::query_scalar(
            "SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::NotFound("authorization invalidation domain is not activated".to_owned())
        })?;
        let mut floors = Vec::new();
        for selector in selectors {
            let fingerprint = selector.fingerprint(community_id);
            if let Some(row) = sqlx::query(
                "SELECT selector_kind,floor_generation,binding_version_floor, \
                        relationship_revision_floor \
                 FROM authorization_invalidation_floors \
                 WHERE community_id=$1 AND selector_kind=$2 AND selector_fingerprint=$3",
            )
            .bind(community_id.as_uuid())
            .bind(selector.kind() as i16)
            .bind(fingerprint.as_slice())
            .fetch_optional(&mut *transaction)
            .await?
            {
                floors.push(AuthorizationInvalidationFloor {
                    kind: AuthorizationInvalidationSelectorKind::from_database(
                        row.try_get("selector_kind")?,
                    )?,
                    fingerprint,
                    generation: database_version(row.try_get("floor_generation")?)?,
                    binding_version_floor: row
                        .try_get::<Option<i64>, _>("binding_version_floor")?
                        .map(database_version)
                        .transpose()?,
                    relationship_revision_floor: row
                        .try_get::<Option<i64>, _>("relationship_revision_floor")?
                        .map(database_version)
                        .transpose()?,
                });
            }
        }
        transaction.commit().await?;
        floors.sort_by_key(|floor| (floor.kind, floor.fingerprint));
        Ok(AuthorizationInvalidationSnapshot {
            community_id,
            generation: database_version(generation)?,
            floors,
        })
    }
}

async fn lock_invalidation_generation_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
) -> Result<u64> {
    let generation: i64 = sqlx::query_scalar(
        "SELECT current_generation FROM authorization_invalidation_domains \
         WHERE community_id=$1 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| {
        DbError::NotFound("authorization invalidation domain is not activated".to_owned())
    })?;
    database_version(generation)
}

async fn advance_locked_invalidation_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    selectors: &[AuthorizationInvalidationSelector],
    before: u64,
) -> Result<AuthorizationInvalidationAdvance> {
    let generation = before.checked_add(1).ok_or_else(|| {
        DbError::InvalidData("authorization invalidation generation exhausted".to_owned())
    })?;
    let updated = sqlx::query(
        "UPDATE authorization_invalidation_domains \
         SET current_generation=$2,updated_at=clock_timestamp() \
         WHERE community_id=$1 AND current_generation=$3",
    )
    .bind(community_id.as_uuid())
    .bind(to_database_version(generation)?)
    .bind(to_database_version(before)?)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "authorization invalidation generation changed concurrently".to_owned(),
        ));
    }

    let mut deltas = vec![AuthorizationOperationVersionDelta::new(
        AuthorizationVersionComponentKind::InvalidationGeneration,
        authorization_version_invalidation_generation_component_key(community_id),
        before,
        generation,
    )?];
    for selector in selectors {
        let selector_fingerprint = selector.fingerprint(community_id);
        let prior_relationship_floor = if selector.relationship_floor().is_some() {
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT relationship_revision_floor FROM authorization_invalidation_floors \
                 WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2 FOR UPDATE",
            )
            .bind(community_id.as_uuid())
            .bind(selector_fingerprint.as_slice())
            .fetch_optional(&mut **transaction)
            .await?
            .flatten()
            .map(database_version)
            .transpose()?
            .unwrap_or(0)
        } else {
            0
        };
        sqlx::query(
            "INSERT INTO authorization_invalidation_floors \
             (community_id,selector_kind,selector_fingerprint,floor_generation, \
              binding_version_floor,relationship_revision_floor,operation_id,request_fingerprint) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
             ON CONFLICT (community_id,selector_kind,selector_fingerprint) DO UPDATE SET \
               floor_generation=GREATEST(authorization_invalidation_floors.floor_generation, \
                                         EXCLUDED.floor_generation), \
               binding_version_floor=CASE WHEN EXCLUDED.binding_version_floor IS NULL \
                 THEN authorization_invalidation_floors.binding_version_floor \
                 ELSE GREATEST(authorization_invalidation_floors.binding_version_floor, \
                               EXCLUDED.binding_version_floor) END, \
               relationship_revision_floor=CASE WHEN EXCLUDED.relationship_revision_floor IS NULL \
                 THEN authorization_invalidation_floors.relationship_revision_floor \
                 ELSE GREATEST(authorization_invalidation_floors.relationship_revision_floor, \
                               EXCLUDED.relationship_revision_floor) END, \
               operation_id=EXCLUDED.operation_id,request_fingerprint=EXCLUDED.request_fingerprint, \
               updated_at=clock_timestamp()",
        )
        .bind(community_id.as_uuid())
        .bind(selector.kind() as i16)
        .bind(selector_fingerprint.as_slice())
        .bind(to_database_version(generation)?)
        .bind(selector.binding_floor().map(to_database_version).transpose()?)
        .bind(selector.relationship_floor().map(to_database_version).transpose()?)
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
        if let Some(relationship_floor) = selector.relationship_floor() {
            if relationship_floor > prior_relationship_floor {
                deltas.push(AuthorizationOperationVersionDelta::new(
                    AuthorizationVersionComponentKind::DelegatedRelationship,
                    selector_fingerprint,
                    prior_relationship_floor,
                    relationship_floor,
                )?);
            }
        }
    }

    notify_authorization_change_tx(
        transaction,
        &format!("D:{}:{generation}", community_id.as_uuid()),
    )
    .await?;

    Ok(AuthorizationInvalidationAdvance {
        generation,
        authority_objects: Vec::new(),
        deltas,
    })
}

pub(crate) async fn notify_protected_publication_advance_tx(
    transaction: &mut Transaction<'_, Postgres>,
    dependency: &ProtectedPublicationDependency,
    authority_epoch: u64,
) -> Result<()> {
    if authority_epoch == 0 {
        return Err(DbError::InvalidData(
            "authorization publication notice epoch is invalid".to_owned(),
        ));
    }
    notify_authorization_change_tx(
        transaction,
        &format!(
            "P:{}:{}:{}:{authority_epoch}",
            dependency.authorization_domain().as_uuid(),
            dependency.object_kind() as i16,
            hex::encode(dependency.object_key())
        ),
    )
    .await
}

async fn notify_authorization_change_tx(
    transaction: &mut Transaction<'_, Postgres>,
    payload: &str,
) -> Result<()> {
    sqlx::query("SELECT pg_notify($1,$2)")
        .bind(AUTHORIZATION_INVALIDATION_CHANNEL)
        .bind(payload)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn parse_authorization_invalidation_notice(
    payload: &str,
) -> Result<AuthorizationInvalidationNotice> {
    let fields: Vec<_> = payload.split(':').collect();
    match fields.as_slice() {
        ["D", domain, generation] => {
            let authorization_domain =
                CommunityId::from_uuid(Uuid::parse_str(domain).map_err(|_| {
                    DbError::InvalidData(
                        "authorization invalidation notice domain is invalid".to_owned(),
                    )
                })?);
            let generation = generation.parse::<u64>().map_err(|_| {
                DbError::InvalidData(
                    "authorization invalidation notice generation is invalid".to_owned(),
                )
            })?;
            if authorization_domain.as_uuid().is_nil() || generation == 0 {
                return Err(DbError::InvalidData(
                    "authorization invalidation notice is invalid".to_owned(),
                ));
            }
            Ok(AuthorizationInvalidationNotice::DomainAdvanced {
                authorization_domain,
                generation,
            })
        }
        ["P", domain, kind, key, authority_epoch] => {
            let authorization_domain =
                CommunityId::from_uuid(Uuid::parse_str(domain).map_err(|_| {
                    DbError::InvalidData(
                        "authorization publication notice domain is invalid".to_owned(),
                    )
                })?);
            let object_kind = kind
                .parse::<i16>()
                .map_err(|_| {
                    DbError::InvalidData(
                        "authorization publication notice kind is invalid".to_owned(),
                    )
                })
                .and_then(AuthorizationProtectedObjectKind::from_database)?;
            let key = hex::decode(key).map_err(|_| {
                DbError::InvalidData("authorization publication notice key is invalid".to_owned())
            })?;
            let object_key: [u8; 32] = key.try_into().map_err(|_| {
                DbError::InvalidData("authorization publication notice key is invalid".to_owned())
            })?;
            let authority_epoch = authority_epoch.parse::<u64>().map_err(|_| {
                DbError::InvalidData("authorization publication notice epoch is invalid".to_owned())
            })?;
            if authority_epoch == 0 {
                return Err(DbError::InvalidData(
                    "authorization publication notice epoch is invalid".to_owned(),
                ));
            }
            Ok(
                AuthorizationInvalidationNotice::ProtectedPublicationAdvanced {
                    dependency: ProtectedPublicationDependency::from_database_parts(
                        authorization_domain,
                        object_kind,
                        object_key,
                    )?,
                    authority_epoch,
                },
            )
        }
        _ => Err(DbError::InvalidData(
            "authorization invalidation notice is malformed".to_owned(),
        )),
    }
}

fn invalidation_manifest_matches_request(
    manifest: &AuthorizationOperationVersionDeltaManifest,
    request: &AuthorizationInvalidationRequest,
) -> bool {
    if manifest.request_fingerprint() != request.request_fingerprint {
        return false;
    }
    let invalidation_key =
        authorization_version_invalidation_generation_component_key(request.community_id);
    let delegated: BTreeSet<_> = request
        .selectors
        .iter()
        .filter_map(|selector| match selector {
            AuthorizationInvalidationSelector::DelegatedRelationship {
                relationship_id,
                invalid_through,
            } => Some((
                authorization_version_delegated_relationship_component_key(
                    request.community_id,
                    *relationship_id,
                ),
                *invalid_through,
            )),
            _ => None,
        })
        .collect();
    let mut invalidation_count = 0_usize;
    for component in manifest.components() {
        match component.component_kind() {
            AuthorizationVersionComponentKind::InvalidationGeneration => {
                invalidation_count += 1;
                if component.component_key() != invalidation_key
                    || component.before_version().checked_add(1) != Some(component.after_version())
                {
                    return false;
                }
            }
            AuthorizationVersionComponentKind::DelegatedRelationship => {
                if !delegated.contains(&(component.component_key(), component.after_version())) {
                    return false;
                }
            }
            AuthorizationVersionComponentKind::AuthorityEpoch => {}
            _ => return false,
        }
    }
    invalidation_count == 1
}

fn validate_request(request: &AuthorizationInvalidationRequest) -> Result<()> {
    if request.community_id.as_uuid().is_nil()
        || request.operation_id.is_nil()
        || request.request_fingerprint == [0; 32]
        || !request.actor.is_authenticated()
        || !request.actor.is_bound_to(request.community_id)
        || !actor_matches_selectors(&request.actor, &request.selectors)
        || request.correlation_id.is_nil()
        || request.attempt_id.is_nil()
        || request.event_id.is_nil()
    {
        return Err(DbError::InvalidData(
            "authorization invalidation request is invalid".to_owned(),
        ));
    }
    validate_selectors(request.community_id, &request.selectors)
}

fn actor_matches_selectors(
    actor: &AuthorizationEventActor,
    selectors: &[AuthorizationInvalidationSelector],
) -> bool {
    actor
        .authority_loss_target()
        .is_some_and(|target| loss_target_matches_selectors(target, selectors))
}

fn loss_target_matches_selectors(
    target: crate::authorization_events::AuthorizationAuthorityLossTarget,
    selectors: &[AuthorizationInvalidationSelector],
) -> bool {
    if selectors.len() != 1 {
        return false;
    }
    match target {
        crate::authorization_events::AuthorizationAuthorityLossTarget::Binding(
            binding_id,
            binding_version,
        ) => selectors.iter().any(|selector| {
            matches!(selector, AuthorizationInvalidationSelector::Binding {
                binding_id: candidate_id,
                invalid_through,
            } if *candidate_id == binding_id && *invalid_through == binding_version)
        }),
        crate::authorization_events::AuthorizationAuthorityLossTarget::Policy(policy_revision) => {
            selectors.iter().any(|selector| {
                matches!(selector, AuthorizationInvalidationSelector::ConfigurationRevision(
                revision
            ) if *revision == policy_revision)
            })
        }
        crate::authorization_events::AuthorizationAuthorityLossTarget::DelegatedRelationship(
            relationship_id,
            relationship_revision,
        ) => selectors.iter().any(|selector| {
            matches!(selector, AuthorizationInvalidationSelector::DelegatedRelationship {
                relationship_id: candidate_id,
                invalid_through,
            } if *candidate_id == relationship_id && *invalid_through == relationship_revision)
        }),
    }
}

async fn latch_invalidation_audit_failure(
    db: &Db,
    community_id: CommunityId,
    failure: crate::authorization_events::AuthorizationAuditFailureCode,
) {
    if let Err(error) = db
        .latch_authorization_event_failure(community_id, failure)
        .await
    {
        tracing::error!(error = %error, "failed to durably latch invalidation audit health");
    }
}

fn validate_selectors(
    community_id: CommunityId,
    selectors: &[AuthorizationInvalidationSelector],
) -> Result<()> {
    if community_id.as_uuid().is_nil()
        || selectors.is_empty()
        || selectors.len() > MAX_AUTHORIZATION_INVALIDATION_SELECTORS
    {
        return Err(DbError::InvalidData(
            "authorization invalidation selector set is invalid".to_owned(),
        ));
    }
    let unique: BTreeSet<_> = selectors
        .iter()
        .map(|selector| (selector.kind(), selector.fingerprint(community_id)))
        .collect();
    if unique.len() != selectors.len() {
        return Err(DbError::InvalidData(
            "authorization invalidation selector set contains duplicates".to_owned(),
        ));
    }
    Ok(())
}

fn invalidation_result_digest(
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    generation: u64,
    selectors: &[AuthorizationInvalidationSelector],
) -> [u8; 32] {
    let mut coordinates: Vec<_> = selectors
        .iter()
        .map(|selector| {
            (
                selector.kind(),
                selector.fingerprint(community_id),
                selector.binding_floor(),
                selector.relationship_floor(),
            )
        })
        .collect();
    coordinates.sort();
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-invalidation-result:v1");
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, operation_id.as_bytes());
    framed(&mut digest, &request_fingerprint);
    framed(&mut digest, &generation.to_be_bytes());
    for (kind, fingerprint, binding_floor, relationship_floor) in coordinates {
        framed(&mut digest, &(kind as i16).to_be_bytes());
        framed(&mut digest, &fingerprint);
        framed(
            &mut digest,
            &binding_floor.unwrap_or_default().to_be_bytes(),
        );
        framed(
            &mut digest,
            &relationship_floor.unwrap_or_default().to_be_bytes(),
        );
    }
    digest.finalize().into()
}

fn nonzero(value: [u8; 32], name: &str) -> Result<()> {
    if value == [0; 32] {
        return Err(DbError::InvalidData(format!(
            "authorization {name} must not be zero"
        )));
    }
    Ok(())
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn to_database_version(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| DbError::InvalidData("authorization version exceeds BIGINT".to_owned()))
}

fn database_version(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("authorization version is negative".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_auth::AuthorizationEventCapacityPolicy;
    use sqlx::PgPool;

    use crate::authorization_events::{
        resolve_local_admission_loss_actor_tx, LocalAdmissionLossCause,
    };

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(1))
    }

    fn loopback_test_database_url() -> String {
        format!(
            "{}://{}:{}@{}:{}/{}",
            "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
        )
    }

    #[test]
    fn selector_coordinates_are_closed_and_redacted() {
        let binding = AuthorizationInvalidationSelector::binding(Uuid::from_u128(2), 7)
            .expect("valid binding selector");
        let relationship =
            AuthorizationInvalidationSelector::delegated_relationship(Uuid::from_u128(2), 7)
                .expect("valid relationship selector");
        assert_ne!(
            binding.fingerprint(domain()),
            relationship.fingerprint(domain())
        );
        assert_eq!(
            relationship.fingerprint(domain()),
            authorization_version_delegated_relationship_component_key(
                domain(),
                Uuid::from_u128(2)
            )
        );
        assert!(!format!("{binding:?}").contains(&Uuid::from_u128(2).to_string()));
        assert!(AuthorizationInvalidationSelector::binding(Uuid::nil(), 7).is_err());
        assert!(
            AuthorizationInvalidationSelector::delegated_relationship(Uuid::from_u128(2), 0)
                .is_err()
        );
    }

    #[test]
    fn selected_domain_and_duplicate_sets_fail_closed() {
        let selector = AuthorizationInvalidationSelector::domain();
        assert!(validate_selectors(domain(), std::slice::from_ref(&selector)).is_ok());
        assert!(validate_selectors(domain(), &[selector.clone(), selector]).is_err());
        assert!(validate_selectors(
            CommunityId::from_uuid(Uuid::nil()),
            &[AuthorizationInvalidationSelector::domain()]
        )
        .is_err());
    }

    #[test]
    fn snapshot_requires_generation_and_floors_to_match_exactly() {
        let original = AuthorizationInvalidationSnapshot {
            community_id: domain(),
            generation: 7,
            floors: Vec::new(),
        };
        assert!(original.accepts_exact_recheck(&original));
        let changed = AuthorizationInvalidationSnapshot {
            community_id: domain(),
            generation: 8,
            floors: Vec::new(),
        };
        assert!(!original.accepts_exact_recheck(&changed));
    }

    #[test]
    fn live_notice_payloads_are_typed_bounded_and_redacted() {
        let domain = domain();
        let domain_notice =
            parse_authorization_invalidation_notice(&format!("D:{}:7", domain.as_uuid()))
                .expect("valid domain notice");
        assert!(matches!(
            domain_notice,
            AuthorizationInvalidationNotice::DomainAdvanced {
                authorization_domain,
                generation: 7,
            } if authorization_domain == domain
        ));
        let object_notice = parse_authorization_invalidation_notice(&format!(
            "P:{}:3:{}:9",
            domain.as_uuid(),
            hex::encode([8_u8; 32])
        ))
        .expect("valid publication notice");
        assert!(matches!(
            &object_notice,
            AuthorizationInvalidationNotice::ProtectedPublicationAdvanced {
                authority_epoch: 9,
                ..
            }
        ));
        assert!(!format!("{object_notice:?}").contains(&hex::encode([8_u8; 32])));
        for malformed in [
            "D:nil:1",
            "D:00000000-0000-0000-0000-000000000000:1",
            "D:00000000-0000-0000-0000-000000000001:0",
            "P:00000000-0000-0000-0000-000000000001:3:00:1",
            "P:00000000-0000-0000-0000-000000000001:99:0000000000000000000000000000000000000000000000000000000000000000:1",
        ] {
            assert!(parse_authorization_invalidation_notice(malformed).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replay_rejects_missing_event_and_incomplete_manifest() {
        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPool::connect(&admin_url).await.expect("connect admin");
        let name = format!("s5_invalidation_replay_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let scratch_url = format!("{}/{}", &admin_url[..split], name);
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");

        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community_uuid)
            .bind(format!("invalidation-{}.example", community_uuid.simple()))
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
        .expect("install capacity");
        let actor = {
            let mut transaction = pool.begin().await.expect("begin actor recheck");
            let actor = resolve_local_admission_loss_actor_tx(
                &mut transaction,
                community,
                LocalAdmissionLossCause::Policy { policy_revision: 1 },
            )
            .await
            .expect("resolve local policy actor");
            transaction.commit().await.expect("commit actor recheck");
            actor
        };
        let mut live_notices = db
            .subscribe_authorization_invalidations()
            .await
            .expect("install ready independent listener");

        let first = invalidation_request(community, actor.clone(), 10);
        let mut mismatched = first.clone();
        mismatched.operation_id = Uuid::from_u128(9_999);
        mismatched.selectors = vec![AuthorizationInvalidationSelector::domain()];
        assert!(db
            .apply_authorization_invalidation(mismatched)
            .await
            .is_err());
        let applied = db
            .apply_authorization_invalidation(first.clone())
            .await
            .expect("apply first invalidation");
        assert!(!applied.replay);
        assert!(matches!(
            live_notices.recv().await.expect("receive first generation"),
            AuthorizationInvalidationNotice::DomainAdvanced {
                authorization_domain,
                generation: 1,
            } if authorization_domain == community
        ));
        assert!(
            db.apply_authorization_invalidation(first.clone())
                .await
                .expect("exact replay")
                .replay
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), live_notices.recv())
                .await
                .is_err(),
            "exact replay emits no live notice"
        );

        let relationship_id = Uuid::new_v4();
        let relationship_selector =
            AuthorizationInvalidationSelector::delegated_relationship(relationship_id, 7)
                .expect("valid delegated relationship selector");
        let relationship_operation = Uuid::new_v4();
        let relationship_request = [71_u8; 32];
        let mut relationship_tx = pool.begin().await.expect("begin relationship floor");
        let relationship_before = lock_invalidation_generation_tx(&mut relationship_tx, community)
            .await
            .expect("lock relationship generation");
        let relationship_advance = advance_locked_invalidation_tx(
            &mut relationship_tx,
            community,
            relationship_operation,
            relationship_request,
            std::slice::from_ref(&relationship_selector),
            relationship_before,
        )
        .await
        .expect("advance relationship floor");
        let relationship_receipt = AuthorizationOperationReceipt::new(
            community,
            relationship_operation,
            relationship_request,
            AuthorizationOperationKind::Invalidation,
            actor.clone(),
            AuthorizationOperationOutcome::Applied,
            [72_u8; 32],
        )
        .expect("relationship receipt");
        record_authorization_operation_receipt_tx(&mut relationship_tx, &relationship_receipt)
            .await
            .expect("record relationship receipt");
        record_authorization_operation_version_delta_tx(
            &mut relationship_tx,
            community,
            relationship_operation,
            relationship_request,
            relationship_advance.into_deltas(),
        )
        .await
        .expect("record relationship manifest");
        relationship_tx
            .commit()
            .await
            .expect("commit relationship floor");
        assert!(matches!(
            live_notices
                .recv()
                .await
                .expect("receive relationship generation"),
            AuthorizationInvalidationNotice::DomainAdvanced {
                authorization_domain,
                generation: 2,
            } if authorization_domain == community
        ));
        let relationship_manifest = db
            .authorization_operation_version_delta(
                community,
                relationship_operation,
                relationship_request,
            )
            .await
            .expect("load relationship manifest");
        assert!(relationship_manifest.components().iter().any(|component| {
            component.component_kind() == AuthorizationVersionComponentKind::DelegatedRelationship
                && component.component_key()
                    == authorization_version_delegated_relationship_component_key(
                        community,
                        relationship_id,
                    )
                && component.before_version() == 0
                && component.after_version() == 7
        }));
        let floors = db
            .authorization_version_component_floors(community)
            .await
            .expect("load relationship floor");
        assert!(floors.iter().any(|floor| {
            floor.component_kind == AuthorizationVersionComponentKind::DelegatedRelationship
                && floor.component_key
                    == authorization_version_delegated_relationship_component_key(
                        community,
                        relationship_id,
                    )
                && floor.version == 7
        }));

        let mut corrupt = pool.acquire().await.expect("corrupt connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *corrupt)
            .await
            .expect("disable immutable triggers");
        sqlx::query("DELETE FROM authorization_events WHERE community_id=$1 AND operation_id=$2")
            .bind(community_uuid)
            .bind(first.operation_id)
            .execute(&mut *corrupt)
            .await
            .expect("remove replay event");
        assert!(db.apply_authorization_invalidation(first).await.is_err());

        let second = invalidation_request(community, actor.clone(), 20);
        db.apply_authorization_invalidation(second.clone())
            .await
            .expect("apply second invalidation");
        sqlx::query(
            "DELETE FROM authorization_operation_version_deltas \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(second.operation_id)
        .execute(&mut *corrupt)
        .await
        .expect("remove replay delta");
        assert!(db.apply_authorization_invalidation(second).await.is_err());

        let third = invalidation_request(community, actor.clone(), 30);
        db.apply_authorization_invalidation(third.clone())
            .await
            .expect("apply third invalidation");
        sqlx::query(
            "UPDATE authorization_operation_receipts SET result_digest=$3 \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(third.operation_id)
        .bind(vec![99_u8; 32])
        .execute(&mut *corrupt)
        .await
        .expect("corrupt replay receipt");
        assert!(db.apply_authorization_invalidation(third).await.is_err());

        let fourth = invalidation_request(community, actor, 40);
        db.apply_authorization_invalidation(fourth.clone())
            .await
            .expect("apply fourth invalidation");
        sqlx::query(
            "UPDATE authorization_events SET canonical_envelope=$3 \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_uuid)
        .bind(fourth.operation_id)
        .bind(vec![98_u8; 32])
        .execute(&mut *corrupt)
        .await
        .expect("corrupt replay envelope");
        assert!(db.apply_authorization_invalidation(fourth).await.is_err());
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *corrupt)
            .await
            .expect("restore immutable triggers");
        drop(corrupt);

        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }

    fn invalidation_request(
        community_id: CommunityId,
        actor: AuthorizationEventActor,
        tag: u8,
    ) -> AuthorizationInvalidationRequest {
        AuthorizationInvalidationRequest {
            community_id,
            operation_id: Uuid::from_u128(u128::from(tag) + 1_000),
            request_fingerprint: [tag; 32],
            actor,
            subject_fingerprint: None,
            correlation_id: Uuid::from_u128(u128::from(tag) + 2_000),
            attempt_id: Uuid::from_u128(u128::from(tag) + 3_000),
            event_id: Uuid::from_u128(u128::from(tag) + 4_000),
            selectors: vec![AuthorizationInvalidationSelector::configuration_revision(1)
                .expect("valid policy selector")],
        }
    }
}
