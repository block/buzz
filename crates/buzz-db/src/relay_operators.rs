//! Deployment-global relay operator/moderator roster persistence.
//!
//! Backs the `relay_operators` table from `migrations/0028_relay_operators.sql`.
//!
//! Config-backed operators (`RELAY_OPERATOR_PUBKEYS`, owner-fallback) are
//! resolved at request time in the relay — this module only handles DB rows.
//! Config outranks DB: a DB moderator row for a config-backed Operator is
//! never returned as authoritative; that check happens at the relay layer.
//!
//! Lane ownership: relay admin API (Duncan).

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;

/// A row in `relay_operators`.
#[derive(Debug, Clone)]
pub struct RelayOperatorRecord {
    /// 32-byte pubkey (binary).
    pub pubkey: Vec<u8>,
    /// `"operator"` | `"moderator"`.
    pub role: String,
    /// Pubkey of the operator who added this entry (32 bytes binary).
    pub added_by: Vec<u8>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Insert or update a relay operator/moderator DB row.
///
/// If the pubkey already exists, updates the role and added_by atomically.
/// Returns the pubkey (unchanged) on success.
pub async fn upsert(pool: &PgPool, pubkey: &[u8], role: &str, added_by: &[u8]) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO relay_operators (pubkey, role, added_by)
        VALUES ($1, $2, $3)
        ON CONFLICT (pubkey) DO UPDATE SET
            role = EXCLUDED.role,
            added_by = EXCLUDED.added_by
        "#,
    )
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a relay operator/moderator DB row. Returns `true` if a row was
/// deleted, `false` if the pubkey was not found (idempotent).
pub async fn remove(pool: &PgPool, pubkey: &[u8]) -> Result<bool> {
    let result = sqlx::query("DELETE FROM relay_operators WHERE pubkey = $1")
        .bind(pubkey)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Fetch one relay operator/moderator row by pubkey.
pub async fn get(pool: &PgPool, pubkey: &[u8]) -> Result<Option<RelayOperatorRecord>> {
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at FROM relay_operators WHERE pubkey = $1",
    )
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;

    row.map(
        |r| -> std::result::Result<RelayOperatorRecord, sqlx::Error> {
            Ok(RelayOperatorRecord {
                pubkey: r.try_get("pubkey")?,
                role: r.try_get("role")?,
                added_by: r.try_get("added_by")?,
                created_at: r.try_get("created_at")?,
            })
        },
    )
    .transpose()
    .map_err(crate::error::DbError::from)
}

/// List all relay operator/moderator rows, ordered by creation time.
pub async fn list(pool: &PgPool) -> Result<Vec<RelayOperatorRecord>> {
    let rows = sqlx::query(
        "SELECT pubkey, role, added_by, created_at FROM relay_operators ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(
            |r| -> std::result::Result<RelayOperatorRecord, sqlx::Error> {
                Ok(RelayOperatorRecord {
                    pubkey: r.try_get("pubkey")?,
                    role: r.try_get("role")?,
                    added_by: r.try_get("added_by")?,
                    created_at: r.try_get("created_at")?,
                })
            },
        )
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .map_err(crate::error::DbError::from)
}
