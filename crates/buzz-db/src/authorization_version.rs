//! Exact, operation-bound authorization version attribution.
//!
//! Every mutation records the complete set of authority components it advanced
//! under the same operation receipt. Restore witnessing consumes only this
//! manifest; there is deliberately no domain-global vector fallback.

mod protected;

pub(crate) use protected::protected_object_key;
#[cfg(test)]
pub(crate) use protected::protected_publication_request_for_restore_test;

pub use protected::{
    ProtectedPublicationCommit, ProtectedPublicationCoordinate, ProtectedPublicationDependency,
    ProtectedPublicationDisposition, ProtectedPublicationError,
    ProtectedPublicationOperationIdentity, ProtectedPublicationRequest,
    ProtectedPublicationRestoreIdentity, ProtectedPublicationValidity, ProtectedPublicationWitness,
};

use std::{collections::BTreeSet, fmt, time::Duration};

use buzz_auth::RouteCapability;
use buzz_core::{AuthorizationLeaseFence, CommunityId};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgConnection, Connection, PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    authorization_events::{AuthorizationAuthorityLossTarget, AuthorizationEventActor},
    Db, DbError, Result,
};

const MAX_COMPONENTS: usize = 1_024;
const MAX_SUPERSESSION_STEPS: usize = 1_024;
const MAX_DATABASE_VERSION: u64 = i64::MAX as u64;

#[cfg(test)]
pub(crate) fn loopback_test_database_url() -> String {
    format!(
        "{}://{}:{}@{}:{}/{}",
        "postgres", "buzz", "buzz_dev", "localhost", 5432, "buzz"
    )
}

fn authorization_operation_fence_key(
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
) -> i64 {
    let mut digest = Sha256::new();
    digest.update(b"buzz:nip-fi-operation-session-fence:v1");
    digest.update(community_id.as_uuid().as_bytes());
    digest.update(operation_id.as_bytes());
    digest.update(request_fingerprint);
    let digest: [u8; 32] = digest.finalize().into();
    let mut fence_key = [0_u8; 8];
    fence_key.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(fence_key)
}

/// Opaque lock-owning primary session issued only inside operation restore.
///
/// Canonical writers accept this capability rather than a raw PostgreSQL
/// connection. Its exact operation identity and session lock cannot be
/// changed or inspected by callers.
///
pub(crate) struct AuthorizationOperationFence {
    connection: Option<PgConnection>,
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    lock_key: i64,
}

/// Bounded failures while operation restore acquires its opaque session.
#[derive(Debug, Error)]
pub(crate) enum AuthorizationOperationFenceAcquireError {
    /// No dedicated primary connection became available within the bound.
    #[error("authorization operation fence pool is saturated")]
    PoolSaturated,
    /// The advisory-lock statement did not complete within the bound.
    #[error("authorization operation fence lock timed out")]
    LockTimedOut,
    /// PostgreSQL failed while acquiring the dedicated session.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl AuthorizationOperationFence {
    pub(crate) async fn acquire_for_operation_restore(
        pool: &PgPool,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        timeout: Duration,
    ) -> std::result::Result<Option<Self>, AuthorizationOperationFenceAcquireError> {
        if community_id.as_uuid().is_nil()
            || operation_id.is_nil()
            || request_fingerprint == [0; 32]
        {
            return Err(AuthorizationOperationFenceAcquireError::Database(
                sqlx::Error::Protocol(
                    "authorization operation fence identity is invalid".to_owned(),
                ),
            ));
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let pooled = tokio::time::timeout(timeout, pool.acquire())
            .await
            .map_err(|_| AuthorizationOperationFenceAcquireError::PoolSaturated)??;
        let mut connection = pooled.detach();
        let lock_key =
            authorization_operation_fence_key(community_id, operation_id, request_fingerprint);
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            drop(connection);
            return Err(AuthorizationOperationFenceAcquireError::LockTimedOut);
        }
        let acquired = tokio::time::timeout(
            remaining,
            sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
                .bind(lock_key)
                .fetch_one(&mut connection),
        )
        .await
        .map_err(|_| AuthorizationOperationFenceAcquireError::LockTimedOut)??;
        if !acquired {
            drop(connection);
            return Ok(None);
        }
        Ok(Some(Self {
            connection: Some(connection),
            community_id,
            operation_id,
            request_fingerprint,
            lock_key,
        }))
    }

    pub(crate) fn connection_for(
        &mut self,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<&mut PgConnection> {
        if self.community_id != community_id
            || self.operation_id != operation_id
            || self.request_fingerprint != request_fingerprint
        {
            return Err(DbError::InvalidData(
                "authorization operation fence identity mismatch".to_owned(),
            ));
        }
        self.connection.as_mut().ok_or_else(|| {
            DbError::InvalidData("authorization operation fence is consumed".to_owned())
        })
    }

    pub(crate) async fn release(mut self, timeout: Duration) -> Result<()> {
        let Some(mut connection) = self.connection.take() else {
            return Err(DbError::InvalidData(
                "authorization operation fence is consumed".to_owned(),
            ));
        };
        let unlocked = tokio::time::timeout(
            timeout,
            sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
                .bind(self.lock_key)
                .fetch_one(&mut connection),
        )
        .await
        .map_err(|_| DbError::InvalidData("authorization fence unlock timed out".to_owned()))??;
        if !unlocked {
            return Err(DbError::InvalidData(
                "authorization operation fence was not held".to_owned(),
            ));
        }
        tokio::time::timeout(timeout, connection.close())
            .await
            .map_err(|_| {
                DbError::InvalidData("authorization fence close timed out".to_owned())
            })??;
        Ok(())
    }
}

impl fmt::Debug for AuthorizationOperationFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationOperationFence([REDACTED])")
    }
}

/// Closed component classes persisted by migration 0030.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i16)]
pub enum AuthorizationVersionComponentKind {
    /// Immutable local binding generation.
    Binding = 1,
    /// Local enrollment-policy revision.
    Policy = 2,
    /// Domain invalidation generation.
    InvalidationGeneration = 3,
    /// Protected-object authority epoch.
    AuthorityEpoch = 4,
    /// Verified delegated-relationship revision.
    DelegatedRelationship = 6,
    /// Immutable lifecycle-selector fact generation.
    LifecycleSelector = 7,
}

/// Closed protected-object classes persisted by migration 0030.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i16)]
pub enum AuthorizationProtectedObjectKind {
    /// Whole authorization domain.
    Domain = 1,
    /// Relay channel.
    Channel = 2,
    /// Git repository.
    Repository = 3,
    /// Immutable media object.
    Media = 4,
    /// Moderation target.
    ModerationTarget = 5,
    /// Audio session.
    AudioSession = 6,
}

impl AuthorizationProtectedObjectKind {
    /// Whether this object class admits the exact read capability.
    pub fn admits_read(self, capability: RouteCapability) -> bool {
        match self {
            Self::Repository => matches!(
                capability,
                RouteCapability::ReposRead
                    | RouteCapability::ReposWrite
                    | RouteCapability::GitRead
                    | RouteCapability::GitWrite
                    | RouteCapability::GitStream
            ),
            Self::Media => matches!(
                capability,
                RouteCapability::MediaRead | RouteCapability::MediaWrite
            ),
            Self::ModerationTarget => capability == RouteCapability::Moderation,
            Self::Domain | Self::Channel | Self::AudioSession => false,
        }
    }

    /// Whether this object class admits the exact mutation capability.
    pub fn admits_mutation(self, capability: RouteCapability) -> bool {
        match self {
            Self::Repository => matches!(
                capability,
                RouteCapability::ReposWrite | RouteCapability::GitWrite
            ),
            Self::Media => capability == RouteCapability::MediaWrite,
            Self::ModerationTarget => capability == RouteCapability::Moderation,
            Self::Domain | Self::Channel | Self::AudioSession => false,
        }
    }

    pub(crate) fn from_database(value: i16) -> Result<Self> {
        match value {
            1 => Ok(Self::Domain),
            2 => Ok(Self::Channel),
            3 => Ok(Self::Repository),
            4 => Ok(Self::Media),
            5 => Ok(Self::ModerationTarget),
            6 => Ok(Self::AudioSession),
            _ => Err(DbError::InvalidData(
                "authorization protected object kind is invalid".to_owned(),
            )),
        }
    }
}

impl AuthorizationVersionComponentKind {
    fn from_database(value: i16) -> Result<Self> {
        match value {
            1 => Ok(Self::Binding),
            2 => Ok(Self::Policy),
            3 => Ok(Self::InvalidationGeneration),
            4 => Ok(Self::AuthorityEpoch),
            6 => Ok(Self::DelegatedRelationship),
            7 => Ok(Self::LifecycleSelector),
            _ => Err(DbError::InvalidData(
                "authorization version component kind is invalid".to_owned(),
            )),
        }
    }
}

/// One strict before-to-after advance for an exact authority coordinate.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationOperationVersionDelta {
    component_kind: AuthorizationVersionComponentKind,
    component_key: [u8; 32],
    before_version: u64,
    after_version: u64,
    component_digest: [u8; 32],
}

impl AuthorizationOperationVersionDelta {
    /// Construct a strict, database-representable version advance.
    pub(crate) fn new(
        component_kind: AuthorizationVersionComponentKind,
        component_key: [u8; 32],
        before_version: u64,
        after_version: u64,
    ) -> Result<Self> {
        if component_key == [0; 32]
            || before_version > MAX_DATABASE_VERSION
            || after_version > MAX_DATABASE_VERSION
            || after_version <= before_version
        {
            return Err(DbError::InvalidData(
                "authorization version delta is invalid".to_owned(),
            ));
        }
        let component_digest =
            component_digest(component_kind, component_key, before_version, after_version);
        Ok(Self {
            component_kind,
            component_key,
            before_version,
            after_version,
            component_digest,
        })
    }

    /// Closed component class.
    pub const fn component_kind(&self) -> AuthorizationVersionComponentKind {
        self.component_kind
    }

    /// Opaque exact component coordinate.
    pub const fn component_key(&self) -> [u8; 32] {
        self.component_key
    }

    /// Version observed before the operation.
    pub const fn before_version(&self) -> u64 {
        self.before_version
    }

    /// Version committed by the operation.
    pub const fn after_version(&self) -> u64 {
        self.after_version
    }

    /// Digest binding the complete component advance.
    pub const fn component_digest(&self) -> [u8; 32] {
        self.component_digest
    }
}

impl fmt::Debug for AuthorizationOperationVersionDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationOperationVersionDelta")
            .field("component_kind", &self.component_kind)
            .field("component_key", &"[REDACTED]")
            .field("before_version", &"[REDACTED]")
            .field("after_version", &"[REDACTED]")
            .field("component_digest", &"[REDACTED]")
            .finish()
    }
}

/// Canonical exact-operation version manifest loaded from PostgreSQL.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationOperationVersionDeltaManifest {
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    before_digest: [u8; 32],
    after_digest: [u8; 32],
    manifest_digest: [u8; 32],
    components: Vec<AuthorizationOperationVersionDelta>,
}

impl AuthorizationOperationVersionDeltaManifest {
    /// Server-resolved authorization domain.
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Exact operation receipt identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Exact request fingerprint bound to the operation receipt.
    pub const fn request_fingerprint(&self) -> [u8; 32] {
        self.request_fingerprint
    }

    /// Digest of the complete before-state projection.
    pub const fn before_digest(&self) -> [u8; 32] {
        self.before_digest
    }

    /// Digest of the complete after-state projection.
    pub const fn after_digest(&self) -> [u8; 32] {
        self.after_digest
    }

    /// Digest binding operation identity and every component.
    pub const fn manifest_digest(&self) -> [u8; 32] {
        self.manifest_digest
    }

    /// Strictly sorted, unique component advances.
    pub fn components(&self) -> &[AuthorizationOperationVersionDelta] {
        &self.components
    }
}

impl fmt::Debug for AuthorizationOperationVersionDeltaManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationOperationVersionDeltaManifest")
            .field("community_id", &"[REDACTED]")
            .field("operation_id", &"[REDACTED]")
            .field("request_fingerprint", &"[REDACTED]")
            .field("before_digest", &"[REDACTED]")
            .field("after_digest", &"[REDACTED]")
            .field("manifest_digest", &"[REDACTED]")
            .field("component_count", &self.components.len())
            .finish()
    }
}

/// One authoritative component baseline used to provision an external witness.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationVersionComponentFloor {
    /// Closed component class.
    pub component_kind: AuthorizationVersionComponentKind,
    /// Opaque exact component coordinate.
    pub component_key: [u8; 32],
    /// Current authoritative version; an absent resource has baseline zero.
    pub version: u64,
}

/// Exact protected-object authority coordinate advanced after a local
/// admission loss. Values are opaque and Debug-redacted outside this module.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AuthorizationAuthorityObjectEvidence {
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
}

impl AuthorizationAuthorityObjectEvidence {
    /// Closed migration-0030 object kind and canonical object key.
    #[allow(dead_code)] // Consumed by canonical admission integration.
    pub(crate) const fn coordinate(&self) -> (AuthorizationProtectedObjectKind, [u8; 32]) {
        (self.object_kind, self.object_key)
    }

    /// New exact epoch and nonzero observable fence.
    #[allow(dead_code)] // Consumed by canonical admission integration.
    pub(crate) const fn epoch_and_fence(&self) -> (u64, AuthorizationLeaseFence) {
        (self.authority_epoch, self.fence)
    }
}

impl fmt::Debug for AuthorizationAuthorityObjectEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizationAuthorityObjectEvidence([REDACTED])")
    }
}

/// Opaque exact set of protected-authority advances for one admission loss.
#[allow(dead_code)] // Consumed by lifecycle integration.
pub(crate) struct AuthorizationAuthorityEpochAdvance {
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    loss_target: AuthorizationAuthorityLossTarget,
    objects: Vec<AuthorizationAuthorityObjectEvidence>,
    deltas: Vec<AuthorizationOperationVersionDelta>,
}

impl AuthorizationAuthorityEpochAdvance {
    /// Exact object coordinates re-fenced in canonical order.
    #[allow(dead_code)] // Consumed by canonical admission integration.
    pub(crate) fn objects(&self) -> &[AuthorizationAuthorityObjectEvidence] {
        &self.objects
    }

    /// Consume the exact database-owned AuthorityEpoch deltas for the complete
    /// operation manifest.
    #[allow(dead_code)] // Consumed by canonical admission integration.
    pub(crate) fn matches_operation(
        &self,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> bool {
        self.community_id == community_id
            && self.operation_id == operation_id
            && self.request_fingerprint == request_fingerprint
    }

    /// Database-rechecked admission-loss coordinate that authorized this
    /// authority-first advance.
    pub(crate) const fn loss_target(&self) -> AuthorizationAuthorityLossTarget {
        self.loss_target
    }

    /// Consume the authority evidence and its exact DB-owned deltas.
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<AuthorizationAuthorityObjectEvidence>,
        Vec<AuthorizationOperationVersionDelta>,
    ) {
        (self.objects, self.deltas)
    }
}

impl fmt::Debug for AuthorizationAuthorityEpochAdvance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationAuthorityEpochAdvance")
            .field("object_count", &self.objects.len())
            .finish()
    }
}

impl fmt::Debug for AuthorizationVersionComponentFloor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationVersionComponentFloor")
            .field("component_kind", &self.component_kind)
            .field("component_key", &"[REDACTED]")
            .field("version", &"[REDACTED]")
            .finish()
    }
}

/// Derive an opaque, length-framed component coordinate.
pub fn authorization_version_component_key(namespace: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-version-component:v1");
    framed(&mut digest, namespace);
    for part in parts {
        framed(&mut digest, part);
    }
    digest.finalize().into()
}

/// Canonical binding-generation component coordinate.
pub fn authorization_version_binding_component_key(
    community_id: CommunityId,
    binding_id: Uuid,
) -> [u8; 32] {
    authorization_version_component_key(
        b"binding",
        &[community_id.as_uuid().as_bytes(), binding_id.as_bytes()],
    )
}

/// Canonical domain policy-revision component coordinate.
pub fn authorization_version_policy_component_key(community_id: CommunityId) -> [u8; 32] {
    authorization_version_component_key(b"policy", &[community_id.as_uuid().as_bytes()])
}

/// Canonical domain invalidation-generation component coordinate.
pub fn authorization_version_invalidation_generation_component_key(
    community_id: CommunityId,
) -> [u8; 32] {
    authorization_version_component_key(b"invalidation", &[community_id.as_uuid().as_bytes()])
}

/// Canonical protected-object authority-epoch component coordinate.
pub fn authorization_version_authority_epoch_component_key(
    community_id: CommunityId,
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
) -> [u8; 32] {
    authorization_version_component_key(
        b"authority-epoch",
        &[
            community_id.as_uuid().as_bytes(),
            &(object_kind as i16).to_be_bytes(),
            &object_key,
        ],
    )
}

/// Canonical delegated-relationship component coordinate.
pub fn authorization_version_delegated_relationship_component_key(
    community_id: CommunityId,
    relationship_id: Uuid,
) -> [u8; 32] {
    authorization_version_component_key(
        b"delegated-relationship",
        &[
            community_id.as_uuid().as_bytes(),
            relationship_id.as_bytes(),
        ],
    )
}

/// Canonical lifecycle-selector component coordinate.
pub fn authorization_version_lifecycle_selector_component_key(
    community_id: CommunityId,
    selector_id: Uuid,
) -> [u8; 32] {
    authorization_version_component_key(
        b"lifecycle-selector",
        &[community_id.as_uuid().as_bytes(), selector_id.as_bytes()],
    )
}

/// Advance every protected-object authority row affected by one exact,
/// database-rechecked local admission loss.
///
/// Lock order is all matching `authorization_authority_epochs` rows in
/// canonical `(object_kind, object_key)` order, followed by their coupled
/// `protected_object_authority` rows. No matching protected object is an
/// explicit valid outcome and returns empty evidence/deltas. Existing rows
/// must advance together with a new nonzero fence; partial state fails closed.
#[allow(dead_code)] // Consumed by admission-loss repair integration.
pub(crate) async fn advance_admission_loss_authority_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    actor: &AuthorizationEventActor,
) -> Result<AuthorizationAuthorityEpochAdvance> {
    if community_id.as_uuid().is_nil() || operation_id.is_nil() || request_fingerprint == [0; 32] {
        return Err(DbError::InvalidData(
            "authorization authority advance identity is invalid".to_owned(),
        ));
    }
    if !actor.is_bound_to(community_id) {
        return Err(DbError::InvalidData(
            "authorization authority actor domain does not match".to_owned(),
        ));
    }
    let target = actor.authority_loss_target().ok_or_else(|| {
        DbError::InvalidData(
            "authorization authority advance lacks a rechecked loss cause".to_owned(),
        )
    })?;
    let (
        target_kind,
        binding_id,
        binding_version,
        policy_revision,
        relationship_id,
        relationship_revision,
    ) = match target {
        AuthorizationAuthorityLossTarget::Binding(binding_id, binding_version) => (
            1_i16,
            Some(binding_id),
            Some(to_database_version(binding_version)?),
            None,
            None,
            None,
        ),
        AuthorizationAuthorityLossTarget::Policy(policy_revision) => (
            2_i16,
            None,
            None,
            Some(to_database_version(policy_revision)?),
            None,
            None,
        ),
        AuthorizationAuthorityLossTarget::DelegatedRelationship(
            relationship_id,
            relationship_revision,
        ) => (
            3_i16,
            None,
            None,
            None,
            Some(relationship_id),
            Some(to_database_version(relationship_revision)?),
        ),
    };

    let rows = sqlx::query(
        "SELECT epoch.object_kind,epoch.object_key,epoch.authority_epoch,epoch.fence \
         FROM authorization_authority_epochs epoch \
         WHERE epoch.community_id=$1 AND EXISTS ( \
           SELECT 1 FROM protected_object_authority protected \
           WHERE protected.community_id=epoch.community_id \
             AND protected.object_kind=epoch.object_kind AND protected.object_key=epoch.object_key \
             AND (($2=1 AND protected.binding_id=$3 AND protected.binding_version=$4) \
               OR ($2=2 AND protected.policy_revision=$5) \
               OR ($2=3 AND protected.delegated_relationship_id=$6 \
                         AND protected.delegated_relationship_revision=$7))) \
         ORDER BY epoch.object_kind,epoch.object_key LIMIT 1025 FOR UPDATE OF epoch",
    )
    .bind(community_id.as_uuid())
    .bind(target_kind)
    .bind(binding_id)
    .bind(binding_version)
    .bind(policy_revision)
    .bind(relationship_id)
    .bind(relationship_revision)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() > MAX_COMPONENTS {
        return Err(DbError::InvalidData(
            "authorization authority advance exceeds manifest capacity".to_owned(),
        ));
    }

    let mut objects = Vec::with_capacity(rows.len());
    let mut deltas = Vec::with_capacity(rows.len());
    for row in rows {
        let object_kind =
            AuthorizationProtectedObjectKind::from_database(row.try_get("object_kind")?)?;
        let object_key = bytes32(row.try_get("object_key")?, "authority object key")?;
        let before = from_database_version(row.try_get("authority_epoch")?)?;
        let old_fence = authorization_lease_fence(row.try_get("fence")?, "authority fence")?;
        let after = before.checked_add(1).ok_or_else(|| {
            DbError::InvalidData("authorization authority epoch exhausted".to_owned())
        })?;
        let fence = next_authority_fence(
            community_id,
            operation_id,
            request_fingerprint,
            object_kind,
            object_key,
            before,
            old_fence,
        )?;

        let locked = sqlx::query(
            "SELECT 1 FROM protected_object_authority \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3 \
               AND authority_epoch=$4 AND fence=$5 \
               AND (($6=1 AND binding_id=$7 AND binding_version=$8) \
                 OR ($6=2 AND policy_revision=$9) \
                 OR ($6=3 AND delegated_relationship_id=$10 \
                           AND delegated_relationship_revision=$11)) FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(object_kind as i16)
        .bind(object_key.as_slice())
        .bind(to_database_version(before)?)
        .bind(old_fence.as_bytes().as_slice())
        .bind(target_kind)
        .bind(binding_id)
        .bind(binding_version)
        .bind(policy_revision)
        .bind(relationship_id)
        .bind(relationship_revision)
        .fetch_optional(&mut **transaction)
        .await?;
        if locked.is_none() {
            return Err(DbError::InvalidData(
                "authorization protected authority changed during refence".to_owned(),
            ));
        }

        let epoch_updated = sqlx::query(
            "UPDATE authorization_authority_epochs SET authority_epoch=$6,fence=$7, \
               operation_id=$8,request_fingerprint=$9, \
               updated_at=GREATEST(clock_timestamp(),updated_at + INTERVAL '1 microsecond') \
             WHERE community_id=$1 AND object_kind=$2 AND object_key=$3 \
               AND authority_epoch=$4 AND fence=$5",
        )
        .bind(community_id.as_uuid())
        .bind(object_kind as i16)
        .bind(object_key.as_slice())
        .bind(to_database_version(before)?)
        .bind(old_fence.as_bytes().as_slice())
        .bind(to_database_version(after)?)
        .bind(fence.as_bytes().as_slice())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
        if epoch_updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization authority epoch did not advance exactly once".to_owned(),
            ));
        }

        let protected_updated = sqlx::query(
            "WITH next_time AS (SELECT GREATEST(clock_timestamp(),issued_at + \
                    INTERVAL '1 microsecond') AS value FROM protected_object_authority \
                    WHERE community_id=$1 AND object_kind=$2 AND object_key=$3) \
             UPDATE protected_object_authority protected SET authority_epoch=$6,fence=$7, \
               operation_id=$8,request_fingerprint=$9,issued_at=next_time.value, \
               expires_at=next_time.value + INTERVAL '1 microsecond' \
             FROM next_time \
             WHERE protected.community_id=$1 AND protected.object_kind=$2 \
               AND protected.object_key=$3 AND protected.authority_epoch=$4 \
               AND protected.fence=$5",
        )
        .bind(community_id.as_uuid())
        .bind(object_kind as i16)
        .bind(object_key.as_slice())
        .bind(to_database_version(before)?)
        .bind(old_fence.as_bytes().as_slice())
        .bind(to_database_version(after)?)
        .bind(fence.as_bytes().as_slice())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .execute(&mut **transaction)
        .await?;
        if protected_updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization protected authority did not advance exactly once".to_owned(),
            ));
        }

        objects.push(AuthorizationAuthorityObjectEvidence {
            object_kind,
            object_key,
            authority_epoch: after,
            fence,
        });
        deltas.push(AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::AuthorityEpoch,
            authorization_version_authority_epoch_component_key(
                community_id,
                object_kind,
                object_key,
            ),
            before,
            after,
        )?);
    }
    Ok(AuthorizationAuthorityEpochAdvance {
        community_id,
        operation_id,
        request_fingerprint,
        loss_target: target,
        objects,
        deltas,
    })
}

fn next_authority_fence(
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    object_kind: AuthorizationProtectedObjectKind,
    object_key: [u8; 32],
    before: u64,
    old_fence: AuthorizationLeaseFence,
) -> Result<AuthorizationLeaseFence> {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-authority-refence:v1");
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, operation_id.as_bytes());
    framed(&mut digest, &request_fingerprint);
    framed(&mut digest, &(object_kind as i16).to_be_bytes());
    framed(&mut digest, &object_key);
    framed(&mut digest, &before.to_be_bytes());
    framed(&mut digest, old_fence.as_bytes());
    let fence: [u8; 32] = digest.finalize().into();
    if fence == *old_fence.as_bytes() {
        return Err(DbError::InvalidData(
            "authorization authority fence did not advance".to_owned(),
        ));
    }
    AuthorizationLeaseFence::from_bytes(fence)
        .map_err(|_| DbError::InvalidData("authorization authority fence is invalid".to_owned()))
}

/// Record a complete operation manifest inside the caller-owned transaction.
///
/// The generic operation receipt may be inserted before or after this call;
/// migration 0030's deferred foreign key validates the pair at commit.
pub(crate) async fn record_authorization_operation_version_delta_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    components: Vec<AuthorizationOperationVersionDelta>,
) -> Result<AuthorizationOperationVersionDeltaManifest> {
    let candidate = build_manifest(community_id, operation_id, request_fingerprint, components)?;

    if let Some(existing) =
        load_manifest_connection(transaction, community_id, operation_id).await?
    {
        if existing == candidate {
            return Ok(existing);
        }
        return Err(DbError::InvalidData(
            "authorization operation version attribution conflicts with prior receipt".to_owned(),
        ));
    }

    sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id, operation_id, request_fingerprint, component_count, \
          before_digest, after_digest, manifest_digest) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(i32::try_from(candidate.components.len()).map_err(|_| {
        DbError::InvalidData("authorization version component count is invalid".to_owned())
    })?)
    .bind(candidate.before_digest.as_slice())
    .bind(candidate.after_digest.as_slice())
    .bind(candidate.manifest_digest.as_slice())
    .execute(&mut **transaction)
    .await?;

    for component in &candidate.components {
        sqlx::query(
            "INSERT INTO authorization_operation_version_deltas \
             (community_id, operation_id, component_kind, component_key, before_version, \
              after_version, component_digest) VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .bind(component.component_kind as i16)
        .bind(component.component_key.as_slice())
        .bind(to_database_version(component.before_version)?)
        .bind(to_database_version(component.after_version)?)
        .bind(component.component_digest.as_slice())
        .execute(&mut **transaction)
        .await?;
    }

    Ok(candidate)
}

impl Db {
    /// Read PostgreSQL's authoritative wall clock for external restore leases.
    pub async fn authorization_restore_clock_millis(&self) -> Result<u64> {
        let value: i64 = sqlx::query_scalar(
            "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::BIGINT",
        )
        .fetch_one(&self.pool)
        .await?;
        u64::try_from(value)
            .map_err(|_| DbError::InvalidData("authorization restore clock is invalid".to_owned()))
    }

    /// Load one exact operation manifest; missing or malformed attribution fails closed.
    pub async fn authorization_operation_version_delta(
        &self,
        community_id: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<AuthorizationOperationVersionDeltaManifest> {
        let mut connection = self.pool.acquire().await?;
        let manifest = load_manifest_connection(&mut connection, community_id, operation_id)
            .await?
            .ok_or_else(|| {
                DbError::NotFound("authorization operation version attribution".to_owned())
            })?;
        if manifest.request_fingerprint != request_fingerprint {
            return Err(DbError::InvalidData(
                "authorization operation version attribution identity mismatch".to_owned(),
            ));
        }
        Ok(manifest)
    }

    /// Resolve the immediate canonical operation predecessors of a pending
    /// manifest. This lets bounded restore maintenance find a predecessor in
    /// another immutable operation shard without scanning unrelated locks.
    pub async fn authorization_operation_version_predecessors(
        &self,
        community_id: CommunityId,
        components: &[(AuthorizationVersionComponentKind, [u8; 32], u64)],
    ) -> Result<Vec<AuthorizationOperationVersionDeltaManifest>> {
        if community_id.as_uuid().is_nil() || components.len() > MAX_COMPONENTS {
            return Err(DbError::InvalidData(
                "authorization predecessor coordinates are invalid".to_owned(),
            ));
        }
        let mut connection = self.pool.acquire().await?.detach();
        let mut identities = BTreeSet::new();
        for (kind, key, before) in components {
            if *key == [0; 32] || *before > MAX_DATABASE_VERSION {
                return Err(DbError::InvalidData(
                    "authorization predecessor component is invalid".to_owned(),
                ));
            }
            if *before == 0 {
                continue;
            }
            let rows = sqlx::query(
                "SELECT d.operation_id,m.request_fingerprint \
                 FROM authorization_operation_version_deltas d \
                 JOIN authorization_operation_version_delta_manifests m \
                   ON m.community_id=d.community_id AND m.operation_id=d.operation_id \
                 WHERE d.community_id=$1 AND d.component_kind=$2 AND d.component_key=$3 \
                   AND d.after_version=$4 \
                 ORDER BY d.operation_id LIMIT 2",
            )
            .bind(community_id.as_uuid())
            .bind(*kind as i16)
            .bind(key.as_slice())
            .bind(to_database_version(*before)?)
            .fetch_all(&mut connection)
            .await?;
            if rows.len() > 1 {
                return Err(DbError::InvalidData(
                    "authorization predecessor branches are ambiguous".to_owned(),
                ));
            }
            if let Some(row) = rows.first() {
                identities.insert((
                    row.try_get::<Uuid, _>("operation_id")?,
                    bytes32(row.try_get("request_fingerprint")?, "request fingerprint")?,
                ));
            }
        }

        let mut predecessors = Vec::with_capacity(identities.len());
        for (operation_id, request_fingerprint) in identities {
            let manifest = load_manifest_connection(&mut connection, community_id, operation_id)
                .await?
                .ok_or_else(|| {
                    DbError::InvalidData("authorization predecessor manifest is missing".to_owned())
                })?;
            if manifest.request_fingerprint != request_fingerprint {
                return Err(DbError::InvalidData(
                    "authorization predecessor identity mismatches".to_owned(),
                ));
            }
            predecessors.push(manifest);
        }
        connection.close().await?;
        Ok(predecessors)
    }

    /// Prove that a higher external component floor causally supersedes an
    /// exact pending operation component.
    ///
    /// Every immutable PostgreSQL delta in the interval is loaded through the
    /// canonical manifest validator. Gaps, overlaps, duplicate branches, or a
    /// latest provenance mismatch fail closed.
    #[allow(clippy::too_many_arguments)]
    pub async fn authorization_version_proves_component_supersession(
        &self,
        community_id: CommunityId,
        component_kind: AuthorizationVersionComponentKind,
        component_key: [u8; 32],
        pending_after: u64,
        floor_version: u64,
        floor_operation_id: Uuid,
        floor_request_fingerprint: [u8; 32],
        floor_component_digest: [u8; 32],
        floor_manifest_digest: [u8; 32],
    ) -> Result<()> {
        if community_id.as_uuid().is_nil()
            || component_key == [0; 32]
            || pending_after == 0
            || floor_version <= pending_after
            || floor_version > MAX_DATABASE_VERSION
            || floor_operation_id.is_nil()
            || floor_request_fingerprint == [0; 32]
            || floor_component_digest == [0; 32]
            || floor_manifest_digest == [0; 32]
        {
            return Err(DbError::InvalidData(
                "authorization supersession coordinates are invalid".to_owned(),
            ));
        }

        let before_floor = self
            .authorization_version_component_floors(community_id)
            .await?
            .into_iter()
            .find(|floor| {
                floor.component_kind == component_kind && floor.component_key == component_key
            })
            .map(|floor| floor.version)
            .unwrap_or(0);
        if before_floor != floor_version {
            return Err(DbError::InvalidData(
                "authorization supersession floor is not current".to_owned(),
            ));
        }

        let row_limit = i64::try_from(MAX_SUPERSESSION_STEPS + 1).map_err(|_| {
            DbError::InvalidData("authorization supersession step bound is invalid".to_owned())
        })?;
        let rows = sqlx::query(
            "SELECT d.operation_id,m.request_fingerprint,m.manifest_digest,\
                    d.before_version,d.after_version,d.component_digest \
             FROM authorization_operation_version_deltas d \
             JOIN authorization_operation_version_delta_manifests m \
               ON m.community_id=d.community_id AND m.operation_id=d.operation_id \
             WHERE d.community_id=$1 AND d.component_kind=$2 AND d.component_key=$3 \
               AND d.after_version>$4 AND d.after_version<=$5 \
             ORDER BY d.before_version,d.after_version,d.operation_id \
             LIMIT $6",
        )
        .bind(community_id.as_uuid())
        .bind(component_kind as i16)
        .bind(component_key.as_slice())
        .bind(i64::try_from(pending_after).map_err(|_| {
            DbError::InvalidData("authorization supersession version overflow".to_owned())
        })?)
        .bind(i64::try_from(floor_version).map_err(|_| {
            DbError::InvalidData("authorization supersession version overflow".to_owned())
        })?)
        .bind(row_limit)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() || rows.len() > MAX_SUPERSESSION_STEPS {
            return Err(DbError::InvalidData(
                "authorization supersession chain is missing or unbounded".to_owned(),
            ));
        }

        let mut expected_before = pending_after;
        let mut latest = None;
        for row in rows {
            let operation_id: Uuid = row.try_get("operation_id")?;
            let request_fingerprint =
                bytes32(row.try_get("request_fingerprint")?, "request fingerprint")?;
            let manifest_digest = bytes32(row.try_get("manifest_digest")?, "manifest digest")?;
            let before = from_database_version(row.try_get("before_version")?)?;
            let after = from_database_version(row.try_get("after_version")?)?;
            let component_digest = bytes32(row.try_get("component_digest")?, "component digest")?;
            if before != expected_before || after <= before {
                return Err(DbError::InvalidData(
                    "authorization supersession chain has a gap or overlap".to_owned(),
                ));
            }
            let manifest = self
                .authorization_operation_version_delta(
                    community_id,
                    operation_id,
                    request_fingerprint,
                )
                .await?;
            if manifest.manifest_digest != manifest_digest
                || !manifest.components.iter().any(|component| {
                    component.component_kind == component_kind
                        && component.component_key == component_key
                        && component.before_version == before
                        && component.after_version == after
                        && component.component_digest == component_digest
                })
            {
                return Err(DbError::InvalidData(
                    "authorization supersession manifest provenance mismatches".to_owned(),
                ));
            }
            expected_before = after;
            latest = Some((
                operation_id,
                request_fingerprint,
                component_digest,
                manifest_digest,
            ));
        }
        if expected_before != floor_version
            || latest
                != Some((
                    floor_operation_id,
                    floor_request_fingerprint,
                    floor_component_digest,
                    floor_manifest_digest,
                ))
        {
            return Err(DbError::InvalidData(
                "authorization supersession latest provenance mismatches".to_owned(),
            ));
        }

        let after_floor = self
            .authorization_version_component_floors(community_id)
            .await?
            .into_iter()
            .find(|floor| {
                floor.component_kind == component_kind && floor.component_key == component_key
            })
            .map(|floor| floor.version)
            .unwrap_or(0);
        if after_floor != floor_version {
            return Err(DbError::InvalidData(
                "authorization supersession floor changed during proof".to_owned(),
            ));
        }
        Ok(())
    }

    /// Read every current component floor for witness provisioning.
    ///
    /// Scalar policy and invalidation coordinates are returned even when their
    /// backing row is absent, using the frozen absent baseline zero.
    pub async fn authorization_version_component_floors(
        &self,
        community_id: CommunityId,
    ) -> Result<Vec<AuthorizationVersionComponentFloor>> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let mut floors = Vec::new();

        let binding_rows = sqlx::query(
            "SELECT binding_id, binding_version FROM identity_bindings \
             WHERE community_id=$1 ORDER BY binding_id",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        for row in binding_rows {
            let binding_id: Uuid = row.try_get("binding_id")?;
            floors.push(AuthorizationVersionComponentFloor {
                component_kind: AuthorizationVersionComponentKind::Binding,
                component_key: authorization_version_binding_component_key(
                    community_id,
                    binding_id,
                ),
                version: from_database_version(row.try_get("binding_version")?)?,
            });
        }

        let policy_version: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(policy_revision) FROM identity_enrollment_policies WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await?;
        floors.push(AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Policy,
            component_key: authorization_version_policy_component_key(community_id),
            version: policy_version
                .map(from_database_version)
                .transpose()?
                .unwrap_or(0),
        });

        let invalidation: Option<i64> = sqlx::query_scalar(
            "SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await?;
        floors.push(AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::InvalidationGeneration,
            component_key: authorization_version_invalidation_generation_component_key(
                community_id,
            ),
            version: invalidation
                .map(from_database_version)
                .transpose()?
                .unwrap_or(0),
        });

        let authority_rows = sqlx::query(
            "SELECT object_kind, object_key, authority_epoch \
             FROM authorization_authority_epochs WHERE community_id=$1 \
             ORDER BY object_kind, object_key",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        for row in authority_rows {
            let object_key = bytes32(row.try_get("object_key")?, "authority object key")?;
            let object_kind =
                AuthorizationProtectedObjectKind::from_database(row.try_get("object_kind")?)?;
            floors.push(AuthorizationVersionComponentFloor {
                component_kind: AuthorizationVersionComponentKind::AuthorityEpoch,
                component_key: authorization_version_authority_epoch_component_key(
                    community_id,
                    object_kind,
                    object_key,
                ),
                version: from_database_version(row.try_get("authority_epoch")?)?,
            });
        }

        let relationship_rows = sqlx::query(
            "SELECT selector_fingerprint, relationship_revision_floor \
             FROM authorization_invalidation_floors \
             WHERE community_id=$1 AND selector_kind=7 \
             ORDER BY selector_fingerprint",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        for row in relationship_rows {
            floors.push(AuthorizationVersionComponentFloor {
                component_kind: AuthorizationVersionComponentKind::DelegatedRelationship,
                component_key: bytes32(
                    row.try_get("selector_fingerprint")?,
                    "delegated relationship key",
                )?,
                version: from_database_version(row.try_get("relationship_revision_floor")?)?,
            });
        }

        let selector_rows = sqlx::query(
            "SELECT selector_id, fact_generation FROM identity_lifecycle_selectors \
             WHERE community_id=$1 ORDER BY selector_id",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        for row in selector_rows {
            let selector_id: Uuid = row.try_get("selector_id")?;
            floors.push(AuthorizationVersionComponentFloor {
                component_kind: AuthorizationVersionComponentKind::LifecycleSelector,
                component_key: authorization_version_lifecycle_selector_component_key(
                    community_id,
                    selector_id,
                ),
                version: from_database_version(row.try_get("fact_generation")?)?,
            });
        }

        transaction.commit().await?;
        floors.sort_by_key(|floor| (floor.component_kind, floor.component_key));
        if floors.windows(2).any(|window| {
            window[0].component_kind == window[1].component_kind
                && window[0].component_key == window[1].component_key
        }) {
            return Err(DbError::InvalidData(
                "authorization component floors are not unique".to_owned(),
            ));
        }
        Ok(floors)
    }
}

fn build_manifest(
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    mut components: Vec<AuthorizationOperationVersionDelta>,
) -> Result<AuthorizationOperationVersionDeltaManifest> {
    if community_id.as_uuid().is_nil()
        || operation_id.is_nil()
        || request_fingerprint == [0; 32]
        || components.len() > MAX_COMPONENTS
    {
        return Err(DbError::InvalidData(
            "authorization operation version manifest identity is invalid".to_owned(),
        ));
    }
    components.sort_by_key(|component| (component.component_kind, component.component_key));
    let coordinates: BTreeSet<_> = components
        .iter()
        .map(|component| (component.component_kind, component.component_key))
        .collect();
    if coordinates.len() != components.len() {
        return Err(DbError::InvalidData(
            "authorization operation version manifest has duplicate components".to_owned(),
        ));
    }

    let before_digest = state_digest(
        b"before",
        community_id,
        operation_id,
        request_fingerprint,
        &components,
        false,
    );
    let after_digest = state_digest(
        b"after",
        community_id,
        operation_id,
        request_fingerprint,
        &components,
        true,
    );
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-operation-manifest:v1");
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, operation_id.as_bytes());
    framed(&mut digest, &request_fingerprint);
    framed(&mut digest, &before_digest);
    framed(&mut digest, &after_digest);
    framed(&mut digest, &(components.len() as u64).to_be_bytes());
    for component in &components {
        framed(&mut digest, &component.component_digest);
    }
    Ok(AuthorizationOperationVersionDeltaManifest {
        community_id,
        operation_id,
        request_fingerprint,
        before_digest,
        after_digest,
        manifest_digest: digest.finalize().into(),
        components,
    })
}

pub(crate) async fn load_manifest_connection(
    connection: &mut PgConnection,
    community_id: CommunityId,
    operation_id: Uuid,
) -> Result<Option<AuthorizationOperationVersionDeltaManifest>> {
    let Some(row) = sqlx::query(
        "SELECT request_fingerprint, component_count, before_digest, after_digest, manifest_digest \
         FROM authorization_operation_version_delta_manifests \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };

    let request_fingerprint = bytes32(row.try_get("request_fingerprint")?, "request fingerprint")?;
    let component_count: i32 = row.try_get("component_count")?;
    if !(0..=MAX_COMPONENTS as i32).contains(&component_count) {
        return Err(DbError::InvalidData(
            "authorization operation component count is invalid".to_owned(),
        ));
    }
    let component_rows = sqlx::query(
        "SELECT component_kind, component_key, before_version, after_version, component_digest \
         FROM authorization_operation_version_deltas \
         WHERE community_id=$1 AND operation_id=$2 \
         ORDER BY component_kind, component_key",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;
    if component_rows.len() != component_count as usize {
        return Err(DbError::InvalidData(
            "authorization operation manifest is incomplete".to_owned(),
        ));
    }
    let mut components = Vec::with_capacity(component_rows.len());
    for component_row in component_rows {
        let kind = AuthorizationVersionComponentKind::from_database(
            component_row.try_get("component_kind")?,
        )?;
        let key = bytes32(component_row.try_get("component_key")?, "component key")?;
        let before = from_database_version(component_row.try_get("before_version")?)?;
        let after = from_database_version(component_row.try_get("after_version")?)?;
        let component = AuthorizationOperationVersionDelta::new(kind, key, before, after)?;
        let stored_digest = bytes32(
            component_row.try_get("component_digest")?,
            "component digest",
        )?;
        if component.component_digest != stored_digest {
            return Err(DbError::InvalidData(
                "authorization operation component digest mismatch".to_owned(),
            ));
        }
        components.push(component);
    }
    let rebuilt = build_manifest(community_id, operation_id, request_fingerprint, components)?;
    if rebuilt.before_digest != bytes32(row.try_get("before_digest")?, "before digest")?
        || rebuilt.after_digest != bytes32(row.try_get("after_digest")?, "after digest")?
        || rebuilt.manifest_digest != bytes32(row.try_get("manifest_digest")?, "manifest digest")?
    {
        return Err(DbError::InvalidData(
            "authorization operation manifest digest mismatch".to_owned(),
        ));
    }
    Ok(Some(rebuilt))
}

fn component_digest(
    kind: AuthorizationVersionComponentKind,
    key: [u8; 32],
    before: u64,
    after: u64,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-version-delta:v1");
    framed(&mut digest, &(kind as i16).to_be_bytes());
    framed(&mut digest, &key);
    framed(&mut digest, &before.to_be_bytes());
    framed(&mut digest, &after.to_be_bytes());
    digest.finalize().into()
}

fn state_digest(
    label: &[u8],
    community_id: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    components: &[AuthorizationOperationVersionDelta],
    after: bool,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    framed(&mut digest, b"buzz:authorization-version-state:v1");
    framed(&mut digest, label);
    framed(&mut digest, community_id.as_uuid().as_bytes());
    framed(&mut digest, operation_id.as_bytes());
    framed(&mut digest, &request_fingerprint);
    framed(&mut digest, &(components.len() as u64).to_be_bytes());
    for component in components {
        framed(
            &mut digest,
            &(component.component_kind as i16).to_be_bytes(),
        );
        framed(&mut digest, &component.component_key);
        framed(
            &mut digest,
            &if after {
                component.after_version
            } else {
                component.before_version
            }
            .to_be_bytes(),
        );
    }
    digest.finalize().into()
}

fn framed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn bytes32(value: Vec<u8>, field: &str) -> Result<[u8; 32]> {
    value.try_into().map_err(|_| {
        DbError::InvalidData(format!(
            "authorization {field} must contain exactly 32 bytes"
        ))
    })
}

fn authorization_lease_fence(value: Vec<u8>, field: &str) -> Result<AuthorizationLeaseFence> {
    AuthorizationLeaseFence::from_bytes(bytes32(value, field)?)
        .map_err(|_| DbError::InvalidData(format!("authorization {field} must be nonzero")))
}

fn to_database_version(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| DbError::InvalidData("authorization version exceeds BIGINT".to_owned()))
}

fn from_database_version(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("authorization version is negative".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Acquire, PgPool};

    use crate::authorization_events::{
        record_authorization_operation_receipt_tx, resolve_local_admission_loss_actor_tx,
        AuthorizationOperationKind, AuthorizationOperationOutcome, AuthorizationOperationReceipt,
        LocalAdmissionLossCause,
    };

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(1))
    }

    #[test]
    fn deltas_are_strict_bounded_and_redacted() {
        let key = authorization_version_policy_component_key(domain());
        let delta = AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::Policy,
            key,
            7,
            9,
        )
        .expect("strict non-unit advance");
        assert_eq!(delta.before_version(), 7);
        assert_eq!(delta.after_version(), 9);
        assert!(!format!("{delta:?}").contains(&hex::encode(key)));
        assert!(AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::Policy,
            key,
            7,
            7,
        )
        .is_err());
        assert!(AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::Policy,
            [0; 32],
            7,
            8,
        )
        .is_err());
    }

    #[test]
    fn manifests_are_canonical_and_empty_is_explicit() {
        let operation = Uuid::from_u128(2);
        let request = [3; 32];
        let first = AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::Policy,
            authorization_version_policy_component_key(domain()),
            1,
            2,
        )
        .expect("policy delta");
        let second = AuthorizationOperationVersionDelta::new(
            AuthorizationVersionComponentKind::InvalidationGeneration,
            authorization_version_invalidation_generation_component_key(domain()),
            4,
            8,
        )
        .expect("generation delta");
        let forward = build_manifest(
            domain(),
            operation,
            request,
            vec![first.clone(), second.clone()],
        )
        .expect("forward manifest");
        let reverse = build_manifest(domain(), operation, request, vec![second, first])
            .expect("reverse manifest");
        assert_eq!(forward, reverse);

        let empty = build_manifest(domain(), operation, request, Vec::new())
            .expect("explicit empty manifest");
        assert!(empty.components().is_empty());
        assert_ne!(empty.manifest_digest(), [0; 32]);
    }

    #[test]
    fn component_namespaces_and_coordinates_do_not_alias() {
        let binding = authorization_version_binding_component_key(domain(), Uuid::from_u128(9));
        let selector =
            authorization_version_lifecycle_selector_component_key(domain(), Uuid::from_u128(9));
        let relation = authorization_version_delegated_relationship_component_key(
            domain(),
            Uuid::from_u128(9),
        );
        assert_ne!(binding, selector);
        assert_ne!(binding, relation);
        assert_ne!(selector, relation);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn component_floor_provisioning_is_one_repeatable_read_snapshot() {
        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPool::connect(&admin_url).await.expect("connect admin");
        let name = format!("s5_floor_snapshot_{}", Uuid::new_v4().simple());
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

        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community)
            .bind(format!("floor-{}.example", community.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        let mut seed = pool.acquire().await.expect("seed connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *seed)
            .await
            .expect("disable seed triggers");
        insert_policy(&mut seed, community, 1).await;
        insert_binding(&mut seed, community, 1).await;
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *seed)
            .await
            .expect("restore seed triggers");
        drop(seed);

        let mut blocker = pool.acquire().await.expect("blocker connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *blocker)
            .await
            .expect("disable blocker triggers");
        let mut blocker_tx = blocker.begin().await.expect("begin blocker");
        sqlx::query("LOCK TABLE identity_enrollment_policies IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker_tx)
            .await
            .expect("lock policy table");

        let db = Db::from_pool(pool.clone());
        let domain = CommunityId::from_uuid(community);
        let reader = tokio::spawn(async move {
            db.authorization_version_component_floors(domain)
                .await
                .expect("read component floors")
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        insert_policy(&mut blocker_tx, community, 2).await;
        insert_binding(&mut blocker_tx, community, 2).await;
        blocker_tx
            .commit()
            .await
            .expect("commit concurrent advance");
        let floors = reader.await.expect("join floor reader");

        let binding_count = floors
            .iter()
            .filter(|floor| floor.component_kind == AuthorizationVersionComponentKind::Binding)
            .count() as u64;
        let policy_version = floors
            .iter()
            .find(|floor| floor.component_kind == AuthorizationVersionComponentKind::Policy)
            .expect("policy floor")
            .version;
        assert_eq!(
            binding_count, policy_version,
            "all floor classes must come from one snapshot"
        );

        let mut absent_tx = pool.begin().await.expect("begin absent authority check");
        let absent_actor = resolve_local_admission_loss_actor_tx(
            &mut absent_tx,
            domain,
            LocalAdmissionLossCause::Policy { policy_revision: 1 },
        )
        .await
        .expect("resolve absent policy actor");
        let absent = advance_admission_loss_authority_tx(
            &mut absent_tx,
            domain,
            Uuid::new_v4(),
            [50_u8; 32],
            &absent_actor,
        )
        .await
        .expect("absence is an explicit valid result");
        assert!(absent.objects().is_empty());
        assert!(absent.into_parts().1.is_empty());
        absent_tx.rollback().await.expect("rollback absent check");

        // Exercise the coupled AuthorityEpoch + protected-object refence seam
        // on the same fresh schema. Seed the prior immutable authority through
        // the test-only replication role, then require one exact atomic advance.
        let old_operation = Uuid::new_v4();
        let new_operation = Uuid::new_v4();
        let object_key = [51_u8; 32];
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,11,$4,1,$5)",
        )
        .bind(community)
        .bind(old_operation)
        .bind(vec![52_u8; 32])
        .bind(vec![53_u8; 32])
        .bind(vec![54_u8; 32])
        .execute(&mut *blocker)
        .await
        .expect("seed prior authority receipt");
        sqlx::query(
            "INSERT INTO authorization_authority_epochs \
             (community_id,object_kind,object_key,authority_epoch,fence,operation_id,request_fingerprint) \
             VALUES ($1,2,$2,1,$3,$4,$5)",
        )
        .bind(community)
        .bind(object_key.as_slice())
        .bind(vec![55_u8; 32])
        .bind(old_operation)
        .bind(vec![52_u8; 32])
        .execute(&mut *blocker)
        .await
        .expect("seed authority epoch");
        sqlx::query(
            "INSERT INTO protected_object_authority \
             (community_id,object_kind,object_key,capability,actor_pubkey,binding_id,binding_version, \
              policy_revision,invalidation_generation,authority_epoch,fence,issued_at,expires_at, \
              operation_id,request_fingerprint) \
             VALUES ($1,2,$2,1,$3,$4,2,2,0,1,$5,clock_timestamp()-INTERVAL '1 second', \
                     clock_timestamp()+INTERVAL '1 hour',$6,$7)",
        )
        .bind(community)
        .bind(object_key.as_slice())
        .bind(vec![12_u8; 32])
        .bind(Uuid::from_u128(102))
        .bind(vec![55_u8; 32])
        .bind(old_operation)
        .bind(vec![52_u8; 32])
        .execute(&mut *blocker)
        .await
        .expect("seed protected authority");
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *blocker)
            .await
            .expect("restore blocker triggers");
        drop(blocker);

        let request_fingerprint = [61_u8; 32];
        let mut authority_tx = pool.begin().await.expect("begin authority advance");
        let actor = resolve_local_admission_loss_actor_tx(
            &mut authority_tx,
            domain,
            LocalAdmissionLossCause::Policy { policy_revision: 2 },
        )
        .await
        .expect("resolve policy actor");
        let advance = advance_admission_loss_authority_tx(
            &mut authority_tx,
            domain,
            new_operation,
            request_fingerprint,
            &actor,
        )
        .await
        .expect("advance coupled authority");
        assert_eq!(advance.objects().len(), 1);
        assert_eq!(
            advance.objects()[0].coordinate(),
            (AuthorizationProtectedObjectKind::Channel, object_key)
        );
        assert_eq!(advance.objects()[0].epoch_and_fence().0, 2);
        let (_, deltas) = advance.into_parts();
        let receipt = AuthorizationOperationReceipt::new(
            domain,
            new_operation,
            request_fingerprint,
            // The seam itself is operation-kind neutral; use kind 11 here so
            // This focused test does not need to fabricate lifecycle history.
            // The lifecycle integration test supplies the kind-9 history row.
            AuthorizationOperationKind::ProtectedMutation,
            actor,
            AuthorizationOperationOutcome::Applied,
            [62_u8; 32],
        )
        .expect("authority receipt");
        record_authorization_operation_receipt_tx(&mut authority_tx, &receipt)
            .await
            .expect("record authority receipt");
        record_authorization_operation_version_delta_tx(
            &mut authority_tx,
            domain,
            new_operation,
            request_fingerprint,
            deltas,
        )
        .await
        .expect("record authority delta");
        authority_tx
            .commit()
            .await
            .expect("commit authority advance");
        let coupled: (i64, Vec<u8>, i64, Vec<u8>) = sqlx::query_as(
            "SELECT epoch.authority_epoch,epoch.fence,protected.authority_epoch,protected.fence \
             FROM authorization_authority_epochs epoch JOIN protected_object_authority protected \
               USING (community_id,object_kind,object_key) \
             WHERE epoch.community_id=$1 AND epoch.object_kind=2 AND epoch.object_key=$2",
        )
        .bind(community)
        .bind(object_key.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read coupled authority");
        assert_eq!(coupled.0, 2);
        assert_eq!(coupled.2, 2);
        assert_eq!(coupled.1, coupled.3);

        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn supersession_proof_requires_one_complete_contiguous_manifest_chain() {
        let admin_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| loopback_test_database_url());
        let admin = PgPool::connect(&admin_url).await.expect("connect admin");
        let name = format!("s5_supersession_{}", Uuid::new_v4().simple());
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
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community)
            .bind(format!("supersession-{}.example", community.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        let mut seed = pool.acquire().await.expect("seed connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *seed)
            .await
            .expect("disable seed triggers");
        for revision in 1..=3 {
            insert_policy(&mut seed, community, revision).await;
        }
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *seed)
            .await
            .expect("restore seed triggers");
        drop(seed);

        let domain = CommunityId::from_uuid(community);
        let key = authorization_version_policy_component_key(domain);
        let operations = [Uuid::new_v4(), Uuid::new_v4()];
        let requests = [[21_u8; 32], [22_u8; 32]];
        for (index, (operation, request)) in operations
            .iter()
            .copied()
            .zip(requests.iter().copied())
            .enumerate()
        {
            let mut transaction = pool.begin().await.expect("begin causal operation");
            let actor = resolve_local_admission_loss_actor_tx(
                &mut transaction,
                domain,
                LocalAdmissionLossCause::Policy { policy_revision: 3 },
            )
            .await
            .expect("resolve policy actor");
            let delta = AuthorizationOperationVersionDelta::new(
                AuthorizationVersionComponentKind::Policy,
                key,
                index as u64 + 1,
                index as u64 + 2,
            )
            .expect("strict causal delta");
            record_authorization_operation_version_delta_tx(
                &mut transaction,
                domain,
                operation,
                request,
                vec![delta],
            )
            .await
            .expect("record causal manifest");
            let receipt = AuthorizationOperationReceipt::new(
                domain,
                operation,
                request,
                AuthorizationOperationKind::ProtectedMutation,
                actor,
                AuthorizationOperationOutcome::Applied,
                [u8::try_from(31 + index).expect("small"); 32],
            )
            .expect("causal receipt");
            record_authorization_operation_receipt_tx(&mut transaction, &receipt)
                .await
                .expect("record causal receipt");
            transaction.commit().await.expect("commit causal operation");
        }

        let db = Db::from_pool(pool.clone());
        let latest = db
            .authorization_operation_version_delta(domain, operations[1], requests[1])
            .await
            .expect("load latest manifest");
        let latest_component = latest.components().first().expect("latest component");
        db.authorization_version_proves_component_supersession(
            domain,
            AuthorizationVersionComponentKind::Policy,
            key,
            1,
            3,
            operations[1],
            requests[1],
            latest_component.component_digest(),
            latest.manifest_digest(),
        )
        .await
        .expect("two-step exact provenance proves supersession");

        let branch_operation = Uuid::new_v4();
        let branch_request = [23_u8; 32];
        let mut branch = pool.begin().await.expect("begin overlapping branch");
        let branch_actor = resolve_local_admission_loss_actor_tx(
            &mut branch,
            domain,
            LocalAdmissionLossCause::Policy { policy_revision: 3 },
        )
        .await
        .expect("resolve branch actor");
        record_authorization_operation_version_delta_tx(
            &mut branch,
            domain,
            branch_operation,
            branch_request,
            vec![AuthorizationOperationVersionDelta::new(
                AuthorizationVersionComponentKind::Policy,
                key,
                1,
                3,
            )
            .expect("overlapping branch delta")],
        )
        .await
        .expect("record overlapping manifest");
        let branch_receipt = AuthorizationOperationReceipt::new(
            domain,
            branch_operation,
            branch_request,
            AuthorizationOperationKind::ProtectedMutation,
            branch_actor,
            AuthorizationOperationOutcome::Applied,
            [33_u8; 32],
        )
        .expect("branch receipt");
        record_authorization_operation_receipt_tx(&mut branch, &branch_receipt)
            .await
            .expect("record branch receipt");
        branch.commit().await.expect("commit overlapping branch");
        assert!(db
            .authorization_version_proves_component_supersession(
                domain,
                AuthorizationVersionComponentKind::Policy,
                key,
                1,
                3,
                operations[1],
                requests[1],
                latest_component.component_digest(),
                latest.manifest_digest(),
            )
            .await
            .is_err());

        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
        admin.close().await;
    }

    async fn insert_policy(connection: &mut PgConnection, community: Uuid, revision: i64) {
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,$2,1,$3,clock_timestamp() - INTERVAL '1 second')",
        )
        .bind(community)
        .bind(revision)
        .bind(vec![revision as u8; 32])
        .execute(&mut *connection)
        .await
        .expect("insert policy");
    }

    async fn insert_binding(connection: &mut PgConnection, community: Uuid, ordinal: u8) {
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,$3,$4,$5,$6,1,1,1,$7,$8,$9,$10,$11)",
        )
        .bind(community)
        .bind(Uuid::from_u128(u128::from(ordinal) + 100))
        .bind(format!("issuer-{ordinal}"))
        .bind(format!("subject-{ordinal}"))
        .bind(vec![ordinal; 32])
        .bind(vec![ordinal.saturating_add(10); 32])
        .bind(i64::from(ordinal))
        .bind(vec![ordinal.saturating_add(20); 32])
        .bind(Uuid::from_u128(u128::from(ordinal) + 200))
        .bind(Uuid::from_u128(u128::from(ordinal) + 300))
        .bind(vec![ordinal.saturating_add(30); 32])
        .execute(&mut *connection)
        .await
        .expect("insert binding");
    }
}
