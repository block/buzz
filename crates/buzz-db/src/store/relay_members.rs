//! Relay-level membership persistence (NIP-43).
//!
//! The `relay_members` table is community-scoped: its primary key is
//! `(community_id, pubkey)`. Every read, write, and list is bound to a single
//! `community_id` so that admitting a pubkey to community A never admits it to
//! community B (NIP-43 admission confinement). `pubkey` values are 64-char
//! lowercase hex strings.

use buzz_core::StoredEvent;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::{observability, replaceable, CommunityId, Db, RouteDecision, RoutePredicate};

/// A single relay member record.
#[derive(Debug, Clone)]
pub struct RelayMember {
    /// 64-char lowercase hex pubkey.
    pub pubkey: String,
    /// Role: `"owner"`, `"admin"`, or `"member"`.
    pub role: String,
    /// Hex pubkey of who added this member, or `None` for bootstrap entries.
    pub added_by: Option<String>,
    /// When the member was added.
    pub created_at: DateTime<Utc>,
    /// When the record was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Returns `true` if `pubkey` (64-char hex) is a member of `community`.
pub async fn is_relay_member(pool: &PgPool, community: CommunityId, pubkey: &str) -> Result<bool> {
    let mut conn =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    is_relay_member_on(&mut conn, community, pubkey).await
}

/// [`is_relay_member`] on a specific session — the replica-routing path runs
/// the lookup on the exact reader connection whose heartbeat observation
/// proved fence coverage.
pub(crate) async fn is_relay_member_on(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    pubkey: &str,
) -> Result<bool> {
    let row = sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(conn)
        .await?;
    Ok(row.is_some())
}

/// Returns `true` if any member of `community` holds the `admin` or `owner`
/// role. Open relays don't *enforce* the roster, but startup
/// (`bootstrap_owner`) and operator provisioning still populate it — this is
/// how the workspace-profile gate detects whether a steward exists.
pub async fn has_admin_or_owner(pool: &PgPool, community: CommunityId) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    let row = sqlx::query(
        "SELECT 1 FROM relay_members \
         WHERE community_id = $1 AND role IN ('admin', 'owner') LIMIT 1",
    )
    .bind(community.as_uuid())
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.is_some())
}

/// Returns the relay member record for `pubkey` in `community`, or `None`.
pub async fn get_relay_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
) -> Result<Option<RelayMember>> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut *connection)
    .await?;

    row.map(|r| -> std::result::Result<RelayMember, sqlx::Error> {
        Ok(RelayMember {
            pubkey: r.try_get("pubkey")?,
            role: r.try_get("role")?,
            added_by: r.try_get("added_by")?,
            created_at: r.try_get("created_at")?,
            updated_at: r.try_get("updated_at")?,
        })
    })
    .transpose()
    .map_err(crate::error::DbError::from)
}

/// Returns all relay members of `community` ordered by `created_at` ascending.
pub async fn list_relay_members(pool: &PgPool, community: CommunityId) -> Result<Vec<RelayMember>> {
    list_relay_members_with_operation(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await
}

async fn list_relay_members_with_operation(
    pool: &PgPool,
    community: CommunityId,
    operation: observability::WriterOperation,
) -> Result<Vec<RelayMember>> {
    let mut connection = observability::acquire_writer(pool, operation).await?;
    let rows = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members WHERE community_id = $1 ORDER BY created_at ASC",
    )
    .bind(community.as_uuid())
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(|r| -> std::result::Result<RelayMember, sqlx::Error> {
            Ok(RelayMember {
                pubkey: r.try_get("pubkey")?,
                role: r.try_get("role")?,
                added_by: r.try_get("added_by")?,
                created_at: r.try_get("created_at")?,
                updated_at: r.try_get("updated_at")?,
            })
        })
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .map_err(crate::error::DbError::from)
}

/// Adds a new relay member to `community`.
///
/// Returns `true` if the row was actually inserted, `false` if the pubkey
/// already existed in this community (idempotent — `ON CONFLICT DO NOTHING` on
/// the `(community_id, pubkey)` primary key).
pub async fn add_relay_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    role: &str,
    added_by: Option<&str>,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<bool>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    let result = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(tx.conn())
    .await?;
    let added = result.rows_affected() > 0;
    commit_membership_write(tx, added, added, relay_keypair).await
}

/// Commit a membership transaction, first writing the NIP-43 snapshot when
/// it `changed` `relay_members`.
pub(crate) async fn commit_membership_write<T>(
    mut tx: crate::AdmittedTx,
    outcome: T,
    changed: bool,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<T>> {
    let snapshot = if changed {
        Some(write_nip43_membership_snapshot(&mut tx, relay_keypair).await?)
    } else {
        None
    };
    tx.commit().await?;
    Ok(MembershipWrite { outcome, snapshot })
}

/// Claims relay membership via an invite and atomically persists policy evidence.
///
/// Returns `true` when membership was inserted, or `false` when the pubkey was
/// already a member. A configured `policy_version` is recorded in the same
/// transaction, so membership cannot be granted without its acceptance record.
pub async fn claim_relay_membership(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    role: &str,
    policy_version: Option<&str>,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<bool>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(role)
    .execute(tx.conn())
    .await?
    .rows_affected()
        > 0;

    if let Some(version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .bind(version)
        .execute(tx.conn())
        .await?;
    }

    commit_membership_write(tx, inserted, inserted, relay_keypair).await
}

/// Returns whether a member has persisted acceptance evidence for a policy version.
pub async fn has_join_policy_acceptance(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    policy_version: &str,
) -> Result<bool> {
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    let row = sqlx::query(
        "SELECT 1 FROM join_policy_acceptances \
         WHERE community_id = $1 AND pubkey = $2 AND policy_version = $3",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(policy_version)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.is_some())
}

/// The result of a relay member removal attempt.
#[derive(Debug, PartialEq)]
pub enum RemoveResult {
    /// Member was successfully removed.
    Removed,
    /// The pubkey belongs to the relay owner — removal is forbidden.
    IsOwner,
    /// No member with the given pubkey exists.
    NotFound,
    /// The member exists but their role doesn't match the expected role.
    RoleMismatch,
}

/// Deletes a membership and its join-policy acceptances inside the caller's
/// admitted transaction.
///
/// With `expected_role`, only a member holding exactly that role is removed;
/// without it, any non-owner is. The member row is locked under that predicate
/// first, so a concurrent acceptance insert (which must lock the member row for
/// its foreign key) either commits before the child delete sees it or fails.
/// Children go before the parent so no foreign-key action is needed. The
/// caller commits.
async fn delete_member_with_acceptances(
    tx: &mut crate::AdmittedTx,
    pubkey: &str,
    expected_role: Option<&str>,
) -> Result<bool> {
    let community = tx.community();
    let locked = sqlx::query(
        "SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2 \
         AND CASE WHEN $3::text IS NULL THEN role <> 'owner' ELSE role = $3 END \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(expected_role)
    .fetch_optional(tx.conn())
    .await?;
    if locked.is_none() {
        return Ok(false);
    }
    sqlx::query("DELETE FROM join_policy_acceptances WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .execute(tx.conn())
        .await?;
    sqlx::query("DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .execute(tx.conn())
        .await?;
    Ok(true)
}

/// Removes a relay member atomically, refusing to delete the owner.
///
/// The owner-protection check and the deletion run in one transaction under a
/// row lock, so there is no TOCTOU race between a separate read and delete.
pub async fn remove_relay_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<RemoveResult>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    if delete_member_with_acceptances(&mut tx, pubkey, None).await? {
        return commit_membership_write(tx, RemoveResult::Removed, true, relay_keypair).await;
    }

    // Nothing was locked: either not found or is owner. One cheap read to
    // distinguish the two cases so callers can return the right error message.
    let exists = sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(tx.conn())
        .await?;

    tx.commit().await?;
    Ok(MembershipWrite::unchanged(if exists.is_some() {
        RemoveResult::IsOwner
    } else {
        RemoveResult::NotFound
    }))
}

/// Removes a relay member only if their current role matches `expected_role`.
///
/// The role check and the delete run in one transaction under a row lock, so
/// there is no TOCTOU race between a prior read and this delete.
///
/// Returns:
/// - `Removed` — row was deleted.
/// - `NotFound` — no member with that pubkey exists.
/// - `IsOwner` — member exists with role `"owner"` (cannot be removed).
/// - `RoleMismatch` — member exists but their role no longer matches
///   `expected_role` (e.g., they were promoted between the caller's read and
///   this delete).
pub async fn remove_relay_member_if_role(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    expected_role: &str,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<RemoveResult>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    if delete_member_with_acceptances(&mut tx, pubkey, Some(expected_role)).await? {
        return commit_membership_write(tx, RemoveResult::Removed, true, relay_keypair).await;
    }

    // Nothing was locked: either not found or role changed. One cheap read to
    // distinguish the cases so callers can return the right error message.
    let row = sqlx::query("SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(tx.conn())
        .await?;

    tx.commit().await?;
    let outcome = match row {
        None => RemoveResult::NotFound,
        Some(r) => {
            let role: String = r.try_get("role")?;
            if role == "owner" {
                RemoveResult::IsOwner
            } else {
                // Role changed between the caller's check and this delete
                // (e.g., target was promoted to admin). Signal that the
                // caller no longer has authority to remove this target.
                RemoveResult::RoleMismatch
            }
        }
    };
    Ok(MembershipWrite::unchanged(outcome))
}

/// Updates the role of an existing relay member in `community`. Returns `true`
/// if updated.
pub async fn update_relay_member_role(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    new_role: &str,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<bool>> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    let result = sqlx::query(
        "UPDATE relay_members SET role = $1, updated_at = now() \
         WHERE community_id = $2 AND pubkey = $3 AND role <> 'owner'",
    )
    .bind(new_role)
    .bind(community.as_uuid())
    .bind(pubkey)
    .execute(tx.conn())
    .await?;
    let updated = result.rows_affected() > 0;
    commit_membership_write(tx, updated, updated, relay_keypair).await
}

/// Ensures the configured owner pubkey holds the `"owner"` role *in
/// `community`*, and demotes any other owners in that community to `"admin"`.
/// This handles owner rotation: if `RELAY_OWNER_PUBKEY` changes, the old owner
/// is automatically demoted only while the community is active and has no
/// durable deletion intent. Scoped to one community — an owner of community A
/// is never bootstrapped into community B. Initial insertion and exact-owner
/// convergence remain allowed because neither rotates existing ownership.
///
/// Runs in a single transaction. Safe to call at every startup — idempotent.
///
/// **Deployment-root authority exception:** This function is called only by
/// startup initialization and legacy operator provisioning
/// (`community_provisioning.rs`). It is NOT an end-user path and does NOT
/// enforce the per-owner community limit (`MAX_COMMUNITIES_PER_OWNER`) or
/// acquire the per-recipient advisory lock. The per-owner limit is an
/// end-user invariant enforced by `create_community_with_owner` and
/// `transfer_ownership`; deployment-root operations may exceed it by design.
pub async fn bootstrap_owner(
    pool: &PgPool,
    community: CommunityId,
    owner_pubkey: &str,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<()>> {
    let write = bootstrap_owner_with_operation(
        pool,
        community,
        owner_pubkey,
        relay_keypair,
        observability::WriterOperation::Bootstrap,
    )
    .await?;
    match write.outcome {
        ProvisionOwnerResult::Applied => Ok(MembershipWrite {
            outcome: (),
            snapshot: write.snapshot,
        }),
        ProvisionOwnerResult::LifecycleConflict => Err(DbError::AccessDenied(
            "community lifecycle freezes owner rotation".to_string(),
        )),
        ProvisionOwnerResult::DeletionPending => Err(DbError::AccessDenied(
            "community deletion is pending".to_string(),
        )),
    }
}

#[derive(Clone, Copy)]
enum OwnerMutationMode {
    Transfer,
    Converge,
}

enum OwnerMutationAdmission {
    Allowed(Vec<String>),
    NotFound,
    LifecycleConflict,
    DeletionPending,
}

async fn lock_owner_mutation_admission(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community: CommunityId,
    proposed_owner: &str,
    mode: OwnerMutationMode,
) -> Result<OwnerMutationAdmission> {
    crate::deletion::lock_community_deletion_shared(tx, community).await?;
    // `FOR NO KEY UPDATE` still serializes with archive (`FOR NO KEY UPDATE`),
    // unarchive and deletion admission (`FOR UPDATE`), but not with the
    // `FOR KEY SHARE` that every foreign-key child insert takes. A membership
    // writer holding a member row or the list lock inserts its kind:13534
    // list into `events` last, so `FOR UPDATE` here would deadlock with it.
    let target = sqlx::query(
        "SELECT archived_at, deletion_state, deleted_at FROM communities \
         WHERE id = $1 FOR NO KEY UPDATE",
    )
    .bind(community.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(target) = target else {
        return Ok(OwnerMutationAdmission::NotFound);
    };
    let existing_owners: Vec<String> = sqlx::query_scalar(
        "SELECT pubkey FROM relay_members \
         WHERE community_id = $1 AND role = 'owner' \
         ORDER BY pubkey FOR UPDATE",
    )
    .bind(community.as_uuid())
    .fetch_all(&mut **tx)
    .await?;

    let changes_existing_owner = match mode {
        OwnerMutationMode::Transfer => true,
        OwnerMutationMode::Converge => {
            !(existing_owners.is_empty()
                || existing_owners.len() == 1 && existing_owners[0] == proposed_owner)
        }
    };
    if !changes_existing_owner {
        return Ok(OwnerMutationAdmission::Allowed(existing_owners));
    }

    if deletion_pending(tx, community).await? {
        return Ok(OwnerMutationAdmission::DeletionPending);
    }

    let archived_at: Option<chrono::DateTime<chrono::Utc>> = target.try_get("archived_at")?;
    let deletion_state: String = target.try_get("deletion_state")?;
    let deleted_at: Option<chrono::DateTime<chrono::Utc>> = target.try_get("deleted_at")?;
    if archived_at.is_some() || deletion_state != "active" || deleted_at.is_some() {
        return Ok(OwnerMutationAdmission::LifecycleConflict);
    }

    Ok(OwnerMutationAdmission::Allowed(existing_owners))
}

/// Whether `community` has a non-aborted deletion request.
///
/// A quiescing or fenced community always has one, so owner mutations that
/// admission refuses report `DeletionPending` rather than a generic lifecycle
/// conflict.
async fn deletion_pending(conn: &mut sqlx::PgConnection, community: CommunityId) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM community_deletion_requests \
         WHERE community_id = $1 AND stage <> 'aborted')",
    )
    .bind(community.as_uuid())
    .fetch_one(conn)
    .await?)
}

async fn bootstrap_owner_with_operation(
    pool: &PgPool,
    community: CommunityId,
    owner_pubkey: &str,
    relay_keypair: &nostr::Keys,
    operation: observability::WriterOperation,
) -> Result<MembershipWrite<ProvisionOwnerResult>> {
    let pubkey = owner_pubkey.to_ascii_lowercase();
    let connection = observability::acquire_writer(pool, operation).await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    if let Err(error) = crate::deletion::guard_community_write(&mut tx, community).await {
        return match error {
            DbError::AccessDenied(_) if deletion_pending(&mut tx, community).await? => Ok(
                MembershipWrite::unchanged(ProvisionOwnerResult::DeletionPending),
            ),
            DbError::AccessDenied(_) => Ok(MembershipWrite::unchanged(
                ProvisionOwnerResult::LifecycleConflict,
            )),
            other => Err(other),
        };
    }

    match lock_owner_mutation_admission(&mut tx, community, &pubkey, OwnerMutationMode::Converge)
        .await?
    {
        OwnerMutationAdmission::Allowed(_) => {}
        OwnerMutationAdmission::NotFound | OwnerMutationAdmission::LifecycleConflict => {
            tx.rollback().await?;
            return Ok(MembershipWrite::unchanged(
                ProvisionOwnerResult::LifecycleConflict,
            ));
        }
        OwnerMutationAdmission::DeletionPending => {
            tx.rollback().await?;
            return Ok(MembershipWrite::unchanged(
                ProvisionOwnerResult::DeletionPending,
            ));
        }
    }

    // 1. Upsert the configured owner for this community. An owner that is
    //    already in place is left untouched, so a no-op startup writes nothing.
    let upserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET role = 'owner', updated_at = now() \
         WHERE relay_members.role <> 'owner'",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    // 2. Demote any other owners in this community to admin.
    let demoted = sqlx::query(
        "UPDATE relay_members SET role = 'admin', updated_at = now() \
         WHERE community_id = $1 AND role = 'owner' AND pubkey <> $2",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let tx = crate::AdmittedTx::admit(
        tx,
        &crate::deletion::DeletionStore::new(pool.clone()),
        community,
    )
    .await?;
    commit_membership_write(
        tx,
        ProvisionOwnerResult::Applied,
        upserted + demoted > 0,
        relay_keypair,
    )
    .await
}

/// The result of a transfer-ownership attempt.
#[derive(Debug, PartialEq)]
pub enum TransferResult {
    /// Transfer completed: the new owner was upserted and the previous
    /// owner(s) demoted to `member`.
    Transferred {
        /// Pubkey of the previous sole owner, if exactly one existed.
        previous_owner: Option<String>,
    },
    /// The new owner pubkey is already the sole owner — nothing to do.
    AlreadyOwner,
    /// No owner row exists for this community (community may not exist).
    NoOwner,
    /// The `expected_owner_pubkey` did not match the current owner. A
    /// concurrent transfer or owner rotation has already changed ownership.
    /// The caller must NOT retry blindly — re-read ownership and re-evaluate.
    OwnerConflict,
    /// The community is archived, quiescing, or deleted; ownership is frozen.
    LifecycleConflict,
    /// Durable deletion intent exists and wins over ownership mutation.
    DeletionPending,
    /// The transferee has reached the active or lifetime community limit.
    /// Enforced atomically inside the transfer transaction so concurrent
    /// transfers to the same recipient cannot both pass the limit.
    LimitReached,
}

/// Result of converging an owner through deployment-root provisioning.
#[derive(Debug, PartialEq, Eq)]
pub enum ProvisionOwnerResult {
    /// The initial owner was inserted or existing ownership converged.
    Applied,
    /// The community lifecycle freezes ownership mutation.
    LifecycleConflict,
    /// Durable deletion intent exists and wins over owner convergence.
    DeletionPending,
}

/// Default maximum number of communities a single pubkey can own. Enforced at
/// the relay layer — the authoritative layer — so that concurrent transfers or
/// transfer-vs-create races cannot both pass a preflight count.
pub const MAX_COMMUNITIES_PER_OWNER: i64 = 50;

/// Effective per-owner community limit for this deployment.
///
/// Reads `BUZZ_MAX_COMMUNITIES_PER_OWNER` once (cached for the process
/// lifetime); a missing, unparsable, or non-positive value falls back to
/// [`MAX_COMMUNITIES_PER_OWNER`]. Lets multi-tenant operators raise the active
/// cap without a source change while keeping the stock default for everyone
/// else. [`MAX_LIFETIME_COMMUNITIES_PER_OWNER`] still applies and counts live
/// ownership, so values above it are unreachable: no owner can hold more live
/// communities than the lifetime cap allows.
pub fn max_communities_per_owner() -> i64 {
    static LIMIT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        effective_owner_limit(
            std::env::var("BUZZ_MAX_COMMUNITIES_PER_OWNER")
                .ok()
                .as_deref(),
        )
    })
}

/// Pure resolution of the owner limit from a raw env value — extracted from
/// [`max_communities_per_owner`] so the parse/fallback rules are testable
/// without process-global env state.
fn effective_owner_limit(raw: Option<&str>) -> i64 {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(MAX_COMMUNITIES_PER_OWNER)
}

/// Stable advisory-lock key for serializing ownership-granting operations
/// (transfer + create) per recipient pubkey. Uses FNV-1a over the hex pubkey
/// so the same recipient always maps to the same lock across processes.
pub fn owner_count_advisory_lock_key(pubkey_hex: &str) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in pubkey_hex.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    h as i64
}

/// Lifetime cap on communities a pubkey may own, counting owner-deleted
/// communities whose tombstones permanently retain their hosts. Bounds
/// create-then-delete host squatting. Absolute: it does not scale with
/// `BUZZ_MAX_COMMUNITIES_PER_OWNER`.
pub const MAX_LIFETIME_COMMUNITIES_PER_OWNER: i64 = 50;

/// One owner's quota usage, read inside the admitting transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnerQuota {
    /// Live ownership plus incomplete owner-deletion reservations.
    pub active: i64,
    /// Live ownership plus every non-aborted owner deletion, completed or not.
    pub lifetime: i64,
}

impl OwnerQuota {
    /// Whether this owner may gain one more community.
    pub fn admits(self) -> bool {
        self.active < max_communities_per_owner()
            && self.lifetime < MAX_LIFETIME_COMMUNITIES_PER_OWNER
    }
}

/// Read an owner's active and lifetime community counts.
///
/// `UNION` deliberately de-duplicates the live membership and deletion row
/// before PostgreSQL purges membership. An active reservation remains until
/// the logical-completion transition records `completed_at`; the lifetime
/// count keeps it forever. Aborted requests restore the community, which is
/// then counted through its live membership.
pub(crate) async fn owner_quota_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_pubkey: &str,
) -> Result<OwnerQuota> {
    let (active, lifetime): (i64, i64) = sqlx::query_as(
        r#"
        WITH owned AS (
            SELECT community_id
            FROM relay_members
            WHERE pubkey = $1 AND role = 'owner'
        ), deleted AS (
            SELECT community_id, completed_at
            FROM community_deletion_requests
            WHERE request_origin = 'owner'
              AND owner_pubkey = $1
              AND stage <> 'aborted'
        )
        SELECT
            (SELECT count(*) FROM (
                SELECT community_id FROM owned
                UNION
                SELECT community_id FROM deleted WHERE completed_at IS NULL
            ) active)::BIGINT,
            (SELECT count(*) FROM (
                SELECT community_id FROM owned
                UNION
                SELECT community_id FROM deleted
            ) lifetime)::BIGINT
        "#,
    )
    .bind(owner_pubkey)
    .fetch_one(&mut **tx)
    .await?;
    Ok(OwnerQuota { active, lifetime })
}

/// Atomically transfers ownership of `community` to `new_owner_pubkey`.
///
/// Runs in a single transaction:
/// 1. Acquires a transaction-scoped advisory lock on the *transferee* pubkey
///    so that concurrent transfers to the same recipient serialize. The same
///    lock key is also used by `Db::create_community_with_owner` to prevent
///    transfer-vs-create races.
/// 2. Locks the community row and rejects archive/non-active lifecycle or any
///    non-aborted deletion request.
///    Owner-deletion admission takes the same row lock, so whichever operation
///    commits first makes the other re-evaluate and conflict.
/// 3. Locks the current owner row `FOR UPDATE` and verifies
///    `expected_owner_pubkey` matches. This prevents a stale-owner race where
///    a delayed/retried request overwrites a completed transfer.
/// 4. Enforces the transferee's active and lifetime limits
///    ([`OwnerQuota::admits`]) inside the same transaction.
/// 5. Upserts `new_owner_pubkey` as `owner` (insert or promote).
/// 6. Demotes every other owner in this community to `member` — **not**
///    `admin`, per product decision: the former owner retains no management
///    capabilities.
///
/// Scoped to one community — an ownership transfer in A never touches B.
pub async fn transfer_ownership(
    pool: &PgPool,
    community: CommunityId,
    new_owner_pubkey: &str,
    expected_owner_pubkey: &str,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<TransferResult>> {
    let pubkey = new_owner_pubkey.to_ascii_lowercase();
    let expected_owner = expected_owner_pubkey.to_ascii_lowercase();
    let connection =
        observability::acquire_writer(pool, observability::WriterOperation::Authorization).await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    if let Err(error) = crate::deletion::guard_community_write(&mut tx, community).await {
        return match error {
            DbError::AccessDenied(_) if deletion_pending(&mut tx, community).await? => {
                Ok(MembershipWrite::unchanged(TransferResult::DeletionPending))
            }
            DbError::AccessDenied(_) => {
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM communities WHERE id = $1)")
                        .bind(community.as_uuid())
                        .fetch_one(&mut *tx)
                        .await?;
                Ok(MembershipWrite::unchanged(if exists {
                    TransferResult::LifecycleConflict
                } else {
                    TransferResult::NoOwner
                }))
            }
            other => Err(other),
        };
    }

    // 1. Serialize on the transferee so concurrent transfers to the same
    //    recipient cannot both pass the ownership count check.
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::Membership,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(owner_count_advisory_lock_key(&pubkey))
            .execute(&mut *tx),
    )
    .await?;

    let existing_owners = match lock_owner_mutation_admission(
        &mut tx,
        community,
        &pubkey,
        OwnerMutationMode::Transfer,
    )
    .await?
    {
        OwnerMutationAdmission::Allowed(existing_owners) => existing_owners,
        OwnerMutationAdmission::NotFound => {
            tx.rollback().await?;
            return Ok(MembershipWrite::unchanged(TransferResult::NoOwner));
        }
        OwnerMutationAdmission::LifecycleConflict => {
            tx.rollback().await?;
            return Ok(MembershipWrite::unchanged(
                TransferResult::LifecycleConflict,
            ));
        }
        OwnerMutationAdmission::DeletionPending => {
            tx.rollback().await?;
            return Ok(MembershipWrite::unchanged(TransferResult::DeletionPending));
        }
    };

    if existing_owners.is_empty() {
        tx.rollback().await?;
        return Ok(MembershipWrite::unchanged(TransferResult::NoOwner));
    }

    // Stale-owner guard: if the current owner doesn't match the expected
    // owner, a concurrent transfer or rotation has already changed hands.
    if !existing_owners.iter().any(|p| p == &expected_owner) {
        tx.rollback().await?;
        return Ok(MembershipWrite::unchanged(TransferResult::OwnerConflict));
    }

    // Already the sole owner — no transfer needed.
    if existing_owners.len() == 1 && existing_owners[0] == pubkey {
        tx.rollback().await?;
        return Ok(MembershipWrite::unchanged(TransferResult::AlreadyOwner));
    }

    let previous_owner = if existing_owners.len() == 1 {
        Some(existing_owners[0].clone())
    } else {
        existing_owners.iter().find(|p| **p != pubkey).cloned()
    };

    // 4. Enforce the transferee's community ownership limit inside the same
    //    transaction that holds the advisory lock. This is the authoritative
    //    check — kgoose's preflight count is advisory only.
    if !owner_quota_in_transaction(&mut tx, &pubkey).await?.admits() {
        tx.rollback().await?;
        return Ok(MembershipWrite::unchanged(TransferResult::LimitReached));
    }

    // 5. Upsert the new owner.
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET role = 'owner', updated_at = now()",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    // 6. Demote all other owners to member (not admin).
    sqlx::query(
        "UPDATE relay_members SET role = 'member', updated_at = now() \
         WHERE community_id = $1 AND role = 'owner' AND pubkey <> $2",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    let tx = crate::AdmittedTx::admit(
        tx,
        &crate::deletion::DeletionStore::new(pool.clone()),
        community,
    )
    .await?;
    commit_membership_write(
        tx,
        TransferResult::Transferred { previous_owner },
        true,
        relay_keypair,
    )
    .await
}

/// Migrates existing `pubkey_allowlist` entries into `relay_members` for
/// `community` (the deployment's default community).
///
/// Converts BYTEA pubkeys to lowercase hex text and inserts them as members of
/// `community`. Returns the number of rows inserted, or 0 if:
/// - the `pubkey_allowlist` table doesn't exist,
/// - `pubkey_allowlist` has no rows for this community, or
/// - `relay_members` already has rows for this community (migration ran in a
///   prior startup).
///
/// The empty-table guard prevents re-adding members that were intentionally
/// removed by an admin after the initial backfill.
pub async fn backfill_from_allowlist(
    pool: &PgPool,
    community: CommunityId,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<u64>> {
    // The common startup case writes nothing: the legacy table is gone, it
    // has no rows for this community, or the community already has members.
    // Answer those with plain reads so a startup with nothing to backfill
    // never needs write admission, and a quiescing deployment community keeps
    // starting as it did before.
    let mut connection =
        observability::acquire_writer(pool, observability::WriterOperation::Bootstrap).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'pubkey_allowlist')",
    )
    .fetch_one(&mut *connection)
    .await?;
    if !exists || community_has_members(&mut connection, community).await? {
        return Ok(MembershipWrite::unchanged(0));
    }
    let has_allowlist_rows: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pubkey_allowlist WHERE community_id = $1)",
    )
    .bind(community.as_uuid())
    .fetch_one(&mut *connection)
    .await?;
    if !has_allowlist_rows {
        return Ok(MembershipWrite::unchanged(0));
    }
    drop(connection);

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Bootstrap,
    )
    .await?;
    // Re-check under admission: another pod may have backfilled meanwhile.
    if community_has_members(tx.conn(), community).await? {
        tx.commit().await?;
        return Ok(MembershipWrite::unchanged(0));
    }
    let result = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by, created_at) \
         SELECT $1, encode(pubkey, 'hex'), 'member', NULL, added_at \
         FROM pubkey_allowlist \
         WHERE community_id = $1 \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .execute(tx.conn())
    .await?;
    let inserted = result.rows_affected();
    commit_membership_write(tx, inserted, inserted > 0, relay_keypair).await
}

/// Whether `community` has any `relay_members` row. Once it does (from a
/// previous backfill or admin commands), the backfill must not re-add members
/// that were intentionally removed.
async fn community_has_members(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM relay_members WHERE community_id = $1)")
            .bind(community.as_uuid())
            .fetch_one(conn)
            .await?,
    )
}

/// A membership mutation's outcome and the NIP-43 snapshot it committed.
///
/// `snapshot` is `Some` exactly when the mutation changed `relay_members`; the
/// kind:13534 list was written in the same transaction, so a committed change
/// always has a matching list. Callers fan the snapshot out to live
/// subscribers after commit.
#[derive(Debug)]
#[must_use = "fan out `snapshot` to live subscribers"]
pub struct MembershipWrite<T> {
    /// What the mutation did.
    pub outcome: T,
    /// The kind:13534 list committed with the change, if anything changed.
    pub snapshot: Option<StoredEvent>,
}

impl<T> MembershipWrite<T> {
    pub(crate) fn unchanged(outcome: T) -> Self {
        Self {
            outcome,
            snapshot: None,
        }
    }
}

/// Write the relay-signed NIP-43 membership list (kind:13534) for the
/// transaction's community, replacing the live one.
///
/// Call it as the last statement before commit of any transaction that
/// changes `relay_members`, so the list commits or rolls back with the change.
/// The per-community list lock is the last explicit lock: every other lock a
/// membership transaction takes comes first, so the final writer reads every
/// committed change. The list insert then takes `FOR KEY SHARE` on the
/// community row through `events.community_id`'s foreign key; owner mutations
/// lock that row `FOR NO KEY UPDATE`, which doesn't conflict, so no lock cycle
/// forms. `created_at` is at least one second
/// past every earlier list's, so the id is new and a client replacing by
/// timestamp never keeps an older list from the same second. A burst of
/// changes runs the timestamp ahead of the clock by up to one second per
/// change; it falls back to the clock once the burst ends.
pub(crate) async fn write_nip43_membership_snapshot(
    tx: &mut crate::AdmittedTx,
    relay_keypair: &nostr::Keys,
) -> Result<StoredEvent> {
    use nostr::{EventBuilder, Kind, Tag, Timestamp};

    let community_id = tx.community();
    let kind_i32 = buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32;
    let pubkey_bytes = relay_keypair.public_key().to_bytes();
    let lock_key = replaceable::event_replacement_lock_key(
        community_id,
        kind_i32,
        pubkey_bytes.as_slice(),
        None,
    );
    observability::observe_advisory_lock(
        observability::LockType::Membership,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(tx.conn()),
    )
    .await?;

    let rows = sqlx::query(
        "SELECT pubkey, role FROM relay_members \
         WHERE community_id = $1 ORDER BY created_at ASC",
    )
    .bind(community_id.as_uuid())
    .fetch_all(tx.conn())
    .await?;
    // Retired lists count too: a list newer than every prior one can't
    // repeat an earlier list's id.
    let newest_created_at: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT max(created_at) FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(kind_i32)
    .bind(pubkey_bytes.as_slice())
    .fetch_one(tx.conn())
    .await?;

    let mut tags: Vec<Tag> = Vec::with_capacity(rows.len() + 1);
    // NIP-70 protected-event marker.
    tags.push(
        Tag::parse(["-"])
            .map_err(|e| DbError::InvalidData(format!("failed to build '-' tag: {e}")))?,
    );
    for row in &rows {
        let pubkey: String = row.try_get("pubkey")?;
        let role: String = row.try_get("role")?;
        tags.push(
            Tag::parse(["member", &pubkey, &role])
                .map_err(|e| DbError::InvalidData(format!("failed to build member tag: {e}")))?,
        );
    }
    let now = Timestamp::now().as_secs();
    let created_at_secs = newest_created_at
        .map(|newest| (newest.timestamp() as u64 + 1).max(now))
        .unwrap_or(now);
    let event = EventBuilder::new(Kind::Custom(kind_i32 as u16), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at_secs))
        .sign_with_keys(relay_keypair)
        .map_err(|e| DbError::InvalidData(format!("failed to sign kind:13534: {e}")))?;

    let created_at = DateTime::from_timestamp(created_at_secs as i64, 0)
        .ok_or(DbError::InvalidTimestamp(created_at_secs as i64))?;
    let received_at = Utc::now();
    let d_tag = crate::event::extract_d_tag(&event);

    // The relay is authoritative: retire every prior list unconditionally.
    sqlx::query(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
         AND channel_id IS NULL AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(kind_i32)
    .bind(pubkey_bytes.as_slice())
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(pubkey_bytes.as_slice())
    .bind(created_at)
    .bind(kind_i32)
    .bind(serde_json::to_value(&event.tags)?)
    .bind(&event.content)
    .bind(event.sig.serialize().as_slice())
    .bind(received_at)
    .bind::<Option<Uuid>>(None)
    .bind(d_tag.as_deref())
    .execute(tx.conn())
    .await?;
    crate::store::event_follow_up::after_admitted_insert(
        tx,
        event.id.as_bytes().as_slice(),
        kind_i32,
        None,
    )
    .await?;

    Ok(StoredEvent::with_received_at(
        event,
        received_at,
        None,
        true,
    ))
}

/// Rewrite `community`'s kind:13534 list from the committed `relay_members`
/// rows, whether or not it already matches.
///
/// The operator repair path for a list that is missing or stale (a
/// deployment that turned on membership after running without it, a manual
/// `relay_members` edit, a restore). Ordinary membership changes never need it.
pub async fn rewrite_nip43_membership_list(
    pool: &PgPool,
    community: CommunityId,
    relay_keypair: &nostr::Keys,
) -> Result<MembershipWrite<()>> {
    let tx = crate::begin_community_write_transaction(
        pool,
        community,
        observability::WriterOperation::Authorization,
    )
    .await?;
    commit_membership_write(tx, (), true, relay_keypair).await
}

impl Db {
    /// Returns `true` if `pubkey` (64-char hex) is a member of `community`.
    ///
    /// Replica-routed on the bounded arm — the one PERMISSION read routed by
    /// explicit product decision (bounded-stale membership beats the 10s
    /// cache it replaced). Admits and revokes may lag by at most the budget
    /// `B`; everything else fails closed to the writer, exactly like
    /// [`Db::query_events_routed_bounded`]. Not precedent for routing other
    /// permission reads.
    #[datastore_span(name = "is_relay_member", system = "postgresql")]
    pub async fn is_relay_member(&self, community: CommunityId, pubkey: &str) -> Result<bool> {
        let path = "relay_membership";
        match self
            .route_read(
                path,
                RoutePredicate::Bounded,
                crate::observability::ReaderOperation::Authorization,
            )
            .await
        {
            RouteDecision::Replica(mut tx, _entry, reason) => {
                match is_relay_member_on(&mut tx, community, pubkey).await {
                    Ok(is_member) => {
                        Self::record_route(path, "replica", reason);
                        Ok(is_member)
                    }
                    // Surface a cancel instead of re-running it on the writer; see
                    // `DbError::is_statement_cancelled` for the trade-off.
                    Err(e) if e.is_statement_cancelled() => {
                        Self::record_route(path, "replica", "statement_cancelled");
                        Err(e)
                    }
                    Err(e) => {
                        tracing::warn!(path, "replica read failed; re-running on writer: {e}");
                        Self::record_route(path, "writer", "replica_error");
                        is_relay_member(&self.pool, community, pubkey).await
                    }
                }
            }
            RouteDecision::Writer => is_relay_member(&self.pool, community, pubkey).await,
        }
    }

    /// Writer-authoritative [`Db::is_relay_member`]: never replica-routed.
    /// For admission fences that run after a removal's disconnect may already
    /// have passed, where a stale replica answer would grant lasting access.
    #[datastore_span(name = "is_relay_member_writer", system = "postgresql")]
    pub async fn is_relay_member_writer(
        &self,
        community: CommunityId,
        pubkey: &str,
    ) -> Result<bool> {
        is_relay_member(&self.pool, community, pubkey).await
    }

    /// Returns the relay member record for `pubkey` in `community`, or `None` if not found.
    #[datastore_span(name = "get_relay_member", system = "postgresql")]
    pub async fn get_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
    ) -> Result<Option<RelayMember>> {
        get_relay_member(&self.pool, community, pubkey).await
    }

    /// Returns all relay members of `community` ordered by `created_at` ascending.
    #[datastore_span(name = "list_relay_members", system = "postgresql")]
    pub async fn list_relay_members(&self, community: CommunityId) -> Result<Vec<RelayMember>> {
        list_relay_members(&self.pool, community).await
    }

    /// Adds a new relay member to `community`.
    ///
    /// Returns `true` if the row was actually inserted, `false` if the pubkey
    /// already existed in `community` (idempotent — `ON CONFLICT DO NOTHING`).
    #[datastore_span(name = "add_relay_member", system = "postgresql")]
    pub async fn add_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
        role: &str,
        added_by: Option<&str>,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<bool>> {
        add_relay_member(&self.pool, community, pubkey, role, added_by, relay_keypair).await
    }

    /// Claims relay membership via an invite and atomically persists the
    /// accepted policy version when a policy is configured.
    #[datastore_span(name = "claim_relay_membership", system = "postgresql")]
    pub async fn claim_relay_membership(
        &self,
        community: CommunityId,
        pubkey: &str,
        role: &str,
        policy_version: Option<&str>,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<bool>> {
        claim_relay_membership(
            &self.pool,
            community,
            pubkey,
            role,
            policy_version,
            relay_keypair,
        )
        .await
    }

    /// Returns whether a member has persisted acceptance evidence for a policy version.
    #[datastore_span(name = "has_join_policy_acceptance", system = "postgresql")]
    pub async fn has_join_policy_acceptance(
        &self,
        community: CommunityId,
        pubkey: &str,
        policy_version: &str,
    ) -> Result<bool> {
        has_join_policy_acceptance(&self.pool, community, pubkey, policy_version).await
    }

    /// Removes a relay member from `community` atomically, refusing to delete the owner.
    #[datastore_span(name = "remove_relay_member", system = "postgresql")]
    pub async fn remove_relay_member(
        &self,
        community: CommunityId,
        pubkey: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<RemoveResult>> {
        remove_relay_member(&self.pool, community, pubkey, relay_keypair).await
    }

    /// Removes a relay member from `community` only if their current role matches `expected_role`.
    ///
    /// Atomic conditional delete — eliminates the TOCTOU race between a
    /// prior role read and the delete. See [`remove_relay_member_if_role`].
    #[datastore_span(name = "remove_relay_member_if_role", system = "postgresql")]
    pub async fn remove_relay_member_if_role(
        &self,
        community: CommunityId,
        pubkey: &str,
        expected_role: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<RemoveResult>> {
        remove_relay_member_if_role(&self.pool, community, pubkey, expected_role, relay_keypair)
            .await
    }

    /// Updates the role of an existing relay member in `community`. Returns `true` if updated.
    #[datastore_span(name = "update_relay_member_role", system = "postgresql")]
    pub async fn update_relay_member_role(
        &self,
        community: CommunityId,
        pubkey: &str,
        new_role: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<bool>> {
        update_relay_member_role(&self.pool, community, pubkey, new_role, relay_keypair).await
    }

    /// Ensures the owner pubkey exists with role `"owner"` in `community`. Called at startup.
    #[datastore_span(name = "bootstrap_owner", system = "postgresql")]
    pub async fn bootstrap_owner(
        &self,
        community: CommunityId,
        owner_pubkey: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<()>> {
        bootstrap_owner(&self.pool, community, owner_pubkey, relay_keypair).await
    }

    /// Ensure an owner during operator-driven community provisioning.
    #[datastore_span(name = "provision_owner", system = "postgresql")]
    pub async fn provision_owner(
        &self,
        community: CommunityId,
        owner_pubkey: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<ProvisionOwnerResult>> {
        bootstrap_owner_with_operation(
            &self.pool,
            community,
            owner_pubkey,
            relay_keypair,
            observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Returns `true` if any member of `community` holds the `admin` or
    /// `owner` role.
    #[datastore_span(name = "has_admin_or_owner", system = "postgresql")]
    pub async fn has_admin_or_owner(&self, community: CommunityId) -> Result<bool> {
        has_admin_or_owner(&self.pool, community).await
    }

    /// Atomically transfers ownership of `community` to `new_owner_pubkey`,
    /// demoting the previous owner(s) to `member`. Verifies
    /// `expected_owner_pubkey` matches the current owner inside the same
    /// transaction to prevent stale-owner races.
    #[datastore_span(name = "transfer_ownership", system = "postgresql")]
    pub async fn transfer_ownership(
        &self,
        community: CommunityId,
        new_owner_pubkey: &str,
        expected_owner_pubkey: &str,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<TransferResult>> {
        transfer_ownership(
            &self.pool,
            community,
            new_owner_pubkey,
            expected_owner_pubkey,
            relay_keypair,
        )
        .await
    }

    /// Migrates existing `pubkey_allowlist` entries into `relay_members` for `community`.
    ///
    /// Idempotent — uses `ON CONFLICT DO NOTHING`. Returns the number of rows
    /// inserted, or 0 if the `pubkey_allowlist` table doesn't exist.
    #[datastore_span(name = "backfill_from_allowlist", system = "postgresql")]
    pub async fn backfill_from_allowlist(
        &self,
        community: CommunityId,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<u64>> {
        backfill_from_allowlist(&self.pool, community, relay_keypair).await
    }

    /// Rewrite `community`'s kind:13534 list from `relay_members`. See
    /// [`rewrite_nip43_membership_list`].
    #[datastore_span(name = "rewrite_nip43_membership_list", system = "postgresql")]
    pub async fn rewrite_nip43_membership_list(
        &self,
        community: CommunityId,
        relay_keypair: &nostr::Keys,
    ) -> Result<MembershipWrite<()>> {
        rewrite_nip43_membership_list(&self.pool, community, relay_keypair).await
    }
}

/// Assert that `community` has exactly one live kind:13534 list signed by
/// `relay_keypair`, that it is `snapshot`, and that it lists exactly the
/// committed `relay_members` rows.
#[cfg(test)]
pub(crate) async fn assert_membership_list_matches(
    pool: &PgPool,
    community: CommunityId,
    relay_keypair: &nostr::Keys,
    snapshot: Option<&StoredEvent>,
) {
    let live: Vec<(Vec<u8>, serde_json::Value)> = sqlx::query_as(
        "SELECT id, tags FROM events \
         WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
         AND channel_id IS NULL AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32)
    .bind(relay_keypair.public_key().to_bytes().as_slice())
    .fetch_all(pool)
    .await
    .expect("read live membership lists");
    assert_eq!(live.len(), 1, "exactly one live membership list");
    let (id, tags) = &live[0];
    let snapshot = snapshot.expect("a membership change returns its list");
    assert_eq!(
        id.as_slice(),
        snapshot.event.id.as_bytes().as_slice(),
        "the returned list is the stored one"
    );
    let tags: Vec<Vec<String>> = serde_json::from_value(tags.clone()).expect("list tags");
    let mut listed: Vec<(String, String)> = tags
        .into_iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("member"))
        .map(|tag| (tag[1].clone(), tag[2].clone()))
        .collect();
    listed.sort();
    let mut members: Vec<(String, String)> = list_relay_members(pool, community)
        .await
        .expect("list relay members")
        .into_iter()
        .map(|member| (member.pubkey, member.role))
        .collect();
    members.sort();
    assert_eq!(listed, members, "the list names every committed member");
}

#[cfg(test)]
mod postgres_tests {
    #[test]
    fn owner_limit_defaults_when_unset_or_invalid() {
        assert_eq!(
            super::effective_owner_limit(None),
            50,
            "stock deployment permits 50 active communities"
        );
        assert_eq!(
            super::effective_owner_limit(Some("not-a-number")),
            super::MAX_COMMUNITIES_PER_OWNER
        );
        assert_eq!(
            super::effective_owner_limit(Some("0")),
            super::MAX_COMMUNITIES_PER_OWNER
        );
        assert_eq!(
            super::effective_owner_limit(Some("-5")),
            super::MAX_COMMUNITIES_PER_OWNER
        );
    }

    #[test]
    fn owner_quota_admits_only_under_active_and_lifetime_caps() {
        let quota = |active, lifetime| super::OwnerQuota { active, lifetime };
        let limit = super::max_communities_per_owner();
        let lifetime = super::MAX_LIFETIME_COMMUNITIES_PER_OWNER;
        assert!(quota(0, 0).admits());
        assert!(quota(limit - 1, lifetime - 1).admits());
        assert!(!quota(limit, limit).admits(), "active cap");
        assert!(
            !quota(0, lifetime).admits(),
            "tombstones count toward lifetime"
        );
    }

    #[test]
    fn owner_limit_honors_positive_override() {
        assert_eq!(super::effective_owner_limit(Some("100")), 100);
        assert_eq!(super::effective_owner_limit(Some(" 12 ")), 12);
    }

    use super::*;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("relay-members-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    fn test_pubkey() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    async fn assert_role(pool: &PgPool, community: CommunityId, pubkey: &str, role: &str) {
        assert_eq!(
            get_relay_member(pool, community, pubkey)
                .await
                .expect("get relay member")
                .map(|member| member.role)
                .as_deref(),
            Some(role)
        );
    }

    async fn acceptances(pool: &PgPool, community: CommunityId, pubkey: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM join_policy_acceptances WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_one(pool)
        .await
        .expect("count acceptances")
    }

    /// Both removal paths delete acceptance evidence themselves: with every
    /// foreign-key delete action retired, nothing cascades. Owners and a role
    /// mismatch keep theirs.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn member_removal_deletes_acceptances_without_cascades() {
        let pool = setup_pool().await;
        crate::test_support::retire_foreign_key_delete_actions(&pool).await;
        let (community, owner) = owned_community(&pool).await;
        let version = "a".repeat(64);
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&owner)
        .bind(&version)
        .execute(&pool)
        .await
        .expect("owner acceptance");
        let member = test_pubkey();
        let admin = test_pubkey();
        for (pubkey, role) in [(&member, "member"), (&admin, "admin")] {
            claim_relay_membership(
                &pool,
                community,
                pubkey,
                role,
                Some(&version),
                &nostr::Keys::generate(),
            )
            .await
            .map(|write| write.outcome)
            .expect("claim membership");
            assert_eq!(acceptances(&pool, community, pubkey).await, 1);
        }

        assert_eq!(
            remove_relay_member(&pool, community, &owner, &nostr::Keys::generate())
                .await
                .map(|write| write.outcome)
                .expect("owner removal"),
            RemoveResult::IsOwner
        );
        assert_eq!(acceptances(&pool, community, &owner).await, 1);
        assert_eq!(
            remove_relay_member_if_role(
                &pool,
                community,
                &admin,
                "member",
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("mismatched removal"),
            RemoveResult::RoleMismatch
        );
        assert_eq!(acceptances(&pool, community, &admin).await, 1);

        assert_eq!(
            remove_relay_member(&pool, community, &member, &nostr::Keys::generate())
                .await
                .map(|write| write.outcome)
                .expect("member removal"),
            RemoveResult::Removed
        );
        assert_eq!(acceptances(&pool, community, &member).await, 0);
        assert_eq!(
            remove_relay_member_if_role(
                &pool,
                community,
                &admin,
                "admin",
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("role removal"),
            RemoveResult::Removed
        );
        assert_eq!(acceptances(&pool, community, &admin).await, 0);
    }

    /// An acceptance inserted concurrently with the removal holds the member
    /// row's key-share lock. Removal must wait for it at its row lock and then
    /// delete it; with no cascade, a removal that took its child snapshot first
    /// would fail the foreign-key check on the member.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn member_removal_waits_for_and_removes_concurrent_acceptance() {
        let pool = setup_pool().await;
        crate::test_support::retire_foreign_key_delete_actions(&pool).await;
        let (community, _owner) = owned_community(&pool).await;
        let member = test_pubkey();
        claim_relay_membership(
            &pool,
            community,
            &member,
            "member",
            Some(&"a".repeat(64)),
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("claim membership");

        let mut inserter = pool.begin().await.expect("begin acceptance insert");
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3)",
        )
        .bind(community.as_uuid())
        .bind(&member)
        .bind("b".repeat(64))
        .execute(&mut *inserter)
        .await
        .expect("insert concurrent acceptance");

        let name = format!("member-remove-{}", Uuid::new_v4().simple());
        let remover = crate::test_support::named_pool(&name).await;
        let removal = tokio::spawn({
            let member = member.clone();
            async move {
                remove_relay_member(&remover, community, &member, &nostr::Keys::generate())
                    .await
                    .map(|write| write.outcome)
            }
        });
        crate::test_support::wait_for_lock_wait(&pool, &name).await;
        inserter
            .commit()
            .await
            .expect("commit concurrent acceptance");

        assert_eq!(
            removal
                .await
                .expect("join removal")
                .expect("removal after concurrent acceptance"),
            RemoveResult::Removed
        );
        assert_eq!(acceptances(&pool, community, &member).await, 0);
    }

    async fn owned_community(pool: &PgPool) -> (CommunityId, String) {
        let community = make_test_community(pool).await;
        let owner = test_pubkey();
        bootstrap_owner(pool, community, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner");
        (community, owner)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn invite_claim_persists_policy_version_and_legacy_claim_does_not() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let policy_member = test_pubkey();
        let legacy_member = test_pubkey();
        let version = "a".repeat(64);

        assert!(claim_relay_membership(
            &pool,
            community,
            &policy_member,
            "member",
            Some(&version),
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("claim membership with policy"));
        assert!(
            has_join_policy_acceptance(&pool, community, &policy_member, &version)
                .await
                .expect("policy acceptance lookup")
        );

        assert!(claim_relay_membership(
            &pool,
            community,
            &legacy_member,
            "member",
            None,
            &nostr::Keys::generate()
        )
        .await
        .map(|write| write.outcome)
        .expect("legacy claim membership"));
        assert!(
            !has_join_policy_acceptance(&pool, community, &legacy_member, &version)
                .await
                .expect("legacy acceptance lookup")
        );
    }

    /// NIP-43 admission confinement: a pubkey admitted to community A is *not*
    /// admitted to community B. This is the exact mutation #1285 targets — a
    /// `WHERE pubkey = $1` membership check (no community predicate) would let an
    /// A-member authenticate against B. We add the pubkey only to A and assert
    /// every read path (`is_relay_member`, `get_relay_member`, `list_relay_members`)
    /// confines it to A.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn membership_is_confined_to_its_community() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        // 64-char lowercase hex, unique per run so reruns don't collide.
        let pubkey = test_pubkey();

        let inserted = add_relay_member(
            &pool,
            community_a,
            &pubkey,
            "member",
            None,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("add member to community A");
        assert!(inserted, "first insert into A should report inserted");

        // is_relay_member: member of A, NOT of B.
        assert!(
            is_relay_member(&pool, community_a, &pubkey)
                .await
                .expect("is_relay_member A"),
            "pubkey must be a member of community A"
        );
        assert!(
            !is_relay_member(&pool, community_b, &pubkey)
                .await
                .expect("is_relay_member B"),
            "pubkey admitted to A must NOT be a member of B (admission confinement)"
        );

        // get_relay_member (used by the NIP-OA owner check + admin role lookups):
        // resolves in A, absent in B.
        assert!(
            get_relay_member(&pool, community_a, &pubkey)
                .await
                .expect("get_relay_member A")
                .is_some(),
            "get_relay_member must resolve in community A"
        );
        assert!(
            get_relay_member(&pool, community_b, &pubkey)
                .await
                .expect("get_relay_member B")
                .is_none(),
            "get_relay_member must not resolve the A pubkey in community B"
        );

        // list_relay_members: B's list never contains A's member.
        let list_a = list_relay_members(&pool, community_a)
            .await
            .expect("list A");
        assert!(
            list_a.iter().any(|m| m.pubkey == pubkey),
            "community A list must contain the admitted pubkey"
        );
        let list_b = list_relay_members(&pool, community_b)
            .await
            .expect("list B");
        assert!(
            list_b.iter().all(|m| m.pubkey != pubkey),
            "community B list must not contain A's member"
        );
    }

    /// Owner bootstrap is community-scoped: bootstrapping the owner in A does not
    /// make that pubkey an owner (or member) of B. Guards against a global
    /// `INSERT ... (pubkey, role)` bootstrap leaking the owner across tenants.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_bootstrap_is_confined_to_its_community() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let owner = test_pubkey();

        bootstrap_owner(&pool, community_a, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner in A");

        let in_a = get_relay_member(&pool, community_a, &owner)
            .await
            .expect("get owner A")
            .expect("owner exists in A");
        assert_eq!(in_a.role, "owner", "bootstrapped pubkey must be owner in A");

        assert!(
            !is_relay_member(&pool, community_b, &owner)
                .await
                .expect("is_relay_member B"),
            "owner bootstrapped in A must NOT be a member of B"
        );
    }

    /// Transfer ownership: upserts new owner, demotes previous owner to
    /// `member` (not `admin`), and returns the previous owner's pubkey.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_demotes_old_owner_to_member() {
        let pool = setup_pool().await;
        let (community, old_owner) = owned_community(&pool).await;
        let new_owner = test_pubkey();

        let result = transfer_ownership(
            &pool,
            community,
            &new_owner,
            &old_owner,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer ownership");

        assert_eq!(
            result,
            TransferResult::Transferred {
                previous_owner: Some(old_owner.clone()),
            }
        );

        assert_role(&pool, community, &new_owner, "owner").await;
        assert_role(&pool, community, &old_owner, "member").await;
    }

    /// Transferring to the current sole owner is a no-op (`AlreadyOwner`).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_already_owner_is_noop() {
        let pool = setup_pool().await;
        let (community, owner) = owned_community(&pool).await;

        let result = transfer_ownership(&pool, community, &owner, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("transfer ownership to self");

        assert_eq!(result, TransferResult::AlreadyOwner);

        assert_role(&pool, community, &owner, "owner").await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_rejects_archived_community_without_membership_change() {
        let pool = setup_pool().await;
        let (community, owner) = owned_community(&pool).await;
        let replacement = test_pubkey();
        sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .expect("archive community");

        let result = transfer_ownership(
            &pool,
            community,
            &replacement,
            &owner,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer archived community");

        assert_eq!(result, TransferResult::LifecycleConflict);
        assert_role(&pool, community, &owner, "owner").await;
        assert!(
            get_relay_member(&pool, community, &replacement)
                .await
                .expect("get replacement")
                .is_none(),
            "archived transfer must not add the replacement owner"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn provision_owner_still_bootstraps_initial_owner() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let owner = test_pubkey();
        let db = Db::from_pool(pool.clone());

        assert_eq!(
            db.provision_owner(community, &owner, &nostr::Keys::generate())
                .await
                .map(|write| write.outcome)
                .expect("provision initial owner"),
            ProvisionOwnerResult::Applied
        );
        assert_role(&pool, community, &owner, "owner").await;
    }

    /// Transferring a community with no owner row returns `NoOwner`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_no_owner_returns_no_owner() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let new_owner = test_pubkey();
        let expected = test_pubkey();

        // No bootstrap — community exists but has no owner row.

        let result = transfer_ownership(
            &pool,
            community,
            &new_owner,
            &expected,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer ownership on empty community");

        assert_eq!(result, TransferResult::NoOwner);
    }

    /// Transfer ownership is community-scoped: transferring in A does not
    /// affect ownership in B.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_is_community_scoped() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let owner_a = test_pubkey();
        let owner_b = test_pubkey();
        let new_owner = test_pubkey();

        bootstrap_owner(&pool, community_a, &owner_a, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner A");
        bootstrap_owner(&pool, community_b, &owner_b, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner B");

        transfer_ownership(
            &pool,
            community_a,
            &new_owner,
            &owner_a,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer A");

        assert_role(&pool, community_a, &new_owner, "owner").await;
        assert_role(&pool, community_a, &owner_a, "member").await;
        assert_role(&pool, community_b, &owner_b, "owner").await;
        assert!(
            !is_relay_member(&pool, community_b, &new_owner)
                .await
                .expect("is_relay_member B"),
            "new owner of A must NOT be a member of B"
        );
    }

    /// Transfer ownership to someone who is already a member promotes them to
    /// owner and demotes the old owner to member.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_promotes_existing_member() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let old_owner = test_pubkey();
        let existing_member = test_pubkey();

        bootstrap_owner(&pool, community, &old_owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner");
        add_relay_member(
            &pool,
            community,
            &existing_member,
            "member",
            None,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("add member");

        let result = transfer_ownership(
            &pool,
            community,
            &existing_member,
            &old_owner,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer to existing member");

        assert!(matches!(result, TransferResult::Transferred { .. }));

        assert_eq!(
            get_relay_member(&pool, community, &existing_member)
                .await
                .expect("get new owner")
                .expect("exists")
                .role,
            "owner"
        );
        assert_eq!(
            get_relay_member(&pool, community, &old_owner)
                .await
                .expect("get old owner")
                .expect("exists")
                .role,
            "member"
        );
    }

    /// Transfer returns `OwnerConflict` when `expected_owner_pubkey` doesn't
    /// match the current owner — simulates a stale/delayed request after a
    /// concurrent transfer has already changed ownership.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_returns_owner_conflict_when_expected_mismatches() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let old_owner = test_pubkey();
        let new_owner = test_pubkey();
        let wrong_expected = test_pubkey();

        bootstrap_owner(&pool, community, &old_owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap initial owner");

        // expected_owner_pubkey doesn't match the actual owner — should conflict.
        let result = transfer_ownership(
            &pool,
            community,
            &new_owner,
            &wrong_expected,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("transfer ownership with wrong expected");

        assert_eq!(result, TransferResult::OwnerConflict);

        // Old owner is still owner — nothing changed.
        assert_eq!(
            get_relay_member(&pool, community, &old_owner)
                .await
                .expect("get old owner")
                .expect("exists")
                .role,
            "owner"
        );
        // New owner was not added.
        assert!(
            get_relay_member(&pool, community, &new_owner)
                .await
                .expect("get new owner")
                .is_none(),
            "new owner must not be added on conflict"
        );
    }

    /// Transfer returns `LimitReached` when the transferee already owns the
    /// maximum number of communities. The limit is enforced inside the
    /// transfer transaction at the relay layer.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transfer_ownership_returns_limit_reached_for_maxed_transferee() {
        let pool = setup_pool().await;
        let owner = test_pubkey();
        let transferee = test_pubkey();

        // At 49 ownerships, the fiftieth transfer succeeds; at 50, the
        // fifty-first is rejected by the same transaction-side admission.
        for _ in 0..(MAX_COMMUNITIES_PER_OWNER - 1) {
            let c = make_test_community(&pool).await;
            bootstrap_owner(&pool, c, &transferee, &nostr::Keys::generate())
                .await
                .map(|write| write.outcome)
                .expect("bootstrap transferee community");
        }

        let fiftieth = make_test_community(&pool).await;
        bootstrap_owner(&pool, fiftieth, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner");
        assert!(matches!(
            transfer_ownership(
                &pool,
                fiftieth,
                &transferee,
                &owner,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("fiftieth transfer"),
            TransferResult::Transferred { .. }
        ));
        assert_eq!(
            get_relay_member(&pool, fiftieth, &transferee)
                .await
                .expect("get transferee")
                .expect("exists")
                .role,
            "owner"
        );
        assert_eq!(
            get_relay_member(&pool, fiftieth, &owner)
                .await
                .expect("get previous owner")
                .expect("exists")
                .role,
            "member"
        );

        let fifty_first = make_test_community(&pool).await;
        bootstrap_owner(&pool, fifty_first, &owner, &nostr::Keys::generate())
            .await
            .map(|write| write.outcome)
            .expect("bootstrap owner");
        assert_eq!(
            transfer_ownership(
                &pool,
                fifty_first,
                &transferee,
                &owner,
                &nostr::Keys::generate()
            )
            .await
            .map(|write| write.outcome)
            .expect("fifty-first transfer"),
            TransferResult::LimitReached
        );
        assert_eq!(
            get_relay_member(&pool, fifty_first, &owner)
                .await
                .expect("get owner")
                .expect("exists")
                .role,
            "owner"
        );
        assert!(
            get_relay_member(&pool, fifty_first, &transferee)
                .await
                .expect("get transferee")
                .is_none(),
            "rejected transfer must not add transferee"
        );
    }

    /// Id of the live kind:13534 list, if any.
    async fn live_list_id(
        pool: &PgPool,
        community: CommunityId,
        keys: &nostr::Keys,
    ) -> Option<Vec<u8>> {
        sqlx::query_scalar(
            "SELECT id FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 \
             AND channel_id IS NULL AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32)
        .bind(keys.public_key().to_bytes().as_slice())
        .fetch_optional(pool)
        .await
        .expect("read live list id")
    }

    /// Every `relay_members` writer commits the matching list with its change,
    /// and a call that changes nothing writes no list.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn every_membership_write_commits_its_list() {
        let pool = setup_pool().await;
        let db = Db::from_pool(pool.clone());
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let owner = test_pubkey();
        let member = test_pubkey();
        let joiner = test_pubkey();
        let new_owner = test_pubkey();

        let write = bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("bootstrap owner");
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        // A restart with the owner in place changes nothing and writes no list.
        let live = live_list_id(&pool, community, &keys).await;
        let write = bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("re-bootstrap owner");
        assert!(write.snapshot.is_none(), "no-op bootstrap writes no list");
        assert_eq!(live_list_id(&pool, community, &keys).await, live);

        // Demoting a legacy co-owner alone still rewrites the list.
        let legacy_owner = test_pubkey();
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'owner')",
        )
        .bind(community.as_uuid())
        .bind(&legacy_owner)
        .execute(&pool)
        .await
        .expect("seed legacy co-owner");
        let write = bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("bootstrap demotes co-owner");
        assert!(write.snapshot.is_some(), "a demotion writes a list");
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let write = add_relay_member(&pool, community, &member, "member", Some(&owner), &keys)
            .await
            .expect("add member");
        assert!(write.outcome);
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
        let live = live_list_id(&pool, community, &keys).await;

        let write = add_relay_member(&pool, community, &member, "member", Some(&owner), &keys)
            .await
            .expect("re-add member");
        assert!(!write.outcome);
        assert!(write.snapshot.is_none(), "no change writes no list");
        assert_eq!(live_list_id(&pool, community, &keys).await, live);

        let write = claim_relay_membership(&pool, community, &joiner, "member", None, &keys)
            .await
            .expect("claim membership");
        assert!(write.outcome);
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let write = update_relay_member_role(&pool, community, &member, "admin", &keys)
            .await
            .expect("promote member");
        assert!(write.outcome);
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let write = remove_relay_member_if_role(&pool, community, &member, "member", &keys)
            .await
            .expect("role-mismatched removal");
        assert_eq!(write.outcome, RemoveResult::RoleMismatch);
        assert!(write.snapshot.is_none(), "no change writes no list");

        let write = remove_relay_member_if_role(&pool, community, &member, "admin", &keys)
            .await
            .expect("remove admin");
        assert_eq!(write.outcome, RemoveResult::Removed);
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let write = remove_relay_member(&pool, community, &joiner, &keys)
            .await
            .expect("remove joiner");
        assert_eq!(write.outcome, RemoveResult::Removed);
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let write = transfer_ownership(&pool, community, &new_owner, &owner, &keys)
            .await
            .expect("transfer ownership");
        assert!(matches!(write.outcome, TransferResult::Transferred { .. }));
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        let provisioned = make_test_community(&pool).await;
        let write = db
            .provision_owner(provisioned, &owner, &keys)
            .await
            .expect("provision owner");
        assert_eq!(write.outcome, ProvisionOwnerResult::Applied);
        assert_membership_list_matches(&pool, provisioned, &keys, write.snapshot.as_ref()).await;

        let backfilled = make_test_community(&pool).await;
        sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
            .bind(backfilled.as_uuid())
            .bind(vec![0x5a_u8; 32])
            .execute(&pool)
            .await
            .expect("seed legacy allowlist row");
        let write = backfill_from_allowlist(&pool, backfilled, &keys)
            .await
            .expect("backfill allowlist");
        assert_eq!(write.outcome, 1);
        assert_membership_list_matches(&pool, backfilled, &keys, write.snapshot.as_ref()).await;
    }

    /// The operator repair rewrites a missing or stale list from the stored
    /// members.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn rewrite_repairs_a_missing_or_stale_list() {
        let pool = setup_pool().await;
        let db = Db::from_pool(pool.clone());
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let member = test_pubkey();
        let _ = bootstrap_owner(&pool, community, &test_pubkey(), &keys)
            .await
            .expect("bootstrap owner");
        let _ = add_relay_member(&pool, community, &member, "member", None, &keys)
            .await
            .expect("add member");

        // Stale: a role edited behind the list's back.
        sqlx::query(
            "UPDATE relay_members SET role = 'admin' WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(&member)
        .execute(&pool)
        .await
        .expect("edit role by hand");
        let write = db
            .rewrite_nip43_membership_list(community, &keys)
            .await
            .expect("repair stale list");
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;

        // Missing: no live list at all.
        sqlx::query(
            "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 \
             AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32)
        .execute(&pool)
        .await
        .expect("retire the live list");
        assert!(live_list_id(&pool, community, &keys).await.is_none());
        let write = db
            .rewrite_nip43_membership_list(community, &keys)
            .await
            .expect("repair missing list");
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
    }

    /// The list write is part of the membership transaction: when it fails,
    /// the membership change rolls back and the previous list stays live.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_failed_list_write_rolls_back_the_membership_change() {
        let pool = setup_pool().await;
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let bootstrapped = bootstrap_owner(&pool, community, &test_pubkey(), &keys)
            .await
            .expect("bootstrap owner");
        assert!(bootstrapped.snapshot.is_some(), "bootstrap writes a list");
        let before = live_list_id(&pool, community, &keys).await;
        let joiner = test_pubkey();

        // Hold the community's list lock, so the join blocks on its final
        // statement, then cancel that statement to fail the list write.
        let lock_key = replaceable::event_replacement_lock_key(
            community,
            buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
            keys.public_key().to_bytes().as_slice(),
            None,
        );
        let mut holder = pool.begin().await.expect("begin lock holder");
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *holder)
            .await
            .expect("hold list lock");

        let join = tokio::spawn({
            let pool = pool.clone();
            let keys = keys.clone();
            let joiner = joiner.clone();
            async move { add_relay_member(&pool, community, &joiner, "member", None, &keys).await }
        });
        let (classid, objid) = (
            (lock_key as u64 >> 32) as i64,
            (lock_key as u64 & 0xffff_ffff) as i64,
        );
        let mut cancelled = false;
        for _ in 0..200 {
            cancelled = sqlx::query_scalar(
                "SELECT coalesce(bool_or(pg_cancel_backend(pid)), false) FROM pg_locks \
                 WHERE locktype = 'advisory' AND NOT granted \
                 AND classid = $1::bigint::oid AND objid = $2::bigint::oid AND objsubid = 1",
            )
            .bind(classid)
            .bind(objid)
            .fetch_one(&pool)
            .await
            .expect("cancel blocked list write");
            if cancelled {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(cancelled, "the join must block on the list lock");
        let result = join.await.expect("join task");
        holder.rollback().await.expect("release list lock");

        assert!(result.is_err(), "a failed list write fails the join");
        assert!(
            !is_relay_member(&pool, community, &joiner)
                .await
                .expect("joiner membership"),
            "the join rolled back with its list"
        );
        assert_eq!(live_list_id(&pool, community, &keys).await, before);
    }

    /// Concurrent joins serialize on the list lock, so the final list names
    /// every member that committed.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_joins_all_land_in_the_list() {
        let pool = setup_pool().await;
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let mut joins = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let keys = keys.clone();
            joins.spawn(async move {
                add_relay_member(&pool, community, &test_pubkey(), "member", None, &keys).await
            });
        }
        let mut newest: Option<StoredEvent> = None;
        while let Some(joined) = joins.join_next().await {
            let write = joined.expect("join task").expect("concurrent join");
            assert!(write.outcome);
            let snapshot = write.snapshot.expect("each join writes a list");
            if newest
                .as_ref()
                .is_none_or(|prior| snapshot.event.created_at > prior.event.created_at)
            {
                newest = Some(snapshot);
            }
        }
        assert_eq!(
            list_relay_members(&pool, community)
                .await
                .expect("members")
                .len(),
            8
        );
        assert_membership_list_matches(&pool, community, &keys, newest.as_ref()).await;
    }

    /// Wait until some backend is blocked behind `holder_pid`.
    async fn wait_until_blocked_by(pool: &PgPool, holder_pid: i32) {
        for _ in 0..400 {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                 WHERE $1 = ANY(pg_blocking_pids(pid)))",
            )
            .bind(holder_pid)
            .fetch_one(pool)
            .await
            .expect("read blocked backends");
            if blocked {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("nothing blocked behind backend {holder_pid}");
    }

    /// Run `transfer_ownership` in the background.
    fn spawn_transfer(
        pool: &PgPool,
        community: CommunityId,
        new_owner: &str,
        owner: &str,
        keys: &nostr::Keys,
    ) -> tokio::task::JoinHandle<Result<MembershipWrite<TransferResult>>> {
        let (pool, keys) = (pool.clone(), keys.clone());
        let (new_owner, owner) = (new_owner.to_owned(), owner.to_owned());
        tokio::spawn(async move {
            transfer_ownership(&pool, community, &new_owner, &owner, &keys).await
        })
    }

    /// A list repair that holds the list lock while a transfer waits on it
    /// still inserts its list: the transfer's community row lock doesn't
    /// block the insert's foreign-key `FOR KEY SHARE`, so both commit.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_transfer_and_list_repair_both_commit() {
        let pool = setup_pool().await;
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let owner = test_pubkey();
        let new_owner = test_pubkey();
        let _ = bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("bootstrap owner");

        let mut repair = crate::begin_community_write_transaction(
            &pool,
            community,
            observability::WriterOperation::Authorization,
        )
        .await
        .expect("begin repair");
        let repair_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(repair.conn())
            .await
            .expect("repair pid");
        let lock_key = replaceable::event_replacement_lock_key(
            community,
            buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST as i32,
            keys.public_key().to_bytes().as_slice(),
            None,
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(repair.conn())
            .await
            .expect("hold list lock");

        // The transfer locks the community row, then waits on the list lock.
        let transfer = spawn_transfer(&pool, community, &new_owner, &owner, &keys);
        wait_until_blocked_by(&pool, repair_pid).await;

        let repaired = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            write_nip43_membership_snapshot(&mut repair, &keys).await?;
            repair.commit().await
        })
        .await
        .expect("repair finishes");
        repaired.expect("repair commits alongside the transfer");
        let write = tokio::time::timeout(std::time::Duration::from_secs(30), transfer)
            .await
            .expect("transfer finishes")
            .expect("transfer task")
            .expect("transfer commits alongside the repair");
        assert!(matches!(write.outcome, TransferResult::Transferred { .. }));
        assert_role(&pool, community, &new_owner, "owner").await;
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
    }

    /// Removing the transferee while a transfer waits on its member row:
    /// the removal's list insert doesn't wait on the transfer's community
    /// row lock, so the removal commits and the transfer then re-adds the
    /// transferee as owner.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn owner_transfer_and_member_removal_both_commit() {
        let pool = setup_pool().await;
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let owner = test_pubkey();
        let member = test_pubkey();
        let _ = bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("bootstrap owner");
        let _ = add_relay_member(&pool, community, &member, "member", None, &keys)
            .await
            .expect("add member");

        let mut removal = crate::begin_community_write_transaction(
            &pool,
            community,
            observability::WriterOperation::Authorization,
        )
        .await
        .expect("begin removal");
        let removal_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(removal.conn())
            .await
            .expect("removal pid");
        assert!(delete_member_with_acceptances(&mut removal, &member, None)
            .await
            .expect("delete member"));

        // The transfer locks the community row, then waits on the member row.
        let transfer = spawn_transfer(&pool, community, &member, &owner, &keys);
        wait_until_blocked_by(&pool, removal_pid).await;

        let removed = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            commit_membership_write(removal, RemoveResult::Removed, true, &keys),
        )
        .await
        .expect("removal finishes")
        .expect("removal commits alongside the transfer");
        assert!(removed.snapshot.is_some());
        let write = tokio::time::timeout(std::time::Duration::from_secs(30), transfer)
            .await
            .expect("transfer finishes")
            .expect("transfer task")
            .expect("transfer commits alongside the removal");
        assert!(matches!(write.outcome, TransferResult::Transferred { .. }));
        assert_role(&pool, community, &member, "owner").await;
        assert_role(&pool, community, &owner, "member").await;
        assert_membership_list_matches(&pool, community, &keys, write.snapshot.as_ref()).await;
    }

    /// Returning to an earlier membership within the same second yields a new
    /// list with a later `created_at`, never the earlier list's id.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn same_second_return_to_an_earlier_membership_writes_a_new_list() {
        let pool = setup_pool().await;
        let keys = nostr::Keys::generate();
        let community = make_test_community(&pool).await;
        let owner = test_pubkey();
        let guest = test_pubkey();

        let mut lists = vec![bootstrap_owner(&pool, community, &owner, &keys)
            .await
            .expect("bootstrap owner")
            .snapshot
            .expect("owner list")];
        for _ in 0..2 {
            let added = add_relay_member(&pool, community, &guest, "member", None, &keys)
                .await
                .expect("add guest");
            lists.push(added.snapshot.expect("add list"));
            let removed = remove_relay_member(&pool, community, &guest, &keys)
                .await
                .expect("remove guest");
            assert_eq!(removed.outcome, RemoveResult::Removed);
            lists.push(removed.snapshot.expect("remove list"));
        }
        for pair in lists.windows(2) {
            assert!(
                pair[1].event.created_at > pair[0].event.created_at,
                "each list is newer than the one it replaces"
            );
        }
        let mut ids: Vec<_> = lists.iter().map(|list| list.event.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), lists.len(), "no list repeats an earlier id");
        assert_membership_list_matches(&pool, community, &keys, lists.last()).await;
    }
}
