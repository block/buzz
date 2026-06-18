//! User CRUD operations.

use crate::error::Result;
use sqlx::PgPool;
use sqlx::Row;

/// A user's profile fields.
#[derive(Debug, Clone)]
pub struct UserProfile {
    /// Raw 32-byte compressed public key.
    pub pubkey: Vec<u8>,
    /// Human-readable display name chosen by the user.
    pub display_name: Option<String>,
    /// URL of the user's avatar image.
    pub avatar_url: Option<String>,
    /// Short bio or description provided by the user.
    pub about: Option<String>,
    /// NIP-05 identifier (user@domain).
    pub nip05_handle: Option<String>,
}

/// Lightweight user record returned from search.
#[derive(Debug, Clone)]
pub struct UserSearchProfile {
    /// Raw 32-byte compressed public key.
    pub pubkey: Vec<u8>,
    /// Human-readable display name chosen by the user.
    pub display_name: Option<String>,
    /// URL of the user's avatar image.
    pub avatar_url: Option<String>,
    /// NIP-05 identifier (user@domain).
    pub nip05_handle: Option<String>,
}

/// Ensure a user record exists for the given pubkey (upsert).
/// Creates with minimal fields if not present; no-op if already exists.
pub async fn ensure_user(pool: &PgPool, pubkey: &[u8]) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO users (pubkey)
        VALUES ($1)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(pubkey)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get a single user record by pubkey.
pub async fn get_user(pool: &PgPool, pubkey: &[u8]) -> Result<Option<UserProfile>> {
    let row = sqlx::query_as::<
        _,
        (
            Vec<u8>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT pubkey, display_name, avatar_url, about, nip05_handle
        FROM users
        WHERE pubkey = $1
        "#,
    )
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(pubkey, display_name, avatar_url, about, nip05_handle)| UserProfile {
            pubkey,
            display_name,
            avatar_url,
            about,
            nip05_handle,
        },
    ))
}

/// Update a user's profile fields (display_name, avatar_url, about, nip05_handle).
/// Only updates fields that are Some -- None fields are left unchanged.
/// At least one field must be Some, otherwise returns Ok(()) without touching the DB.
///
/// Empty strings are treated as "clear to NULL" -- this is important for kind:0
/// absolute-state semantics where absent fields must be cleared, and for the
/// `nip05_handle` column which has a UNIQUE constraint (multiple NULLs are allowed,
/// but multiple empty strings would violate uniqueness).
pub async fn update_user_profile(
    pool: &PgPool,
    pubkey: &[u8],
    display_name: Option<&str>,
    avatar_url: Option<&str>,
    about: Option<&str>,
    nip05_handle: Option<&str>,
) -> Result<()> {
    let mut set_parts: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if display_name.is_some() {
        set_parts.push(format!("display_name = ${param_idx}"));
        param_idx += 1;
    }
    if avatar_url.is_some() {
        set_parts.push(format!("avatar_url = ${param_idx}"));
        param_idx += 1;
    }
    if about.is_some() {
        set_parts.push(format!("about = ${param_idx}"));
        param_idx += 1;
    }
    if nip05_handle.is_some() {
        set_parts.push(format!("nip05_handle = ${param_idx}"));
        param_idx += 1;
    }

    if set_parts.is_empty() {
        return Ok(());
    }

    // Helper: convert empty string to None (NULL in DB). This ensures UNIQUE
    // columns like nip05_handle don't collide on empty strings, and keeps
    // semantics clean: absent profile data is NULL, not "".
    fn empty_to_none(val: Option<&str>) -> Option<&str> {
        val.filter(|s| !s.is_empty())
    }

    let sql = format!(
        "UPDATE users SET {} WHERE pubkey = ${param_idx}",
        set_parts.join(", ")
    );
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    if display_name.is_some() {
        query = query.bind(empty_to_none(display_name));
    }
    if avatar_url.is_some() {
        query = query.bind(empty_to_none(avatar_url));
    }
    if about.is_some() {
        query = query.bind(empty_to_none(about));
    }
    if nip05_handle.is_some() {
        query = query.bind(empty_to_none(nip05_handle));
    }
    query = query.bind(pubkey);
    query.execute(pool).await?;
    Ok(())
}

/// Look up a user by their full NIP-05 handle (exact match, case-insensitive).
/// Both `local_part` and `domain` must already be lowercased by the caller.
pub async fn get_user_by_nip05(
    pool: &PgPool,
    local_part: &str,
    domain: &str,
) -> Result<Option<UserProfile>> {
    let handle = format!("{}@{}", local_part, domain);
    let row = sqlx::query_as::<
        _,
        (
            Vec<u8>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT pubkey, display_name, avatar_url, about, nip05_handle
        FROM users
        WHERE LOWER(nip05_handle) = LOWER($1)
        LIMIT 1
        "#,
    )
    .bind(&handle)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(pubkey, display_name, avatar_url, about, nip05_handle)| UserProfile {
            pubkey,
            display_name,
            avatar_url,
            about,
            nip05_handle,
        },
    ))
}

/// Escape SQL LIKE metacharacters (`%`, `_`, `\`) so user input is treated
/// as literal text.  Used with `ESCAPE '\'` in the query.
///
/// Without this, a search query of `"%"` would match every row (full table
/// scan) and `"_"` would act as a single-character wildcard.
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Search users by display name, NIP-05 handle, or pubkey prefix.
///
/// Empty queries return an empty vec and do not hit the database.
pub async fn search_users(
    pool: &PgPool,
    query: &str,
    limit: u32,
) -> Result<Vec<UserSearchProfile>> {
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    let escaped = escape_like(&normalized);
    let contains_pattern = format!("%{escaped}%");
    let prefix_pattern = format!("{escaped}%");
    let limit = limit.clamp(1, 50) as i64;

    let rows = sqlx::query_as::<_, (Vec<u8>, Option<String>, Option<String>, Option<String>)>(
        r#"
        SELECT pubkey, display_name, avatar_url, nip05_handle
        FROM users
        WHERE LOWER(COALESCE(display_name, '')) LIKE $1 ESCAPE '\'
           OR LOWER(COALESCE(nip05_handle, '')) LIKE $1 ESCAPE '\'
           OR LOWER(encode(pubkey, 'hex')) LIKE $1 ESCAPE '\'
        ORDER BY
            CASE
                WHEN LOWER(COALESCE(display_name, '')) = $2 THEN 0
                WHEN LOWER(COALESCE(nip05_handle, '')) = $2 THEN 1
                WHEN LOWER(encode(pubkey, 'hex')) = $2 THEN 2
                WHEN LOWER(COALESCE(display_name, '')) LIKE $3 ESCAPE '\' THEN 3
                WHEN LOWER(COALESCE(nip05_handle, '')) LIKE $3 ESCAPE '\' THEN 4
                WHEN LOWER(encode(pubkey, 'hex')) LIKE $3 ESCAPE '\' THEN 5
                ELSE 6
            END,
            COALESCE(NULLIF(display_name, ''), NULLIF(nip05_handle, ''), LOWER(encode(pubkey, 'hex')))
        LIMIT $4
        "#,
    )
    .bind(&contains_pattern)
    .bind(&normalized)
    .bind(&prefix_pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(pubkey, display_name, avatar_url, nip05_handle)| UserSearchProfile {
                pubkey,
                display_name,
                avatar_url,
                nip05_handle,
            },
        )
        .collect())
}

/// Set the owner pubkey for an agent user.
/// The owner pubkey must already exist in the users table (FK constraint).
/// Returns an error if the agent pubkey is not found (rows_affected == 0).
/// Atomically set agent owner — only if no owner is currently assigned.
///
/// Returns Ok(true) if ownership was set, Ok(false) if an owner already exists
/// (caller should check whether the existing owner matches). Returns Err if the
/// agent pubkey doesn't exist in the users table.
pub async fn set_agent_owner(
    pool: &PgPool,
    agent_pubkey: &[u8],
    owner_pubkey: &[u8],
) -> Result<bool> {
    // Conditional UPDATE: only set owner if currently NULL. This makes
    // "first mint wins" atomic — no TOCTOU race between concurrent mints.
    let result = sqlx::query(
        r#"UPDATE users SET agent_owner_pubkey = $1 WHERE pubkey = $2 AND agent_owner_pubkey IS NULL"#,
    )
    .bind(owner_pubkey)
    .bind(agent_pubkey)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        // Could be: (a) pubkey not found, or (b) owner already set.
        // Check which case by querying the row.
        let exists = sqlx::query(r#"SELECT 1 FROM users WHERE pubkey = $1"#)
            .bind(agent_pubkey)
            .fetch_optional(pool)
            .await?;
        if exists.is_none() {
            return Err(crate::error::DbError::NotFound(
                "agent pubkey not found in users table".into(),
            ));
        }
        // Row exists but owner already set — return false (not an error).
        return Ok(false);
    }
    Ok(true)
}

/// Get the channel_add_policy and agent_owner_pubkey for a user.
/// Returns None if the pubkey is not in the users table.
/// Returns Some((policy_str, owner_bytes_or_none)) if found.
pub async fn get_agent_channel_policy(
    pool: &PgPool,
    pubkey: &[u8],
) -> Result<Option<(String, Option<Vec<u8>>)>> {
    let row = sqlx::query(
        r#"SELECT channel_add_policy::text AS channel_add_policy, agent_owner_pubkey FROM users WHERE pubkey = $1"#,
    )
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;

    row.map(|r| -> Result<(String, Option<Vec<u8>>)> {
        let policy: String = r.try_get("channel_add_policy")?;
        let owner: Option<Vec<u8>> = r.try_get("agent_owner_pubkey").unwrap_or(None);
        Ok((policy, owner))
    })
    .transpose()
}

/// Check whether `actor_pubkey` is the `agent_owner_pubkey` of `target_pubkey`.
/// Queries `agent_owner_pubkey` directly rather than going through
/// `get_agent_channel_policy`, which would fetch unrelated fields.
pub async fn is_agent_owner(
    pool: &PgPool,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
) -> Result<bool> {
    let row = sqlx::query_scalar::<_, bool>(
        "SELECT agent_owner_pubkey = $2 FROM users WHERE pubkey = $1 AND agent_owner_pubkey IS NOT NULL",
    )
    .bind(target_pubkey)
    .bind(actor_pubkey)
    .fetch_optional(pool)
    .await?;
    Ok(row.unwrap_or(false))
}

/// Set the channel_add_policy for a user.
/// Returns an error if the pubkey is not found (rows_affected == 0).
/// Returns an error if `policy` is not one of the valid ENUM values.
pub async fn set_channel_add_policy(pool: &PgPool, pubkey: &[u8], policy: &str) -> Result<()> {
    if !matches!(policy, "anyone" | "owner_only" | "nobody") {
        return Err(crate::error::DbError::InvalidData(format!(
            "invalid channel_add_policy: {policy}"
        )));
    }
    let result = sqlx::query(
        r#"UPDATE users SET channel_add_policy = $1::channel_add_policy WHERE pubkey = $2"#,
    )
    .bind(policy)
    .bind(pubkey)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(
            "pubkey not found in users table".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use nostr::Keys;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_db() -> Db {
        let pool = PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    /// Setting an agent owner then reading back the policy should return
    /// the default "anyone" policy and the owner pubkey.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_agent_owner_and_get_policy() {
        let db = setup_db().await;
        let agent_pk = random_pubkey();
        let owner_pk = random_pubkey();

        ensure_user(&db.pool, &agent_pk)
            .await
            .expect("ensure agent");
        ensure_user(&db.pool, &owner_pk)
            .await
            .expect("ensure owner");

        let was_set = set_agent_owner(&db.pool, &agent_pk, &owner_pk)
            .await
            .expect("set_agent_owner");
        assert!(was_set, "first set_agent_owner should return true");

        let result = get_agent_channel_policy(&db.pool, &agent_pk)
            .await
            .expect("get_agent_channel_policy");

        let (policy, owner) = result.expect("should return Some for known pubkey");
        assert_eq!(policy, "anyone", "default policy should be 'anyone'");
        assert_eq!(
            owner,
            Some(owner_pk),
            "owner pubkey should match what was set"
        );
    }

    /// set_channel_add_policy should persist each of the three valid policies.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_channel_add_policy() {
        let db = setup_db().await;
        let pk = random_pubkey();
        ensure_user(&db.pool, &pk).await.expect("ensure user");

        // owner_only
        set_channel_add_policy(&db.pool, &pk, "owner_only")
            .await
            .expect("set owner_only");
        let (policy, owner) = get_agent_channel_policy(&db.pool, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(policy, "owner_only");
        assert!(owner.is_none(), "no owner was set");

        // nobody
        set_channel_add_policy(&db.pool, &pk, "nobody")
            .await
            .expect("set nobody");
        let (policy, owner) = get_agent_channel_policy(&db.pool, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(policy, "nobody");
        assert!(owner.is_none());

        // anyone (reset to default)
        set_channel_add_policy(&db.pool, &pk, "anyone")
            .await
            .expect("set anyone");
        let (policy, owner) = get_agent_channel_policy(&db.pool, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(policy, "anyone");
        assert!(owner.is_none());
    }

    /// get_agent_channel_policy should return None for a pubkey that has
    /// never been inserted into the users table.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_get_policy_unknown_pubkey() {
        let db = setup_db().await;
        let pk = random_pubkey();

        let result = get_agent_channel_policy(&db.pool, &pk)
            .await
            .expect("query should not error");

        assert!(result.is_none(), "unknown pubkey should return None");
    }

    /// set_agent_owner should return Err when the agent pubkey does not exist
    /// in the users table.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_agent_owner_nonexistent_agent() {
        let db = setup_db().await;
        let agent_pk = random_pubkey();
        let owner_pk = random_pubkey();

        // Only ensure the owner exists -- agent is intentionally absent.
        ensure_user(&db.pool, &owner_pk)
            .await
            .expect("ensure owner");

        let result = set_agent_owner(&db.pool, &agent_pk, &owner_pk).await;
        assert!(
            result.is_err(),
            "should error when agent pubkey is not in users table"
        );
    }

    /// set_agent_owner should return Ok(false) when the agent already has an owner.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_agent_owner_already_owned() {
        let db = setup_db().await;
        let agent_pk = random_pubkey();
        let owner1 = random_pubkey();
        let owner2 = random_pubkey();

        ensure_user(&db.pool, &agent_pk)
            .await
            .expect("ensure agent");
        ensure_user(&db.pool, &owner1).await.expect("ensure owner1");
        ensure_user(&db.pool, &owner2).await.expect("ensure owner2");

        let first = set_agent_owner(&db.pool, &agent_pk, &owner1)
            .await
            .expect("first set");
        assert!(first, "first set should succeed");

        let second = set_agent_owner(&db.pool, &agent_pk, &owner2)
            .await
            .expect("second set should not error");
        assert!(!second, "second set should return false (already owned)");

        // Verify original owner is preserved.
        let (_, owner) = get_agent_channel_policy(&db.pool, &agent_pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(owner, Some(owner1), "original owner should be preserved");
    }

    /// set_channel_add_policy should return Err when the pubkey does not exist
    /// in the users table (0 rows affected -> NotFound).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_channel_add_policy_nonexistent_user() {
        let db = setup_db().await;
        let pk = random_pubkey();

        let result = set_channel_add_policy(&db.pool, &pk, "nobody").await;
        assert!(
            result.is_err(),
            "should error when pubkey is not in users table"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_channel_add_policy_rejects_invalid() {
        let db = setup_db().await;
        let pubkey = nostr::Keys::generate().public_key().to_bytes().to_vec();
        ensure_user(&db.pool, &pubkey).await.unwrap();
        let result = set_channel_add_policy(&db.pool, &pubkey, "invalid_policy").await;
        assert!(result.is_err(), "should reject invalid policy value");
    }

    // ── LIKE escaping unit tests (no DB required) ──────────────────────

    // Use the production `escape_like` function directly — no local mirror.
    use super::escape_like;

    #[test]
    fn like_escape_percent() {
        assert_eq!(escape_like("%"), "\\%");
        assert_eq!(escape_like("100%match"), "100\\%match");
    }

    #[test]
    fn like_escape_underscore() {
        assert_eq!(escape_like("_"), "\\_");
        assert_eq!(escape_like("a_b"), "a\\_b");
    }

    #[test]
    fn like_escape_backslash() {
        assert_eq!(escape_like("\\"), "\\\\");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
    }

    #[test]
    fn like_escape_combined() {
        // All three metacharacters in one string
        assert_eq!(escape_like("%_\\"), "\\%\\_\\\\");
    }

    #[test]
    fn like_escape_normal_input_unchanged() {
        assert_eq!(escape_like("alice"), "alice");
        assert_eq!(escape_like("bob@example.com"), "bob@example.com");
        assert_eq!(escape_like(""), "");
    }

    /// A user with "owner_only" policy but no agent_owner_pubkey set should
    /// return Some(("owner_only", None)).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_owner_only_with_no_owner() {
        let db = setup_db().await;
        let pk = random_pubkey();
        ensure_user(&db.pool, &pk).await.expect("ensure user");

        set_channel_add_policy(&db.pool, &pk, "owner_only")
            .await
            .expect("set owner_only");

        let result = get_agent_channel_policy(&db.pool, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");

        assert_eq!(result.0, "owner_only");
        assert!(result.1.is_none(), "owner should be None when never set");
    }
}
