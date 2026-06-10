//! Relay-level membership persistence (NIP-43).
//!
//! The `relay_members` table stores pubkeys (hex), roles, and audit metadata.
//! All pubkey values are 64-char lowercase hex strings.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;

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

/// Returns `true` if `pubkey` (64-char hex) is in the relay member list.
pub async fn is_relay_member(pool: &PgPool, pubkey: &str) -> Result<bool> {
    let row = sqlx::query("SELECT 1 FROM relay_members WHERE pubkey = $1")
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// Returns the relay member record for `pubkey`, or `None` if not found.
pub async fn get_relay_member(pool: &PgPool, pubkey: &str) -> Result<Option<RelayMember>> {
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members WHERE pubkey = $1",
    )
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

/// Returns all relay members ordered by `created_at` ascending.
pub async fn list_relay_members(pool: &PgPool) -> Result<Vec<RelayMember>> {
    let rows = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at \
         FROM relay_members ORDER BY created_at ASC",
    )
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

/// Adds a new relay member.
///
/// Returns `true` if the row was actually inserted, `false` if the pubkey
/// already existed (idempotent — `ON CONFLICT DO NOTHING`).
pub async fn add_relay_member(
    pool: &PgPool,
    pubkey: &str,
    role: &str,
    added_by: Option<&str>,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO relay_members (pubkey, role, added_by) \
         VALUES ($1, $2, $3) ON CONFLICT (pubkey) DO NOTHING",
    )
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
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
pub async fn remove_relay_member(pool: &PgPool, pubkey: &str) -> Result<RemoveResult> {
    let result = sqlx::query("DELETE FROM relay_members WHERE pubkey = $1 AND role <> 'owner'")
        .bind(pubkey)
        .execute(pool)
        .await?;

    if result.rows_affected() > 0 {
        return Ok(RemoveResult::Removed);
    }

    // rows_affected == 0: either not found or is owner.  One cheap read to
    // distinguish the two cases so callers can return the right error message.
    let exists = sqlx::query("SELECT 1 FROM relay_members WHERE pubkey = $1")
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
    pubkey: &str,
    expected_role: &str,
) -> Result<RemoveResult> {
    let result = sqlx::query("DELETE FROM relay_members WHERE pubkey = $1 AND role = $2")
        .bind(pubkey)
        .bind(expected_role)
        .execute(pool)
        .await?;

    if result.rows_affected() > 0 {
        return Ok(RemoveResult::Removed);
    }

    // rows_affected == 0: either not found or role changed. One cheap read to
    // distinguish the cases so callers can return the right error message.
    let row = sqlx::query("SELECT role FROM relay_members WHERE pubkey = $1")
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

/// Updates the role of an existing relay member. Returns `true` if updated.
pub async fn update_relay_member_role(pool: &PgPool, pubkey: &str, new_role: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE relay_members SET role = $1, updated_at = now() WHERE pubkey = $2 AND role <> 'owner'",
    )
    .bind(new_role)
    .bind(pubkey)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Ensures the configured owner pubkey holds the `"owner"` role, and demotes
/// any other owners to `"admin"`. This handles owner rotation: if
/// `RELAY_OWNER_PUBKEY` changes, the old owner is automatically demoted.
///
/// Runs in a single transaction. Safe to call at every startup — idempotent.
pub async fn bootstrap_owner(pool: &PgPool, owner_pubkey: &str) -> Result<()> {
    let pubkey = owner_pubkey.to_ascii_lowercase();
    let mut tx = pool.begin().await?;

    // 1. Upsert the configured owner.
    sqlx::query(
        "INSERT INTO relay_members (pubkey, role, added_by) \
         VALUES ($1, 'owner', NULL) \
         ON CONFLICT (pubkey) DO UPDATE SET role = 'owner', updated_at = now()",
    )
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    // 2. Demote any other owners to admin.
    sqlx::query(
        "UPDATE relay_members SET role = 'admin', updated_at = now() \
         WHERE role = 'owner' AND pubkey <> $1",
    )
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Migrates existing `pubkey_allowlist` entries into `relay_members`.
///
/// Converts BYTEA pubkeys to lowercase hex text and inserts them as members.
/// Returns the number of rows inserted, or 0 if:
/// - the `pubkey_allowlist` table doesn't exist, or
/// - `relay_members` already has rows (migration ran in a prior startup).
///
/// The empty-table guard prevents re-adding members that were intentionally
/// removed by an admin after the initial backfill.
pub async fn backfill_from_allowlist(pool: &PgPool) -> Result<u64> {
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

    // Only backfill if relay_members is empty — once the table has rows
    // (from a previous backfill or manual admin commands), we must not
    // re-add members that were intentionally removed.
    let has_members: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM relay_members)")
        .fetch_one(pool)
        .await?;

    if has_members {
        return Ok(0);
    }

    let result = sqlx::query(
        "INSERT INTO relay_members (pubkey, role, added_by, created_at) \
         SELECT encode(pubkey, 'hex'), 'member', NULL, added_at \
         FROM pubkey_allowlist \
         ON CONFLICT (pubkey) DO NOTHING",
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
