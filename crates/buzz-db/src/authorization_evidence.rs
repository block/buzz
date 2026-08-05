//! Durable authorization evidence storage.
//!
//! Mutation callers use [`append_outbox_tx`] inside their existing transaction.
//! Non-mutating decisions use a separate short transaction through the decision
//! APIs in this module. Immutable event rows never carry mutable delivery state.

use buzz_audit::authorization::{
    AcceptedEventMetadata, AuthorizationEventV1, CanonicalEvent, CapacityClass, ControlCode,
    DeliveryAttemptId, DeliveryDisposition, DeliveryKind, DeliveryLease, EventId,
    EvidenceStreamKind, ExporterId, RetryPolicy, StreamId,
};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Row, Transaction};
use std::time::Duration;
use uuid::Uuid;

use crate::{DbError, Result};

/// Receipt proving one event was durably accepted into a stream.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AcceptedEvidence {
    /// Immutable event identity.
    pub event_id: EventId,
    /// Durable stream identity.
    pub stream_id: StreamId,
    /// Stream-local position.
    pub stream_position: u64,
    /// Semantic content digest used for idempotency.
    pub content_digest: [u8; 32],
    /// Chain digest at this stream position.
    pub chain_digest: [u8; 32],
}

impl std::fmt::Debug for AcceptedEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcceptedEvidence")
            .field("event_id", &"[redacted]")
            .field("stream_id", &"[redacted]")
            .field("stream_position", &self.stream_position)
            .field("content_digest", &"[redacted]")
            .field("chain_digest", &"[redacted]")
            .finish()
    }
}

/// Opaque release token for a value whose non-mutating decision is durable.
pub struct AcceptedDecision<T> {
    value: T,
    evidence: AcceptedEvidence,
}

impl<T> AcceptedDecision<T> {
    /// Consume the token and release the protected value.
    pub fn into_value(self) -> T {
        self.value
    }

    /// Inspect only the redaction-safe durable acceptance receipt.
    pub const fn evidence(&self) -> AcceptedEvidence {
        self.evidence
    }
}

impl<T> std::fmt::Debug for AcceptedDecision<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AcceptedDecision")
            .field("value", &"[redacted]")
            .field("evidence", &self.evidence)
            .finish()
    }
}

impl crate::Db {
    /// Durably accept a non-mutating decision before releasing its value.
    pub async fn accept_authorization_decision<T>(
        &self,
        event: &AuthorizationEventV1,
        capacity: CapacityClass,
        value: T,
    ) -> Result<AcceptedDecision<T>> {
        let mut tx = self.pool.begin().await?;
        let evidence = append_decision_tx(&mut tx, event, capacity).await?;
        tx.commit().await?;
        Ok(AcceptedDecision { value, evidence })
    }

    /// Claim the earliest unexported event while preserving stream-local order.
    pub async fn claim_authorization_delivery(
        &self,
        community_id: CommunityId,
        kind: DeliveryKind,
        exporter_id: ExporterId,
        lease_duration: Duration,
    ) -> Result<Option<DeliveryLease>> {
        let lease_millis = i64::try_from(lease_duration.as_millis()).map_err(|_| {
            DbError::InvalidData("authorization delivery lease is out of range".into())
        })?;
        if lease_millis <= 0 || lease_duration > Duration::from_secs(5 * 60) {
            return Err(DbError::InvalidData(
                "authorization delivery lease is out of range".into(),
            ));
        }
        let lane = EvidenceLane::from_delivery(kind);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(lane.claim_sql())
            .bind(community_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let event_id = EventId::from_uuid(row.try_get("event_id")?)
            .map_err(|error| DbError::InvalidData(error.to_string()))?;
        let attempt = positive_u32(
            row.try_get::<i32, _>("attempt_count")?
                .checked_add(1)
                .ok_or_else(|| {
                    DbError::InvalidData("authorization delivery attempt exhausted".into())
                })?,
            "attempt",
        )?;
        let delivery_attempt_id = DeliveryAttemptId::generate();
        let lease_expires_at: DateTime<Utc> = sqlx::query_scalar(lane.lease_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .bind(delivery_attempt_id.as_uuid())
            .bind(exporter_id.as_uuid())
            .bind(lease_millis)
            .fetch_one(&mut *tx)
            .await?;
        let lease = DeliveryLease::new(
            kind,
            event_id,
            StreamId::from_uuid(row.try_get("stream_id")?)
                .map_err(|error| DbError::InvalidData(error.to_string()))?,
            positive_u64(row.try_get("stream_position")?, "stream position")?,
            delivery_attempt_id,
            attempt,
            lease_expires_at,
            row.try_get("canonical_event")?,
            digest_array(row.try_get("content_digest")?)?,
            digest_array(row.try_get("chain_digest")?)?,
        );
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        lease
            .validate(now)
            .map_err(|error| DbError::InvalidData(error.to_string()))?;
        tx.commit().await?;
        Ok(Some(lease))
    }

    /// Idempotently acknowledge sink acceptance for the current delivery attempt.
    pub async fn acknowledge_authorization_delivery(
        &self,
        community_id: CommunityId,
        kind: DeliveryKind,
        event_id: EventId,
        delivery_attempt_id: DeliveryAttemptId,
        content_digest: [u8; 32],
    ) -> Result<()> {
        let lane = EvidenceLane::from_delivery(kind);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(lane.delivery_state_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::InvalidData("authorization delivery was not found".into()))?;
        let state: String = row.try_get("delivery_state")?;
        let stored_digest = digest_array(row.try_get("content_digest")?)?;
        if stored_digest != content_digest {
            return Err(DbError::InvalidData(
                "authorization sink acknowledgement digest does not match".into(),
            ));
        }
        if state == "exported" {
            tx.commit().await?;
            return Ok(());
        }
        let stored_attempt: Option<Uuid> = row.try_get("delivery_attempt_id")?;
        if state != "leased" || stored_attempt != Some(delivery_attempt_id.as_uuid()) {
            return Err(DbError::InvalidData(
                "authorization delivery attempt is not current".into(),
            ));
        }
        let capacity = parse_capacity(row.try_get("capacity_class")?)?;
        let updated = sqlx::query(lane.ack_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .bind(delivery_attempt_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization delivery acknowledgement lost its lease".into(),
            ));
        }
        release_capacity_tx(&mut tx, community_id, capacity).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Retry or quarantine one failed delivery without mutating its event row.
    #[allow(clippy::too_many_arguments)]
    pub async fn fail_authorization_delivery(
        &self,
        community_id: CommunityId,
        kind: DeliveryKind,
        event_id: EventId,
        delivery_attempt_id: DeliveryAttemptId,
        disposition: DeliveryDisposition,
        retry_policy: RetryPolicy,
    ) -> Result<()> {
        let lane = EvidenceLane::from_delivery(kind);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(lane.delivery_state_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::InvalidData("authorization delivery was not found".into()))?;
        let state: String = row.try_get("delivery_state")?;
        if state == "exported" || state == "quarantined" {
            tx.commit().await?;
            return Ok(());
        }
        let stored_attempt: Option<Uuid> = row.try_get("delivery_attempt_id")?;
        if state != "leased" || stored_attempt != Some(delivery_attempt_id.as_uuid()) {
            return Err(DbError::InvalidData(
                "authorization delivery attempt is not current".into(),
            ));
        }
        let attempt = positive_u32(row.try_get("attempt_count")?, "attempt")?;
        let (control_code, force_quarantine) = match disposition {
            DeliveryDisposition::Accepted => {
                return Err(DbError::InvalidData(
                    "accepted authorization delivery must use acknowledgement".into(),
                ));
            }
            DeliveryDisposition::Retry(code) => (code, attempt >= retry_policy.maximum_attempts()),
            DeliveryDisposition::Quarantine(code) => (code, true),
        };
        if force_quarantine {
            quarantine_delivery_tx(
                &mut tx,
                community_id,
                lane,
                event_id,
                delivery_attempt_id,
                control_code,
            )
            .await?;
        } else {
            let delay = retry_policy.delay_for(attempt).ok_or_else(|| {
                DbError::InvalidData("authorization retry attempt is out of range".into())
            })?;
            let delay_millis = i64::try_from(delay.as_millis()).map_err(|_| {
                DbError::InvalidData("authorization retry delay is out of range".into())
            })?;
            let updated = sqlx::query(lane.retry_sql())
                .bind(community_id.as_uuid())
                .bind(event_id.as_uuid())
                .bind(delivery_attempt_id.as_uuid())
                .bind(control_code as i16)
                .bind(delay_millis)
                .execute(&mut *tx)
                .await?;
            if updated.rows_affected() != 1 {
                return Err(DbError::InvalidData(
                    "authorization delivery retry lost its lease".into(),
                ));
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Requeue one quarantined event while retaining immutable dead-letter evidence.
    #[allow(clippy::too_many_arguments)]
    pub async fn restore_authorization_delivery(
        &self,
        community_id: CommunityId,
        kind: DeliveryKind,
        event_id: EventId,
        content_digest: [u8; 32],
        actor_reference: [u8; 32],
        control_code: ControlCode,
    ) -> Result<Uuid> {
        if actor_reference == [0; 32] {
            return Err(DbError::InvalidData(
                "authorization restoration actor reference is invalid".into(),
            ));
        }
        let lane = EvidenceLane::from_delivery(kind);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(lane.delivery_state_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| DbError::InvalidData("authorization delivery was not found".into()))?;
        let state: String = row.try_get("delivery_state")?;
        let stored_digest = digest_array(row.try_get("content_digest")?)?;
        if state != "quarantined" || stored_digest != content_digest {
            return Err(DbError::InvalidData(
                "authorization restoration target is not the exact quarantined event".into(),
            ));
        }
        let prior_attempt: Uuid = sqlx::query_scalar(lane.dead_letter_attempt_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::InvalidData("authorization quarantine has no dead-letter attempt".into())
            })?;
        let updated = sqlx::query(lane.restore_sql())
            .bind(community_id.as_uuid())
            .bind(event_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        if updated.rows_affected() != 1 {
            return Err(DbError::InvalidData(
                "authorization restoration lost its quarantined event".into(),
            ));
        }
        let restoration_id = Uuid::new_v4();
        sqlx::query(lane.insert_restoration_sql())
            .bind(community_id.as_uuid())
            .bind(restoration_id)
            .bind(event_id.as_uuid())
            .bind(prior_attempt)
            .bind(actor_reference.as_slice())
            .bind(control_code as i16)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(restoration_id)
    }
}

/// Append mutation evidence without committing the caller-owned transaction.
pub async fn append_outbox_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &AuthorizationEventV1,
    capacity: CapacityClass,
) -> Result<AcceptedEvidence> {
    append_event_tx(tx, event, capacity, EvidenceLane::AuditOutbox).await
}

/// Append non-mutating decision evidence without committing the caller transaction.
pub async fn append_decision_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &AuthorizationEventV1,
    capacity: CapacityClass,
) -> Result<AcceptedEvidence> {
    append_event_tx(tx, event, capacity, EvidenceLane::Decision).await
}

async fn append_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &AuthorizationEventV1,
    capacity: CapacityClass,
    lane: EvidenceLane,
) -> Result<AcceptedEvidence> {
    let content_digest = CanonicalEvent::semantic_digest(event);
    // Serialize the first append and every replay by exact domain, lane, and
    // event identity. Without this lock two concurrent first attempts can both
    // miss the lookup and turn an equal replay into a unique-key error.
    let append_lock = format!(
        "buzz-authorization-evidence:{}:{}:{}",
        event.domain().as_uuid(),
        lane.stream_kind().discriminant(),
        event.event_id().as_uuid(),
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(append_lock)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO authorization_evidence_event_registry \
         (community_id, event_id, stream_kind, content_digest) VALUES ($1,$2,$3,$4) \
         ON CONFLICT (community_id, event_id) DO NOTHING",
    )
    .bind(event.domain().as_uuid())
    .bind(event.event_id().as_uuid())
    .bind(i16::from(lane.stream_kind().discriminant()))
    .bind(content_digest.as_slice())
    .execute(&mut **tx)
    .await?;
    let registered = sqlx::query(
        "SELECT stream_kind, content_digest FROM authorization_evidence_event_registry \
         WHERE community_id=$1 AND event_id=$2 FOR SHARE",
    )
    .bind(event.domain().as_uuid())
    .bind(event.event_id().as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    let registered_kind: i16 = registered.try_get("stream_kind")?;
    let registered_digest = digest_array(registered.try_get("content_digest")?)?;
    if registered_kind != i16::from(lane.stream_kind().discriminant())
        || registered_digest != content_digest
    {
        return Err(DbError::InvalidData(
            "authorization event identity was reused across evidence lanes or content".into(),
        ));
    }
    if let Some(existing) = existing_event_tx(tx, event, lane).await? {
        if existing.content_digest != content_digest {
            return Err(DbError::InvalidData(
                "authorization event ID was reused with different content".into(),
            ));
        }
        return Ok(existing);
    }

    reserve_capacity_tx(tx, event, capacity).await?;
    let proposed_stream = StreamId::generate();
    sqlx::query(
        "INSERT INTO authorization_evidence_stream_heads \
         (community_id, stream_kind, stream_id) VALUES ($1, $2, $3) \
         ON CONFLICT (community_id, stream_kind) DO NOTHING",
    )
    .bind(event.domain().as_uuid())
    .bind(i16::from(lane.stream_kind().discriminant()))
    .bind(proposed_stream.as_uuid())
    .execute(&mut **tx)
    .await?;

    let head = sqlx::query(
        "SELECT stream_id, next_position, terminal_digest \
         FROM authorization_evidence_stream_heads \
         WHERE community_id=$1 AND stream_kind=$2 FOR UPDATE",
    )
    .bind(event.domain().as_uuid())
    .bind(i16::from(lane.stream_kind().discriminant()))
    .fetch_one(&mut **tx)
    .await?;
    let stream_id = StreamId::from_uuid(head.try_get("stream_id")?)
        .map_err(|error| DbError::InvalidData(error.to_string()))?;
    let stream_position = positive_u64(head.try_get("next_position")?, "stream position")?;
    let previous_digest = digest_array(head.try_get("terminal_digest")?)?;
    let accepted_at: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?;
    let accepted =
        AcceptedEventMetadata::new(stream_id, stream_position, previous_digest, accepted_at)
            .map_err(|error| DbError::InvalidData(error.to_string()))?;
    let canonical = CanonicalEvent::encode(event, accepted);
    sqlx::query(lane.insert_event_sql())
        .bind(event.domain().as_uuid())
        .bind(event.event_id().as_uuid())
        .bind(stream_id.as_uuid())
        .bind(
            i64::try_from(stream_position).map_err(|_| {
                DbError::InvalidData("authorization stream position exhausted".into())
            })?,
        )
        .bind(event.occurred_at())
        .bind(accepted_at)
        .bind(event.operation_id().map(|value| value.as_uuid()))
        .bind(event.correlation_id().as_uuid())
        .bind(event.attempt_id().as_uuid())
        .bind(
            i16::try_from(event.kind().discriminant()).map_err(|_| {
                DbError::InvalidData("authorization event kind is out of range".into())
            })?,
        )
        .bind(i16::try_from(event.result().discriminant()).map_err(|_| {
            DbError::InvalidData("authorization event result is out of range".into())
        })?)
        .bind(i16::try_from(event.reason().discriminant()).map_err(|_| {
            DbError::InvalidData("authorization decision reason is out of range".into())
        })?)
        .bind(
            i16::try_from(event.actor_class().discriminant()).map_err(|_| {
                DbError::InvalidData("authorization actor class is out of range".into())
            })?,
        )
        .bind(canonical.bytes())
        .bind(canonical.content_digest().as_slice())
        .bind(previous_digest.as_slice())
        .bind(canonical.chain_digest().as_slice())
        .execute(&mut **tx)
        .await?;

    sqlx::query(lane.insert_delivery_sql())
        .bind(event.domain().as_uuid())
        .bind(event.event_id().as_uuid())
        .bind(capacity as i16)
        .execute(&mut **tx)
        .await?;
    let next_position = stream_position
        .checked_add(1)
        .ok_or_else(|| DbError::InvalidData("authorization stream position exhausted".into()))?;
    sqlx::query(
        "UPDATE authorization_evidence_stream_heads \
         SET next_position=$3, terminal_digest=$4, updated_at=clock_timestamp() \
         WHERE community_id=$1 AND stream_kind=$2",
    )
    .bind(event.domain().as_uuid())
    .bind(i16::from(lane.stream_kind().discriminant()))
    .bind(
        i64::try_from(next_position)
            .map_err(|_| DbError::InvalidData("authorization stream position exhausted".into()))?,
    )
    .bind(canonical.chain_digest().as_slice())
    .execute(&mut **tx)
    .await?;

    Ok(AcceptedEvidence {
        event_id: event.event_id(),
        stream_id,
        stream_position,
        content_digest: canonical.content_digest(),
        chain_digest: canonical.chain_digest(),
    })
}

async fn existing_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &AuthorizationEventV1,
    lane: EvidenceLane,
) -> Result<Option<AcceptedEvidence>> {
    let row = sqlx::query(lane.select_event_sql())
        .bind(event.domain().as_uuid())
        .bind(event.event_id().as_uuid())
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|row| {
        Ok(AcceptedEvidence {
            event_id: event.event_id(),
            stream_id: StreamId::from_uuid(row.try_get("stream_id")?)
                .map_err(|error| DbError::InvalidData(error.to_string()))?,
            stream_position: positive_u64(row.try_get("stream_position")?, "stream position")?,
            content_digest: digest_array(row.try_get("content_digest")?)?,
            chain_digest: digest_array(row.try_get("chain_digest")?)?,
        })
    })
    .transpose()
}

async fn reserve_capacity_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &AuthorizationEventV1,
    capacity: CapacityClass,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO authorization_evidence_capacity_state (community_id) \
         VALUES ($1) ON CONFLICT (community_id) DO NOTHING",
    )
    .bind(event.domain().as_uuid())
    .execute(&mut **tx)
    .await?;
    let query = match capacity {
        CapacityClass::RestrictiveReserve => {
            "UPDATE authorization_evidence_capacity_state \
             SET restrictive_remaining=restrictive_remaining-1, revision=revision+1, \
                 updated_at=clock_timestamp() \
             WHERE community_id=$1 AND restrictive_remaining>0 RETURNING revision"
        }
        CapacityClass::NewAllow => {
            "UPDATE authorization_evidence_capacity_state \
             SET general_remaining=general_remaining-1, revision=revision+1, \
                 updated_at=clock_timestamp() \
             WHERE community_id=$1 AND general_remaining>0 RETURNING revision"
        }
        CapacityClass::NonessentialRead => {
            "UPDATE authorization_evidence_capacity_state \
             SET general_remaining=general_remaining-1, revision=revision+1, \
                 updated_at=clock_timestamp() \
             WHERE community_id=$1 AND general_remaining>allow_reserve RETURNING revision"
        }
    };
    if sqlx::query_scalar::<_, i64>(query)
        .bind(event.domain().as_uuid())
        .fetch_optional(&mut **tx)
        .await?
        .is_none()
    {
        return Err(DbError::InvalidData(
            "authorization evidence capacity is exhausted".into(),
        ));
    }
    Ok(())
}

async fn release_capacity_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    capacity: CapacityClass,
) -> Result<()> {
    let query = match capacity {
        CapacityClass::RestrictiveReserve => {
            "UPDATE authorization_evidence_capacity_state \
             SET restrictive_remaining=restrictive_remaining+1, revision=revision+1, \
                 updated_at=clock_timestamp() WHERE community_id=$1"
        }
        CapacityClass::NewAllow | CapacityClass::NonessentialRead => {
            "UPDATE authorization_evidence_capacity_state \
             SET general_remaining=general_remaining+1, revision=revision+1, \
                 updated_at=clock_timestamp() WHERE community_id=$1"
        }
    };
    let updated = sqlx::query(query)
        .bind(community_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "authorization evidence capacity state was not found".into(),
        ));
    }
    Ok(())
}

async fn quarantine_delivery_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    lane: EvidenceLane,
    event_id: EventId,
    delivery_attempt_id: DeliveryAttemptId,
    control_code: ControlCode,
) -> Result<()> {
    let updated = sqlx::query(lane.quarantine_sql())
        .bind(community_id.as_uuid())
        .bind(event_id.as_uuid())
        .bind(delivery_attempt_id.as_uuid())
        .bind(control_code as i16)
        .execute(&mut **tx)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(DbError::InvalidData(
            "authorization delivery quarantine lost its lease".into(),
        ));
    }
    sqlx::query(lane.dead_letter_sql())
        .bind(community_id.as_uuid())
        .bind(Uuid::new_v4())
        .bind(event_id.as_uuid())
        .bind(delivery_attempt_id.as_uuid())
        .bind(control_code as i16)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn parse_capacity(value: i16) -> Result<CapacityClass> {
    match value {
        1 => Ok(CapacityClass::RestrictiveReserve),
        2 => Ok(CapacityClass::NewAllow),
        3 => Ok(CapacityClass::NonessentialRead),
        _ => Err(DbError::InvalidData(
            "authorization evidence capacity class is invalid".into(),
        )),
    }
}

fn positive_u64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        DbError::InvalidData(format!("authorization evidence {label} is out of range"))
    })
}

fn positive_u32(value: i32, label: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| {
        DbError::InvalidData(format!("authorization evidence {label} is out of range"))
    })
}

fn digest_array(value: Vec<u8>) -> Result<[u8; 32]> {
    value.try_into().map_err(|_| {
        DbError::InvalidData("authorization evidence digest has invalid length".into())
    })
}

#[derive(Clone, Copy)]
enum EvidenceLane {
    AuditOutbox,
    Decision,
}

impl EvidenceLane {
    const fn from_delivery(kind: DeliveryKind) -> Self {
        match kind {
            DeliveryKind::AuditOutbox => Self::AuditOutbox,
            DeliveryKind::Decision => Self::Decision,
        }
    }

    const fn stream_kind(self) -> EvidenceStreamKind {
        match self {
            Self::AuditOutbox => EvidenceStreamKind::AuditOutbox,
            Self::Decision => EvidenceStreamKind::Decision,
        }
    }

    const fn select_event_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "SELECT stream_id, stream_position, content_digest, chain_digest \
                 FROM authorization_audit_outbox WHERE community_id=$1 AND event_id=$2"
            }
            Self::Decision => {
                "SELECT stream_id, stream_position, content_digest, chain_digest \
                 FROM authorization_decision_events WHERE community_id=$1 AND event_id=$2"
            }
        }
    }

    const fn insert_event_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "INSERT INTO authorization_audit_outbox \
                 (community_id, event_id, stream_id, stream_position, schema_version, \
                  occurred_at, accepted_at, operation_id, correlation_id, attempt_id, \
                  event_kind, event_result, decision_reason, actor_class, canonical_event, \
                  content_digest, previous_digest, chain_digest) \
                 VALUES ($1,$2,$3,$4,1,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"
            }
            Self::Decision => {
                "INSERT INTO authorization_decision_events \
                 (community_id, event_id, stream_id, stream_position, schema_version, \
                  occurred_at, accepted_at, operation_id, correlation_id, attempt_id, \
                  event_kind, event_result, decision_reason, actor_class, canonical_event, \
                  content_digest, previous_digest, chain_digest) \
                 VALUES ($1,$2,$3,$4,1,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"
            }
        }
    }

    const fn insert_delivery_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "INSERT INTO authorization_audit_outbox_delivery \
                 (community_id, event_id, capacity_class) VALUES ($1, $2, $3)"
            }
            Self::Decision => {
                "INSERT INTO authorization_decision_delivery \
                 (community_id, event_id, capacity_class) VALUES ($1, $2, $3)"
            }
        }
    }

    const fn claim_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "SELECT event.event_id, event.stream_id, event.stream_position, \
                        event.canonical_event, event.content_digest, event.chain_digest, delivery.attempt_count \
                 FROM authorization_audit_outbox event \
                 JOIN authorization_audit_outbox_delivery delivery \
                   ON delivery.community_id=event.community_id \
                  AND delivery.event_id=event.event_id \
                 WHERE event.community_id=$1 \
                   AND ((delivery.delivery_state='pending' \
                         AND delivery.next_attempt_at<=clock_timestamp()) \
                     OR (delivery.delivery_state='leased' \
                         AND delivery.lease_expires_at<=clock_timestamp())) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM authorization_audit_outbox prior \
                       JOIN authorization_audit_outbox_delivery prior_delivery \
                         ON prior_delivery.community_id=prior.community_id \
                        AND prior_delivery.event_id=prior.event_id \
                       WHERE prior.community_id=event.community_id \
                         AND prior.stream_id=event.stream_id \
                         AND prior.stream_position<event.stream_position \
                         AND prior_delivery.delivery_state NOT IN ('exported','quarantined') \
                   ) \
                 ORDER BY event.stream_position LIMIT 1 \
                 FOR UPDATE OF delivery SKIP LOCKED"
            }
            Self::Decision => {
                "SELECT event.event_id, event.stream_id, event.stream_position, \
                        event.canonical_event, event.content_digest, event.chain_digest, delivery.attempt_count \
                 FROM authorization_decision_events event \
                 JOIN authorization_decision_delivery delivery \
                   ON delivery.community_id=event.community_id \
                  AND delivery.event_id=event.event_id \
                 WHERE event.community_id=$1 \
                   AND ((delivery.delivery_state='pending' \
                         AND delivery.next_attempt_at<=clock_timestamp()) \
                     OR (delivery.delivery_state='leased' \
                         AND delivery.lease_expires_at<=clock_timestamp())) \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM authorization_decision_events prior \
                       JOIN authorization_decision_delivery prior_delivery \
                         ON prior_delivery.community_id=prior.community_id \
                        AND prior_delivery.event_id=prior.event_id \
                       WHERE prior.community_id=event.community_id \
                         AND prior.stream_id=event.stream_id \
                         AND prior.stream_position<event.stream_position \
                         AND prior_delivery.delivery_state NOT IN ('exported','quarantined') \
                   ) \
                 ORDER BY event.stream_position LIMIT 1 \
                 FOR UPDATE OF delivery SKIP LOCKED"
            }
        }
    }

    const fn lease_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "UPDATE authorization_audit_outbox_delivery \
                 SET delivery_state='leased', attempt_count=attempt_count+1, \
                     delivery_attempt_id=$3, lease_owner=$4, \
                     lease_expires_at=clock_timestamp()+($5*interval '1 millisecond'), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 \
                 RETURNING lease_expires_at"
            }
            Self::Decision => {
                "UPDATE authorization_decision_delivery \
                 SET delivery_state='leased', attempt_count=attempt_count+1, \
                     delivery_attempt_id=$3, lease_owner=$4, \
                     lease_expires_at=clock_timestamp()+($5*interval '1 millisecond'), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 \
                 RETURNING lease_expires_at"
            }
        }
    }

    const fn delivery_state_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "SELECT delivery.delivery_state, delivery.attempt_count, \
                        delivery.delivery_attempt_id, delivery.capacity_class, event.content_digest \
                 FROM authorization_audit_outbox_delivery delivery \
                 JOIN authorization_audit_outbox event \
                   ON event.community_id=delivery.community_id AND event.event_id=delivery.event_id \
                 WHERE delivery.community_id=$1 AND delivery.event_id=$2 FOR UPDATE OF delivery"
            }
            Self::Decision => {
                "SELECT delivery.delivery_state, delivery.attempt_count, \
                        delivery.delivery_attempt_id, delivery.capacity_class, event.content_digest \
                 FROM authorization_decision_delivery delivery \
                 JOIN authorization_decision_events event \
                   ON event.community_id=delivery.community_id AND event.event_id=delivery.event_id \
                 WHERE delivery.community_id=$1 AND delivery.event_id=$2 FOR UPDATE OF delivery"
            }
        }
    }

    const fn restore_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "UPDATE authorization_audit_outbox_delivery \
                 SET delivery_state='pending', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, next_attempt_at=clock_timestamp(), \
                     last_control_code=NULL, updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='quarantined'"
            }
            Self::Decision => {
                "UPDATE authorization_decision_delivery \
                 SET delivery_state='pending', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, next_attempt_at=clock_timestamp(), \
                     last_control_code=NULL, updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='quarantined'"
            }
        }
    }

    const fn insert_restoration_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "INSERT INTO authorization_evidence_restorations \
                 (community_id,restoration_id,audit_event_id,prior_delivery_attempt_id, \
                  actor_reference,control_code) VALUES ($1,$2,$3,$4,$5,$6)"
            }
            Self::Decision => {
                "INSERT INTO authorization_evidence_restorations \
                 (community_id,restoration_id,decision_event_id,prior_delivery_attempt_id, \
                  actor_reference,control_code) VALUES ($1,$2,$3,$4,$5,$6)"
            }
        }
    }

    const fn dead_letter_attempt_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "SELECT delivery_attempt_id FROM authorization_evidence_dead_letters \
                 WHERE community_id=$1 AND audit_event_id=$2 \
                 ORDER BY observed_at DESC, observation_id DESC LIMIT 1"
            }
            Self::Decision => {
                "SELECT delivery_attempt_id FROM authorization_evidence_dead_letters \
                 WHERE community_id=$1 AND decision_event_id=$2 \
                 ORDER BY observed_at DESC, observation_id DESC LIMIT 1"
            }
        }
    }

    const fn ack_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "UPDATE authorization_audit_outbox_delivery \
                 SET delivery_state='exported', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, acknowledged_at=clock_timestamp(), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
            Self::Decision => {
                "UPDATE authorization_decision_delivery \
                 SET delivery_state='exported', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, acknowledged_at=clock_timestamp(), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
        }
    }

    const fn retry_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "UPDATE authorization_audit_outbox_delivery \
                 SET delivery_state='pending', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, last_control_code=$4, \
                     next_attempt_at=clock_timestamp()+($5*interval '1 millisecond'), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
            Self::Decision => {
                "UPDATE authorization_decision_delivery \
                 SET delivery_state='pending', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, last_control_code=$4, \
                     next_attempt_at=clock_timestamp()+($5*interval '1 millisecond'), \
                     updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
        }
    }

    const fn quarantine_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "UPDATE authorization_audit_outbox_delivery \
                 SET delivery_state='quarantined', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, last_control_code=$4, updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
            Self::Decision => {
                "UPDATE authorization_decision_delivery \
                 SET delivery_state='quarantined', delivery_attempt_id=NULL, lease_owner=NULL, \
                     lease_expires_at=NULL, last_control_code=$4, updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND event_id=$2 AND delivery_state='leased' \
                   AND delivery_attempt_id=$3"
            }
        }
    }

    const fn dead_letter_sql(self) -> &'static str {
        match self {
            Self::AuditOutbox => {
                "INSERT INTO authorization_evidence_dead_letters \
                 (community_id, observation_id, audit_event_id, decision_event_id, \
                  delivery_attempt_id, control_code) \
                 VALUES ($1,$2,$3,NULL,$4,$5)"
            }
            Self::Decision => {
                "INSERT INTO authorization_evidence_dead_letters \
                 (community_id, observation_id, audit_event_id, decision_event_id, \
                  delivery_attempt_id, control_code) \
                 VALUES ($1,$2,NULL,$3,$4,$5)"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use buzz_audit::authorization::{
        ActorReference, AttemptId, AuthorizationEventV1, CapacityClass, ControlCode, CorrelationId,
        DecisionReason, DeliveryDisposition, DeliveryKind, EventId, EventKind, EventPayloadV1,
        EventResult, ExporterId, OperationClass, OperationId, RetryPolicy, SourceClass, StreamId,
        TransportClass, VersionVectorV1,
    };
    use buzz_core::CommunityId;
    use chrono::Utc;
    use sqlx::Row;
    use uuid::Uuid;

    use crate::test_support::IsolatedPostgres;

    use super::{append_outbox_tx, AcceptedDecision, AcceptedEvidence};

    fn event(
        domain: CommunityId,
        event_id: EventId,
        kind: EventKind,
        result: EventResult,
        reason: DecisionReason,
    ) -> AuthorizationEventV1 {
        AuthorizationEventV1::new(
            event_id,
            domain,
            Utc::now(),
            Some(OperationId::generate()),
            CorrelationId::generate(),
            AttemptId::generate(),
            None,
            ActorReference::ControlPlane,
            TransportClass::Internal,
            OperationClass::Lifecycle,
            SourceClass::Lifecycle,
            kind,
            result,
            reason,
            VersionVectorV1::default(),
            EventPayloadV1::None,
        )
    }

    #[test]
    fn migration_has_immutable_payload_and_separate_delivery() {
        let outbox = include_str!("../../../migrations/0046_authorization_audit_outbox.sql");
        let decisions = include_str!("../../../migrations/0047_authorization_decision_queue.sql");
        let delivery = include_str!("../../../migrations/0048_authorization_evidence_delivery.sql");
        assert!(outbox.contains("CREATE TABLE authorization_audit_outbox"));
        assert!(delivery.contains("CREATE TABLE authorization_audit_outbox_delivery"));
        assert!(decisions.contains("CREATE TABLE authorization_decision_events"));
        assert!(outbox.contains("BEFORE UPDATE OR DELETE ON authorization_audit_outbox"));
        assert!(!format!("{outbox}{decisions}{delivery}").contains("JSONB"));
        assert!(!outbox.contains("authorization_operation_receipts"));
    }

    #[test]
    fn accepted_decision_debug_redacts_protected_value() {
        let token = AcceptedDecision {
            value: "synthetic-private-result",
            evidence: AcceptedEvidence {
                event_id: EventId::generate(),
                stream_id: StreamId::generate(),
                stream_position: 1,
                content_digest: [1; 32],
                chain_digest: [2; 32],
            },
        };
        assert!(!format!("{token:?}").contains("synthetic-private-result"));
        assert!(!format!("{token:?}").contains(&hex::encode([1; 32])));
        assert!(!format!("{token:?}").contains(&hex::encode([2; 32])));
    }

    #[test]
    fn delivery_sql_preserves_order_and_immutable_payloads() {
        for lane in [
            super::EvidenceLane::AuditOutbox,
            super::EvidenceLane::Decision,
        ] {
            assert!(lane.claim_sql().contains("NOT EXISTS"));
            assert!(lane
                .claim_sql()
                .contains("stream_position<event.stream_position"));
            assert!(lane.ack_sql().contains("delivery_state='exported'"));
            assert!(!lane.ack_sql().contains("canonical_event"));
            assert!(lane
                .dead_letter_sql()
                .contains("authorization_evidence_dead_letters"));
        }
        let sql = include_str!("../../../migrations/0048_authorization_evidence_delivery.sql");
        assert!(sql.contains("allow_reserve"));
    }

    #[tokio::test]
    async fn postgres_o5_outbox_rollback_delivery_restore_and_capacity_are_non_vacuous() {
        let fixture = IsolatedPostgres::migrated("evidence").await;
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!("{}.o5.test", domain.as_uuid()))
            .execute(&fixture.pool)
            .await
            .expect("insert synthetic evidence domain");
        let mut scenarios = 0_u32;

        let rolled_back = event(
            domain,
            EventId::generate(),
            EventKind::OperatorBindingRevoked,
            EventResult::Applied,
            DecisionReason::Applied,
        );
        let mut transaction = fixture.pool.begin().await.expect("begin rollback case");
        append_outbox_tx(&mut transaction, &rolled_back, CapacityClass::NewAllow)
            .await
            .expect("append inside rollback transaction");
        transaction
            .rollback()
            .await
            .expect("rollback outbox append");
        let rolled_back_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authorization_audit_outbox WHERE community_id=$1 AND event_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(rolled_back.event_id().as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .expect("inspect rollback");
        assert_eq!(rolled_back_count, 0, "outbox must share caller rollback");
        scenarios += 1;

        let first = event(
            domain,
            EventId::generate(),
            EventKind::AdmissionAllowed,
            EventResult::Allowed,
            DecisionReason::PolicyAllowed,
        );
        let second = event(
            domain,
            EventId::generate(),
            EventKind::AdmissionDenied,
            EventResult::Denied,
            DecisionReason::PolicyDenied,
        );
        let accepted_first = fixture
            .db
            .accept_authorization_decision(&first, CapacityClass::NewAllow, ())
            .await
            .expect("accept first decision")
            .evidence();
        let replay = fixture
            .db
            .accept_authorization_decision(&first, CapacityClass::NewAllow, ())
            .await
            .expect("idempotently accept duplicate decision")
            .evidence();
        assert_eq!(accepted_first, replay);
        fixture
            .db
            .accept_authorization_decision(&second, CapacityClass::RestrictiveReserve, ())
            .await
            .expect("accept second decision");
        scenarios += 1;

        let conflicting = event(
            domain,
            first.event_id(),
            EventKind::AdmissionDenied,
            EventResult::Denied,
            DecisionReason::PolicyDenied,
        );
        assert!(fixture
            .db
            .accept_authorization_decision(&conflicting, CapacityClass::RestrictiveReserve, ())
            .await
            .is_err());
        scenarios += 1;

        let exporter = ExporterId::generate();
        let crashed = fixture
            .db
            .claim_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                exporter,
                Duration::from_secs(30),
            )
            .await
            .expect("claim first event")
            .expect("first event lease");
        assert_eq!(crashed.stream_position(), 1);
        sqlx::query(
            "UPDATE authorization_decision_delivery SET lease_expires_at=clock_timestamp()-INTERVAL '1 second' \
             WHERE community_id=$1 AND event_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(crashed.event_id().as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("simulate exporter crash after claim");
        let retried = fixture
            .db
            .claim_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                exporter,
                Duration::from_secs(30),
            )
            .await
            .expect("reclaim expired lease")
            .expect("retried first event");
        assert_eq!(retried.event_id(), crashed.event_id());
        assert_eq!(retried.attempt(), 2);
        assert_ne!(retried.delivery_attempt_id(), crashed.delivery_attempt_id());
        scenarios += 1;

        fixture
            .db
            .fail_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                retried.event_id(),
                retried.delivery_attempt_id(),
                DeliveryDisposition::Quarantine(ControlCode::PoisonEvent),
                RetryPolicy::new(Duration::from_millis(1), Duration::from_secs(1), 3)
                    .expect("retry policy"),
            )
            .await
            .expect("quarantine poison event");
        let after_poison = fixture
            .db
            .claim_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                exporter,
                Duration::from_secs(30),
            )
            .await
            .expect("claim after poison")
            .expect("later stream event is not blocked by quarantine");
        assert_eq!(after_poison.stream_position(), 2);
        fixture
            .db
            .acknowledge_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                after_poison.event_id(),
                after_poison.delivery_attempt_id(),
                after_poison.content_digest(),
            )
            .await
            .expect("acknowledge later event");
        scenarios += 1;

        let restoration_id = fixture
            .db
            .restore_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                retried.event_id(),
                retried.content_digest(),
                [91; 32],
                ControlCode::RestoreMismatch,
            )
            .await
            .expect("restore exact quarantined event");
        assert!(!restoration_id.is_nil());
        let restored = fixture
            .db
            .claim_authorization_delivery(
                domain,
                DeliveryKind::Decision,
                exporter,
                Duration::from_secs(30),
            )
            .await
            .expect("claim restored event")
            .expect("restored event lease");
        assert_eq!(restored.event_id(), retried.event_id());
        assert_eq!(restored.attempt(), 3);
        scenarios += 1;

        sqlx::query(
            "UPDATE authorization_evidence_capacity_state \
             SET general_remaining=0,allow_reserve=0 WHERE community_id=$1",
        )
        .bind(domain.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("exhaust new-allow capacity");
        let blocked_allow = event(
            domain,
            EventId::generate(),
            EventKind::AdmissionAllowed,
            EventResult::Allowed,
            DecisionReason::PolicyAllowed,
        );
        assert!(fixture
            .db
            .accept_authorization_decision(&blocked_allow, CapacityClass::NewAllow, ())
            .await
            .is_err());
        let blocked_row: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authorization_decision_events WHERE community_id=$1 AND event_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(blocked_allow.event_id().as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .expect("inspect failed allow append");
        assert_eq!(blocked_row, 0, "failed allow acceptance must be atomic");
        let preserved_deny = event(
            domain,
            EventId::generate(),
            EventKind::AdmissionDenied,
            EventResult::Denied,
            DecisionReason::PolicyDenied,
        );
        fixture
            .db
            .accept_authorization_decision(&preserved_deny, CapacityClass::RestrictiveReserve, ())
            .await
            .expect("restrictive reserve remains available");
        scenarios += 1;

        let tamper = sqlx::query(
            "UPDATE authorization_decision_events SET canonical_event=$3 \
             WHERE community_id=$1 AND event_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(first.event_id().as_uuid())
        .bind(b"tampered".as_slice())
        .execute(&fixture.pool)
        .await;
        assert!(
            tamper.is_err(),
            "immutable event tampering must be rejected"
        );
        let dead_letter = sqlx::query(
            "SELECT delivery_attempt_id,control_code FROM authorization_evidence_dead_letters \
             WHERE community_id=$1 AND decision_event_id=$2",
        )
        .bind(domain.as_uuid())
        .bind(first.event_id().as_uuid())
        .fetch_one(&fixture.pool)
        .await
        .expect("dead letter remains after restore");
        let dead_letter_attempt: Uuid = dead_letter.try_get("delivery_attempt_id").unwrap();
        assert_eq!(dead_letter_attempt, retried.delivery_attempt_id().as_uuid());
        scenarios += 1;

        assert_eq!(scenarios, 8, "all O5 evidence scenarios executed");
        fixture.cleanup().await;
    }
}
