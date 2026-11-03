//! Monotonic community authentication requirements and durable operator receipts.
//!
//! Receipts are deployment-global and intentionally survive target deletion:
//! an operation UUID must never acquire a new meaning after a lost response.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use serde::Serialize;
use sqlx::{FromRow, Row};
use uuid::Uuid;

use crate::{Db, Result};

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
    /// Issuer precondition at acceptance.
    pub expected_issuer: String,
    /// Operator authorized to recover this receipt.
    pub operator_pubkey: String,
    /// Accepted generation; concurrent enables converge on this value.
    pub policy_generation: i64,
    /// Receipt proves acceptance; availability does not undo this fact.
    pub requires_nip_fi: bool,
    /// Pending until BUZZ-512 supplies applied-and-drained evidence.
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

/// A refused enable never mutates policy or creates a receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// Committed or recovered acceptance, including current target state.
    Accepted(UpgradeRecord),
    /// Stable machine-readable precondition failure.
    Refused(&'static str),
}

/// Authoritative policy for a community in the active lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct CommunityAuthPolicy {
    /// Durable monotonic requirement. Global enforce remains an additional floor.
    pub requires_nip_fi: bool,
    /// Zero for legacy optional communities; positive after the first enable.
    pub policy_generation: i64,
}

// One statement gives status a single primary-database snapshot. Missing trust,
// host/policy drift and terminal lifecycle never become a usable active result.
const RECEIPT: &str = "SELECT u.*, true AS requires_nip_fi,
    CASE WHEN c.id IS NOT NULL AND c.host = u.expected_host
        AND c.archived_at IS NULL AND c.deleted_at IS NULL
        AND c.deletion_state = 'active' AND c.requires_nip_fi
        AND c.policy_generation = u.policy_generation
        AND NOT EXISTS (SELECT 1 FROM community_deletion_requests d
            WHERE d.community_id = c.id AND d.stage <> 'aborted')
        AND EXISTS (SELECT 1 FROM community_nip_fi_issuers i
            WHERE i.community_id = c.id AND i.issuer = u.expected_issuer)
    THEN 'pending' ELSE 'unavailable' END AS state,
    ARRAY['registry_baseline_unverified']::text[] AS pending_reasons
    FROM community_auth_upgrades u
    LEFT JOIN communities c ON c.id = u.target_community_id
    WHERE u.operation_id = $1";

impl Db {
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
        Ok(sqlx::query_as("SELECT requires_nip_fi, policy_generation FROM communities
            WHERE id = $1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state = 'active'")
            .bind(community.as_uuid()).fetch_optional(&mut *connection).await?)
    }

    /// Recover a receipt only for its authenticated originating operator.
    /// Ownership changes do not destroy the original caller's recovery authority.
    #[datastore_span(name = "community_auth_upgrade_status", system = "postgresql")]
    pub async fn community_auth_upgrade_status(
        &self,
        operation: Uuid,
        operator: &str,
    ) -> Result<Option<UpgradeRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let record: Option<UpgradeRecord> = sqlx::query_as(RECEIPT)
            .bind(operation)
            .fetch_optional(&mut *connection)
            .await?;
        Ok(record.filter(|record| record.operator_pubkey == operator))
    }

    /// Atomically enable FI and persist the full idempotency tuple.
    ///
    /// `configured_trust` is the relay's trusted configuration check. Stored
    /// binding presence is rechecked under a row lock here. No issuer is added.
    /// Retries recover acceptance before rechecking mutable preconditions.
    #[datastore_span(name = "enable_community_nip_fi", system = "postgresql")]
    pub async fn enable_community_nip_fi(
        &self,
        request: &UpgradeRequest,
        configured_trust: std::result::Result<(), &'static str>,
    ) -> Result<UpgradeOutcome> {
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
        let existing: Option<UpgradeRecord> = sqlx::query_as(RECEIPT)
            .bind(request.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(record) = existing {
            return Ok(if record.matches(request) {
                UpgradeOutcome::Accepted(record)
            } else {
                UpgradeOutcome::Refused("idempotency_conflict")
            });
        }
        if let Err(code) = configured_trust {
            return Ok(UpgradeOutcome::Refused(code));
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
            return Ok(UpgradeOutcome::Refused("community_not_found"));
        };
        if row.try_get::<String, _>("host")? != request.expected_host {
            return Ok(UpgradeOutcome::Refused("host_conflict"));
        }
        let deletion_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM
            community_deletion_requests WHERE community_id = $1 AND stage <> 'aborted')",
        )
        .bind(request.community_id)
        .fetch_one(&mut *tx)
        .await?;
        if !row.try_get::<bool, _>("active")? || deletion_pending {
            return Ok(UpgradeOutcome::Refused("lifecycle_conflict"));
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
            return Ok(UpgradeOutcome::Refused("owner_conflict"));
        }
        let issuer: Option<String> = sqlx::query_scalar(
            "SELECT issuer FROM community_nip_fi_issuers
            WHERE community_id = $1 AND issuer = $2 FOR UPDATE",
        )
        .bind(request.community_id)
        .bind(&request.expected_issuer)
        .fetch_optional(&mut *tx)
        .await?;
        if issuer.is_none() {
            return Ok(UpgradeOutcome::Refused("issuer_conflict"));
        }
        // Admission is also required for this communities-row mutation.
        self.deletion_store()
            .guard_transaction(&mut tx, community)
            .await?;
        let generation: i64 = sqlx::query_scalar("UPDATE communities SET
            policy_generation = CASE WHEN requires_nip_fi THEN policy_generation ELSE policy_generation + 1 END,
            requires_nip_fi = true WHERE id = $1 RETURNING policy_generation")
            .bind(request.community_id).fetch_one(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO community_auth_upgrades
            (operation_id, target_community_id, expected_host, actor_pubkey, expected_owner_pubkey,
             expected_issuer, operator_pubkey, policy_generation) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(request.operation_id)
        .bind(request.community_id)
        .bind(&request.expected_host)
        .bind(&request.actor_pubkey)
        .bind(&request.expected_owner_pubkey)
        .bind(&request.expected_issuer)
        .bind(&request.operator_pubkey)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
        let record = sqlx::query_as(RECEIPT)
            .bind(request.operation_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(UpgradeOutcome::Accepted(record))
    }
}

#[cfg(test)]
mod postgres_tests;
