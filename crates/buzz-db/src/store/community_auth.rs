//! Community authentication policy management and durable operator receipts.
//!
//! Receipts are deployment-global and intentionally survive target deletion:
//! an operation UUID must never acquire a new meaning after a lost response.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use serde::Serialize;
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::{Db, Result};

/// Fleet protocol bound for a cached host's policy, measured before checkout.
/// Increasing this requires a coordinated acceptance-writer rollout first.
pub const POLICY_LEASE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(1);
// Allow clock-rate drift between pods. Never shorten below the fleet lease bound.
const POLICY_LEASE_DRAIN: std::time::Duration =
    POLICY_LEASE_MAX_AGE.saturating_add(std::time::Duration::from_millis(100));
// Two-int advisory key space, separate from the existing bigint lock protocols.
const POLICY_LEASE_NAMESPACE: i32 = 0x4649504c;

/// Canonical request tuple, with the authenticated operator added by the relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeRequest {
    /// Caller-generated, deployment-global idempotency UUID.
    pub operation_id: Uuid,
    /// Immutable community identity.
    pub community_id: Uuid,
    /// Normalized expected community host.
    pub expected_host: String,
    /// Registered key of the authenticated invoking owner.
    pub actor_pubkey: String,
    /// Owner precondition, checked under the community row lock.
    pub expected_owner_pubkey: String,
    /// Exact issuer selected by trusted deployment configuration.
    pub expected_issuer: String,
    /// NIP-98 authenticated deployment operator; never taken from request JSON.
    pub operator_pubkey: String,
}

/// General policy intent; both preconditions are checked under the writer row lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRequest {
    /// Canonical operation tuple. Disable uses an empty issuer.
    pub tuple: UpgradeRequest,
    /// Desired stored requirement.
    pub requires_nip_fi: bool,
    /// Last observed generation.
    pub expected_policy_generation: i64,
    /// Last observed stored requirement.
    pub expected_requires_nip_fi: bool,
}

/// Durable accepted intent and the current primary-database snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
pub struct PolicyRecord {
    /// Original operation tuple and accepted generation, never current authority.
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub acceptance: UpgradeRecord,
    /// Null only for legacy, unfenced requests.
    pub expected_policy_generation: Option<i64>,
    /// Null only for legacy, unfenced requests.
    pub expected_requires_nip_fi: Option<bool>,
    /// Current target generation, absent after target deletion.
    pub current_policy_generation: Option<i64>,
    /// Current stored flag, absent after target deletion.
    pub current_requires_nip_fi: Option<bool>,
}

/// Refusal or an immutable acceptance with current status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyOutcome {
    /// Durable acceptance or identical read-only replay.
    Accepted(Box<PolicyRecord>),
    /// No mutation or receipt was committed.
    Refused(&'static str),
}

/// Durable acceptance tuple plus the current availability of its target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
pub struct UpgradeRecord {
    /// Caller idempotency UUID.
    pub operation_id: Uuid,
    /// Target identity, preserved after deletion.
    #[sqlx(rename = "target_community_id")]
    pub community_id: Uuid,
    /// Host at acceptance.
    pub expected_host: String,
    /// Invoking owner's registered key at acceptance.
    pub actor_pubkey: String,
    /// Owner precondition at acceptance.
    pub expected_owner_pubkey: String,
    /// Issuer precondition at acceptance; omitted for disable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub expected_issuer: String,
    /// Operator authorized to recover this receipt.
    pub operator_pubkey: String,
    /// Immutable generation accepted for this operation.
    pub policy_generation: i64,
    /// Receipt proves acceptance; availability does not undo this fact.
    pub requires_nip_fi: bool,
    /// Current availability, with no certification of fleet-wide socket closure.
    pub state: String,
    /// Extensible activation obligations; unknown reasons must remain pending.
    pub pending_reasons: Vec<String>,
}

impl UpgradeRecord {
    fn matches(&self, request: &UpgradeRequest) -> bool {
        self.operation_id == request.operation_id
            && self.community_id == request.community_id
            && self.expected_host == request.expected_host
            && self.actor_pubkey == request.actor_pubkey
            && self.expected_owner_pubkey == request.expected_owner_pubkey
            && self.expected_issuer == request.expected_issuer
            && self.operator_pubkey == request.operator_pubkey
    }
}

/// Authoritative policy for a community in the active lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct CommunityAuthPolicy {
    /// Immutable identity from the same row as this policy.
    #[sqlx(rename = "id")]
    pub community_id: Uuid,
    /// Durable stored requirement. Global enforce remains an additional floor.
    pub requires_nip_fi: bool,
    /// Zero before the first operation; increases with every accepted CAS intent.
    pub policy_generation: i64,
}

// One statement gives status a single primary-database snapshot. Missing trust,
// host/policy drift and terminal lifecycle never become a usable active result.
const RECEIPT: &str = "SELECT u.operation_id, u.target_community_id, u.expected_host,
    u.actor_pubkey, u.expected_owner_pubkey, u.expected_issuer, u.operator_pubkey,
    u.policy_generation, u.requested_requires_nip_fi AS requires_nip_fi,
    u.expected_policy_generation, u.expected_requires_nip_fi,
    c.policy_generation AS current_policy_generation,
    c.requires_nip_fi AS current_requires_nip_fi,
    CASE WHEN c.id IS NULL OR c.host <> u.expected_host
        OR c.archived_at IS NOT NULL OR c.deleted_at IS NOT NULL
        OR c.deletion_state <> 'active'
        OR EXISTS (SELECT 1 FROM community_deletion_requests d
            WHERE d.community_id = c.id AND d.stage <> 'aborted') THEN 'unavailable'
    WHEN c.policy_generation <> u.policy_generation
        OR c.requires_nip_fi <> u.requested_requires_nip_fi THEN 'superseded'
    WHEN NOT u.requested_requires_nip_fi THEN 'applied'
    WHEN EXISTS (SELECT 1 FROM community_nip_fi_issuers i
            WHERE i.community_id = c.id AND i.issuer = u.expected_issuer)
        THEN 'pending' ELSE 'unavailable' END AS state,
    CASE WHEN u.requested_requires_nip_fi
        THEN ARRAY['connection_closure_unverified']::text[]
        ELSE ARRAY[]::text[] END AS pending_reasons
    FROM community_auth_upgrades u
    LEFT JOIN communities c ON c.id = u.target_community_id
    WHERE u.operation_id = $1";

impl Db {
    /// Read a cacheable serving policy under the host's shared lease lock.
    ///
    /// Callers must anchor expiry BEFORE invoking this method and refuse an
    /// expired result. Acceptance excludes new leases and drains old ones before
    /// committing. The lock and row read MUST be separate READ COMMITTED
    /// statements: a snapshot from before lock acquisition is not a lease.
    /// Contention refuses immediately; it never leaves a server-side lock wait.
    pub async fn community_auth_policy_lease_for_host(
        &self,
        host: &str,
    ) -> Result<Option<CommunityAuthPolicy>> {
        let host = buzz_core::tenant::normalize_host(host);
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '400ms'")
            .execute(&mut *tx)
            .await?;
        let acquired: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock_shared($1, hashtext($2))")
                .bind(POLICY_LEASE_NAMESPACE)
                .bind(&host)
                .fetch_one(&mut *tx)
                .await?;
        if !acquired {
            return Err(crate::DbError::AccessDenied(
                "community policy transition".into(),
            ));
        }
        let policy = sqlx::query_as(super::community::SERVING_HOST_QUERY)
            .bind(&host)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(policy)
    }

    /// Bind a serving host and its authentication policy from one writer row.
    /// A lifecycle transition between earlier checks and this lookup cannot
    /// supply a tenant with another snapshot's optional policy.
    #[datastore_span(name = "community_auth_binding_for_host", system = "postgresql")]
    pub async fn community_auth_binding_for_host(
        &self,
        host: &str,
    ) -> Result<Option<(crate::CommunityRecord, CommunityAuthPolicy)>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::TenantResolution,
        )
        .await?;
        let row = sqlx::query(super::community::SERVING_HOST_QUERY)
            .bind(host)
            .fetch_optional(&mut *connection)
            .await?;
        row.map(|row| {
            Ok((
                crate::CommunityRecord {
                    id: CommunityId::from_uuid(row.try_get("id")?),
                    host: row.try_get("host")?,
                },
                CommunityAuthPolicy {
                    community_id: row.try_get("id")?,
                    requires_nip_fi: row.try_get("requires_nip_fi")?,
                    policy_generation: row.try_get("policy_generation")?,
                },
            ))
        })
        .transpose()
    }

    /// Resolve active host policy in one writer read. No replica or cross-request cache.
    #[datastore_span(name = "community_auth_policy_for_host", system = "postgresql")]
    pub async fn community_auth_policy_for_host(
        &self,
        host: &str,
    ) -> Result<Option<CommunityAuthPolicy>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        Ok(sqlx::query_as(super::community::SERVING_HOST_QUERY)
            .bind(host)
            .fetch_optional(&mut *connection)
            .await?)
    }

    /// Read current active-community policy from the writer, never a replica.
    #[datastore_span(name = "community_auth_policy", system = "postgresql")]
    pub async fn community_auth_policy(
        &self,
        community: CommunityId,
    ) -> Result<Option<CommunityAuthPolicy>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        Ok(sqlx::query_as("SELECT id, requires_nip_fi, policy_generation FROM communities
            WHERE id = $1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state = 'active'")
            .bind(community.as_uuid()).fetch_optional(&mut *connection).await?)
    }

    /// Recover any policy receipt for its original authenticated operator.
    pub async fn community_auth_policy_status(
        &self,
        operation: Uuid,
        operator: &str,
    ) -> Result<Option<PolicyRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let record: Option<PolicyRecord> = sqlx::query_as(RECEIPT)
            .bind(operation)
            .fetch_optional(&mut *connection)
            .await?;
        Ok(record.filter(|record| record.acceptance.operator_pubkey == operator))
    }

    /// Apply a compare-and-set policy intent; identical committed replay is read-only.
    pub async fn set_community_nip_fi_policy(
        &self,
        request: &PolicyRequest,
        configured_trust: std::result::Result<(), &'static str>,
    ) -> Result<PolicyOutcome> {
        let desired = request.requires_nip_fi;
        let expected_generation = request.expected_policy_generation;
        let expected_state = request.expected_requires_nip_fi;
        let request = &request.tuple;
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        // Lock UUID first across all targets, so cross-community UUID reuse
        // cannot race the INSERT. Hash collisions merely serialize callers.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 510))")
            .bind(request.operation_id.to_string())
            .execute(&mut *tx)
            .await?;
        let existing: Option<PolicyRecord> = sqlx::query_as(RECEIPT)
            .bind(request.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(record) = existing {
            return Ok(
                if record.acceptance.matches(request)
                    && record.acceptance.requires_nip_fi == desired
                    && record.expected_policy_generation == Some(expected_generation)
                    && record.expected_requires_nip_fi == Some(expected_state)
                {
                    PolicyOutcome::Accepted(Box::new(record))
                } else {
                    PolicyOutcome::Refused("idempotency_conflict")
                },
            );
        }
        if let Err(code) = configured_trust {
            return Ok(PolicyOutcome::Refused(code));
        }
        let community = CommunityId::from_uuid(request.community_id);
        crate::deletion::lock_community_deletion_shared(&mut tx, community).await?;
        // Same order and lock strength as ownership transfer; archive/delete
        // also take this row lock. Read ownership only after acquiring it.
        let row = sqlx::query(
            "SELECT host, archived_at IS NULL AND deleted_at IS NULL
            AND deletion_state = 'active' AS active, requires_nip_fi, policy_generation
            FROM communities WHERE id = $1 FOR NO KEY UPDATE",
        )
        .bind(request.community_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(PolicyOutcome::Refused("community_not_found"));
        };
        if row.try_get::<String, _>("host")? != request.expected_host {
            return Ok(PolicyOutcome::Refused("host_conflict"));
        }
        let old_required: bool = row.try_get("requires_nip_fi")?;
        let old_generation: i64 = row.try_get("policy_generation")?;
        if expected_generation != old_generation || expected_state != old_required {
            return Ok(PolicyOutcome::Refused("policy_conflict"));
        }
        if old_generation == i64::MAX {
            return Ok(PolicyOutcome::Refused("policy_generation_exhausted"));
        }
        // Lock order: deletion-shared, community row, host lease. Fills take
        // only the shared host lease and never a community row lock. Derive
        // the key from the locked, validated row, not an earlier host snapshot.
        let lease_host = buzz_core::tenant::normalize_host(&row.try_get::<String, _>("host")?);
        sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))")
            .bind(POLICY_LEASE_NAMESPACE)
            .bind(&lease_host)
            .execute(&mut *tx)
            .await?;
        // Keep the row stable while ALL optional leases expire, including
        // leases under former host keys. No remap (single or double) can
        // refill another host's optional cache while this drain is running.
        if desired && !old_required {
            tokio::time::sleep(POLICY_LEASE_DRAIN).await;
        }
        let deletion_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM
            community_deletion_requests WHERE community_id = $1 AND stage <> 'aborted')",
        )
        .bind(request.community_id)
        .fetch_one(&mut *tx)
        .await?;
        if !row.try_get::<bool, _>("active")? || deletion_pending {
            return Ok(PolicyOutcome::Refused("lifecycle_conflict"));
        }
        let owners: Vec<String> = sqlx::query_scalar(
            "SELECT pubkey FROM relay_members
            WHERE community_id = $1 AND role = 'owner' ORDER BY pubkey FOR UPDATE",
        )
        .bind(request.community_id)
        .fetch_all(&mut *tx)
        .await?;
        if owners != [request.expected_owner_pubkey.clone()]
            || request.actor_pubkey != request.expected_owner_pubkey
        {
            return Ok(PolicyOutcome::Refused("owner_conflict"));
        }
        if desired {
            let issuer: Option<String> = sqlx::query_scalar(
                "SELECT issuer FROM community_nip_fi_issuers
            WHERE community_id = $1 AND issuer = $2 FOR UPDATE",
            )
            .bind(request.community_id)
            .bind(&request.expected_issuer)
            .fetch_optional(&mut *tx)
            .await?;
            if issuer.is_none() {
                return Ok(PolicyOutcome::Refused("issuer_conflict"));
            }
        }
        // Admission is also required for this communities-row mutation.
        self.deletion_store()
            .guard_transaction(&mut tx, community)
            .await?;
        let generation = old_generation + 1;
        sqlx::query(
            "UPDATE communities SET policy_generation = $2, requires_nip_fi = $3 WHERE id = $1",
        )
        .bind(request.community_id)
        .bind(generation)
        .bind(desired)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO community_auth_upgrades
            (operation_id, target_community_id, expected_host, actor_pubkey, expected_owner_pubkey,
             expected_issuer, operator_pubkey, policy_generation, requested_requires_nip_fi,
             expected_policy_generation, expected_requires_nip_fi) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(request.operation_id)
        .bind(request.community_id)
        .bind(&request.expected_host)
        .bind(&request.actor_pubkey)
        .bind(&request.expected_owner_pubkey)
        .bind(&request.expected_issuer)
        .bind(&request.operator_pubkey)
        .bind(generation)
        .bind(desired)
        .bind(expected_generation)
        .bind(expected_state)
        .execute(&mut *tx)
        .await?;
        let record = sqlx::query_as(RECEIPT)
            .bind(request.operation_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(PolicyOutcome::Accepted(Box::new(record)))
    }
}

#[cfg(test)]
mod postgres_tests;
