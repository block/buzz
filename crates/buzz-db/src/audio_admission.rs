//! Transaction-owned audio admission for an existing channel member.

use buzz_core::CommunityId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

/// Commit an expiring audio admission inside the authorization transaction.
///
/// The channel and membership rows are locked and rechecked immediately before
/// insertion. This function never creates membership.
pub async fn admit_existing_audio_member_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    admission_id: Uuid,
    channel_id: Uuid,
    pubkey: &[u8; 32],
    claimant_id: Uuid,
    lease_expires_at: u64,
) -> Result<()> {
    if claimant_id.is_nil() {
        return Err(DbError::InvalidData(
            "audio attachment claimant must be non-nil".into(),
        ));
    }
    crate::channel::acquire_channel_membership_lock(transaction, community_id, channel_id).await?;
    let channel_active: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM channels \
         WHERE community_id = $1 AND id = $2 \
           AND archived_at IS NULL AND deleted_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if channel_active.is_none() {
        return Err(DbError::InvalidData("audio channel is unavailable".into()));
    }

    let membership_active: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
           AND removed_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    if membership_active.is_none() {
        return Err(DbError::InvalidData(
            "audio admission requires existing membership".into(),
        ));
    }

    let inserted = sqlx::query(
        "INSERT INTO audio_session_admissions \
         (community_id, admission_id, channel_id, pubkey, lease_expires_at, state, \
          claimant_id, claim_expires_at) \
         VALUES ($1, $2, $3, $4, to_timestamp($5::double precision), 'reserved', \
                 $6, to_timestamp($5::double precision)) \
         ON CONFLICT (community_id, admission_id) DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .bind(channel_id)
    .bind(pubkey.as_slice())
    .bind(lease_expires_at as f64)
    .bind(claimant_id)
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    if inserted == 0 {
        let exact: Option<bool> = sqlx::query_scalar(
            "SELECT channel_id = $3 AND pubkey = $4 AND claimant_id = $6 AND \
                    lease_expires_at = to_timestamp($5::double precision) AND \
                    state IN ('reserved', 'active', 'visible') \
             FROM audio_session_admissions \
             WHERE community_id = $1 AND admission_id = $2 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .bind(channel_id)
        .bind(pubkey.as_slice())
        .bind(lease_expires_at as f64)
        .bind(claimant_id)
        .fetch_optional(&mut **transaction)
        .await?;
        if exact != Some(true) {
            return Err(DbError::InvalidData(
                "audio admission retry conflicts with durable lifecycle".into(),
            ));
        }
    }
    Ok(())
}

/// Activate one reserved attempt inside the caller's authorization-owned
/// transaction immediately before any peer-visible effect.
pub async fn activate_audio_admission_tx(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    admission_id: Uuid,
    channel_id: Uuid,
    pubkey: &[u8; 32],
    claimant_id: Uuid,
    lease_expires_at: u64,
) -> Result<()> {
    if claimant_id.is_nil() {
        return Err(DbError::InvalidData(
            "audio attachment claimant must be non-nil".into(),
        ));
    }
    crate::channel::acquire_channel_membership_lock(transaction, community_id, channel_id).await?;
    let state: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT state, claimant_id FROM audio_session_admissions \
         WHERE community_id = $1 AND admission_id = $2 \
           AND channel_id = $3 AND pubkey = $4 \
           AND lease_expires_at = to_timestamp($5::double precision) \
         FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .bind(channel_id)
    .bind(pubkey.as_slice())
    .bind(lease_expires_at as f64)
    .fetch_optional(&mut **transaction)
    .await?;
    match state {
        Some((state, Some(existing))) if state == "reserved" && existing == claimant_id => {
            let changed = sqlx::query(
                "UPDATE audio_session_admissions \
                 SET state='active', state_version=state_version+1, \
                     activated_at=COALESCE(activated_at, clock_timestamp()), \
                     claimant_id=$3, attachment_generation=1, \
                     claim_expires_at=lease_expires_at, \
                     updated_at=clock_timestamp(), failure_code=NULL \
                 WHERE community_id=$1 AND admission_id=$2 \
                   AND state='reserved' \
                   AND lease_expires_at > clock_timestamp() \
                   AND EXISTS ( \
                     SELECT 1 FROM channel_members cm \
                     WHERE cm.community_id=audio_session_admissions.community_id \
                       AND cm.channel_id=audio_session_admissions.channel_id \
                       AND cm.pubkey=audio_session_admissions.pubkey \
                       AND cm.removed_at IS NULL) \
                   AND EXISTS ( \
                     SELECT 1 FROM channels c \
                     WHERE c.community_id=audio_session_admissions.community_id \
                       AND c.id=audio_session_admissions.channel_id \
                       AND c.archived_at IS NULL AND c.deleted_at IS NULL)",
            )
            .bind(community_id.as_uuid())
            .bind(admission_id)
            .bind(claimant_id)
            .execute(&mut **transaction)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(DbError::InvalidData(
                    "audio admission expired or lost authority before activation".into(),
                ));
            }
        }
        Some((state, Some(existing)))
            if matches!(state.as_str(), "active" | "visible") && existing == claimant_id => {}
        _ => {
            return Err(DbError::InvalidData(
                "audio admission is not activatable".into(),
            ))
        }
    }
    Ok(())
}

/// Whether an already-replayed activation still represents this exact live
/// attempt. Terminal receipts cannot be reused to create a second attachment.
pub async fn audio_admission_is_active(
    db: &crate::Db,
    community_id: CommunityId,
    admission_id: Uuid,
    channel_id: Uuid,
    pubkey: &[u8; 32],
    claimant_id: Uuid,
) -> Result<bool> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM audio_session_admissions \
         WHERE community_id=$1 AND admission_id=$2 AND channel_id=$3 \
           AND pubkey=$4 AND claimant_id=$5 AND attachment_generation=1 \
           AND state IN ('active','visible') AND lease_expires_at > clock_timestamp() \
           AND claim_expires_at > clock_timestamp() \
           AND EXISTS (SELECT 1 FROM channel_members cm \
             WHERE cm.community_id=audio_session_admissions.community_id \
               AND cm.channel_id=audio_session_admissions.channel_id \
               AND cm.pubkey=audio_session_admissions.pubkey \
               AND cm.removed_at IS NULL))",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .bind(channel_id)
    .bind(pubkey.as_slice())
    .bind(claimant_id)
    .fetch_one(&db.pool)
    .await?;
    Ok(active)
}

/// Durably record that one exact authorized attempt became peer-visible.
///
/// This is evidence that visibility occurred once, not evidence of current
/// live presence. The operation receipt and state transition commit together
/// so independent restore protection can recover an interrupted witness.
#[allow(clippy::too_many_arguments)]
pub async fn mark_audio_admission_visible_with_receipt(
    db: &crate::Db,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<()> {
    if claimant_id.is_nil() || operation_id.is_nil() {
        return Err(DbError::InvalidData(
            "audio visibility identity must be non-nil".into(),
        ));
    }
    let mut tx = db.pool.begin().await?;
    let existing: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT request_fingerprint FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.as_slice() != request_fingerprint {
            return Err(DbError::InvalidData(
                "audio visibility operation ID conflicts with prior request".into(),
            ));
        }
        tx.commit().await?;
        return Ok(());
    }

    let current: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT state, claimant_id FROM audio_session_admissions \
         WHERE community_id=$1 AND admission_id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .fetch_optional(&mut *tx)
    .await?;
    match current {
        Some((state, existing_claimant))
            if state == "active" && existing_claimant == Some(claimant_id) =>
        {
            let changed = sqlx::query(
                "UPDATE audio_session_admissions SET \
                   state='visible', state_version=state_version+1, \
                   visibility_observed_at=COALESCE(visibility_observed_at, clock_timestamp()), \
                   updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND admission_id=$2 AND state='active' \
                   AND claimant_id=$3 AND lease_expires_at > clock_timestamp() \
                   AND claim_expires_at > clock_timestamp() \
                   AND EXISTS (SELECT 1 FROM channel_members cm \
                     WHERE cm.community_id=audio_session_admissions.community_id \
                       AND cm.channel_id=audio_session_admissions.channel_id \
                       AND cm.pubkey=audio_session_admissions.pubkey \
                       AND cm.removed_at IS NULL) \
                   AND EXISTS (SELECT 1 FROM channels c \
                     WHERE c.community_id=audio_session_admissions.community_id \
                       AND c.id=audio_session_admissions.channel_id \
                       AND c.archived_at IS NULL AND c.deleted_at IS NULL)",
            )
            .bind(community_id.as_uuid())
            .bind(admission_id)
            .bind(claimant_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(DbError::InvalidData(
                    "audio admission expired or lost authority before visibility".into(),
                ));
            }
        }
        Some((state, existing_claimant))
            if state == "visible" && existing_claimant == Some(claimant_id) => {}
        _ => {
            return Err(DbError::InvalidData(
                "audio admission is not visibility-confirmable".into(),
            ))
        }
    }
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, operation_kind, request_fingerprint, \
          result_payload, lease_expires_at) \
         VALUES ($1,$2,'audio.admission.visible.v1',$3,$4, \
                 clock_timestamp()+interval '100 years')",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(b"visible".as_slice())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Durably record that an exact live attempt is being detached.
///
/// The owning caller independently witnesses this transaction before its
/// direct compensation. Other replicas still respect the claimant's durable
/// deadline and cleanup grace before orphan takeover.
pub async fn request_audio_admission_cleanup_with_receipt(
    db: &crate::Db,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<()> {
    if claimant_id.is_nil() || operation_id.is_nil() {
        return Err(DbError::InvalidData(
            "audio cleanup identity must be non-nil".into(),
        ));
    }
    let mut tx = db.pool.begin().await?;
    let existing: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT request_fingerprint FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.as_slice() != request_fingerprint {
            return Err(DbError::InvalidData(
                "audio cleanup operation ID conflicts with prior request".into(),
            ));
        }
        tx.commit().await?;
        return Ok(());
    }

    let current: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT state, claimant_id FROM audio_session_admissions \
         WHERE community_id=$1 AND admission_id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .fetch_optional(&mut *tx)
    .await?;
    match current {
        Some((state, existing_claimant))
            if existing_claimant == Some(claimant_id)
                && matches!(state.as_str(), "reserved" | "active" | "visible") =>
        {
            sqlx::query(
                "UPDATE audio_session_admissions SET \
                   cleanup_requested_at=COALESCE(cleanup_requested_at, clock_timestamp()), \
                   state_version=state_version+1, updated_at=clock_timestamp() \
                 WHERE community_id=$1 AND admission_id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(admission_id)
            .execute(&mut *tx)
            .await?;
        }
        Some((state, existing_claimant))
            if existing_claimant == Some(claimant_id)
                && matches!(state.as_str(), "aborted" | "finished") => {}
        _ => {
            return Err(DbError::InvalidData(
                "audio cleanup request conflicts with durable lifecycle".into(),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, operation_kind, request_fingerprint, \
          result_payload, lease_expires_at) \
         VALUES ($1,$2,'audio.admission.cleanup-request.v1',$3,$4, \
                 clock_timestamp()+interval '100 years')",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .bind(request_fingerprint.as_slice())
    .bind(b"cleanup_requested".as_slice())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Atomically finish or compensate an exact attachment and retain the restore
/// operation receipt needed to recover a crash between PostgreSQL and the
/// independent witness commit.
#[allow(clippy::too_many_arguments)]
pub async fn complete_claimed_audio_admission_with_receipt(
    db: &crate::Db,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    finished: bool,
    failure_code: Option<&str>,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<()> {
    complete_claimed_audio_admission_inner(
        db,
        community_id,
        admission_id,
        claimant_id,
        finished,
        failure_code,
        operation_id,
        request_fingerprint,
        None,
    )
    .await
    .map(|_| ())
}

/// Reconcile only the exact crash-remnant version discovered by the caller.
///
/// `Ok(false)` means another transaction advanced the lifecycle first. The
/// caller must abort its pending restore witness and must not downgrade the
/// newer state.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile_claimed_audio_admission_with_receipt(
    db: &crate::Db,
    community_id: CommunityId,
    candidate: AudioAdmissionReconciliationCandidate,
    finished: bool,
    failure_code: Option<&str>,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
) -> Result<bool> {
    complete_claimed_audio_admission_inner(
        db,
        community_id,
        candidate.admission_id,
        candidate.claimant_id,
        finished,
        failure_code,
        operation_id,
        request_fingerprint,
        Some((candidate.source_state.as_str(), candidate.state_version)),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn complete_claimed_audio_admission_inner(
    db: &crate::Db,
    community_id: CommunityId,
    admission_id: Uuid,
    claimant_id: Uuid,
    finished: bool,
    failure_code: Option<&str>,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    expected_source: Option<(&str, i64)>,
) -> Result<bool> {
    if claimant_id.is_nil() || operation_id.is_nil() {
        return Err(DbError::InvalidData(
            "audio completion identity must be non-nil".into(),
        ));
    }
    if finished {
        if failure_code.is_some() {
            return Err(DbError::InvalidData(
                "finished audio completion cannot carry a failure code".into(),
            ));
        }
    } else {
        validate_failure_code(failure_code.ok_or_else(|| {
            DbError::InvalidData("aborted audio completion requires a failure code".into())
        })?)?;
    }
    let mut tx = db.pool.begin().await?;
    let target = if finished { "finished" } else { "aborted" };
    let current: Option<(String, Option<Uuid>, i64)> = sqlx::query_as(
        "SELECT state, claimant_id, state_version FROM audio_session_admissions \
         WHERE community_id=$1 AND admission_id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(admission_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let (Some((expected_state, expected_version)), Some((state, _, state_version))) =
        (expected_source, current.as_ref())
    {
        if state != target && (state != expected_state || *state_version != expected_version) {
            tx.rollback().await?;
            return Ok(false);
        }
    }
    match current {
        Some((state, existing_claimant, _)) if state == target => {
            if existing_claimant.is_some() && existing_claimant != Some(claimant_id) {
                return Err(DbError::InvalidData(
                    "audio completion claimant conflicts with durable state".into(),
                ));
            }
        }
        Some((state, existing_claimant, _))
            if ((!finished && matches!(state.as_str(), "reserved" | "active" | "visible"))
                || (finished && state == "visible"))
                && existing_claimant == Some(claimant_id) =>
        {
            let updated = sqlx::query(
                "UPDATE audio_session_admissions \
                 SET state=$3, state_version=state_version+1, \
                     finished_at=CASE WHEN $3='finished' THEN \
                       COALESCE(finished_at, clock_timestamp()) ELSE finished_at END, \
                     aborted_at=CASE WHEN $3='aborted' THEN \
                       COALESCE(aborted_at, clock_timestamp()) ELSE aborted_at END, \
                     updated_at=clock_timestamp(), failure_code=$4 \
                 WHERE community_id=$1 AND admission_id=$2 \
                   AND ($3 <> 'finished' OR \
                        (lease_expires_at > clock_timestamp() \
                         AND claim_expires_at > clock_timestamp()))",
            )
            .bind(community_id.as_uuid())
            .bind(admission_id)
            .bind(target)
            .bind(failure_code)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(DbError::InvalidData(
                    "expired audio admission cannot become finished".into(),
                ));
            }
        }
        _ => {
            return Err(DbError::InvalidData(
                "audio completion is stale or conflicts with durable state".into(),
            ))
        }
    }
    let existing: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT request_fingerprint FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.as_slice() != request_fingerprint {
            return Err(DbError::InvalidData(
                "audio completion operation ID conflicts with prior request".into(),
            ));
        }
    } else {
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, operation_kind, request_fingerprint, \
              result_payload, lease_expires_at) \
             VALUES ($1, $2, 'audio.admission.complete.v1', $3, $4, \
                     clock_timestamp() + INTERVAL '100 years')",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .bind(request_fingerprint.as_slice())
        .bind(target.as_bytes())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Nonterminal durable state observed by reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioAdmissionReconciliationState {
    /// Membership was authorized, but attachment activation did not commit.
    Reserved,
    /// Activation committed, but peer visibility was not durably observed.
    Active,
    /// Peer visibility was durably observed at least once.
    Visible,
}

impl AudioAdmissionReconciliationState {
    /// Stable database representation used by exact compare-and-set cleanup.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Active => "active",
            Self::Visible => "visible",
        }
    }

    /// Only a durably observed attachment may be recorded as finished.
    pub const fn visibility_was_observed(self) -> bool {
        matches!(self, Self::Visible)
    }

    /// Stable terminal failure for a conservatively aborted attempt.
    pub const fn abort_failure_code(self) -> Option<&'static str> {
        match self {
            Self::Reserved => Some("stale_reservation"),
            Self::Active => Some("unobserved_attachment"),
            Self::Visible => None,
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "reserved" => Ok(Self::Reserved),
            "active" => Ok(Self::Active),
            "visible" => Ok(Self::Visible),
            _ => Err(DbError::InvalidData(
                "audio reconciliation state is invalid".into(),
            )),
        }
    }
}

/// One crash remnant that requires a restore-witnessed terminal transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioAdmissionReconciliationCandidate {
    /// Stable attempt identifier.
    pub admission_id: Uuid,
    /// Exact process attachment claimant recorded at reservation.
    pub claimant_id: Uuid,
    /// Exact nonterminal state observed by discovery.
    pub source_state: AudioAdmissionReconciliationState,
    /// Exact lifecycle version observed with `source_state`.
    pub state_version: i64,
}

/// Discover crash remnants without mutating authority state. The relay owns
/// each terminal transition so it can witness the PostgreSQL commit in the
/// independent restore-protection store.
pub async fn reconcilable_audio_admissions(
    db: &crate::Db,
    community_id: CommunityId,
) -> Result<Vec<AudioAdmissionReconciliationCandidate>> {
    audio_admissions_requiring_reconciliation_after(db, community_id, None).await
}

/// Discover orphaned nonterminal attempts during startup.
///
/// An unexpired claim may belong to another healthy replica and is never a
/// takeover candidate. The durable claim deadline plus cleanup grace is the
/// bounded liveness proof required before another replica may compensate it.
pub async fn unfinished_audio_admissions(
    db: &crate::Db,
    community_id: CommunityId,
) -> Result<Vec<AudioAdmissionReconciliationCandidate>> {
    unfinished_audio_admissions_after(db, community_id, None).await
}

/// Read one deterministic reconciliation page strictly after an admission ID.
/// Cursor pagination prevents one persistent poisoned prefix from starving
/// later cleanup candidates.
pub async fn reconcilable_audio_admissions_after(
    db: &crate::Db,
    community_id: CommunityId,
    after_admission_id: Option<Uuid>,
) -> Result<Vec<AudioAdmissionReconciliationCandidate>> {
    audio_admissions_requiring_reconciliation_after(db, community_id, after_admission_id).await
}

/// Read one startup-reconciliation page after an admission ID.
pub async fn unfinished_audio_admissions_after(
    db: &crate::Db,
    community_id: CommunityId,
    after_admission_id: Option<Uuid>,
) -> Result<Vec<AudioAdmissionReconciliationCandidate>> {
    audio_admissions_requiring_reconciliation_after(db, community_id, after_admission_id).await
}

async fn audio_admissions_requiring_reconciliation_after(
    db: &crate::Db,
    community_id: CommunityId,
    after_admission_id: Option<Uuid>,
) -> Result<Vec<AudioAdmissionReconciliationCandidate>> {
    let rows: Vec<(Uuid, Uuid, String, i64)> = sqlx::query_as(
        "SELECT admission_id, claimant_id, state, state_version \
         FROM audio_session_admissions \
         WHERE community_id=$1 \
           AND ($2::uuid IS NULL OR admission_id > $2) \
           AND claimant_id IS NOT NULL \
           AND state IN ('reserved','active','visible') \
           AND claim_expires_at <= clock_timestamp() - interval '30 seconds' \
         ORDER BY admission_id LIMIT 256",
    )
    .bind(community_id.as_uuid())
    .bind(after_admission_id)
    .fetch_all(&db.pool)
    .await?;
    rows.into_iter()
        .map(|(admission_id, claimant_id, state, state_version)| {
            Ok(AudioAdmissionReconciliationCandidate {
                admission_id,
                claimant_id,
                source_state: AudioAdmissionReconciliationState::parse(&state)?,
                state_version,
            })
        })
        .collect()
}

fn validate_failure_code(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(DbError::InvalidData(
            "audio admission failure code is invalid".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelType, ChannelVisibility};

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup() -> (crate::Db, CommunityId, Uuid, [u8; 32]) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("test migrations");
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id.as_uuid())
            .bind(format!(
                "audio-admission-{}.example",
                Uuid::new_v4().simple()
            ))
            .execute(&pool)
            .await
            .expect("test community");
        let member = [4_u8; 32];
        let channel_id = Uuid::new_v4();
        crate::channel::create_channel_with_id(
            &pool,
            community_id,
            channel_id,
            "Audio admission",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            &member,
            None,
        )
        .await
        .expect("test channel");
        (crate::Db::from_pool(pool), community_id, channel_id, member)
    }

    fn epoch_after(seconds: u64) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_secs()
            + seconds
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn admission_requires_existing_member_and_never_creates_membership() {
        let (db, community_id, channel_id, member) = setup().await;
        let mut admitted = db.begin_transaction().await.expect("transaction");
        admit_existing_audio_member_tx(
            &mut admitted,
            community_id,
            Uuid::new_v4(),
            channel_id,
            &member,
            Uuid::from_u128(0x1000),
            epoch_after(60),
        )
        .await
        .expect("existing member admission");
        admitted.commit().await.expect("commit admission");

        let outsider = [8_u8; 32];
        let mut denied = db.begin_transaction().await.expect("transaction");
        assert!(admit_existing_audio_member_tx(
            &mut denied,
            community_id,
            Uuid::new_v4(),
            channel_id,
            &outsider,
            Uuid::from_u128(0x1001),
            epoch_after(60),
        )
        .await
        .is_err());
        denied.rollback().await.expect("rollback denial");

        let membership_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
               AND removed_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(outsider.as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("membership count");
        assert_eq!(membership_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn lifecycle_is_idempotent_and_terminal() {
        let (db, community_id, channel_id, member) = setup().await;
        let admission_id = Uuid::new_v4();
        let lease_expires_at = epoch_after(60);
        let mut transaction = db.begin_transaction().await.expect("transaction");
        admit_existing_audio_member_tx(
            &mut transaction,
            community_id,
            admission_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1001),
            lease_expires_at,
        )
        .await
        .expect("reserve");
        transaction.commit().await.expect("commit reserve");

        let mut activation = db.begin_transaction().await.expect("activation tx");
        activate_audio_admission_tx(
            &mut activation,
            community_id,
            admission_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1001),
            lease_expires_at,
        )
        .await
        .expect("activate");
        activation.commit().await.expect("commit activation");
        let cleanup_id = Uuid::new_v4();
        request_audio_admission_cleanup_with_receipt(
            &db,
            community_id,
            admission_id,
            Uuid::from_u128(0x1001),
            cleanup_id,
            [5_u8; 32],
        )
        .await
        .expect("request cleanup");
        request_audio_admission_cleanup_with_receipt(
            &db,
            community_id,
            admission_id,
            Uuid::from_u128(0x1001),
            cleanup_id,
            [5_u8; 32],
        )
        .await
        .expect("cleanup request replay");
        assert!(reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("healthy claimant remains exclusive")
            .is_empty());
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .expect("expire durable claimant");
        assert_eq!(
            reconcilable_audio_admissions(&db, community_id)
                .await
                .expect("expired claimant is discoverable"),
            vec![AudioAdmissionReconciliationCandidate {
                admission_id,
                claimant_id: Uuid::from_u128(0x1001),
                source_state: AudioAdmissionReconciliationState::Active,
                state_version: 3,
            }]
        );
        let completion_id = Uuid::new_v4();
        complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            Uuid::from_u128(0x1001),
            false,
            Some("cancelled"),
            completion_id,
            [6_u8; 32],
        )
        .await
        .expect("compensate");
        complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            Uuid::from_u128(0x1001),
            false,
            Some("cancelled"),
            completion_id,
            [6_u8; 32],
        )
        .await
        .expect("idempotent compensate");
        let mut terminal = db.begin_transaction().await.expect("terminal tx");
        assert!(activate_audio_admission_tx(
            &mut terminal,
            community_id,
            admission_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1001),
            lease_expires_at,
        )
        .await
        .is_err());
        terminal.rollback().await.expect("rollback terminal check");

        let state: String = sqlx::query_scalar(
            "SELECT state FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .fetch_one(&db.pool)
        .await
        .expect("state");
        assert_eq!(state, "aborted");
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn exact_claimant_and_completion_receipt_prevent_replay() {
        let (db, community_id, channel_id, member) = setup().await;
        let admission_id = Uuid::new_v4();
        let claimant = Uuid::new_v4();
        let other_claimant = Uuid::new_v4();
        let lease_expires_at = epoch_after(60);

        let mut reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut reserve,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("reserve exact claimant");
        reserve.commit().await.expect("commit reserve");

        let mut conflicting_reserve = db.begin_transaction().await.expect("conflicting reserve");
        assert!(admit_existing_audio_member_tx(
            &mut conflicting_reserve,
            community_id,
            admission_id,
            channel_id,
            &member,
            other_claimant,
            lease_expires_at,
        )
        .await
        .is_err());
        conflicting_reserve
            .rollback()
            .await
            .expect("rollback claimant conflict");

        let mut activate = db.begin_transaction().await.expect("activate tx");
        activate_audio_admission_tx(
            &mut activate,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("activate exact claimant");
        activate.commit().await.expect("commit activation");
        assert!(audio_admission_is_active(
            &db,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
        )
        .await
        .expect("exact claimant state"));
        assert!(!audio_admission_is_active(
            &db,
            community_id,
            admission_id,
            channel_id,
            &member,
            other_claimant,
        )
        .await
        .expect("other claimant state"));
        assert!(unfinished_audio_admissions(&db, community_id)
            .await
            .expect("startup preserves a healthy active claimant")
            .into_iter()
            .all(|candidate| candidate.admission_id != admission_id));

        assert!(complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            true,
            None,
            Uuid::new_v4(),
            [7_u8; 32],
        )
        .await
        .is_err());
        assert!(sqlx::query(
            "UPDATE audio_session_admissions SET state='finished' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .is_err());

        let visibility_operation = Uuid::new_v4();
        let visibility_request = [8_u8; 32];
        mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            visibility_operation,
            visibility_request,
        )
        .await
        .expect("mark exact attachment visible");
        mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            visibility_operation,
            visibility_request,
        )
        .await
        .expect("visibility receipt replay");
        assert!(mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            visibility_operation,
            [11_u8; 32],
        )
        .await
        .is_err());
        assert!(mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            admission_id,
            other_claimant,
            Uuid::new_v4(),
            [12_u8; 32],
        )
        .await
        .is_err());
        assert!(unfinished_audio_admissions(&db, community_id)
            .await
            .expect("startup preserves a healthy visible claimant")
            .into_iter()
            .all(|candidate| candidate.admission_id != admission_id));

        let operation_id = Uuid::new_v4();
        let request = [9_u8; 32];
        complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            true,
            None,
            operation_id,
            request,
        )
        .await
        .expect("finish with receipt");
        complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            true,
            None,
            operation_id,
            request,
        )
        .await
        .expect("idempotent receipt retry");
        assert!(complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            true,
            None,
            operation_id,
            [10_u8; 32],
        )
        .await
        .is_err());
        assert!(!audio_admission_is_active(
            &db,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
        )
        .await
        .expect("terminal attempt cannot reappear"));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn orphaned_active_and_visible_admissions_both_abort() {
        let (db, community_id, channel_id, member) = setup().await;
        let active_id = Uuid::new_v4();
        let visible_id = Uuid::new_v4();
        let claimant = Uuid::new_v4();
        let lease_expires_at = epoch_after(300);
        for admission_id in [active_id, visible_id] {
            let mut reserve = db.begin_transaction().await.expect("reserve tx");
            admit_existing_audio_member_tx(
                &mut reserve,
                community_id,
                admission_id,
                channel_id,
                &member,
                claimant,
                lease_expires_at,
            )
            .await
            .expect("reserve attempt");
            reserve.commit().await.expect("commit reserve");
            let mut activate = db.begin_transaction().await.expect("activate tx");
            activate_audio_admission_tx(
                &mut activate,
                community_id,
                admission_id,
                channel_id,
                &member,
                claimant,
                lease_expires_at,
            )
            .await
            .expect("activate attempt");
            activate.commit().await.expect("commit activation");
        }
        mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            visible_id,
            claimant,
            Uuid::new_v4(),
            [21_u8; 32],
        )
        .await
        .expect("record observed visibility");
        for admission_id in [active_id, visible_id] {
            request_audio_admission_cleanup_with_receipt(
                &db,
                community_id,
                admission_id,
                claimant,
                Uuid::new_v4(),
                [22_u8; 32],
            )
            .await
            .expect("request cleanup");
        }
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id IN ($2,$3)",
        )
        .bind(community_id.as_uuid())
        .bind(active_id)
        .bind(visible_id)
        .execute(&db.pool)
        .await
        .expect("expire both durable claimants");

        let candidates = reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("discover both crash states");
        assert!(candidates.iter().any(|candidate| {
            candidate.admission_id == active_id
                && candidate.source_state == AudioAdmissionReconciliationState::Active
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.admission_id == visible_id
                && candidate.source_state == AudioAdmissionReconciliationState::Visible
        }));
        for candidate in candidates {
            assert!(reconcile_claimed_audio_admission_with_receipt(
                &db,
                community_id,
                candidate,
                false,
                Some("orphaned_attachment"),
                Uuid::new_v4(),
                [23_u8; 32],
            )
            .await
            .expect("reconcile exact state"));
        }
        let states: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT admission_id, state FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id IN ($2,$3)",
        )
        .bind(community_id.as_uuid())
        .bind(active_id)
        .bind(visible_id)
        .fetch_all(&db.pool)
        .await
        .expect("terminal states");
        assert!(states.contains(&(active_id, "aborted".to_owned())));
        assert!(states.contains(&(visible_id, "aborted".to_owned())));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn expired_visible_admission_cannot_finish_and_can_be_compensated() {
        let (db, community_id, channel_id, member) = setup().await;
        let admission_id = Uuid::new_v4();
        let claimant = Uuid::new_v4();
        let lease_expires_at = epoch_after(300);
        let mut reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut reserve,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("reserve attempt");
        reserve.commit().await.expect("commit reserve");
        let mut activate = db.begin_transaction().await.expect("activate tx");
        activate_audio_admission_tx(
            &mut activate,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("activate attempt");
        activate.commit().await.expect("commit activation");
        mark_audio_admission_visible_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            Uuid::new_v4(),
            [0xa1; 32],
        )
        .await
        .expect("record visible attachment");
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .expect("expire admission and durable owner claim");

        let finish_operation = Uuid::new_v4();
        assert!(complete_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            admission_id,
            claimant,
            true,
            None,
            finish_operation,
            [0xa2; 32],
        )
        .await
        .is_err());
        assert!(db
            .authorization_operation_receipt_fingerprint(community_id, finish_operation)
            .await
            .expect("query rejected finish receipt")
            .is_none());

        let candidate = reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("expired visible admission is reconcilable")
            .into_iter()
            .find(|candidate| candidate.admission_id == admission_id)
            .expect("visible orphan candidate");
        assert_eq!(
            candidate.source_state,
            AudioAdmissionReconciliationState::Visible
        );
        assert!(reconcile_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            candidate,
            false,
            Some("orphaned_attachment"),
            Uuid::new_v4(),
            [0xa3; 32],
        )
        .await
        .expect("compensate expired attachment"));
        let state: String = sqlx::query_scalar(
            "SELECT state FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .fetch_one(&db.pool)
        .await
        .expect("terminal state");
        assert_eq!(state, "aborted");
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn stale_reserved_reconciliation_cannot_abort_newly_active_attempt() {
        let (db, community_id, channel_id, member) = setup().await;
        let admission_id = Uuid::new_v4();
        let claimant = Uuid::new_v4();
        let lease_expires_at = epoch_after(300);
        let mut reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut reserve,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("reserve attempt");
        reserve.commit().await.expect("commit reserve");
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET updated_at=clock_timestamp()-interval '3 minutes' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .expect("age reservation");
        assert!(reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("age is not durable takeover evidence")
            .is_empty());
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .expect("expire claimant after healthy-owner assertion");
        let candidate = reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("discover stale reservation")
            .into_iter()
            .find(|candidate| candidate.admission_id == admission_id)
            .expect("candidate");
        assert_eq!(
            candidate.source_state,
            AudioAdmissionReconciliationState::Reserved
        );

        let mut activate = db.begin_transaction().await.expect("activation tx");
        activate_audio_admission_tx(
            &mut activate,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            lease_expires_at,
        )
        .await
        .expect("activate after discovery");
        activate.commit().await.expect("commit activation");

        let changed = reconcile_claimed_audio_admission_with_receipt(
            &db,
            community_id,
            candidate,
            false,
            Some("stale_reservation"),
            Uuid::new_v4(),
            [0x81; 32],
        )
        .await
        .expect("stale reconciliation loses CAS without error");
        assert!(!changed);
        let state: (String, i64) = sqlx::query_as(
            "SELECT state, state_version FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .fetch_one(&db.pool)
        .await
        .expect("durable admission");
        assert_eq!(state, ("active".to_owned(), candidate.state_version + 1));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn expired_claim_has_one_idempotent_multi_replica_takeover() {
        let (db, community_id, channel_id, member) = setup().await;
        let admission_id = Uuid::new_v4();
        let claimant = Uuid::new_v4();
        let mut reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut reserve,
            community_id,
            admission_id,
            channel_id,
            &member,
            claimant,
            epoch_after(300),
        )
        .await
        .expect("reserve attempt");
        reserve.commit().await.expect("commit reserve");
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .execute(&db.pool)
        .await
        .expect("expire durable claim");
        let candidate = reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("discover orphan")
            .into_iter()
            .find(|candidate| candidate.admission_id == admission_id)
            .expect("orphan candidate");
        let operation_id = Uuid::new_v4();
        let fingerprint = [0x91; 32];
        let db_b = db.clone();
        let (replica_a, replica_b) = tokio::join!(
            reconcile_claimed_audio_admission_with_receipt(
                &db,
                community_id,
                candidate,
                false,
                Some("orphaned_attachment"),
                operation_id,
                fingerprint,
            ),
            reconcile_claimed_audio_admission_with_receipt(
                &db_b,
                community_id,
                candidate,
                false,
                Some("orphaned_attachment"),
                operation_id,
                fingerprint,
            ),
        );
        assert!(replica_a.expect("replica A converges"));
        assert!(replica_b.expect("replica B converges"));
        let lifecycle: (String, i64) = sqlx::query_as(
            "SELECT state,state_version FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(admission_id)
        .fetch_one(&db.pool)
        .await
        .expect("terminal lifecycle");
        assert_eq!(lifecycle, ("aborted".to_owned(), 2));
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM authorization_operation_receipts \
             WHERE community_id=$1 AND operation_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .fetch_one(&db.pool)
        .await
        .expect("single takeover receipt");
        assert_eq!(receipts, 1);
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn activation_rechecks_expiry_and_membership() {
        let (db, community_id, channel_id, member) = setup().await;

        let expired_id = Uuid::new_v4();
        let future_expiry = epoch_after(60);
        let mut expired_reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut expired_reserve,
            community_id,
            expired_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1002),
            future_expiry,
        )
        .await
        .expect("reserve expiring attempt");
        expired_reserve.commit().await.expect("commit reserve");
        let expired_at = epoch_after(0).saturating_sub(1);
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET admitted_at=to_timestamp($3::double precision)-interval '1 second', \
                 lease_expires_at=to_timestamp($3::double precision) \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(expired_id)
        .bind(expired_at as f64)
        .execute(&db.pool)
        .await
        .expect("expire attempt deterministically");
        let mut expired_activation = db.begin_transaction().await.expect("activation tx");
        assert!(activate_audio_admission_tx(
            &mut expired_activation,
            community_id,
            expired_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1002),
            expired_at,
        )
        .await
        .is_err());
        expired_activation
            .rollback()
            .await
            .expect("rollback expired activation");

        let revoked_id = Uuid::new_v4();
        let revoked_expiry = epoch_after(60);
        let mut revoked_reserve = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut revoked_reserve,
            community_id,
            revoked_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1003),
            revoked_expiry,
        )
        .await
        .expect("reserve membership attempt");
        revoked_reserve.commit().await.expect("commit reserve");
        sqlx::query(
            "UPDATE channel_members SET removed_at=clock_timestamp() \
             WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(member.as_slice())
        .execute(&db.pool)
        .await
        .expect("remove membership");
        let mut revoked_activation = db.begin_transaction().await.expect("activation tx");
        assert!(activate_audio_admission_tx(
            &mut revoked_activation,
            community_id,
            revoked_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1003),
            revoked_expiry,
        )
        .await
        .is_err());
        revoked_activation
            .rollback()
            .await
            .expect("rollback revoked activation");
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn rollback_and_reconciliation_are_fail_closed() {
        let (db, community_id, channel_id, member) = setup().await;
        let rolled_back_id = Uuid::new_v4();
        let mut rolled_back = db.begin_transaction().await.expect("reserve tx");
        admit_existing_audio_member_tx(
            &mut rolled_back,
            community_id,
            rolled_back_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1004),
            epoch_after(60),
        )
        .await
        .expect("reserve before rollback");
        rolled_back.rollback().await.expect("rollback reserve");
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(rolled_back_id)
        .fetch_one(&db.pool)
        .await
        .expect("rolled-back count");
        assert_eq!(count, 0);

        let stale_id = Uuid::new_v4();
        let active_id = Uuid::new_v4();
        let lease_expires_at = epoch_after(300);
        for (admission_id, claimant_id) in [
            (stale_id, Uuid::from_u128(0x1005)),
            (active_id, Uuid::from_u128(0x1006)),
        ] {
            let mut transaction = db.begin_transaction().await.expect("reserve tx");
            admit_existing_audio_member_tx(
                &mut transaction,
                community_id,
                admission_id,
                channel_id,
                &member,
                claimant_id,
                lease_expires_at,
            )
            .await
            .expect("reserve attempt");
            transaction.commit().await.expect("commit reserve");
        }
        sqlx::query(
            "UPDATE audio_session_admissions \
             SET updated_at=clock_timestamp()-interval '3 minutes', \
                 claim_expires_at=clock_timestamp()-interval '31 seconds' \
             WHERE community_id=$1 AND admission_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(stale_id)
        .execute(&db.pool)
        .await
        .expect("age reserved attempt");
        let mut activation = db.begin_transaction().await.expect("activation tx");
        activate_audio_admission_tx(
            &mut activation,
            community_id,
            active_id,
            channel_id,
            &member,
            Uuid::from_u128(0x1006),
            lease_expires_at,
        )
        .await
        .expect("activate live attempt");
        activation.commit().await.expect("commit activation");

        let candidates = reconcilable_audio_admissions(&db, community_id)
            .await
            .expect("discover reconciliation candidates");
        assert!(candidates.iter().any(|candidate| {
            candidate.admission_id == stale_id
                && candidate.claimant_id == Uuid::from_u128(0x1005)
                && candidate.source_state == AudioAdmissionReconciliationState::Reserved
        }));
        for candidate in candidates {
            let operation_id = Uuid::new_v4();
            let finished = candidate.source_state.visibility_was_observed();
            complete_claimed_audio_admission_with_receipt(
                &db,
                community_id,
                candidate.admission_id,
                candidate.claimant_id,
                finished,
                candidate.source_state.abort_failure_code(),
                operation_id,
                [7_u8; 32],
            )
            .await
            .expect("reconcile exact candidate");
        }
        let states: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT admission_id, state FROM audio_session_admissions \
             WHERE community_id=$1 AND admission_id IN ($2, $3) \
             ORDER BY admission_id",
        )
        .bind(community_id.as_uuid())
        .bind(stale_id)
        .bind(active_id)
        .fetch_all(&db.pool)
        .await
        .expect("lifecycle states");
        assert!(states.contains(&(stale_id, "aborted".to_owned())));
        assert!(states.contains(&(active_id, "active".to_owned())));
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn reconciliation_cursor_covers_more_than_one_bounded_page() {
        let (db, community_id, channel_id, member) = setup().await;
        sqlx::query(
            "INSERT INTO audio_session_admissions \
             (community_id,admission_id,channel_id,pubkey,lease_expires_at,admitted_at, \
              state,state_version,updated_at,claimant_id,attachment_generation,claim_expires_at) \
             SELECT $1,gen_random_uuid(),$2,$3, \
                    clock_timestamp()-interval '1 minute', \
                    clock_timestamp()-interval '2 minutes', \
                    'reserved',1,clock_timestamp()-interval '2 minutes', \
                    gen_random_uuid(),0,clock_timestamp()-interval '1 minute' \
             FROM generate_series(1,300)",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(member.as_slice())
        .execute(&db.pool)
        .await
        .expect("populate more than one reconciliation page");

        let first = reconcilable_audio_admissions_after(&db, community_id, None)
            .await
            .expect("first page");
        assert_eq!(first.len(), 256);
        let cursor = first.last().expect("first page cursor").admission_id;
        let second = reconcilable_audio_admissions_after(&db, community_id, Some(cursor))
            .await
            .expect("second page");
        assert_eq!(second.len(), 44);
        assert!(second
            .iter()
            .all(|candidate| candidate.admission_id > cursor));
        assert!(reconcilable_audio_admissions_after(
            &db,
            community_id,
            second.last().map(|candidate| candidate.admission_id),
        )
        .await
        .expect("empty terminal page")
        .is_empty());
    }
}
