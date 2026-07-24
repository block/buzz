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

/// Namespace for the per-community push-gate advisory lock. Must match the
/// key built inside the `enqueue_push_match_job` trigger (migration 0023):
/// event inserts take it SHARED there; every lease transition that can make
/// match eligibility true takes it EXCLUSIVE here, forcing a total order so
/// a concurrent event insert either sees the committed lease or strictly
/// precedes the activation (in which case no wake was owed). Distinct key
/// domain from the audit lock and the lease address/author locks.
const PUSH_GATE_LOCK_NAMESPACE: &str = "buzz_push_gate:";

async fn acquire_push_gate_lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community: CommunityId,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{PUSH_GATE_LOCK_NAMESPACE}{}", community.as_uuid()))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Recovery window for the activation backfill: recent events that the gate
/// legitimately skipped (no eligible lease at their commit time) are enqueued
/// when a lease activates, so a user who registers moments after a message
/// still gets woken. Product coverage only — the advisory-lock total order is
/// what makes the gate correct; see `PUSH_GATE_LOCK_NAMESPACE`.
const PUSH_GATE_BACKFILL_SECS: i64 = 120;

/// Enqueue match jobs for recent gate-skipped events. MUST run inside the same
/// transaction that holds the exclusive push-gate lock: after this commit,
/// every event is either backfilled here or ordered after the activation and
/// enqueued by the trigger — running it post-commit would reopen the gap.
/// Keyed on relay `received_at` (not author-controlled `created_at`); the kind
/// list mirrors the trigger allowlist; `ON CONFLICT DO NOTHING` dedups against
/// rows the trigger already enqueued.
async fn backfill_push_match_jobs(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community: CommunityId,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO push_match_queue (community_id, event_id) \
         SELECT community_id, id FROM events \
         WHERE community_id = $1 \
           AND kind IN (7, 9, 1059, 40007, 46010) \
           AND deleted_at IS NULL \
           AND received_at > now() - make_interval(secs => $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(PUSH_GATE_BACKFILL_SECS as f64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

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
    // T1b: an activation can flip the community from "no eligible lease" to
    // "eligible", so it must serialize against the trigger's shared gate lock.
    // Acquired after the address/author locks to keep one global lock order.
    if active.is_some() {
        acquire_push_gate_lock(&mut tx, community).await?;
    }

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
    if is_active {
        backfill_push_match_jobs(&mut tx, community).await?;
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

    // T1b: an activating replacement can flip the community from "no eligible
    // lease" to "eligible"; serialize it against the trigger's shared gate
    // lock (gate → lease row, matching accept_lease_event's global order).
    // Revocations (is_active = false) never make eligibility true and skip it.
    let mut tx = pool.begin().await?;
    if is_active {
        acquire_push_gate_lock(&mut tx, community).await?;
    }

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
    .fetch_optional(&mut *tx)
    .await?;

    if accepted.is_some() {
        if is_active {
            backfill_push_match_jobs(&mut tx, community).await?;
        }
        tx.commit().await?;
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
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
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

/// One matcher-produced wake request inside a set-wise enqueue.
#[derive(Debug, Clone)]
pub struct WakeRequest {
    /// Lease owner's raw public key.
    pub author: Vec<u8>,
    /// Installation address within the tenant.
    pub installation_id: String,
    /// Generation observed by the matcher.
    pub lease_generation: i64,
    /// Accepted event id that caused the wake (32 bytes).
    pub event_id: Vec<u8>,
    /// Effective wake class.
    pub class: String,
    /// Delivery deadline, in Unix seconds.
    pub expires_at: i64,
}

/// Atomically enqueue at most one job per community, endpoint, and event.
///
/// Batch-of-one shape of [`enqueue_wakes`]; see there for the protocol.
pub async fn enqueue_wake(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    installation_id: &str,
    wake: NewWake<'_>,
) -> Result<EnqueueWakeOutcome> {
    let outcomes = enqueue_wakes(
        pool,
        community,
        &[WakeRequest {
            author: author.to_vec(),
            installation_id: installation_id.to_string(),
            lease_generation: wake.lease_generation,
            event_id: wake.event_id.to_vec(),
            class: wake.class.to_string(),
            expires_at: wake.expires_at,
        }],
    )
    .await?;
    Ok(outcomes
        .into_iter()
        .next()
        .expect("one outcome per request"))
}

/// Set-wise counterpart of [`enqueue_wake`]: one transaction and a constant
/// number of statements for any number of matched (lease, event) pairs (T2b).
///
/// Endpoint identity and the endpoint grant are copied from each current
/// lease; callers cannot redirect a wake by supplying either value.
/// Revalidation locks the distinct lease rows `FOR UPDATE` in one statement,
/// ordered by (author, installation_id) so concurrent batches and
/// `replace_active_lease` (single-row lock) acquire in a consistent order. If
/// enqueue wins a lock race, a later replacement can leave a durable job
/// queued, but worker revalidation suppresses it; if replacement wins, the
/// generation comparison fails and that request reports `InactiveLease`.
///
/// Returns one outcome per request, index-aligned with `requests`.
pub async fn enqueue_wakes(
    pool: &PgPool,
    community: CommunityId,
    requests: &[WakeRequest],
) -> Result<Vec<EnqueueWakeOutcome>> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = pool.begin().await?;

    // 1. Lock and read the current lease row for every distinct requested
    //    (author, installation), in deterministic order.
    let mut pairs: Vec<(&[u8], &str)> = requests
        .iter()
        .map(|r| (r.author.as_slice(), r.installation_id.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let lock_authors: Vec<&[u8]> = pairs.iter().map(|(a, _)| *a).collect();
    let lock_installs: Vec<&str> = pairs.iter().map(|(_, i)| *i).collect();
    let lease_rows = sqlx::query(
        r#"
        SELECT author, installation_id, generation, endpoint_hash
        FROM push_leases
        WHERE community_id = $1
          AND (author, installation_id) IN
              (SELECT a, i FROM UNNEST($2::bytea[], $3::text[]) AS t(a, i))
          AND active
          AND endpoint_enabled
          AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        ORDER BY author, installation_id
        FOR UPDATE
        "#,
    )
    .bind(community.as_uuid())
    .bind(&lock_authors)
    .bind(&lock_installs)
    .fetch_all(&mut *tx)
    .await?;
    let mut leases: std::collections::HashMap<(Vec<u8>, String), (i64, Vec<u8>)> =
        std::collections::HashMap::with_capacity(lease_rows.len());
    for row in lease_rows {
        leases.insert(
            (row.try_get("author")?, row.try_get("installation_id")?),
            (row.try_get("generation")?, row.try_get("endpoint_hash")?),
        );
    }

    // 2. Resolve per-request eligibility; collect the insert arrays.
    // `None` marks InactiveLease; `Some(endpoint_hash)` carries the key half
    // that maps insert/duplicate rows back to their requests.
    let resolved: Vec<Option<Vec<u8>>> = requests
        .iter()
        .map(|r| {
            leases
                .get(&(r.author.clone(), r.installation_id.clone()))
                .filter(|(generation, _)| *generation == r.lease_generation)
                .map(|(_, endpoint_hash)| endpoint_hash.clone())
        })
        .collect();
    let mut ins_authors: Vec<&[u8]> = Vec::new();
    let mut ins_installs: Vec<&str> = Vec::new();
    let mut ins_generations: Vec<i64> = Vec::new();
    let mut ins_endpoints: Vec<&[u8]> = Vec::new();
    let mut ins_events: Vec<&[u8]> = Vec::new();
    let mut ins_classes: Vec<&str> = Vec::new();
    let mut ins_expires: Vec<i64> = Vec::new();
    for (request, endpoint_hash) in requests.iter().zip(&resolved) {
        let Some(endpoint_hash) = endpoint_hash else {
            continue;
        };
        ins_authors.push(&request.author);
        ins_installs.push(&request.installation_id);
        ins_generations.push(request.lease_generation);
        ins_endpoints.push(endpoint_hash);
        ins_events.push(&request.event_id);
        ins_classes.push(&request.class);
        ins_expires.push(request.expires_at);
    }

    // 3. One multi-row insert. ON CONFLICT covers both pre-existing jobs and
    //    dedup-key collisions between rows of this same statement.
    let mut job_ids: std::collections::HashMap<(Vec<u8>, Vec<u8>), Uuid> =
        std::collections::HashMap::new();
    let mut inserted_keys: std::collections::HashSet<(Vec<u8>, Vec<u8>)> =
        std::collections::HashSet::new();
    if !ins_events.is_empty() {
        let inserted = sqlx::query(
            r#"
            INSERT INTO push_wake_outbox (
                community_id, author, installation_id, lease_generation,
                endpoint_hash, event_id, class, expires_at
            )
            SELECT $1, a, i, g, eh, ev, c, ex
            FROM UNNEST(
                $2::bytea[], $3::text[], $4::bigint[], $5::bytea[],
                $6::bytea[], $7::text[], $8::bigint[]
            ) AS t(a, i, g, eh, ev, c, ex)
            ON CONFLICT (community_id, endpoint_hash, event_id) DO NOTHING
            RETURNING endpoint_hash, event_id, id
            "#,
        )
        .bind(community.as_uuid())
        .bind(&ins_authors)
        .bind(&ins_installs)
        .bind(&ins_generations)
        .bind(&ins_endpoints)
        .bind(&ins_events)
        .bind(&ins_classes)
        .bind(&ins_expires)
        .fetch_all(&mut *tx)
        .await?;
        for row in inserted {
            let key: (Vec<u8>, Vec<u8>) = (row.try_get("endpoint_hash")?, row.try_get("event_id")?);
            job_ids.insert(key.clone(), row.try_get("id")?);
            inserted_keys.insert(key);
        }
        // 4. Set-wise duplicate lookup for eligible requests the insert
        //    skipped. A separate statement so READ COMMITTED observes a
        //    competing transaction whose unique-key insert completed while
        //    ours waited.
        if inserted_keys.len() < ins_events.len() {
            let dup_rows = sqlx::query(
                r#"
                SELECT endpoint_hash, event_id, id FROM push_wake_outbox
                WHERE community_id = $1
                  AND (endpoint_hash, event_id) IN
                      (SELECT eh, ev FROM UNNEST($2::bytea[], $3::bytea[]) AS t(eh, ev))
                "#,
            )
            .bind(community.as_uuid())
            .bind(&ins_endpoints)
            .bind(&ins_events)
            .fetch_all(&mut *tx)
            .await?;
            for row in dup_rows {
                let key: (Vec<u8>, Vec<u8>) =
                    (row.try_get("endpoint_hash")?, row.try_get("event_id")?);
                job_ids.entry(key).or_insert(row.try_get("id")?);
            }
        }
    }
    tx.commit().await?;

    // First request to claim an inserted key reports Enqueued; later
    // same-key requests in this batch report Duplicate, matching what
    // sequential single-request calls would have returned.
    let mut reported: std::collections::HashSet<(Vec<u8>, Vec<u8>)> =
        std::collections::HashSet::new();
    requests
        .iter()
        .zip(&resolved)
        .map(|(request, endpoint_hash)| {
            let Some(endpoint_hash) = endpoint_hash else {
                return Ok(EnqueueWakeOutcome::InactiveLease);
            };
            let key = (endpoint_hash.clone(), request.event_id.clone());
            let id = *job_ids.get(&key).ok_or_else(|| {
                crate::error::DbError::InvalidData(
                    "wake enqueue resolved neither insert nor duplicate".into(),
                )
            })?;
            Ok(if inserted_keys.contains(&key) && reported.insert(key) {
                EnqueueWakeOutcome::Enqueued(id)
            } else {
                EnqueueWakeOutcome::Duplicate(id)
            })
        })
        .collect()
}

/// One batch of matcher jobs claimed from a single community.
#[derive(Debug, Clone)]
pub struct ClaimedMatchBatch {
    /// Tenant that owns every job in this batch.
    pub community: CommunityId,
    /// Fencing token shared by the whole batch.
    pub claim_id: Uuid,
    /// Claimed jobs whose non-deleted source events loaded successfully.
    pub jobs: Vec<BatchedMatch>,
}

/// One claimed job inside a [`ClaimedMatchBatch`].
#[derive(Debug, Clone)]
pub struct BatchedMatch {
    /// Non-deleted source event loaded after the claim commits.
    pub event: buzz_core::StoredEvent,
    /// Attempt number, starting at one for the first claim.
    pub attempt: i32,
}

/// Exclusively claim up to `limit` due matcher jobs from ONE community and
/// load their non-deleted events in a single query (T2b).
///
/// The batch is community-scoped so downstream lease and membership loads are
/// each one statement. Jobs whose source event is absent or soft-deleted are
/// completed inside this call (privacy-preserving terminal outcome, identical
/// to the previous per-job path). Poison reaping is NOT performed here — it
/// lives in [`reap_exhausted_matches`], off the claim path, because the reap
/// DELETE rescans the pending set and under backlog made every claim slower.
pub async fn claim_due_match_batch(
    pool: &PgPool,
    limit: i64,
    lease_until: DateTime<Utc>,
) -> Result<Option<ClaimedMatchBatch>> {
    claim_due_match_batch_with_loader(
        pool,
        limit,
        lease_until,
        |pool, community, ids| async move {
            let refs: Vec<&[u8]> = ids.iter().map(Vec::as_slice).collect();
            crate::event::get_events_by_ids(&pool, community, &refs).await
        },
    )
    .await
}

async fn claim_due_match_batch_with_loader<F, Fut>(
    pool: &PgPool,
    limit: i64,
    lease_until: DateTime<Utc>,
    load: F,
) -> Result<Option<ClaimedMatchBatch>>
where
    F: FnOnce(PgPool, CommunityId, Vec<Vec<u8>>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<buzz_core::StoredEvent>>>,
{
    let claim_id = Uuid::new_v4();
    let rows = sqlx::query(
        r#"
        WITH target AS (
            SELECT community_id
            FROM push_match_queue
            WHERE attempts < $3
              AND next_attempt_at <= now()
              AND (state = 'pending' OR (state = 'matching' AND lease_until < now()))
            ORDER BY next_attempt_at, created_at
            LIMIT 1
        ),
        candidates AS (
            SELECT q.community_id, q.event_id
            FROM push_match_queue q
            JOIN target t ON q.community_id = t.community_id
            WHERE q.attempts < $3
              AND q.next_attempt_at <= now()
              AND (q.state = 'pending' OR (q.state = 'matching' AND q.lease_until < now()))
            ORDER BY q.next_attempt_at, q.created_at
            FOR UPDATE OF q SKIP LOCKED
            LIMIT $4
        )
        UPDATE push_match_queue q
        SET state='matching', claim_id=$1, lease_until=$2, attempts=q.attempts+1
        FROM candidates c
        WHERE q.community_id=c.community_id AND q.event_id=c.event_id
        RETURNING q.community_id, q.event_id, q.attempts
        "#,
    )
    .bind(claim_id)
    .bind(lease_until)
    .bind(MAX_MATCH_ATTEMPTS)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let community = CommunityId::from_uuid(rows[0].try_get("community_id")?);
    let mut attempts = std::collections::HashMap::with_capacity(rows.len());
    for row in &rows {
        let event_id: Vec<u8> = row.try_get("event_id")?;
        let attempt: i32 = row.try_get("attempts")?;
        attempts.insert(event_id, attempt);
    }
    let ids: Vec<Vec<u8>> = attempts.keys().cloned().collect();
    let events = load(pool.clone(), community, ids).await?;
    let mut jobs = Vec::with_capacity(events.len());
    for event in events {
        let attempt = attempts
            .remove(event.event.id.as_bytes().as_slice())
            .unwrap_or(1);
        jobs.push(BatchedMatch { event, attempt });
    }
    // Whatever is left in `attempts` had no loadable source event: absence and
    // soft deletion are deliberate privacy-preserving terminal outcomes.
    // Query errors above propagate instead, leaving the fenced jobs
    // recoverable after their claim lease expires.
    let gone: Vec<Vec<u8>> = attempts.into_keys().collect();
    if !gone.is_empty() {
        sqlx::query(
            "DELETE FROM push_match_queue \
             WHERE community_id=$1 AND claim_id=$2 AND state='matching' AND event_id = ANY($3)",
        )
        .bind(community.as_uuid())
        .bind(claim_id)
        .bind(&gone)
        .execute(pool)
        .await?;
    }
    if jobs.is_empty() {
        return Ok(None);
    }
    Ok(Some(ClaimedMatchBatch {
        community,
        claim_id,
        jobs,
    }))
}

/// Delete exhausted matcher jobs so a worker crash on the final attempt
/// cannot leave an unclaimable row pinning outbox retention forever.
///
/// Runs on a periodic sweep, never inside the claim path: the scan is not
/// served by the due partial index, so putting it in every claim made claims
/// slower exactly when a backlog needed them fastest.
pub async fn reap_exhausted_matches(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM push_match_queue WHERE attempts >= $1 \
         AND (state='pending' OR (state='matching' AND lease_until < now()))",
    )
    .bind(MAX_MATCH_ATTEMPTS)
    .execute(pool)
    .await?
    .rows_affected())
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

/// Delete matcher jobs from one claimed batch while its fence is held (one
/// statement for any number of jobs). Returns how many rows were completed;
/// jobs whose fence was lost are left for their next claimant.
pub async fn complete_match_batch(
    pool: &PgPool,
    community: CommunityId,
    claim_id: Uuid,
    event_ids: &[Vec<u8>],
) -> Result<u64> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    Ok(sqlx::query(
        "DELETE FROM push_match_queue \
         WHERE community_id=$1 AND claim_id=$2 AND state='matching' AND event_id = ANY($3)",
    )
    .bind(community.as_uuid())
    .bind(claim_id)
    .bind(event_ids)
    .execute(pool)
    .await?
    .rows_affected())
}

/// Release fenced matcher claims from one batch for retry at the supplied
/// time (one statement for any number of jobs).
pub async fn retry_match_batch(
    pool: &PgPool,
    community: CommunityId,
    claim_id: Uuid,
    event_ids: &[Vec<u8>],
    next: DateTime<Utc>,
) -> Result<u64> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    Ok(sqlx::query(
        "UPDATE push_match_queue \
         SET state='pending', claim_id=NULL, lease_until=NULL, next_attempt_at=$4 \
         WHERE community_id=$1 AND claim_id=$2 AND state='matching' AND event_id = ANY($3)",
    )
    .bind(community.as_uuid())
    .bind(claim_id)
    .bind(event_ids)
    .bind(next)
    .execute(pool)
    .await?
    .rows_affected())
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn setwise_enqueue_maps_outcomes_per_request() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let alice = [21; 32];
        let bob = [22; 32];
        let alice_endpoint = [23; 32];
        let bob_endpoint = [24; 32];
        let event_x = [25; 32];
        let event_y = [26; 32];
        activate(&pool, community, &alice, "install", &alice_endpoint, 1).await;
        activate(&pool, community, &bob, "install", &bob_endpoint, 2).await;

        // Pre-existing durable job for (alice, event_x): the batch's matching
        // request must come back Duplicate with the same id.
        let existing = enqueue_one(&pool, community, &alice, &event_x, 1).await;
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
             VALUES ($1, $2, $3, to_timestamp(1), 9, '[]', '', $4)",
        )
        .bind(community.as_uuid())
        .bind(event_y)
        .bind([42_u8; 32])
        .bind([43_u8; 64])
        .execute(&pool)
        .await
        .expect("insert second wake source event");

        let request = |author: [u8; 32], event: [u8; 32], generation: i64| WakeRequest {
            author: author.to_vec(),
            installation_id: "install".into(),
            lease_generation: generation,
            event_id: event.to_vec(),
            class: "default".into(),
            expires_at: i64::MAX / 2,
        };
        let outcomes = enqueue_wakes(
            &pool,
            community,
            &[
                // Duplicate of the pre-existing job.
                request(alice, event_x, 1),
                // Fresh insert.
                request(alice, event_y, 1),
                // Same dedup key within this same batch.
                request(alice, event_y, 1),
                // Stale generation: lease revalidation must reject it.
                request(bob, event_x, 7),
                // Unknown lease entirely.
                request([99; 32], event_x, 1),
                // Fresh insert for the second lease.
                request(bob, event_y, 2),
            ],
        )
        .await
        .expect("set-wise enqueue");

        assert_eq!(outcomes.len(), 6, "index-aligned outcomes");
        assert_eq!(outcomes[0], EnqueueWakeOutcome::Duplicate(existing));
        let EnqueueWakeOutcome::Enqueued(alice_y) = outcomes[1] else {
            panic!(
                "expected fresh insert for (alice, event_y), got {:?}",
                outcomes[1]
            );
        };
        assert_eq!(
            outcomes[2],
            EnqueueWakeOutcome::Duplicate(alice_y),
            "same-batch dedup collision resolves to the row the batch inserted"
        );
        assert_eq!(outcomes[3], EnqueueWakeOutcome::InactiveLease);
        assert_eq!(outcomes[4], EnqueueWakeOutcome::InactiveLease);
        assert!(matches!(outcomes[5], EnqueueWakeOutcome::Enqueued(id) if id != alice_y));

        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_wake_outbox WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count outbox rows");
        assert_eq!(total, 3, "one row per distinct (endpoint, event) key");
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
        // Global claim assertions below require a queue free of other tests'
        // leftovers.
        sqlx::query("DELETE FROM push_match_queue")
            .execute(&pool)
            .await
            .expect("drain matcher queue");
        let community = make_community(&pool).await;
        // The T1b gate only enqueues match jobs for communities with an
        // eligible lease; give this one an active lease first.
        activate(&pool, community, &[77; 32], "install", &[78; 32], 1).await;
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
            claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
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
        // Eligible lease required for the T1b-gated trigger to enqueue.
        activate(&pool, community, &[79; 32], "install", &[80; 32], 1).await;
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "retry me")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign event");
        crate::event::insert_event(&pool, community, &event, None)
            .await
            .expect("insert event");

        let error = claim_due_match_batch_with_loader(
            &pool,
            16,
            Utc::now() - chrono::Duration::seconds(1),
            |_pool, _community, _event_ids| async {
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
            claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
                .await
                .expect("expired claim remains recoverable")
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn matcher_claim_is_exclusive_across_workers() {
        let pool = setup_pool().await;
        // Drain leftovers from other tests sharing this database: a racing
        // worker claiming an unrelated community's stale batch would count as
        // a second success.
        sqlx::query("DELETE FROM push_match_queue")
            .execute(&pool)
            .await
            .expect("drain matcher queue");
        let community = make_community(&pool).await;
        // Eligible lease required for the T1b-gated trigger to enqueue.
        activate(&pool, community, &[81; 32], "install", &[82; 32], 1).await;
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
                claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
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
        // Global claim assertions below require a queue free of other tests'
        // leftovers.
        sqlx::query("DELETE FROM push_match_queue")
            .execute(&pool)
            .await
            .expect("drain matcher queue");
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
        // The reap lives off the claim path now (periodic sweep): a claim must
        // skip the exhausted row, and the sweep must delete it.
        assert!(
            claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
                .await
                .expect("claim skips exhausted matcher")
                .is_none()
        );
        assert_eq!(
            reap_exhausted_matches(&pool)
                .await
                .expect("reap exhausted matcher"),
            1
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

    /// T2b batch contract: one claim returns jobs from exactly ONE community
    /// (so downstream lease/membership loads are single statements), the
    /// set-wise complete and retry honor the claim fence, and a retried job
    /// becomes claimable again.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn batch_claim_is_single_community_and_setwise_ops_honor_the_fence() {
        let pool = setup_pool().await;
        // The batch claim targets the globally oldest due job, so leftover
        // queue rows from other tests sharing this database would hijack the
        // target community. Start from a drained queue.
        sqlx::query("DELETE FROM push_match_queue")
            .execute(&pool)
            .await
            .expect("drain matcher queue");
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        activate(&pool, community_a, &[83; 32], "install", &[84; 32], 1).await;
        activate(&pool, community_b, &[85; 32], "install", &[86; 32], 1).await;
        let mut a_ids = Vec::new();
        for i in 0..3 {
            let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), format!("a{i}"))
                .sign_with_keys(&nostr::Keys::generate())
                .expect("sign event");
            crate::event::insert_event(&pool, community_a, &event, None)
                .await
                .expect("insert community-a event");
            a_ids.push(event.id.as_bytes().to_vec());
        }
        let b_event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "b0")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign event");
        crate::event::insert_event(&pool, community_b, &b_event, None)
            .await
            .expect("insert community-b event");

        let batch = claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
            .await
            .expect("claim first batch")
            .expect("batch present");
        assert_eq!(
            batch.jobs.len(),
            3,
            "batch must take ALL due jobs from one community"
        );
        assert_eq!(batch.community, community_a, "oldest community first");
        assert!(batch.jobs.iter().all(|job| job.attempt == 1));

        let claimed_ids: Vec<Vec<u8>> = batch
            .jobs
            .iter()
            .map(|job| job.event.event.id.as_bytes().to_vec())
            .collect();
        // A stale fence must not complete or retry anything.
        assert_eq!(
            complete_match_batch(&pool, batch.community, Uuid::new_v4(), &claimed_ids)
                .await
                .expect("stale complete"),
            0
        );
        assert_eq!(
            retry_match_batch(
                &pool,
                batch.community,
                Uuid::new_v4(),
                &claimed_ids,
                Utc::now()
            )
            .await
            .expect("stale retry"),
            0
        );
        // Complete two under the real fence, retry the third immediately.
        assert_eq!(
            complete_match_batch(&pool, batch.community, batch.claim_id, &claimed_ids[..2])
                .await
                .expect("complete two"),
            2
        );
        assert_eq!(
            retry_match_batch(
                &pool,
                batch.community,
                batch.claim_id,
                &claimed_ids[2..],
                Utc::now()
            )
            .await
            .expect("retry one"),
            1
        );

        // The retried job is claimable again, but its retry time is later
        // than community B's untouched row, so B's batch comes first.
        let second = claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
            .await
            .expect("claim community-b batch")
            .expect("community-b batch present");
        assert_eq!(second.community, community_b);
        assert_eq!(second.jobs.len(), 1);
        assert_eq!(
            complete_match_batch(
                &pool,
                second.community,
                second.claim_id,
                &[second.jobs[0].event.event.id.as_bytes().to_vec()]
            )
            .await
            .expect("complete community-b job"),
            1
        );

        // Then the retried community-a job, on its second attempt.
        let third = claim_due_match_batch(&pool, 16, Utc::now() + chrono::Duration::minutes(1))
            .await
            .expect("claim retried job")
            .expect("retried job present");
        assert_eq!(third.community, community_a);
        assert_eq!(third.jobs.len(), 1);
        assert_eq!(third.jobs[0].attempt, 2);
        assert_eq!(
            complete_match_batch(
                &pool,
                third.community,
                third.claim_id,
                &[third.jobs[0].event.event.id.as_bytes().to_vec()]
            )
            .await
            .expect("complete retried job"),
            1
        );
    }

    /// T1b lost-wake race, forced (migration 0023): with no eligible lease the
    /// events trigger must skip the match enqueue, and a lease activation
    /// racing an in-flight event insert must be ordered AFTER it by the gate
    /// lock — so the activation's backfill enqueues the event the trigger
    /// skipped, exactly once. Without the shared/exclusive advisory pair, the
    /// trigger could read "no lease" while the activation commits
    /// concurrently, dropping that wake with no retry.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn gate_orders_lease_activation_after_in_flight_event_and_backfills_it() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;

        // Phase 0: no lease anywhere in this community — a committed gated-kind
        // event must not be enqueued.
        let skipped = nostr::EventBuilder::new(nostr::Kind::Custom(9), "gate skips me")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign skipped event");
        crate::event::insert_event(&pool, community, &skipped, None)
            .await
            .expect("insert lease-less event");
        let queued: i64 =
            sqlx::query_scalar("SELECT count(*) FROM push_match_queue WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count queue after lease-less insert");
        assert_eq!(queued, 0, "gate must skip enqueue with no eligible lease");

        // Phase 1: hold an event-insert transaction open past its INSERT. The
        // (non-deferred) trigger has already run: shared gate lock held, EXISTS
        // saw no lease, enqueue skipped — the classic lost-wake window.
        let raced = nostr::EventBuilder::new(nostr::Kind::Custom(9), "raced wake")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign raced event");
        let mut insert_tx = pool.begin().await.expect("begin raced insert");
        sqlx::query(
            "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at) \
             VALUES ($1, $2, $3, now(), 9, '[]', 'raced wake', $4, now())",
        )
        .bind(community.as_uuid())
        .bind(raced.id.as_bytes().as_slice())
        .bind(raced.pubkey.as_bytes().as_slice())
        .bind(raced.sig.serialize().as_slice())
        .execute(&mut *insert_tx)
        .await
        .expect("insert raced event inside held txn");

        // A concurrent activation must block on the exclusive gate lock until
        // the insert transaction resolves.
        let activation = {
            let pool = pool.clone();
            tokio::spawn(async move {
                replace_active_lease(
                    &pool,
                    community,
                    &[91; 32],
                    "install",
                    version(1, 10, 1),
                    ActiveLease {
                        app_profile: "ios-production",
                        endpoint_hash: &[92; 32],
                        endpoint_grant: "opaque-grant",
                        max_class: "default",
                        subscriptions: &serde_json::json!([]),
                    },
                )
                .await
                .expect("activate racing lease")
            })
        };

        // Wait until the activation is provably parked on the advisory lock
        // (not merely unscheduled), then confirm it has not completed.
        let mut parked = false;
        for _ in 0..100 {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted",
            )
            .fetch_one(&pool)
            .await
            .expect("inspect advisory waiters");
            if waiting > 0 {
                parked = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(parked, "activation must block on the exclusive gate lock");
        assert!(!activation.is_finished());

        // Release the event; the activation acquires the gate, and its
        // backfill must enqueue the event the trigger skipped — exactly once.
        insert_tx.commit().await.expect("commit raced insert");
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(10), activation)
                .await
                .expect("activation completes once the gate is free")
                .expect("join activation"),
            ReplaceLeaseOutcome::Accepted
        );
        let backfilled: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT event_id FROM push_match_queue WHERE community_id=$1 ORDER BY created_at",
        )
        .bind(community.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("read backfilled queue");
        assert!(
            backfilled.contains(&raced.id.as_bytes().to_vec()),
            "raced event must be recovered by the activation backfill"
        );

        // Phase 2: with the lease now active, the trigger enqueues directly and
        // the backfill's ON CONFLICT dedup keeps it single.
        let direct = nostr::EventBuilder::new(nostr::Kind::Custom(9), "direct enqueue")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign direct event");
        crate::event::insert_event(&pool, community, &direct, None)
            .await
            .expect("insert post-activation event");
        let per_event: Vec<i64> = sqlx::query_scalar(
            "SELECT count(*) FROM push_match_queue WHERE community_id=$1 GROUP BY event_id",
        )
        .bind(community.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("count queue rows per event");
        assert!(per_event.iter().all(|count| *count == 1));
    }
}
