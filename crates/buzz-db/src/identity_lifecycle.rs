//! Linearizable storage transitions for relay-verified identity bindings.
//!
//! This module consumes caller-verified lifecycle facts. It does not define an
//! operator transport or authorization policy. Every transition shares the
//! authorization lock coordinates from [`crate::identity_binding`] and records
//! state history in the same transaction.

use std::fmt;

use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::identity_binding::{
    binding_lock_coordinate, key_lock_coordinate, lock_identity_coordinates_tx,
    operation_lock_coordinate, principal_lock_coordinate, BindingEvidence, BindingProvenance,
    BindingState, CreationAttributionKind, EnrollmentMode,
};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};

/// Opaque server-issued identifier for one retryable lifecycle operation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LifecycleOperationId(Uuid);

impl LifecycleOperationId {
    /// Mint a new server-controlled identifier before accepting a transition.
    pub fn issue() -> Self {
        Self(Uuid::new_v4())
    }

    /// Stable UUID retained by the server across retries.
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    #[cfg(test)]
    pub(crate) const fn from_uuid_for_test(value: Uuid) -> Self {
        Self(value)
    }
}

impl fmt::Debug for LifecycleOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LifecycleOperationId")
            .field(&"[redacted]")
            .finish()
    }
}

/// Immutable evidence common to one privileged lifecycle request.
#[derive(Clone, Copy)]
pub struct LifecycleContext<'a> {
    /// Server-issued idempotency identifier retained across retries.
    pub operation_id: LifecycleOperationId,
    /// Authenticated actor key.
    pub actor: &'a [u8],
    /// Non-empty private transition reason.
    pub reason: &'a str,
}

impl fmt::Debug for LifecycleContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LifecycleContext")
            .field("operation_id", &"[redacted]")
            .field("actor", &"[redacted]")
            .field("reason", &"[redacted]")
            .finish()
    }
}

/// Exact issuer-qualified principal in one server-resolved domain.
#[derive(Clone, Copy)]
pub struct IdentityPrincipal<'a> {
    /// Exact validated issuer.
    pub issuer: &'a str,
    /// Exact validated subject.
    pub subject: &'a str,
}

impl fmt::Debug for IdentityPrincipal<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityPrincipal")
            .field("issuer", &"[redacted]")
            .field("subject", &"[redacted]")
            .finish()
    }
}

/// Replacement-key storage facts accepted only after an in-crate verifier has
/// completed fresh proof. The fields and constructor are intentionally not
/// available to external callers; a later proof-owning boundary must supply
/// this value rather than selecting provenance or policy from raw input.
#[derive(Clone, Copy)]
pub struct VerifiedReplacementKey<'a> {
    pubkey: &'a [u8],
    display_name: Option<&'a str>,
    provenance: BindingProvenance,
    created_policy_version: &'a str,
}

impl<'a> VerifiedReplacementKey<'a> {
    /// Construct storage input after fresh replacement-key proof is verified.
    #[allow(dead_code)] // The proof-owning runtime boundary must wire this before activation.
    pub(crate) fn after_verified_proof(
        pubkey: &'a [u8],
        display_name: Option<&'a str>,
        provenance: BindingProvenance,
        created_policy_version: &'a str,
    ) -> Result<Self> {
        validate_pubkey(pubkey)?;
        if provenance == BindingProvenance::Tofu {
            return Err(DbError::InvalidData(
                "privileged replacement cannot use TOFU provenance".to_string(),
            ));
        }
        if created_policy_version.is_empty() {
            return Err(DbError::InvalidData(
                "identity policy version must not be empty".to_string(),
            ));
        }
        Ok(VerifiedReplacementKey {
            pubkey,
            display_name,
            provenance,
            created_policy_version,
        })
    }

    /// Proven replacement public key.
    pub const fn pubkey(&self) -> &[u8] {
        self.pubkey
    }
}

impl fmt::Debug for VerifiedReplacementKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedReplacementKey")
            .field("pubkey", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("provenance", &"[redacted]")
            .field("created_policy_version", &"[redacted]")
            .finish()
    }
}

/// Exact pending-replacement selector observed by a caller.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingLineage {
    /// Retired key selected by Q.
    pub retired_pubkey: Vec<u8>,
    /// Stable retired binding identifier.
    pub retired_binding_id: Uuid,
    /// Version of the retired binding referenced by Q.
    pub retired_binding_version: u64,
    /// Monotonic selector version protecting against ABA.
    pub selector_version: u64,
}

impl fmt::Debug for PendingLineage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingLineage")
            .field("retired_pubkey", &"[redacted]")
            .field("retired_binding_id", &"[redacted]")
            .field("retired_binding_version", &"[redacted]")
            .field("selector_version", &"[redacted]")
            .finish()
    }
}

/// Redaction-safe receipt for a committed lifecycle transition.
#[derive(Clone, PartialEq, Eq)]
pub struct LifecycleReceipt {
    /// Binding primarily affected by the transition.
    pub binding_id: Option<Uuid>,
    /// Replacement binding created by the transition.
    pub replacement_binding_id: Option<Uuid>,
    /// Resulting authorization-relevant binding version.
    pub binding_version: Option<u64>,
    /// Version of the replacement binding ID, when a replacement was created.
    pub replacement_binding_version: Option<u64>,
    /// Resulting pending selector version.
    pub selector_version: Option<u64>,
}

impl fmt::Debug for LifecycleReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LifecycleReceipt")
            .field("binding_id", &"[redacted]")
            .field("replacement_binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("replacement_binding_version", &"[redacted]")
            .field("selector_version", &"[redacted]")
            .finish()
    }
}

/// Outcome of an idempotent lifecycle operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleResult {
    /// This invocation committed the transition.
    Applied(LifecycleReceipt),
    /// The same operation ID and request had already committed.
    AlreadyApplied(LifecycleReceipt),
}

#[derive(Clone)]
struct ActiveBinding {
    binding_id: Uuid,
    issuer: String,
    subject: String,
    pubkey: Vec<u8>,
    binding_version: u64,
    provenance: BindingProvenance,
}

const OP_PROVISION: &str = "provision";
const OP_RETIRE_PAIR: &str = "retire_pair";
const OP_DISABLE: &str = "disable_identity";
const OP_REVOKE_KEY: &str = "revoke_key";
const OP_ROTATE: &str = "rotate";
const OP_RECOVER: &str = "recover";
const OP_ENABLE: &str = "enable_identity";
const OP_ARCHIVE: &str = "archive";

fn validate_context(context: LifecycleContext<'_>) -> Result<()> {
    if context.operation_id.as_uuid().is_nil() || context.reason.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity lifecycle requires an operation ID and reason".to_string(),
        ));
    }
    validate_pubkey(context.actor)?;
    Ok(())
}

fn validate_principal(principal: IdentityPrincipal<'_>) -> Result<()> {
    if principal.issuer.is_empty() || principal.subject.is_empty() {
        return Err(DbError::InvalidData(
            "identity lifecycle principal must be exact and non-empty".to_string(),
        ));
    }
    Ok(())
}

fn validate_pubkey(pubkey: &[u8]) -> Result<()> {
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(
            "identity lifecycle pubkey must be 32 bytes".to_string(),
        ));
    }
    Ok(())
}

fn digest_parts(parts: &[&[u8]]) -> Vec<u8> {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().to_vec()
}

fn request_fingerprint(
    kind: &str,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    parts: &[&[u8]],
) -> Vec<u8> {
    let operation_id = context.operation_id.as_uuid();
    let mut all = vec![
        kind.as_bytes(),
        community_id.as_uuid().as_bytes(),
        operation_id.as_bytes(),
        context.actor,
        context.reason.as_bytes(),
    ];
    all.extend_from_slice(parts);
    digest_parts(&all)
}

fn optional_fingerprint_bytes(value: Option<&[u8]>) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(1 + value.map_or(0, <[u8]>::len));
    encoded.push(u8::from(value.is_some()));
    if let Some(value) = value {
        encoded.extend_from_slice(value);
    }
    encoded
}

fn replacement_request_fingerprint(
    kind: &str,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    parts: &[&[u8]],
    replacement: &VerifiedReplacementKey<'_>,
) -> Vec<u8> {
    let display_name = optional_fingerprint_bytes(replacement.display_name.map(str::as_bytes));
    let policy_version = replacement.created_policy_version.as_bytes();
    let mut all = parts.to_vec();
    all.push(replacement.pubkey);
    all.push(&display_name);
    all.push(replacement.provenance.as_str().as_bytes());
    all.push(policy_version);
    request_fingerprint(kind, community_id, context, &all)
}

fn i64_version(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| DbError::InvalidData("identity lifecycle version is out of range".to_string()))
}

fn u64_version(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("identity lifecycle version is out of range".to_string()))
}

async fn begin_locked<'a>(
    pool: &'a PgPool,
    coordinates: Vec<Vec<u8>>,
) -> Result<Transaction<'a, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_coordinates_tx(&mut tx, coordinates).await?;
    Ok(tx)
}

fn lifecycle_coordinates(
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: Option<IdentityPrincipal<'_>>,
    pubkeys: &[&[u8]],
    binding_id: Option<Uuid>,
) -> Vec<Vec<u8>> {
    let mut coordinates = vec![operation_lock_coordinate(
        community_id,
        context.operation_id.as_uuid(),
    )];
    if let Some(principal) = principal {
        coordinates.push(principal_lock_coordinate(
            community_id,
            principal.issuer,
            principal.subject,
        ));
    }
    coordinates.extend(
        pubkeys
            .iter()
            .map(|pubkey| key_lock_coordinate(community_id, pubkey)),
    );
    if let Some(binding_id) = binding_id {
        coordinates.push(binding_lock_coordinate(community_id, binding_id));
    }
    coordinates
}

async fn principal_quarantined_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_migration_denials \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3",
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn require_not_quarantined_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<()> {
    if principal_quarantined_tx(tx, community_id, principal).await? {
        return Err(DbError::InvalidData(
            "identity principal is blocked by migrated lineage".to_string(),
        ));
    }
    Ok(())
}

async fn key_quarantined_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_migration_denied_keys \
         WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn require_key_not_quarantined_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<()> {
    if key_quarantined_tx(tx, community_id, pubkey).await? {
        return Err(DbError::InvalidData(
            "identity key is blocked by migrated lineage".to_string(),
        ));
    }
    Ok(())
}

async fn active_principal_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<Option<ActiveBinding>> {
    #[cfg(test)]
    crate::identity_binding::test_lock_schedule::row_checkpoint(
        tx,
        crate::identity_binding::test_lock_schedule::RowLockPhase::Request,
    )
    .await;
    let row = sqlx::query(
        "SELECT binding_id, issuer, uid, pubkey, binding_version, binding_provenance \
         FROM identity_bindings \
         WHERE community_id=$1 AND issuer=$2 AND uid=$3 \
           AND binding_state='active' AND revoked_at IS NULL \
         FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .fetch_optional(&mut **tx)
    .await?;
    #[cfg(test)]
    crate::identity_binding::test_lock_schedule::row_checkpoint(
        tx,
        crate::identity_binding::test_lock_schedule::RowLockPhase::Acquired,
    )
    .await;
    active_binding_from_row(row.as_ref())
}

async fn active_key_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<ActiveBinding>> {
    #[cfg(test)]
    crate::identity_binding::test_lock_schedule::row_checkpoint(
        tx,
        crate::identity_binding::test_lock_schedule::RowLockPhase::Request,
    )
    .await;
    let row = sqlx::query(
        "SELECT binding_id, issuer, uid, pubkey, binding_version, binding_provenance \
         FROM identity_bindings \
         WHERE community_id=$1 AND pubkey=$2 \
           AND binding_state='active' AND revoked_at IS NULL \
         FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    #[cfg(test)]
    crate::identity_binding::test_lock_schedule::row_checkpoint(
        tx,
        crate::identity_binding::test_lock_schedule::RowLockPhase::Acquired,
    )
    .await;
    active_binding_from_row(row.as_ref())
}

fn active_binding_from_row(row: Option<&sqlx::postgres::PgRow>) -> Result<Option<ActiveBinding>> {
    row.map(|row| {
        let version: i64 = row.try_get("binding_version")?;
        let provenance: String = row.try_get("binding_provenance")?;
        Ok(ActiveBinding {
            binding_id: row.try_get("binding_id")?,
            issuer: row.try_get("issuer")?,
            subject: row.try_get("uid")?,
            pubkey: row.try_get("pubkey")?,
            binding_version: u64_version(version)?,
            provenance: BindingProvenance::parse(&provenance)?,
        })
    })
    .transpose()
}

async fn principal_disabled_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_principals \
         WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND disabled_at IS NOT NULL",
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn key_revoked_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(
        sqlx::query("SELECT 1 FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2")
            .bind(community_id.as_uuid())
            .bind(pubkey)
            .fetch_optional(&mut **tx)
            .await?
            .is_some(),
    )
}

async fn pair_retired_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_retired_pairs \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND pubkey=$4 \
         UNION ALL \
         SELECT 1 FROM identity_bindings \
         WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND pubkey=$4 \
           AND revoked_at IS NOT NULL \
         LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn pending_lineage_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<Option<PendingLineage>> {
    let row = sqlx::query(
        r#"
        SELECT retired_pubkey, retired_binding_id, retired_binding_version, selector_version
        FROM identity_pending_replacements
        WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(PendingLineage {
            retired_pubkey: row.try_get("retired_pubkey")?,
            retired_binding_id: row.try_get("retired_binding_id")?,
            retired_binding_version: u64_version(row.try_get("retired_binding_version")?)?,
            selector_version: u64_version(row.try_get("selector_version")?)?,
        })
    })
    .transpose()
}

/// Read the current exact pending lineage under the shared principal lock.
pub async fn get_pending_lineage(
    pool: &PgPool,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
) -> Result<Option<PendingLineage>> {
    validate_principal(principal)?;
    let mut tx = begin_locked(
        pool,
        vec![principal_lock_coordinate(
            community_id,
            principal.issuer,
            principal.subject,
        )],
    )
    .await?;
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    let pending = pending_lineage_tx(&mut tx, community_id, principal).await?;
    tx.commit().await?;
    Ok(pending)
}

async fn replacement_eligible_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
    replacement: &VerifiedReplacementKey<'_>,
) -> Result<()> {
    if active_key_tx(tx, community_id, replacement.pubkey)
        .await?
        .is_some()
        || key_quarantined_tx(tx, community_id, replacement.pubkey).await?
        || key_revoked_tx(tx, community_id, replacement.pubkey).await?
        || pair_retired_tx(tx, community_id, principal, replacement.pubkey).await?
    {
        return Err(DbError::InvalidData(
            "replacement identity key is not eligible".to_string(),
        ));
    }
    Ok(())
}

async fn insert_binding_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    replacement: &VerifiedReplacementKey<'_>,
    transition_kind: &str,
) -> Result<BindingEvidence> {
    let version = 1;
    let binding_id = Uuid::new_v4();
    let created_at: DateTime<Utc> = sqlx::query_scalar(
        r#"
        INSERT INTO identity_bindings
            (community_id, issuer, uid, pubkey, display_name, source, binding_id,
             binding_version, binding_state, binding_provenance, created_by,
             created_policy_version, creation_attribution_kind)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9, $10, $11, $12)
        RETURNING created_at
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(replacement.pubkey)
    .bind(replacement.display_name)
    .bind(replacement.provenance.legacy_source())
    .bind(binding_id)
    .bind(i64_version(version)?)
    .bind(replacement.provenance.as_str())
    .bind(context.actor)
    .bind(replacement.created_policy_version)
    .bind(CreationAttributionKind::Operator.as_str())
    .fetch_one(&mut **tx)
    .await?;
    append_history_tx(
        tx,
        community_id,
        context,
        binding_id,
        version,
        principal,
        replacement.pubkey,
        "active",
        replacement.provenance,
        transition_kind,
        None,
    )
    .await?;
    Ok(BindingEvidence {
        authorization_domain: community_id,
        issuer: principal.issuer.to_owned(),
        subject: principal.subject.to_owned(),
        bound_pubkey: replacement.pubkey.to_vec(),
        binding_id,
        binding_version: version,
        binding_state: BindingState::Active,
        provenance: replacement.provenance,
        creation_attribution: CreationAttributionKind::Operator,
        created_by: Some(context.actor.to_vec()),
        created_policy_version: Some(replacement.created_policy_version.to_owned()),
        expires_at: None,
        created_at,
    })
}

#[allow(clippy::too_many_arguments)]
async fn append_history_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    binding_id: Uuid,
    binding_version: u64,
    principal: IdentityPrincipal<'_>,
    pubkey: &[u8],
    binding_state: &str,
    provenance: BindingProvenance,
    transition_kind: &str,
    replacement_binding_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO identity_binding_history
            (community_id, binding_id, binding_version, issuer, subject, pubkey,
             binding_state, binding_provenance, transition_kind,
             replacement_binding_id, operation_id, actor, reason)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(binding_id)
    .bind(i64_version(binding_version)?)
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(pubkey)
    .bind(binding_state)
    .bind(provenance.as_str())
    .bind(transition_kind)
    .bind(replacement_binding_id)
    .bind(context.operation_id.as_uuid())
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_pair_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    binding: &ActiveBinding,
    retired_version: u64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO identity_retired_pairs
            (community_id, issuer, subject, pubkey, retired_binding_id,
             retired_binding_version, retired_at, retired_by, reason)
        VALUES ($1,$2,$3,$4,$5,$6,NOW(),$7,$8)
        ON CONFLICT (community_id, issuer, subject, pubkey) DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(&binding.pubkey)
    .bind(binding.binding_id)
    .bind(i64_version(retired_version)?)
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_pending_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    binding: &ActiveBinding,
    retired_version: u64,
) -> Result<u64> {
    let selector_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(selector_version), 0) + 1 \
         FROM identity_pending_replacements \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3",
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO identity_pending_replacements
            (community_id, issuer, subject, selector_version, retired_pubkey,
             retired_binding_id, retired_binding_version, created_operation_id)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(selector_version)
    .bind(&binding.pubkey)
    .bind(binding.binding_id)
    .bind(i64_version(retired_version)?)
    .bind(context.operation_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    u64_version(selector_version)
}

struct RetireActive<'a> {
    transition_kind: &'a str,
    revocation_scope: &'a str,
    replacement: Option<(&'a [u8], Uuid)>,
    create_pending: bool,
}

async fn retire_active_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    binding: &ActiveBinding,
    transition: RetireActive<'_>,
) -> Result<(u64, Option<u64>)> {
    if pending_lineage_tx(tx, community_id, principal)
        .await?
        .is_some()
    {
        return Err(DbError::InvalidData(
            "active identity binding overlaps pending lineage".to_string(),
        ));
    }
    let next_version = binding
        .binding_version
        .checked_add(1)
        .ok_or_else(|| DbError::InvalidData("identity binding version exhausted".to_string()))?;
    let (replacement_pubkey, replacement_binding_id) = transition
        .replacement
        .map(|(key, id)| (Some(key), Some(id)))
        .unwrap_or((None, None));
    let changed = sqlx::query(
        r#"
        UPDATE identity_bindings
        SET binding_version=$5, binding_state=$6, revoked_at=NOW(),
            revoked_by=$7, revoked_reason=$8, revocation_scope=$9,
            rotation_completed_at=CASE WHEN $6='rotated' THEN NOW() ELSE rotation_completed_at END,
            rotated_to_pubkey=CASE WHEN $6='rotated' THEN $10 ELSE rotated_to_pubkey END,
            rotation_by=CASE WHEN $6='rotated' THEN $7 ELSE rotation_by END,
            rotation_reason=CASE WHEN $6='rotated' THEN $8 ELSE rotation_reason END,
            replacement_binding_id=$11, updated_at=NOW()
        WHERE community_id=$1 AND binding_id=$2 AND issuer=$3 AND uid=$4
          AND binding_state='active' AND revoked_at IS NULL AND binding_version=$12
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(binding.binding_id)
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(i64_version(next_version)?)
    .bind(if transition.replacement.is_some() {
        "rotated"
    } else {
        "revoked"
    })
    .bind(context.actor)
    .bind(context.reason)
    .bind(transition.revocation_scope)
    .bind(replacement_pubkey)
    .bind(replacement_binding_id)
    .bind(i64_version(binding.binding_version)?)
    .execute(&mut **tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "identity binding changed during lifecycle transition".to_string(),
        ));
    }
    insert_pair_tx(tx, community_id, context, principal, binding, next_version).await?;
    let selector_version = if transition.create_pending {
        Some(insert_pending_tx(tx, community_id, context, principal, binding, next_version).await?)
    } else {
        None
    };
    append_history_tx(
        tx,
        community_id,
        context,
        binding.binding_id,
        next_version,
        principal,
        &binding.pubkey,
        if transition.replacement.is_some() {
            "rotated"
        } else {
            "revoked"
        },
        binding.provenance,
        transition.transition_kind,
        replacement_binding_id,
    )
    .await?;
    Ok((next_version, selector_version))
}

fn receipt_from_row(row: &sqlx::postgres::PgRow) -> Result<LifecycleReceipt> {
    let binding_version: Option<i64> = row.try_get("binding_version")?;
    let replacement_binding_version: Option<i64> = row.try_get("replacement_binding_version")?;
    let selector_version: Option<i64> = row.try_get("selector_version")?;
    Ok(LifecycleReceipt {
        binding_id: row.try_get("binding_id")?,
        replacement_binding_id: row.try_get("replacement_binding_id")?,
        binding_version: binding_version.map(u64_version).transpose()?,
        replacement_binding_version: replacement_binding_version.map(u64_version).transpose()?,
        selector_version: selector_version.map(u64_version).transpose()?,
    })
}

async fn existing_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    kind: &str,
    fingerprint: &[u8],
) -> Result<Option<LifecycleReceipt>> {
    let row = sqlx::query(
        r#"
        SELECT operation_kind, request_fingerprint, binding_id,
               replacement_binding_id, binding_version,
               replacement_binding_version, selector_version
        FROM identity_lifecycle_operations
        WHERE community_id=$1 AND operation_id=$2
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(context.operation_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let existing_kind: String = row.try_get("operation_kind")?;
    let existing_fingerprint: Vec<u8> = row.try_get("request_fingerprint")?;
    if existing_kind != kind || existing_fingerprint != fingerprint {
        return Err(DbError::InvalidData(
            "identity lifecycle operation ID was reused".to_string(),
        ));
    }
    Ok(Some(receipt_from_row(&row)?))
}

struct OperationFinish<'a> {
    kind: &'a str,
    fingerprint: &'a [u8],
    principal: Option<IdentityPrincipal<'a>>,
    pubkey: Option<&'a [u8]>,
    replacement_pubkey: Option<&'a [u8]>,
    receipt: LifecycleReceipt,
}

async fn record_operation_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    completion: &OperationFinish<'_>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO identity_lifecycle_operations
            (community_id, operation_id, operation_kind, request_fingerprint,
             issuer, subject, pubkey, replacement_pubkey, binding_id,
             replacement_binding_id, binding_version, replacement_binding_version,
             selector_version, actor, reason)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(context.operation_id.as_uuid())
    .bind(completion.kind)
    .bind(completion.fingerprint)
    .bind(completion.principal.map(|value| value.issuer))
    .bind(completion.principal.map(|value| value.subject))
    .bind(completion.pubkey)
    .bind(completion.replacement_pubkey)
    .bind(completion.receipt.binding_id)
    .bind(completion.receipt.replacement_binding_id)
    .bind(
        completion
            .receipt
            .binding_version
            .map(i64_version)
            .transpose()?,
    )
    .bind(
        completion
            .receipt
            .replacement_binding_version
            .map(i64_version)
            .transpose()?,
    )
    .bind(
        completion
            .receipt
            .selector_version
            .map(i64_version)
            .transpose()?,
    )
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn finish(
    mut tx: Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    completion: OperationFinish<'_>,
) -> Result<LifecycleResult> {
    record_operation_tx(&mut tx, community_id, context, &completion).await?;
    tx.commit().await?;
    Ok(LifecycleResult::Applied(completion.receipt))
}

/// Provision a binding only after the server resolves provisioned mode.
pub async fn provision_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    enrollment_mode: EnrollmentMode,
    replacement: VerifiedReplacementKey<'_>,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    if enrollment_mode != EnrollmentMode::Provisioned
        || replacement.provenance != BindingProvenance::Provisioned
    {
        return Err(DbError::InvalidData(
            "provisioning requires server-resolved provisioned mode".to_string(),
        ));
    }
    let fingerprint = replacement_request_fingerprint(
        OP_PROVISION,
        community_id,
        context,
        &[principal.issuer.as_bytes(), principal.subject.as_bytes()],
        &replacement,
    );
    let coordinates = lifecycle_coordinates(
        community_id,
        context,
        Some(principal),
        &[replacement.pubkey],
        None,
    );
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_PROVISION, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    if active_principal_tx(&mut tx, community_id, principal)
        .await?
        .is_some()
        || principal_disabled_tx(&mut tx, community_id, principal).await?
        || pending_lineage_tx(&mut tx, community_id, principal)
            .await?
            .is_some()
    {
        return Err(DbError::InvalidData(
            "identity principal is not eligible for provisioning".to_string(),
        ));
    }
    replacement_eligible_tx(&mut tx, community_id, principal, &replacement).await?;
    let binding = insert_binding_tx(
        &mut tx,
        community_id,
        context,
        principal,
        &replacement,
        OP_PROVISION,
    )
    .await?;
    let receipt = LifecycleReceipt {
        binding_id: Some(binding.binding_id),
        replacement_binding_id: None,
        binding_version: Some(binding.binding_version),
        replacement_binding_version: None,
        selector_version: None,
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_PROVISION,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: None,
            replacement_pubkey: Some(replacement.pubkey),
            receipt,
        },
    )
    .await
}

/// Retire one exact active pair without revoking its key domain-wide.
pub async fn retire_identity_pair(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    pubkey: &[u8],
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    validate_pubkey(pubkey)?;
    let fingerprint = request_fingerprint(
        OP_RETIRE_PAIR,
        community_id,
        context,
        &[
            principal.issuer.as_bytes(),
            principal.subject.as_bytes(),
            pubkey,
        ],
    );
    let coordinates =
        lifecycle_coordinates(community_id, context, Some(principal), &[pubkey], None);
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_RETIRE_PAIR, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    let active = active_principal_tx(&mut tx, community_id, principal)
        .await?
        .filter(|binding| binding.pubkey == pubkey)
        .ok_or_else(|| DbError::InvalidData("identity pair is not active".to_string()))?;
    let (version, selector_version) = retire_active_tx(
        &mut tx,
        community_id,
        context,
        principal,
        &active,
        RetireActive {
            transition_kind: OP_RETIRE_PAIR,
            revocation_scope: "rotation",
            replacement: None,
            create_pending: true,
        },
    )
    .await?;
    let receipt = LifecycleReceipt {
        binding_id: Some(active.binding_id),
        replacement_binding_id: None,
        binding_version: Some(version),
        replacement_binding_version: None,
        selector_version,
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_RETIRE_PAIR,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: Some(pubkey),
            replacement_pubkey: None,
            receipt,
        },
    )
    .await
}

/// Disable an exact principal, preserving any existing pending lineage.
pub async fn disable_identity_principal(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    let fingerprint = request_fingerprint(
        OP_DISABLE,
        community_id,
        context,
        &[principal.issuer.as_bytes(), principal.subject.as_bytes()],
    );
    let coordinates = lifecycle_coordinates(community_id, context, Some(principal), &[], None);
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_DISABLE, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    let active = active_principal_tx(&mut tx, community_id, principal).await?;
    let pending = pending_lineage_tx(&mut tx, community_id, principal).await?;
    if active.is_some() && pending.is_some() {
        return Err(DbError::InvalidData(
            "active identity binding overlaps pending lineage".to_string(),
        ));
    }
    sqlx::query(
        r#"
        INSERT INTO identity_principals
            (community_id, issuer, uid, disabled_at, disabled_by, disabled_reason)
        VALUES ($1,$2,$3,NOW(),$4,$5)
        ON CONFLICT (community_id, issuer, uid) DO UPDATE
        SET disabled_at=COALESCE(identity_principals.disabled_at, NOW()),
            disabled_by=COALESCE(identity_principals.disabled_by, EXCLUDED.disabled_by),
            disabled_reason=COALESCE(identity_principals.disabled_reason, EXCLUDED.disabled_reason)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut *tx)
    .await?;
    let (binding_id, version, selector_version) = if let Some(active) = active.as_ref() {
        let (version, selector_version) = retire_active_tx(
            &mut tx,
            community_id,
            context,
            principal,
            active,
            RetireActive {
                transition_kind: OP_DISABLE,
                revocation_scope: "principal",
                replacement: None,
                create_pending: true,
            },
        )
        .await?;
        (Some(active.binding_id), Some(version), selector_version)
    } else {
        (None, None, pending.map(|value| value.selector_version))
    };
    let receipt = LifecycleReceipt {
        binding_id,
        replacement_binding_id: None,
        binding_version: version,
        replacement_binding_version: None,
        selector_version,
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_DISABLE,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: None,
            replacement_pubkey: None,
            receipt,
        },
    )
    .await
}

/// Revoke one key throughout a server-resolved authorization domain.
pub async fn revoke_identity_key(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    pubkey: &[u8],
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_pubkey(pubkey)?;
    let fingerprint = request_fingerprint(OP_REVOKE_KEY, community_id, context, &[pubkey]);
    let coordinates = lifecycle_coordinates(community_id, context, None, &[pubkey], None);
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_REVOKE_KEY, &fingerprint).await?
    {
        tx.commit().await?;
        verify_key_revocation_committed(pool, community_id, context, pubkey, None).await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    let active = active_key_tx(&mut tx, community_id, pubkey).await?;
    let principal = active.as_ref().map(|binding| IdentityPrincipal {
        issuer: binding.issuer.as_str(),
        subject: binding.subject.as_str(),
    });
    if let Some(principal) = principal {
        require_not_quarantined_tx(&mut tx, community_id, principal).await?;
        if pending_lineage_tx(&mut tx, community_id, principal)
            .await?
            .is_some()
        {
            return Err(DbError::InvalidData(
                "active identity binding overlaps pending lineage".to_string(),
            ));
        }
    }
    sqlx::query(
        r#"
        INSERT INTO identity_revoked_keys
            (community_id, pubkey, revoked_at, revoked_by, reason)
        VALUES ($1,$2,NOW(),$3,$4)
        ON CONFLICT (community_id, pubkey) DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut *tx)
    .await?;
    let (binding_id, version, selector_version) =
        if let (Some(active), Some(principal)) = (active.as_ref(), principal) {
            let (version, selector_version) = retire_active_tx(
                &mut tx,
                community_id,
                context,
                principal,
                active,
                RetireActive {
                    transition_kind: OP_REVOKE_KEY,
                    revocation_scope: "key",
                    replacement: None,
                    create_pending: true,
                },
            )
            .await?;
            (Some(active.binding_id), Some(version), selector_version)
        } else {
            (None, None, None)
        };
    let receipt = LifecycleReceipt {
        binding_id,
        replacement_binding_id: None,
        binding_version: version,
        replacement_binding_version: None,
        selector_version,
    };
    let outcome = finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_REVOKE_KEY,
            fingerprint: &fingerprint,
            principal,
            pubkey: Some(pubkey),
            replacement_pubkey: None,
            receipt,
        },
    )
    .await?;
    verify_key_revocation_committed(
        pool,
        community_id,
        context,
        pubkey,
        active.as_ref().zip(version),
    )
    .await?;
    Ok(outcome)
}

async fn link_replacement_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    old_binding_id: Uuid,
    replacement_binding_id: Uuid,
    replacement_pubkey: &[u8],
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO identity_binding_lineage
            (community_id, predecessor_binding_id, successor_binding_id)
        VALUES ($1,$2,$3)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(old_binding_id)
    .bind(replacement_binding_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE identity_bindings \
         SET replacement_binding_id=$3, \
             rotation_completed_at=COALESCE(rotation_completed_at, NOW()), \
             rotated_to_pubkey=COALESCE(rotated_to_pubkey, $4), \
             rotation_by=COALESCE(rotation_by, $5), \
             rotation_reason=COALESCE(rotation_reason, $6), updated_at=NOW() \
         WHERE community_id=$1 AND binding_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(old_binding_id)
    .bind(replacement_binding_id)
    .bind(replacement_pubkey)
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn verify_key_revocation_committed(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    pubkey: &[u8],
    retired: Option<(&ActiveBinding, u64)>,
) -> Result<()> {
    let selector_and_operation: (bool, bool) = sqlx::query_as(
        "SELECT \
           EXISTS(SELECT 1 FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2), \
           EXISTS(SELECT 1 FROM identity_lifecycle_operations WHERE community_id=$1 AND operation_id=$3 AND operation_kind='revoke_key')",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(context.operation_id.as_uuid())
    .fetch_one(pool)
    .await?;
    if selector_and_operation != (true, true) {
        return Err(DbError::InvalidData(
            "committed identity key revocation could not be verified".to_string(),
        ));
    }
    if let Some((binding, retired_version)) = retired {
        let pair: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM identity_retired_pairs \
             WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND pubkey=$4 \
               AND retired_binding_id=$5 AND retired_binding_version=$6)",
        )
        .bind(community_id.as_uuid())
        .bind(&binding.issuer)
        .bind(&binding.subject)
        .bind(pubkey)
        .bind(binding.binding_id)
        .bind(i64_version(retired_version)?)
        .fetch_one(pool)
        .await?;
        if !pair {
            return Err(DbError::InvalidData(
                "committed identity pair retirement could not be verified".to_string(),
            ));
        }
    }
    Ok(())
}

/// Atomically replace one active pair without creating a domain key tombstone.
pub async fn rotate_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    old_pubkey: &[u8],
    replacement: VerifiedReplacementKey<'_>,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    validate_pubkey(old_pubkey)?;
    if old_pubkey == replacement.pubkey {
        return Err(DbError::InvalidData(
            "identity rotation requires a different key".to_string(),
        ));
    }
    let fingerprint = replacement_request_fingerprint(
        OP_ROTATE,
        community_id,
        context,
        &[
            principal.issuer.as_bytes(),
            principal.subject.as_bytes(),
            old_pubkey,
        ],
        &replacement,
    );
    let coordinates = lifecycle_coordinates(
        community_id,
        context,
        Some(principal),
        &[old_pubkey, replacement.pubkey],
        None,
    );
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_ROTATE, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    require_key_not_quarantined_tx(&mut tx, community_id, old_pubkey).await?;
    if key_revoked_tx(&mut tx, community_id, old_pubkey).await? {
        return Err(DbError::InvalidData(
            "revoked identity key cannot rotate".to_string(),
        ));
    }
    if principal_disabled_tx(&mut tx, community_id, principal).await? {
        return Err(DbError::InvalidData(
            "disabled identity cannot rotate".to_string(),
        ));
    }
    let active = active_principal_tx(&mut tx, community_id, principal)
        .await?
        .filter(|binding| binding.pubkey == old_pubkey)
        .ok_or_else(|| DbError::InvalidData("rotation source is not active".to_string()))?;
    replacement_eligible_tx(&mut tx, community_id, principal, &replacement).await?;
    if pending_lineage_tx(&mut tx, community_id, principal)
        .await?
        .is_some()
    {
        return Err(DbError::InvalidData(
            "rotation cannot consume pending lineage".to_string(),
        ));
    }
    let replacement_binding_id = Uuid::new_v4();
    let (old_version, _) = retire_active_tx(
        &mut tx,
        community_id,
        context,
        principal,
        &active,
        RetireActive {
            transition_kind: OP_ROTATE,
            revocation_scope: "rotation",
            replacement: Some((replacement.pubkey, replacement_binding_id)),
            create_pending: false,
        },
    )
    .await?;
    let new_version = 1;
    sqlx::query(
        r#"
        INSERT INTO identity_bindings
            (community_id, issuer, uid, pubkey, display_name, source, binding_id,
             binding_version, binding_state, binding_provenance, created_by,
             created_policy_version, creation_attribution_kind)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10,$11,$12)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(replacement.pubkey)
    .bind(replacement.display_name)
    .bind(replacement.provenance.legacy_source())
    .bind(replacement_binding_id)
    .bind(i64_version(new_version)?)
    .bind(replacement.provenance.as_str())
    .bind(context.actor)
    .bind(replacement.created_policy_version)
    .bind(CreationAttributionKind::Operator.as_str())
    .execute(&mut *tx)
    .await?;
    append_history_tx(
        &mut tx,
        community_id,
        context,
        replacement_binding_id,
        new_version,
        principal,
        replacement.pubkey,
        "active",
        replacement.provenance,
        OP_ROTATE,
        None,
    )
    .await?;
    link_replacement_tx(
        &mut tx,
        community_id,
        context,
        active.binding_id,
        replacement_binding_id,
        replacement.pubkey,
    )
    .await?;
    let receipt = LifecycleReceipt {
        binding_id: Some(active.binding_id),
        replacement_binding_id: Some(replacement_binding_id),
        binding_version: Some(old_version),
        replacement_binding_version: Some(new_version),
        selector_version: None,
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_ROTATE,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: Some(old_pubkey),
            replacement_pubkey: Some(replacement.pubkey),
            receipt,
        },
    )
    .await
}

async fn compare_and_clear_pending_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    expected: &PendingLineage,
) -> Result<()> {
    let changed = sqlx::query(
        r#"
        UPDATE identity_pending_replacements
        SET cleared_at=NOW(), cleared_operation_id=$8
        WHERE community_id=$1 AND issuer=$2 AND subject=$3
          AND retired_pubkey=$4 AND retired_binding_id=$5
          AND retired_binding_version=$6 AND selector_version=$7
          AND cleared_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(&expected.retired_pubkey)
    .bind(expected.retired_binding_id)
    .bind(i64_version(expected.retired_binding_version)?)
    .bind(i64_version(expected.selector_version)?)
    .bind(context.operation_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "pending identity lineage changed concurrently".to_string(),
        ));
    }
    Ok(())
}

async fn require_expected_pending_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    principal: IdentityPrincipal<'_>,
    expected: &PendingLineage,
) -> Result<()> {
    let current = pending_lineage_tx(tx, community_id, principal)
        .await?
        .filter(|current| current == expected)
        .ok_or_else(|| DbError::InvalidData("pending identity lineage is stale".to_string()))?;
    let retired: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM identity_retired_pairs
            WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND pubkey=$4
              AND retired_binding_id=$5 AND retired_binding_version=$6
        )
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .bind(&current.retired_pubkey)
    .bind(current.retired_binding_id)
    .bind(i64_version(current.retired_binding_version)?)
    .fetch_one(&mut **tx)
    .await?;
    if !retired {
        return Err(DbError::InvalidData(
            "pending identity lineage lacks an exact retired pair".to_string(),
        ));
    }
    Ok(())
}

/// Recover a non-disabled principal using exact compare-and-clear lineage.
pub async fn recover_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    expected: &PendingLineage,
    replacement: VerifiedReplacementKey<'_>,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    validate_pubkey(&expected.retired_pubkey)?;
    let retired_binding_version = expected.retired_binding_version.to_be_bytes();
    let selector_version = expected.selector_version.to_be_bytes();
    let fingerprint = replacement_request_fingerprint(
        OP_RECOVER,
        community_id,
        context,
        &[
            principal.issuer.as_bytes(),
            principal.subject.as_bytes(),
            &expected.retired_pubkey,
            expected.retired_binding_id.as_bytes(),
            &retired_binding_version,
            &selector_version,
        ],
        &replacement,
    );
    let coordinates = lifecycle_coordinates(
        community_id,
        context,
        Some(principal),
        &[&expected.retired_pubkey, replacement.pubkey],
        Some(expected.retired_binding_id),
    );
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_RECOVER, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    require_key_not_quarantined_tx(&mut tx, community_id, &expected.retired_pubkey).await?;
    if principal_disabled_tx(&mut tx, community_id, principal).await?
        || active_principal_tx(&mut tx, community_id, principal)
            .await?
            .is_some()
    {
        return Err(DbError::InvalidData(
            "identity principal is not eligible for recovery".to_string(),
        ));
    }
    require_expected_pending_tx(&mut tx, community_id, principal, expected).await?;
    replacement_eligible_tx(&mut tx, community_id, principal, &replacement).await?;
    compare_and_clear_pending_tx(&mut tx, community_id, context, principal, expected).await?;
    let binding = insert_binding_tx(
        &mut tx,
        community_id,
        context,
        principal,
        &replacement,
        OP_RECOVER,
    )
    .await?;
    link_replacement_tx(
        &mut tx,
        community_id,
        context,
        expected.retired_binding_id,
        binding.binding_id,
        replacement.pubkey,
    )
    .await?;
    let receipt = LifecycleReceipt {
        binding_id: Some(expected.retired_binding_id),
        replacement_binding_id: Some(binding.binding_id),
        binding_version: Some(expected.retired_binding_version),
        replacement_binding_version: Some(binding.binding_version),
        selector_version: Some(expected.selector_version),
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_RECOVER,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: Some(&expected.retired_pubkey),
            replacement_pubkey: Some(replacement.pubkey),
            receipt,
        },
    )
    .await
}

/// Re-enable a disabled principal and optionally clear exact pending lineage.
pub async fn enable_identity_principal(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    principal: IdentityPrincipal<'_>,
    expected: Option<&PendingLineage>,
    replacement: VerifiedReplacementKey<'_>,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    validate_principal(principal)?;
    let empty = [];
    let expected_presence = [u8::from(expected.is_some())];
    let expected_key = expected.map_or(empty.as_slice(), |value| value.retired_pubkey.as_slice());
    let expected_binding_id = expected
        .map(|value| value.retired_binding_id.as_bytes().as_slice())
        .unwrap_or(empty.as_slice());
    let expected_binding_version = expected
        .map(|value| value.retired_binding_version)
        .unwrap_or_default()
        .to_be_bytes();
    let expected_selector_version = expected
        .map(|value| value.selector_version)
        .unwrap_or_default()
        .to_be_bytes();
    let fingerprint = replacement_request_fingerprint(
        OP_ENABLE,
        community_id,
        context,
        &[
            principal.issuer.as_bytes(),
            principal.subject.as_bytes(),
            &expected_presence,
            expected_key,
            expected_binding_id,
            &expected_binding_version,
            &expected_selector_version,
        ],
        &replacement,
    );
    let mut keys = vec![replacement.pubkey];
    if let Some(expected) = expected {
        validate_pubkey(&expected.retired_pubkey)?;
        keys.push(&expected.retired_pubkey);
    }
    let coordinates = lifecycle_coordinates(
        community_id,
        context,
        Some(principal),
        &keys,
        expected.map(|value| value.retired_binding_id),
    );
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_ENABLE, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    require_not_quarantined_tx(&mut tx, community_id, principal).await?;
    if let Some(expected) = expected {
        require_key_not_quarantined_tx(&mut tx, community_id, &expected.retired_pubkey).await?;
    }
    if !principal_disabled_tx(&mut tx, community_id, principal).await?
        || active_principal_tx(&mut tx, community_id, principal)
            .await?
            .is_some()
    {
        return Err(DbError::InvalidData(
            "identity principal is not eligible for re-enablement".to_string(),
        ));
    }
    let current = pending_lineage_tx(&mut tx, community_id, principal).await?;
    match (current.as_ref(), expected) {
        (Some(current), Some(expected)) if current == expected => {
            require_expected_pending_tx(&mut tx, community_id, principal, expected).await?;
        }
        (None, None) => {}
        _ => {
            return Err(DbError::InvalidData(
                "pending identity lineage is stale".to_string(),
            ))
        }
    }
    replacement_eligible_tx(&mut tx, community_id, principal, &replacement).await?;
    if let Some(expected) = expected {
        compare_and_clear_pending_tx(&mut tx, community_id, context, principal, expected).await?;
    }
    let binding = insert_binding_tx(
        &mut tx,
        community_id,
        context,
        principal,
        &replacement,
        OP_ENABLE,
    )
    .await?;
    let changed = sqlx::query(
        r#"
        UPDATE identity_principals
        SET disabled_at=NULL, disabled_by=NULL, disabled_reason=NULL
        WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND disabled_at IS NOT NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(principal.issuer)
    .bind(principal.subject)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "identity disablement changed concurrently".to_string(),
        ));
    }
    if let Some(expected) = expected {
        link_replacement_tx(
            &mut tx,
            community_id,
            context,
            expected.retired_binding_id,
            binding.binding_id,
            replacement.pubkey,
        )
        .await?;
    }
    let receipt = expected.map_or_else(
        || LifecycleReceipt {
            binding_id: Some(binding.binding_id),
            replacement_binding_id: None,
            binding_version: Some(binding.binding_version),
            replacement_binding_version: None,
            selector_version: None,
        },
        |expected| LifecycleReceipt {
            binding_id: Some(expected.retired_binding_id),
            replacement_binding_id: Some(binding.binding_id),
            binding_version: Some(expected.retired_binding_version),
            replacement_binding_version: Some(binding.binding_version),
            selector_version: Some(expected.selector_version),
        },
    );
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_ENABLE,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: expected.map(|value| value.retired_pubkey.as_slice()),
            replacement_pubkey: Some(replacement.pubkey),
            receipt,
        },
    )
    .await
}

/// Archive one already-inactive binding with complete actor and reason
/// attribution. Archival never creates or restores authorization.
pub async fn archive_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    context: LifecycleContext<'_>,
    binding_id: Uuid,
) -> Result<LifecycleResult> {
    validate_context(context)?;
    if binding_id.is_nil() {
        return Err(DbError::InvalidData(
            "identity archive requires a binding ID".to_string(),
        ));
    }
    let fingerprint =
        request_fingerprint(OP_ARCHIVE, community_id, context, &[binding_id.as_bytes()]);
    let coordinates = lifecycle_coordinates(community_id, context, None, &[], Some(binding_id));
    let mut tx = begin_locked(pool, coordinates).await?;
    if let Some(receipt) =
        existing_operation_tx(&mut tx, community_id, context, OP_ARCHIVE, &fingerprint).await?
    {
        tx.commit().await?;
        return Ok(LifecycleResult::AlreadyApplied(receipt));
    }
    let row = sqlx::query(
        r#"
        SELECT issuer, uid, pubkey, binding_version, binding_provenance,
               replacement_binding_id
        FROM identity_bindings
        WHERE community_id=$1 AND binding_id=$2
          AND binding_state IN ('revoked', 'rotated') AND revoked_at IS NOT NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(binding_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DbError::InvalidData("identity binding is not archivable".to_string()))?;
    let issuer: String = row.try_get("issuer")?;
    let subject: String = row.try_get("uid")?;
    let pubkey: Vec<u8> = row.try_get("pubkey")?;
    let version = u64_version(row.try_get("binding_version")?)?;
    let provenance = BindingProvenance::parse(row.try_get("binding_provenance")?)?;
    let replacement_binding_id: Option<Uuid> = row.try_get("replacement_binding_id")?;
    let principal = IdentityPrincipal {
        issuer: &issuer,
        subject: &subject,
    };
    let changed = sqlx::query(
        r#"
        UPDATE identity_bindings
        SET binding_state='archived', archived_at=NOW(), archived_by=$3,
            archived_reason=$4, updated_at=NOW()
        WHERE community_id=$1 AND binding_id=$2
          AND binding_state IN ('revoked', 'rotated') AND revoked_at IS NOT NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(binding_id)
    .bind(context.actor)
    .bind(context.reason)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "identity binding changed during archive".to_string(),
        ));
    }
    append_history_tx(
        &mut tx,
        community_id,
        context,
        binding_id,
        version,
        principal,
        &pubkey,
        "archived",
        provenance,
        OP_ARCHIVE,
        replacement_binding_id,
    )
    .await?;
    let receipt = LifecycleReceipt {
        binding_id: Some(binding_id),
        replacement_binding_id: None,
        binding_version: Some(version),
        replacement_binding_version: None,
        selector_version: None,
    };
    finish(
        tx,
        community_id,
        context,
        OperationFinish {
            kind: OP_ARCHIVE,
            fingerprint: &fingerprint,
            principal: Some(principal),
            pubkey: Some(&pubkey),
            replacement_pubkey: None,
            receipt,
        },
    )
    .await
}

#[cfg(test)]
#[path = "identity_lifecycle_deterministic_tests.rs"]
mod deterministic_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::identity_binding::{
        resolve_identity_binding, BindingDenial, ResolveBindingInput, ResolveBindingResult,
    };

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
    const ISSUER: &str = "https://idp.example";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect test DB");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(id)
            .bind(format!("identity-lifecycle-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    async fn enroll(
        pool: &PgPool,
        community_id: CommunityId,
        subject: &str,
        pubkey: &[u8],
    ) -> BindingEvidence {
        match resolve_identity_binding(
            pool,
            &ResolveBindingInput {
                authorization_domain: community_id,
                issuer: ISSUER,
                subject,
                pubkey,
                display_name: None,
                enrollment_mode: EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("resolve enrollment")
        {
            ResolveBindingResult::Enrolled(evidence) => evidence,
            other => panic!("unexpected enrollment result: {other:?}"),
        }
    }

    fn context<'a>(reason: &'a str) -> LifecycleContext<'a> {
        const ACTOR: [u8; 32] = [0xA3; 32];
        LifecycleContext {
            operation_id: LifecycleOperationId::issue(),
            actor: &ACTOR,
            reason,
        }
    }

    fn replacement(pubkey: &[u8]) -> VerifiedReplacementKey<'_> {
        VerifiedReplacementKey::after_verified_proof(
            pubkey,
            None,
            BindingProvenance::AttestedKey,
            "test-policy-v1",
        )
        .expect("verified replacement")
    }

    #[test]
    fn privileged_replacement_rejects_tofu() {
        assert!(VerifiedReplacementKey::after_verified_proof(
            &[1_u8; 32],
            None,
            BindingProvenance::Tofu,
            "test-policy-v1",
        )
        .is_err());
    }

    #[test]
    fn lifecycle_reason_must_contain_non_whitespace_attribution() {
        assert!(validate_context(context(" \t\n ")).is_err());
    }

    #[test]
    fn server_operation_identifier_is_opaque_and_redacted() {
        let raw = Uuid::from_u128(0x1234);
        let identifier = LifecycleOperationId::from_uuid_for_test(raw);
        assert_eq!(identifier.as_uuid(), raw);
        assert!(!format!("{identifier:?}").contains(&raw.to_string()));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn inactive_binding_archive_is_attributed_and_idempotent() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let pubkey = [0xA5_u8; 32];
        let evidence = enroll(&pool, community_id, "archive-subject", &pubkey).await;
        retire_identity_pair(
            &pool,
            community_id,
            context("retire before archive"),
            IdentityPrincipal {
                issuer: ISSUER,
                subject: "archive-subject",
            },
            &pubkey,
        )
        .await
        .expect("retire binding");
        let pending_before = get_pending_lineage(
            &pool,
            community_id,
            IdentityPrincipal {
                issuer: ISSUER,
                subject: "archive-subject",
            },
        )
        .await
        .expect("read pending lineage before archive")
        .expect("retirement creates pending lineage");
        let archive_context = context("archive inactive binding");
        let applied =
            archive_identity_binding(&pool, community_id, archive_context, evidence.binding_id())
                .await
                .expect("archive binding");
        assert!(matches!(applied, LifecycleResult::Applied(_)));
        let replay =
            archive_identity_binding(&pool, community_id, archive_context, evidence.binding_id())
                .await
                .expect("replay archive");
        assert!(matches!(replay, LifecycleResult::AlreadyApplied(_)));
        type ArchivedBindingRow = (
            String,
            Option<DateTime<Utc>>,
            Option<Vec<u8>>,
            Option<String>,
            i64,
        );
        let archived: ArchivedBindingRow = sqlx::query_as(
            "SELECT binding_state, archived_at, archived_by, archived_reason, binding_version \
             FROM identity_bindings \
             WHERE community_id=$1 AND binding_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(evidence.binding_id())
        .fetch_one(&pool)
        .await
        .expect("read archived binding");
        assert_eq!(archived.0, "archived");
        assert!(archived.1.is_some());
        assert_eq!(archived.2.as_deref(), Some(archive_context.actor));
        assert_eq!(archived.3.as_deref(), Some(archive_context.reason));
        assert_eq!(
            u64::try_from(archived.4).unwrap(),
            pending_before.retired_binding_version
        );
        assert_eq!(
            get_pending_lineage(
                &pool,
                community_id,
                IdentityPrincipal {
                    issuer: ISSUER,
                    subject: "archive-subject",
                },
            )
            .await
            .expect("read pending lineage after archive")
            .expect("archive preserves recovery lineage"),
            pending_before
        );
        let archive_history: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_binding_history \
             WHERE community_id=$1 AND binding_id=$2 AND transition_kind='archive'",
        )
        .bind(community_id.as_uuid())
        .bind(evidence.binding_id())
        .fetch_one(&pool)
        .await
        .expect("count archive history");
        assert_eq!(archive_history, 1);
        let ordinary = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community_id,
                issuer: ISSUER,
                subject: "archive-subject",
                pubkey: &pubkey,
                display_name: None,
                enrollment_mode: EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("ordinary authorization returns typed denial");
        assert_eq!(
            ordinary,
            ResolveBindingResult::Denied(BindingDenial::Revoked)
        );
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn committed_rotation_replays_exact_receipt_after_response_loss_and_new_pool() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "response-loss-rotation",
        };
        let old_key = [0xB1_u8; 32];
        let new_key = [0xB2_u8; 32];
        let old_evidence = enroll(&pool, community_id, principal.subject, &old_key).await;
        let request = LifecycleContext {
            operation_id: LifecycleOperationId::issue(),
            actor: &old_key,
            reason: "rotate with lost response",
        };
        let first = rotate_identity_binding(
            &pool,
            community_id,
            request,
            principal,
            &old_key,
            replacement(&new_key),
        )
        .await
        .expect("commit rotation before simulated response loss");
        let LifecycleResult::Applied(applied_receipt) = first else {
            panic!("first rotation must apply");
        };
        assert_eq!(applied_receipt.binding_id, Some(old_evidence.binding_id()));
        drop(pool);

        let restarted_pool = setup_pool().await;
        let replay = rotate_identity_binding(
            &restarted_pool,
            community_id,
            request,
            principal,
            &old_key,
            replacement(&new_key),
        )
        .await
        .expect("retry exact rotation through a new pool");
        assert_eq!(
            replay,
            LifecycleResult::AlreadyApplied(applied_receipt.clone())
        );
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_lifecycle_operations \
                WHERE community_id=$1 AND operation_id=$2)",
        )
        .bind(community_id.as_uuid())
        .bind(request.operation_id.as_uuid())
        .fetch_one(&restarted_pool)
        .await
        .expect("read response-loss retry counts");
        assert_eq!(counts, (2, 3, 1));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn pending_lineage_is_cleared_only_by_exact_compare_and_clear() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "cas-subject",
        };
        let retired_key = [1_u8; 32];
        let replacement_key = [2_u8; 32];
        let retired_evidence = enroll(&pool, community_id, principal.subject, &retired_key).await;
        revoke_identity_key(
            &pool,
            community_id,
            context("retire for recovery"),
            &retired_key,
        )
        .await
        .expect("revoke old key");
        let expected = get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read pending")
            .expect("pending selector");
        let mut wrong_key = expected.clone();
        wrong_key.retired_pubkey = vec![3_u8; 32];
        let mut wrong_id = expected.clone();
        wrong_id.retired_binding_id = Uuid::new_v4();
        let mut wrong_version = expected.clone();
        wrong_version.retired_binding_version += 1;
        let mut wrong_selector = expected.clone();
        wrong_selector.selector_version += 1;
        for stale in [wrong_key, wrong_id, wrong_version, wrong_selector] {
            assert!(recover_identity_binding(
                &pool,
                community_id,
                context("stale recovery"),
                principal,
                &stale,
                replacement(&replacement_key),
            )
            .await
            .is_err());
            assert_eq!(
                get_pending_lineage(&pool, community_id, principal)
                    .await
                    .expect("read unchanged pending"),
                Some(expected.clone())
            );
        }

        let recovered = recover_identity_binding(
            &pool,
            community_id,
            context("exact recovery"),
            principal,
            &expected,
            replacement(&replacement_key),
        )
        .await
        .expect("recover by exact selector");
        let LifecycleResult::Applied(receipt) = recovered else {
            panic!("first exact recovery must apply");
        };
        assert_eq!(receipt.binding_id, Some(retired_evidence.binding_id()));
        assert_eq!(
            receipt.binding_version,
            Some(expected.retired_binding_version)
        );
        assert!(receipt.replacement_binding_id.is_some());
        assert_ne!(receipt.replacement_binding_id, receipt.binding_id);
        assert_eq!(receipt.replacement_binding_version, Some(1));
        assert_eq!(receipt.selector_version, Some(expected.selector_version));
        assert!(get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read cleared pending")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn enablement_requires_exact_pending_lineage_and_restores_authority() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "enable-cas-subject",
        };
        let retired_key = [4_u8; 32];
        let replacement_key = [5_u8; 32];
        let retired_evidence = enroll(&pool, community_id, principal.subject, &retired_key).await;
        disable_identity_principal(
            &pool,
            community_id,
            context("disable before enable"),
            principal,
        )
        .await
        .expect("disable identity");
        let expected = get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read enable pending")
            .expect("enable pending selector");
        let mut stale = expected.clone();
        stale.selector_version += 1;
        assert!(enable_identity_principal(
            &pool,
            community_id,
            context("stale enable"),
            principal,
            Some(&stale),
            replacement(&replacement_key),
        )
        .await
        .is_err());
        let enable_request = context("exact enable");
        let applied = enable_identity_principal(
            &pool,
            community_id,
            enable_request,
            principal,
            Some(&expected),
            replacement(&replacement_key),
        )
        .await
        .expect("enable by exact selector");
        let LifecycleResult::Applied(receipt) = applied else {
            panic!("first exact enablement must apply");
        };
        assert_eq!(receipt.binding_id, Some(retired_evidence.binding_id()));
        assert_eq!(
            receipt.binding_version,
            Some(expected.retired_binding_version)
        );
        assert!(receipt.replacement_binding_id.is_some());
        assert_ne!(receipt.replacement_binding_id, receipt.binding_id);
        assert_eq!(receipt.replacement_binding_version, Some(1));
        assert_eq!(receipt.selector_version, Some(expected.selector_version));
        assert_eq!(
            enable_identity_principal(
                &pool,
                community_id,
                enable_request,
                principal,
                Some(&expected),
                replacement(&replacement_key),
            )
            .await
            .expect("replay exact enablement"),
            LifecycleResult::AlreadyApplied(receipt)
        );
        let resolved = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community_id,
                issuer: principal.issuer,
                subject: principal.subject,
                pubkey: &replacement_key,
                display_name: None,
                enrollment_mode: EnrollmentMode::AttestedKey,
                key_attested: true,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("authorize enabled identity");
        assert!(matches!(resolved, ResolveBindingResult::Existing(_)));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn operation_id_reuse_rejects_changed_replacement_and_pending_inputs() {
        let pool = setup_pool().await;
        for variant in 0_u8..2 {
            let community_id = make_community(&pool).await;
            let subject = format!("fingerprint-replacement-{variant}");
            let principal = IdentityPrincipal {
                issuer: ISSUER,
                subject: &subject,
            };
            let key = [60_u8 + variant; 32];
            let operation_id = Uuid::new_v4();
            let request = LifecycleContext {
                operation_id: LifecycleOperationId::from_uuid_for_test(operation_id),
                actor: &key,
                reason: "fingerprint replacement request",
            };
            let original = VerifiedReplacementKey::after_verified_proof(
                &key,
                None,
                BindingProvenance::Provisioned,
                "policy-v1",
            )
            .expect("construct original replacement");
            provision_identity_binding(
                &pool,
                community_id,
                request,
                principal,
                EnrollmentMode::Provisioned,
                original,
            )
            .await
            .expect("apply original operation");
            let changed = match variant {
                0 => VerifiedReplacementKey::after_verified_proof(
                    &key,
                    Some("changed display"),
                    BindingProvenance::AttestedKey,
                    "policy-v1",
                ),
                _ => VerifiedReplacementKey::after_verified_proof(
                    &key,
                    None,
                    BindingProvenance::Provisioned,
                    "policy-v2",
                ),
            }
            .expect("construct changed replacement");
            assert!(provision_identity_binding(
                &pool,
                community_id,
                request,
                principal,
                EnrollmentMode::Provisioned,
                changed,
            )
            .await
            .is_err());
            let counts: (i64, i64, i64) = sqlx::query_as(
                "SELECT \
                   (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
                   (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
                   (SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1)",
            )
            .bind(community_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read replacement fingerprint counts");
            assert_eq!(counts, (1, 1, 1));
        }

        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "fingerprint-provenance",
        };
        let old_key = [64_u8; 32];
        let new_key = [65_u8; 32];
        let old_evidence = enroll(&pool, community_id, principal.subject, &old_key).await;
        let request = LifecycleContext {
            operation_id: LifecycleOperationId::issue(),
            actor: &old_key,
            reason: "fingerprint rotation request",
        };
        let original = VerifiedReplacementKey::after_verified_proof(
            &new_key,
            None,
            BindingProvenance::Provisioned,
            "policy-v1",
        )
        .expect("construct provisioned rotation");
        let rotated =
            rotate_identity_binding(&pool, community_id, request, principal, &old_key, original)
                .await
                .expect("apply original rotation");
        let LifecycleResult::Applied(receipt) = rotated else {
            panic!("first rotation must apply");
        };
        assert_eq!(receipt.binding_id, Some(old_evidence.binding_id()));
        assert_eq!(
            receipt.binding_version,
            Some(old_evidence.binding_version() + 1)
        );
        assert!(receipt.replacement_binding_id.is_some());
        assert_ne!(receipt.replacement_binding_id, receipt.binding_id);
        assert_eq!(receipt.replacement_binding_version, Some(1));
        assert_eq!(receipt.selector_version, None);
        let changed = VerifiedReplacementKey::after_verified_proof(
            &new_key,
            None,
            BindingProvenance::AttestedKey,
            "policy-v1",
        )
        .expect("construct changed rotation provenance");
        assert!(rotate_identity_binding(
            &pool,
            community_id,
            request,
            principal,
            &old_key,
            changed,
        )
        .await
        .is_err());
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1)",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read provenance fingerprint counts");
        assert_eq!(counts, (2, 3, 1));

        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "fingerprint-enable",
        };
        let retired_key = [70_u8; 32];
        let replacement_key = [71_u8; 32];
        enroll(&pool, community_id, principal.subject, &retired_key).await;
        disable_identity_principal(
            &pool,
            community_id,
            context("disable fingerprint principal"),
            principal,
        )
        .await
        .expect("disable fingerprint principal");
        let expected = get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read fingerprint pending")
            .expect("fingerprint pending selector");
        let operation_id = Uuid::new_v4();
        let request = LifecycleContext {
            operation_id: LifecycleOperationId::from_uuid_for_test(operation_id),
            actor: &old_key,
            reason: "fingerprint enable request",
        };
        enable_identity_principal(
            &pool,
            community_id,
            request,
            principal,
            Some(&expected),
            replacement(&replacement_key),
        )
        .await
        .expect("apply exact enable request");
        let mut changed = [
            expected.clone(),
            expected.clone(),
            expected.clone(),
            expected.clone(),
        ];
        changed[0].retired_pubkey = vec![72_u8; 32];
        changed[1].retired_binding_id = Uuid::new_v4();
        changed[2].retired_binding_version += 1;
        changed[3].selector_version += 1;
        for stale in changed {
            assert!(enable_identity_principal(
                &pool,
                community_id,
                request,
                principal,
                Some(&stale),
                replacement(&replacement_key),
            )
            .await
            .is_err());
        }
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1), \
               (SELECT COUNT(*) FROM identity_lifecycle_operations WHERE community_id=$1 AND operation_id=$2)",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_one(&pool)
        .await
        .expect("read enable fingerprint counts");
        assert_eq!(counts, (2, 3, 1));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn arbitrary_rotation_history_is_lossless_without_new_key_tombstones() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "rotations",
        };
        let keys = [[10_u8; 32], [11_u8; 32], [12_u8; 32], [13_u8; 32]];
        enroll(&pool, community_id, principal.subject, &keys[0]).await;
        for pair in keys.windows(2) {
            rotate_identity_binding(
                &pool,
                community_id,
                context("verified rotation"),
                principal,
                &pair[0],
                replacement(&pair[1]),
            )
            .await
            .expect("rotate binding");
        }

        let versions: Vec<(Vec<u8>, i64, String)> = sqlx::query_as(
            "SELECT pubkey, binding_version, binding_state FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3 ORDER BY pubkey",
        )
        .bind(community_id.as_uuid())
        .bind(principal.issuer)
        .bind(principal.subject)
        .fetch_all(&pool)
        .await
        .expect("read rotation history");
        assert_eq!(versions.len(), keys.len());
        let expected_versions = [2_i64, 2, 2, 1];
        for (index, (pubkey, version, state)) in versions.iter().enumerate() {
            assert_eq!(pubkey, &keys[index]);
            assert_eq!(*version, expected_versions[index]);
            assert_eq!(
                state,
                if index + 1 == keys.len() {
                    "active"
                } else {
                    "rotated"
                }
            );
        }
        let fresh_tombstones: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM identity_revoked_keys WHERE community_id=$1")
                .bind(community_id.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count tombstones");
        assert_eq!(fresh_tombstones, 0);
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn migration_denial_blocks_authorization_and_lifecycle() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "ambiguous",
        };
        let key = [21_u8; 32];
        enroll(&pool, community_id, principal.subject, &key).await;
        sqlx::query(
            "INSERT INTO identity_migration_denials (community_id, issuer, subject, reason) \
             VALUES ($1,$2,$3,'ambiguous test lineage')",
        )
        .bind(community_id.as_uuid())
        .bind(principal.issuer)
        .bind(principal.subject)
        .execute(&pool)
        .await
        .expect("quarantine principal");

        assert_eq!(
            resolve_identity_binding(
                &pool,
                &ResolveBindingInput {
                    authorization_domain: community_id,
                    issuer: principal.issuer,
                    subject: principal.subject,
                    pubkey: &key,
                    display_name: None,
                    enrollment_mode: EnrollmentMode::AttestedKey,
                    key_attested: true,
                    policy_version: "test-policy-v1",
                    evidence_valid_from: 0,
                    evidence_valid_until: i64::MAX as u64,
                },
            )
            .await
            .expect("resolve quarantined binding"),
            ResolveBindingResult::Denied(BindingDenial::Revoked)
        );
        assert!(disable_identity_principal(
            &pool,
            community_id,
            context("must not mutate quarantine"),
            principal,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn concurrent_enrollment_and_disablement_finish_fail_closed() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let gate = Arc::new(tokio::sync::Barrier::new(3));
        let enroll_pool = pool.clone();
        let enroll_gate = Arc::clone(&gate);
        let enroll_task = tokio::spawn(async move {
            let key = [31_u8; 32];
            enroll_gate.wait().await;
            resolve_identity_binding(
                &enroll_pool,
                &ResolveBindingInput {
                    authorization_domain: community_id,
                    issuer: ISSUER,
                    subject: "racing-subject",
                    pubkey: &key,
                    display_name: None,
                    enrollment_mode: EnrollmentMode::AttestedKey,
                    key_attested: true,
                    policy_version: "test-policy-v1",
                    evidence_valid_from: 0,
                    evidence_valid_until: i64::MAX as u64,
                },
            )
            .await
        });
        let disable_pool = pool.clone();
        let disable_gate = Arc::clone(&gate);
        let disable_task = tokio::spawn(async move {
            disable_gate.wait().await;
            disable_identity_principal(
                &disable_pool,
                community_id,
                context("concurrent disable"),
                IdentityPrincipal {
                    issuer: ISSUER,
                    subject: "racing-subject",
                },
            )
            .await
        });
        gate.wait().await;
        enroll_task
            .await
            .expect("join enrollment")
            .expect("enrollment result");
        disable_task
            .await
            .expect("join disablement")
            .expect("disablement result");

        let state: (bool, bool) = sqlx::query_as(
            "SELECT \
               EXISTS(SELECT 1 FROM identity_principals WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND disabled_at IS NOT NULL), \
               EXISTS(SELECT 1 FROM identity_bindings WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND binding_state='active' AND revoked_at IS NULL)",
        )
        .bind(community_id.as_uuid())
        .bind(ISSUER)
        .bind("racing-subject")
        .fetch_one(&pool)
        .await
        .expect("read final race state");
        assert_eq!(state, (true, false));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn identical_identity_and_key_coordinates_are_independent_across_domains() {
        let pool = setup_pool().await;
        let domains = [make_community(&pool).await, make_community(&pool).await];
        let key = [79_u8; 32];
        let gate = Arc::new(tokio::sync::Barrier::new(3));
        let mut tasks = Vec::new();
        for community_id in domains {
            let task_pool = pool.clone();
            let task_gate = Arc::clone(&gate);
            tasks.push(tokio::spawn(async move {
                task_gate.wait().await;
                resolve_identity_binding(
                    &task_pool,
                    &ResolveBindingInput {
                        authorization_domain: community_id,
                        issuer: ISSUER,
                        subject: "same-cross-domain-principal",
                        pubkey: &key,
                        display_name: None,
                        enrollment_mode: EnrollmentMode::AttestedKey,
                        key_attested: true,
                        policy_version: "test-policy-v1",
                        evidence_valid_from: 0,
                        evidence_valid_until: i64::MAX as u64,
                    },
                )
                .await
            }));
        }
        gate.wait().await;
        for task in tasks {
            assert!(matches!(
                task.await
                    .expect("join cross-domain enrollment")
                    .expect("cross-domain enrollment result"),
                ResolveBindingResult::Enrolled(_)
            ));
        }
        for community_id in domains {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM identity_bindings \
                 WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND pubkey=$4 \
                   AND binding_state='active' AND revoked_at IS NULL",
            )
            .bind(community_id.as_uuid())
            .bind(ISSUER)
            .bind("same-cross-domain-principal")
            .bind(key)
            .fetch_one(&pool)
            .await
            .expect("count cross-domain binding");
            assert_eq!(count, 1);
        }
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn competing_recoveries_and_enablements_have_one_winner() {
        let pool = setup_pool().await;
        for enable in [false, true] {
            let community_id = make_community(&pool).await;
            let subject = if enable {
                "competing-enable"
            } else {
                "competing-recovery"
            };
            let principal = IdentityPrincipal {
                issuer: ISSUER,
                subject,
            };
            let retired_key = [if enable { 81_u8 } else { 80_u8 }; 32];
            enroll(&pool, community_id, subject, &retired_key).await;
            if enable {
                disable_identity_principal(
                    &pool,
                    community_id,
                    context("prepare competing enable"),
                    principal,
                )
                .await
                .expect("disable before competing enable");
            } else {
                revoke_identity_key(
                    &pool,
                    community_id,
                    context("prepare competing recovery"),
                    &retired_key,
                )
                .await
                .expect("revoke before competing recovery");
            }
            let expected = get_pending_lineage(&pool, community_id, principal)
                .await
                .expect("read competing pending")
                .expect("competing pending selector");
            let gate = Arc::new(tokio::sync::Barrier::new(3));
            let mut tasks = Vec::new();
            for byte in [82_u8, 83_u8] {
                let task_pool = pool.clone();
                let task_gate = Arc::clone(&gate);
                let task_expected = expected.clone();
                tasks.push(tokio::spawn(async move {
                    let replacement_key = [byte; 32];
                    task_gate.wait().await;
                    if enable {
                        enable_identity_principal(
                            &task_pool,
                            community_id,
                            context("competing enable"),
                            principal,
                            Some(&task_expected),
                            replacement(&replacement_key),
                        )
                        .await
                    } else {
                        recover_identity_binding(
                            &task_pool,
                            community_id,
                            context("competing recovery"),
                            principal,
                            &task_expected,
                            replacement(&replacement_key),
                        )
                        .await
                    }
                }));
            }
            gate.wait().await;
            let first = tasks.remove(0).await.expect("join first competitor");
            let second = tasks.remove(0).await.expect("join second competitor");
            assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
            let state: (i64, bool, bool) = sqlx::query_as(
                "SELECT \
                   (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND binding_state='active' AND revoked_at IS NULL), \
                   EXISTS(SELECT 1 FROM identity_principals WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND disabled_at IS NOT NULL), \
                   EXISTS(SELECT 1 FROM identity_pending_replacements WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL)",
            )
            .bind(community_id.as_uuid())
            .bind(principal.issuer)
            .bind(principal.subject)
            .fetch_one(&pool)
            .await
            .expect("read competing transition state");
            assert_eq!(state, (1, false, false));
        }
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn recovery_disable_and_enable_revoke_races_fail_closed() {
        let pool = setup_pool().await;

        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "recover-vs-disable",
        };
        let retired_key = [84_u8; 32];
        let replacement_key = [85_u8; 32];
        enroll(&pool, community_id, principal.subject, &retired_key).await;
        revoke_identity_key(
            &pool,
            community_id,
            context("prepare recover disable race"),
            &retired_key,
        )
        .await
        .expect("prepare recovery pending");
        let expected = get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read recovery pending")
            .expect("recovery pending selector");
        let gate = Arc::new(tokio::sync::Barrier::new(3));
        let recover_pool = pool.clone();
        let recover_gate = Arc::clone(&gate);
        let recover_task = tokio::spawn(async move {
            recover_gate.wait().await;
            recover_identity_binding(
                &recover_pool,
                community_id,
                context("racing recovery"),
                principal,
                &expected,
                replacement(&replacement_key),
            )
            .await
        });
        let disable_pool = pool.clone();
        let disable_gate = Arc::clone(&gate);
        let disable_task = tokio::spawn(async move {
            disable_gate.wait().await;
            disable_identity_principal(
                &disable_pool,
                community_id,
                context("racing disable"),
                principal,
            )
            .await
        });
        gate.wait().await;
        let _ = recover_task.await.expect("join racing recovery");
        disable_task
            .await
            .expect("join racing disable")
            .expect("disable wins or follows recovery");
        let state: (bool, bool, bool) = sqlx::query_as(
            "SELECT \
               EXISTS(SELECT 1 FROM identity_principals WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND disabled_at IS NOT NULL), \
               EXISTS(SELECT 1 FROM identity_bindings WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND binding_state='active' AND revoked_at IS NULL), \
               EXISTS(SELECT 1 FROM identity_pending_replacements WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND cleared_at IS NULL)",
        )
        .bind(community_id.as_uuid())
        .bind(principal.issuer)
        .bind(principal.subject)
        .fetch_one(&pool)
        .await
        .expect("read recover disable final state");
        assert_eq!(state, (true, false, true));

        let community_id = make_community(&pool).await;
        let principal = IdentityPrincipal {
            issuer: ISSUER,
            subject: "enable-vs-revoke",
        };
        let retired_key = [86_u8; 32];
        let replacement_key = [87_u8; 32];
        enroll(&pool, community_id, principal.subject, &retired_key).await;
        disable_identity_principal(
            &pool,
            community_id,
            context("prepare enable revoke race"),
            principal,
        )
        .await
        .expect("prepare disabled principal");
        let expected = get_pending_lineage(&pool, community_id, principal)
            .await
            .expect("read enable pending")
            .expect("enable pending selector");
        let gate = Arc::new(tokio::sync::Barrier::new(3));
        let enable_pool = pool.clone();
        let enable_gate = Arc::clone(&gate);
        let enable_task = tokio::spawn(async move {
            enable_gate.wait().await;
            enable_identity_principal(
                &enable_pool,
                community_id,
                context("racing enable"),
                principal,
                Some(&expected),
                replacement(&replacement_key),
            )
            .await
        });
        let revoke_pool = pool.clone();
        let revoke_gate = Arc::clone(&gate);
        let revoke_task = tokio::spawn(async move {
            revoke_gate.wait().await;
            revoke_identity_key(
                &revoke_pool,
                community_id,
                context("racing replacement revoke"),
                &replacement_key,
            )
            .await
        });
        gate.wait().await;
        let _ = enable_task.await.expect("join racing enable");
        revoke_task
            .await
            .expect("join replacement revoke")
            .expect("replacement revoke commits");
        let state: (bool, bool, bool) = sqlx::query_as(
            "SELECT \
               EXISTS(SELECT 1 FROM identity_bindings binding \
                      JOIN identity_revoked_keys revoked USING (community_id,pubkey) \
                      WHERE binding.community_id=$1 AND binding.binding_state='active' AND binding.revoked_at IS NULL), \
               EXISTS(SELECT 1 FROM identity_bindings WHERE community_id=$1 AND pubkey=$2 AND binding_state='active' AND revoked_at IS NULL), \
               EXISTS(SELECT 1 FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2)",
        )
        .bind(community_id.as_uuid())
        .bind(replacement_key)
        .fetch_one(&pool)
        .await
        .expect("read enable revoke final state");
        assert_eq!(state, (false, false, true));
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn rotation_against_old_or_new_key_revocation_never_resurrects_authority() {
        let pool = setup_pool().await;
        for revoke_new in [false, true] {
            let community_id = make_community(&pool).await;
            let subject = if revoke_new {
                "rotate-vs-new-revoke"
            } else {
                "rotate-vs-old-revoke"
            };
            let principal = IdentityPrincipal {
                issuer: ISSUER,
                subject,
            };
            let old_key = [if revoke_new { 88_u8 } else { 90_u8 }; 32];
            let new_key = [if revoke_new { 89_u8 } else { 91_u8 }; 32];
            let revoked_key = if revoke_new { new_key } else { old_key };
            enroll(&pool, community_id, subject, &old_key).await;
            let gate = Arc::new(tokio::sync::Barrier::new(3));
            let rotate_pool = pool.clone();
            let rotate_gate = Arc::clone(&gate);
            let rotate_task = tokio::spawn(async move {
                rotate_gate.wait().await;
                rotate_identity_binding(
                    &rotate_pool,
                    community_id,
                    context("racing rotation"),
                    principal,
                    &old_key,
                    replacement(&new_key),
                )
                .await
            });
            let revoke_pool = pool.clone();
            let revoke_gate = Arc::clone(&gate);
            let revoke_task = tokio::spawn(async move {
                revoke_gate.wait().await;
                revoke_identity_key(
                    &revoke_pool,
                    community_id,
                    context("racing key revoke"),
                    &revoked_key,
                )
                .await
            });
            gate.wait().await;
            let _ = rotate_task.await.expect("join racing rotation");
            revoke_task
                .await
                .expect("join racing key revoke")
                .expect("key revocation commits");
            let state: (i64, bool, bool) = sqlx::query_as(
                "SELECT \
                   (SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1 AND binding_state='active' AND revoked_at IS NULL), \
                   EXISTS(SELECT 1 FROM identity_bindings binding \
                          JOIN identity_revoked_keys revoked USING (community_id,pubkey) \
                          WHERE binding.community_id=$1 AND binding.binding_state='active' AND binding.revoked_at IS NULL), \
                   EXISTS(SELECT 1 FROM identity_bindings binding \
                          JOIN identity_retired_pairs retired \
                            ON retired.community_id=binding.community_id \
                           AND retired.issuer=binding.issuer \
                           AND retired.subject=binding.uid \
                           AND retired.pubkey=binding.pubkey \
                          WHERE binding.community_id=$1 AND binding.binding_state='active' AND binding.revoked_at IS NULL)",
            )
            .bind(community_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("read rotate revoke final state");
            assert!(state.0 <= 1);
            assert_eq!((state.1, state.2), (false, false));
        }
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable Postgres database"]
    async fn lifecycle_failure_rolls_back_selectors_history_and_authority() {
        let pool = setup_pool().await;
        let community_id = make_community(&pool).await;
        let key = [41_u8; 32];
        enroll(&pool, community_id, "rollback-subject", &key).await;
        let reason = "identity lifecycle failure injection";
        sqlx::query(
            "CREATE OR REPLACE FUNCTION identity_test_fail_history() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF NEW.reason = 'identity lifecycle failure injection' THEN RAISE EXCEPTION 'injected history failure'; END IF; RETURN NEW; END $$",
        )
        .execute(&pool)
        .await
        .expect("create failure function");
        sqlx::query(
            "CREATE TRIGGER identity_test_fail_history_trigger BEFORE INSERT ON identity_binding_history \
             FOR EACH ROW EXECUTE FUNCTION identity_test_fail_history()",
        )
        .execute(&pool)
        .await
        .expect("create failure trigger");

        let failed = revoke_identity_key(&pool, community_id, context(reason), &key)
            .await
            .is_err();
        let state: (bool, bool, bool, bool) = sqlx::query_as(
            "SELECT \
               EXISTS(SELECT 1 FROM identity_bindings WHERE community_id=$1 AND pubkey=$2 AND binding_state='active' AND revoked_at IS NULL), \
               EXISTS(SELECT 1 FROM identity_revoked_keys WHERE community_id=$1 AND pubkey=$2), \
               EXISTS(SELECT 1 FROM identity_retired_pairs WHERE community_id=$1 AND pubkey=$2), \
               EXISTS(SELECT 1 FROM identity_pending_replacements WHERE community_id=$1 AND retired_pubkey=$2 AND cleared_at IS NULL)",
        )
        .bind(community_id.as_uuid())
        .bind(key)
        .fetch_one(&pool)
        .await
        .expect("read rollback state");

        sqlx::query("DROP TRIGGER identity_test_fail_history_trigger ON identity_binding_history")
            .execute(&pool)
            .await
            .expect("drop failure trigger");
        sqlx::query("DROP FUNCTION identity_test_fail_history()")
            .execute(&pool)
            .await
            .expect("drop failure function");
        assert!(failed);
        assert_eq!(state, (true, false, false, false));
    }
}
