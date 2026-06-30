//! API token CRUD operations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{DbError, Result};

/// Create a new API token record. The caller is responsible for generating
/// the raw token and computing its SHA-256 hash.
///
/// `community_id` is row zero: every token is scoped to a community, derived
/// from the request's resolved tenant — never client-supplied here.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_token(
    pool: &PgPool,
    community_id: Uuid,
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
        INSERT INTO api_tokens
            (community_id, id, token_hash, owner_pubkey, name, scopes, channel_ids, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(community_id)
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
/// The 10-token limit is per (community, owner) — a user's quota is scoped to
/// their community, never global.
///
/// Returns `Ok(Some(uuid))` on success, `Ok(None)` if the 10-token limit is exceeded.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_token_if_under_limit(
    pool: &PgPool,
    community_id: Uuid,
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

    // Conditional INSERT: only inserts if active (non-revoked, non-expired) token count < 10
    // **for this (community, owner) pair**. The subquery and insert execute atomically --
    // no separate count + insert race.
    let result = sqlx::query(
        r#"
        INSERT INTO api_tokens
            (community_id, id, token_hash, owner_pubkey, name, scopes, channel_ids, expires_at, created_by_self_mint)
        SELECT $1, $2, $3, $4, $5, $6, $7, $8, TRUE
        WHERE (
            SELECT COUNT(*)
            FROM api_tokens
            WHERE community_id = $1
              AND owner_pubkey = $9
              AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
        ) < 10
        "#,
    )
    .bind(community_id)
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

/// Look up an API token by its SHA-256 hash, **including revoked tokens**,
/// scoped to the request's community.
///
/// The lookup is keyed on `(community_id, token_hash)` — the same key the
/// storage UNIQUE index uses. This closes the row-44 conformance obligation:
/// a token minted in community A must never authorize in community B, even
/// if (by birthday-style collision or adversarial mint) the same hash exists
/// in both. The UNIQUE index is a *storage* guarantee; this `AND community_id`
/// clause is the *query* guarantee — both must hold for the property to be
/// load-bearing under all schemas.
///
/// Unlike [`crate::Db::get_api_token_by_hash`] (which filters `revoked_at IS NULL`),
/// this function returns the full record regardless of revocation status.
/// The relay layer uses this to return distinct `token_revoked` vs `invalid_token`
/// error responses rather than treating both as "not found".
pub async fn get_api_token_by_hash_including_revoked(
    pool: &PgPool,
    community_id: Uuid,
    hash: &[u8],
) -> Result<Option<crate::ApiTokenRecord>> {
    let row = sqlx::query(
        r#"
        SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
               created_at, expires_at, last_used_at, revoked_at
        FROM api_tokens
        WHERE community_id = $1 AND token_hash = $2
        "#,
    )
    .bind(community_id)
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

/// List all tokens (including revoked) for a (community, owner) pair,
/// ordered by creation time descending.
///
/// Returns the full [`crate::ApiTokenRecord`] including `token_hash`. Callers are
/// responsible for stripping `token_hash` before returning data to clients -- the
/// raw token value is never exposed after the initial mint response.
/// Used by `GET /api/tokens` to show a user their full token history.
pub async fn list_tokens_by_owner(
    pool: &PgPool,
    community_id: Uuid,
    pubkey: &[u8],
) -> Result<Vec<crate::ApiTokenRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT id, token_hash, owner_pubkey, name, scopes, channel_ids,
               created_at, expires_at, last_used_at, revoked_at
        FROM api_tokens
        WHERE community_id = $1 AND owner_pubkey = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(community_id)
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

/// Revoke a single token by ID, scoped to (community, owner).
///
/// Only revokes if the token is in `community_id`, owned by `owner_pubkey`, and not already revoked.
/// Returns `true` if the token was revoked, `false` if not found, not owned, or already revoked.
pub async fn revoke_token(
    pool: &PgPool,
    community_id: Uuid,
    id: Uuid,
    owner_pubkey: &[u8],
    revoked_by: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE api_tokens
        SET revoked_at = NOW(), revoked_by = $1
        WHERE community_id = $2
          AND id = $3
          AND owner_pubkey = $4
          AND revoked_at IS NULL
        "#,
    )
    .bind(revoked_by)
    .bind(community_id)
    .bind(id)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Revoke all active tokens for a (community, owner) pair.
///
/// Skips already-revoked tokens (idempotent). Returns the count of newly revoked tokens.
/// If all tokens are already revoked, returns 0 with no error.
pub async fn revoke_all_tokens(
    pool: &PgPool,
    community_id: Uuid,
    owner_pubkey: &[u8],
    revoked_by: &[u8],
) -> Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE api_tokens
        SET revoked_at = NOW(), revoked_by = $1
        WHERE community_id = $2
          AND owner_pubkey = $3
          AND revoked_at IS NULL
        "#,
    )
    .bind(revoked_by)
    .bind(community_id)
    .bind(owner_pubkey)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    //! Row-44 conformance: API token lookups MUST be keyed on
    //! `(community_id, token_hash)`, not on `token_hash` alone. The storage
    //! UNIQUE index is a *storage* guarantee; the WHERE clause here is the
    //! *query* guarantee. Both must hold — a query that filters on hash
    //! alone could return a foreign-community row, defeating the row-zero
    //! tenancy fence. This test directly inserts two same-hash rows in two
    //! communities (only possible by bypassing the unique index, which we
    //! achieve via distinct hashes that the test then queries-by-hash for
    //! both — see below for the actual property under test).
    //!
    //! The load-bearing property: even if storage uniqueness is ever relaxed
    //! or a hash collision occurs, the query-side `AND community_id = $N`
    //! clause guarantees the lookup returns the row for the *requested*
    //! tenant. Mutate-bite proof: drop the clause, the test fails.
    use super::*;
    use crate::{ApiTokenRecord, Db};
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_db() -> Db {
        let pool = PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB");
        Db { pool }
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("api-token-tenancy-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    async fn insert_user(pool: &PgPool, community_id: Uuid, pubkey: &[u8]) {
        sqlx::query(
            r#"
            INSERT INTO users (community_id, pubkey)
            VALUES ($1, $2)
            "#,
        )
        .bind(community_id)
        .bind(pubkey)
        .execute(pool)
        .await
        .expect("insert user");
    }

    /// Direct INSERT bypassing `create_api_token` so the test pins the
    /// **lookup**'s scoping, not the insert path's.
    async fn raw_insert_token(
        pool: &PgPool,
        community_id: Uuid,
        token_hash: &[u8],
        owner_pubkey: &[u8],
        name: &str,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let scopes = serde_json::json!(["files:read", "files:write"]);
        sqlx::query(
            r#"
            INSERT INTO api_tokens
                (community_id, id, token_hash, owner_pubkey, name, scopes)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(token_hash)
        .bind(owner_pubkey)
        .bind(name)
        .bind(&scopes)
        .execute(pool)
        .await
        .expect("insert api_token");
        id
    }

    /// Row-44 sharp test: two communities, **same** 32-byte token hash in each,
    /// lookup scoped to community A returns A's row only (and B-scoped lookup
    /// returns B's row only). The storage UNIQUE index is `(community_id,
    /// token_hash)` so this is a legal state. The lookup must not return the
    /// foreign row.
    ///
    /// Mutate-bite handle: the WHERE clause in
    /// `get_api_token_by_hash_including_revoked` is the only thing keeping
    /// this test green. Strip `AND community_id = $1` and the lookup becomes
    /// hash-only — Postgres returns whichever row it picks (insert-order
    /// dependent), and the cross-tenancy assertion fails.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lookup_by_hash_is_scoped_to_community() {
        let db = setup_db().await;

        let community_a = make_community(&db.pool).await;
        let community_b = make_community(&db.pool).await;

        // Distinct pubkeys per community — FK is (community_id, owner_pubkey).
        let owner_a = vec![0xAAu8; 32];
        let owner_b = vec![0xBBu8; 32];
        insert_user(&db.pool, community_a, &owner_a).await;
        insert_user(&db.pool, community_b, &owner_b).await;

        // SAME hash in both communities — legal under UNIQUE(community_id, token_hash).
        let shared_hash = vec![0xCCu8; 32];
        let id_a = raw_insert_token(&db.pool, community_a, &shared_hash, &owner_a, "token-A").await;
        let id_b = raw_insert_token(&db.pool, community_b, &shared_hash, &owner_b, "token-B").await;
        assert_ne!(id_a, id_b, "ids must differ");

        let cid_a = buzz_core::CommunityId::from_uuid(community_a);
        let cid_b = buzz_core::CommunityId::from_uuid(community_b);

        // Lookup scoped to A returns A's row, never B's.
        let from_a: ApiTokenRecord = db
            .get_api_token_by_hash_including_revoked(cid_a, &shared_hash)
            .await
            .expect("lookup A")
            .expect("row in A");
        assert_eq!(from_a.id, id_a, "community-A lookup must return A's row");
        assert_eq!(
            from_a.owner_pubkey, owner_a,
            "community-A lookup must return A's owner",
        );

        // Lookup scoped to B returns B's row, never A's.
        let from_b: ApiTokenRecord = db
            .get_api_token_by_hash_including_revoked(cid_b, &shared_hash)
            .await
            .expect("lookup B")
            .expect("row in B");
        assert_eq!(from_b.id, id_b, "community-B lookup must return B's row");
        assert_eq!(
            from_b.owner_pubkey, owner_b,
            "community-B lookup must return B's owner",
        );

        // Lookup with the hash but a third (unrelated) community returns None.
        let community_c = make_community(&db.pool).await;
        let cid_c = buzz_core::CommunityId::from_uuid(community_c);
        let from_c = db
            .get_api_token_by_hash_including_revoked(cid_c, &shared_hash)
            .await
            .expect("lookup C");
        assert!(
            from_c.is_none(),
            "community-C has no token with this hash; lookup must return None, got {from_c:?}",
        );
    }

    /// Active (non-revoked) lookup also enforces community scope.
    /// Mirrors the obligation for the `revoked_at IS NULL` variant at
    /// `Db::get_api_token_by_hash`.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn active_lookup_by_hash_is_scoped_to_community() {
        let db = setup_db().await;

        let community_a = make_community(&db.pool).await;
        let community_b = make_community(&db.pool).await;

        let owner_a = vec![0x11u8; 32];
        let owner_b = vec![0x22u8; 32];
        insert_user(&db.pool, community_a, &owner_a).await;
        insert_user(&db.pool, community_b, &owner_b).await;

        let shared_hash = vec![0x33u8; 32];
        let id_a =
            raw_insert_token(&db.pool, community_a, &shared_hash, &owner_a, "active-A").await;
        let id_b =
            raw_insert_token(&db.pool, community_b, &shared_hash, &owner_b, "active-B").await;

        let cid_a = buzz_core::CommunityId::from_uuid(community_a);
        let cid_b = buzz_core::CommunityId::from_uuid(community_b);

        let from_a = db
            .get_api_token_by_hash(cid_a, &shared_hash)
            .await
            .expect("active lookup A")
            .expect("row in A");
        assert_eq!(from_a.id, id_a);

        let from_b = db
            .get_api_token_by_hash(cid_b, &shared_hash)
            .await
            .expect("active lookup B")
            .expect("row in B");
        assert_eq!(from_b.id, id_b);
    }
}
