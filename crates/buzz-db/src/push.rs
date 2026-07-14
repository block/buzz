//! Community-scoped NIP-PL lease and durable wake-outbox persistence.
//!
//! Every operation requires a server-resolved [`CommunityId`]. Client-provided
//! origins never select rows in this module.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::Result;

/// Maximum claims for a malformed matcher job before it is discarded.
pub const MAX_MATCH_ATTEMPTS: i32 = 8;

/// Common signed-event ordering fields for a lease replacement.
#[derive(Debug, Clone, Copy)]
pub struct LeaseVersion<'a> {
    /// Signed kind:30350 event id (32 bytes).
    pub source_event_id: &'a [u8],
    /// Signed event `created_at`, in Unix seconds.
    pub source_created_at: i64,
    /// Strictly increasing installation generation.
    pub generation: i64,
    /// Public NIP-40 expiration, in Unix seconds.
    pub expires_at: i64,
}

/// Effective fields for an active APNs lease.
#[derive(Debug, Clone, Copy)]
pub struct ActiveLease<'a> {
    /// Application profile selected from the executor descriptor.
    pub app_profile: &'a str,
    /// SHA-256 of the platform endpoint.
    pub endpoint_hash: &'a [u8],
    /// Opaque endpoint grant issued by the stateless gateway.
    pub endpoint_grant: &'a str,
    /// Highest delivery class this lease permits.
    pub max_class: &'a str,
    /// Validated subscription array stored for matching.
    pub subscriptions: &'a Value,
}

/// Result of applying a lease replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceLeaseOutcome {
    /// The replacement became the effective lease state.
    Accepted,
    /// The signed event did not win NIP-01 addressable-event ordering.
    StaleEvent,
    /// The generation did not exceed the persisted watermark.
    StaleGeneration,
}

/// Result of an idempotent outbox enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueWakeOutcome {
    /// A new durable job was inserted.
    Enqueued(Uuid),
    /// The endpoint/event dedup key already had a durable job.
    Duplicate(Uuid),
    /// No current active, unexpired lease matched the supplied generation.
    InactiveLease,
}

/// Durable wake fields not copied from the effective lease.
#[derive(Debug, Clone, Copy)]
pub struct NewWake<'a> {
    /// Generation observed by the matcher.
    pub lease_generation: i64,
    /// Accepted event id that caused the wake (32 bytes).
    pub event_id: &'a [u8],
    /// Effective wake class.
    pub class: &'a str,
    /// Delivery deadline, in Unix seconds.
    pub expires_at: i64,
}

/// One exclusively claimed wake, already revalidated against its current lease.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedWake {
    /// Server-resolved tenant that owns this wake.
    pub community: CommunityId,
    /// Durable job id; this is also the stable gateway/APNs request id.
    pub id: Uuid,
    /// Claim fencing token required by every completion operation.
    pub claim_id: Uuid,
    /// Accepted event that caused the wake.
    pub event_id: Vec<u8>,
    /// Event channel used for send-time authorization revalidation.
    pub channel_id: Option<Uuid>,
    /// Lease author whose read authorization must be rechecked by the relay.
    pub author: Vec<u8>,
    /// Installation address within the community.
    pub installation_id: String,
    /// Generation captured when the job was enqueued.
    pub lease_generation: i64,
    /// Opaque endpoint capability for the stateless gateway.
    pub endpoint_grant: String,
    /// Wake class sent to the gateway.
    pub class: String,
    /// Delivery deadline, in Unix seconds.
    pub expires_at: i64,
    /// Attempt number, starting at one for the first claim.
    pub attempt: i32,
}

/// Outcome when a worker performs the final, load-bearing send-time check.
#[derive(Debug, Clone, PartialEq)]
pub enum RevalidateWakeOutcome {
    /// The claim and current lease still authorize delivery.
    Deliver(Box<ClaimedWake>),
    /// The claim was lost or the lease rotated, revoked, expired, or disabled.
    Suppressed,
}

/// One durably accepted event claimed for push matching.
#[derive(Debug, Clone)]
pub struct ClaimedMatch {
    /// Tenant that owns both the event and matcher job.
    pub community: CommunityId,
    /// Non-deleted source event loaded after the claim commits.
    pub event: buzz_core::StoredEvent,
    /// Fencing token required to complete or retry this claim.
    pub claim_id: Uuid,
    /// Attempt number, starting at one for the first claim.
    pub attempt: i32,
}

/// Current active lease candidate for matcher evaluation.
#[derive(Debug, Clone)]
pub struct MatchLease {
    /// Lease owner's raw public key.
    pub author: Vec<u8>,
    /// Installation address within the tenant.
    pub installation_id: String,
    /// Monotonic generation captured into any resulting wake.
    pub generation: i64,
    /// Validated restricted subscription array.
    pub subscriptions: Value,
    /// Lease expiry as a Unix timestamp.
    pub expires_at: i64,
}

/// Result of atomically accepting a signed push lease and its effective state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptLeaseOutcome {
    /// The source event and effective lease committed together.
    Accepted,
    /// The incoming event lost NIP-01 addressable ordering.
    StaleEvent,
    /// The incoming generation did not exceed the durable watermark.
    StaleGeneration,
    /// Another active address already owns this endpoint tuple.
    EndpointAlreadyLeased,
    /// The author already has the configured maximum active leases.
    LeaseQuotaExceeded,
    /// The source event id is already bound to another lease address.
    SourceEventCollision,
    /// A validated lease still violated a database integrity constraint.
    ConstraintViolation,
}

/// Atomically persist one validated kind:30350 event and its effective lease.
///
/// All policy inputs must already be validated. The transaction serializes both
/// the lease address and author-wide quota/endpoint namespace before changing
/// either the public source event or effective state.
#[allow(clippy::too_many_arguments)]
pub async fn accept_lease_event(
    pool: &PgPool,
    community: CommunityId,
    event: &nostr::Event,
    installation_id: &str,
    version: LeaseVersion<'_>,
    active: Option<ActiveLease<'_>>,
    max_active_leases: i64,
) -> Result<AcceptLeaseOutcome> {
    let author = event.pubkey.as_bytes();
    let mut tx = pool.begin().await?;
    let mut address_lock = Vec::with_capacity(16 + author.len() + installation_id.len());
    address_lock.extend_from_slice(community.as_uuid().as_bytes());
    address_lock.extend_from_slice(author);
    address_lock.extend_from_slice(installation_id.as_bytes());
    let address_lock = i64::from_le_bytes(Sha256::digest(&address_lock)[..8].try_into().unwrap());
    let mut author_lock = Vec::with_capacity(16 + author.len());
    author_lock.extend_from_slice(community.as_uuid().as_bytes());
    author_lock.extend_from_slice(author);
    let author_lock = i64::from_le_bytes(Sha256::digest(&author_lock)[..8].try_into().unwrap());
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(address_lock)
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(author_lock)
        .execute(&mut *tx)
        .await?;

    if let Some(row) = sqlx::query(
        "SELECT author, installation_id FROM push_leases WHERE community_id=$1 AND source_event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(version.source_event_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let existing_author: Vec<u8> = row.try_get("author")?;
        let existing_installation: String = row.try_get("installation_id")?;
        if existing_author.as_slice() != author || existing_installation != installation_id {
            return Ok(AcceptLeaseOutcome::SourceEventCollision);
        }
        return Ok(AcceptLeaseOutcome::StaleEvent);
    }

    if let Some(row) = sqlx::query(
        "SELECT source_event_id, source_created_at, generation FROM push_leases          WHERE community_id=$1 AND author=$2 AND installation_id=$3 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let current_created_at: i64 = row.try_get("source_created_at")?;
        let current_event_id: Vec<u8> = row.try_get("source_event_id")?;
        let current_generation: i64 = row.try_get("generation")?;
        let wins_event = version.source_created_at > current_created_at
            || (version.source_created_at == current_created_at
                && version.source_event_id < current_event_id.as_slice());
        if !wins_event {
            return Ok(AcceptLeaseOutcome::StaleEvent);
        }
        if version.generation <= current_generation {
            return Ok(AcceptLeaseOutcome::StaleGeneration);
        }
    }

    // Expired leases are ineffective and must not consume quota or endpoint
    // uniqueness forever. The author lock makes this cleanup atomic with the
    // subsequent author-wide checks and replacement.
    sqlx::query(
        "UPDATE push_leases SET active=false, endpoint_enabled=false, updated_at=now() \
         WHERE community_id=$1 AND author=$2 AND active \
           AND expires_at <= EXTRACT(EPOCH FROM now())::bigint",
    )
    .bind(community.as_uuid())
    .bind(author)
    .execute(&mut *tx)
    .await?;

    if let Some(active) = active {
        let active_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_leases WHERE community_id=$1 AND author=$2              AND active AND installation_id<>$3",
        )
        .bind(community.as_uuid())
        .bind(author)
        .bind(installation_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_count >= max_active_leases {
            return Ok(AcceptLeaseOutcome::LeaseQuotaExceeded);
        }
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM push_leases WHERE community_id=$1 AND author=$2              AND installation_id<>$3 AND active AND app_profile=$4 AND endpoint_hash=$5)",
        )
        .bind(community.as_uuid())
        .bind(author)
        .bind(installation_id)
        .bind(active.app_profile)
        .bind(active.endpoint_hash)
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Ok(AcceptLeaseOutcome::EndpointAlreadyLeased);
        }
    }

    sqlx::query(
        "UPDATE events SET deleted_at=now() WHERE community_id=$1 AND kind=30350          AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .execute(&mut *tx)
    .await?;
    let created_at = DateTime::from_timestamp(version.source_created_at, 0)
        .ok_or(crate::DbError::InvalidTimestamp(version.source_created_at))?;
    if let Err(error) = sqlx::query(
        "INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id,d_tag)          VALUES ($1,$2,$3,$4,30350,$5,$6,$7,now(),NULL,$8)",
    )
    .bind(community.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(author)
    .bind(created_at)
    .bind(serde_json::to_value(&event.tags)?)
    .bind(&event.content)
    .bind(event.sig.serialize().as_slice())
    .bind(installation_id)
    .execute(&mut *tx)
    .await
    {
        if let Some(outcome) = constraint_acceptance_outcome(&error) {
            return Ok(outcome);
        }
        return Err(error.into());
    }

    let (is_active, app_profile, endpoint_hash, endpoint_grant, max_class, subscriptions) = active
        .map_or((false, None, None, None, None, None), |active| {
            (
                true,
                Some(active.app_profile),
                Some(active.endpoint_hash),
                Some(active.endpoint_grant),
                Some(active.max_class),
                Some(active.subscriptions),
            )
        });
    if let Err(error) = sqlx::query(
        r#"INSERT INTO push_leases (community_id,author,installation_id,source_event_id,
            source_created_at,generation,active,app_profile,endpoint_hash,endpoint_grant,max_class,
            subscriptions,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
        ON CONFLICT (community_id,author,installation_id) DO UPDATE SET
            source_event_id=EXCLUDED.source_event_id, source_created_at=EXCLUDED.source_created_at,
            generation=EXCLUDED.generation, active=EXCLUDED.active, endpoint_enabled=true,
            app_profile=EXCLUDED.app_profile, endpoint_hash=EXCLUDED.endpoint_hash,
            endpoint_grant=EXCLUDED.endpoint_grant, max_class=EXCLUDED.max_class,
            subscriptions=EXCLUDED.subscriptions, expires_at=EXCLUDED.expires_at, updated_at=now()"#,
    )
    .bind(community.as_uuid()).bind(author).bind(installation_id)
    .bind(version.source_event_id).bind(version.source_created_at).bind(version.generation)
    .bind(is_active).bind(app_profile).bind(endpoint_hash).bind(endpoint_grant)
    .bind(max_class).bind(subscriptions).bind(version.expires_at)
    .execute(&mut *tx).await
    {
        if let Some(outcome) = constraint_acceptance_outcome(&error) {
            return Ok(outcome);
        }
        return Err(error.into());
    }
    tx.commit().await?;
    Ok(AcceptLeaseOutcome::Accepted)
}

fn constraint_acceptance_outcome(error: &sqlx::Error) -> Option<AcceptLeaseOutcome> {
    let sqlx::Error::Database(error) = error else {
        return None;
    };
    match error.code().as_deref() {
        Some("23505") if error.constraint() == Some("push_leases_endpoint_unique") => {
            Some(AcceptLeaseOutcome::EndpointAlreadyLeased)
        }
        Some("23505")
            if error.constraint() == Some("push_leases_community_id_source_event_id_key") =>
        {
            Some(AcceptLeaseOutcome::SourceEventCollision)
        }
        // Every integrity violation is a protocol-invalid lease, even if a
        // future migration renames/adds a constraint that validation missed.
        Some(code) if code.starts_with("23") => Some(AcceptLeaseOutcome::ConstraintViolation),
        _ => None,
    }
}

/// Create or rotate an active lease if both ordering gates win atomically.
#[allow(clippy::too_many_arguments)]
pub async fn replace_active_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
    active: ActiveLease<'_>,
) -> Result<ReplaceLeaseOutcome> {
    replace_lease(
        pool,
        community,
        author,
        installation_id,
        version,
        Some(active),
    )
    .await
}

/// Revoke one installation with a higher-generation inactive replacement.
pub async fn revoke_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
) -> Result<ReplaceLeaseOutcome> {
    replace_lease(pool, community, author, installation_id, version, None).await
}

async fn replace_lease(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    version: LeaseVersion<'_>,
    active: Option<ActiveLease<'_>>,
) -> Result<ReplaceLeaseOutcome> {
    let (is_active, app_profile, endpoint_hash, endpoint_grant, max_class, subscriptions) =
        match active {
            Some(active) => (
                true,
                Some(active.app_profile),
                Some(active.endpoint_hash),
                Some(active.endpoint_grant),
                Some(active.max_class),
                Some(active.subscriptions),
            ),
            None => (false, None, None, None, None, None),
        };

    // The conflict predicate is the acceptance state machine. Keeping both
    // orderings in the upsert closes the missing-row race: concurrent initial
    // publications cannot bypass a preceding SELECT/row lock.
    let accepted = sqlx::query(
        r#"
        INSERT INTO push_leases (
            community_id, author, installation_id, source_event_id,
            source_created_at, generation, active, app_profile, endpoint_hash,
            endpoint_grant, max_class, subscriptions, expires_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (community_id, author, installation_id) DO UPDATE SET
            source_event_id = EXCLUDED.source_event_id,
            source_created_at = EXCLUDED.source_created_at,
            generation = EXCLUDED.generation,
            active = EXCLUDED.active,
            endpoint_enabled = true,
            app_profile = EXCLUDED.app_profile,
            endpoint_hash = EXCLUDED.endpoint_hash,
            endpoint_grant = EXCLUDED.endpoint_grant,
            max_class = EXCLUDED.max_class,
            subscriptions = EXCLUDED.subscriptions,
            expires_at = EXCLUDED.expires_at,
            updated_at = now()
        WHERE (
                EXCLUDED.source_created_at > push_leases.source_created_at
                OR (
                    EXCLUDED.source_created_at = push_leases.source_created_at
                    AND EXCLUDED.source_event_id < push_leases.source_event_id
                )
              )
          AND EXCLUDED.generation > push_leases.generation
        RETURNING generation
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(version.source_event_id)
    .bind(version.source_created_at)
    .bind(version.generation)
    .bind(is_active)
    .bind(app_profile)
    .bind(endpoint_hash)
    .bind(endpoint_grant)
    .bind(max_class)
    .bind(subscriptions)
    .bind(version.expires_at)
    .fetch_optional(pool)
    .await?;

    if accepted.is_some() {
        return Ok(ReplaceLeaseOutcome::Accepted);
    }

    let current = sqlx::query(
        "SELECT source_event_id, source_created_at, generation \
         FROM push_leases \
         WHERE community_id = $1 AND author = $2 AND installation_id = $3",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;
    let current_created_at: i64 = current.try_get("source_created_at")?;
    let current_event_id: Vec<u8> = current.try_get("source_event_id")?;
    let wins_event_order = version.source_created_at > current_created_at
        || (version.source_created_at == current_created_at
            && version.source_event_id < current_event_id.as_slice());
    if !wins_event_order {
        Ok(ReplaceLeaseOutcome::StaleEvent)
    } else {
        Ok(ReplaceLeaseOutcome::StaleGeneration)
    }
}

/// Atomically enqueue at most one job per community, endpoint, and event.
///
/// Endpoint identity and the endpoint grant are copied from the current lease;
/// callers cannot redirect a wake by supplying either value. A generation that
/// lost a replacement race is ineligible in the same statement that inserts.
pub async fn enqueue_wake(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    wake: NewWake<'_>,
) -> Result<EnqueueWakeOutcome> {
    let mut tx = pool.begin().await?;
    // Serialize against lease replacement. If enqueue wins the lock, a later
    // replacement can leave this durable job queued, but worker revalidation
    // will suppress it; if replacement wins, the generation predicate fails.
    let endpoint_hash = sqlx::query(
        r#"
        SELECT endpoint_hash
        FROM push_leases
        WHERE community_id = $1
          AND author = $2
          AND installation_id = $3
          AND generation = $4
          AND active
          AND endpoint_enabled
          AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        FOR UPDATE
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(wake.lease_generation)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(endpoint_hash) = endpoint_hash else {
        return Ok(EnqueueWakeOutcome::InactiveLease);
    };
    let endpoint_hash: Vec<u8> = endpoint_hash.try_get("endpoint_hash")?;

    let inserted = sqlx::query(
        r#"
        INSERT INTO push_wake_outbox (
            community_id, author, installation_id, lease_generation,
            endpoint_hash, event_id, class, expires_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (community_id, endpoint_hash, event_id) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(wake.lease_generation)
    .bind(&endpoint_hash)
    .bind(wake.event_id)
    .bind(wake.class)
    .bind(wake.expires_at)
    .fetch_optional(&mut *tx)
    .await?;

    let outcome = if let Some(row) = inserted {
        EnqueueWakeOutcome::Enqueued(row.try_get("id")?)
    } else {
        // This is a separate statement so READ COMMITTED observes a competing
        // transaction whose unique-key insert completed while ours waited.
        let row = sqlx::query(
            "SELECT id FROM push_wake_outbox \
             WHERE community_id = $1 AND endpoint_hash = $2 AND event_id = $3",
        )
        .bind(community.as_uuid())
        .bind(&endpoint_hash)
        .bind(wake.event_id)
        .fetch_one(&mut *tx)
        .await?;
        EnqueueWakeOutcome::Duplicate(row.try_get("id")?)
    };
    tx.commit().await?;
    Ok(outcome)
}

/// Exclusively claim the next due matcher job and load its non-deleted event.
pub async fn claim_due_match(
    pool: &PgPool,
    lease_until: DateTime<Utc>,
) -> Result<Option<ClaimedMatch>> {
    claim_due_match_with_loader(pool, lease_until, |pool, community, event_id| async move {
        Ok(
            crate::event::get_events_by_ids(&pool, community, &[&event_id])
                .await?
                .into_iter()
                .next(),
        )
    })
    .await
}

async fn claim_due_match_with_loader<F, Fut>(
    pool: &PgPool,
    lease_until: DateTime<Utc>,
    load: F,
) -> Result<Option<ClaimedMatch>>
where
    F: FnOnce(PgPool, CommunityId, Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = Result<Option<buzz_core::StoredEvent>>>,
{
    let claim_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    // Reap poison jobs before claiming so a worker crash on the final attempt
    // cannot leave an unclaimable row pinning outbox retention forever.
    sqlx::query(
        "DELETE FROM push_match_queue WHERE attempts >= $1 \
         AND (state='pending' OR (state='matching' AND lease_until < now()))",
    )
    .bind(MAX_MATCH_ATTEMPTS)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query(
        r#"
        WITH candidate AS (
            SELECT community_id, event_id
            FROM push_match_queue
            WHERE attempts < $3
              AND next_attempt_at <= now()
              AND (state = 'pending' OR (state = 'matching' AND lease_until < now()))
            ORDER BY next_attempt_at, created_at
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE push_match_queue q
        SET state='matching', claim_id=$1, lease_until=$2, attempts=attempts+1
        FROM candidate c
        WHERE q.community_id=c.community_id AND q.event_id=c.event_id
        RETURNING q.community_id, q.event_id, q.attempts
        "#,
    )
    .bind(claim_id)
    .bind(lease_until)
    .bind(MAX_MATCH_ATTEMPTS)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    let community = CommunityId::from_uuid(row.try_get("community_id")?);
    let event_id: Vec<u8> = row.try_get("event_id")?;
    let attempt: i32 = row.try_get("attempts")?;
    tx.commit().await?;
    let event = load(pool.clone(), community, event_id.clone()).await?;
    let Some(event) = event else {
        // Source absence and soft deletion are deliberate privacy-preserving
        // terminal outcomes. Query errors above propagate instead, leaving the
        // fenced job recoverable after its claim lease expires.
        sqlx::query(
            "DELETE FROM push_match_queue \
             WHERE community_id=$1 AND event_id=$2 AND claim_id=$3 AND state='matching'",
        )
        .bind(community.as_uuid())
        .bind(&event_id)
        .bind(claim_id)
        .execute(pool)
        .await?;
        return Ok(None);
    };
    Ok(Some(ClaimedMatch {
        community,
        event,
        claim_id,
        attempt,
    }))
}

/// Load active endpoint-enabled leases for one tenant.
pub async fn active_match_leases(pool: &PgPool, community: CommunityId) -> Result<Vec<MatchLease>> {
    let rows = sqlx::query(
        "SELECT author, installation_id, generation, subscriptions, expires_at \
         FROM push_leases WHERE community_id=$1 AND active AND endpoint_enabled \
         AND expires_at > EXTRACT(EPOCH FROM now())::bigint",
    )
    .bind(community.as_uuid())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(MatchLease {
                author: row.try_get("author")?,
                installation_id: row.try_get("installation_id")?,
                generation: row.try_get("generation")?,
                subscriptions: row.try_get("subscriptions")?,
                expires_at: row.try_get("expires_at")?,
            })
        })
        .collect()
}

/// Delete a matcher job only while its claim fence is held.
pub async fn complete_match(pool: &PgPool, match_job: &ClaimedMatch) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM push_match_queue WHERE community_id=$1 AND event_id=$2 AND claim_id=$3 AND state='matching'")
        .bind(match_job.community.as_uuid()).bind(match_job.event.event.id.as_bytes().as_slice())
        .bind(match_job.claim_id).execute(pool).await?.rows_affected() == 1)
}

/// Release a fenced matcher claim for retry at the supplied time.
pub async fn retry_match(
    pool: &PgPool,
    match_job: &ClaimedMatch,
    next: DateTime<Utc>,
) -> Result<bool> {
    Ok(sqlx::query("UPDATE push_match_queue SET state='pending', claim_id=NULL, lease_until=NULL, next_attempt_at=$4 WHERE community_id=$1 AND event_id=$2 AND claim_id=$3 AND state='matching'")
        .bind(match_job.community.as_uuid()).bind(match_job.event.event.id.as_bytes().as_slice())
        .bind(match_job.claim_id).bind(next).execute(pool).await?.rows_affected() == 1)
}

/// Claim due jobs for one community, recovering expired worker leases.
///
/// Claiming performs an early lease check, but callers MUST invoke
/// [`revalidate_wake_for_send`] immediately before the transport call.
pub async fn claim_due_wakes(
    pool: &PgPool,
    community: CommunityId,
    limit: i64,
    lease_until: DateTime<Utc>,
) -> Result<Vec<ClaimedWake>> {
    let claim_id = Uuid::new_v4();
    let rows = sqlx::query(
        r#"
        WITH candidates AS (
            SELECT o.id, e.channel_id
            FROM push_wake_outbox o
            JOIN push_leases l
              ON l.community_id = o.community_id
             AND l.author = o.author
             AND l.installation_id = o.installation_id
             AND l.generation = o.lease_generation
             AND l.endpoint_hash = o.endpoint_hash
            LEFT JOIN events e
              ON e.community_id = o.community_id
             AND e.id = o.event_id
             AND e.deleted_at IS NULL
            WHERE o.community_id = $1
              AND e.id IS NOT NULL
              AND o.expires_at > EXTRACT(EPOCH FROM now())::bigint
              AND o.next_attempt_at <= now()
              AND (o.state = 'pending' OR (o.state = 'sending' AND o.lease_until < now()))
              AND l.active
              AND l.endpoint_enabled
              AND l.expires_at > EXTRACT(EPOCH FROM now())::bigint
            ORDER BY o.next_attempt_at, o.created_at, o.id
            FOR UPDATE OF o SKIP LOCKED
            LIMIT $2
        )
        UPDATE push_wake_outbox o
        SET state = 'sending', claim_id = $3, lease_until = $4, attempts = attempts + 1
        FROM candidates c, push_leases l
        WHERE o.community_id = $1
          AND o.id = c.id
          AND l.community_id = o.community_id
          AND l.author = o.author
          AND l.installation_id = o.installation_id
          AND l.generation = o.lease_generation
          AND l.endpoint_hash = o.endpoint_hash
        RETURNING o.community_id, o.id, o.claim_id, o.event_id, c.channel_id,
                  o.author, o.installation_id, o.lease_generation,
                  l.endpoint_grant, o.class, o.expires_at, o.attempts
        "#,
    )
    .bind(community.as_uuid())
    .bind(limit)
    .bind(claim_id)
    .bind(lease_until)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_claimed_wake).collect()
}

/// Revalidate a fenced claim immediately before sending it.
///
/// This exact community + generation + endpoint join is the load-bearing RF1
/// gate. Claim-time eligibility and replacement-time cancellation are only
/// optimizations; neither can replace this send-time check.
pub async fn revalidate_wake_for_send(
    pool: &PgPool,
    community: CommunityId,
    id: Uuid,
    claim_id: Uuid,
) -> Result<RevalidateWakeOutcome> {
    let row = sqlx::query(
        r#"
        SELECT o.community_id, o.id, o.claim_id, o.event_id, e.channel_id,
               o.author, o.installation_id, o.lease_generation,
               l.endpoint_grant, o.class, o.expires_at, o.attempts
        FROM push_wake_outbox o
        JOIN push_leases l
          ON l.community_id = o.community_id
         AND l.author = o.author
         AND l.installation_id = o.installation_id
         AND l.generation = o.lease_generation
         AND l.endpoint_hash = o.endpoint_hash
        JOIN events e
          ON e.community_id = o.community_id
         AND e.id = o.event_id
         AND e.deleted_at IS NULL
        WHERE o.community_id = $1
          AND o.id = $2
          AND o.claim_id = $3
          AND o.state = 'sending'
          AND o.lease_until >= now()
          AND o.expires_at > EXTRACT(EPOCH FROM now())::bigint
          AND l.active
          AND l.endpoint_enabled
          AND l.expires_at > EXTRACT(EPOCH FROM now())::bigint
        "#,
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(claim_id)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_claimed_wake)
        .transpose()?
        .map_or(Ok(RevalidateWakeOutcome::Suppressed), |wake| {
            Ok(RevalidateWakeOutcome::Deliver(Box::new(wake)))
        })
}

/// Mark a fenced claim delivered. Stale workers cannot complete a newer claim.
pub async fn complete_wake(
    pool: &PgPool,
    community: CommunityId,
    id: Uuid,
    claim_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE push_wake_outbox \
         SET state = 'delivered', claim_id = NULL, lease_until = NULL \
         WHERE community_id = $1 AND id = $2 AND claim_id = $3 AND state = 'sending'",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(claim_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Return a fenced claim to the pending queue for a bounded retry.
pub async fn retry_wake(
    pool: &PgPool,
    community: CommunityId,
    id: Uuid,
    claim_id: Uuid,
    next_attempt_at: DateTime<Utc>,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE push_wake_outbox \
         SET state = 'pending', next_attempt_at = $4, claim_id = NULL, lease_until = NULL \
         WHERE community_id = $1 AND id = $2 AND claim_id = $3 AND state = 'sending'",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(claim_id)
    .bind(next_attempt_at)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Permanently fail one fenced claim without affecting its lease or siblings.
pub async fn fail_wake(
    pool: &PgPool,
    community: CommunityId,
    id: Uuid,
    claim_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE push_wake_outbox \
         SET state = 'failed', claim_id = NULL, lease_until = NULL \
         WHERE community_id = $1 AND id = $2 AND claim_id = $3 AND state = 'sending'",
    )
    .bind(community.as_uuid())
    .bind(id)
    .bind(claim_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Disable exactly the current endpoint generation after a permanent response.
///
/// Strict generation monotonicity is the underlying safety invariant. The
/// current-generation predicate makes stale responses clean no-ops.
pub async fn disable_endpoint_generation(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    generation: i64,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE push_leases SET endpoint_enabled = false, updated_at = now() \
         WHERE community_id = $1 AND author = $2 AND installation_id = $3 \
           AND generation = $4 AND active AND endpoint_enabled",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(installation_id)
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Delete terminal/expired outbox rows older than a retention cutoff.
///
/// NIP-RS hard purge only targets kind 30078, which is not push-eligible and
/// therefore cannot have a matcher row; any other absent source is handled by
/// the matcher's fenced load-miss deletion.
pub async fn prune_wake_outbox(
    pool: &PgPool,
    community: CommunityId,
    before: DateTime<Utc>,
) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM push_wake_outbox o \
         WHERE o.community_id = $1 AND o.created_at < $2 \
           AND (o.state IN ('delivered', 'failed') \
                OR o.expires_at <= EXTRACT(EPOCH FROM now())::bigint) \
           AND NOT EXISTS ( \
               SELECT 1 FROM push_match_queue q \
               WHERE q.community_id = o.community_id AND q.event_id = o.event_id \
           )",
    )
    .bind(community.as_uuid())
    .bind(before)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn row_to_claimed_wake(row: sqlx::postgres::PgRow) -> Result<ClaimedWake> {
    Ok(ClaimedWake {
        community: CommunityId::from_uuid(row.try_get("community_id")?),
        id: row.try_get("id")?,
        claim_id: row.try_get("claim_id")?,
        event_id: row.try_get("event_id")?,
        channel_id: row.try_get("channel_id")?,
        author: row.try_get("author")?,
        installation_id: row.try_get("installation_id")?,
        lease_generation: row.try_get("lease_generation")?,
        endpoint_grant: row.try_get("endpoint_grant")?,
        class: row.try_get("class")?,
        expires_at: row.try_get("expires_at")?,
        attempt: row.try_get("attempts")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    fn lease_event(keys: &nostr::Keys, installation: &str, created_at: u64) -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::Custom(30_350), "ciphertext")
            .tag(nostr::Tag::parse(["d", installation]).expect("d tag"))
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("sign lease event")
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("push-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    fn version(event: u8, created_at: i64, generation: i64) -> LeaseVersion<'static> {
        LeaseVersion {
            source_event_id: Box::leak(Box::new([event; 32])),
            source_created_at: created_at,
            generation,
            expires_at: i64::MAX / 2,
        }
    }

    async fn activate(
        pool: &PgPool,
        community: CommunityId,
        author: &[u8],
        installation: &str,
        endpoint: &[u8],
        generation: i64,
    ) {
        assert_eq!(
            replace_active_lease(
                pool,
                community,
                author,
                installation,
                version(generation as u8, generation * 10, generation),
                ActiveLease {
                    app_profile: "ios-production",
                    endpoint_hash: endpoint,
                    endpoint_grant: "opaque-grant",
                    max_class: "default",
                    subscriptions: &serde_json::json!([]),
                },
            )
            .await
            .expect("activate lease"),
            ReplaceLeaseOutcome::Accepted
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn acceptance_constraint_failure_rolls_back_source_event() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let keys = nostr::Keys::generate();
        let event = lease_event(&keys, "install", 100);
        let endpoint = [42; 32];
        let subscriptions = serde_json::json!([]);

        let outcome = accept_lease_event(
            &pool,
            community,
            &event,
            "install",
            LeaseVersion {
                source_event_id: event.id.as_bytes(),
                source_created_at: 100,
                generation: 1,
                expires_at: 200,
            },
            Some(ActiveLease {
                app_profile: "ios-production",
                endpoint_hash: &endpoint,
                endpoint_grant: "opaque-grant",
                max_class: "not-a-class",
                subscriptions: &subscriptions,
            }),
            16,
        )
        .await
        .expect("constraint maps to an acceptance outcome");
        assert_eq!(outcome, AcceptLeaseOutcome::ConstraintViolation);

        let event_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(event.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count source events");
        let lease_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_leases WHERE community_id=$1 AND author=$2 AND installation_id=$3",
        )
        .bind(community.as_uuid())
        .bind(event.pubkey.as_bytes())
        .bind("install")
        .fetch_one(&pool)
        .await
        .expect("count leases");
        assert_eq!((event_count, lease_count), (0, 0));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn source_event_collision_is_protocol_outcome_without_event_insert() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let keys = nostr::Keys::generate();
        let event = lease_event(&keys, "incoming", 100);
        let author = event.pubkey.to_bytes();
        let endpoint = [43; 32];
        let subscriptions = serde_json::json!([]);
        replace_active_lease(
            &pool,
            community,
            &author,
            "existing",
            LeaseVersion {
                source_event_id: event.id.as_bytes(),
                source_created_at: 90,
                generation: 1,
                expires_at: 200,
            },
            ActiveLease {
                app_profile: "ios-production",
                endpoint_hash: &endpoint,
                endpoint_grant: "opaque-grant",
                max_class: "default",
                subscriptions: &subscriptions,
            },
        )
        .await
        .expect("seed colliding lease");

        let outcome = accept_lease_event(
            &pool,
            community,
            &event,
            "incoming",
            LeaseVersion {
                source_event_id: event.id.as_bytes(),
                source_created_at: 100,
                generation: 2,
                expires_at: 200,
            },
            None,
            16,
        )
        .await
        .expect("collision is not an internal error");
        assert_eq!(outcome, AcceptLeaseOutcome::SourceEventCollision);
        let event_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(event.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("count source events");
        assert_eq!(event_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replacement_and_revoke_are_community_scoped_and_dual_ordered() {
        let pool = setup_pool().await;
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;
        let author = [7; 32];
        let endpoint = [8; 32];
        activate(&pool, a, &author, "install", &endpoint, 1).await;
        activate(&pool, b, &author, "install", &endpoint, 1).await;

        assert_eq!(
            revoke_lease(&pool, a, &author, "install", version(2, 20, 2))
                .await
                .expect("revoke A"),
            ReplaceLeaseOutcome::Accepted
        );
        assert_eq!(
            replace_active_lease(
                &pool,
                a,
                &author,
                "install",
                version(3, 15, 99),
                ActiveLease {
                    app_profile: "ios-production",
                    endpoint_hash: &endpoint,
                    endpoint_grant: "grant",
                    max_class: "default",
                    subscriptions: &serde_json::json!([]),
                },
            )
            .await
            .expect("old event loses"),
            ReplaceLeaseOutcome::StaleEvent
        );

        let active: bool = sqlx::query_scalar(
            "SELECT active FROM push_leases \
             WHERE community_id = $1 AND author = $2 AND installation_id = $3",
        )
        .bind(b.as_uuid())
        .bind(author)
        .bind("install")
        .fetch_one(&pool)
        .await
        .expect("read B");
        assert!(active, "revoking A must not touch B");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_enqueue_is_atomic_and_community_scoped() {
        let pool = setup_pool().await;
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;
        let author = [9; 32];
        let endpoint = [10; 32];
        let event = [11; 32];
        activate(&pool, a, &author, "install", &endpoint, 1).await;
        activate(&pool, b, &author, "install", &endpoint, 1).await;

        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                enqueue_wake(
                    &pool,
                    a,
                    &author,
                    "install",
                    NewWake {
                        lease_generation: 1,
                        event_id: &event,
                        class: "default",
                        expires_at: i64::MAX / 2,
                    },
                )
                .await
                .expect("enqueue")
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(match task.await.expect("join") {
                EnqueueWakeOutcome::Enqueued(id) | EnqueueWakeOutcome::Duplicate(id) => id,
                EnqueueWakeOutcome::InactiveLease => panic!("lease unexpectedly inactive"),
            });
        }
        assert!(ids.iter().all(|id| *id == ids[0]));
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_wake_outbox \
             WHERE community_id = $1 AND endpoint_hash = $2 AND event_id = $3",
        )
        .bind(a.as_uuid())
        .bind(endpoint)
        .bind(event)
        .fetch_one(&pool)
        .await
        .expect("count A jobs");
        assert_eq!(count, 1);

        assert!(matches!(
            enqueue_wake(
                &pool,
                b,
                &author,
                "install",
                NewWake {
                    lease_generation: 1,
                    event_id: &event,
                    class: "default",
                    expires_at: i64::MAX / 2,
                },
            )
            .await
            .expect("enqueue B"),
            EnqueueWakeOutcome::Enqueued(_)
        ));
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_wake_outbox \
             WHERE endpoint_hash = $1 AND event_id = $2",
        )
        .bind(endpoint)
        .bind(event)
        .fetch_one(&pool)
        .await
        .expect("count all jobs");
        assert_eq!(total, 2, "same dedup key is independent per community");
    }

    async fn enqueue_one(
        pool: &PgPool,
        community: CommunityId,
        author: &[u8],
        event_id: &[u8; 32],
        generation: i64,
    ) -> Uuid {
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
             VALUES ($1, $2, $3, to_timestamp(1), 9, '[]', '', $4)",
        )
        .bind(community.as_uuid())
        .bind(event_id)
        .bind([42_u8; 32])
        .bind([43_u8; 64])
        .execute(pool)
        .await
        .expect("insert wake source event");
        match enqueue_wake(
            pool,
            community,
            author,
            "install",
            NewWake {
                lease_generation: generation,
                event_id,
                class: "default",
                expires_at: i64::MAX / 2,
            },
        )
        .await
        .expect("enqueue wake")
        {
            EnqueueWakeOutcome::Enqueued(id) => id,
            other => panic!("expected fresh enqueue, got {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn send_revalidation_suppresses_rotated_claim_and_retry_preserves_id() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let author = [12; 32];
        activate(&pool, community, &author, "install", &[13; 32], 1).await;
        let id = enqueue_one(&pool, community, &author, &[14; 32], 1).await;
        let claim = claim_due_wakes(
            &pool,
            community,
            1,
            Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .expect("claim")
        .pop()
        .expect("claimed job");
        assert_eq!(claim.id, id);
        assert_eq!(claim.attempt, 1);

        activate(&pool, community, &author, "install", &[15; 32], 2).await;
        assert_eq!(
            revalidate_wake_for_send(&pool, community, id, claim.claim_id)
                .await
                .expect("revalidate after rotate"),
            RevalidateWakeOutcome::Suppressed
        );

        let event = [16; 32];
        let retry_id = enqueue_one(&pool, community, &author, &event, 2).await;
        let first = claim_due_wakes(
            &pool,
            community,
            1,
            Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .expect("first claim")
        .into_iter()
        .find(|wake| wake.id == retry_id)
        .expect("retry job claimed");
        let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
            .fetch_one(&pool)
            .await
            .expect("read database clock");
        assert!(retry_wake(
            &pool,
            community,
            retry_id,
            first.claim_id,
            database_now - chrono::Duration::seconds(1),
        )
        .await
        .expect("schedule retry"));
        let second = claim_due_wakes(
            &pool,
            community,
            10,
            Utc::now() + chrono::Duration::minutes(1),
        )
        .await
        .expect("second claim")
        .into_iter()
        .find(|wake| wake.id == retry_id)
        .expect("retry reclaimed");
        assert_eq!(second.id, first.id, "durable request id must be stable");
        assert_eq!(second.attempt, 2);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn endpoint_invalidation_is_scoped_to_community_and_generation() {
        let pool = setup_pool().await;
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;
        let author = [17; 32];
        let endpoint = [18; 32];
        activate(&pool, a, &author, "install", &endpoint, 1).await;
        activate(&pool, b, &author, "install", &endpoint, 1).await;

        assert!(disable_endpoint_generation(&pool, a, &author, "install", 1)
            .await
            .expect("disable A generation 1"));
        assert!(
            !disable_endpoint_generation(&pool, a, &author, "install", 1)
                .await
                .expect("duplicate disable is a no-op")
        );
        assert!(matches!(
            enqueue_wake(
                &pool,
                a,
                &author,
                "install",
                NewWake {
                    lease_generation: 1,
                    event_id: &[19; 32],
                    class: "default",
                    expires_at: i64::MAX / 2,
                },
            )
            .await
            .expect("enqueue disabled A"),
            EnqueueWakeOutcome::InactiveLease
        ));
        assert!(matches!(
            enqueue_wake(
                &pool,
                b,
                &author,
                "install",
                NewWake {
                    lease_generation: 1,
                    event_id: &[19; 32],
                    class: "default",
                    expires_at: i64::MAX / 2,
                },
            )
            .await
            .expect("enqueue healthy B"),
            EnqueueWakeOutcome::Enqueued(_)
        ));

        activate(&pool, a, &author, "install", &[20; 32], 2).await;
        assert!(
            !disable_endpoint_generation(&pool, a, &author, "install", 1)
                .await
                .expect("stale response")
        );
        assert!(matches!(
            enqueue_wake(
                &pool,
                a,
                &author,
                "install",
                NewWake {
                    lease_generation: 2,
                    event_id: &[21; 32],
                    class: "default",
                    expires_at: i64::MAX / 2,
                },
            )
            .await
            .expect("new generation stays enabled"),
            EnqueueWakeOutcome::Enqueued(_)
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn matcher_trigger_is_allowlisted_and_deleted_events_are_discarded() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let keys = nostr::Keys::generate();
        let push_event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "push")
            .sign_with_keys(&keys)
            .expect("sign push event");
        let read_state = nostr::EventBuilder::new(nostr::Kind::Custom(30_078), "read")
            .sign_with_keys(&keys)
            .expect("sign read state");
        crate::event::insert_event(&pool, community, &push_event, None)
            .await
            .expect("insert push event");
        crate::event::insert_event(&pool, community, &read_state, None)
            .await
            .expect("insert non-push event");

        let queued: Vec<i32> = sqlx::query_scalar(
            "SELECT e.kind FROM push_match_queue q JOIN events e \
             ON e.community_id=q.community_id AND e.id=q.event_id \
             WHERE q.community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("read matcher queue");
        assert_eq!(queued, vec![9]);

        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(push_event.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("soft delete before matching");
        assert!(
            claim_due_match(&pool, Utc::now() + chrono::Duration::minutes(1))
                .await
                .expect("claim deleted event")
                .is_none()
        );
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_match_queue WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count discarded job");
        assert_eq!(remaining, 0, "deleted content must never produce a wake");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn matcher_load_error_preserves_claimed_job_for_recovery() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "retry me")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign event");
        crate::event::insert_event(&pool, community, &event, None)
            .await
            .expect("insert event");

        let error = claim_due_match_with_loader(
            &pool,
            Utc::now() - chrono::Duration::seconds(1),
            |_pool, _community, _event_id| async {
                Err(crate::DbError::InvalidData("injected load failure".into()))
            },
        )
        .await
        .expect_err("load error must propagate");
        assert!(error.to_string().contains("injected load failure"));
        let row: (String, i32) = sqlx::query_as(
            "SELECT state, attempts FROM push_match_queue WHERE community_id=$1 AND event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("load failure must preserve matcher row");
        assert_eq!(row, ("matching".to_string(), 1));
        assert!(
            claim_due_match(&pool, Utc::now() + chrono::Duration::minutes(1))
                .await
                .expect("expired claim remains recoverable")
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn matcher_claim_is_exclusive_across_workers() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "one job")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign event");
        crate::event::insert_event(&pool, community, &event, None)
            .await
            .expect("insert event");
        let barrier = Arc::new(Barrier::new(8));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                claim_due_match(&pool, Utc::now() + chrono::Duration::minutes(1))
                    .await
                    .expect("claim matcher job")
            }));
        }
        let mut claimed = 0;
        for task in tasks {
            claimed += usize::from(task.await.expect("join").is_some());
        }
        assert_eq!(claimed, 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn delivered_wake_is_retained_while_rematch_is_queued() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let author = [22; 32];
        let event_id = [23; 32];
        activate(&pool, community, &author, "install", &[24; 32], 1).await;
        let wake_id = enqueue_one(&pool, community, &author, &event_id, 1).await;
        sqlx::query(
            "UPDATE push_wake_outbox SET state='delivered', created_at=now()-interval '2 days' \
             WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(wake_id)
        .execute(&pool)
        .await
        .expect("mark old wake delivered");

        let cutoff = Utc::now() - chrono::Duration::days(1);
        assert_eq!(
            prune_wake_outbox(&pool, community, cutoff).await.unwrap(),
            0
        );
        sqlx::query("DELETE FROM push_match_queue WHERE community_id=$1 AND event_id=$2")
            .bind(community.as_uuid())
            .bind(event_id)
            .execute(&pool)
            .await
            .expect("complete rematch");
        assert_eq!(
            prune_wake_outbox(&pool, community, cutoff).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn exhausted_match_job_is_reaped_and_cannot_pin_retention() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "poison")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign event");
        crate::event::insert_event(&pool, community, &event, None)
            .await
            .expect("insert event");
        let author = [25; 32];
        activate(&pool, community, &author, "install", &[26; 32], 1).await;
        let wake_id = match enqueue_wake(
            &pool,
            community,
            &author,
            "install",
            NewWake {
                lease_generation: 1,
                event_id: event.id.as_bytes(),
                class: "default",
                expires_at: i64::MAX / 2,
            },
        )
        .await
        .expect("enqueue wake")
        {
            EnqueueWakeOutcome::Enqueued(id) => id,
            other => panic!("expected fresh wake, got {other:?}"),
        };
        sqlx::query(
            "UPDATE push_wake_outbox SET state='delivered', created_at=now()-interval '2 days' \
             WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(wake_id)
        .execute(&pool)
        .await
        .expect("mark old wake delivered");
        let cutoff = Utc::now() - chrono::Duration::days(1);
        assert_eq!(
            prune_wake_outbox(&pool, community, cutoff).await.unwrap(),
            0
        );
        sqlx::query(
            "UPDATE push_match_queue SET attempts=$3, state='matching', lease_until=now()-interval '1 second' \
             WHERE community_id=$1 AND event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(MAX_MATCH_ATTEMPTS)
        .execute(&pool)
        .await
        .expect("exhaust matcher job");
        assert!(
            claim_due_match(&pool, Utc::now() + chrono::Duration::minutes(1))
                .await
                .expect("reap exhausted matcher")
                .is_none()
        );
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM push_match_queue WHERE community_id=$1 AND event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(
            prune_wake_outbox(&pool, community, cutoff).await.unwrap(),
            1,
            "reaped poison job must release delivered-wake retention"
        );
    }
}
