//! User CRUD operations.

use crate::error::Result;
use crate::Db;
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use sqlx::PgPool;
use sqlx::Row;

/// A user's profile fields.
#[derive(Debug, Clone)]
pub struct UserProfile {
    /// Raw 32-byte compressed public key.
    pub pubkey: Vec<u8>,
    /// Raw kind-0 `name` (a handle-like short name).
    pub name: Option<String>,
    /// Kind-0 `display_name`/`displayName`. Readers wanting a label should
    /// fall back to `name` when this is absent.
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
    /// Raw kind-0 `name`.
    pub name: Option<String>,
    /// Kind-0 `display_name`/`displayName`.
    pub display_name: Option<String>,
    /// URL of the user's avatar image.
    pub avatar_url: Option<String>,
    /// NIP-05 identifier (user@domain).
    pub nip05_handle: Option<String>,
}

/// Parameters for [`search_users`].
#[derive(Debug, Clone, Default)]
pub struct SearchUsersParams<'a> {
    /// Free-text query; matched case-insensitively against `name`,
    /// `display_name`, `nip05_handle`, and the hex pubkey.
    pub query: &'a str,
    /// Restrict results to these pubkeys (NIP-01 `authors`); `None` means all.
    ///
    /// `Some(&[])` returns no rows at this seam. The relay maps an empty
    /// `authors` list to `None` before calling, so "empty means any" holds at
    /// the REQ boundary (matching the FTS path) but not here.
    pub authors: Option<&'a [Vec<u8>]>,
    /// Maximum rows to return, clamped to `1..=500`.
    pub limit: u32,
    /// Rows to skip in rank order (the bridge's `page` extension).
    pub offset: u32,
}

/// Ensure a user record exists for the given pubkey (upsert).
/// Creates with minimal fields if not present; no-op if already exists.
///
/// Returns `true` if a new row was inserted, `false` if the user already existed.
/// The `true` case is the reliable signal for "user was just registered" — used
/// by callers to increment `buzz_users_created_total`.
pub async fn ensure_user(pool: &PgPool, community_id: CommunityId, pubkey: &[u8]) -> Result<bool> {
    ensure_user_with_operation(
        pool,
        community_id,
        pubkey,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
}

async fn ensure_user_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    operation: crate::observability::WriterOperation,
) -> Result<bool> {
    // Almost every call finds the user already registered. Answer that with
    // one read, so the hot path skips the admission round trips; no write
    // happens, so the fence contract is unaffected.
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE community_id = $1 AND pubkey = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_one(&mut *connection)
    .await?;
    if exists {
        return Ok(false);
    }
    drop(connection);

    let mut tx = crate::begin_community_write_transaction(pool, community_id, operation).await?;
    let result = sqlx::query(
        r#"
        INSERT INTO users (community_id, pubkey)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .execute(tx.conn())
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

/// Get a single user record by pubkey.
pub async fn get_user(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<UserProfile>> {
    let row = sqlx::query_as::<_, UserProfileRow>(
        r#"
        SELECT pubkey, name, display_name, avatar_url, about, nip05_handle
        FROM users
        WHERE community_id = $1 AND pubkey = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(UserProfile::from_row))
}

type UserProfileRow = (
    Vec<u8>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl UserProfile {
    fn from_row(
        (pubkey, name, display_name, avatar_url, about, nip05_handle): UserProfileRow,
    ) -> Self {
        Self {
            pubkey,
            name,
            display_name,
            avatar_url,
            about,
            nip05_handle,
        }
    }
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
    community_id: CommunityId,
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
        "UPDATE users SET {} WHERE community_id = ${param_idx} AND pubkey = ${}",
        set_parts.join(", "),
        param_idx + 1
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
    query = query.bind(community_id.as_uuid());
    query = query.bind(pubkey);
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    query.execute(tx.conn()).await?;
    tx.commit().await?;
    Ok(())
}

/// Look up a user by their full NIP-05 handle (exact match, case-insensitive).
/// Both `local_part` and `domain` must already be lowercased by the caller.
pub async fn get_user_by_nip05(
    pool: &PgPool,
    community_id: CommunityId,
    local_part: &str,
    domain: &str,
) -> Result<Option<UserProfile>> {
    let handle = format!("{}@{}", local_part, domain);
    let row = sqlx::query_as::<_, UserProfileRow>(
        r#"
        SELECT pubkey, name, display_name, avatar_url, about, nip05_handle
        FROM users
        WHERE community_id = $1 AND LOWER(nip05_handle) = LOWER($2)
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(&handle)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(UserProfile::from_row))
}

/// Escape SQL LIKE metacharacters (`%`, `_`, `\`) so user input is treated
/// as literal text.  Used with `ESCAPE '\'` in the query.
///
/// Without this, a search query of `"%"` would match every row (full table
/// scan) and `"_"` would act as a single-character wildcard.
pub(crate) fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Search users by name, display name, NIP-05 handle, or pubkey prefix.
///
/// Only rows projected from a live kind-0 head (`metadata_event_id IS NOT
/// NULL`) are candidates: a row `ensure_user` created for a pubkey that only
/// authenticated can never hydrate to a profile event, so it must not take a
/// `limit`/`offset` slot. Names and handles match as substrings; the hex
/// pubkey matches as a prefix only, because a one- or two-character typeahead
/// query drawn from `[0-9a-f]` would otherwise substring-match a large share
/// of every pubkey in the community.
///
/// Results are ranked exact > prefix > contains (name/display_name before
/// nip05 before pubkey within each tier), then by label, then by pubkey so
/// the order is total and stable across `offset` pages. Deactivated users
/// are excluded. Empty queries return an empty vec and do not hit the
/// database.
pub async fn search_users(
    pool: &PgPool,
    community_id: CommunityId,
    params: &SearchUsersParams<'_>,
) -> Result<Vec<UserSearchProfile>> {
    let normalized = params.query.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if params.authors.is_some_and(|authors| authors.is_empty()) {
        return Ok(Vec::new());
    }

    let escaped = escape_like(&normalized);
    let contains_pattern = format!("%{escaped}%");
    let prefix_pattern = format!("{escaped}%");
    let limit = params.limit.clamp(1, 500) as i64;
    let offset = i64::from(params.offset);

    let rows = sqlx::query_as::<
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
        SELECT pubkey, name, display_name, avatar_url, nip05_handle
        FROM users
        WHERE community_id = $1
          AND deactivated_at IS NULL
          AND metadata_event_id IS NOT NULL
          AND ($6::bytea[] IS NULL OR pubkey = ANY($6))
          AND (LOWER(COALESCE(name, '')) LIKE $2 ESCAPE '\'
           OR LOWER(COALESCE(display_name, '')) LIKE $2 ESCAPE '\'
           OR LOWER(COALESCE(nip05_handle, '')) LIKE $2 ESCAPE '\'
           OR LOWER(encode(pubkey, 'hex')) LIKE $4 ESCAPE '\')
        ORDER BY
            CASE
                WHEN LOWER(COALESCE(name, '')) = $3
                  OR LOWER(COALESCE(display_name, '')) = $3 THEN 0
                WHEN LOWER(COALESCE(nip05_handle, '')) = $3 THEN 1
                WHEN LOWER(encode(pubkey, 'hex')) = $3 THEN 2
                WHEN LOWER(COALESCE(name, '')) LIKE $4 ESCAPE '\'
                  OR LOWER(COALESCE(display_name, '')) LIKE $4 ESCAPE '\' THEN 3
                WHEN LOWER(COALESCE(nip05_handle, '')) LIKE $4 ESCAPE '\' THEN 4
                WHEN LOWER(encode(pubkey, 'hex')) LIKE $4 ESCAPE '\' THEN 5
                ELSE 6
            END,
            COALESCE(NULLIF(display_name, ''), NULLIF(name, ''), NULLIF(nip05_handle, ''), LOWER(encode(pubkey, 'hex'))),
            pubkey
        LIMIT $5 OFFSET $7
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(&contains_pattern)
    .bind(&normalized)
    .bind(&prefix_pattern)
    .bind(limit)
    .bind(params.authors)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(pubkey, name, display_name, avatar_url, nip05_handle)| UserSearchProfile {
                pubkey,
                name,
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
    community_id: CommunityId,
    agent_pubkey: &[u8],
    owner_pubkey: &[u8],
) -> Result<bool> {
    set_agent_owner_with_operation(
        pool,
        community_id,
        agent_pubkey,
        owner_pubkey,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
}

async fn set_agent_owner_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    agent_pubkey: &[u8],
    owner_pubkey: &[u8],
    operation: crate::observability::WriterOperation,
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    // Ownership is first-write-wins and is never cleared by supported writers.
    // Already-linked agents are the common authorization path: avoid queuing
    // those read-only calls behind a community's claim/restriction transaction.
    let recorded: Option<Option<Vec<u8>>> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community_id.as_uuid())
    .bind(agent_pubkey)
    .fetch_optional(&mut *connection)
    .await?;
    match recorded {
        None => {
            return Err(crate::error::DbError::NotFound(
                "agent pubkey not found in users table".into(),
            ))
        }
        Some(Some(_)) => return Ok(false),
        Some(None) => {}
    }
    drop(connection);
    let mut tx = crate::begin_community_write_transaction(pool, community_id, operation).await?;
    crate::relay_invite::lock_admission(tx.conn(), community_id).await?;
    // Conditional UPDATE: only set owner if currently NULL. This makes
    // "first mint wins" atomic — no TOCTOU race between concurrent mints.
    let result = sqlx::query(
        r#"UPDATE users SET agent_owner_pubkey = $1 WHERE community_id = $2 AND pubkey = $3 AND agent_owner_pubkey IS NULL"#,
    )
    .bind(owner_pubkey)
    .bind(community_id.as_uuid())
    .bind(agent_pubkey)
    .execute(tx.conn())
    .await?;

    if result.rows_affected() == 0 {
        // Could be: (a) pubkey not found, or (b) owner already set.
        // Check which case by querying the row.
        let exists = sqlx::query(r#"SELECT 1 FROM users WHERE community_id = $1 AND pubkey = $2"#)
            .bind(community_id.as_uuid())
            .bind(agent_pubkey)
            .fetch_optional(tx.conn())
            .await?;
        if exists.is_none() {
            return Err(crate::error::DbError::NotFound(
                "agent pubkey not found in users table".into(),
            ));
        }
        // Row exists but owner already set — return false (not an error).
        tx.commit().await?;
        return Ok(false);
    }
    crate::relay_invite::revoke_restricted_agent_invites(
        tx.conn(),
        community_id,
        agent_pubkey,
        owner_pubkey,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Get the channel_add_policy and agent_owner_pubkey for a user.
/// Returns None if the pubkey is not in the users table.
/// Returns Some((policy_str, owner_bytes_or_none)) if found.
pub async fn get_agent_channel_policy(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<(String, Option<Vec<u8>>)>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        r#"SELECT channel_add_policy::text AS channel_add_policy, agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2"#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut *connection)
    .await?;

    row.map(|r| -> Result<(String, Option<Vec<u8>>)> {
        let policy: String = r.try_get("channel_add_policy")?;
        let owner: Option<Vec<u8>> = r.try_get("agent_owner_pubkey").unwrap_or(None);
        Ok((policy, owner))
    })
    .transpose()
}

/// A user's `channel_add_policy`, and their `agent_owner_pubkey` if any.
pub type AgentChannelPolicy = (String, Option<Vec<u8>>);

/// [`get_agent_channel_policy`] for several pubkeys in one query. A pubkey
/// missing from the map is not in the users table.
pub async fn get_agent_channel_policies(
    pool: &PgPool,
    community_id: CommunityId,
    pubkeys: &[Vec<u8>],
) -> Result<std::collections::HashMap<Vec<u8>, AgentChannelPolicy>> {
    if pubkeys.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let rows = sqlx::query(
        r#"SELECT pubkey, channel_add_policy::text AS channel_add_policy, agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = ANY($2)"#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkeys)
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(|r| -> Result<(Vec<u8>, AgentChannelPolicy)> {
            let pubkey: Vec<u8> = r.try_get("pubkey")?;
            let policy: String = r.try_get("channel_add_policy")?;
            let owner: Option<Vec<u8>> = r.try_get("agent_owner_pubkey").unwrap_or(None);
            Ok((pubkey, (policy, owner)))
        })
        .collect()
}

/// List the agents whose `agent_owner_pubkey` is `owner_pubkey` in one community.
pub async fn list_agents_for_owner(
    pool: &PgPool,
    community_id: CommunityId,
    owner_pubkey: &[u8],
) -> Result<Vec<Vec<u8>>> {
    Ok(sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT pubkey FROM users WHERE community_id = $1 AND agent_owner_pubkey = $2",
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .fetch_all(pool)
    .await?)
}

/// Of `candidates`, the agents whose `agent_owner_pubkey` is `owner_pubkey` in
/// one community.
pub async fn owned_agents_among(
    pool: &PgPool,
    community_id: CommunityId,
    owner_pubkey: &[u8],
    candidates: &[Vec<u8>],
) -> Result<Vec<Vec<u8>>> {
    Ok(sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT pubkey FROM users WHERE community_id = $1 AND agent_owner_pubkey = $2 AND pubkey = ANY($3)",
    )
    .bind(community_id.as_uuid())
    .bind(owner_pubkey)
    .bind(candidates)
    .fetch_all(pool)
    .await?)
}

/// Check whether `actor_pubkey` is the `agent_owner_pubkey` of `target_pubkey`.
/// Queries `agent_owner_pubkey` directly rather than going through
/// `get_agent_channel_policy`, which would fetch unrelated fields.
pub async fn is_agent_owner(
    pool: &PgPool,
    community_id: CommunityId,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query_scalar::<_, bool>(
        "SELECT agent_owner_pubkey = $3 FROM users WHERE community_id = $1 AND pubkey = $2 AND agent_owner_pubkey IS NOT NULL",
    )
    .bind(community_id.as_uuid())
    .bind(target_pubkey)
    .bind(actor_pubkey)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(row.unwrap_or(false))
}

/// Set the channel_add_policy for a user.
/// Returns an error if the pubkey is not found (rows_affected == 0).
/// Returns an error if `policy` is not one of the valid ENUM values.
pub async fn set_channel_add_policy(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    policy: &str,
) -> Result<()> {
    if !matches!(policy, "anyone" | "owner_only" | "nobody") {
        return Err(crate::error::DbError::InvalidData(format!(
            "invalid channel_add_policy: {policy}"
        )));
    }
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let result = sqlx::query(
        r#"UPDATE users SET channel_add_policy = $1::channel_add_policy WHERE community_id = $2 AND pubkey = $3"#,
    )
    .bind(policy)
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .execute(tx.conn())
    .await?;
    tx.commit().await?;
    if result.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(
            "pubkey not found in users table".into(),
        ));
    }
    Ok(())
}

impl Db {
    /// Ensure a user record exists (upsert).
    ///
    /// Returns `true` if a new row was inserted (first time), `false` if it
    /// already existed. Callers use the `true` return to increment
    /// `buzz_users_created_total`.
    #[datastore_span(name = "ensure_user", system = "postgresql")]
    pub async fn ensure_user(&self, community_id: CommunityId, pubkey: &[u8]) -> Result<bool> {
        crate::user::ensure_user(&self.pool, community_id, pubkey).await
    }

    /// Ensure a principal while materializing an authenticated NIP-OA
    /// authorization relationship.
    #[datastore_span(name = "ensure_user_for_authorization", system = "postgresql")]
    pub async fn ensure_user_for_authorization(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<bool> {
        ensure_user_with_operation(
            &self.pool,
            community_id,
            pubkey,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Get a single user record by pubkey.
    #[datastore_span(name = "get_user", system = "postgresql")]
    pub async fn get_user(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<UserProfile>> {
        crate::user::get_user(&self.pool, community_id, pubkey).await
    }

    /// Update a user's profile fields.
    #[datastore_span(name = "update_user_profile", system = "postgresql")]
    pub async fn update_user_profile(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        display_name: Option<&str>,
        avatar_url: Option<&str>,
        about: Option<&str>,
        nip05_handle: Option<&str>,
    ) -> Result<()> {
        crate::user::update_user_profile(
            &self.pool,
            community_id,
            pubkey,
            display_name,
            avatar_url,
            about,
            nip05_handle,
        )
        .await
    }

    /// Look up a user by NIP-05 handle.
    #[datastore_span(name = "get_user_by_nip05", system = "postgresql")]
    pub async fn get_user_by_nip05(
        &self,
        community_id: CommunityId,
        local_part: &str,
        domain: &str,
    ) -> Result<Option<UserProfile>> {
        crate::user::get_user_by_nip05(&self.pool, community_id, local_part, domain).await
    }

    /// Search users by name, display name, NIP-05 handle, or pubkey prefix.
    /// See [`crate::user::search_users`] for ranking and filtering rules.
    #[datastore_span(name = "search_users", system = "postgresql")]
    pub async fn search_users(
        &self,
        community_id: CommunityId,
        params: &SearchUsersParams<'_>,
    ) -> Result<Vec<UserSearchProfile>> {
        crate::user::search_users(&self.pool, community_id, params).await
    }

    /// Atomically set agent owner — only if no owner is currently assigned.
    /// Returns Ok(true) if set, Ok(false) if an owner already exists.
    #[datastore_span(name = "set_agent_owner", system = "postgresql")]
    pub async fn set_agent_owner(
        &self,
        community_id: CommunityId,
        agent_pubkey: &[u8],
        owner_pubkey: &[u8],
    ) -> Result<bool> {
        crate::user::set_agent_owner(&self.pool, community_id, agent_pubkey, owner_pubkey).await
    }

    /// Materialize an authenticated NIP-OA agent-owner relationship under
    /// authorization attribution.
    #[datastore_span(name = "set_agent_owner_for_authorization", system = "postgresql")]
    pub async fn set_agent_owner_for_authorization(
        &self,
        community_id: CommunityId,
        agent_pubkey: &[u8],
        owner_pubkey: &[u8],
    ) -> Result<bool> {
        set_agent_owner_with_operation(
            &self.pool,
            community_id,
            agent_pubkey,
            owner_pubkey,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Get the channel_add_policy and agent_owner_pubkey for a user.
    #[datastore_span(name = "get_agent_channel_policy", system = "postgresql")]
    pub async fn get_agent_channel_policy(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<(String, Option<Vec<u8>>)>> {
        crate::user::get_agent_channel_policy(&self.pool, community_id, pubkey).await
    }

    /// [`Db::get_agent_channel_policy`] for several pubkeys in one query.
    #[datastore_span(name = "get_agent_channel_policies", system = "postgresql")]
    pub async fn get_agent_channel_policies(
        &self,
        community_id: CommunityId,
        pubkeys: &[Vec<u8>],
    ) -> Result<std::collections::HashMap<Vec<u8>, AgentChannelPolicy>> {
        crate::user::get_agent_channel_policies(&self.pool, community_id, pubkeys).await
    }

    /// List the agents owned by `owner_pubkey` in one community.
    #[datastore_span(name = "list_agents_for_owner", system = "postgresql")]
    pub async fn list_agents_for_owner(
        &self,
        community_id: CommunityId,
        owner_pubkey: &[u8],
    ) -> Result<Vec<Vec<u8>>> {
        crate::user::list_agents_for_owner(&self.pool, community_id, owner_pubkey).await
    }

    /// Of `candidates`, the agents owned by `owner_pubkey` in one community.
    #[datastore_span(name = "owned_agents_among", system = "postgresql")]
    pub async fn owned_agents_among(
        &self,
        community_id: CommunityId,
        owner_pubkey: &[u8],
        candidates: &[Vec<u8>],
    ) -> Result<Vec<Vec<u8>>> {
        crate::user::owned_agents_among(&self.pool, community_id, owner_pubkey, candidates).await
    }

    /// Check whether `actor_pubkey` is the agent owner of `target_pubkey`.
    #[datastore_span(name = "is_agent_owner", system = "postgresql")]
    pub async fn is_agent_owner(
        &self,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<bool> {
        crate::user::is_agent_owner(&self.pool, community_id, target_pubkey, actor_pubkey).await
    }

    /// Set the channel_add_policy for a user.
    #[datastore_span(name = "set_channel_add_policy", system = "postgresql")]
    pub async fn set_channel_add_policy(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
        policy: &str,
    ) -> Result<()> {
        crate::user::set_channel_add_policy(&self.pool, community_id, pubkey, policy).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::Db;
    use nostr::Keys;

    async fn setup_db() -> Db {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = uuid::Uuid::new_v4();
        let host = format!("user-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    /// Setting an agent owner then reading back the policy should return
    /// the default "anyone" policy and the owner pubkey.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_agent_owner_and_get_policy() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let agent_pk = random_pubkey();
        let owner_pk = random_pubkey();

        ensure_user(&db.pool, community, &agent_pk)
            .await
            .expect("ensure agent");
        ensure_user(&db.pool, community, &owner_pk)
            .await
            .expect("ensure owner");

        let was_set = set_agent_owner(&db.pool, community, &agent_pk, &owner_pk)
            .await
            .expect("set_agent_owner");
        assert!(was_set, "first set_agent_owner should return true");

        let result = get_agent_channel_policy(&db.pool, community, &agent_pk)
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
        let community = make_community(&db.pool).await;
        let pk = random_pubkey();
        ensure_user(&db.pool, community, &pk)
            .await
            .expect("ensure user");

        // owner_only
        set_channel_add_policy(&db.pool, community, &pk, "owner_only")
            .await
            .expect("set owner_only");
        let (policy, owner) = get_agent_channel_policy(&db.pool, community, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(policy, "owner_only");
        assert!(owner.is_none(), "no owner was set");

        // nobody
        set_channel_add_policy(&db.pool, community, &pk, "nobody")
            .await
            .expect("set nobody");
        let (policy, owner) = get_agent_channel_policy(&db.pool, community, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");
        assert_eq!(policy, "nobody");
        assert!(owner.is_none());

        // anyone (reset to default)
        set_channel_add_policy(&db.pool, community, &pk, "anyone")
            .await
            .expect("set anyone");
        let (policy, owner) = get_agent_channel_policy(&db.pool, community, &pk)
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
        let community = make_community(&db.pool).await;
        let pk = random_pubkey();

        let result = get_agent_channel_policy(&db.pool, community, &pk)
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
        let community = make_community(&db.pool).await;
        let agent_pk = random_pubkey();
        let owner_pk = random_pubkey();

        // Only ensure the owner exists -- agent is intentionally absent.
        ensure_user(&db.pool, community, &owner_pk)
            .await
            .expect("ensure owner");

        let result = set_agent_owner(&db.pool, community, &agent_pk, &owner_pk).await;
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
        let community = make_community(&db.pool).await;
        let agent_pk = random_pubkey();
        let owner1 = random_pubkey();
        let owner2 = random_pubkey();

        ensure_user(&db.pool, community, &agent_pk)
            .await
            .expect("ensure agent");
        ensure_user(&db.pool, community, &owner1)
            .await
            .expect("ensure owner1");
        ensure_user(&db.pool, community, &owner2)
            .await
            .expect("ensure owner2");

        let first = set_agent_owner(&db.pool, community, &agent_pk, &owner1)
            .await
            .expect("first set");
        assert!(first, "first set should succeed");

        let second = set_agent_owner(&db.pool, community, &agent_pk, &owner2)
            .await
            .expect("second set should not error");
        assert!(!second, "second set should return false (already owned)");

        // Verify original owner is preserved.
        let (_, owner) = get_agent_channel_policy(&db.pool, community, &agent_pk)
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
        let community = make_community(&db.pool).await;
        let pk = random_pubkey();

        let result = set_channel_add_policy(&db.pool, community, &pk, "nobody").await;
        assert!(
            result.is_err(),
            "should error when pubkey is not in users table"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_set_channel_add_policy_rejects_invalid() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let pubkey = nostr::Keys::generate().public_key().to_bytes().to_vec();
        ensure_user(&db.pool, community, &pubkey).await.unwrap();
        let result = set_channel_add_policy(&db.pool, community, &pubkey, "invalid_policy").await;
        assert!(result.is_err(), "should reject invalid policy value");
    }

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
        let community = make_community(&db.pool).await;
        let pk = random_pubkey();
        ensure_user(&db.pool, community, &pk)
            .await
            .expect("ensure user");

        set_channel_add_policy(&db.pool, community, &pk, "owner_only")
            .await
            .expect("set owner_only");

        let result = get_agent_channel_policy(&db.pool, community, &pk)
            .await
            .expect("get policy")
            .expect("should be Some");

        assert_eq!(result.0, "owner_only");
        assert!(result.1.is_none(), "owner should be None when never set");
    }

    /// Seed a `users` row with the searchable columns set explicitly. `head`
    /// is the kind-0 id the row was projected from; `None` is a row only
    /// `ensure_user` created (a pubkey that authenticated without a profile).
    #[allow(clippy::too_many_arguments)]
    async fn seed_user_row(
        pool: &PgPool,
        community: CommunityId,
        pubkey: &[u8],
        name: Option<&str>,
        display_name: Option<&str>,
        nip05: Option<&str>,
        head: Option<&[u8]>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO users (community_id, pubkey, name, display_name, nip05_handle, metadata_event_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .bind(name)
        .bind(display_name)
        .bind(nip05)
        .bind(head)
        .execute(pool)
        .await
        .expect("seed user");
    }

    /// A projected `users` row for a fresh random pubkey.
    async fn seed_user(
        pool: &PgPool,
        community: CommunityId,
        name: Option<&str>,
        display_name: Option<&str>,
        nip05: Option<&str>,
    ) -> Vec<u8> {
        let pubkey = random_pubkey();
        let head = random_pubkey();
        seed_user_row(
            pool,
            community,
            &pubkey,
            name,
            display_name,
            nip05,
            Some(&head),
        )
        .await;
        pubkey
    }

    async fn search(
        db: &Db,
        community: CommunityId,
        params: SearchUsersParams<'_>,
    ) -> Vec<Vec<u8>> {
        db.search_users(community, &params)
            .await
            .expect("search users")
            .into_iter()
            .map(|profile| profile.pubkey)
            .collect()
    }

    /// `name` is searched independently of `display_name`: a profile whose
    /// only match is in `name` is found, and exact matches in either column
    /// outrank prefix matches.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_matches_name_and_display_name_separately() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let name_only = seed_user(&db.pool, community, Some("wes"), None, None).await;
        let display_prefix =
            seed_user(&db.pool, community, Some("wh"), Some("Wesley H"), None).await;
        let display_exact = seed_user(&db.pool, community, Some("zz"), Some("Wes"), None).await;
        let _unrelated = seed_user(&db.pool, community, Some("alice"), Some("Alice"), None).await;

        let found = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 10,
                ..Default::default()
            },
        )
        .await;

        assert_eq!(found.len(), 3, "both name and display_name matches surface");
        // Both exact matches share tier 0; their relative order is by label
        // collation, which is locale-dependent, so compare the tier as a set.
        let first_tier: std::collections::BTreeSet<_> = found[..2].iter().cloned().collect();
        let expected_tier: std::collections::BTreeSet<_> =
            [display_exact, name_only].into_iter().collect();
        assert_eq!(
            first_tier, expected_tier,
            "exact matches (name or display_name) fill the first tier"
        );
        assert_eq!(found[2], display_prefix, "prefix match ranks after exact");
    }

    /// `authors` restricts the result set to the supplied pubkeys and an
    /// empty author list short-circuits to no rows.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_honours_authors_filter() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let wanted = seed_user(&db.pool, community, Some("wes-a"), None, None).await;
        let _other = seed_user(&db.pool, community, Some("wes-b"), None, None).await;

        let authors = vec![wanted.clone()];
        let found = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                authors: Some(&authors),
                limit: 10,
                offset: 0,
            },
        )
        .await;
        assert_eq!(found, vec![wanted]);

        let none = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                authors: Some(&[]),
                limit: 10,
                offset: 0,
            },
        )
        .await;
        assert!(none.is_empty(), "empty authors list matches nothing");
    }

    /// `offset` pages through the ranked order without repeats or gaps.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_offset_pages_in_rank_order() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        for label in ["wes-1", "wes-2", "wes-3", "wes-4", "wes-5"] {
            seed_user(&db.pool, community, Some(label), None, None).await;
        }

        let all = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 10,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(all.len(), 5);

        let page_one = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 2,
                offset: 0,
                ..Default::default()
            },
        )
        .await;
        let page_two = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 2,
                offset: 2,
                ..Default::default()
            },
        )
        .await;
        let page_three = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 2,
                offset: 4,
                ..Default::default()
            },
        )
        .await;

        let paged: Vec<Vec<u8>> = page_one
            .into_iter()
            .chain(page_two)
            .chain(page_three)
            .collect();
        assert_eq!(paged, all, "pages concatenate to the unpaged ranking");
    }

    /// Deactivated users never appear in search results.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_excludes_deactivated_users() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let active = seed_user(&db.pool, community, Some("wes-active"), None, None).await;
        let deactivated = seed_user(&db.pool, community, Some("wes-gone"), None, None).await;
        sqlx::query(
            "UPDATE users SET deactivated_at = NOW() WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(&deactivated)
        .execute(&db.pool)
        .await
        .expect("deactivate user");

        let found = search(
            &db,
            community,
            SearchUsersParams {
                query: "wes",
                limit: 10,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(found, vec![active]);
    }

    /// A row without a projected head (AUTH created it; no kind-0 yet) can
    /// never hydrate to an event, so it must not take a page slot. Here it
    /// would have outranked the only real match: a pubkey-prefix hit sorts in
    /// tier 5, ahead of a name that merely contains the query (tier 6).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_skips_rows_without_a_projected_head() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let gabby = seed_user(&db.pool, community, Some("Gabby"), None, None).await;
        let mut headless = random_pubkey();
        headless[0] = 0xab;
        seed_user_row(&db.pool, community, &headless, None, None, None, None).await;

        let first_slot = search(
            &db,
            community,
            SearchUsersParams {
                query: "ab",
                limit: 1,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            first_slot,
            vec![gabby.clone()],
            "the headless row must not take the first slot"
        );
        let all = search(
            &db,
            community,
            SearchUsersParams {
                query: "ab",
                limit: 10,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(all, vec![gabby], "the headless row is never a result");
    }

    /// The hex pubkey matches as a prefix only: a one-character query finds
    /// the pubkeys that start with it, not every pubkey that contains it.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn search_users_matches_pubkey_hex_by_prefix_only() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let mut leading = random_pubkey();
        leading[0] = 0xa0;
        seed_user_row(
            &db.pool,
            community,
            &leading,
            Some("zed"),
            None,
            None,
            Some(&random_pubkey()),
        )
        .await;
        let mut inner = random_pubkey();
        inner[0] = 0x1a;
        seed_user_row(
            &db.pool,
            community,
            &inner,
            Some("zoe"),
            None,
            None,
            Some(&random_pubkey()),
        )
        .await;

        let one_char = search(
            &db,
            community,
            SearchUsersParams {
                query: "a",
                limit: 10,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            one_char,
            vec![leading],
            "`1a…` contains `a` but does not start with it"
        );
        let two_chars = search(
            &db,
            community,
            SearchUsersParams {
                query: "1a",
                limit: 10,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(two_chars, vec![inner]);
    }
}
