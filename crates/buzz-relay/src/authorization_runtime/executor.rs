//! Transaction-owned protected mutation execution.
//!
//! A sealed permit is derived only from finalized enforcing authority. The
//! executor locks the durable invalidation and binding authorities, validates
//! expiry with the database clock, and commits the mutation and idempotency
//! receipt in one PostgreSQL transaction.

use std::{fmt, sync::Arc};

use buzz_auth::{AuthorizationCapability, FederatedAuthorization};
use buzz_core::CommunityId;
use buzz_db::authorization_invalidation::AuthorizationSelectorKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use uuid::Uuid;

use super::transport::{
    LeaseCurrentStateError, ProtectedOperationAuthority, ProtectedTransportError,
};

const MAX_RESULT_BYTES: usize = 65_536;
const RESULT_VERSION: i16 = 1;

/// One exact durable invalidation dependency carried to the commit boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct CommitDependency {
    kind: AuthorizationSelectorKind,
    fingerprint: [u8; 32],
    binding_version: Option<u64>,
}

impl CommitDependency {
    /// Preserve a dependency produced by the trusted invalidation runtime.
    pub fn from_trusted_runtime(
        kind: AuthorizationSelectorKind,
        fingerprint: [u8; 32],
        binding_version: Option<u64>,
    ) -> Result<Self, AuthorizationExecutionError> {
        if (kind == AuthorizationSelectorKind::Binding) != binding_version.is_some()
            || binding_version == Some(0)
        {
            return Err(AuthorizationExecutionError::InvalidCommitFence);
        }
        Ok(Self {
            kind,
            fingerprint,
            binding_version,
        })
    }
}

impl fmt::Debug for CommitDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommitDependency")
            .field("kind", &self.kind)
            .field("fingerprint", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .finish()
    }
}

/// Captured generation and complete selector set for one finalized authority.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationCommitFence {
    evaluation_generation: u64,
    dependencies: Vec<CommitDependency>,
}

impl AuthorizationCommitFence {
    /// Construct a commit fence from trusted finalization/invalidation state.
    pub fn from_trusted_runtime(
        evaluation_generation: u64,
        mut dependencies: Vec<CommitDependency>,
    ) -> Result<Self, AuthorizationExecutionError> {
        if dependencies.is_empty() {
            return Err(AuthorizationExecutionError::InvalidCommitFence);
        }
        dependencies.sort_by_key(|dependency| (dependency.kind, dependency.fingerprint));
        dependencies.dedup();
        let has_domain = dependencies
            .iter()
            .any(|dependency| dependency.kind == AuthorizationSelectorKind::Domain);
        let has_binding = dependencies
            .iter()
            .any(|dependency| dependency.kind == AuthorizationSelectorKind::Binding);
        let has_policy = dependencies
            .iter()
            .any(|dependency| dependency.kind == AuthorizationSelectorKind::PolicyVersion);
        if !(has_domain && has_binding && has_policy) {
            return Err(AuthorizationExecutionError::InvalidCommitFence);
        }
        Ok(Self {
            evaluation_generation,
            dependencies,
        })
    }

    /// Construct a binding-independent fence for direct first enrollment.
    pub fn from_trusted_enrollment_runtime(
        evaluation_generation: u64,
        mut dependencies: Vec<CommitDependency>,
    ) -> Result<Self, AuthorizationExecutionError> {
        if dependencies.is_empty() {
            return Err(AuthorizationExecutionError::InvalidCommitFence);
        }
        dependencies.sort_by_key(|dependency| (dependency.kind, dependency.fingerprint));
        dependencies.dedup();
        let has = |kind| {
            dependencies
                .iter()
                .any(|dependency| dependency.kind == kind)
        };
        if !has(AuthorizationSelectorKind::Domain)
            || !has(AuthorizationSelectorKind::PrincipalFingerprint)
            || !has(AuthorizationSelectorKind::NostrKey)
            || !has(AuthorizationSelectorKind::PolicyVersion)
            || has(AuthorizationSelectorKind::Binding)
        {
            return Err(AuthorizationExecutionError::InvalidCommitFence);
        }
        Ok(Self {
            evaluation_generation,
            dependencies,
        })
    }
}

impl fmt::Debug for AuthorizationCommitFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationCommitFence")
            .field("evaluation_generation", &self.evaluation_generation)
            .field("dependency_count", &self.dependencies.len())
            .finish()
    }
}

/// Signed Redis-safe snapshot of one finalized ephemeral sender authority.
///
/// It contains only opaque fingerprints and stable binding identifiers. Raw
/// issuer, subject, profile, and policy values never enter the fan-out wire.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct EphemeralAuthorityClaim {
    pub(crate) community_id: Uuid,
    pub(crate) event_id: [u8; 32],
    pub(crate) actor_pubkey: [u8; 32],
    pub(crate) bound_pubkey: [u8; 32],
    pub(crate) binding_id: Uuid,
    pub(crate) binding_version: u64,
    pub(crate) expires_at: u64,
    pub(crate) capability: String,
    evaluation_generation: u64,
    dependencies: Vec<EphemeralCommitDependency>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EphemeralCommitDependency {
    kind: String,
    fingerprint: [u8; 32],
    binding_version: Option<u64>,
}

impl EphemeralAuthorityClaim {
    pub(super) fn from_authority(
        authority: &ProtectedOperationAuthority,
        event_id: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        authority.revalidate()?;
        let capability = ephemeral_capability_label(authority.capability())
            .ok_or(ProtectedTransportError::SurfaceCapabilityMismatch)?;
        let context = authority.context();
        let lease = context
            .authorization_lease()
            .ok_or(ProtectedTransportError::MissingAccessLease)?;
        let binding = match context.federated_authorization() {
            FederatedAuthorization::Direct { binding, .. } => binding,
            FederatedAuthorization::Delegated { owner, .. } => owner,
            FederatedAuthorization::NotRequired => {
                return Err(ProtectedTransportError::MissingAccessLease)
            }
        };
        let fence = authority
            .observer()
            .observe_commit_fence()
            .map_err(ProtectedTransportError::CurrentState)?;
        Ok(Self {
            community_id: *context.tenant().community().as_uuid(),
            event_id,
            actor_pubkey: context.pubkey().to_bytes(),
            bound_pubkey: binding.bound_pubkey().to_bytes(),
            binding_id: binding.binding_id(),
            binding_version: binding.binding_version().get(),
            expires_at: lease.expires_at(),
            capability: capability.to_owned(),
            evaluation_generation: fence.evaluation_generation,
            dependencies: fence
                .dependencies
                .iter()
                .map(|dependency| EphemeralCommitDependency {
                    kind: dependency.kind.as_str().to_owned(),
                    fingerprint: dependency.fingerprint,
                    binding_version: dependency.binding_version,
                })
                .collect(),
        })
    }
}

/// Recheck a signed ephemeral claim against the writer database immediately
/// before a remote node releases it to a socket.
pub(crate) async fn revalidate_ephemeral_claim(
    db: &buzz_db::Db,
    claim: &EphemeralAuthorityClaim,
) -> Result<(), AuthorizationExecutionError> {
    if claim.binding_version == 0
        || claim.dependencies.is_empty()
        || !matches!(claim.capability.as_str(), "community_write" | "audio_join")
    {
        return Err(AuthorizationExecutionError::InvalidCommitFence);
    }
    let mut transaction = db.begin_transaction().await?;
    let generation: Option<i64> = sqlx::query_scalar(
        "SELECT generation FROM authorization_invalidation_domains \
         WHERE community_id = $1 FOR SHARE",
    )
    .bind(claim.community_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let generation = generation.ok_or(AuthorizationExecutionError::Invalidated)?;
    if generation < 0 || (generation as u64) < claim.evaluation_generation {
        return Err(AuthorizationExecutionError::Invalidated);
    }
    for dependency in &claim.dependencies {
        let row = sqlx::query(
            "SELECT generation, sticky_deny, binding_version_floor \
             FROM authorization_invalidation_floors \
             WHERE community_id = $1 AND selector_kind = $2 \
               AND selector_fingerprint = $3 FOR SHARE",
        )
        .bind(claim.community_id)
        .bind(&dependency.kind)
        .bind(dependency.fingerprint.as_slice())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else { continue };
        let floor_generation: i64 = row.try_get("generation")?;
        let sticky_deny: bool = row.try_get("sticky_deny")?;
        let binding_floor: Option<i64> = row.try_get("binding_version_floor")?;
        let fenced_after_evaluation =
            floor_generation < 0 || floor_generation as u64 > claim.evaluation_generation;
        let binding_denied = match (binding_floor, dependency.binding_version) {
            (Some(floor), Some(version)) => floor < 0 || version <= floor as u64,
            _ => false,
        };
        if sticky_deny || fenced_after_evaluation || binding_denied {
            return Err(AuthorizationExecutionError::Invalidated);
        }
    }
    let version = i64::try_from(claim.binding_version)
        .map_err(|_| AuthorizationExecutionError::InvalidBinding)?;
    let active: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM identity_bindings \
         WHERE community_id = $1 AND binding_id = $2 AND pubkey = $3 \
           AND binding_version = $4 AND binding_state = 'active' FOR SHARE",
    )
    .bind(claim.community_id)
    .bind(claim.binding_id)
    .bind(claim.bound_pubkey.as_slice())
    .bind(version)
    .fetch_optional(&mut *transaction)
    .await?;
    if active.is_none() {
        return Err(AuthorizationExecutionError::InvalidBinding);
    }
    ensure_not_expired(&mut transaction, claim.expires_at).await?;
    transaction.commit().await?;
    Ok(())
}

/// Stable retry identity for one protected operation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtectedOperationId(Uuid);

impl ProtectedOperationId {
    /// Derive a deterministic UUID from a domain-separated stable operation key.
    pub fn derive(
        authorization_domain: CommunityId,
        operation_kind: &'static str,
        stable_key: &[u8],
    ) -> Result<Self, AuthorizationExecutionError> {
        if operation_kind.is_empty() || operation_kind.len() > 128 || stable_key.is_empty() {
            return Err(AuthorizationExecutionError::InvalidOperationIdentity);
        }
        let mut digest = Sha256::new();
        digest.update(b"buzz-protected-operation-id-v1");
        digest.update(authorization_domain.as_uuid().as_bytes());
        digest.update((operation_kind.len() as u64).to_be_bytes());
        digest.update(operation_kind.as_bytes());
        digest.update((stable_key.len() as u64).to_be_bytes());
        digest.update(stable_key);
        let digest: [u8; 32] = digest.finalize().into();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        // Mark the digest-derived value as an RFC 4122 variant/version-5 UUID.
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Self(Uuid::from_bytes(bytes)))
    }

    pub(crate) const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Debug for ProtectedOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedOperationId([redacted])")
    }
}

/// Permit sealed from a finalized enforcing authorization context.
pub struct SealedOperationPermit {
    community_id: CommunityId,
    operation_id: ProtectedOperationId,
    operation_kind: &'static str,
    request_fingerprint: [u8; 32],
    actor_pubkey: [u8; 32],
    bound_pubkey: [u8; 32],
    binding_id: Uuid,
    binding_version: u64,
    issuer: String,
    subject: String,
    expires_at: u64,
    fence: AuthorizationCommitFence,
}

impl SealedOperationPermit {
    pub(super) fn from_authority(
        authority: &ProtectedOperationAuthority,
        operation_id: ProtectedOperationId,
        operation_kind: &'static str,
        request_fingerprint: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        authority.revalidate()?;
        let context = authority.context();
        let lease = context
            .authorization_lease()
            .ok_or(ProtectedTransportError::MissingAccessLease)?;
        let binding = match context.federated_authorization() {
            FederatedAuthorization::Direct { binding, .. } => binding,
            FederatedAuthorization::Delegated { owner, .. } => owner,
            FederatedAuthorization::NotRequired => {
                return Err(ProtectedTransportError::MissingAccessLease)
            }
        };
        let fence = authority
            .observer()
            .observe_commit_fence()
            .map_err(ProtectedTransportError::CurrentState)?;

        let mut digest = Sha256::new();
        digest.update(b"buzz-protected-operation-request-v1");
        digest.update(context.tenant().community().as_uuid().as_bytes());
        digest.update(operation_id.as_uuid().as_bytes());
        digest.update((operation_kind.len() as u64).to_be_bytes());
        digest.update(operation_kind.as_bytes());
        let capability = capability_label(authority.capability())
            .ok_or(ProtectedTransportError::SurfaceCapabilityMismatch)?;
        digest.update(capability.as_bytes());
        digest.update(context.pubkey().to_bytes());
        if let Some(owner) = context.agent_owner_pubkey() {
            digest.update(owner.to_bytes());
        }
        digest.update(lease.binding_id().as_bytes());
        digest.update(lease.binding_version().get().to_be_bytes());
        digest.update(lease.profile_id().as_str().as_bytes());
        digest.update(lease.policy_version().as_str().as_bytes());
        digest.update(lease.lease_version().get().to_be_bytes());
        digest.update(lease.expires_at().to_be_bytes());
        digest.update(request_fingerprint);

        Ok(Self {
            community_id: context.tenant().community(),
            operation_id,
            operation_kind,
            request_fingerprint: digest.finalize().into(),
            actor_pubkey: context.pubkey().to_bytes(),
            bound_pubkey: binding.bound_pubkey().to_bytes(),
            binding_id: binding.binding_id(),
            binding_version: binding.binding_version().get(),
            issuer: binding.principal().issuer().to_owned(),
            subject: binding.principal().subject().to_owned(),
            expires_at: lease.expires_at(),
            fence,
        })
    }
}

impl fmt::Debug for SealedOperationPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedOperationPermit")
            .field("community_id", &"[redacted]")
            .field("operation_id", &"[redacted]")
            .field("operation_kind", &self.operation_kind)
            .field("authority", &"[redacted]")
            .finish()
    }
}

/// Binding-independent permit for one atomic direct first enrollment.
pub struct SealedEnrollmentPermit {
    community_id: CommunityId,
    operation_id: ProtectedOperationId,
    operation_kind: &'static str,
    request_fingerprint: [u8; 32],
    actor_pubkey: [u8; 32],
    issuer: String,
    subject: String,
    expires_at: u64,
    fence: AuthorizationCommitFence,
}

impl SealedEnrollmentPermit {
    pub(super) fn from_authority(
        authority: &super::transport::ProtectedEnrollmentAuthority,
        operation_id: ProtectedOperationId,
        operation_kind: &'static str,
        request_fingerprint: [u8; 32],
    ) -> Result<Self, ProtectedTransportError> {
        authority.revalidate()?;
        let disposition = authority.disposition();
        let fence = authority
            .observer()
            .observe_commit_fence()
            .map_err(ProtectedTransportError::CurrentState)?;
        let mut digest = Sha256::new();
        digest.update(b"buzz-protected-enrollment-request-v1");
        digest.update(disposition.authorization_domain().as_uuid().as_bytes());
        digest.update(operation_id.as_uuid().as_bytes());
        digest.update(operation_kind.as_bytes());
        digest.update(disposition.actor_pubkey().to_bytes());
        digest.update(disposition.principal().issuer().as_bytes());
        digest.update(disposition.principal().subject().as_bytes());
        digest.update(disposition.profile_id().as_str().as_bytes());
        digest.update(disposition.policy_version().as_str().as_bytes());
        digest.update(disposition.expires_at().to_be_bytes());
        digest.update(request_fingerprint);
        Ok(Self {
            community_id: disposition.authorization_domain(),
            operation_id,
            operation_kind,
            request_fingerprint: digest.finalize().into(),
            actor_pubkey: disposition.actor_pubkey().to_bytes(),
            issuer: disposition.principal().issuer().to_owned(),
            subject: disposition.principal().subject().to_owned(),
            expires_at: disposition.expires_at(),
            fence,
        })
    }
}

/// Start result for an idempotent protected operation.
pub enum AuthorizedOperationStart {
    /// The same operation already committed; return this original typed payload.
    Replay(Vec<u8>),
    /// The caller owns the only transaction in which effects may be written.
    Execute(Box<AuthorizedOperation>),
}

/// Start result for an atomic direct first enrollment.
pub enum AuthorizedEnrollmentStart {
    /// The exact enrollment already committed.
    Replay(Vec<u8>),
    /// The caller owns the sole enrollment transaction.
    Execute(Box<AuthorizedEnrollmentOperation>),
}

/// Open binding-independent enrollment transaction.
pub struct AuthorizedEnrollmentOperation {
    transaction: Transaction<'static, Postgres>,
    permit: SealedEnrollmentPermit,
    restore: Arc<super::restore::RestoreProtectionRuntime>,
}

impl AuthorizedEnrollmentOperation {
    /// Transaction used to bind identity, consume the invite, and add membership.
    pub fn transaction(&mut self) -> &mut Transaction<'static, Postgres> {
        &mut self.transaction
    }

    /// Direct actor key bound by the staged assertion and Nostr proof.
    pub const fn actor_pubkey(&self) -> &[u8; 32] {
        &self.permit.actor_pubkey
    }

    /// Literal validated issuer.
    pub fn issuer(&self) -> &str {
        &self.permit.issuer
    }

    /// Literal validated subject.
    pub fn subject(&self) -> &str {
        &self.permit.subject
    }

    /// Commit the enrollment and its bounded idempotency result together.
    pub async fn commit(
        mut self,
        result_payload: &[u8],
    ) -> Result<Vec<u8>, AuthorizationExecutionError> {
        if result_payload.len() > MAX_RESULT_BYTES {
            return Err(AuthorizationExecutionError::ResultTooLarge);
        }
        ensure_not_expired(&mut self.transaction, self.permit.expires_at).await?;
        insert_receipt(
            &mut self.transaction,
            self.permit.community_id,
            self.permit.operation_id,
            self.permit.operation_kind,
            &self.permit.request_fingerprint,
            self.permit.expires_at,
            result_payload,
        )
        .await?;
        let witness = self
            .restore
            .begin(
                self.permit.community_id,
                self.permit.operation_id.as_uuid(),
                self.permit.request_fingerprint,
            )
            .await?;
        let commit_result = self.transaction.commit().await;
        let witness_result = witness.commit().await;
        if let Err(error) = witness_result {
            return Err(error.into());
        }
        if let Err(error) = commit_result {
            // RestoreMutationGuard::commit returned success only after reading
            // the exact durable operation receipt back from PostgreSQL. Never
            // turn a bare failed commit acknowledgement into success.
            tracing::warn!(%error, "protected enrollment commit acknowledgement was ambiguous; exact durable receipt verified");
        }
        Ok(result_payload.to_vec())
    }
}

/// Open transaction that has validated and locked its authorization authority.
pub struct AuthorizedOperation {
    transaction: Transaction<'static, Postgres>,
    permit: SealedOperationPermit,
    restore: Option<Arc<super::restore::RestoreProtectionRuntime>>,
}

impl AuthorizedOperation {
    /// Transaction to pass to transaction-aware mutation APIs.
    pub fn transaction(&mut self) -> &mut Transaction<'static, Postgres> {
        &mut self.transaction
    }

    /// Atomically commit a bounded replay result with all transaction effects.
    pub async fn commit(
        mut self,
        result_payload: &[u8],
    ) -> Result<Vec<u8>, AuthorizationExecutionError> {
        if result_payload.len() > MAX_RESULT_BYTES {
            return Err(AuthorizationExecutionError::ResultTooLarge);
        }
        ensure_not_expired(&mut self.transaction, self.permit.expires_at).await?;
        insert_receipt(
            &mut self.transaction,
            self.permit.community_id,
            self.permit.operation_id,
            self.permit.operation_kind,
            &self.permit.request_fingerprint,
            self.permit.expires_at,
            result_payload,
        )
        .await?;
        let witness = match &self.restore {
            Some(restore) => Some(
                restore
                    .begin(
                        self.permit.community_id,
                        self.permit.operation_id.as_uuid(),
                        self.permit.request_fingerprint,
                    )
                    .await?,
            ),
            #[cfg(test)]
            None => None,
            #[cfg(not(test))]
            None => return Err(AuthorizationExecutionError::RestoreUnavailable),
        };
        let commit_result = self.transaction.commit().await;
        if let Some(witness) = witness {
            witness.commit().await?;
            if let Err(error) = commit_result {
                // A successful witness commit proves that the exact receipt is
                // durable. Without that proof the error above is returned.
                tracing::warn!(%error, "protected operation commit acknowledgement was ambiguous; exact durable receipt verified");
            }
        } else {
            // Only test-only construction can omit restore protection. Keep
            // even that path honest so a failed database commit is never
            // reported as a successful protected effect.
            commit_result?;
        }
        Ok(result_payload.to_vec())
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_receipt(
    transaction: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    operation_id: ProtectedOperationId,
    operation_kind: &'static str,
    request_fingerprint: &[u8; 32],
    expires_at: u64,
    result_payload: &[u8],
) -> Result<(), AuthorizationExecutionError> {
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, operation_kind, request_fingerprint, \
          result_version, result_payload, lease_expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, to_timestamp($7::double precision))",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id.as_uuid())
    .bind(operation_kind)
    .bind(request_fingerprint.as_slice())
    .bind(RESULT_VERSION)
    .bind(result_payload)
    .bind(expires_at as f64)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Begin a transaction-owned protected mutation or replay its original result.
pub async fn begin_authorized_operation(
    state: &crate::state::AppState,
    permit: SealedOperationPermit,
) -> Result<AuthorizedOperationStart, AuthorizationExecutionError> {
    let restore = state
        .restore_protection()
        .ok_or(AuthorizationExecutionError::RestoreUnavailable)?;
    begin_authorized_operation_inner(&state.db, Some(Arc::clone(restore)), permit).await
}

async fn begin_authorized_operation_inner(
    db: &buzz_db::Db,
    restore: Option<Arc<super::restore::RestoreProtectionRuntime>>,
    permit: SealedOperationPermit,
) -> Result<AuthorizedOperationStart, AuthorizationExecutionError> {
    let mut transaction = db.begin_transaction().await?;

    // Serialize identical retries before reading the receipt. This avoids two
    // first attempts executing the mutation body concurrently.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(permit.operation_id.as_uuid().to_string())
        .execute(&mut *transaction)
        .await?;

    let committed_receipt = if let Some(row) = sqlx::query(
        "SELECT operation_kind, request_fingerprint, result_version, result_payload \
         FROM authorization_operation_receipts \
         WHERE community_id = $1 AND operation_id = $2 FOR SHARE",
    )
    .bind(permit.community_id.as_uuid())
    .bind(permit.operation_id.as_uuid())
    .fetch_optional(&mut *transaction)
    .await?
    {
        let operation_kind: String = row.try_get("operation_kind")?;
        let request_fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
        let result_version: i16 = row.try_get("result_version")?;
        let result_payload: Vec<u8> = row.try_get("result_payload")?;
        if operation_kind != permit.operation_kind
            || request_fingerprint.as_slice() != permit.request_fingerprint
            || result_version != RESULT_VERSION
        {
            return Err(AuthorizationExecutionError::ConflictingRetry);
        }
        Some(result_payload)
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO authorization_invalidation_domains (community_id) \
         VALUES ($1) ON CONFLICT (community_id) DO NOTHING",
    )
    .bind(permit.community_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    let generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM authorization_invalidation_domains \
         WHERE community_id = $1 FOR SHARE",
    )
    .bind(permit.community_id.as_uuid())
    .fetch_one(&mut *transaction)
    .await?;
    if generation < 0 || (generation as u64) < permit.fence.evaluation_generation {
        return Err(AuthorizationExecutionError::Invalidated);
    }

    validate_dependency_floors(&mut transaction, permit.community_id, &permit.fence).await?;
    validate_active_binding(&mut transaction, &permit).await?;
    ensure_not_expired(&mut transaction, permit.expires_at).await?;

    if let Some(result_payload) = committed_receipt {
        transaction.commit().await?;
        return Ok(AuthorizedOperationStart::Replay(result_payload));
    }

    Ok(AuthorizedOperationStart::Execute(Box::new(
        AuthorizedOperation {
            transaction,
            permit,
            restore,
        },
    )))
}

#[cfg(test)]
async fn begin_authorized_operation_for_test(
    db: &buzz_db::Db,
    permit: SealedOperationPermit,
) -> Result<AuthorizedOperationStart, AuthorizationExecutionError> {
    begin_authorized_operation_inner(db, None, permit).await
}

/// Begin a binding-independent transaction that atomically creates first enrollment.
pub async fn begin_authorized_enrollment(
    state: &crate::state::AppState,
    permit: SealedEnrollmentPermit,
) -> Result<AuthorizedEnrollmentStart, AuthorizationExecutionError> {
    let restore = state
        .restore_protection()
        .ok_or(AuthorizationExecutionError::RestoreUnavailable)?;
    let mut transaction = state.db.begin_transaction().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(permit.operation_id.as_uuid().to_string())
        .execute(&mut *transaction)
        .await?;

    let committed_receipt = if let Some(row) = sqlx::query(
        "SELECT operation_kind, request_fingerprint, result_version, result_payload \
         FROM authorization_operation_receipts \
         WHERE community_id = $1 AND operation_id = $2 FOR SHARE",
    )
    .bind(permit.community_id.as_uuid())
    .bind(permit.operation_id.as_uuid())
    .fetch_optional(&mut *transaction)
    .await?
    {
        let operation_kind: String = row.try_get("operation_kind")?;
        let request_fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
        let result_version: i16 = row.try_get("result_version")?;
        let result_payload: Vec<u8> = row.try_get("result_payload")?;
        if operation_kind != permit.operation_kind
            || request_fingerprint.as_slice() != permit.request_fingerprint
            || result_version != RESULT_VERSION
        {
            return Err(AuthorizationExecutionError::ConflictingRetry);
        }
        Some(result_payload)
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO authorization_invalidation_domains (community_id) \
         VALUES ($1) ON CONFLICT (community_id) DO NOTHING",
    )
    .bind(permit.community_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    let generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM authorization_invalidation_domains \
         WHERE community_id = $1 FOR SHARE",
    )
    .bind(permit.community_id.as_uuid())
    .fetch_one(&mut *transaction)
    .await?;
    if generation < 0 || (generation as u64) < permit.fence.evaluation_generation {
        return Err(AuthorizationExecutionError::Invalidated);
    }
    validate_dependency_floors(&mut transaction, permit.community_id, &permit.fence).await?;
    ensure_not_expired(&mut transaction, permit.expires_at).await?;

    if let Some(result_payload) = committed_receipt {
        transaction.commit().await?;
        return Ok(AuthorizedEnrollmentStart::Replay(result_payload));
    }
    Ok(AuthorizedEnrollmentStart::Execute(Box::new(
        AuthorizedEnrollmentOperation {
            transaction,
            permit,
            restore: Arc::clone(restore),
        },
    )))
}

async fn validate_dependency_floors(
    transaction: &mut Transaction<'static, Postgres>,
    community_id: CommunityId,
    fence: &AuthorizationCommitFence,
) -> Result<(), AuthorizationExecutionError> {
    for dependency in &fence.dependencies {
        let row = sqlx::query(
            "SELECT generation, sticky_deny, binding_version_floor \
             FROM authorization_invalidation_floors \
             WHERE community_id = $1 AND selector_kind = $2 \
               AND selector_fingerprint = $3",
        )
        .bind(community_id.as_uuid())
        .bind(dependency.kind.as_str())
        .bind(dependency.fingerprint.as_slice())
        .fetch_optional(&mut **transaction)
        .await?;
        let Some(row) = row else { continue };
        let generation: i64 = row.try_get("generation")?;
        let sticky_deny: bool = row.try_get("sticky_deny")?;
        let binding_floor: Option<i64> = row.try_get("binding_version_floor")?;
        let fenced_after_evaluation =
            generation < 0 || generation as u64 > fence.evaluation_generation;
        let binding_denied = match (binding_floor, dependency.binding_version) {
            (Some(floor), Some(version)) => floor < 0 || version <= floor as u64,
            _ => false,
        };
        if sticky_deny || fenced_after_evaluation || binding_denied {
            return Err(AuthorizationExecutionError::Invalidated);
        }
    }
    Ok(())
}

async fn validate_active_binding(
    transaction: &mut Transaction<'static, Postgres>,
    permit: &SealedOperationPermit,
) -> Result<(), AuthorizationExecutionError> {
    let version = i64::try_from(permit.binding_version)
        .map_err(|_| AuthorizationExecutionError::InvalidBinding)?;
    let active: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM identity_bindings \
         WHERE community_id = $1 AND binding_id = $2 \
           AND issuer = $3 AND uid = $4 AND pubkey = $5 \
           AND binding_version = $6 AND binding_state = 'active' \
         FOR SHARE",
    )
    .bind(permit.community_id.as_uuid())
    .bind(permit.binding_id)
    .bind(&permit.issuer)
    .bind(&permit.subject)
    .bind(permit.bound_pubkey.as_slice())
    .bind(version)
    .fetch_optional(&mut **transaction)
    .await?;
    if active.is_none() {
        return Err(AuthorizationExecutionError::InvalidBinding);
    }
    // The actor is included in the sealed fingerprint even when delegated;
    // direct/owner agreement was already enforced by AuthContext finalization.
    let _ = permit.actor_pubkey;
    Ok(())
}

async fn ensure_not_expired(
    transaction: &mut Transaction<'static, Postgres>,
    expires_at: u64,
) -> Result<(), AuthorizationExecutionError> {
    let current: f64 =
        sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM clock_timestamp())::double precision")
            .fetch_one(&mut **transaction)
            .await?;
    if !current.is_finite() || current >= expires_at as f64 {
        return Err(AuthorizationExecutionError::Expired);
    }
    Ok(())
}

const fn capability_label(capability: AuthorizationCapability) -> Option<&'static str> {
    match capability {
        AuthorizationCapability::CommunityRead => Some("community_read"),
        AuthorizationCapability::CommunityWrite => Some("community_write"),
        AuthorizationCapability::Moderate => Some("moderate"),
        AuthorizationCapability::InviteMint => Some("invite_mint"),
        AuthorizationCapability::InviteClaim => Some("invite_claim"),
        AuthorizationCapability::MediaRead => Some("media_read"),
        AuthorizationCapability::MediaWrite => Some("media_write"),
        AuthorizationCapability::GitRead => Some("git_read"),
        AuthorizationCapability::GitWrite => Some("git_write"),
        AuthorizationCapability::AudioJoin => Some("audio_join"),
        _ => None,
    }
}

const fn ephemeral_capability_label(capability: AuthorizationCapability) -> Option<&'static str> {
    match capability {
        AuthorizationCapability::CommunityWrite => Some("community_write"),
        AuthorizationCapability::AudioJoin => Some("audio_join"),
        _ => None,
    }
}

/// Fail-closed protected mutation execution error.
#[derive(Debug, Error)]
pub enum AuthorizationExecutionError {
    /// Database operation failed.
    #[error("protected operation database transaction failed")]
    Database(#[from] sqlx::Error),
    /// Database wrapper failed before the transaction began.
    #[error("protected operation database transaction failed")]
    Db(#[from] buzz_db::DbError),
    /// Independent restore witness was unavailable or rejected reconciliation.
    #[error("protected operation restore witness failed")]
    Restore(#[from] super::restore::RestoreProtectionError),
    /// Production execution was attempted without the mandatory witness.
    #[error("protected operation restore witness is unavailable")]
    RestoreUnavailable,
    /// Captured generation or dependency set was incomplete.
    #[error("protected operation commit fence is invalid")]
    InvalidCommitFence,
    /// Stable operation identity was empty or malformed.
    #[error("protected operation identity is invalid")]
    InvalidOperationIdentity,
    /// The stable operation ID was reused for different input.
    #[error("protected operation retry conflicts with the committed request")]
    ConflictingRetry,
    /// Current durable invalidation state denies the operation.
    #[error("protected operation authority was invalidated")]
    Invalidated,
    /// The exact active binding changed or disappeared.
    #[error("protected operation binding is no longer active")]
    InvalidBinding,
    /// The lease expired before the commit boundary.
    #[error("protected operation authorization expired before commit")]
    Expired,
    /// Replay result exceeded the bounded receipt payload.
    #[error("protected operation result is too large")]
    ResultTooLarge,
}

impl From<AuthorizationExecutionError> for LeaseCurrentStateError {
    fn from(_error: AuthorizationExecutionError) -> Self {
        LeaseCurrentStateError::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
    const TEST_ISSUER: &str = "https://idp.example";
    const TEST_SUBJECT: &str = "executor-subject";

    fn dependency(
        kind: AuthorizationSelectorKind,
        marker: u8,
        binding_version: Option<u64>,
    ) -> CommitDependency {
        CommitDependency::from_trusted_runtime(kind, [marker; 32], binding_version)
            .expect("valid dependency")
    }

    fn operation_fence() -> AuthorizationCommitFence {
        AuthorizationCommitFence::from_trusted_runtime(
            0,
            vec![
                dependency(AuthorizationSelectorKind::Domain, 1, None),
                dependency(AuthorizationSelectorKind::Binding, 2, Some(1)),
                dependency(AuthorizationSelectorKind::PolicyVersion, 3, None),
            ],
        )
        .expect("complete operation fence")
    }

    fn epoch_after(seconds: u64) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_secs()
            + seconds
    }

    fn permit(
        community_id: CommunityId,
        binding_id: Uuid,
        operation_id: ProtectedOperationId,
        request_marker: u8,
        expires_at: u64,
    ) -> SealedOperationPermit {
        SealedOperationPermit {
            community_id,
            operation_id,
            operation_kind: "executor.test.v1",
            request_fingerprint: [request_marker; 32],
            actor_pubkey: [7; 32],
            bound_pubkey: [7; 32],
            binding_id,
            binding_version: 1,
            issuer: TEST_ISSUER.to_owned(),
            subject: TEST_SUBJECT.to_owned(),
            expires_at,
            fence: operation_fence(),
        }
    }

    async fn integration_setup() -> (buzz_db::Db, CommunityId, Uuid) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("test migrations");
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id.as_uuid())
            .bind(format!(
                "executor-{}.example",
                community_id.as_uuid().simple()
            ))
            .execute(&pool)
            .await
            .expect("test community");
        let binding_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity_bindings \
             (community_id, issuer, uid, pubkey, source, binding_id, \
              binding_version, binding_state, binding_provenance) \
             VALUES ($1, $2, $3, $4, 'jwt_npub', $5, 1, 'active', 'attested_key')",
        )
        .bind(community_id.as_uuid())
        .bind(TEST_ISSUER)
        .bind(TEST_SUBJECT)
        .bind([7_u8; 32].as_slice())
        .bind(binding_id)
        .execute(&pool)
        .await
        .expect("active test binding");
        (buzz_db::Db::from_pool(pool), community_id, binding_id)
    }

    #[test]
    fn stable_operation_identity_is_domain_and_kind_separated() {
        let first_domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let second_domain = CommunityId::from_uuid(Uuid::from_u128(2));
        let first = ProtectedOperationId::derive(first_domain, "event.ingest.v1", b"event")
            .expect("operation id");
        assert_eq!(
            first,
            ProtectedOperationId::derive(first_domain, "event.ingest.v1", b"event")
                .expect("stable operation id")
        );
        assert_ne!(
            first,
            ProtectedOperationId::derive(second_domain, "event.ingest.v1", b"event")
                .expect("domain-separated operation id")
        );
        assert_ne!(
            first,
            ProtectedOperationId::derive(first_domain, "invite.mint.v1", b"event")
                .expect("kind-separated operation id")
        );
    }

    #[test]
    fn operation_and_enrollment_fences_require_distinct_authority_sets() {
        assert!(AuthorizationCommitFence::from_trusted_runtime(
            0,
            vec![
                dependency(AuthorizationSelectorKind::Domain, 1, None),
                dependency(AuthorizationSelectorKind::PolicyVersion, 3, None),
            ],
        )
        .is_err());
        assert!(AuthorizationCommitFence::from_trusted_enrollment_runtime(
            0,
            vec![
                dependency(AuthorizationSelectorKind::Domain, 1, None),
                dependency(AuthorizationSelectorKind::PrincipalFingerprint, 2, None,),
                dependency(AuthorizationSelectorKind::NostrKey, 3, None),
                dependency(AuthorizationSelectorKind::PolicyVersion, 4, None),
                dependency(AuthorizationSelectorKind::Binding, 5, Some(1)),
            ],
        )
        .is_err());
        assert!(AuthorizationCommitFence::from_trusted_enrollment_runtime(
            0,
            vec![
                dependency(AuthorizationSelectorKind::Domain, 1, None),
                dependency(AuthorizationSelectorKind::PrincipalFingerprint, 2, None,),
                dependency(AuthorizationSelectorKind::NostrKey, 3, None),
                dependency(AuthorizationSelectorKind::PolicyVersion, 4, None),
            ],
        )
        .is_ok());
        let _ = operation_fence();
    }

    #[test]
    fn capability_receipt_labels_are_stable() {
        assert_eq!(
            capability_label(AuthorizationCapability::CommunityWrite),
            Some("community_write")
        );
        assert_eq!(
            capability_label(AuthorizationCapability::AudioJoin),
            Some("audio_join")
        );
        assert_eq!(
            ephemeral_capability_label(AuthorizationCapability::CommunityWrite),
            Some("community_write")
        );
        assert_eq!(
            ephemeral_capability_label(AuthorizationCapability::AudioJoin),
            Some("audio_join")
        );
        assert_eq!(
            ephemeral_capability_label(AuthorizationCapability::MediaWrite),
            None
        );
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn postgres_executor_serializes_retries_rolls_back_and_checks_expiry() {
        let (db, community_id, binding_id) = integration_setup().await;
        let concurrent_id =
            ProtectedOperationId::derive(community_id, "executor.test.v1", b"concurrent")
                .expect("operation id");
        let run = |permit| async {
            match begin_authorized_operation_for_test(&db, permit)
                .await
                .expect("authorized operation")
            {
                AuthorizedOperationStart::Execute(operation) => {
                    operation.commit(b"committed-once").await.expect("commit");
                    true
                }
                AuthorizedOperationStart::Replay(payload) => {
                    assert_eq!(payload, b"committed-once");
                    false
                }
            }
        };
        let (first, second) = tokio::join!(
            run(permit(
                community_id,
                binding_id,
                concurrent_id,
                1,
                epoch_after(60),
            )),
            run(permit(
                community_id,
                binding_id,
                concurrent_id,
                1,
                epoch_after(60),
            )),
        );
        assert_ne!(first, second, "exactly one concurrent retry executes");

        assert!(matches!(
            begin_authorized_operation_for_test(
                &db,
                permit(community_id, binding_id, concurrent_id, 2, epoch_after(60),),
            )
            .await,
            Err(AuthorizationExecutionError::ConflictingRetry)
        ));

        let rollback_id =
            ProtectedOperationId::derive(community_id, "executor.test.v1", b"rollback")
                .expect("operation id");
        let AuthorizedOperationStart::Execute(rolled_back) = begin_authorized_operation_for_test(
            &db,
            permit(community_id, binding_id, rollback_id, 3, epoch_after(60)),
        )
        .await
        .expect("open rollback operation") else {
            panic!("uncommitted operation cannot replay")
        };
        rolled_back
            .transaction
            .rollback()
            .await
            .expect("explicit rollback");
        assert!(matches!(
            begin_authorized_operation_for_test(
                &db,
                permit(community_id, binding_id, rollback_id, 3, epoch_after(60),),
            )
            .await
            .expect("retry after rollback"),
            AuthorizedOperationStart::Execute(_)
        ));

        let expired_id = ProtectedOperationId::derive(community_id, "executor.test.v1", b"expired")
            .expect("operation id");
        assert!(matches!(
            begin_authorized_operation_for_test(
                &db,
                permit(
                    community_id,
                    binding_id,
                    expired_id,
                    4,
                    epoch_after(0).saturating_sub(1),
                ),
            )
            .await,
            Err(AuthorizationExecutionError::Expired)
        ));

        let invalidation_id = Uuid::new_v4();
        let mut invalidation = db.begin_transaction().await.expect("invalidation tx");
        let invalidation_generation: i64 = sqlx::query_scalar(
            "UPDATE authorization_invalidation_domains \
             SET generation = generation + 1 WHERE community_id = $1 \
             RETURNING generation",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&mut *invalidation)
        .await
        .expect("advance invalidation generation");
        sqlx::query(
            "INSERT INTO authorization_invalidation_receipts \
             (community_id, event_id, generation, request_fingerprint) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(community_id.as_uuid())
        .bind(invalidation_id)
        .bind(invalidation_generation)
        .bind([9_u8; 32].as_slice())
        .execute(&mut *invalidation)
        .await
        .expect("invalidation receipt");
        sqlx::query(
            "INSERT INTO authorization_invalidation_floors \
             (community_id, selector_kind, selector_fingerprint, generation, sticky_deny) \
             VALUES ($1, 'domain', $2, $3, true)",
        )
        .bind(community_id.as_uuid())
        .bind([1_u8; 32].as_slice())
        .bind(invalidation_generation)
        .execute(&mut *invalidation)
        .await
        .expect("domain deny floor");
        invalidation.commit().await.expect("commit invalidation");

        let invalidated_id =
            ProtectedOperationId::derive(community_id, "executor.test.v1", b"invalidated")
                .expect("operation id");
        assert!(matches!(
            begin_authorized_operation_for_test(
                &db,
                permit(community_id, binding_id, invalidated_id, 5, epoch_after(60),),
            )
            .await,
            Err(AuthorizationExecutionError::Invalidated)
        ));
    }
}
