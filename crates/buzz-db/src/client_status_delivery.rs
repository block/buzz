//! Crash-recoverable delivery journal for connection-local binding status.
//!
//! The status producer inserts the exact signed event and its authoritative
//! transition in a caller-owned transaction. Workers then use a fenced claim
//! token to acknowledge delivery or record a bounded failure. Expired claims
//! are reclaimable, and every claim/result is appended to a compact audit
//! timeline. Routing fields accept only domain-scoped fingerprints. The
//! payload is opaque producer-prevalidated wire data; typed kind-24244 parsing
//! and privacy validation belong to the relay adapter and remain mandatory.

use std::{fmt, time::Duration};

use buzz_auth::{
    ActiveLocalBinding, AuthoritativeAuthorizationRecheck, AuthorizationFinalizationRechecker,
    AuthorizationFinalizer, CurrentBindingStatusEvidenceRequest, FinalizedAuthContext,
    LocalAuthorizationPolicy, LocalStatusEvidenceResolver, PreparedAuthorizationRecheck,
    ProofTransport, RouteCapability, VerifiedBindingStatusProof,
};
use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{Db, DbError, Result};

const MAX_ATTEMPTS: i16 = 16;
const MAX_CLAIM_SECONDS: u64 = 300;
const MAX_RETRY_SECONDS: u64 = 300;
const WRITE_DEADLINE_MARGIN_SECONDS: i64 = 1;
const MAX_STATUS_WRITE_SECONDS: u64 = 5;
const STATUS_EVIDENCE_LIFETIME_SECONDS: i64 = 300;

/// Status event placed on the dedicated authenticated connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum StatusDeliveryKind {
    /// Current binding presentation.
    Current = 1,
    /// Opaque withdrawal superseding a prior presentation.
    Withdrawal = 2,
}

/// Closed, privacy-safe delivery failure taxonomy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i16)]
pub enum StatusDeliveryFailure {
    /// Writer or transport unavailable; retry is permitted.
    Transient = 1,
    /// The target authenticated connection no longer exists.
    ConnectionGone = 2,
    /// The final authorization fence no longer permits presentation.
    StaleFence = 3,
    /// The stored signed payload failed local validation.
    InvalidPayload = 4,
    /// The presentation reached its exclusive freshness bound.
    Expired = 5,
    /// The bounded retry budget was consumed.
    AttemptsExhausted = 6,
    /// A worker claim expired after visibility became ambiguous.
    LeaseExpiredUnknown = 7,
}

impl StatusDeliveryFailure {
    const fn is_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}

/// Immutable enqueue input created only after allocation, signing, and the
/// final presentation fence have all succeeded.
#[derive(Clone, Copy)]
pub struct NewStatusDelivery<'a> {
    /// Server-resolved tenant.
    pub community_id: CommunityId,
    /// Stable status delivery identity.
    pub delivery_id: Uuid,
    /// Immutable transition identity for this exact connection generation.
    pub transition_id: Uuid,
    /// Stable causal operation identity.
    pub operation_id: Uuid,
    /// Digest of the complete status request.
    pub request_fingerprint: [u8; 32],
    /// Current or withdrawal wire event.
    pub kind: StatusDeliveryKind,
    /// Domain-scoped digest of the status subject.
    pub subject_fingerprint: [u8; 32],
    /// Domain-scoped digest of the relay signing identity.
    pub signer_fingerprint: [u8; 32],
    /// Domain-scoped digest of the authenticated connection.
    pub connection_fingerprint: [u8; 32],
    /// Strictly increasing status revision for this connection generation.
    pub status_revision: u64,
    /// Exact prior revision superseded by a withdrawal.
    pub supersedes_revision: Option<u64>,
    /// Exclusive validity bound encoded into the signed event.
    pub fresh_until: DateTime<Utc>,
    /// Opaque producer-prevalidated relay-signed event bytes.
    pub signed_payload: &'a [u8],
    /// Private authoritative tuple required to re-fence a current delivery
    /// after a worker or process crash. Withdrawals never carry this value.
    pub current_evidence: Option<&'a CanonicalCurrentBindingEvidence>,
}

impl fmt::Debug for NewStatusDelivery<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NewStatusDelivery([REDACTED])")
    }
}

/// Result of idempotently recording one authoritative status transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueStatusDeliveryOutcome {
    /// Transition and pending delivery were inserted together.
    Enqueued,
    /// Existing still-fresh transition acquired a distinct reconnect target.
    TargetEnqueued,
    /// The exact immutable transition already exists.
    ExactReplay,
}

/// One exclusively claimed signed status event.
pub struct ClaimedStatusDelivery {
    community_id: CommunityId,
    delivery_id: Uuid,
    claim_id: Uuid,
    connection_fingerprint: [u8; 32],
    kind: StatusDeliveryKind,
    status_revision: u64,
    signed_payload: Vec<u8>,
    payload_digest: [u8; 32],
    current_evidence: Option<CanonicalCurrentBindingEvidence>,
    claimed_until: DateTime<Utc>,
    attempt: u16,
}

impl ClaimedStatusDelivery {
    /// Server-resolved tenant owning this delivery.
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Stable delivery identity.
    pub const fn delivery_id(&self) -> Uuid {
        self.delivery_id
    }

    /// Fencing token required for completion.
    pub const fn claim_id(&self) -> Uuid {
        self.claim_id
    }

    /// Exact redacted target selected by the authenticated connection owner.
    pub const fn connection_fingerprint(&self) -> [u8; 32] {
        self.connection_fingerprint
    }

    /// Current or withdrawal event.
    pub const fn kind(&self) -> StatusDeliveryKind {
        self.kind
    }

    /// Connection-local revision carried by the signed event.
    pub const fn status_revision(&self) -> u64 {
        self.status_revision
    }

    /// Exact relay-signed event bytes.
    pub fn signed_payload(&self) -> &[u8] {
        &self.signed_payload
    }

    /// Digest of the exact bytes covered by this claim.
    pub const fn payload_digest(&self) -> [u8; 32] {
        self.payload_digest
    }

    /// Original private evidence tuple for a current presentation.
    pub fn current_evidence(&self) -> Option<&CanonicalCurrentBindingEvidence> {
        self.current_evidence.as_ref()
    }

    /// Exclusive database claim deadline. Writer I/O must complete before it.
    pub const fn claimed_until(&self) -> DateTime<Utc> {
        self.claimed_until
    }

    /// One-based bounded delivery attempt.
    pub const fn attempt(&self) -> u16 {
        self.attempt
    }
}

impl fmt::Debug for ClaimedStatusDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedStatusDelivery")
            .field("kind", &self.kind)
            .field("attempt", &self.attempt)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Database-fenced permission to perform the physical writer flush.
///
/// Only [`Db::authorize_status_delivery`] can construct this value. A later
/// head transition cannot erase a flush that already occurred, so completion
/// records the physical outcome under this exact claim without rechecking the
/// mutable head.
pub struct AuthorizedStatusDelivery {
    community_id: CommunityId,
    delivery_id: Uuid,
    claim_id: Uuid,
    kind: StatusDeliveryKind,
    status_revision: u64,
    payload_digest: [u8; 32],
    write_budget: Duration,
    transaction: Option<Transaction<'static, Postgres>>,
}

impl AuthorizedStatusDelivery {
    /// Stable delivery identity bound to the authorized bytes.
    pub const fn delivery_id(&self) -> Uuid {
        self.delivery_id
    }

    /// Exact claim fence bound to the authorized bytes.
    pub const fn claim_id(&self) -> Uuid {
        self.claim_id
    }

    /// Current or withdrawal disposition bound to the authorization.
    pub const fn kind(&self) -> StatusDeliveryKind {
        self.kind
    }

    /// Connection-local wire revision bound to the authorization.
    pub const fn status_revision(&self) -> u64 {
        self.status_revision
    }

    /// SHA-256 digest of the only bytes this authorization permits.
    pub const fn payload_digest(&self) -> [u8; 32] {
        self.payload_digest
    }

    /// Monotonic writer budget computed from PostgreSQL time and kept strictly
    /// inside the database claim lease.
    pub const fn write_budget(&self) -> Duration {
        self.write_budget
    }
}

impl fmt::Debug for AuthorizedStatusDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizedStatusDelivery([REDACTED])")
    }
}

/// Idempotent completion result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompleteStatusDeliveryOutcome {
    /// The claimed delivery became durably delivered.
    Delivered,
    /// This exact claim had already been acknowledged.
    ExactReplay,
    /// The row is owned by a newer claim or has a different terminal result.
    LostClaim,
}

/// Failure-recording result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailStatusDeliveryOutcome {
    /// The row returned to pending with a bounded retry time.
    RetryScheduled,
    /// The row entered a terminal, auditable failure state.
    Terminal,
    /// This exact terminal failure had already been recorded.
    ExactReplay,
    /// The row is owned by a newer claim or has another result.
    LostClaim,
}

impl LocalStatusEvidenceResolver for Db {
    type Error = DbError;

    async fn current_status_evidence(
        &self,
        request: &CurrentBindingStatusEvidenceRequest,
    ) -> std::result::Result<CanonicalCurrentBindingEvidence, Self::Error> {
        load_current_status_evidence(self, request).await
    }

    async fn recheck_current_status_evidence(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
    ) -> std::result::Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), Self::Error> {
        let request = CurrentBindingStatusEvidenceRequest::new(
            evidence.authorization_domain(),
            evidence.event_author_pubkey(),
        )
        .map_err(|_| invalid_delivery())?;
        let (current, authoritative_now) = load_current_status_evidence_at(self, &request).await?;
        let exact_coordinates = current.authorization_domain() == evidence.authorization_domain()
            && current.event_author_pubkey() == evidence.event_author_pubkey()
            && current.binding_id() == evidence.binding_id()
            && current.binding_version() == evidence.binding_version()
            && current.policy_revision() == evidence.policy_revision()
            && current.invalidation_generation() == evidence.invalidation_generation()
            && current.authority_epoch() == evidence.authority_epoch()
            && current.fence() == evidence.fence();
        if !exact_coordinates || !evidence.is_fresh_at(authoritative_now) {
            return Err(DbError::InvalidData(
                "client status evidence is no longer current".to_owned(),
            ));
        }
        Ok((evidence.clone(), authoritative_now))
    }
}

#[derive(Clone)]
struct StatusAuthorizationFinalRechecker {
    db: Db,
}

impl AuthorizationFinalizationRechecker for StatusAuthorizationFinalRechecker {
    async fn recheck(
        &self,
        request: &PreparedAuthorizationRecheck,
    ) -> std::result::Result<AuthoritativeAuthorizationRecheck, buzz_auth::AuthorizationError> {
        let snapshot = request.lease_dependencies();
        let (_, domain) = snapshot.identity();
        let (capability, actor, owner) = snapshot.authority();
        let (binding_id, binding_version) = snapshot.binding();
        let (request_fingerprint, target, transport, _) = snapshot.request_binding();
        let (policy_revision, invalidation_generation, authority_epoch) =
            snapshot.dependency_versions();
        if capability != RouteCapability::BindingStatus
            || transport != ProofTransport::Nip42
            || owner.is_some()
            || request_fingerprint == &[0; 32]
            || target == &[0; 32]
            || request.verifier_stamp().is_some()
        {
            return Err(buzz_auth::AuthorizationError::StaleRecheck);
        }
        let mut transaction = self
            .db
            .pool
            .begin()
            .await
            .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        sqlx::query("LOCK TABLE identity_enrollment_policies IN SHARE MODE")
            .execute(&mut *transaction)
            .await
            .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        let authoritative_now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        let row = sqlx::query(
            "SELECT binding.binding_id, binding.binding_version, binding.policy_revision, \
                    domain.current_generation \
             FROM identity_bindings binding \
             JOIN authorization_invalidation_domains domain \
               ON domain.community_id=binding.community_id \
             JOIN identity_enrollment_policies policy \
               ON policy.community_id=binding.community_id \
              AND policy.policy_revision=binding.policy_revision \
             WHERE binding.community_id=$1 AND binding.binding_id=$2 \
               AND binding.binding_version=$3 AND binding.event_author_pubkey=$4 \
               AND binding.binding_state=1 AND binding.lifecycle_revision=1 \
               AND (binding.expires_at IS NULL OR binding.expires_at > $5) \
               AND policy.effective_at <= $5 \
               AND (policy.expires_at IS NULL OR policy.expires_at > $5) \
               AND NOT EXISTS (SELECT 1 FROM identity_enrollment_policies newer \
                   WHERE newer.community_id=binding.community_id \
                     AND newer.policy_revision > binding.policy_revision) \
             FOR SHARE OF binding, domain, policy",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .bind(
            i64::try_from(binding_version)
                .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?,
        )
        .bind(actor.as_bytes())
        .bind(authoritative_now)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?
        .ok_or(buzz_auth::AuthorizationError::StaleRecheck)?;
        let current_policy = positive_u64(
            row.try_get("policy_revision")
                .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?,
        )
        .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        let current_generation = u64::try_from(
            row.try_get::<i64, _>("current_generation")
                .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?,
        )
        .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        let current_epoch = current_generation
            .checked_add(1)
            .ok_or(buzz_auth::AuthorizationError::StaleRecheck)?;
        let current_fence = derive_status_evidence_fence(
            domain,
            actor,
            binding_id,
            binding_version,
            current_policy,
            current_generation,
            current_epoch,
        )
        .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        if current_policy != policy_revision
            || current_generation != invalidation_generation
            || current_epoch != authority_epoch
            || current_fence != snapshot.fence()
        {
            return Err(buzz_auth::AuthorizationError::StaleRecheck);
        }
        transaction
            .commit()
            .await
            .map_err(|_| buzz_auth::AuthorizationError::StaleRecheck)?;
        Ok(AuthoritativeAuthorizationRecheck::from_authoritative_parts(
            snapshot,
            None,
            authoritative_now,
        ))
    }
}

/// Insert a status transition and pending delivery in a caller-owned
/// transaction. Committing that transaction makes both visible atomically;
/// rolling it back makes neither visible.
pub async fn enqueue_status_delivery_tx(
    transaction: &mut Transaction<'_, Postgres>,
    delivery: &NewStatusDelivery<'_>,
) -> Result<EnqueueStatusDeliveryOutcome> {
    validate_delivery(delivery)?;
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended(\
         'buzz:status-delivery-operation:v1:' || $1::text || ':' || $2::text, 0))",
    )
    .bind(delivery.community_id.as_uuid())
    .bind(delivery.operation_id)
    .execute(&mut **transaction)
    .await?;
    let payload_digest: [u8; 32] = Sha256::digest(delivery.signed_payload).into();
    if let Some(existing) = sqlx::query(
        "SELECT transition_id, request_fingerprint, delivery_kind, subject_fingerprint, \
                signer_fingerprint, connection_fingerprint, status_revision, \
                supersedes_revision, signed_payload, payload_digest, fresh_until, \
                evidence_author_pubkey, evidence_binding_id, evidence_binding_version, \
                evidence_policy_revision, evidence_invalidation_generation, \
                evidence_authority_epoch, evidence_fence, evidence_observed_at \
         FROM client_status_transitions WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(delivery.community_id.as_uuid())
    .bind(delivery.operation_id)
    .fetch_optional(&mut **transaction)
    .await?
    {
        let supersedes = delivery
            .supersedes_revision
            .map(|revision| i64::try_from(revision).map_err(|_| invalid_delivery()))
            .transpose()?;
        let exact = existing.try_get::<Uuid, _>("transition_id")? == delivery.transition_id
            && existing.try_get::<Vec<u8>, _>("request_fingerprint")?
                == delivery.request_fingerprint
            && existing.try_get::<i16, _>("delivery_kind")? == delivery.kind as i16
            && existing.try_get::<Vec<u8>, _>("subject_fingerprint")?
                == delivery.subject_fingerprint
            && existing.try_get::<Vec<u8>, _>("signer_fingerprint")? == delivery.signer_fingerprint
            && existing.try_get::<Vec<u8>, _>("connection_fingerprint")?
                == delivery.connection_fingerprint
            && existing.try_get::<i64, _>("status_revision")?
                == i64::try_from(delivery.status_revision).map_err(|_| invalid_delivery())?
            && existing.try_get::<Option<i64>, _>("supersedes_revision")? == supersedes
            && existing.try_get::<Vec<u8>, _>("signed_payload")? == delivery.signed_payload
            && existing.try_get::<Vec<u8>, _>("payload_digest")? == payload_digest
            && existing.try_get::<DateTime<Utc>, _>("fresh_until")? == delivery.fresh_until
            && evidence_row_matches(&existing, delivery.current_evidence)?;
        if !exact {
            return Err(DbError::InvalidData(
                "client status delivery replay conflicts with prior transition".to_owned(),
            ));
        }
        let still_current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM client_status_transition_heads head \
             JOIN client_status_transitions transition \
               ON transition.community_id=head.community_id \
              AND transition.transition_id=head.transition_id \
             WHERE head.community_id=$1 AND head.connection_fingerprint=$2 \
               AND head.transition_id=$3 \
               AND transition.fresh_until > transaction_timestamp())",
        )
        .bind(delivery.community_id.as_uuid())
        .bind(delivery.connection_fingerprint.as_slice())
        .bind(delivery.transition_id)
        .fetch_one(&mut **transaction)
        .await?;
        if !still_current {
            return Err(DbError::InvalidData(
                "client status transition is no longer current".to_owned(),
            ));
        }
        if let Some(existing_delivery_id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT delivery_id FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND transition_id=$2 AND connection_fingerprint=$3",
        )
        .bind(delivery.community_id.as_uuid())
        .bind(delivery.transition_id)
        .bind(delivery.connection_fingerprint.as_slice())
        .fetch_optional(&mut **transaction)
        .await?
        {
            if existing_delivery_id != delivery.delivery_id {
                return Err(DbError::InvalidData(
                    "client status delivery replay conflicts with prior target".to_owned(),
                ));
            }
            return Ok(EnqueueStatusDeliveryOutcome::ExactReplay);
        }
        let inserted = insert_delivery_target(transaction, delivery, false).await?;
        return Ok(if inserted {
            EnqueueStatusDeliveryOutcome::TargetEnqueued
        } else {
            EnqueueStatusDeliveryOutcome::ExactReplay
        });
    }

    let prior = sqlx::query(
        "SELECT status_revision FROM client_status_transition_heads \
         WHERE community_id=$1 AND connection_fingerprint=$2 FOR UPDATE",
    )
    .bind(delivery.community_id.as_uuid())
    .bind(delivery.connection_fingerprint.as_slice())
    .fetch_optional(&mut **transaction)
    .await?;
    let prior_revision = prior
        .as_ref()
        .map(|row| row.try_get::<i64, _>("status_revision"))
        .transpose()?;
    let revision = i64::try_from(delivery.status_revision).map_err(|_| invalid_delivery())?;
    let expected = prior_revision.map_or(1, |prior| prior.saturating_add(1));
    let supersedes = delivery
        .supersedes_revision
        .map(|value| i64::try_from(value).map_err(|_| invalid_delivery()))
        .transpose()?;
    if revision != expected
        || (delivery.kind == StatusDeliveryKind::Current && supersedes.is_some())
        || (delivery.kind == StatusDeliveryKind::Withdrawal && supersedes != prior_revision)
    {
        return Err(DbError::InvalidData(
            "client status transition does not advance the authoritative head".to_owned(),
        ));
    }
    let evidence = delivery.current_evidence;
    sqlx::query(
        "INSERT INTO client_status_transitions \
         (community_id, transition_id, operation_id, request_fingerprint, delivery_kind, \
          subject_fingerprint, signer_fingerprint, connection_fingerprint, status_revision, \
          supersedes_revision, signed_payload, payload_digest, fresh_until, retain_until, \
          evidence_author_pubkey, evidence_binding_id, evidence_binding_version, \
          evidence_policy_revision, evidence_invalidation_generation, \
          evidence_authority_epoch, evidence_fence, evidence_observed_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$13 + INTERVAL '1 day', \
                 $14,$15,$16,$17,$18,$19,$20,$21)",
    )
    .bind(delivery.community_id.as_uuid())
    .bind(delivery.transition_id)
    .bind(delivery.operation_id)
    .bind(delivery.request_fingerprint.as_slice())
    .bind(delivery.kind as i16)
    .bind(delivery.subject_fingerprint.as_slice())
    .bind(delivery.signer_fingerprint.as_slice())
    .bind(delivery.connection_fingerprint.as_slice())
    .bind(revision)
    .bind(supersedes)
    .bind(delivery.signed_payload)
    .bind(payload_digest.as_slice())
    .bind(delivery.fresh_until)
    .bind(evidence.map(|value| value.event_author_pubkey().to_bytes()))
    .bind(evidence.map(CanonicalCurrentBindingEvidence::binding_id))
    .bind(
        evidence
            .map(CanonicalCurrentBindingEvidence::binding_version)
            .map(i64::try_from)
            .transpose()
            .map_err(|_| invalid_delivery())?,
    )
    .bind(
        evidence
            .map(CanonicalCurrentBindingEvidence::policy_revision)
            .map(i64::try_from)
            .transpose()
            .map_err(|_| invalid_delivery())?,
    )
    .bind(
        evidence
            .map(CanonicalCurrentBindingEvidence::invalidation_generation)
            .map(i64::try_from)
            .transpose()
            .map_err(|_| invalid_delivery())?,
    )
    .bind(
        evidence
            .map(CanonicalCurrentBindingEvidence::authority_epoch)
            .map(i64::try_from)
            .transpose()
            .map_err(|_| invalid_delivery())?,
    )
    .bind(evidence.map(|value| value.fence().as_bytes().to_vec()))
    .bind(evidence.map(CanonicalCurrentBindingEvidence::observed_at))
    .execute(&mut **transaction)
    .await?;
    if prior.is_some() {
        sqlx::query(
            "UPDATE client_status_transition_heads SET transition_id=$3, status_revision=$4, \
             delivery_kind=$5, updated_at=transaction_timestamp() \
             WHERE community_id=$1 AND connection_fingerprint=$2",
        )
        .bind(delivery.community_id.as_uuid())
        .bind(delivery.connection_fingerprint.as_slice())
        .bind(delivery.transition_id)
        .bind(revision)
        .bind(delivery.kind as i16)
        .execute(&mut **transaction)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO client_status_transition_heads \
             (community_id, connection_fingerprint, subject_fingerprint, signer_fingerprint, \
              transition_id, status_revision, delivery_kind) VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(delivery.community_id.as_uuid())
        .bind(delivery.connection_fingerprint.as_slice())
        .bind(delivery.subject_fingerprint.as_slice())
        .bind(delivery.signer_fingerprint.as_slice())
        .bind(delivery.transition_id)
        .bind(revision)
        .bind(delivery.kind as i16)
        .execute(&mut **transaction)
        .await?;
    }
    let inserted = insert_delivery_target(transaction, delivery, true).await?;
    debug_assert!(inserted);
    Ok(EnqueueStatusDeliveryOutcome::Enqueued)
}

impl Db {
    /// Read PostgreSQL transaction time for typed payload validation.
    pub async fn status_delivery_authoritative_now(&self) -> Result<DateTime<Utc>> {
        Ok(sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Finalize one direct, purpose-sealed connection status authorization.
    pub async fn finalize_binding_status_authorization(
        &self,
        proof: VerifiedBindingStatusProof,
    ) -> Result<FinalizedAuthContext> {
        let domain = proof.authorization_domain();
        let actor = proof.actor_pubkey();
        let proof_expires_at = proof.expires_at();
        let row = sqlx::query(
            "SELECT binding.issuer, binding.subject, binding.binding_id, \
                    binding.binding_version, binding.expires_at AS binding_expires_at, \
                    binding.policy_revision, policy.expires_at AS policy_expires_at, \
                    domain.current_generation, transaction_timestamp() AS now \
             FROM identity_bindings binding \
             JOIN authorization_invalidation_domains domain \
               ON domain.community_id=binding.community_id \
             JOIN identity_enrollment_policies policy \
               ON policy.community_id=binding.community_id \
              AND policy.policy_revision=binding.policy_revision \
             WHERE binding.community_id=$1 AND binding.event_author_pubkey=$2 \
               AND binding.binding_state=1 AND binding.lifecycle_revision=1 \
               AND (binding.expires_at IS NULL OR binding.expires_at > transaction_timestamp()) \
               AND policy.effective_at <= transaction_timestamp() \
               AND (policy.expires_at IS NULL OR policy.expires_at > transaction_timestamp()) \
               AND NOT EXISTS (SELECT 1 FROM identity_enrollment_policies newer \
                   WHERE newer.community_id=binding.community_id \
                     AND newer.policy_revision > binding.policy_revision)",
        )
        .bind(domain.as_uuid())
        .bind(actor.as_bytes())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::InvalidData("binding status authorization denied".to_owned()))?;
        let authoritative_now: DateTime<Utc> = row.try_get("now")?;
        let binding_id: Uuid = row.try_get("binding_id")?;
        let binding_version = positive_u64(row.try_get("binding_version")?)?;
        let policy_revision = positive_u64(row.try_get("policy_revision")?)?;
        let invalidation_generation = u64::try_from(row.try_get::<i64, _>("current_generation")?)
            .map_err(|_| invalid_delivery())?;
        let authority_epoch = invalidation_generation
            .checked_add(1)
            .ok_or_else(invalid_delivery)?;
        let binding_expires_at: Option<DateTime<Utc>> = row.try_get("binding_expires_at")?;
        let policy_expires_at: Option<DateTime<Utc>> = row.try_get("policy_expires_at")?;
        let protocol_expires_at = authoritative_now
            .checked_add_signed(chrono::Duration::seconds(STATUS_EVIDENCE_LIFETIME_SECONDS))
            .ok_or_else(invalid_delivery)?;
        let mut expires_at = proof_expires_at.min(protocol_expires_at);
        if let Some(bound) = binding_expires_at {
            expires_at = expires_at.min(bound);
        }
        if let Some(bound) = policy_expires_at {
            expires_at = expires_at.min(bound);
        }
        if expires_at <= authoritative_now {
            return Err(DbError::InvalidData(
                "binding status authorization denied".to_owned(),
            ));
        }
        let binding = ActiveLocalBinding::from_storage_parts(
            domain,
            row.try_get("issuer")?,
            row.try_get("subject")?,
            binding_id,
            binding_version,
            actor,
            binding_expires_at,
        )
        .ok_or_else(invalid_delivery)?;
        let fence = derive_status_evidence_fence(
            domain,
            actor,
            binding_id,
            binding_version,
            policy_revision,
            invalidation_generation,
            authority_epoch,
        )?;
        let policy = LocalAuthorizationPolicy::from_database(
            domain,
            Uuid::new_v4(),
            policy_revision,
            invalidation_generation,
            authority_epoch,
            fence,
            RouteCapability::BindingStatus,
            expires_at,
            None,
            None,
        )
        .ok_or_else(invalid_delivery)?;
        let prepared = proof
            .prepare_authorization(binding, policy, authoritative_now)
            .map_err(|_| DbError::InvalidData("binding status authorization denied".to_owned()))?;
        let rechecker = StatusAuthorizationFinalRechecker { db: self.clone() };
        let witness = AuthorizationFinalizer::recheck(&prepared, &rechecker)
            .await
            .map_err(|_| DbError::InvalidData("binding status authorization denied".to_owned()))?;
        AuthorizationFinalizer::finalize(prepared, witness)
            .map_err(|_| DbError::InvalidData("binding status authorization denied".to_owned()))
    }

    /// Install the mandatory bounded delivery-audit capacity row. Status
    /// production fails closed until this succeeds.
    pub async fn install_status_delivery_capacity(&self, community_id: CommunityId) -> Result<()> {
        sqlx::query(
            "INSERT INTO client_status_delivery_capacity (community_id) VALUES ($1) \
             ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Latch delivery audit unhealthy after a persistence or reconciliation
    /// failure. New transitions then fail closed.
    pub async fn latch_status_delivery_failure(
        &self,
        community_id: CommunityId,
        reason: u16,
    ) -> Result<()> {
        if !(1..=3).contains(&reason) {
            return Err(invalid_delivery());
        }
        let changed = sqlx::query(
            "UPDATE client_status_delivery_capacity SET healthy=FALSE, failure_reason=$2, \
             updated_at=transaction_timestamp() WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .bind(i16::try_from(reason).map_err(|_| invalid_delivery())?)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(DbError::InvalidData(
                "client status delivery capacity is not installed".to_owned(),
            ));
        }
        Ok(())
    }

    /// Atomically commit one status transition and its pending delivery.
    pub async fn enqueue_status_delivery(
        &self,
        delivery: &NewStatusDelivery<'_>,
    ) -> Result<EnqueueStatusDeliveryOutcome> {
        let mut transaction = self.pool.begin().await?;
        let outcome = enqueue_status_delivery_tx(&mut transaction, delivery).await?;
        transaction.commit().await?;
        Ok(outcome)
    }

    /// Claim one ready delivery. An expired claim may be reclaimed after a
    /// worker crash; its signed revision makes duplicate sends replay-safe.
    pub async fn claim_status_delivery(
        &self,
        community_id: CommunityId,
        connection_fingerprint: [u8; 32],
        claim_lease: Duration,
    ) -> Result<Option<ClaimedStatusDelivery>> {
        if connection_fingerprint == [0; 32] {
            return Err(invalid_delivery());
        }
        let lease_seconds = bounded_seconds(claim_lease, MAX_CLAIM_SECONDS)?;
        let mut transaction = self.pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT delivery.delivery_id, transition.delivery_kind, transition.status_revision, \
                    transition.signed_payload, transition.payload_digest, transition.fresh_until, \
                    transition.evidence_author_pubkey, transition.evidence_binding_id, \
                    transition.evidence_binding_version, transition.evidence_policy_revision, \
                    transition.evidence_invalidation_generation, \
                    transition.evidence_authority_epoch, transition.evidence_fence, \
                    transition.evidence_observed_at, delivery.attempt_count, delivery.claim_id, \
                    delivery.claimed_until \
             FROM client_status_delivery_outbox delivery \
             JOIN client_status_transitions transition \
              ON transition.community_id=delivery.community_id \
             AND transition.transition_id=delivery.transition_id \
             JOIN client_status_transition_heads head \
               ON head.community_id=transition.community_id \
              AND head.connection_fingerprint=delivery.connection_fingerprint \
              AND head.transition_id=transition.transition_id \
             WHERE delivery.community_id=$1 AND delivery.connection_fingerprint=$2 \
               AND delivery.delivery_state=1 \
               AND delivery.next_attempt_at <= transaction_timestamp() \
               AND transition.fresh_until > transaction_timestamp() \
               AND (claim_id IS NULL OR claimed_until <= transaction_timestamp()) \
               AND attempt_count < $3 \
             ORDER BY delivery.created_at, delivery.delivery_id \
             FOR UPDATE OF delivery SKIP LOCKED LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(connection_fingerprint.as_slice())
        .bind(MAX_ATTEMPTS)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            transaction.commit().await?;
            return Ok(None);
        };
        let delivery_id: Uuid = row.try_get("delivery_id")?;
        let next_attempt = row.try_get::<i16, _>("attempt_count")? + 1;
        let expired_claim: Option<Uuid> = row.try_get("claim_id")?;
        if expired_claim.is_some() {
            let sequence = next_event_sequence(&mut transaction, community_id, delivery_id).await?;
            insert_delivery_event(
                &mut transaction,
                community_id,
                delivery_id,
                sequence,
                8,
                StatusDeliveryFailure::LeaseExpiredUnknown as i16,
                next_attempt - 1,
            )
            .await?;
        }
        let claim_id = Uuid::new_v4();
        let claimed_until: DateTime<Utc> = sqlx::query_scalar(
            "UPDATE client_status_delivery_outbox SET claim_id=$3, \
             claimed_until=transaction_timestamp()+make_interval(secs => $4), \
             attempt_count=$5, attempt_started_at=clock_timestamp(), \
             last_failure_reason=CASE WHEN claim_id IS NULL THEN last_failure_reason ELSE 7 END \
             WHERE community_id=$1 AND delivery_id=$2 RETURNING claimed_until",
        )
        .bind(community_id.as_uuid())
        .bind(delivery_id)
        .bind(claim_id)
        .bind(lease_seconds as f64)
        .bind(next_attempt)
        .fetch_one(&mut *transaction)
        .await?;
        let sequence = next_event_sequence(&mut transaction, community_id, delivery_id).await?;
        insert_delivery_event(
            &mut transaction,
            community_id,
            delivery_id,
            sequence,
            4,
            0,
            next_attempt,
        )
        .await?;
        let kind = delivery_kind(row.try_get("delivery_kind")?)?;
        let current_evidence = evidence_from_row(&row, community_id, kind)?;
        let payload_digest = fixed_digest(row.try_get("payload_digest")?)?;
        transaction.commit().await?;
        Ok(Some(ClaimedStatusDelivery {
            community_id,
            delivery_id,
            claim_id,
            connection_fingerprint,
            kind,
            status_revision: u64::try_from(row.try_get::<i64, _>("status_revision")?)
                .map_err(|_| invalid_delivery())?,
            signed_payload: row.try_get("signed_payload")?,
            payload_digest,
            current_evidence,
            claimed_until,
            attempt: u16::try_from(next_attempt).map_err(|_| invalid_delivery())?,
        }))
    }

    /// Recheck the authoritative head immediately before any writer I/O.
    ///
    /// A stale or expired exact claim is terminalized before the caller can
    /// flush bytes. `None` therefore means that no writer I/O is authorized.
    pub async fn authorize_status_delivery(
        &self,
        claimed: &ClaimedStatusDelivery,
    ) -> Result<Option<AuthorizedStatusDelivery>> {
        let mut transaction = self.pool.begin().await?;
        // The relay hard-bounds its fence-owning task at seven seconds. These
        // server-side limits independently prevent a disconnected worker from
        // retaining mutable tenant state if its runtime can no longer poll the
        // transaction rollback.
        sqlx::query("SET LOCAL statement_timeout = '2s'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SET LOCAL lock_timeout = '2s'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SET LOCAL idle_in_transaction_session_timeout = '12s'")
            .execute(&mut *transaction)
            .await?;
        let Some(row) =
            delivery_state_for_update(&mut transaction, claimed.community_id, claimed.delivery_id)
                .await?
        else {
            transaction.commit().await?;
            return Ok(None);
        };
        let state: i16 = row.try_get("delivery_state")?;
        let active_claim: Option<Uuid> = row.try_get("claim_id")?;
        let claimed_until: Option<DateTime<Utc>> = row.try_get("claimed_until")?;
        let authoritative_now: DateTime<Utc> = sqlx::query_scalar("SELECT transaction_timestamp()")
            .fetch_one(&mut *transaction)
            .await?;
        if state != 1
            || active_claim != Some(claimed.claim_id)
            || claimed_until.is_none_or(|deadline| deadline <= authoritative_now)
        {
            transaction.commit().await?;
            return Ok(None);
        }
        let transition_fresh_until: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT transition.fresh_until \
             FROM client_status_delivery_outbox delivery \
             JOIN client_status_transitions transition \
               ON transition.community_id=delivery.community_id \
              AND transition.transition_id=delivery.transition_id \
             JOIN client_status_transition_heads head \
               ON head.community_id=transition.community_id \
              AND head.connection_fingerprint=delivery.connection_fingerprint \
              AND head.transition_id=transition.transition_id \
             WHERE delivery.community_id=$1 AND delivery.delivery_id=$2 \
             FOR SHARE OF transition, head",
        )
        .bind(claimed.community_id.as_uuid())
        .bind(claimed.delivery_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let transition_current = transition_fresh_until.is_some();
        let transition_fresh =
            transition_fresh_until.is_some_and(|fresh_until| fresh_until > authoritative_now);
        let evidence_current = if transition_current && transition_fresh {
            match claimed.current_evidence() {
                Some(evidence) => {
                    // Migration 0038 makes the community row the tenant-scoped
                    // policy-insert fence. Exact binding, domain, and policy
                    // rows are locked by the evidence recheck below.
                    sqlx::query("SELECT id FROM communities WHERE id=$1 FOR SHARE")
                        .bind(claimed.community_id.as_uuid())
                        .fetch_one(&mut *transaction)
                        .await?;
                    recheck_status_evidence_tx(&mut transaction, evidence, authoritative_now)
                        .await?
                }
                None => claimed.kind == StatusDeliveryKind::Withdrawal,
            }
        } else {
            false
        };
        if !transition_current || !transition_fresh || !evidence_current {
            let failure = if transition_current && !transition_fresh {
                StatusDeliveryFailure::Expired
            } else {
                StatusDeliveryFailure::StaleFence
            };
            let attempt: i16 = row.try_get("attempt_count")?;
            sqlx::query(
                "UPDATE client_status_delivery_outbox SET delivery_state=3, claim_id=NULL, \
                 claimed_until=NULL, completion_claim_id=$3, last_failure_reason=$4, \
                 terminal_at=clock_timestamp() WHERE community_id=$1 AND delivery_id=$2",
            )
            .bind(claimed.community_id.as_uuid())
            .bind(claimed.delivery_id)
            .bind(claimed.claim_id)
            .bind(failure as i16)
            .execute(&mut *transaction)
            .await?;
            let sequence =
                next_event_sequence(&mut transaction, claimed.community_id, claimed.delivery_id)
                    .await?;
            insert_delivery_event(
                &mut transaction,
                claimed.community_id,
                claimed.delivery_id,
                sequence,
                7,
                failure as i16,
                attempt,
            )
            .await?;
            transaction.commit().await?;
            return Ok(None);
        }
        let Some(write_deadline) = claimed_until.and_then(|deadline| {
            deadline.checked_sub_signed(chrono::Duration::seconds(WRITE_DEADLINE_MARGIN_SECONDS))
        }) else {
            transaction.commit().await?;
            return Ok(None);
        };
        if write_deadline <= authoritative_now {
            transaction.commit().await?;
            return Ok(None);
        }
        let write_budget = (write_deadline - authoritative_now)
            .to_std()
            .map_err(|_| invalid_delivery())?
            .min(Duration::from_secs(MAX_STATUS_WRITE_SECONDS));
        Ok(Some(AuthorizedStatusDelivery {
            community_id: claimed.community_id,
            delivery_id: claimed.delivery_id,
            claim_id: claimed.claim_id,
            kind: claimed.kind,
            status_revision: claimed.status_revision,
            payload_digest: claimed.payload_digest,
            write_budget,
            transaction: Some(transaction),
        }))
    }

    /// Release an unused pre-send authorization fence without changing its
    /// delivery claim. Recovery can reclaim that exact job after the lease.
    pub async fn abort_status_delivery_authorization(
        &self,
        mut authorization: AuthorizedStatusDelivery,
    ) -> Result<()> {
        if let Some(transaction) = authorization.transaction.take() {
            transaction.rollback().await?;
        }
        Ok(())
    }

    /// Record a physical writer flush authorized by the exact claim token.
    ///
    /// This method is called only after the writer reports that the bytes were
    /// flushed. It intentionally does not recheck the mutable status head:
    /// supersession after authorization cannot make already-flushed bytes
    /// retroactively undelivered.
    pub async fn complete_status_delivery(
        &self,
        authorization: &mut AuthorizedStatusDelivery,
    ) -> Result<CompleteStatusDeliveryOutcome> {
        let mut transaction = match authorization.transaction.take() {
            Some(transaction) => transaction,
            None => self.pool.begin().await?,
        };
        let Some(row) = delivery_state_for_update(
            &mut transaction,
            authorization.community_id,
            authorization.delivery_id,
        )
        .await?
        else {
            transaction.commit().await?;
            return Ok(CompleteStatusDeliveryOutcome::LostClaim);
        };
        let state: i16 = row.try_get("delivery_state")?;
        let active_claim: Option<Uuid> = row.try_get("claim_id")?;
        let completed_claim: Option<Uuid> = row.try_get("completion_claim_id")?;
        if state == 2 && completed_claim == Some(authorization.claim_id) {
            transaction.commit().await?;
            return Ok(CompleteStatusDeliveryOutcome::ExactReplay);
        }
        if state != 1 || active_claim != Some(authorization.claim_id) {
            transaction.commit().await?;
            return Ok(CompleteStatusDeliveryOutcome::LostClaim);
        }
        let attempt: i16 = row.try_get("attempt_count")?;
        sqlx::query(
            "UPDATE client_status_delivery_outbox SET delivery_state=2, claim_id=NULL, \
             claimed_until=NULL, completion_claim_id=$3, last_failure_reason=0, \
             delivered_at=clock_timestamp() WHERE community_id=$1 AND delivery_id=$2",
        )
        .bind(authorization.community_id.as_uuid())
        .bind(authorization.delivery_id)
        .bind(authorization.claim_id)
        .execute(&mut *transaction)
        .await?;
        let sequence = next_event_sequence(
            &mut transaction,
            authorization.community_id,
            authorization.delivery_id,
        )
        .await?;
        insert_delivery_event(
            &mut transaction,
            authorization.community_id,
            authorization.delivery_id,
            sequence,
            5,
            0,
            attempt,
        )
        .await?;
        transaction.commit().await?;
        Ok(CompleteStatusDeliveryOutcome::Delivered)
    }

    /// Record a retryable or terminal failure under the exact claim token.
    pub async fn fail_status_delivery(
        &self,
        community_id: CommunityId,
        delivery_id: Uuid,
        claim_id: Uuid,
        failure: StatusDeliveryFailure,
        retry_after: Duration,
    ) -> Result<FailStatusDeliveryOutcome> {
        let retry_seconds = bounded_seconds(retry_after, MAX_RETRY_SECONDS)?;
        let mut transaction = self.pool.begin().await?;
        let Some(row) =
            delivery_state_for_update(&mut transaction, community_id, delivery_id).await?
        else {
            transaction.commit().await?;
            return Ok(FailStatusDeliveryOutcome::LostClaim);
        };
        let state: i16 = row.try_get("delivery_state")?;
        let active_claim: Option<Uuid> = row.try_get("claim_id")?;
        let completed_claim: Option<Uuid> = row.try_get("completion_claim_id")?;
        let prior_reason: i16 = row.try_get("last_failure_reason")?;
        if state == 3 && completed_claim == Some(claim_id) && prior_reason == failure as i16 {
            transaction.commit().await?;
            return Ok(FailStatusDeliveryOutcome::ExactReplay);
        }
        if state != 1 || active_claim != Some(claim_id) {
            transaction.commit().await?;
            return Ok(FailStatusDeliveryOutcome::LostClaim);
        }
        let attempt: i16 = row.try_get("attempt_count")?;
        let retry = failure.is_retryable() && attempt < MAX_ATTEMPTS;
        let recorded_failure = if failure.is_retryable() && !retry {
            StatusDeliveryFailure::AttemptsExhausted
        } else {
            failure
        };
        if retry {
            sqlx::query(
                "UPDATE client_status_delivery_outbox SET claim_id=NULL, claimed_until=NULL, \
                 last_failure_reason=$3, next_attempt_at=transaction_timestamp()+make_interval(secs => $4) \
                 WHERE community_id=$1 AND delivery_id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(delivery_id)
            .bind(recorded_failure as i16)
            .bind(retry_seconds as f64)
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query(
                "UPDATE client_status_delivery_outbox SET delivery_state=3, claim_id=NULL, \
                 claimed_until=NULL, completion_claim_id=$3, last_failure_reason=$4, \
                 terminal_at=clock_timestamp() WHERE community_id=$1 AND delivery_id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(delivery_id)
            .bind(claim_id)
            .bind(recorded_failure as i16)
            .execute(&mut *transaction)
            .await?;
        }
        let sequence = next_event_sequence(&mut transaction, community_id, delivery_id).await?;
        insert_delivery_event(
            &mut transaction,
            community_id,
            delivery_id,
            sequence,
            if retry { 6 } else { 7 },
            recorded_failure as i16,
            attempt,
        )
        .await?;
        transaction.commit().await?;
        Ok(if retry {
            FailStatusDeliveryOutcome::RetryScheduled
        } else {
            FailStatusDeliveryOutcome::Terminal
        })
    }

    /// Terminalize every pending job owned by one exact dead connection.
    /// Reconnects use a new fingerprint and can never inherit these rows.
    pub async fn terminalize_status_connection(
        &self,
        community_id: CommunityId,
        connection_fingerprint: [u8; 32],
    ) -> Result<u64> {
        if connection_fingerprint == [0; 32] {
            return Err(invalid_delivery());
        }
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT delivery_id, claim_id, attempt_count \
             FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND connection_fingerprint=$2 AND delivery_state=1 \
             ORDER BY delivery_id FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(connection_fingerprint.as_slice())
        .fetch_all(&mut *transaction)
        .await?;
        for row in &rows {
            let delivery_id: Uuid = row.try_get("delivery_id")?;
            let completion_claim = row
                .try_get::<Option<Uuid>, _>("claim_id")?
                .unwrap_or_else(Uuid::new_v4);
            let attempt: i16 = row.try_get("attempt_count")?;
            sqlx::query(
                "UPDATE client_status_delivery_outbox SET delivery_state=3, claim_id=NULL, \
                 claimed_until=NULL, completion_claim_id=$3, last_failure_reason=2, \
                 terminal_at=clock_timestamp() WHERE community_id=$1 AND delivery_id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(delivery_id)
            .bind(completion_claim)
            .execute(&mut *transaction)
            .await?;
            let sequence = next_event_sequence(&mut transaction, community_id, delivery_id).await?;
            insert_delivery_event(
                &mut transaction,
                community_id,
                delivery_id,
                sequence,
                7,
                StatusDeliveryFailure::ConnectionGone as i16,
                attempt,
            )
            .await?;
        }
        transaction.commit().await?;
        u64::try_from(rows.len()).map_err(|_| invalid_delivery())
    }

    /// Terminalize expired or retry-exhausted pending jobs so crash recovery
    /// cannot pin bounded capacity indefinitely.
    pub async fn reconcile_status_deliveries(
        &self,
        community_id: CommunityId,
        limit: u16,
    ) -> Result<u64> {
        if limit == 0 || limit > 1024 {
            return Err(invalid_delivery());
        }
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT delivery.delivery_id, delivery.claim_id, delivery.attempt_count, \
                    transition.fresh_until <= transaction_timestamp() AS expired, \
                    head.transition_id IS DISTINCT FROM delivery.transition_id AS stale \
             FROM client_status_delivery_outbox delivery \
             JOIN client_status_transitions transition \
               ON transition.community_id=delivery.community_id \
              AND transition.transition_id=delivery.transition_id \
             LEFT JOIN client_status_transition_heads head \
               ON head.community_id=transition.community_id \
              AND head.connection_fingerprint=delivery.connection_fingerprint \
             WHERE delivery.community_id=$1 AND delivery.delivery_state=1 \
               AND (head.transition_id IS DISTINCT FROM delivery.transition_id \
                    OR transition.fresh_until <= transaction_timestamp() \
                    OR delivery.attempt_count >= $2) \
               AND (delivery.claim_id IS NULL \
                    OR delivery.claimed_until <= transaction_timestamp()) \
             ORDER BY delivery.created_at, delivery.delivery_id \
             LIMIT $3 FOR UPDATE OF delivery SKIP LOCKED",
        )
        .bind(community_id.as_uuid())
        .bind(MAX_ATTEMPTS)
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await?;
        for row in &rows {
            let delivery_id: Uuid = row.try_get("delivery_id")?;
            let claim_id: Option<Uuid> = row.try_get("claim_id")?;
            let attempt: i16 = row.try_get("attempt_count")?;
            let expired: bool = row.try_get("expired")?;
            let stale: bool = row.try_get("stale")?;
            if claim_id.is_some() {
                let sequence =
                    next_event_sequence(&mut transaction, community_id, delivery_id).await?;
                insert_delivery_event(
                    &mut transaction,
                    community_id,
                    delivery_id,
                    sequence,
                    8,
                    StatusDeliveryFailure::LeaseExpiredUnknown as i16,
                    attempt,
                )
                .await?;
            }
            let failure = if stale {
                StatusDeliveryFailure::StaleFence
            } else if expired {
                StatusDeliveryFailure::Expired
            } else {
                StatusDeliveryFailure::AttemptsExhausted
            };
            let completion_claim = claim_id.unwrap_or_else(Uuid::new_v4);
            sqlx::query(
                "UPDATE client_status_delivery_outbox SET delivery_state=3, claim_id=NULL, \
                 claimed_until=NULL, completion_claim_id=$3, last_failure_reason=$4, \
                 terminal_at=clock_timestamp() WHERE community_id=$1 AND delivery_id=$2",
            )
            .bind(community_id.as_uuid())
            .bind(delivery_id)
            .bind(completion_claim)
            .bind(failure as i16)
            .execute(&mut *transaction)
            .await?;
            let sequence = next_event_sequence(&mut transaction, community_id, delivery_id).await?;
            insert_delivery_event(
                &mut transaction,
                community_id,
                delivery_id,
                sequence,
                7,
                failure as i16,
                attempt,
            )
            .await?;
        }
        transaction.commit().await?;
        u64::try_from(rows.len()).map_err(|_| invalid_delivery())
    }

    /// Delete terminal delivery evidence only after its strict retention
    /// boundary. Pending work is never reaped.
    pub async fn reap_status_deliveries(
        &self,
        community_id: CommunityId,
        limit: u16,
    ) -> Result<u64> {
        if limit == 0 || limit > 1024 {
            return Err(invalid_delivery());
        }
        let mut transaction = self.pool.begin().await?;
        let delivery_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT delivery_id FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND delivery_state IN (2,3) \
               AND transaction_timestamp() > retain_until \
             ORDER BY retain_until, delivery_id LIMIT $2 FOR UPDATE SKIP LOCKED",
        )
        .bind(community_id.as_uuid())
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await?;
        if !delivery_ids.is_empty() {
            sqlx::query(
                "DELETE FROM client_status_delivery_events \
                 WHERE community_id=$1 AND delivery_id=ANY($2)",
            )
            .bind(community_id.as_uuid())
            .bind(&delivery_ids)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "DELETE FROM client_status_delivery_outbox \
                 WHERE community_id=$1 AND delivery_id=ANY($2)",
            )
            .bind(community_id.as_uuid())
            .bind(&delivery_ids)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "DELETE FROM client_status_transition_heads head USING client_status_transitions transition \
             WHERE head.community_id=$1 AND transition.community_id=head.community_id \
               AND transition.transition_id=head.transition_id \
               AND transition.connection_fingerprint=head.connection_fingerprint \
               AND transaction_timestamp() > transition.retain_until \
               AND NOT EXISTS (SELECT 1 FROM client_status_delivery_outbox delivery \
                   WHERE delivery.community_id=transition.community_id \
                     AND delivery.transition_id=transition.transition_id)",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "DELETE FROM client_status_transitions transition WHERE transition.community_id=$1 \
               AND transaction_timestamp() > transition.retain_until \
               AND NOT EXISTS (SELECT 1 FROM client_status_transition_heads head \
                   WHERE head.community_id=transition.community_id \
                     AND head.transition_id=transition.transition_id) \
               AND NOT EXISTS (SELECT 1 FROM client_status_delivery_outbox delivery \
                   WHERE delivery.community_id=transition.community_id \
                     AND delivery.transition_id=transition.transition_id)",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        u64::try_from(delivery_ids.len()).map_err(|_| invalid_delivery())
    }
}

async fn load_current_status_evidence(
    db: &Db,
    request: &CurrentBindingStatusEvidenceRequest,
) -> Result<CanonicalCurrentBindingEvidence> {
    load_current_status_evidence_at(db, request)
        .await
        .map(|(evidence, _)| evidence)
}

async fn load_current_status_evidence_at(
    db: &Db,
    request: &CurrentBindingStatusEvidenceRequest,
) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>)> {
    let row = sqlx::query(
        "SELECT binding.binding_id, binding.binding_version, binding.policy_revision, \
                domain.current_generation, binding.expires_at, transaction_timestamp() AS now \
         FROM identity_bindings binding \
         JOIN authorization_invalidation_domains domain \
           ON domain.community_id=binding.community_id \
         WHERE binding.community_id=$1 AND binding.event_author_pubkey=$2 \
           AND binding.binding_state=1 AND binding.lifecycle_revision=1 \
           AND (binding.expires_at IS NULL OR binding.expires_at > transaction_timestamp()) \
           AND binding.policy_revision=(SELECT max(policy.policy_revision) \
               FROM identity_enrollment_policies policy WHERE policy.community_id=$1)",
    )
    .bind(request.authorization_domain().as_uuid())
    .bind(request.event_author_pubkey().as_bytes())
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| {
        DbError::InvalidData("current client binding status is unavailable".to_owned())
    })?;
    let binding_id: Uuid = row.try_get("binding_id")?;
    let binding_version = positive_u64(row.try_get("binding_version")?)?;
    let policy_revision = positive_u64(row.try_get("policy_revision")?)?;
    let invalidation_generation = u64::try_from(row.try_get::<i64, _>("current_generation")?)
        .map_err(|_| invalid_delivery())?;
    let authority_epoch = invalidation_generation
        .checked_add(1)
        .ok_or_else(invalid_delivery)?;
    let authoritative_now: DateTime<Utc> = row.try_get("now")?;
    let protocol_fresh_until = authoritative_now
        .checked_add_signed(chrono::Duration::seconds(STATUS_EVIDENCE_LIFETIME_SECONDS))
        .ok_or_else(invalid_delivery)?;
    let binding_expiry: Option<DateTime<Utc>> = row.try_get("expires_at")?;
    let fresh_until = binding_expiry.map_or(protocol_fresh_until, |expiry| {
        expiry.min(protocol_fresh_until)
    });
    let fence = derive_status_evidence_fence(
        request.authorization_domain(),
        request.event_author_pubkey(),
        binding_id,
        binding_version,
        policy_revision,
        invalidation_generation,
        authority_epoch,
    )?;
    let evidence = CanonicalCurrentBindingEvidence::new(
        request.authorization_domain(),
        request.event_author_pubkey(),
        binding_id,
        binding_version,
        policy_revision,
        invalidation_generation,
        authority_epoch,
        fence,
        authoritative_now,
        fresh_until,
    )
    .map_err(|_| invalid_delivery())?;
    Ok((evidence, authoritative_now))
}

async fn recheck_status_evidence_tx(
    transaction: &mut Transaction<'_, Postgres>,
    evidence: &CanonicalCurrentBindingEvidence,
    authoritative_now: DateTime<Utc>,
) -> Result<bool> {
    recheck_connection_status_evidence_tx(transaction, evidence, authoritative_now).await
}

async fn recheck_connection_status_evidence_tx(
    transaction: &mut Transaction<'_, Postgres>,
    evidence: &CanonicalCurrentBindingEvidence,
    authoritative_now: DateTime<Utc>,
) -> Result<bool> {
    let row = sqlx::query(
        "SELECT binding.binding_id,binding.binding_version,binding.policy_revision, \
                domain.current_generation \
         FROM identity_bindings binding \
         JOIN authorization_invalidation_domains domain \
           ON domain.community_id=binding.community_id \
         JOIN identity_enrollment_policies policy \
           ON policy.community_id=binding.community_id \
          AND policy.policy_revision=binding.policy_revision \
         WHERE binding.community_id=$1 AND binding.event_author_pubkey=$2 \
           AND binding.binding_state=1 AND binding.lifecycle_revision=1 \
           AND (binding.expires_at IS NULL OR binding.expires_at > $3) \
           AND policy.effective_at <= $3 \
           AND (policy.expires_at IS NULL OR policy.expires_at > $3) \
           AND NOT EXISTS (SELECT 1 FROM identity_enrollment_policies newer \
               WHERE newer.community_id=binding.community_id \
                 AND newer.policy_revision > binding.policy_revision) \
         FOR SHARE OF binding,domain,policy",
    )
    .bind(evidence.authorization_domain().as_uuid())
    .bind(evidence.event_author_pubkey().as_bytes())
    .bind(authoritative_now)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let binding_id: Uuid = row.try_get("binding_id")?;
    let binding_version = positive_u64(row.try_get("binding_version")?)?;
    let policy_revision = positive_u64(row.try_get("policy_revision")?)?;
    let invalidation_generation = u64::try_from(row.try_get::<i64, _>("current_generation")?)
        .map_err(|_| invalid_delivery())?;
    let authority_epoch = invalidation_generation
        .checked_add(1)
        .ok_or_else(invalid_delivery)?;
    let fence = derive_status_evidence_fence(
        evidence.authorization_domain(),
        evidence.event_author_pubkey(),
        binding_id,
        binding_version,
        policy_revision,
        invalidation_generation,
        authority_epoch,
    )?;
    Ok(evidence.binding_id() == binding_id
        && evidence.binding_version() == binding_version
        && evidence.policy_revision() == policy_revision
        && evidence.invalidation_generation() == invalidation_generation
        && evidence.authority_epoch() == authority_epoch
        && evidence.fence() == fence
        && evidence.is_fresh_at(authoritative_now))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_status_evidence_fence(
    community_id: CommunityId,
    author: PublicKey,
    binding_id: Uuid,
    binding_version: u64,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
) -> Result<AuthorizationLeaseFence> {
    let mut digest = Sha256::new();
    for part in [
        b"buzz:client-status-evidence-fence:v1".as_slice(),
        community_id.as_uuid().as_bytes().as_slice(),
        author.as_bytes().as_slice(),
        binding_id.as_bytes().as_slice(),
        binding_version.to_be_bytes().as_slice(),
        policy_revision.to_be_bytes().as_slice(),
        invalidation_generation.to_be_bytes().as_slice(),
        authority_epoch.to_be_bytes().as_slice(),
    ] {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    AuthorizationLeaseFence::from_bytes(digest.finalize().into()).map_err(|_| invalid_delivery())
}

fn validate_delivery(delivery: &NewStatusDelivery<'_>) -> Result<()> {
    if delivery.delivery_id.is_nil()
        || delivery.transition_id.is_nil()
        || delivery.operation_id.is_nil()
        || delivery.request_fingerprint == [0; 32]
        || delivery.subject_fingerprint == [0; 32]
        || delivery.signer_fingerprint == [0; 32]
        || delivery.connection_fingerprint == [0; 32]
        || delivery.status_revision == 0
        || delivery.status_revision > i64::MAX as u64
        || delivery.signed_payload.is_empty()
        || delivery.signed_payload.len() > 4096
        || (delivery.kind == StatusDeliveryKind::Current && delivery.supersedes_revision.is_some())
        || (delivery.kind == StatusDeliveryKind::Withdrawal
            && delivery.supersedes_revision.is_none())
        || (delivery.kind == StatusDeliveryKind::Current && delivery.current_evidence.is_none())
        || (delivery.kind == StatusDeliveryKind::Withdrawal && delivery.current_evidence.is_some())
    {
        return Err(invalid_delivery());
    }
    if let Some(evidence) = delivery.current_evidence {
        if evidence.authorization_domain() != delivery.community_id
            || evidence.fresh_until() != delivery.fresh_until
            || evidence.binding_version() > i64::MAX as u64
            || evidence.policy_revision() > i64::MAX as u64
            || evidence.invalidation_generation() > i64::MAX as u64
            || evidence.authority_epoch() > i64::MAX as u64
        {
            return Err(invalid_delivery());
        }
    }
    Ok(())
}

fn evidence_row_matches(
    row: &sqlx::postgres::PgRow,
    evidence: Option<&CanonicalCurrentBindingEvidence>,
) -> Result<bool> {
    let author: Option<Vec<u8>> = row.try_get("evidence_author_pubkey")?;
    let binding_id: Option<Uuid> = row.try_get("evidence_binding_id")?;
    let binding_version: Option<i64> = row.try_get("evidence_binding_version")?;
    let policy_revision: Option<i64> = row.try_get("evidence_policy_revision")?;
    let invalidation_generation: Option<i64> = row.try_get("evidence_invalidation_generation")?;
    let authority_epoch: Option<i64> = row.try_get("evidence_authority_epoch")?;
    let fence: Option<Vec<u8>> = row.try_get("evidence_fence")?;
    let observed_at: Option<DateTime<Utc>> = row.try_get("evidence_observed_at")?;
    Ok(match evidence {
        Some(value) => {
            author.as_deref() == Some(value.event_author_pubkey().as_bytes())
                && binding_id == Some(value.binding_id())
                && binding_version == i64::try_from(value.binding_version()).ok()
                && policy_revision == i64::try_from(value.policy_revision()).ok()
                && invalidation_generation == i64::try_from(value.invalidation_generation()).ok()
                && authority_epoch == i64::try_from(value.authority_epoch()).ok()
                && fence.as_deref() == Some(value.fence().as_bytes().as_slice())
                && observed_at == Some(value.observed_at())
        }
        None => {
            author.is_none()
                && binding_id.is_none()
                && binding_version.is_none()
                && policy_revision.is_none()
                && invalidation_generation.is_none()
                && authority_epoch.is_none()
                && fence.is_none()
                && observed_at.is_none()
        }
    })
}

fn evidence_from_row(
    row: &sqlx::postgres::PgRow,
    community_id: CommunityId,
    kind: StatusDeliveryKind,
) -> Result<Option<CanonicalCurrentBindingEvidence>> {
    if kind == StatusDeliveryKind::Withdrawal {
        if !evidence_row_matches(row, None)? {
            return Err(invalid_delivery());
        }
        return Ok(None);
    }
    let author_bytes: Vec<u8> = row
        .try_get::<Option<Vec<u8>>, _>("evidence_author_pubkey")?
        .ok_or_else(invalid_delivery)?;
    let author = PublicKey::from_slice(&author_bytes).map_err(|_| invalid_delivery())?;
    let binding_id = row
        .try_get::<Option<Uuid>, _>("evidence_binding_id")?
        .ok_or_else(invalid_delivery)?;
    let binding_version = positive_u64(
        row.try_get::<Option<i64>, _>("evidence_binding_version")?
            .ok_or_else(invalid_delivery)?,
    )?;
    let policy_revision = positive_u64(
        row.try_get::<Option<i64>, _>("evidence_policy_revision")?
            .ok_or_else(invalid_delivery)?,
    )?;
    let invalidation_generation = u64::try_from(
        row.try_get::<Option<i64>, _>("evidence_invalidation_generation")?
            .ok_or_else(invalid_delivery)?,
    )
    .map_err(|_| invalid_delivery())?;
    let authority_epoch = positive_u64(
        row.try_get::<Option<i64>, _>("evidence_authority_epoch")?
            .ok_or_else(invalid_delivery)?,
    )?;
    let fence = AuthorizationLeaseFence::from_bytes(fixed_digest(
        row.try_get::<Option<Vec<u8>>, _>("evidence_fence")?
            .ok_or_else(invalid_delivery)?,
    )?)
    .map_err(|_| invalid_delivery())?;
    let observed_at = row
        .try_get::<Option<DateTime<Utc>>, _>("evidence_observed_at")?
        .ok_or_else(invalid_delivery)?;
    let fresh_until: DateTime<Utc> = row.try_get("fresh_until")?;
    CanonicalCurrentBindingEvidence::new(
        community_id,
        author,
        binding_id,
        binding_version,
        policy_revision,
        invalidation_generation,
        authority_epoch,
        fence,
        observed_at,
        fresh_until,
    )
    .map(Some)
    .map_err(|_| invalid_delivery())
}

fn fixed_digest(bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| invalid_delivery())
}

fn positive_u64(value: i64) -> Result<u64> {
    let value = u64::try_from(value).map_err(|_| invalid_delivery())?;
    if value == 0 {
        Err(invalid_delivery())
    } else {
        Ok(value)
    }
}

async fn insert_delivery_target(
    transaction: &mut Transaction<'_, Postgres>,
    delivery: &NewStatusDelivery<'_>,
    new_transition: bool,
) -> Result<bool> {
    let inserted = sqlx::query(
        "INSERT INTO client_status_delivery_outbox \
         (community_id, delivery_id, transition_id, connection_fingerprint) \
         VALUES ($1,$2,$3,$4) \
         ON CONFLICT (community_id, transition_id, connection_fingerprint) DO NOTHING \
         RETURNING delivery_id",
    )
    .bind(delivery.community_id.as_uuid())
    .bind(delivery.delivery_id)
    .bind(delivery.transition_id)
    .bind(delivery.connection_fingerprint.as_slice())
    .fetch_optional(&mut **transaction)
    .await?
    .is_some();
    if inserted {
        let event_kinds: &[i16] = if new_transition { &[1, 2, 3] } else { &[3] };
        for (index, event_kind) in event_kinds.iter().enumerate() {
            insert_delivery_event(
                transaction,
                delivery.community_id,
                delivery.delivery_id,
                i16::try_from(index + 1).map_err(|_| invalid_delivery())?,
                *event_kind,
                0,
                0,
            )
            .await?;
        }
    }
    Ok(inserted)
}

fn invalid_delivery() -> DbError {
    DbError::InvalidData("client status delivery is invalid".to_owned())
}

fn bounded_seconds(value: Duration, maximum: u64) -> Result<u64> {
    let seconds = value.as_secs();
    if value.subsec_nanos() != 0 || seconds == 0 || seconds > maximum {
        return Err(invalid_delivery());
    }
    Ok(seconds)
}

fn delivery_kind(value: i16) -> Result<StatusDeliveryKind> {
    match value {
        1 => Ok(StatusDeliveryKind::Current),
        2 => Ok(StatusDeliveryKind::Withdrawal),
        _ => Err(invalid_delivery()),
    }
}

async fn delivery_state_for_update<'a>(
    transaction: &mut Transaction<'a, Postgres>,
    community_id: CommunityId,
    delivery_id: Uuid,
) -> Result<Option<sqlx::postgres::PgRow>> {
    Ok(sqlx::query(
        "SELECT delivery_state, claim_id, completion_claim_id, claimed_until, attempt_count, \
                last_failure_reason \
         FROM client_status_delivery_outbox WHERE community_id=$1 AND delivery_id=$2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(delivery_id)
    .fetch_optional(&mut **transaction)
    .await?)
}

async fn next_event_sequence(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    delivery_id: Uuid,
) -> Result<i16> {
    let sequence: i16 = sqlx::query_scalar(
        "SELECT (COALESCE(max(event_sequence), 0) + 1)::smallint \
         FROM client_status_delivery_events WHERE community_id=$1 AND delivery_id=$2",
    )
    .bind(community_id.as_uuid())
    .bind(delivery_id)
    .fetch_one(&mut **transaction)
    .await?;
    if !(1..=64).contains(&sequence) {
        return Err(invalid_delivery());
    }
    Ok(sequence)
}

#[allow(clippy::too_many_arguments)]
async fn insert_delivery_event(
    transaction: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    delivery_id: Uuid,
    sequence: i16,
    event_kind: i16,
    reason: i16,
    attempt: i16,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO client_status_delivery_events \
         (community_id, delivery_id, event_sequence, event_kind, reason_code, attempt_count) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(community_id.as_uuid())
    .bind(delivery_id)
    .bind(sequence)
    .bind(event_kind)
    .bind(reason)
    .bind(attempt)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Minimal audit view used by tests and operator tooling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusDeliveryAuditEvent {
    /// Stable sequence within one delivery.
    pub sequence: u16,
    /// Closed event kind: allocated, signed, fenced, claimed, delivered,
    /// retryable failure, or terminal failure.
    pub kind: u16,
    /// Closed failure reason code; zero on successful stages.
    pub reason: u16,
    /// Bounded attempt count.
    pub attempt: u16,
    /// Database-owned observation time.
    pub occurred_at: DateTime<Utc>,
}

impl Db {
    /// Read the bounded, redaction-safe causal timeline for one delivery.
    pub async fn status_delivery_audit(
        &self,
        community_id: CommunityId,
        delivery_id: Uuid,
    ) -> Result<Vec<StatusDeliveryAuditEvent>> {
        let rows = sqlx::query(
            "SELECT event_sequence, event_kind, reason_code, attempt_count, occurred_at \
             FROM client_status_delivery_events WHERE community_id=$1 AND delivery_id=$2 \
             ORDER BY event_sequence",
        )
        .bind(community_id.as_uuid())
        .bind(delivery_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StatusDeliveryAuditEvent {
                    sequence: u16::try_from(row.try_get::<i16, _>("event_sequence")?)
                        .map_err(|_| invalid_delivery())?,
                    kind: u16::try_from(row.try_get::<i16, _>("event_kind")?)
                        .map_err(|_| invalid_delivery())?,
                    reason: u16::try_from(row.try_get::<i16, _>("reason_code")?)
                        .map_err(|_| invalid_delivery())?,
                    attempt: u16::try_from(row.try_get::<i16, _>("attempt_count")?)
                        .map_err(|_| invalid_delivery())?,
                    occurred_at: row.try_get("occurred_at")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    async fn seed_status_binding(pool: &sqlx::PgPool, community_id: CommunityId) -> nostr::Keys {
        let author = nostr::Keys::generate();
        let operation_id = Uuid::new_v4();
        let history_id = Uuid::new_v4();
        let binding_id = Uuid::new_v4();
        let request_fingerprint = vec![41_u8; 32];
        let mut transaction = pool.begin().await.expect("begin status binding seed");
        sqlx::query("SET LOCAL session_replication_role = replica")
            .execute(&mut *transaction)
            .await
            .expect("isolate resolver fixture from lifecycle triggers");
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
              outcome_code,result_digest) VALUES ($1,$2,$3,1,$4,1,$5)",
        )
        .bind(community_id.as_uuid())
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(vec![42_u8; 32])
        .bind(vec![43_u8; 32])
        .execute(&mut *transaction)
        .await
        .expect("insert status binding receipt");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,'https://issuer.example','status-subject',$3,$4,1,1,2,1,$5,$6,$7,$8) \
             RETURNING binding_version",
        )
        .bind(community_id.as_uuid())
        .bind(binding_id)
        .bind(vec![44_u8; 32])
        .bind(author.public_key().as_bytes())
        .bind(vec![45_u8; 32])
        .bind(history_id)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .fetch_one(&mut *transaction)
        .await
        .expect("insert status binding");
        sqlx::query(
            "INSERT INTO identity_lifecycle_history \
             (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
              successor_binding_version,successor_lifecycle_revision,successor_state, \
              operation_id,request_fingerprint,transition_digest) \
             VALUES ($1,$2,1,1,$3,$4,1,1,$5,$6,$7)",
        )
        .bind(community_id.as_uuid())
        .bind(history_id)
        .bind(binding_id)
        .bind(binding_version)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(vec![46_u8; 32])
        .execute(&mut *transaction)
        .await
        .expect("insert status binding history");
        transaction
            .commit()
            .await
            .expect("commit status binding seed");
        author
    }

    #[test]
    fn debug_output_redacts_delivery_material() {
        let community_id = CommunityId::from_uuid(Uuid::from_u128(1));
        let delivery = NewStatusDelivery {
            community_id,
            delivery_id: Uuid::from_u128(2),
            transition_id: Uuid::from_u128(3),
            operation_id: Uuid::from_u128(4),
            request_fingerprint: [5; 32],
            kind: StatusDeliveryKind::Current,
            subject_fingerprint: [6; 32],
            signer_fingerprint: [7; 32],
            connection_fingerprint: [8; 32],
            status_revision: 9,
            supersedes_revision: None,
            fresh_until: Utc::now() + chrono::Duration::minutes(5),
            signed_payload: b"secret-payload",
            current_evidence: None,
        };
        assert_eq!(format!("{delivery:?}"), "NewStatusDelivery([REDACTED])");
    }

    #[test]
    fn durations_and_envelopes_are_bounded() {
        assert!(bounded_seconds(Duration::ZERO, MAX_CLAIM_SECONDS).is_err());
        assert!(bounded_seconds(Duration::from_millis(1), MAX_CLAIM_SECONDS).is_err());
        assert!(bounded_seconds(Duration::from_secs(301), MAX_CLAIM_SECONDS).is_err());
        assert_eq!(
            bounded_seconds(Duration::from_secs(30), MAX_CLAIM_SECONDS).expect("bounded seconds"),
            30
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn postgres_status_delivery_is_atomic_target_bound_and_crash_recoverable() {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .expect("BUZZ_TEST_DATABASE_URL must name a disposable PostgreSQL database");
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect disposable PostgreSQL");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let db = Db::from_pool(pool.clone());
        let community_id = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community_id.as_uuid())
            .bind(format!("{}.example.test", community_id.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");
        db.install_status_delivery_capacity(community_id)
            .await
            .expect("install capacity");

        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,2,$2,transaction_timestamp())",
        )
        .bind(community_id.as_uuid())
        .bind(vec![40_u8; 32])
        .execute(&pool)
        .await
        .expect("insert status policy");
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(community_id.as_uuid())
        .execute(&pool)
        .await
        .expect("activate status invalidation domain");
        let resolver_keys = seed_status_binding(&pool, community_id).await;
        let resolver_author = resolver_keys.public_key();
        let resolver_request =
            CurrentBindingStatusEvidenceRequest::new(community_id, resolver_author)
                .expect("status resolver request");
        let resolver_evidence = db
            .current_status_evidence(&resolver_request)
            .await
            .expect("resolve current status evidence");
        let (exact_recheck, recheck_now) = db
            .recheck_current_status_evidence(&resolver_evidence)
            .await
            .expect("authoritative status recheck");
        assert!(resolver_evidence.accepts_exact_recheck(&exact_recheck, recheck_now));
        let relay_keys = nostr::Keys::generate();
        let challenge = buzz_auth::generate_challenge();
        let relay_url = "wss://status.example.test/relay";
        let scope_epoch = Uuid::new_v4().to_string();
        let relay_signer = relay_keys.public_key().to_hex();
        let auth_event = nostr::EventBuilder::auth(
            &challenge,
            nostr::RelayUrl::parse(relay_url).expect("relay URL"),
        )
        .tag(
            nostr::Tag::parse([
                buzz_core::client_binding_bootstrap::CLIENT_BINDING_SCOPE_TAG,
                "1",
                scope_epoch.as_str(),
                relay_signer.as_str(),
            ])
            .expect("status scope"),
        )
        .sign_with_keys(&resolver_keys)
        .expect("sign status AUTH");
        let connection_id = Uuid::new_v4();
        let coordinates = buzz_auth::Nip42BindingStatusCoordinates::new(
            community_id,
            relay_url,
            connection_id,
            relay_keys.public_key(),
            &auth_event,
        )
        .expect("status proof coordinates");
        let proof = buzz_auth::verify_nip42_binding_status_proof(
            &auth_event,
            &challenge,
            &coordinates,
            db.status_delivery_authoritative_now()
                .await
                .expect("authoritative proof time"),
        )
        .expect("purpose-sealed status proof");
        let finalized = db
            .finalize_binding_status_authorization(proof)
            .await
            .expect("finalize status authorization");
        assert_eq!(
            finalized.lease().capability(),
            RouteCapability::BindingStatus
        );
        assert_eq!(finalized.lease().actor_pubkey(), resolver_author);
        assert_eq!(
            finalized.lease().request_binding().1,
            &coordinates.target_fingerprint()
        );
        let fresh_until = resolver_evidence.fresh_until();
        let evidence = resolver_evidence.clone();
        let current = NewStatusDelivery {
            community_id,
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [1; 32],
            kind: StatusDeliveryKind::Current,
            subject_fingerprint: [2; 32],
            signer_fingerprint: [3; 32],
            connection_fingerprint: [4; 32],
            status_revision: 1,
            supersedes_revision: None,
            fresh_until,
            signed_payload: br#"{"kind":24244,"revision":1}"#,
            current_evidence: Some(&evidence),
        };

        let mut rolled_back = pool.begin().await.expect("begin rollback transaction");
        assert_eq!(
            enqueue_status_delivery_tx(&mut rolled_back, &current)
                .await
                .expect("stage rolled back status"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        rolled_back.rollback().await.expect("rollback status");
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM client_status_transitions WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count rolled back transitions");
        assert_eq!(rows, 0);
        let pending_after_rollback: i32 = sqlx::query_scalar(
            "SELECT pending_count FROM client_status_delivery_capacity WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read capacity after rollback");
        assert_eq!(pending_after_rollback, 0);

        assert_eq!(
            db.enqueue_status_delivery(&current)
                .await
                .expect("enqueue current"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        assert_eq!(
            db.enqueue_status_delivery(&current)
                .await
                .expect("replay current"),
            EnqueueStatusDeliveryOutcome::ExactReplay
        );
        let conflicting_delivery_id = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            ..current
        };
        assert!(db
            .enqueue_status_delivery(&conflicting_delivery_id)
            .await
            .is_err());

        let first = db
            .claim_status_delivery(
                community_id,
                current.connection_fingerprint,
                Duration::from_secs(3),
            )
            .await
            .expect("claim current")
            .expect("current ready");
        assert_eq!(first.signed_payload(), current.signed_payload);
        let first_authorization = db
            .authorize_status_delivery(&first)
            .await
            .expect("authorize current before writer I/O")
            .expect("current claim is sendable");
        assert!(first_authorization.write_budget() > Duration::ZERO);
        assert!(first_authorization.write_budget() <= Duration::from_secs(2));
        assert_eq!(first_authorization.payload_digest(), first.payload_digest());
        let mut overwrite_transaction = pool.begin().await.expect("begin overwrite probe");
        let overwrite = sqlx::query(
            "UPDATE client_status_delivery_outbox SET claim_id=$3, \
             claimed_until=transaction_timestamp()+INTERVAL '30 seconds', \
             attempt_count=attempt_count+1, attempt_started_at=clock_timestamp() \
             WHERE community_id=$1 AND delivery_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(first.delivery_id())
        .bind(Uuid::new_v4())
        .execute(&mut *overwrite_transaction);
        assert!(tokio::time::timeout(Duration::from_millis(100), overwrite)
            .await
            .is_err());
        let mut invalidation_transaction = pool.begin().await.expect("begin invalidation probe");
        let invalidate = sqlx::query(
            "UPDATE authorization_invalidation_domains \
             SET current_generation=current_generation+1 WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *invalidation_transaction);
        assert!(tokio::time::timeout(Duration::from_millis(100), invalidate)
            .await
            .is_err());
        let independent_community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(independent_community.as_uuid())
            .bind(format!(
                "{}.independent.test",
                independent_community.as_uuid()
            ))
            .execute(&pool)
            .await
            .expect("insert independent tenant");
        let independent_policy = sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,2,$2,transaction_timestamp())",
        )
        .bind(independent_community.as_uuid())
        .bind(vec![47_u8; 32])
        .execute(&pool);
        tokio::time::timeout(Duration::from_secs(1), independent_policy)
            .await
            .expect("other tenant policy is not blocked")
            .expect("insert other tenant policy");
        let mut policy_transaction = pool.begin().await.expect("begin policy probe");
        let same_tenant_policy = sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,2,2,$2,transaction_timestamp())",
        )
        .bind(community_id.as_uuid())
        .bind(vec![48_u8; 32])
        .execute(&mut *policy_transaction);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), same_tenant_policy)
                .await
                .is_err()
        );
        db.abort_status_delivery_authorization(first_authorization)
            .await
            .expect("release abandoned writer fence");
        let _ = overwrite_transaction.rollback().await;
        let _ = invalidation_transaction.rollback().await;
        let _ = policy_transaction.rollback().await;
        let generation: i64 = sqlx::query_scalar(
            "SELECT current_generation FROM authorization_invalidation_domains \
             WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read generation after blocked invalidation");
        assert_eq!(generation, 0);
        let policy_revision_two_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM identity_enrollment_policies \
             WHERE community_id=$1 AND policy_revision=2)",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read blocked policy revision");
        assert!(!policy_revision_two_exists);
        tokio::time::sleep(Duration::from_millis(3100)).await;
        assert!(db
            .authorize_status_delivery(&first)
            .await
            .expect("expired claim authorization query")
            .is_none());
        let replay = db
            .claim_status_delivery(
                community_id,
                current.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("reclaim after crash")
            .expect("crashed current ready");
        assert_eq!(replay.signed_payload(), first.signed_payload());
        assert_eq!(replay.attempt(), 2);
        let mut replay_authorization = db
            .authorize_status_delivery(&replay)
            .await
            .expect("authorize reclaimed delivery before writer I/O")
            .expect("reclaimed delivery is sendable");
        assert_eq!(
            db.complete_status_delivery(&mut replay_authorization)
                .await
                .expect("complete replay"),
            CompleteStatusDeliveryOutcome::Delivered
        );
        assert_eq!(
            db.complete_status_delivery(&mut replay_authorization)
                .await
                .expect("replay completion"),
            CompleteStatusDeliveryOutcome::ExactReplay
        );
        let detached = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [13; 32],
            connection_fingerprint: [13; 32],
            ..current
        };
        db.enqueue_status_delivery(&detached)
            .await
            .expect("enqueue detached writer delivery");
        let detached_claim = db
            .claim_status_delivery(
                community_id,
                detached.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("claim detached writer delivery")
            .expect("detached writer delivery ready");
        let mut detached_authorization = db
            .authorize_status_delivery(&detached_claim)
            .await
            .expect("authorize detached writer delivery")
            .expect("detached writer delivery sendable");
        assert!(detached_authorization.write_budget() <= Duration::from_secs(5));
        let (release_detached, detached_released) = tokio::sync::oneshot::channel();
        let detached_db = db.clone();
        let detached_task = tokio::spawn(async move {
            detached_released.await.expect("release detached writer");
            detached_db
                .complete_status_delivery(&mut detached_authorization)
                .await
                .expect("complete detached writer")
        });
        drop(detached_task);
        let mut detached_invalidation = pool
            .begin()
            .await
            .expect("begin detached invalidation probe");
        let blocked_invalidation = sqlx::query(
            "UPDATE authorization_invalidation_domains \
             SET current_generation=current_generation+1 WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *detached_invalidation);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), blocked_invalidation)
                .await
                .is_err()
        );
        release_detached.send(()).expect("release detached task");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let delivered: bool = sqlx::query_scalar(
                    "SELECT delivery_state=2 FROM client_status_delivery_outbox \
                     WHERE community_id=$1 AND delivery_id=$2",
                )
                .bind(community_id.as_uuid())
                .bind(detached.delivery_id)
                .fetch_one(&pool)
                .await
                .expect("read detached delivery state");
                if delivered {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached authorization completed");
        let _ = detached_invalidation.rollback().await;
        let audit = db
            .status_delivery_audit(community_id, current.delivery_id)
            .await
            .expect("read delivery audit");
        assert_eq!(
            audit.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 8, 4, 5]
        );
        assert_eq!(
            audit[4].reason,
            StatusDeliveryFailure::LeaseExpiredUnknown as u16
        );

        // A reconnect is a new S5 session: its durable head is scoped to the
        // new exact connection and its wire revision starts at one again.
        let reconnect = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [5; 32],
            connection_fingerprint: [5; 32],
            ..current
        };
        assert_eq!(
            db.enqueue_status_delivery(&reconnect)
                .await
                .expect("enqueue reconnect session"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        let reconnect_claim = db
            .claim_status_delivery(
                community_id,
                reconnect.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("claim reconnect")
            .expect("reconnect ready");
        assert_eq!(reconnect_claim.signed_payload(), reconnect.signed_payload);
        let mut reconnect_authorization = db
            .authorize_status_delivery(&reconnect_claim)
            .await
            .expect("authorize reconnect before writer I/O")
            .expect("reconnect is sendable");

        let parallel = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [8; 32],
            connection_fingerprint: [8; 32],
            ..current
        };
        assert_eq!(
            db.enqueue_status_delivery(&parallel)
                .await
                .expect("enqueue parallel session"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        let renewal = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [10; 32],
            status_revision: 2,
            ..current
        };
        assert_eq!(
            db.enqueue_status_delivery(&renewal)
                .await
                .expect("enqueue renewal"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        let renewal_claim = db
            .claim_status_delivery(
                community_id,
                renewal.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("claim renewal before supersession")
            .expect("renewal ready");

        let withdrawal = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [6; 32],
            kind: StatusDeliveryKind::Withdrawal,
            status_revision: 3,
            supersedes_revision: Some(2),
            signed_payload: br#"{"kind":24244,"revision":3,"withdrawn":true}"#,
            current_evidence: None,
            ..current
        };
        assert_eq!(
            db.enqueue_status_delivery(&withdrawal)
                .await
                .expect("enqueue withdrawal"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        let withdrawal_pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND delivery_id=$2 AND delivery_state=1)",
        )
        .bind(community_id.as_uuid())
        .bind(withdrawal.delivery_id)
        .fetch_one(&pool)
        .await
        .expect("withdrawal pending query");
        assert!(withdrawal_pending);
        assert!(db
            .authorize_status_delivery(&renewal_claim)
            .await
            .expect("reject superseded claim before writer I/O")
            .is_none());
        assert_eq!(
            db.complete_status_delivery(&mut reconnect_authorization)
                .await
                .expect("record flush authorized before supersession"),
            CompleteStatusDeliveryOutcome::Delivered
        );
        assert_eq!(
            db.reconcile_status_deliveries(community_id, 16)
                .await
                .expect("terminalize stale pending delivery"),
            0
        );
        let stale_audit = db
            .status_delivery_audit(community_id, renewal.delivery_id)
            .await
            .expect("read stale target audit");
        assert_eq!(stale_audit.last().expect("stale event").reason, 3);
        let stale_target = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            ..renewal
        };
        assert!(db.enqueue_status_delivery(&stale_target).await.is_err());

        // A withdrawal on one connection must not stale another socket for
        // the same author; the parallel session retains its own revision-one
        // head and remains claimable.
        let parallel_claim = db
            .claim_status_delivery(
                community_id,
                parallel.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("parallel claim query")
            .expect("parallel delivery ready");
        let mut parallel_authorization = db
            .authorize_status_delivery(&parallel_claim)
            .await
            .expect("authorize parallel connection")
            .expect("parallel authorization");
        assert_eq!(
            db.complete_status_delivery(&mut parallel_authorization)
                .await
                .expect("complete parallel connection"),
            CompleteStatusDeliveryOutcome::Delivered
        );
        let abandoned = NewStatusDelivery {
            delivery_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            request_fingerprint: [12; 32],
            connection_fingerprint: [12; 32],
            ..current
        };
        assert_eq!(
            db.enqueue_status_delivery(&abandoned)
                .await
                .expect("enqueue abandoned connection"),
            EnqueueStatusDeliveryOutcome::Enqueued
        );
        let abandoned_claim = db
            .claim_status_delivery(
                community_id,
                abandoned.connection_fingerprint,
                Duration::from_secs(30),
            )
            .await
            .expect("claim abandoned connection")
            .expect("abandoned connection ready");
        let abandoned_authorization = db
            .authorize_status_delivery(&abandoned_claim)
            .await
            .expect("authorize abandoned connection")
            .expect("abandoned connection authorization");
        db.abort_status_delivery_authorization(abandoned_authorization)
            .await
            .expect("settle failed abandoned write");
        assert_eq!(
            db.terminalize_status_connection(community_id, abandoned.connection_fingerprint)
                .await
                .expect("terminalize abandoned connection"),
            1
        );
        let abandoned_audit = db
            .status_delivery_audit(community_id, abandoned.delivery_id)
            .await
            .expect("abandoned connection audit");
        assert_eq!(
            abandoned_audit.last().expect("terminal event").reason,
            StatusDeliveryFailure::ConnectionGone as u16
        );

        let direct_reopen = sqlx::query(
            "UPDATE client_status_delivery_outbox SET delivery_state=1, delivered_at=NULL, \
             completion_claim_id=NULL WHERE community_id=$1 AND delivery_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.delivery_id)
        .execute(&pool)
        .await;
        assert!(direct_reopen.is_err());

        let other_community_id = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(other_community_id.as_uuid())
            .bind(format!("{}.example.test", other_community_id.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert isolated community");
        db.install_status_delivery_capacity(other_community_id)
            .await
            .expect("install isolated capacity");
        assert!(db
            .claim_status_delivery(
                other_community_id,
                withdrawal.connection_fingerprint,
                Duration::from_secs(1),
            )
            .await
            .expect("cross-tenant claim query")
            .is_none());
        assert!(db
            .status_delivery_audit(other_community_id, withdrawal.delivery_id)
            .await
            .expect("cross-tenant audit query")
            .is_empty());
        assert_eq!(
            db.reap_status_deliveries(other_community_id, 16)
                .await
                .expect("cross-tenant reap"),
            0
        );

        let pending_before_exhaustion: i32 = sqlx::query_scalar(
            "SELECT pending_count FROM client_status_delivery_capacity WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read pending count before exhaustion");
        let pending_before_rows: Vec<(Uuid, i16)> = sqlx::query_as(
            "SELECT delivery_id, attempt_count FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND delivery_state=1 ORDER BY delivery_id",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("read pending rows before exhaustion");
        assert_eq!(
            pending_before_exhaustion, 1,
            "pending rows before exhaustion: {pending_before_rows:?}"
        );

        for expected_attempt in 1..=15 {
            let claim = db
                .claim_status_delivery(
                    community_id,
                    withdrawal.connection_fingerprint,
                    Duration::from_secs(1),
                )
                .await
                .expect("claim bounded retry")
                .expect("bounded retry is ready");
            assert_eq!(claim.attempt(), expected_attempt);
            assert_eq!(
                db.fail_status_delivery(
                    community_id,
                    claim.delivery_id(),
                    claim.claim_id(),
                    StatusDeliveryFailure::Transient,
                    Duration::from_secs(1),
                )
                .await
                .expect("schedule bounded retry"),
                FailStatusDeliveryOutcome::RetryScheduled
            );
            tokio::time::sleep(Duration::from_millis(1050)).await;
        }
        let final_claim = db
            .claim_status_delivery(
                community_id,
                withdrawal.connection_fingerprint,
                Duration::from_secs(1),
            )
            .await
            .expect("claim final bounded attempt")
            .expect("final bounded attempt is ready");
        assert_eq!(final_claim.attempt(), 16);
        assert_eq!(
            db.reconcile_status_deliveries(community_id, 16)
                .await
                .expect("preserve unexpired final claim"),
            0
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            db.reconcile_status_deliveries(community_id, 16)
                .await
                .expect("terminalize expired final claim"),
            1
        );
        let exhausted_audit = db
            .status_delivery_audit(community_id, withdrawal.delivery_id)
            .await
            .expect("read exhausted audit");
        assert_eq!(
            exhausted_audit[exhausted_audit.len() - 2].reason,
            StatusDeliveryFailure::LeaseExpiredUnknown as u16
        );
        assert_eq!(
            exhausted_audit.last().expect("exhausted event").reason,
            StatusDeliveryFailure::AttemptsExhausted as u16
        );
        let pending_count: i32 = sqlx::query_scalar(
            "SELECT pending_count FROM client_status_delivery_capacity WHERE community_id=$1",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read bounded pending count");
        let pending_deliveries: Vec<(Uuid, i16, Option<Uuid>)> = sqlx::query_as(
            "SELECT delivery_id, attempt_count, claim_id \
             FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND delivery_state=1 ORDER BY delivery_id",
        )
        .bind(community_id.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("read pending delivery identities");
        assert_eq!(
            pending_count, 0,
            "pending deliveries: {pending_deliveries:?}"
        );
        assert!(pending_deliveries.is_empty());

        let mut equality = pool.begin().await.expect("begin retention equality proof");
        sqlx::query(
            "ALTER TABLE client_status_delivery_outbox \
             DISABLE TRIGGER client_status_delivery_state",
        )
        .execute(&mut *equality)
        .await
        .expect("disable state guard for equality fixture");
        sqlx::query(
            "UPDATE client_status_delivery_outbox SET retain_until=transaction_timestamp() \
             WHERE community_id=$1 AND delivery_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.delivery_id)
        .execute(&mut *equality)
        .await
        .expect("set exact retention boundary");
        sqlx::query(
            "ALTER TABLE client_status_delivery_outbox \
             ENABLE TRIGGER client_status_delivery_state",
        )
        .execute(&mut *equality)
        .await
        .expect("restore state guard before equality delete");
        let equality_delete = sqlx::query(
            "DELETE FROM client_status_delivery_outbox \
             WHERE community_id=$1 AND delivery_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.delivery_id)
        .execute(&mut *equality)
        .await;
        assert!(equality_delete.is_err());
        equality
            .rollback()
            .await
            .expect("rollback exact-boundary transaction");

        let mut expired = pool.begin().await.expect("begin strict-after fixture");
        sqlx::query(
            "ALTER TABLE client_status_delivery_outbox \
             DISABLE TRIGGER client_status_delivery_state",
        )
        .execute(&mut *expired)
        .await
        .expect("disable state guard for expired fixture");
        sqlx::query(
            "UPDATE client_status_delivery_outbox SET \
             retain_until=transaction_timestamp()-INTERVAL '1 microsecond' \
             WHERE community_id=$1 AND transition_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.transition_id)
        .execute(&mut *expired)
        .await
        .expect("expire old target evidence");
        sqlx::query(
            "ALTER TABLE client_status_delivery_outbox \
             ENABLE TRIGGER client_status_delivery_state",
        )
        .execute(&mut *expired)
        .await
        .expect("restore state guard before reaping");
        sqlx::query(
            "ALTER TABLE client_status_transitions \
             DISABLE TRIGGER client_status_transition_no_update",
        )
        .execute(&mut *expired)
        .await
        .expect("disable transition immutability for expired fixture");
        sqlx::query(
            "UPDATE client_status_transitions SET \
             allocated_at=transaction_timestamp()-INTERVAL '5 microseconds', \
             signed_at=transaction_timestamp()-INTERVAL '4 microseconds', \
             fenced_at=transaction_timestamp()-INTERVAL '3 microseconds', \
             fresh_until=transaction_timestamp()-INTERVAL '2 microseconds', \
             retain_until=transaction_timestamp()-INTERVAL '1 microsecond' \
             WHERE community_id=$1 AND transition_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.transition_id)
        .execute(&mut *expired)
        .await
        .expect("expire old transition evidence");
        sqlx::query(
            "ALTER TABLE client_status_transitions \
             ENABLE TRIGGER client_status_transition_no_update",
        )
        .execute(&mut *expired)
        .await
        .expect("restore transition immutability before reaping");
        expired.commit().await.expect("commit strict-after fixture");
        assert_eq!(
            db.reap_status_deliveries(community_id, 16)
                .await
                .expect("reap strict-after evidence"),
            1
        );
        let old_transition_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM client_status_transitions \
             WHERE community_id=$1 AND transition_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(current.transition_id)
        .fetch_one(&pool)
        .await
        .expect("count compacted transition");
        assert_eq!(old_transition_count, 0);

        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,2,2,$2,transaction_timestamp())",
        )
        .bind(community_id.as_uuid())
        .bind(vec![47_u8; 32])
        .execute(&pool)
        .await
        .expect("advance status policy");
        assert!(db
            .recheck_current_status_evidence(&resolver_evidence)
            .await
            .is_err());
    }
}
