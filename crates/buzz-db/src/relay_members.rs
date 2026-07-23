//! Relay-level membership persistence (NIP-43).
//!
//! The `relay_members` table is community-scoped: its primary key is
//! `(community_id, pubkey)`. Every read, write, and list is bound to a single
//! `community_id` so that admitting a pubkey to community A never admits it to
//! community B (NIP-43 admission confinement). `pubkey` values are 64-char
//! lowercase hex strings.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;
use crate::CommunityId;

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
    let row = sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// Returns the relay member record for `pubkey` in `community`, or `None`.
pub async fn get_relay_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
) -> Result<Option<RelayMember>> {
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .fetch_optional(pool)
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
    let rows = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members WHERE community_id = $1 ORDER BY created_at ASC",
    )
    .bind(community.as_uuid())
    .fetch_all(pool)
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
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
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
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(role)
    .execute(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(inserted)
}

/// Returns whether a member has persisted acceptance evidence for a policy version.
pub async fn has_join_policy_acceptance(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    policy_version: &str,
) -> Result<bool> {
    let row = sqlx::query(
        "SELECT 1 FROM join_policy_acceptances \
         WHERE community_id = $1 AND pubkey = $2 AND policy_version = $3",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(policy_version)
    .fetch_optional(pool)
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

/// Removes a relay member atomically, refusing to delete the owner.
///
/// Uses a single conditional `DELETE … WHERE role <> 'owner'` so the
/// owner-protection check and the deletion are one atomic operation —
/// no TOCTOU race between a separate read and delete.
pub async fn remove_relay_member(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
) -> Result<RemoveResult> {
    let result = sqlx::query(
        "DELETE FROM relay_members \
         WHERE community_id = $1 AND pubkey = $2 AND role <> 'owner'",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        return Ok(RemoveResult::Removed);
    }

    // rows_affected == 0: either not found or is owner.  One cheap read to
    // distinguish the two cases so callers can return the right error message.
    let exists = sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;

    if exists.is_some() {
        Ok(RemoveResult::IsOwner)
    } else {
        Ok(RemoveResult::NotFound)
    }
}

/// Removes a relay member only if their current role matches `expected_role`.
///
/// The delete and the role check are collapsed into a single
/// `DELETE … WHERE pubkey = $1 AND role = $2`, making the operation atomic —
/// no TOCTOU race between a prior read and this delete.
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
) -> Result<RemoveResult> {
    let result = sqlx::query(
        "DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2 AND role = $3",
    )
    .bind(community.as_uuid())
    .bind(pubkey)
    .bind(expected_role)
    .execute(pool)
    .await?;

    if result.rows_affected() > 0 {
        return Ok(RemoveResult::Removed);
    }

    // rows_affected == 0: either not found or role changed. One cheap read to
    // distinguish the cases so callers can return the right error message.
    let row = sqlx::query("SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2")
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;

    match row {
        None => Ok(RemoveResult::NotFound),
        Some(r) => {
            let role: String = r.try_get("role")?;
            if role == "owner" {
                Ok(RemoveResult::IsOwner)
            } else {
                // Role changed between the caller's check and this delete
                // (e.g., target was promoted to admin). Signal that the
                // caller no longer has authority to remove this target.
                Ok(RemoveResult::RoleMismatch)
            }
        }
    }
}

/// Updates the role of an existing relay member in `community`. Returns `true`
/// if updated.
pub async fn update_relay_member_role(
    pool: &PgPool,
    community: CommunityId,
    pubkey: &str,
    new_role: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE relay_members SET role = $1, updated_at = now() \
         WHERE community_id = $2 AND pubkey = $3 AND role <> 'owner'",
    )
    .bind(new_role)
    .bind(community.as_uuid())
    .bind(pubkey)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Ensures the configured owner pubkey holds the `"owner"` role *in
/// `community`*, and demotes any other owners in that community to `"admin"`.
/// This handles owner rotation: if `RELAY_OWNER_PUBKEY` changes, the old owner
/// is automatically demoted. Scoped to one community — an owner of community A
/// is never bootstrapped into community B.
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
) -> Result<()> {
    let pubkey = owner_pubkey.to_ascii_lowercase();
    let mut tx = pool.begin().await?;

    // 1. Upsert the configured owner for this community.
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET role = 'owner', updated_at = now()",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    // 2. Demote any other owners in this community to admin.
    sqlx::query(
        "UPDATE relay_members SET role = 'admin', updated_at = now() \
         WHERE community_id = $1 AND role = 'owner' AND pubkey <> $2",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
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
    /// The transferee already owns the maximum number of communities.
    /// Enforced atomically inside the transfer transaction so concurrent
    /// transfers to the same recipient cannot both pass the limit.
    LimitReached,
}

/// Maximum number of communities a single pubkey can own. Enforced at the
/// relay layer — the authoritative layer — so that concurrent transfers or
/// transfer-vs-create races cannot both pass a preflight count.
pub const MAX_COMMUNITIES_PER_OWNER: i64 = 3;

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

/// Atomically transfers ownership of `community` to `new_owner_pubkey`.
///
/// Runs in a single transaction:
/// 1. Acquires a transaction-scoped advisory lock on the *transferee* pubkey
///    so that concurrent transfers to the same recipient serialize. The same
///    lock key is also used by `Db::create_community_with_owner` to prevent
///    transfer-vs-create races.
/// 2. Locks the current owner row `FOR UPDATE` and verifies
///    `expected_owner_pubkey` matches. This prevents a stale-owner race where
///    a delayed/retried request overwrites a completed transfer.
/// 3. Enforces the [`MAX_COMMUNITIES_PER_OWNER`] limit on the transferee by
///    counting owned communities inside the same transaction.
/// 4. Upserts `new_owner_pubkey` as `owner` (insert or promote).
/// 5. Demotes every other owner in this community to `member` — **not**
///    `admin`, per product decision: the former owner retains no management
///    capabilities.
///
/// Scoped to one community — an ownership transfer in A never touches B.
pub async fn transfer_ownership(
    pool: &PgPool,
    community: CommunityId,
    new_owner_pubkey: &str,
    expected_owner_pubkey: &str,
) -> Result<TransferResult> {
    let pubkey = new_owner_pubkey.to_ascii_lowercase();
    let expected_owner = expected_owner_pubkey.to_ascii_lowercase();
    let mut tx = pool.begin().await?;

    // 1. Serialize on the transferee so concurrent transfers to the same
    //    recipient cannot both pass the ownership count check.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(owner_count_advisory_lock_key(&pubkey))
        .execute(&mut *tx)
        .await?;

    // 2. Lock the current owner row FOR UPDATE and verify the expected owner.
    //    FOR UPDATE prevents the stale-owner race: a concurrent transfer that
    //    already changed the owner will block on this lock until our txn
    //    completes (or vice versa), and the expected_owner check will fail.
    let existing_owners: Vec<String> = sqlx::query_scalar(
        "SELECT pubkey FROM relay_members \
         WHERE community_id = $1 AND role = 'owner' \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .fetch_all(&mut *tx)
    .await?;

    if existing_owners.is_empty() {
        tx.rollback().await?;
        return Ok(TransferResult::NoOwner);
    }

    // Stale-owner guard: if the current owner doesn't match the expected
    // owner, a concurrent transfer or rotation has already changed hands.
    if !existing_owners.iter().any(|p| p == &expected_owner) {
        tx.rollback().await?;
        return Ok(TransferResult::OwnerConflict);
    }

    // Already the sole owner — no transfer needed.
    if existing_owners.len() == 1 && existing_owners[0] == pubkey {
        tx.rollback().await?;
        return Ok(TransferResult::AlreadyOwner);
    }

    let previous_owner = if existing_owners.len() == 1 {
        Some(existing_owners[0].clone())
    } else {
        existing_owners.iter().find(|p| **p != pubkey).cloned()
    };

    // 3. Enforce the transferee's community ownership limit inside the same
    //    transaction that holds the advisory lock. This is the authoritative
    //    check — kgoose's preflight count is advisory only.
    let owned_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_members WHERE pubkey = $1 AND role = 'owner'",
    )
    .bind(&pubkey)
    .fetch_one(&mut *tx)
    .await?;

    if owned_count >= MAX_COMMUNITIES_PER_OWNER {
        tx.rollback().await?;
        return Ok(TransferResult::LimitReached);
    }

    // 4. Upsert the new owner.
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'owner', NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET role = 'owner', updated_at = now()",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    // 5. Demote all other owners to member (not admin).
    sqlx::query(
        "UPDATE relay_members SET role = 'member', updated_at = now() \
         WHERE community_id = $1 AND role = 'owner' AND pubkey <> $2",
    )
    .bind(community.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(TransferResult::Transferred { previous_owner })
}

/// Migrates existing `pubkey_allowlist` entries into `relay_members` for
/// `community` (the deployment's default community).
///
/// Converts BYTEA pubkeys to lowercase hex text and inserts them as members of
/// `community`. Returns the number of rows inserted, or 0 if:
/// - the `pubkey_allowlist` table doesn't exist, or
/// - `relay_members` already has rows for this community (migration ran in a
///   prior startup).
///
/// The empty-table guard prevents re-adding members that were intentionally
/// removed by an admin after the initial backfill.
pub async fn backfill_from_allowlist(pool: &PgPool, community: CommunityId) -> Result<u64> {
    // Check if pubkey_allowlist table exists.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'pubkey_allowlist')",
    )
    .fetch_one(pool)
    .await?;

    if !exists {
        return Ok(0);
    }

    // Only backfill if this community's relay_members is empty — once it has
    // rows (from a previous backfill or manual admin commands), we must not
    // re-add members that were intentionally removed.
    let has_members: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM relay_members WHERE community_id = $1)")
            .bind(community.as_uuid())
            .fetch_one(pool)
            .await?;

    if has_members {
        return Ok(0);
    }

    let result = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by, created_at) \
         SELECT $1, encode(pubkey, 'hex'), 'member', NULL, added_at \
         FROM pubkey_allowlist \
         WHERE community_id = $1 \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

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

    async fn owned_community(pool: &PgPool) -> (CommunityId, String) {
        let community = make_test_community(pool).await;
        let owner = test_pubkey();
        bootstrap_owner(pool, community, &owner)
            .await
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

        assert!(
            claim_relay_membership(&pool, community, &policy_member, "member", Some(&version),)
                .await
                .expect("claim membership with policy")
        );
        assert!(
            has_join_policy_acceptance(&pool, community, &policy_member, &version)
                .await
                .expect("policy acceptance lookup")
        );

        assert!(
            claim_relay_membership(&pool, community, &legacy_member, "member", None)
                .await
                .expect("legacy claim membership")
        );
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

        let inserted = add_relay_member(&pool, community_a, &pubkey, "member", None)
            .await
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

        bootstrap_owner(&pool, community_a, &owner)
            .await
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

        let result = transfer_ownership(&pool, community, &new_owner, &old_owner)
            .await
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

        let result = transfer_ownership(&pool, community, &owner, &owner)
            .await
            .expect("transfer ownership to self");

        assert_eq!(result, TransferResult::AlreadyOwner);

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

        let result = transfer_ownership(&pool, community, &new_owner, &expected)
            .await
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

        bootstrap_owner(&pool, community_a, &owner_a)
            .await
            .expect("bootstrap owner A");
        bootstrap_owner(&pool, community_b, &owner_b)
            .await
            .expect("bootstrap owner B");

        transfer_ownership(&pool, community_a, &new_owner, &owner_a)
            .await
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

        bootstrap_owner(&pool, community, &old_owner)
            .await
            .expect("bootstrap owner");
        add_relay_member(&pool, community, &existing_member, "member", None)
            .await
            .expect("add member");

        let result = transfer_ownership(&pool, community, &existing_member, &old_owner)
            .await
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

        bootstrap_owner(&pool, community, &old_owner)
            .await
            .expect("bootstrap initial owner");

        // expected_owner_pubkey doesn't match the actual owner — should conflict.
        let result = transfer_ownership(&pool, community, &new_owner, &wrong_expected)
            .await
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

        // Give the transferee 3 communities (the max).
        for _ in 0..3 {
            let c = make_test_community(&pool).await;
            bootstrap_owner(&pool, c, &transferee)
                .await
                .expect("bootstrap transferee community");
        }

        // Create a community owned by `owner` and try to transfer to `transferee`.
        let community = make_test_community(&pool).await;
        bootstrap_owner(&pool, community, &owner)
            .await
            .expect("bootstrap owner");

        let result = transfer_ownership(&pool, community, &transferee, &owner)
            .await
            .expect("transfer to maxed transferee");

        assert_eq!(result, TransferResult::LimitReached);

        // Owner is still owner — transfer did not happen.
        assert_eq!(
            get_relay_member(&pool, community, &owner)
                .await
                .expect("get owner")
                .expect("exists")
                .role,
            "owner"
        );
    }
}
