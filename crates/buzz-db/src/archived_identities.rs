//! Relay-scoped archived identity persistence (NIP-IA).
//!
//! The `archived_identities` table stores a relay-local UI visibility hint for
//! identity pubkeys. Archiving is not a ban: it does not affect membership,
//! relay access, or repository permissions.
//! All pubkey and event ID values are lowercase hex strings.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;

/// A single archived identity record.
#[derive(Debug, Clone)]
pub struct ArchivedIdentity {
    /// 64-char lowercase hex pubkey of the archived identity.
    pub pubkey: String,
    /// Consent path that authorized the archive: `"self"`, `"owner"`, or `"admin"`.
    pub consent_path: String,
    /// 64-char lowercase hex pubkey of the actor that requested the archive.
    pub actor: String,
    /// Optional human-readable archive reason.
    pub reason: Option<String>,
    /// Optional 64-char lowercase hex pubkey replacing this identity.
    pub replaced_by: Option<String>,
    /// Hex event ID of the archive request that created this row.
    pub request_event_id: String,
    /// When the identity was archived.
    pub archived_at: DateTime<Utc>,
}

/// Returns `true` if `pubkey` (64-char hex) is currently archived.
pub async fn is_archived(pool: &PgPool, pubkey: &str) -> Result<bool> {
    let row = sqlx::query("SELECT 1 FROM archived_identities WHERE pubkey = $1")
        .bind(pubkey)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// Archives an identity.
///
/// Returns `true` if the row was inserted, `false` if the identity was already
/// archived. Re-archiving is idempotent and does not mutate the existing row.
pub async fn archive(
    pool: &PgPool,
    pubkey: &str,
    consent_path: &str,
    actor: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    request_event_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO archived_identities \
         (pubkey, consent_path, actor, reason, replaced_by, request_event_id) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (pubkey) DO NOTHING",
    )
    .bind(pubkey)
    .bind(consent_path)
    .bind(actor)
    .bind(reason)
    .bind(replaced_by)
    .bind(request_event_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Unarchives an identity.
///
/// Returns `true` if a row was deleted, `false` if the identity was not archived.
pub async fn unarchive(pool: &PgPool, pubkey: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM archived_identities WHERE pubkey = $1")
        .bind(pubkey)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

/// Returns all archived identities ordered by archive time ascending.
pub async fn list_archived(pool: &PgPool) -> Result<Vec<ArchivedIdentity>> {
    let rows = sqlx::query(
        "SELECT pubkey, consent_path, actor, reason, replaced_by, request_event_id, archived_at \
         FROM archived_identities ORDER BY archived_at ASC",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(row_to_archived_identity)
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .map_err(crate::error::DbError::from)
}

fn row_to_archived_identity(
    row: sqlx::postgres::PgRow,
) -> std::result::Result<ArchivedIdentity, sqlx::Error> {
    Ok(ArchivedIdentity {
        pubkey: row.try_get("pubkey")?,
        consent_path: row.try_get("consent_path")?,
        actor: row.try_get("actor")?,
        reason: row.try_get("reason")?,
        replaced_by: row.try_get("replaced_by")?,
        request_event_id: row.try_get("request_event_id")?,
        archived_at: row.try_get("archived_at")?,
    })
}
