//! Durable reconciliation for the optional relay-authored identity projection.
//!
//! O3 lifecycle rows remain authoritative. This module stores only public
//! event coordinates and opaque binding generations; it is neither an
//! operator API nor a durable audit surface.

use std::{fmt, time::Duration};

use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::Event;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    event::{self, EventQuery},
    identity_binding::{key_lock_coordinate, lock_identity_coordinates_tx},
    Db, DbError, Result,
};

const ASSERTION_KIND: i32 = 30382;
const CLAIM_LEASE: Duration = Duration::from_secs(30);

/// Opaque source generation for one relay-authored public projection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProjectionBindingOrigin {
    binding_id: Uuid,
    binding_version: u64,
}

impl ProjectionBindingOrigin {
    /// Stable binding identifier used only for server-side fencing.
    pub const fn binding_id(self) -> Uuid {
        self.binding_id
    }

    /// Positive binding generation used only for server-side fencing.
    pub const fn binding_version(self) -> u64 {
        self.binding_version
    }
}

impl fmt::Debug for ProjectionBindingOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionBindingOrigin")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .finish()
    }
}

/// Server-only disposition recorded for the current public projection head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionDisposition {
    /// A current binding owns a label-bearing assertion.
    Active,
    /// The assertion is the canonical label-free inactive replacement.
    Inactive,
}

impl ProjectionDisposition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
        }
    }
}

/// Current server-only ownership metadata for an assertion coordinate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProjectionHead {
    event_id: [u8; 32],
    disposition: ProjectionDisposition,
    origin: Option<ProjectionBindingOrigin>,
}

impl ProjectionHead {
    /// Exact signed event installed with this ownership record.
    pub const fn event_id(self) -> [u8; 32] {
        self.event_id
    }

    /// Whether the current projection is active or inactive.
    pub const fn disposition(self) -> ProjectionDisposition {
        self.disposition
    }

    /// Exact binding generation that created the current projection, if known.
    pub const fn origin(self) -> Option<ProjectionBindingOrigin> {
        self.origin
    }
}

impl fmt::Debug for ProjectionHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionHead")
            .field("event_id", &"[redacted]")
            .field("disposition", &self.disposition)
            .field("origin", &"[redacted]")
            .finish()
    }
}

fn checked_version(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| DbError::InvalidData("public projection version is invalid".to_owned()))
}

fn checked_i64(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| DbError::InvalidData("public projection version is invalid".to_owned()))
}

fn validate_pubkey(value: &[u8]) -> Result<()> {
    if value.len() != 32 {
        return Err(DbError::InvalidData(
            "public projection key must be 32 bytes".to_owned(),
        ));
    }
    Ok(())
}

async fn authoritative_binding_for_exact_principal_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    subject: &str,
    pubkey: &[u8],
) -> Result<Option<ProjectionBindingOrigin>> {
    let row = sqlx::query(
        r#"
        SELECT binding.binding_id, binding.binding_version
        FROM identity_bindings binding
        WHERE binding.community_id=$1
          AND binding.issuer=$2
          AND binding.uid=$3
          AND binding.pubkey=$4
          AND binding.binding_state='active'
          AND binding.revoked_at IS NULL
          AND binding.rotation_completed_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM identity_migration_denials denial
              WHERE denial.community_id=binding.community_id
                AND denial.issuer=binding.issuer AND denial.subject=binding.uid)
          AND NOT EXISTS (
              SELECT 1 FROM identity_migration_denied_keys denial
              WHERE denial.community_id=binding.community_id
                AND denial.pubkey=binding.pubkey)
          AND NOT EXISTS (
              SELECT 1 FROM identity_principals principal
              WHERE principal.community_id=binding.community_id
                AND principal.issuer=binding.issuer AND principal.uid=binding.uid
                AND principal.disabled_at IS NOT NULL)
          AND NOT EXISTS (
              SELECT 1 FROM identity_revoked_keys revoked
              WHERE revoked.community_id=binding.community_id
                AND revoked.pubkey=binding.pubkey)
          AND NOT EXISTS (
              SELECT 1 FROM identity_pending_replacements pending
              WHERE pending.community_id=binding.community_id
                AND pending.issuer=binding.issuer AND pending.subject=binding.uid
                AND pending.cleared_at IS NULL)
          AND NOT EXISTS (
              SELECT 1 FROM identity_retired_pairs retired
              WHERE retired.community_id=binding.community_id
                AND retired.issuer=binding.issuer AND retired.subject=binding.uid
                AND retired.pubkey=binding.pubkey)
        FOR SHARE OF binding
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(subject)
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(ProjectionBindingOrigin {
            binding_id: row.try_get("binding_id")?,
            binding_version: checked_version(row.try_get("binding_version")?)?,
        })
    })
    .transpose()
}

async fn authoritative_binding_for_key_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<ProjectionBindingOrigin>> {
    let row = sqlx::query(
        r#"
        SELECT binding.binding_id, binding.binding_version
        FROM identity_bindings binding
        WHERE binding.community_id=$1
          AND binding.pubkey=$2
          AND binding.binding_state='active'
          AND binding.revoked_at IS NULL
          AND binding.rotation_completed_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM identity_migration_denials denial
              WHERE denial.community_id=binding.community_id
                AND denial.issuer=binding.issuer AND denial.subject=binding.uid)
          AND NOT EXISTS (
              SELECT 1 FROM identity_migration_denied_keys denial
              WHERE denial.community_id=binding.community_id
                AND denial.pubkey=binding.pubkey)
          AND NOT EXISTS (
              SELECT 1 FROM identity_principals principal
              WHERE principal.community_id=binding.community_id
                AND principal.issuer=binding.issuer AND principal.uid=binding.uid
                AND principal.disabled_at IS NOT NULL)
          AND NOT EXISTS (
              SELECT 1 FROM identity_revoked_keys revoked
              WHERE revoked.community_id=binding.community_id
                AND revoked.pubkey=binding.pubkey)
          AND NOT EXISTS (
              SELECT 1 FROM identity_pending_replacements pending
              WHERE pending.community_id=binding.community_id
                AND pending.issuer=binding.issuer AND pending.subject=binding.uid
                AND pending.cleared_at IS NULL)
          AND NOT EXISTS (
              SELECT 1 FROM identity_retired_pairs retired
              WHERE retired.community_id=binding.community_id
                AND retired.issuer=binding.issuer AND retired.subject=binding.uid
                AND retired.pubkey=binding.pubkey)
        FOR SHARE OF binding
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(ProjectionBindingOrigin {
            binding_id: row.try_get("binding_id")?,
            binding_version: checked_version(row.try_get("binding_version")?)?,
        })
    })
    .transpose()
}

async fn current_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    relay_pubkey: &[u8],
    subject_pubkey: &[u8],
) -> Result<Option<StoredEvent>> {
    let subject = hex::encode(subject_pubkey);
    Ok(event::query_events_tx(
        tx,
        &EventQuery {
            kinds: Some(vec![ASSERTION_KIND]),
            pubkey: Some(relay_pubkey.to_vec()),
            d_tag: Some(subject),
            global_only: true,
            limit: Some(1),
            ..EventQuery::for_community(community_id)
        },
    )
    .await?
    .into_iter()
    .next())
}

async fn projection_by_id_including_deleted_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event_id: &[u8],
) -> Result<Option<StoredEvent>> {
    let row = sqlx::query(
        "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
         FROM events WHERE community_id=$1 AND id=$2 \
         ORDER BY created_at DESC LIMIT 1 FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(event_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(event::row_to_stored_event)
        .transpose()
        .map(Option::flatten)
}

async fn projection_head_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    relay_pubkey: &[u8],
    subject_pubkey: &[u8],
) -> Result<Option<ProjectionHead>> {
    let row = sqlx::query(
        "SELECT event_id, disposition, source_binding_id, source_binding_version \
         FROM identity_public_projection_heads \
         WHERE community_id=$1 AND relay_pubkey=$2 AND subject_pubkey=$3 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(relay_pubkey)
    .bind(subject_pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        let event_id: Vec<u8> = row.try_get("event_id")?;
        let event_id = event_id.try_into().map_err(|_| {
            DbError::InvalidData("public projection head event id is invalid".to_owned())
        })?;
        let disposition: String = row.try_get("disposition")?;
        let disposition = match disposition.as_str() {
            "active" => ProjectionDisposition::Active,
            "inactive" => ProjectionDisposition::Inactive,
            _ => {
                return Err(DbError::InvalidData(
                    "public projection head disposition is invalid".to_owned(),
                ))
            }
        };
        let binding_id: Option<Uuid> = row.try_get("source_binding_id")?;
        let binding_version: Option<i64> = row.try_get("source_binding_version")?;
        let origin = match (binding_id, binding_version) {
            (Some(binding_id), Some(binding_version)) => Some(ProjectionBindingOrigin {
                binding_id,
                binding_version: checked_version(binding_version)?,
            }),
            (None, None) => None,
            _ => {
                return Err(DbError::InvalidData(
                    "public projection head origin is incomplete".to_owned(),
                ))
            }
        };
        Ok(ProjectionHead {
            event_id,
            disposition,
            origin,
        })
    })
    .transpose()
}

async fn upsert_projection_head_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    relay_pubkey: &[u8],
    subject_pubkey: &[u8],
    event: &Event,
    disposition: ProjectionDisposition,
    origin: Option<ProjectionBindingOrigin>,
) -> Result<()> {
    let created_at = DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or(DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
    sqlx::query(
        r#"
        INSERT INTO identity_public_projection_heads
            (community_id, relay_pubkey, subject_pubkey, event_id,
             event_created_at, disposition, source_binding_id,
             source_binding_version, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW())
        ON CONFLICT (community_id, relay_pubkey, subject_pubkey) DO UPDATE
        SET event_id=EXCLUDED.event_id,
            event_created_at=EXCLUDED.event_created_at,
            disposition=EXCLUDED.disposition,
            source_binding_id=EXCLUDED.source_binding_id,
            source_binding_version=EXCLUDED.source_binding_version,
            updated_at=NOW()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(relay_pubkey)
    .bind(subject_pubkey)
    .bind(event.id.as_bytes().as_slice())
    .bind(created_at)
    .bind(disposition.as_str())
    .bind(origin.map(ProjectionBindingOrigin::binding_id))
    .bind(
        origin
            .map(ProjectionBindingOrigin::binding_version)
            .map(checked_i64)
            .transpose()?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate_event_coordinate(
    event: &Event,
    relay_pubkey: &[u8],
    subject_pubkey: &[u8],
) -> Result<String> {
    let subject = hex::encode(subject_pubkey);
    let exact_tag_count = |name: &str, value: &str| {
        event
            .tags
            .iter()
            .filter(|tag| {
                let parts = tag.as_slice();
                parts.len() == 2 && parts[0] == name && parts[1] == value
            })
            .count()
    };
    if event.kind.as_u16() as i32 != ASSERTION_KIND
        || event.pubkey.as_bytes() != relay_pubkey
        || exact_tag_count("d", &subject) != 1
        || exact_tag_count("p", &subject) != 1
        || exact_tag_count("verified", "relay") != 1
        || !event.verify_id()
        || !event.verify_signature()
    {
        return Err(DbError::InvalidData(
            "public projection event coordinate is invalid".to_owned(),
        ));
    }
    Ok(subject)
}

fn validate_event_disposition(event: &Event, disposition: ProjectionDisposition) -> Result<()> {
    let exact_tag_count = |name: &str, value: &str| {
        event
            .tags
            .iter()
            .filter(|tag| {
                let parts = tag.as_slice();
                parts.len() == 2 && parts[0] == name && parts[1] == value
            })
            .count()
    };
    let expiration = event
        .tags
        .iter()
        .filter(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "expiration"
        })
        .map(|tag| tag.as_slice()[1].parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| DbError::InvalidData("public projection expiration is invalid".to_owned()))?;
    let display_names = event
        .tags
        .iter()
        .filter(|tag| {
            let parts = tag.as_slice();
            parts.len() == 2 && parts[0] == "display_name"
        })
        .map(|tag| tag.as_slice()[1].as_str())
        .collect::<Vec<_>>();
    let valid = event.content.is_empty()
        && match disposition {
            ProjectionDisposition::Active => {
                event.tags.len() == 6
                    && exact_tag_count("active", "true") == 1
                    && exact_tag_count("active", "false") == 0
                    && expiration.len() == 1
                    && expiration[0] > 0
                    && display_names.len() == 1
                    && !display_names[0].is_empty()
            }
            ProjectionDisposition::Inactive => {
                event.tags.len() == 5
                    && exact_tag_count("active", "false") == 1
                    && exact_tag_count("active", "true") == 0
                    && expiration.as_slice() == [0]
                    && display_names.is_empty()
            }
        };
    if !valid {
        return Err(DbError::InvalidData(
            "public projection disposition is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn canonical_later_projection_is_proven(
    current: &StoredEvent,
    head: ProjectionHead,
    expected: &StoredEvent,
    source: ProjectionBindingOrigin,
    relay_pubkey: &[u8],
    subject_pubkey: &[u8],
) -> Result<bool> {
    validate_event_coordinate(&current.event, relay_pubkey, subject_pubkey)?;
    validate_event_disposition(&current.event, head.disposition)?;
    validate_event_coordinate(&expected.event, relay_pubkey, subject_pubkey)?;
    validate_event_disposition(&expected.event, ProjectionDisposition::Inactive)?;

    let head_matches_current = head.event_id.as_slice() == current.event.id.as_bytes().as_slice();
    let current_is_later = current.event.created_at > expected.event.created_at
        || (current.event.created_at == expected.event.created_at
            && current.event.id.as_bytes().as_slice() < expected.event.id.as_bytes().as_slice());
    let later_owner_is_proven = head.origin.is_some_and(|origin| {
        origin.binding_id != source.binding_id || origin.binding_version > source.binding_version
    });

    Ok(head_matches_current && current_is_later && later_owner_is_proven)
}

/// Transaction-owned active publication permit.
pub struct ActivePublicProjectionPermit {
    tx: Transaction<'static, Postgres>,
    community_id: CommunityId,
    relay_pubkey: Vec<u8>,
    subject_pubkey: Vec<u8>,
    origin: ProjectionBindingOrigin,
    current: Option<StoredEvent>,
}

impl fmt::Debug for ActivePublicProjectionPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivePublicProjectionPermit")
            .field("community_id", &"[redacted]")
            .field("relay_pubkey", &"[redacted]")
            .field("subject_pubkey", &"[redacted]")
            .field("origin", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl ActivePublicProjectionPermit {
    /// Current stored projection at the locked coordinate.
    pub fn current_projection(&self) -> Option<&StoredEvent> {
        self.current.as_ref()
    }

    /// Exact active binding generation retained through commit.
    pub const fn origin(&self) -> ProjectionBindingOrigin {
        self.origin
    }

    /// Atomically replace or accept the candidate and record private ownership.
    pub async fn commit(
        mut self,
        event: &Event,
        disposition: ProjectionDisposition,
    ) -> Result<StoredEvent> {
        let subject = validate_event_coordinate(event, &self.relay_pubkey, &self.subject_pubkey)?;
        validate_event_disposition(event, disposition)?;
        let (candidate, inserted) = event::replace_parameterized_event_tx(
            &mut self.tx,
            self.community_id,
            event,
            &subject,
            None,
        )
        .await?;
        let stored = if inserted {
            candidate
        } else {
            let current = current_projection_tx(
                &mut self.tx,
                self.community_id,
                &self.relay_pubkey,
                &self.subject_pubkey,
            )
            .await?
            .ok_or_else(|| {
                DbError::InvalidData("public projection replacement disappeared".to_owned())
            })?;
            if current.event.id != event.id {
                return Err(DbError::InvalidData(
                    "public projection candidate lost ordering".to_owned(),
                ));
            }
            current
        };
        upsert_projection_head_tx(
            &mut self.tx,
            self.community_id,
            &self.relay_pubkey,
            &self.subject_pubkey,
            &stored.event,
            disposition,
            Some(self.origin),
        )
        .await?;
        self.tx.commit().await?;
        Ok(stored)
    }
}

/// Begin active publication while retaining exact binding authority to commit.
pub async fn begin_active_public_projection(
    db: &Db,
    community_id: CommunityId,
    relay_pubkey: &[u8],
    issuer: &str,
    subject: &str,
    subject_pubkey: &[u8],
) -> Result<Option<ActivePublicProjectionPermit>> {
    validate_pubkey(relay_pubkey)?;
    validate_pubkey(subject_pubkey)?;
    if issuer.is_empty() || subject.is_empty() {
        return Err(DbError::InvalidData(
            "public projection principal is invalid".to_owned(),
        ));
    }
    let mut tx = db.pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_coordinates_tx(
        &mut tx,
        vec![key_lock_coordinate(community_id, subject_pubkey)],
    )
    .await?;
    let Some(origin) = authoritative_binding_for_exact_principal_tx(
        &mut tx,
        community_id,
        issuer,
        subject,
        subject_pubkey,
    )
    .await?
    else {
        tx.rollback().await?;
        return Ok(None);
    };
    let current =
        current_projection_tx(&mut tx, community_id, relay_pubkey, subject_pubkey).await?;
    Ok(Some(ActivePublicProjectionPermit {
        tx,
        community_id,
        relay_pubkey: relay_pubkey.to_vec(),
        subject_pubkey: subject_pubkey.to_vec(),
        origin,
        current,
    }))
}

/// Materialize committed O3 revoke/rotate operations as retryable O4 work.
pub async fn materialize_public_projection_retirements(
    db: &Db,
    domains: &[CommunityId],
    relay_pubkey: &[u8],
) -> Result<u64> {
    validate_pubkey(relay_pubkey)?;
    if domains.is_empty() {
        return Ok(0);
    }
    let domain_ids = domains
        .iter()
        .map(|domain| *domain.as_uuid())
        .collect::<Vec<_>>();
    let result = sqlx::query(
        r#"
        INSERT INTO identity_public_projection_retirements
            (community_id, operation_id, relay_pubkey, old_pubkey,
             source_binding_id, source_binding_version, operation_kind)
        SELECT operation.community_id, operation.operation_id, $2,
               operation.pubkey, retired.binding_id,
               retired.binding_version - 1, operation.operation_kind
        FROM identity_lifecycle_operations operation
        LEFT JOIN LATERAL (
            SELECT history.binding_id, history.binding_version
            FROM identity_binding_history history
            WHERE history.community_id=operation.community_id
              AND history.operation_id=operation.operation_id
              AND history.binding_id IS NOT DISTINCT FROM operation.binding_id
              AND history.pubkey=operation.pubkey
              AND history.binding_state IN ('revoked', 'rotated')
              AND history.binding_version > 1
            ORDER BY history.recorded_at DESC, history.history_id
            LIMIT 1
        ) retired ON TRUE
        WHERE operation.community_id = ANY($1)
          AND operation.operation_kind IN ('revoke_key', 'rotate')
          AND operation.pubkey IS NOT NULL
        ON CONFLICT (community_id, operation_id, relay_pubkey) DO NOTHING
        "#,
    )
    .bind(domain_ids)
    .bind(relay_pubkey)
    .execute(&db.pool)
    .await?;
    Ok(result.rows_affected())
}

/// Retryable retirement work claimed by one relay replica.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectionRetirementClaim {
    community_id: CommunityId,
    operation_id: Uuid,
    relay_pubkey: Vec<u8>,
    old_pubkey: Vec<u8>,
    source_origin: Option<ProjectionBindingOrigin>,
    claim_token: Uuid,
}

impl ProjectionRetirementClaim {
    /// Server-resolved authorization domain.
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Public subject key whose old assertion may need retirement.
    pub fn old_pubkey(&self) -> &[u8] {
        &self.old_pubkey
    }

    /// Exact retired source generation, when the lifecycle transition had one.
    pub const fn source_origin(&self) -> Option<ProjectionBindingOrigin> {
        self.source_origin
    }
}

impl fmt::Debug for ProjectionRetirementClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionRetirementClaim")
            .field("community_id", &"[redacted]")
            .field("operation_id", &"[redacted]")
            .field("relay_pubkey", &"[redacted]")
            .field("old_pubkey", &"[redacted]")
            .field("source_origin", &"[redacted]")
            .finish()
    }
}

async fn claim_next(
    db: &Db,
    domains: &[CommunityId],
    relay_pubkey: &[u8],
    phase: &str,
) -> Result<Option<ProjectionRetirementClaim>> {
    validate_pubkey(relay_pubkey)?;
    if domains.is_empty() {
        return Ok(None);
    }
    let domain_ids = domains
        .iter()
        .map(|domain| *domain.as_uuid())
        .collect::<Vec<_>>();
    let lease_seconds = i64::try_from(CLAIM_LEASE.as_secs())
        .map_err(|_| DbError::InvalidData("projection claim lease is invalid".to_owned()))?;
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT community_id, operation_id, relay_pubkey
            FROM identity_public_projection_retirements
            WHERE community_id=ANY($1) AND relay_pubkey=$2 AND phase=$3
              AND next_attempt_at <= NOW()
              AND (claim_token IS NULL OR lease_until <= NOW())
            ORDER BY next_attempt_at, community_id, operation_id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE identity_public_projection_retirements work
        SET claim_token=gen_random_uuid(),
            lease_until=NOW() + ($4::DOUBLE PRECISION * INTERVAL '1 second'),
            attempts=attempts+1,
            updated_at=NOW()
        FROM candidate
        WHERE work.community_id=candidate.community_id
          AND work.operation_id=candidate.operation_id
          AND work.relay_pubkey=candidate.relay_pubkey
        RETURNING work.community_id, work.operation_id, work.relay_pubkey,
                  work.old_pubkey, work.source_binding_id,
                  work.source_binding_version, work.claim_token
        "#,
    )
    .bind(domain_ids)
    .bind(relay_pubkey)
    .bind(phase)
    .bind(lease_seconds)
    .fetch_optional(&db.pool)
    .await?;
    row.map(|row| {
        let source_binding_id: Option<Uuid> = row.try_get("source_binding_id")?;
        let source_binding_version: Option<i64> = row.try_get("source_binding_version")?;
        let source_origin = match (source_binding_id, source_binding_version) {
            (Some(binding_id), Some(binding_version)) => Some(ProjectionBindingOrigin {
                binding_id,
                binding_version: checked_version(binding_version)?,
            }),
            (None, None) => None,
            _ => {
                return Err(DbError::InvalidData(
                    "projection retirement source is incomplete".to_owned(),
                ))
            }
        };
        Ok(ProjectionRetirementClaim {
            community_id: CommunityId::from_uuid(row.try_get("community_id")?),
            operation_id: row.try_get("operation_id")?,
            relay_pubkey: row.try_get("relay_pubkey")?,
            old_pubkey: row.try_get("old_pubkey")?,
            source_origin,
            claim_token: row.try_get("claim_token")?,
        })
    })
    .transpose()
}

/// Claim one ready public-projection retirement operation.
pub async fn claim_public_projection_retirement(
    db: &Db,
    domains: &[CommunityId],
    relay_pubkey: &[u8],
) -> Result<Option<ProjectionRetirementClaim>> {
    claim_next(db, domains, relay_pubkey, "projection").await
}

/// Transaction-owned view of one claimed retirement.
pub struct ProjectionRetirementPermit {
    tx: Transaction<'static, Postgres>,
    claim: ProjectionRetirementClaim,
    current: Option<StoredEvent>,
    head: Option<ProjectionHead>,
    active_origin: Option<ProjectionBindingOrigin>,
}

impl fmt::Debug for ProjectionRetirementPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionRetirementPermit")
            .field("claim", &self.claim)
            .field("current", &self.current.as_ref().map(|_| "[event]"))
            .field("head", &self.head)
            .field("active_origin", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl ProjectionRetirementPermit {
    /// Public subject key selected by the committed lifecycle operation.
    pub fn old_pubkey(&self) -> &[u8] {
        &self.claim.old_pubkey
    }

    /// Current event at the locked assertion coordinate.
    pub fn current_projection(&self) -> Option<&StoredEvent> {
        self.current.as_ref()
    }

    /// Current private projection ownership metadata.
    pub const fn head(&self) -> Option<ProjectionHead> {
        self.head
    }

    /// Current authoritative binding for the key, if it has been reused.
    pub const fn active_origin(&self) -> Option<ProjectionBindingOrigin> {
        self.active_origin
    }

    /// Retired binding generation carried by the durable lifecycle work.
    pub const fn source_origin(&self) -> Option<ProjectionBindingOrigin> {
        self.claim.source_origin
    }

    fn head_for_current(&self, disposition: ProjectionDisposition) -> Result<ProjectionHead> {
        let current = self.current.as_ref().ok_or_else(|| {
            DbError::InvalidData("public projection retirement has no current event".to_owned())
        })?;
        validate_event_coordinate(
            &current.event,
            &self.claim.relay_pubkey,
            &self.claim.old_pubkey,
        )?;
        validate_event_disposition(&current.event, disposition)?;
        let head = self.head.ok_or_else(|| {
            DbError::InvalidData("public projection ownership is unavailable".to_owned())
        })?;
        if head.event_id != *current.event.id.as_bytes() || head.disposition != disposition {
            return Err(DbError::InvalidData(
                "public projection ownership does not match the current event".to_owned(),
            ));
        }
        Ok(head)
    }

    fn current_can_be_retired_by_source(&self) -> Result<bool> {
        let (current, head) = match (self.current.as_ref(), self.head) {
            (None, None) => return Ok(true),
            (None, Some(_)) => {
                return Err(DbError::InvalidData(
                    "public projection ownership exists without an event".to_owned(),
                ))
            }
            (Some(_), None) => return Ok(true),
            (Some(current), Some(head)) => (current, head),
        };
        validate_event_coordinate(
            &current.event,
            &self.claim.relay_pubkey,
            &self.claim.old_pubkey,
        )?;
        validate_event_disposition(&current.event, head.disposition)?;
        if head.event_id != *current.event.id.as_bytes() {
            return Err(DbError::InvalidData(
                "public projection ownership does not match the current event".to_owned(),
            ));
        }
        Ok(match (self.claim.source_origin, head.origin) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(source), Some(origin)) if source.binding_id == origin.binding_id => {
                origin.binding_version <= source.binding_version
            }
            (Some(_), Some(_)) => false,
        })
    }

    async fn finish_terminal(mut self, phase: &str, outcome: &str) -> Result<()> {
        let changed = sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET phase=$5, outcome=$6, claim_token=NULL, lease_until=NULL, \
                 completed_at=NOW(), updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='projection' AND lease_until > NOW()",
        )
        .bind(self.claim.community_id.as_uuid())
        .bind(self.claim.operation_id)
        .bind(&self.claim.relay_pubkey)
        .bind(self.claim.claim_token)
        .bind(phase)
        .bind(outcome)
        .execute(&mut *self.tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "public projection retirement claim expired".to_owned(),
            ));
        }
        self.tx.commit().await?;
        Ok(())
    }

    /// Complete a job whose assertion coordinate is empty.
    pub async fn finish_no_projection(self) -> Result<()> {
        if self.current.is_some() || self.head.is_some() {
            return Err(DbError::InvalidData(
                "public projection coordinate is not empty".to_owned(),
            ));
        }
        self.finish_terminal("completed", "no_projection").await
    }

    /// Preserve a newer legitimate binding and finish the stale job.
    pub async fn finish_superseded(self, current: &Event) -> Result<()> {
        let origin = self.active_origin.ok_or_else(|| {
            DbError::InvalidData("projection supersession lacks active binding".to_owned())
        })?;
        validate_event_coordinate(current, &self.claim.relay_pubkey, &self.claim.old_pubkey)?;
        validate_event_disposition(current, ProjectionDisposition::Active)?;
        let current_id = self
            .current
            .as_ref()
            .map(|stored| stored.event.id)
            .ok_or_else(|| {
                DbError::InvalidData("public projection retirement has no current event".to_owned())
            })?;
        let head = self.head_for_current(ProjectionDisposition::Active)?;
        if current.id != current_id
            || head.origin != Some(origin)
            || self.claim.source_origin == Some(origin)
        {
            return Err(DbError::InvalidData(
                "active binding does not own the current public projection".to_owned(),
            ));
        }
        self.finish_terminal("superseded", "newer_binding").await
    }

    /// Preserve a projection owned by a later binding generation.
    pub async fn finish_newer_projection(self) -> Result<()> {
        let head = self.head_for_current(ProjectionDisposition::Active)?;
        let origin = head.origin.ok_or_else(|| {
            DbError::InvalidData("newer public projection lacks an owner".to_owned())
        })?;
        let is_newer = match self.claim.source_origin {
            None => true,
            Some(source) if source.binding_id == origin.binding_id => {
                origin.binding_version > source.binding_version
            }
            Some(source) => source != origin,
        };
        if !is_newer {
            return Err(DbError::InvalidData(
                "public projection is not owned by a later generation".to_owned(),
            ));
        }
        self.finish_terminal("superseded", "newer_projection").await
    }

    /// Finish a stale/replayed retirement when the exact coordinate is already
    /// the canonical inactive projection.
    ///
    /// This is a metadata-only convergence path: it preserves the event and
    /// ownership head byte-for-byte and never relabels an old assertion to the
    /// replayed job's source generation.
    pub async fn finish_existing_inactive(self) -> Result<()> {
        let current = self.current.as_ref().ok_or_else(|| {
            DbError::InvalidData("public projection retirement has no current event".to_owned())
        })?;
        validate_event_coordinate(
            &current.event,
            &self.claim.relay_pubkey,
            &self.claim.old_pubkey,
        )?;
        validate_event_disposition(&current.event, ProjectionDisposition::Inactive)?;
        if let Some(head) = self.head {
            if head.event_id != *current.event.id.as_bytes()
                || head.disposition != ProjectionDisposition::Inactive
            {
                return Err(DbError::InvalidData(
                    "public projection ownership does not match the current event".to_owned(),
                ));
            }
        }
        self.finish_terminal("completed", "already_inactive").await
    }

    /// Atomically install/accept the canonical inactive event and queue delivery.
    pub async fn finish_inactive(mut self, inactive: &Event) -> Result<StoredEvent> {
        if !self.current_can_be_retired_by_source()? {
            return Err(DbError::InvalidData(
                "public projection belongs to a later binding generation".to_owned(),
            ));
        }
        let subject =
            validate_event_coordinate(inactive, &self.claim.relay_pubkey, &self.claim.old_pubkey)?;
        validate_event_disposition(inactive, ProjectionDisposition::Inactive)?;
        let (candidate, inserted) = event::replace_parameterized_event_tx(
            &mut self.tx,
            self.claim.community_id,
            inactive,
            &subject,
            None,
        )
        .await?;
        let stored = if inserted {
            candidate
        } else {
            let current = current_projection_tx(
                &mut self.tx,
                self.claim.community_id,
                &self.claim.relay_pubkey,
                &self.claim.old_pubkey,
            )
            .await?
            .ok_or_else(|| {
                DbError::InvalidData("inactive projection replacement disappeared".to_owned())
            })?;
            if current.event.id != inactive.id {
                return Err(DbError::InvalidData(
                    "inactive projection candidate lost ordering".to_owned(),
                ));
            }
            current
        };
        upsert_projection_head_tx(
            &mut self.tx,
            self.claim.community_id,
            &self.claim.relay_pubkey,
            &self.claim.old_pubkey,
            &stored.event,
            ProjectionDisposition::Inactive,
            self.claim.source_origin,
        )
        .await?;
        let changed = sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET phase='delivery', outcome=$5, event_id=$6, claim_token=NULL, \
                 lease_until=NULL, next_attempt_at=NOW(), updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='projection' AND lease_until > NOW()",
        )
        .bind(self.claim.community_id.as_uuid())
        .bind(self.claim.operation_id)
        .bind(&self.claim.relay_pubkey)
        .bind(self.claim.claim_token)
        .bind(if inserted {
            "replaced_inactive"
        } else {
            "already_inactive"
        })
        .bind(stored.event.id.as_bytes().as_slice())
        .execute(&mut *self.tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "public projection retirement claim expired".to_owned(),
            ));
        }
        self.tx.commit().await?;
        Ok(stored)
    }

    /// Release retryable work with bounded backoff and no authority change.
    pub async fn defer(mut self) -> Result<()> {
        sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET claim_token=NULL, lease_until=NULL, \
                 next_attempt_at=NOW() + (LEAST(60, GREATEST(1, attempts))::DOUBLE PRECISION * INTERVAL '1 second'), \
                 updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='projection'",
        )
        .bind(self.claim.community_id.as_uuid())
        .bind(self.claim.operation_id)
        .bind(&self.claim.relay_pubkey)
        .bind(self.claim.claim_token)
        .execute(&mut *self.tx)
        .await?;
        self.tx.commit().await?;
        Ok(())
    }
}

/// Revalidate and lock one claimed retirement through its event commit boundary.
pub async fn begin_public_projection_retirement(
    db: &Db,
    claim: ProjectionRetirementClaim,
) -> Result<ProjectionRetirementPermit> {
    let mut tx = db.pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_coordinates_tx(
        &mut tx,
        vec![key_lock_coordinate(claim.community_id, &claim.old_pubkey)],
    )
    .await?;
    let claimed = sqlx::query(
        "SELECT 1 FROM identity_public_projection_retirements \
         WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
           AND claim_token=$4 AND phase='projection' AND lease_until > NOW() FOR UPDATE",
    )
    .bind(claim.community_id.as_uuid())
    .bind(claim.operation_id)
    .bind(&claim.relay_pubkey)
    .bind(claim.claim_token)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !claimed {
        return Err(DbError::InvalidData(
            "public projection retirement claim expired".to_owned(),
        ));
    }
    let active_origin =
        authoritative_binding_for_key_tx(&mut tx, claim.community_id, &claim.old_pubkey).await?;
    let head = projection_head_tx(
        &mut tx,
        claim.community_id,
        &claim.relay_pubkey,
        &claim.old_pubkey,
    )
    .await?;
    let current = current_projection_tx(
        &mut tx,
        claim.community_id,
        &claim.relay_pubkey,
        &claim.old_pubkey,
    )
    .await?;
    Ok(ProjectionRetirementPermit {
        tx,
        claim,
        current,
        head,
        active_origin,
    })
}

/// Delivery work retained until Redis and local fan-out have both been attempted.
pub struct ProjectionDeliveryPermit {
    tx: Transaction<'static, Postgres>,
    claim: ProjectionRetirementClaim,
    stored: StoredEvent,
}

impl fmt::Debug for ProjectionDeliveryPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectionDeliveryPermit")
            .field("claim", &self.claim)
            .field("stored", &"[event]")
            .finish_non_exhaustive()
    }
}

impl ProjectionDeliveryPermit {
    /// Exact current inactive event retained under the identity-key lock.
    pub const fn stored(&self) -> &StoredEvent {
        &self.stored
    }

    /// Server-resolved authorization domain.
    pub const fn community_id(&self) -> CommunityId {
        self.claim.community_id
    }

    /// Mark delivery complete while the exact projection head remains locked.
    pub async fn complete(mut self) -> Result<()> {
        let changed = sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET phase='completed', claim_token=NULL, lease_until=NULL, \
                 completed_at=NOW(), updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='delivery' AND lease_until > NOW()",
        )
        .bind(self.claim.community_id.as_uuid())
        .bind(self.claim.operation_id)
        .bind(&self.claim.relay_pubkey)
        .bind(self.claim.claim_token)
        .execute(&mut *self.tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "public projection delivery claim expired".to_owned(),
            ));
        }
        self.tx.commit().await?;
        Ok(())
    }

    /// Release delivery for bounded retry without changing the event head.
    pub async fn defer(mut self) -> Result<()> {
        sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET claim_token=NULL, lease_until=NULL, \
                 next_attempt_at=NOW() + (LEAST(60, GREATEST(1, attempts))::DOUBLE PRECISION * INTERVAL '1 second'), \
                 updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='delivery'",
        )
        .bind(self.claim.community_id.as_uuid())
        .bind(self.claim.operation_id)
        .bind(&self.claim.relay_pubkey)
        .bind(self.claim.claim_token)
        .execute(&mut *self.tx)
        .await?;
        self.tx.commit().await?;
        Ok(())
    }
}

/// Claim and lock one pending inactive-event delivery.
pub async fn begin_public_projection_delivery(
    db: &Db,
    domains: &[CommunityId],
    relay_pubkey: &[u8],
) -> Result<Option<ProjectionDeliveryPermit>> {
    let Some(claim) = claim_next(db, domains, relay_pubkey, "delivery").await? else {
        return Ok(None);
    };
    let mut tx = db.pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_coordinates_tx(
        &mut tx,
        vec![key_lock_coordinate(claim.community_id, &claim.old_pubkey)],
    )
    .await?;
    let row = sqlx::query(
        "SELECT event_id FROM identity_public_projection_retirements \
         WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
           AND claim_token=$4 AND phase='delivery' AND lease_until > NOW() FOR UPDATE",
    )
    .bind(claim.community_id.as_uuid())
    .bind(claim.operation_id)
    .bind(&claim.relay_pubkey)
    .bind(claim.claim_token)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DbError::InvalidData("public projection delivery claim expired".to_owned()))?;
    let expected_id: Vec<u8> = row.try_get("event_id")?;
    let current = current_projection_tx(
        &mut tx,
        claim.community_id,
        &claim.relay_pubkey,
        &claim.old_pubkey,
    )
    .await?;
    let head = projection_head_tx(
        &mut tx,
        claim.community_id,
        &claim.relay_pubkey,
        &claim.old_pubkey,
    )
    .await?;
    let head_matches_inactive = head.is_some_and(|head| {
        head.event_id.as_slice() == expected_id.as_slice()
            && head.disposition == ProjectionDisposition::Inactive
    });
    let Some(stored) = current
        .as_ref()
        .filter(|stored| {
            stored.event.id.as_bytes().as_slice() == expected_id.as_slice() && head_matches_inactive
        })
        .cloned()
    else {
        let expected =
            projection_by_id_including_deleted_tx(&mut tx, claim.community_id, &expected_id)
                .await?;
        let later_projection_is_proven = match (
            current.as_ref(),
            head,
            expected.as_ref(),
            claim.source_origin,
        ) {
            (Some(current), Some(head), Some(expected), Some(source)) => {
                canonical_later_projection_is_proven(
                    current,
                    head,
                    expected,
                    source,
                    &claim.relay_pubkey,
                    &claim.old_pubkey,
                )?
            }
            _ => false,
        };
        if !later_projection_is_proven {
            return Err(DbError::InvalidData(
                "public projection delivery lost its canonical ownership proof".to_owned(),
            ));
        }
        let changed = sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET phase='superseded', outcome='newer_projection', claim_token=NULL, \
                 lease_until=NULL, completed_at=NOW(), updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4 AND phase='delivery' AND lease_until > NOW()",
        )
        .bind(claim.community_id.as_uuid())
        .bind(claim.operation_id)
        .bind(&claim.relay_pubkey)
        .bind(claim.claim_token)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "public projection delivery claim expired".to_owned(),
            ));
        }
        tx.commit().await?;
        return Ok(None);
    };
    validate_event_coordinate(&stored.event, &claim.relay_pubkey, &claim.old_pubkey)?;
    validate_event_disposition(&stored.event, ProjectionDisposition::Inactive)?;
    Ok(Some(ProjectionDeliveryPermit { tx, claim, stored }))
}

/// Count unfinished work for a relay author and exact domain set.
pub async fn unfinished_public_projection_retirements(
    db: &Db,
    domains: &[CommunityId],
    relay_pubkey: &[u8],
) -> Result<u64> {
    validate_pubkey(relay_pubkey)?;
    if domains.is_empty() {
        return Ok(0);
    }
    let domain_ids = domains
        .iter()
        .map(|domain| *domain.as_uuid())
        .collect::<Vec<_>>();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_public_projection_retirements \
         WHERE community_id=ANY($1) AND relay_pubkey=$2 \
           AND phase IN ('projection', 'delivery')",
    )
    .bind(domain_ids)
    .bind(relay_pubkey)
    .fetch_one(&db.pool)
    .await?;
    u64::try_from(count)
        .map_err(|_| DbError::InvalidData("projection retirement count is invalid".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_binding::{
        resolve_identity_binding, BindingProvenance, EnrollmentMode, ResolveBindingInput,
        ResolveBindingResult,
    };
    use crate::identity_lifecycle::{
        revoke_identity_key, rotate_identity_binding, IdentityPrincipal, LifecycleContext,
        LifecycleOperationId, VerifiedReplacementKey,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
    const ISSUER: &str = "https://idp.example";
    const TEST_ACTOR: [u8; 32] = [0xA4; 32];

    async fn resolve_test_binding(
        db: &Db,
        community: CommunityId,
        issuer: &str,
        subject: &str,
        pubkey: &[u8],
        enrollment_mode: EnrollmentMode,
        key_attested: bool,
    ) -> ResolveBindingResult {
        resolve_identity_binding(
            &db.pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer,
                subject,
                pubkey,
                display_name: None,
                enrollment_mode,
                key_attested,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("resolve test binding")
    }

    fn lifecycle(reason: &str) -> LifecycleContext<'_> {
        lifecycle_with_id(LifecycleOperationId::issue(), reason)
    }

    fn lifecycle_with_id(operation_id: LifecycleOperationId, reason: &str) -> LifecycleContext<'_> {
        LifecycleContext {
            operation_id,
            actor: &TEST_ACTOR,
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

    async fn setup() -> (Db, CommunityId) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPool::connect(&database_url).await.expect("test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(id)
            .bind(format!("public-projection-{}.example", id.simple()))
            .execute(&pool)
            .await
            .expect("insert community");
        (Db::from_pool(pool), CommunityId::from_uuid(id))
    }

    fn assertion(keys: &Keys, subject: nostr::PublicKey, active: bool, at: u64) -> Event {
        let subject = subject.to_hex();
        let active_value = if active { "true" } else { "false" };
        let expiration = if active { "500" } else { "0" };
        let mut tags = vec![
            Tag::parse(["d", subject.as_str()]).expect("d tag"),
            Tag::parse(["p", subject.as_str()]).expect("p tag"),
            Tag::parse(["verified", "relay"]).expect("verified tag"),
            Tag::parse(["active", active_value]).expect("active tag"),
            Tag::parse(["expiration", expiration]).expect("expiration tag"),
        ];
        if active {
            tags.push(Tag::parse(["display_name", "Approved Label"]).expect("display tag"));
        }
        EventBuilder::new(Kind::Custom(ASSERTION_KIND as u16), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(keys)
            .expect("sign assertion")
    }

    fn stored(event: Event) -> StoredEvent {
        StoredEvent::with_received_at(event, Utc::now(), None, true)
    }

    #[test]
    fn delivery_supersession_requires_a_canonical_later_projection_and_owner() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let source = ProjectionBindingOrigin {
            binding_id: Uuid::new_v4(),
            binding_version: 7,
        };
        let later = ProjectionBindingOrigin {
            binding_id: source.binding_id,
            binding_version: 8,
        };
        let expected = stored(assertion(&relay, subject, false, 100));
        let current = stored(assertion(&relay, subject, true, 101));
        let head = ProjectionHead {
            event_id: *current.event.id.as_bytes(),
            disposition: ProjectionDisposition::Active,
            origin: Some(later),
        };

        assert!(canonical_later_projection_is_proven(
            &current,
            head,
            &expected,
            source,
            relay.public_key().as_bytes(),
            subject.as_bytes(),
        )
        .expect("canonical proof"));

        let stale_head = ProjectionHead {
            event_id: *expected.event.id.as_bytes(),
            ..head
        };
        assert!(!canonical_later_projection_is_proven(
            &current,
            stale_head,
            &expected,
            source,
            relay.public_key().as_bytes(),
            subject.as_bytes(),
        )
        .expect("stale head is a denied proof"));

        let unreplaced_head = ProjectionHead {
            origin: Some(source),
            ..head
        };
        assert!(!canonical_later_projection_is_proven(
            &current,
            unreplaced_head,
            &expected,
            source,
            relay.public_key().as_bytes(),
            subject.as_bytes(),
        )
        .expect("same owner generation is a denied proof"));

        let noncanonical_expected = stored(assertion(&relay, subject, true, 100));
        assert!(canonical_later_projection_is_proven(
            &current,
            head,
            &noncanonical_expected,
            source,
            relay.public_key().as_bytes(),
            subject.as_bytes(),
        )
        .is_err());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn delivery_converges_only_after_a_later_projection_owns_the_coordinate() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let source = match resolve_test_binding(
            &db,
            community,
            ISSUER,
            "delivery-race-subject",
            subject.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await
        {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected binding result: {other:?}"),
        };
        let active = assertion(&relay, subject, true, 100);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "delivery-race-subject",
            subject.as_bytes(),
        )
        .await
        .expect("begin active projection")
        .expect("active binding")
        .commit(&active, ProjectionDisposition::Active)
        .await
        .expect("commit active projection");

        revoke_identity_key(
            &db.pool,
            community,
            lifecycle("delivery race"),
            subject.as_bytes(),
        )
        .await
        .expect("commit revocation");
        materialize_public_projection_retirements(&db, &[community], relay.public_key().as_bytes())
            .await
            .expect("materialize retirement");
        let claim =
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim retirement")
                .expect("retirement exists");
        let expected = assertion(&relay, subject, false, 101);
        begin_public_projection_retirement(&db, claim)
            .await
            .expect("begin retirement")
            .finish_inactive(&expected)
            .await
            .expect("commit inactive projection");

        let later = assertion(&relay, subject, true, 102);
        let later_origin = ProjectionBindingOrigin {
            binding_id: source.binding_id,
            binding_version: source.binding_version + 1,
        };
        let mut tx = db.pool.begin().await.expect("begin replacement");
        let subject_hex = subject.to_hex();
        let (_, inserted) =
            event::replace_parameterized_event_tx(&mut tx, community, &later, &subject_hex, None)
                .await
                .expect("replace inactive projection");
        assert!(inserted, "later projection must win canonical ordering");
        upsert_projection_head_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            subject.as_bytes(),
            &later,
            ProjectionDisposition::Active,
            Some(later_origin),
        )
        .await
        .expect("install later projection ownership");
        tx.commit().await.expect("commit later projection");

        assert!(
            begin_public_projection_delivery(&db, &[community], relay.public_key().as_bytes(),)
                .await
                .expect("converge stale delivery")
                .is_none(),
            "a fully proven later projection must terminalize stale delivery"
        );
        assert_eq!(
            unfinished_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("count unfinished work"),
            0
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn committed_revoke_materializes_retries_and_completes_exactly_once() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let subject_keys = Keys::generate();
        let subject = subject_keys.public_key();
        let subject_bytes = subject.to_bytes();
        let resolved = resolve_test_binding(
            &db,
            community,
            ISSUER,
            "subject-one",
            subject_bytes.as_slice(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await;
        let expected_origin = match resolved {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected binding result: {other:?}"),
        };
        assert_eq!(expected_origin.binding_version(), 1);

        let active = assertion(&relay, subject, true, 100);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "subject-one",
            subject.as_bytes(),
        )
        .await
        .expect("begin active projection")
        .expect("active binding")
        .commit(&active, ProjectionDisposition::Active)
        .await
        .expect("commit active projection");

        let operation_id = LifecycleOperationId::issue();
        revoke_identity_key(
            &db.pool,
            community,
            lifecycle_with_id(operation_id, "test revocation"),
            subject.as_bytes(),
        )
        .await
        .expect("commit revocation");

        assert_eq!(
            materialize_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("materialize work"),
            1
        );
        assert_eq!(
            materialize_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("duplicate discovery is idempotent"),
            0
        );
        let domains = [community];
        let relay_pubkey = relay.public_key().to_bytes();
        let (first_replica, second_replica) = tokio::join!(
            claim_public_projection_retirement(&db, &domains, relay_pubkey.as_slice()),
            claim_public_projection_retirement(&db, &domains, relay_pubkey.as_slice()),
        );
        let mut claims = [
            first_replica.expect("first replica claim"),
            second_replica.expect("second replica claim"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1, "only one replica may own the claim");
        let stale_claim = claims.pop().expect("one claim");
        sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET lease_until=NOW() - INTERVAL '1 second' \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND claim_token=$4",
        )
        .bind(community.as_uuid())
        .bind(operation_id.as_uuid())
        .bind(relay.public_key().as_bytes())
        .bind(stale_claim.claim_token)
        .execute(&db.pool)
        .await
        .expect("simulate crashed claim owner");
        assert!(
            begin_public_projection_retirement(&db, stale_claim)
                .await
                .is_err(),
            "an expired owner must not mutate the projection"
        );
        let claim =
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("reclaim crashed work")
                .expect("reclaimable work exists");
        assert_eq!(claim.source_origin(), Some(expected_origin));
        let permit = begin_public_projection_retirement(&db, claim)
            .await
            .expect("begin retirement");
        assert_eq!(permit.active_origin(), None);
        assert_eq!(
            permit.head().and_then(ProjectionHead::origin),
            Some(expected_origin)
        );
        let inactive = assertion(&relay, subject, false, 101);
        permit
            .finish_inactive(&inactive)
            .await
            .expect("commit inactive projection");

        let delivery =
            begin_public_projection_delivery(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim delivery")
                .expect("delivery exists");
        assert_eq!(delivery.stored().event.id, inactive.id);
        delivery.defer().await.expect("defer failed delivery");
        sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET next_attempt_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3 \
               AND phase='delivery'",
        )
        .bind(community.as_uuid())
        .bind(operation_id.as_uuid())
        .bind(relay.public_key().as_bytes())
        .execute(&db.pool)
        .await
        .expect("make deferred delivery ready");
        let delivery =
            begin_public_projection_delivery(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("retry delivery")
                .expect("deferred delivery exists");
        assert_eq!(delivery.stored().event.id, inactive.id);
        delivery.complete().await.expect("complete delivery");
        assert_eq!(
            unfinished_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("count unfinished"),
            0
        );
        assert!(claim_public_projection_retirement(
            &db,
            &[community],
            relay.public_key().as_bytes(),
        )
        .await
        .expect("replay claim")
        .is_none());
        assert!(
            begin_active_public_projection(
                &db,
                community,
                relay.public_key().as_bytes(),
                ISSUER,
                "subject-one",
                subject.as_bytes(),
            )
            .await
            .expect("revalidate revoked principal")
            .is_none(),
            "committed revocation must prevent assertion reactivation"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn committed_rotation_retires_only_the_old_projection_generation() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let old_keys = Keys::generate();
        let new_keys = Keys::generate();
        let old_key = old_keys.public_key();
        let new_key = new_keys.public_key();
        let old_origin = match resolve_test_binding(
            &db,
            community,
            ISSUER,
            "rotated-subject",
            old_key.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await
        {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected rotation enrollment: {other:?}"),
        };
        let old_active = assertion(&relay, old_key, true, 200);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "rotated-subject",
            old_key.as_bytes(),
        )
        .await
        .expect("begin old projection")
        .expect("old binding is active")
        .commit(&old_active, ProjectionDisposition::Active)
        .await
        .expect("commit old projection");

        rotate_identity_binding(
            &db.pool,
            community,
            lifecycle("test rotation"),
            IdentityPrincipal {
                issuer: ISSUER,
                subject: "rotated-subject",
            },
            old_key.as_bytes(),
            replacement(new_key.as_bytes()),
        )
        .await
        .expect("commit rotation");

        let new_active = assertion(&relay, new_key, true, 201);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "rotated-subject",
            new_key.as_bytes(),
        )
        .await
        .expect("begin replacement projection")
        .expect("replacement binding is active")
        .commit(&new_active, ProjectionDisposition::Active)
        .await
        .expect("commit replacement projection");

        assert_eq!(
            materialize_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("materialize rotation"),
            1
        );
        let claim =
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim rotation")
                .expect("rotation work exists");
        assert_eq!(claim.source_origin(), Some(old_origin));
        let permit = begin_public_projection_retirement(&db, claim)
            .await
            .expect("begin old projection retirement");
        assert_eq!(permit.active_origin(), None);
        let old_inactive = assertion(&relay, old_key, false, 202);
        permit
            .finish_inactive(&old_inactive)
            .await
            .expect("retire old projection");

        let mut tx = db.pool.begin().await.expect("inspect projections");
        let current_old = current_projection_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            old_key.as_bytes(),
        )
        .await
        .expect("read old projection")
        .expect("old projection exists");
        let current_new = current_projection_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            new_key.as_bytes(),
        )
        .await
        .expect("read replacement projection")
        .expect("replacement projection exists");
        tx.rollback().await.expect("rollback inspection");
        assert_eq!(current_old.event.id, old_inactive.id);
        assert_eq!(current_new.event.id, new_active.id);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn active_key_reuse_without_republication_cannot_claim_the_old_assertion() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let reused_keys = Keys::generate();
        let replacement_keys = Keys::generate();
        let reused_key = reused_keys.public_key();
        let replacement_key = replacement_keys.public_key();
        let original_origin = match resolve_test_binding(
            &db,
            community,
            ISSUER,
            "original-principal",
            reused_key.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await
        {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected original enrollment: {other:?}"),
        };
        let original = assertion(&relay, reused_key, true, 250);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "original-principal",
            reused_key.as_bytes(),
        )
        .await
        .expect("begin original projection")
        .expect("original binding is active")
        .commit(&original, ProjectionDisposition::Active)
        .await
        .expect("publish original projection");

        rotate_identity_binding(
            &db.pool,
            community,
            lifecycle("free key for unpublished reuse"),
            IdentityPrincipal {
                issuer: ISSUER,
                subject: "original-principal",
            },
            reused_key.as_bytes(),
            replacement(replacement_key.as_bytes()),
        )
        .await
        .expect("rotate original binding");
        let reused_origin = match resolve_test_binding(
            &db,
            community,
            "https://replacement-idp.example",
            "replacement-principal",
            reused_key.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await
        {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected key reuse: {other:?}"),
        };
        assert_ne!(reused_origin, original_origin);

        materialize_public_projection_retirements(&db, &[community], relay.public_key().as_bytes())
            .await
            .expect("materialize original retirement");
        let permit = begin_public_projection_retirement(
            &db,
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim original retirement")
                .expect("original retirement exists"),
        )
        .await
        .expect("begin original retirement");
        assert_eq!(permit.source_origin(), Some(original_origin));
        assert_eq!(permit.active_origin(), Some(reused_origin));
        assert_eq!(
            permit.head().and_then(ProjectionHead::origin),
            Some(original_origin),
            "B has not published and must not own A's assertion"
        );
        let current = permit
            .current_projection()
            .expect("original assertion remains current")
            .event
            .clone();
        assert!(
            permit.finish_superseded(&current).await.is_err(),
            "active binding existence alone cannot transfer projection ownership"
        );

        let mut tx = db.pool.begin().await.expect("inspect unchanged projection");
        let stored = current_projection_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            reused_key.as_bytes(),
        )
        .await
        .expect("read current projection")
        .expect("current projection remains");
        let head = projection_head_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            reused_key.as_bytes(),
        )
        .await
        .expect("read projection head")
        .expect("projection head remains");
        tx.rollback().await.expect("rollback inspection");
        assert_eq!(stored.event.id, original.id);
        assert_eq!(head.event_id(), *original.id.as_bytes());
        assert_eq!(head.origin(), Some(original_origin));
        assert_eq!(
            unfinished_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("count retryable work"),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn binding_version_advance_without_republication_is_not_a_newer_projection() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let subject_keys = Keys::generate();
        let subject = subject_keys.public_key();
        let first_origin = match resolve_test_binding(
            &db,
            community,
            ISSUER,
            "strengthened-principal",
            subject.as_bytes(),
            EnrollmentMode::Tofu,
            false,
        )
        .await
        {
            ResolveBindingResult::Enrolled(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected tofu enrollment: {other:?}"),
        };
        let original = assertion(&relay, subject, true, 275);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "strengthened-principal",
            subject.as_bytes(),
        )
        .await
        .expect("begin original projection")
        .expect("tofu binding is active")
        .commit(&original, ProjectionDisposition::Active)
        .await
        .expect("publish version-one projection");
        let strengthened_origin = match resolve_test_binding(
            &db,
            community,
            ISSUER,
            "strengthened-principal",
            subject.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await
        {
            ResolveBindingResult::Existing(evidence) => ProjectionBindingOrigin {
                binding_id: evidence.binding_id,
                binding_version: evidence.binding_version,
            },
            other => panic!("unexpected strengthening result: {other:?}"),
        };
        assert_eq!(strengthened_origin.binding_id(), first_origin.binding_id());
        assert!(strengthened_origin.binding_version() > first_origin.binding_version());

        revoke_identity_key(
            &db.pool,
            community,
            lifecycle("revoke strengthened binding"),
            subject.as_bytes(),
        )
        .await
        .expect("commit strengthened revocation");
        materialize_public_projection_retirements(&db, &[community], relay.public_key().as_bytes())
            .await
            .expect("materialize strengthened retirement");
        let permit = begin_public_projection_retirement(
            &db,
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim strengthened retirement")
                .expect("strengthened retirement exists"),
        )
        .await
        .expect("begin strengthened retirement");
        assert_eq!(permit.source_origin(), Some(strengthened_origin));
        assert_eq!(permit.active_origin(), None);
        assert_eq!(
            permit.head().and_then(ProjectionHead::origin),
            Some(first_origin),
            "the projection still belongs to version one"
        );
        assert!(
            permit.finish_newer_projection().await.is_err(),
            "an older head cannot terminalize a later-generation retirement"
        );
        assert_eq!(
            unfinished_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("count retryable strengthened work"),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_rotation_work_cannot_retire_a_reused_key_projection() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let reused_keys = Keys::generate();
        let replacement_keys = Keys::generate();
        let reused_key = reused_keys.public_key();
        let replacement_key = replacement_keys.public_key();
        resolve_test_binding(
            &db,
            community,
            ISSUER,
            "first-principal",
            reused_key.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await;
        let first = assertion(&relay, reused_key, true, 300);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "first-principal",
            reused_key.as_bytes(),
        )
        .await
        .expect("begin first projection")
        .expect("first binding is active")
        .commit(&first, ProjectionDisposition::Active)
        .await
        .expect("commit first projection");

        rotate_identity_binding(
            &db.pool,
            community,
            lifecycle("free key for reuse test"),
            IdentityPrincipal {
                issuer: ISSUER,
                subject: "first-principal",
            },
            reused_key.as_bytes(),
            replacement(replacement_key.as_bytes()),
        )
        .await
        .expect("rotate first principal");
        resolve_test_binding(
            &db,
            community,
            "https://second-idp.example",
            "second-principal",
            reused_key.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await;
        let second = assertion(&relay, reused_key, true, 301);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            "https://second-idp.example",
            "second-principal",
            reused_key.as_bytes(),
        )
        .await
        .expect("begin reused projection")
        .expect("reused binding is active")
        .commit(&second, ProjectionDisposition::Active)
        .await
        .expect("commit reused projection");

        materialize_public_projection_retirements(&db, &[community], relay.public_key().as_bytes())
            .await
            .expect("materialize stale rotation");
        let claim =
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim stale rotation")
                .expect("stale rotation exists");
        let permit = begin_public_projection_retirement(&db, claim)
            .await
            .expect("begin stale rotation");
        assert_ne!(permit.active_origin(), permit.source_origin());
        let current = permit
            .current_projection()
            .expect("reused projection exists")
            .event
            .clone();
        permit
            .finish_superseded(&current)
            .await
            .expect("preserve reused projection");

        let mut tx = db.pool.begin().await.expect("inspect reused projection");
        let stored = current_projection_tx(
            &mut tx,
            community,
            relay.public_key().as_bytes(),
            reused_key.as_bytes(),
        )
        .await
        .expect("read reused projection")
        .expect("reused projection remains");
        tx.rollback().await.expect("rollback inspection");
        assert_eq!(stored.event.id, second.id);
        assert_eq!(
            unfinished_public_projection_retirements(
                &db,
                &[community],
                relay.public_key().as_bytes(),
            )
            .await
            .expect("count stale work"),
            0
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn populated_upgrade_retires_an_assertion_without_private_head_metadata() {
        let (db, community) = setup().await;
        let relay = Keys::generate();
        let subject_keys = Keys::generate();
        let subject = subject_keys.public_key();
        resolve_test_binding(
            &db,
            community,
            ISSUER,
            "legacy-projection-subject",
            subject.as_bytes(),
            EnrollmentMode::AttestedKey,
            true,
        )
        .await;
        let active = assertion(&relay, subject, true, 400);
        begin_active_public_projection(
            &db,
            community,
            relay.public_key().as_bytes(),
            ISSUER,
            "legacy-projection-subject",
            subject.as_bytes(),
        )
        .await
        .expect("begin legacy projection")
        .expect("legacy binding is active")
        .commit(&active, ProjectionDisposition::Active)
        .await
        .expect("commit legacy projection");
        sqlx::query(
            "DELETE FROM identity_public_projection_heads \
             WHERE community_id=$1 AND relay_pubkey=$2 AND subject_pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(relay.public_key().as_bytes())
        .bind(subject.as_bytes())
        .execute(&db.pool)
        .await
        .expect("simulate pre-migration projection");

        let operation_id = LifecycleOperationId::issue();
        revoke_identity_key(
            &db.pool,
            community,
            lifecycle_with_id(operation_id, "retire populated upgrade projection"),
            subject.as_bytes(),
        )
        .await
        .expect("commit populated-upgrade revocation");
        materialize_public_projection_retirements(&db, &[community], relay.public_key().as_bytes())
            .await
            .expect("materialize populated-upgrade work");
        let permit = begin_public_projection_retirement(
            &db,
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim populated-upgrade work")
                .expect("populated-upgrade work exists"),
        )
        .await
        .expect("begin populated-upgrade retirement");
        assert_eq!(permit.head(), None);
        let inactive = assertion(&relay, subject, false, 401);
        permit
            .finish_inactive(&inactive)
            .await
            .expect("retire populated-upgrade projection");
        let delivery =
            begin_public_projection_delivery(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim populated-upgrade delivery")
                .expect("populated-upgrade delivery exists");
        assert_eq!(delivery.stored().event.id, inactive.id);
        delivery.complete().await.expect("complete delivery");

        // A crash-restored/pre-head replica can rediscover already-inactive
        // work without private ownership metadata. Convergence must not
        // relabel that public event to the replayed job's source generation.
        sqlx::query(
            "DELETE FROM identity_public_projection_heads \
             WHERE community_id=$1 AND relay_pubkey=$2 AND subject_pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(relay.public_key().as_bytes())
        .bind(subject.as_bytes())
        .execute(&db.pool)
        .await
        .expect("remove restored private head metadata");
        sqlx::query(
            "UPDATE identity_public_projection_retirements \
             SET source_binding_id=NULL, source_binding_version=NULL, \
                 phase='projection', outcome=NULL, event_id=NULL, \
                 completed_at=NULL, next_attempt_at=NOW(), updated_at=NOW() \
             WHERE community_id=$1 AND operation_id=$2 AND relay_pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(operation_id.as_uuid())
        .bind(relay.public_key().as_bytes())
        .execute(&db.pool)
        .await
        .expect("simulate source-less restored retry");
        let replay = begin_public_projection_retirement(
            &db,
            claim_public_projection_retirement(&db, &[community], relay.public_key().as_bytes())
                .await
                .expect("claim restored retry")
                .expect("restored retry exists"),
        )
        .await
        .expect("begin restored retry");
        assert_eq!(replay.head(), None);
        assert_eq!(
            replay
                .current_projection()
                .expect("inactive projection remains")
                .event
                .id,
            inactive.id
        );
        replay
            .finish_existing_inactive()
            .await
            .expect("source-less inactive retry converges");
        let head_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_public_projection_heads \
             WHERE community_id=$1 AND relay_pubkey=$2 AND subject_pubkey=$3",
        )
        .bind(community.as_uuid())
        .bind(relay.public_key().as_bytes())
        .bind(subject.as_bytes())
        .fetch_one(&db.pool)
        .await
        .expect("count private heads after convergence");
        assert_eq!(head_count, 0, "replay must not manufacture ownership");
    }

    #[test]
    fn public_projection_disposition_is_canonical_and_label_free_when_inactive() {
        let relay = Keys::generate();
        let subject = Keys::generate().public_key();
        let active = assertion(&relay, subject, true, 500);
        let inactive = assertion(&relay, subject, false, 501);
        assert!(validate_event_disposition(&active, ProjectionDisposition::Active).is_ok());
        assert!(validate_event_disposition(&inactive, ProjectionDisposition::Inactive).is_ok());
        assert!(validate_event_disposition(&active, ProjectionDisposition::Inactive).is_err());
        assert!(validate_event_disposition(&inactive, ProjectionDisposition::Active).is_err());

        let subject_hex = subject.to_hex();
        let stale_label = EventBuilder::new(Kind::Custom(ASSERTION_KIND as u16), "")
            .tags([
                Tag::parse(["d", subject_hex.as_str()]).expect("d tag"),
                Tag::parse(["p", subject_hex.as_str()]).expect("p tag"),
                Tag::parse(["verified", "relay"]).expect("verified tag"),
                Tag::parse(["active", "false"]).expect("active tag"),
                Tag::parse(["expiration", "0"]).expect("expiration tag"),
                Tag::parse(["display_name", "Stale label"]).expect("display tag"),
            ])
            .custom_created_at(Timestamp::from(502))
            .sign_with_keys(&relay)
            .expect("sign malformed projection");
        assert!(validate_event_disposition(&stale_label, ProjectionDisposition::Inactive).is_err());

        for (active, at) in [(true, 503), (false, 504)] {
            let active_value = if active { "true" } else { "false" };
            let expiration = if active { "500" } else { "0" };
            let mut tags = vec![
                Tag::parse(["d", subject_hex.as_str()]).expect("d tag"),
                Tag::parse(["p", subject_hex.as_str()]).expect("p tag"),
                Tag::parse(["verified", "relay"]).expect("verified tag"),
                Tag::parse(["active", active_value]).expect("active tag"),
                Tag::parse(["expiration", expiration]).expect("expiration tag"),
            ];
            if active {
                tags.push(
                    Tag::parse(["display_name", "Private stale content"]).expect("display tag"),
                );
            }
            let nonempty = EventBuilder::new(
                Kind::Custom(ASSERTION_KIND as u16),
                "private content must never be projected",
            )
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(&relay)
            .expect("sign non-canonical projection");
            let disposition = if active {
                ProjectionDisposition::Active
            } else {
                ProjectionDisposition::Inactive
            };
            assert!(
                validate_event_disposition(&nonempty, disposition).is_err(),
                "signed projection content must be empty"
            );
        }
    }

    #[test]
    fn debug_receipts_redact_binding_and_public_key_coordinates() {
        let claim = ProjectionRetirementClaim {
            community_id: CommunityId::from_uuid(Uuid::new_v4()),
            operation_id: Uuid::new_v4(),
            relay_pubkey: vec![3; 32],
            old_pubkey: vec![4; 32],
            source_origin: Some(ProjectionBindingOrigin {
                binding_id: Uuid::new_v4(),
                binding_version: 7,
            }),
            claim_token: Uuid::new_v4(),
        };
        let debug = format!("{claim:?}");
        assert!(!debug.contains(&hex::encode(&claim.relay_pubkey)));
        assert!(!debug.contains(&hex::encode(&claim.old_pubkey)));
        assert!(!debug.contains(&claim.operation_id.to_string()));
        assert!(!debug.contains("7"));
    }
}
