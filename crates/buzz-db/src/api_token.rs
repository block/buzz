//! API token CRUD operations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Create a new API token record. The caller is responsible for generating
/// the raw token and computing its SHA-256 hash.
pub async fn create_api_token(
    pool: &PgPool,
    token_hash: &[u8],
    owner_pubkey: &[u8],
    name: &str,
    scopes: &[String],
    channel_ids: Option<&[Uuid]>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();

    let scopes_json =
        serde_json::to_value(scopes).map_err(|e| DbError::InvalidData(e.to_string()))?;

    // Serialize channel_ids; propagate errors rather than silently dropping to NULL.
    let channel_ids_json: Option<serde_json::Value> = channel_ids
        .map(|ids| {
            serde_json::to_value(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                .map_err(|e| DbError::InvalidData(format!("channel_ids serialization: {e}")))
        })
        .transpose()?;

    sqlx::query(
        r#"
        INSERT INTO api_tokens (id, token_hash, owner_pubkey, name, scopes, channel_ids, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(id)
    .bind(token_hash)
    .bind(owner_pubkey)
    .bind(name)
    .bind(&scopes_json)
    .bind(&channel_ids_json)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(id)
}

/// Atomic conditional INSERT: create a token only if the owner has fewer than 10 active tokens.
///
/// Uses a subquery so the check and insert are atomic --
/// no TOCTOU race between a separate count query and the insert.
///
/// Returns `Ok(Some(uuid))` on success, `Ok(None)` if the 10-token limit is exceeded.
pub async fn create_api_token_if_under_limit(
    pool: &PgPool,
    token_hash: &[u8],
    owner_pubkey: &[u8],
    name: &str,
    scopes: &[String],
    channel_ids: Option<&[Uuid]>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<Option<Uuid>> {
    let id = Uuid::new_v4();

    let scopes_json =
        serde_json::to_value(scopes).map_err(|e| DbError::InvalidData(e.to_string()))?;

    let channel_ids_json: Option<serde_json::Value> = channel_ids
        .map(|ids| {
            serde_json::to_value(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                .map_err(|e| DbError::InvalidData(format!("channel_ids serialization: {e}")))
        })
        .transpose()?;

    // Conditional INSERT: only inserts if active (non-revoked, non-expired) token count < 10.
    // The subquery and insert execute atomically -- no separate count + insert race.
    let result = sqlx::query(
        r#"
        INSERT INTO api_tokens
            (id, token_hash, owner_pubkey, name, scopes, channel_ids, expires_at, created_by_self_mint)
        SELECT $1, $2, $3, $4, $5, $6, $7, TRUE
        WHERE (
            SELECT COUNT(*)
            FROM api_tokens
            WHERE owner_pubkey = $8
              AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
        ) < 10
        "#,
    )
    .bind(id)
    .bind(token_hash)
    .bind(owner_pubkey)
    .bind(name)
    .bind(&scopes_json)
    .bind(&channel_ids_json)
    .bind(expires_at)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Limit exceeded -- the WHERE clause prevented the INSERT.
        return Ok(None);
    }

    Ok(Some(id))
}

/// Look up an API token by its SHA-256 hash, **including revoked tokens**.
///
/// Unlike [`crate::Db::get_api_token_by_hash`] (which filters `revoked_at IS NULL`),
/// this function returns the full record regardless of revocation status.
/// The relay layer uses this to return distinct `token_revoked` vs `invalid_token`
/// error responses rather than treating both as "not found".
pub async fn get_api_token_by_hash_including_revoked(
    pool: &PgPool,
    hash: &[u8],
) -> Result<Option<crate::ApiTokenRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
               created_at, expires_at, last_used_at, revoked_at
        FROM api_tokens
        WHERE token_hash = $1
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;

    let row = match row {
        None => return Ok(None),
        Some(r) => r,
    };

    let id: Uuid = row.try_get("id")?;

    let scopes_json: serde_json::Value = row.try_get("scopes")?;
    let scopes: Vec<String> = serde_json::from_value(scopes_json)
        .map_err(|e| DbError::InvalidData(format!("scopes JSON: {e}")))?;

    let channel_ids: Option<Vec<Uuid>> = {
        let raw: Option<serde_json::Value> = row.try_get("channel_ids")?;
        match raw {
            None => None,
            Some(v) => {
                let strings: Vec<String> = serde_json::from_value(v)
                    .map_err(|e| DbError::InvalidData(format!("channel_ids JSON: {e}")))?;
                let uuids: std::result::Result<Vec<Uuid>, _> =
                    strings.iter().map(|s| s.parse::<Uuid>()).collect();
                Some(uuids.map_err(|e| DbError::InvalidData(format!("channel_ids UUID: {e}")))?)
            }
        }
    };

    Ok(Some(crate::ApiTokenRecord {
        id,
        token_hash: row.try_get("token_hash")?,
        owner_pubkey: row.try_get("owner_pubkey")?,
        name: row.try_get("name")?,
        scopes,
        channel_ids,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
    }))
}

/// List all tokens (including revoked) for a pubkey, ordered by creation time descending.
///
/// Returns the full [`crate::ApiTokenRecord`] including `token_hash`. Callers are
/// responsible for stripping `token_hash` before returning data to clients -- the
/// raw token value is never exposed after the initial mint response.
/// Used by `GET /api/tokens` to show a user their full token history.
pub async fn list_tokens_by_owner(
    pool: &PgPool,
    pubkey: &[u8],
) -> Result<Vec<crate::ApiTokenRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
               created_at, expires_at, last_used_at, revoked_at
        FROM api_tokens
        WHERE owner_pubkey = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(pubkey)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: Uuid = row.try_get("id")?;

        let scopes_json: serde_json::Value = row.try_get("scopes")?;
        let scopes: Vec<String> = serde_json::from_value(scopes_json)
            .map_err(|e| DbError::InvalidData(format!("scopes JSON: {e}")))?;

        let channel_ids: Option<Vec<Uuid>> = {
            let raw: Option<serde_json::Value> = row.try_get("channel_ids")?;
            match raw {
                None => None,
                Some(v) => {
                    let strings: Vec<String> = serde_json::from_value(v)
                        .map_err(|e| DbError::InvalidData(format!("channel_ids JSON: {e}")))?;
                    let uuids: std::result::Result<Vec<Uuid>, _> =
                        strings.iter().map(|s| s.parse::<Uuid>()).collect();
                    Some(
                        uuids
                            .map_err(|e| DbError::InvalidData(format!("channel_ids UUID: {e}")))?,
                    )
                }
            }
        };

        out.push(crate::ApiTokenRecord {
            id,
            token_hash: row.try_get("token_hash")?,
            owner_pubkey: row.try_get("owner_pubkey")?,
            name: row.try_get("name")?,
            scopes,
            channel_ids,
            created_at: row.try_get("created_at")?,
            expires_at: row.try_get("expires_at")?,
            last_used_at: row.try_get("last_used_at")?,
            revoked_at: row.try_get("revoked_at")?,
        });
    }
    Ok(out)
}

/// Revoke a single token by ID, scoped to the owner.
///
/// Only revokes if the token is owned by `owner_pubkey` and not already revoked.
/// Returns `true` if the token was revoked, `false` if not found, not owned, or already revoked.
pub async fn revoke_token(
    pool: &PgPool,
    id: Uuid,
    owner_pubkey: &[u8],
    revoked_by: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE api_tokens
        SET revoked_at = NOW(), revoked_by = $1
        WHERE id = $2
          AND owner_pubkey = $3
          AND revoked_at IS NULL
        "#,
    )
    .bind(revoked_by)
    .bind(id)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Revoke all active tokens for a pubkey.
///
/// Skips already-revoked tokens (idempotent). Returns the count of newly revoked tokens.
/// If all tokens are already revoked, returns 0 with no error.
pub async fn revoke_all_tokens(
    pool: &PgPool,
    owner_pubkey: &[u8],
    revoked_by: &[u8],
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE api_tokens
        SET revoked_at = NOW(), revoked_by = $1
        WHERE owner_pubkey = $2
          AND revoked_at IS NULL
        "#,
    )
    .bind(revoked_by)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
