//! Channel lifecycle and metadata persistence.
//!
//! Channels have two visibility modes:
//! - `open`: searchable, anyone can join
//! - `private`: hidden, invite-only

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::Db;
use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;

// Re-export the canonical enum definitions from buzz-core.
// These live in core (zero I/O deps) so the SDK can share them
// without pulling in sqlx/tokio.
pub use buzz_core::channel::{ChannelType, ChannelVisibility, MemberRole};

// Keep the established channel module paths compatible while membership SQL
// and invariants live in their dedicated store module.
pub use crate::channel_members::{
    add_member, get_accessible_channel_ids, get_accessible_channels, get_bot_members,
    get_member_count, get_member_counts_bulk, get_member_role, get_members, get_members_bulk,
    get_users_bulk, is_member, list_large_channel_rosters_needing_reconciliation,
    lock_member_snapshot, membership_pairs, remove_member, verify_channel_roster_fence_behavior,
    verify_channel_roster_fence_catalog, AccessibleChannel, BotChannelEntry, BotMemberRecord,
    LargeChannelRoster, LockedMemberSnapshot, MemberRecord, UserRecord,
};

async fn begin_event_write_transaction(
    pool: &PgPool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    Ok(sqlx::Transaction::begin(connection, None).await?)
}

async fn acquire_event_write_connection(
    pool: &PgPool,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    Ok(crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?)
}

/// A channel row as returned from the database.
#[derive(Debug, Clone)]
pub struct ChannelRecord {
    /// Unique channel identifier.
    pub id: Uuid,
    /// Human-readable channel name.
    pub name: String,
    /// Channel type string (e.g. `"stream"`, `"forum"`, `"dm"`).
    pub channel_type: String,
    /// Visibility string (`"open"` or `"private"`).
    pub visibility: String,
    /// Optional channel description.
    pub description: Option<String>,
    /// Optional canvas (rich document) content.
    pub canvas: Option<String>,
    /// Compressed public key bytes of the channel creator.
    pub created_by: Vec<u8>,
    /// When the channel was created.
    pub created_at: DateTime<Utc>,
    /// When the channel was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the channel was archived, if applicable.
    pub archived_at: Option<DateTime<Utc>>,
    /// When the channel was soft-deleted, if applicable.
    pub deleted_at: Option<DateTime<Utc>>,
    /// NIP-29 group ID for external Nostr clients.
    pub nip29_group_id: Option<String>,
    /// Whether posts must be associated with a topic.
    pub topic_required: bool,
    /// Optional cap on the number of members.
    pub max_members: Option<i32>,
    /// Current channel topic (short, visible in header).
    pub topic: Option<String>,
    /// Compressed public key bytes of the user who last set the topic.
    pub topic_set_by: Option<Vec<u8>>,
    /// When the topic was last set.
    pub topic_set_at: Option<DateTime<Utc>>,
    /// Channel purpose / description of intent.
    pub purpose: Option<String>,
    /// Compressed public key bytes of the user who last set the purpose.
    pub purpose_set_by: Option<Vec<u8>>,
    /// When the purpose was last set.
    pub purpose_set_at: Option<DateTime<Utc>>,
    /// TTL in seconds for ephemeral channels. `None` means permanent.
    pub ttl_seconds: Option<i32>,
    /// Deadline by which a new message must arrive or the channel is auto-archived.
    pub ttl_deadline: Option<DateTime<Utc>>,
}

/// Creates a new channel, bootstraps the creator as owner, and returns the record.
#[allow(clippy::too_many_arguments)]
pub async fn create_channel(
    pool: &PgPool,
    community_id: CommunityId,
    name: &str,
    channel_type: ChannelType,
    visibility: ChannelVisibility,
    description: Option<&str>,
    created_by: &[u8],
    ttl_seconds: Option<i32>,
) -> Result<ChannelRecord> {
    if created_by.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "pubkey must be 32 bytes, got {}",
            created_by.len()
        )));
    }

    let name = buzz_core::channel::canonical_channel_name(name);
    if name.trim().is_empty() {
        return Err(DbError::InvalidData("channel name is required".into()));
    }

    let id = Uuid::new_v4();

    let mut tx = begin_event_write_transaction(pool).await?;

    sqlx::query(
        r#"
        INSERT INTO channels (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline)
        VALUES ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END)
        "#,
    )
    .bind(id)
    .bind(community_id.as_uuid())
    .bind(name)
    .bind(channel_type.as_str())
    .bind(visibility.as_str())
    .bind(description)
    .bind(created_by)
    .bind(ttl_seconds)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
        VALUES ($1, $2, $3, 'owner', $4)
        ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
            removed_at = NULL,
            removed_by = NULL,
            role = EXCLUDED.role
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .bind(created_by)
    .bind(created_by)
    .execute(&mut *tx)
    .await?;

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;

    let record = row_to_channel_record(row)?;
    tx.commit().await?;
    Ok(record)
}

/// Creates a channel with a client-supplied UUID (idempotent via ON CONFLICT DO NOTHING).
///
/// Returns `(record, true)` if the channel was newly created, or `(record, false)` if a
/// channel with `channel_id` already exists (duplicate — caller should reject the event).
#[allow(clippy::too_many_arguments)]
pub async fn create_channel_with_id(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    name: &str,
    channel_type: ChannelType,
    visibility: ChannelVisibility,
    description: Option<&str>,
    created_by: &[u8],
    ttl_seconds: Option<i32>,
) -> Result<(ChannelRecord, bool)> {
    if created_by.len() != 32 {
        return Err(DbError::InvalidData(format!(
            "pubkey must be 32 bytes, got {}",
            created_by.len()
        )));
    }

    if channel_id.is_nil() {
        return Err(DbError::InvalidData(
            "channel_id must not be nil (reserved for global fan-out)".into(),
        ));
    }

    let name = buzz_core::channel::canonical_channel_name(name);
    if name.trim().is_empty() {
        return Err(DbError::InvalidData("channel name is required".into()));
    }

    let mut tx = begin_event_write_transaction(pool).await?;

    let rows_affected = sqlx::query(
        r#"
        INSERT INTO channels (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline)
        VALUES ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END)
        ON CONFLICT (community_id, id) DO NOTHING
        "#,
    )
    .bind(channel_id)
    .bind(community_id.as_uuid())
    .bind(name)
    .bind(channel_type.as_str())
    .bind(visibility.as_str())
    .bind(description)
    .bind(created_by)
    .bind(ttl_seconds)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let was_created = rows_affected > 0;

    if was_created {
        // Bootstrap the creator as owner.
        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'owner', $4)
            ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET
                removed_at = NULL,
                removed_by = NULL,
                role = EXCLUDED.role
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(created_by)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
    }

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(&mut *tx)
    .await?;

    let record = row_to_channel_record(row)?;
    tx.commit().await?;
    Ok((record, was_created))
}

/// Fetches a channel record by `(community_id, id)`. Returns `ChannelNotFound` if missing or deleted.
pub async fn get_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<ChannelRecord> {
    get_channel_with_operation(
        pool,
        community_id,
        channel_id,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

async fn get_channel_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    operation: crate::observability::WriterOperation,
) -> Result<ChannelRecord> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline
        FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;

    row_to_channel_record(row)
}

/// Returns the canvas content for a channel, if any.
pub async fn get_canvas(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Option<String>> {
    let row = sqlx::query(
        "SELECT canvas FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;
    Ok(row.try_get("canvas")?)
}

/// Sets or clears the canvas content for a channel.
pub async fn set_canvas(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    canvas: Option<&str>,
) -> Result<()> {
    let rows = sqlx::query(
        "UPDATE channels SET canvas = $1 WHERE community_id = $2 AND id = $3 AND deleted_at IS NULL",
    )
        .bind(canvas)
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(pool)
        .await?;
    if rows.rows_affected() == 0 {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    Ok(())
}

/// Lists channels in a community, optionally filtered by visibility string.
pub async fn list_channels(
    pool: &PgPool,
    community_id: CommunityId,
    visibility: Option<&str>,
) -> Result<Vec<ChannelRecord>> {
    list_channels_with_operation(
        pool,
        community_id,
        visibility,
        crate::observability::WriterOperation::Authorization,
    )
    .await
}

async fn list_channels_with_operation(
    pool: &PgPool,
    community_id: CommunityId,
    visibility: Option<&str>,
    operation: crate::observability::WriterOperation,
) -> Result<Vec<ChannelRecord>> {
    let mut connection = crate::observability::acquire_writer(pool, operation).await?;
    let rows = if let Some(vis) = visibility {
        sqlx::query(
            r#"
            SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
                   description, canvas,
                   created_by, created_at, updated_at, archived_at, deleted_at,
                   nip29_group_id, topic_required, max_members,
                   topic, topic_set_by, topic_set_at,
                   purpose, purpose_set_by, purpose_set_at,
                   ttl_seconds, ttl_deadline
            FROM channels
            WHERE community_id = $1 AND deleted_at IS NULL AND visibility::text = $2
            ORDER BY created_at DESC
            LIMIT 1000
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(vis)
        .fetch_all(&mut *connection)
        .await?
    } else {
        sqlx::query(
            r#"
            SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
                   description, canvas,
                   created_by, created_at, updated_at, archived_at, deleted_at,
                   nip29_group_id, topic_required, max_members,
                   topic, topic_set_by, topic_set_at,
                   purpose, purpose_set_by, purpose_set_at,
                   ttl_seconds, ttl_deadline
            FROM channels
            WHERE community_id = $1 AND deleted_at IS NULL
            ORDER BY created_at DESC
            LIMIT 1000
            "#,
        )
        .bind(community_id.as_uuid())
        .fetch_all(&mut *connection)
        .await?
    };

    rows.into_iter().map(row_to_channel_record).collect()
}

/// A channel archived by the ephemeral-channel reaper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapedEphemeralChannel {
    /// Community that owns the archived channel.
    pub community_id: CommunityId,
    /// Normalized host mapped to that community.
    pub host: String,
    /// Archived channel UUID.
    pub channel_id: Uuid,
}

pub(crate) fn row_to_channel_record(row: sqlx::postgres::PgRow) -> Result<ChannelRecord> {
    let id: Uuid = row.try_get("id")?;
    let topic_required: bool = row.try_get("topic_required")?;

    // topic/purpose fields are new — use try_get and fall back to None if the
    // column is absent (e.g. queries that don't SELECT these columns yet).
    let topic: Option<String> = row.try_get("topic").unwrap_or(None);
    let topic_set_by: Option<Vec<u8>> = row.try_get("topic_set_by").unwrap_or(None);
    let topic_set_at: Option<DateTime<Utc>> = row.try_get("topic_set_at").unwrap_or(None);
    let purpose: Option<String> = row.try_get("purpose").unwrap_or(None);
    let purpose_set_by: Option<Vec<u8>> = row.try_get("purpose_set_by").unwrap_or(None);
    let purpose_set_at: Option<DateTime<Utc>> = row.try_get("purpose_set_at").unwrap_or(None);
    let ttl_seconds: Option<i32> = row.try_get("ttl_seconds").unwrap_or(None);
    let ttl_deadline: Option<DateTime<Utc>> = row.try_get("ttl_deadline").unwrap_or(None);

    Ok(ChannelRecord {
        id,
        name: row.try_get("name")?,
        channel_type: row.try_get("channel_type")?,
        visibility: row.try_get("visibility")?,
        description: row.try_get("description")?,
        canvas: row.try_get("canvas")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        archived_at: row.try_get("archived_at")?,
        deleted_at: row.try_get("deleted_at")?,
        nip29_group_id: row.try_get("nip29_group_id")?,
        topic_required,
        max_members: row.try_get("max_members")?,
        topic,
        topic_set_by,
        topic_set_at,
        purpose,
        purpose_set_by,
        purpose_set_at,
        ttl_seconds,
        ttl_deadline,
    })
}

/// Partial update for channel metadata. Every field is `None` to leave the
/// column unchanged.
#[derive(Default)]
pub struct ChannelUpdate {
    /// New channel name, or `None` to leave unchanged.
    pub name: Option<String>,
    /// New channel description, or `None` to leave unchanged.
    pub description: Option<String>,
    /// New visibility (`"open"`/`"private"`), or `None` to leave unchanged.
    pub visibility: Option<String>,
    /// TTL change: outer `None` leaves it unchanged, `Some(None)` clears the
    /// ephemeral TTL (channel becomes permanent), `Some(Some(secs))` sets it.
    /// On any change the `ttl_deadline` is reset to `NOW() + ttl_seconds`.
    pub ttl_seconds: Option<Option<i32>>,
}

/// Updates channel metadata dynamically.
///
/// At least one field must be provided; returns `InvalidData` otherwise.
/// Returns the updated `ChannelRecord` on success.
pub async fn update_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    mut updates: ChannelUpdate,
) -> Result<ChannelRecord> {
    if updates.name.is_none()
        && updates.description.is_none()
        && updates.visibility.is_none()
        && updates.ttl_seconds.is_none()
    {
        return Err(DbError::InvalidData(
            "at least one field must be provided for update".to_string(),
        ));
    }

    if let Some(name) = updates.name.as_mut() {
        *name = buzz_core::channel::canonical_channel_name(name).to_owned();
        if name.is_empty() {
            return Err(DbError::InvalidData("channel name is required".into()));
        }
    }

    // Build SET clause dynamically — only include fields that are provided.
    // Track parameter index for positional placeholders.
    let mut set_parts: Vec<String> = Vec::new();
    let mut param_idx: usize = 1;
    if updates.name.is_some() {
        set_parts.push(format!("name = ${param_idx}"));
        param_idx += 1;
    }
    if updates.description.is_some() {
        set_parts.push(format!("description = ${param_idx}"));
        param_idx += 1;
    }
    if updates.visibility.is_some() {
        set_parts.push(format!("visibility = ${param_idx}::channel_visibility"));
        param_idx += 1;
    }
    if let Some(ref ttl) = updates.ttl_seconds {
        // Set ttl_seconds, then reset the deadline from now (or clear both).
        set_parts.push(format!("ttl_seconds = ${param_idx}"));
        param_idx += 1;
        match ttl {
            Some(_) => set_parts.push(format!(
                "ttl_deadline = NOW() + (${} || ' seconds')::interval",
                param_idx - 1
            )),
            None => set_parts.push("ttl_deadline = NULL".to_string()),
        }
    }
    let channel_param_idx = param_idx + 1;
    let sql = format!(
        "UPDATE channels SET {}, updated_at = NOW() WHERE community_id = ${param_idx} AND id = ${channel_param_idx} AND deleted_at IS NULL",
        set_parts.join(", ")
    );

    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    if let Some(ref name) = updates.name {
        q = q.bind(name);
    }
    if let Some(ref desc) = updates.description {
        q = q.bind(desc);
    }
    if let Some(ref vis) = updates.visibility {
        q = q.bind(vis);
    }
    if let Some(ref ttl) = updates.ttl_seconds {
        q = q.bind(*ttl);
    }
    q = q.bind(community_id.as_uuid());
    q = q.bind(channel_id);

    // T1a repair: a TTL change can flip this channel's event-trigger fast
    // path (migration 0024 reads ttl_seconds under a SHARED per-channel
    // advisory lock). Take the same key EXCLUSIVE before the UPDATE so a
    // concurrent event either sees the committed TTL or strictly precedes
    // this transition — whose own deadline reset is then the latest word.
    // Non-TTL updates don't touch the fast path and skip the lock.
    if updates.ttl_seconds.is_some() {
        let mut tx = begin_event_write_transaction(pool).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!(
                "buzz_channel_ttl:{}:{}",
                community_id.as_uuid(),
                channel_id
            ))
            .execute(&mut *tx)
            .await?;
        let result = q.execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::ChannelNotFound(channel_id));
        }
        tx.commit().await?;
    } else {
        let mut connection = acquire_event_write_connection(pool).await?;
        let result = q.execute(&mut *connection).await?;
        if result.rows_affected() == 0 {
            return Err(DbError::ChannelNotFound(channel_id));
        }
    }

    get_channel_with_operation(
        pool,
        community_id,
        channel_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
}

/// Sets the topic for a channel, recording who set it and when.
pub async fn set_topic(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    topic: &str,
    set_by: &[u8],
) -> Result<()> {
    let mut connection = acquire_event_write_connection(pool).await?;
    let result = sqlx::query(
        "UPDATE channels SET topic = $1, topic_set_by = $2, topic_set_at = NOW() \
         WHERE community_id = $3 AND id = $4 AND deleted_at IS NULL",
    )
    .bind(topic)
    .bind(set_by)
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .execute(&mut *connection)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    Ok(())
}

/// Sets the purpose for a channel, recording who set it and when.
pub async fn set_purpose(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    purpose: &str,
    set_by: &[u8],
) -> Result<()> {
    let mut connection = acquire_event_write_connection(pool).await?;
    let result = sqlx::query(
        "UPDATE channels SET purpose = $1, purpose_set_by = $2, purpose_set_at = NOW() \
         WHERE community_id = $3 AND id = $4 AND deleted_at IS NULL",
    )
    .bind(purpose)
    .bind(set_by)
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .execute(&mut *connection)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    Ok(())
}

/// Archives a channel.
///
/// Returns `AccessDenied` if the channel is already archived.
/// Returns `ChannelNotFound` if the channel does not exist or is deleted.
pub async fn archive_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<()> {
    let mut connection = acquire_event_write_connection(pool).await?;
    // First check: does the channel exist and what is its state?
    let row = sqlx::query(
        "SELECT archived_at FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut *connection)
        .await?;

    match row {
        None => return Err(DbError::ChannelNotFound(channel_id)),
        Some(r) => {
            let archived_at: Option<DateTime<Utc>> = r.try_get("archived_at")?;
            if archived_at.is_some() {
                return Err(DbError::AccessDenied(
                    "channel is already archived".to_string(),
                ));
            }
        }
    }

    sqlx::query(
        "UPDATE channels SET archived_at = NOW() \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL AND archived_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

/// Unarchives a channel.
///
/// Returns `AccessDenied` if the channel is not currently archived.
/// Returns `ChannelNotFound` if the channel does not exist or is deleted.
pub async fn unarchive_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<()> {
    let mut connection = acquire_event_write_connection(pool).await?;
    // First check: does the channel exist and what is its state?
    let row = sqlx::query(
        "SELECT archived_at FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut *connection)
        .await?;

    match row {
        None => return Err(DbError::ChannelNotFound(channel_id)),
        Some(r) => {
            let archived_at: Option<DateTime<Utc>> = r.try_get("archived_at")?;
            if archived_at.is_none() {
                return Err(DbError::AccessDenied("channel is not archived".to_string()));
            }
        }
    }

    sqlx::query(
        "UPDATE channels SET archived_at = NULL, \
             ttl_deadline = CASE \
                 WHEN ttl_seconds IS NOT NULL THEN NOW() + (ttl_seconds || ' seconds')::interval \
                 ELSE ttl_deadline \
             END \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL AND archived_at IS NOT NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .execute(&mut *connection)
    .await?;

    Ok(())
}

/// Soft-delete a channel by setting `deleted_at = NOW()`.
///
/// Returns `Ok(true)` if the channel was deleted, `Ok(false)` if already
/// deleted or not found.
pub async fn soft_delete_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<bool> {
    let mut connection = acquire_event_write_connection(pool).await?;
    let result = sqlx::query(
        "UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
            .bind(community_id.as_uuid())
            .bind(channel_id)
            .execute(&mut *connection)
            .await?;

    Ok(result.rows_affected() > 0)
}

/// Archive ephemeral channels whose TTL deadline has passed.
///
/// Returns the `(community_id, host, channel_id)` list that was archived. Idempotent — the
/// `archived_at IS NULL` guard prevents double-archiving even if called
/// concurrently from multiple relay pods.
pub async fn reap_expired_ephemeral_channels(pool: &PgPool) -> Result<Vec<ReapedEphemeralChannel>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?;
    let rows = sqlx::query(
        "UPDATE channels AS ch SET archived_at = NOW() \
         FROM communities AS c \
         WHERE ch.community_id = c.id \
           AND ch.ttl_seconds IS NOT NULL \
           AND ch.ttl_deadline < NOW() \
           AND ch.archived_at IS NULL \
           AND ch.deleted_at IS NULL \
           AND c.archived_at IS NULL \
           AND community_write_allowed(ch.community_id) \
         RETURNING ch.community_id, c.host, ch.id",
    )
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(|row| {
            let community_id: Uuid = row.try_get("community_id")?;
            let host: String = row.try_get("host")?;
            let channel_id: Uuid = row.try_get("id")?;
            Ok(ReapedEphemeralChannel {
                community_id: CommunityId::from_uuid(community_id),
                host,
                channel_id,
            })
        })
        .collect()
}

impl Db {
    /// Creates a new channel, bootstraps the creator as owner, and returns the record.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "create_channel", system = "postgresql")]
    pub async fn create_channel(
        &self,
        community_id: CommunityId,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<ChannelRecord> {
        create_channel(
            &self.pool,
            community_id,
            name,
            channel_type,
            visibility,
            description,
            created_by,
            ttl_seconds,
        )
        .await
    }

    /// Creates a channel with a client-supplied UUID.
    ///
    /// Returns `(record, true)` if newly created, `(record, false)` if already exists.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "create_channel_with_id", system = "postgresql")]
    pub async fn create_channel_with_id(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<(ChannelRecord, bool)> {
        create_channel_with_id(
            &self.pool,
            community_id,
            channel_id,
            name,
            channel_type,
            visibility,
            description,
            created_by,
            ttl_seconds,
        )
        .await
    }

    /// Fetches a channel record by ID.
    #[datastore_span(name = "get_channel", system = "postgresql")]
    pub async fn get_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<ChannelRecord> {
        get_channel(&self.pool, community_id, channel_id).await
    }

    /// Fetch a channel whose result directly gates an event mutation or
    /// post-commit event side effect.
    #[datastore_span(name = "get_channel_for_event_write", system = "postgresql")]
    pub async fn get_channel_for_event_write(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<ChannelRecord> {
        get_channel_with_operation(
            &self.pool,
            community_id,
            channel_id,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
    }

    /// Returns the canvas content for a channel, if any.
    #[datastore_span(name = "get_canvas", system = "postgresql")]
    pub async fn get_canvas(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Option<String>> {
        get_canvas(&self.pool, community_id, channel_id).await
    }

    /// Sets or clears the canvas content for a channel.
    #[datastore_span(name = "set_canvas", system = "postgresql")]
    pub async fn set_canvas(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        canvas: Option<&str>,
    ) -> Result<()> {
        set_canvas(&self.pool, community_id, channel_id, canvas).await
    }

    /// Lists channels, optionally filtered by visibility.
    #[datastore_span(name = "list_channels", system = "postgresql")]
    pub async fn list_channels(
        &self,
        community_id: CommunityId,
        visibility: Option<&str>,
    ) -> Result<Vec<ChannelRecord>> {
        list_channels(&self.pool, community_id, visibility).await
    }

    /// Lists channels during startup reconciliation.
    #[datastore_span(name = "list_channels_for_bootstrap", system = "postgresql")]
    pub async fn list_channels_for_bootstrap(
        &self,
        community_id: CommunityId,
        visibility: Option<&str>,
    ) -> Result<Vec<ChannelRecord>> {
        list_channels_with_operation(
            &self.pool,
            community_id,
            visibility,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await
    }

    /// Updates a channel's name and/or description.
    #[datastore_span(name = "update_channel", system = "postgresql")]
    pub async fn update_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        updates: ChannelUpdate,
    ) -> Result<ChannelRecord> {
        update_channel(&self.pool, community_id, channel_id, updates).await
    }

    /// Sets the topic for a channel.
    #[datastore_span(name = "set_topic", system = "postgresql")]
    pub async fn set_topic(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        topic: &str,
        set_by: &[u8],
    ) -> Result<()> {
        set_topic(&self.pool, community_id, channel_id, topic, set_by).await
    }

    /// Sets the purpose for a channel.
    #[datastore_span(name = "set_purpose", system = "postgresql")]
    pub async fn set_purpose(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        purpose: &str,
        set_by: &[u8],
    ) -> Result<()> {
        set_purpose(&self.pool, community_id, channel_id, purpose, set_by).await
    }

    /// Archives a channel.
    #[datastore_span(name = "archive_channel", system = "postgresql")]
    pub async fn archive_channel(&self, community_id: CommunityId, channel_id: Uuid) -> Result<()> {
        archive_channel(&self.pool, community_id, channel_id).await
    }

    /// Unarchives a channel.
    #[datastore_span(name = "unarchive_channel", system = "postgresql")]
    pub async fn unarchive_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<()> {
        unarchive_channel(&self.pool, community_id, channel_id).await
    }

    /// Soft-delete a channel.
    #[datastore_span(name = "soft_delete_channel", system = "postgresql")]
    pub async fn soft_delete_channel(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<bool> {
        soft_delete_channel(&self.pool, community_id, channel_id).await
    }

    /// Archive ephemeral channels whose TTL deadline has passed.
    #[datastore_span(name = "reap_expired_ephemeral_channels", system = "postgresql")]
    pub async fn reap_expired_ephemeral_channels(&self) -> Result<Vec<ReapedEphemeralChannel>> {
        reap_expired_ephemeral_channels(&self.pool).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::user::ensure_user;
    use nostr::Keys;

    async fn setup_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn make_test_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("channel-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_test_channel(
        pool: &PgPool,
        community_id: Uuid,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Result<ChannelRecord> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline)
            VALUES
                ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                 CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END)
            "#,
        )
        .bind(id)
        .bind(community_id)
        .bind(name)
        .bind(channel_type.as_str())
        .bind(visibility.as_str())
        .bind(description)
        .bind(created_by)
        .bind(ttl_seconds)
        .execute(pool)
        .await
        .expect("insert test channel");

        sqlx::query(
            r#"
            INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
            VALUES ($1, $2, $3, 'owner', $4)
            "#,
        )
        .bind(community_id)
        .bind(id)
        .bind(created_by)
        .bind(created_by)
        .execute(pool)
        .await
        .expect("insert owner membership");

        get_channel(pool, CommunityId::from_uuid(community_id), id).await
    }

    async fn insert_channel_with_id(
        pool: &PgPool,
        community_id: Uuid,
        id: Uuid,
        name: &str,
        created_by: &[u8],
    ) {
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES
                ($1, $2, $3, 'stream', 'open', $4)
            "#,
        )
        .bind(id)
        .bind(community_id)
        .bind(name)
        .bind(created_by)
        .execute(pool)
        .await
        .expect("insert channel with fixed id");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn get_channel_is_scoped_when_channel_uuid_collides_across_communities() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let channel_id = Uuid::new_v4();
        let creator = random_pubkey();

        insert_channel_with_id(
            &pool,
            community_a,
            channel_id,
            "community-a-channel",
            &creator,
        )
        .await;
        insert_channel_with_id(
            &pool,
            community_b,
            channel_id,
            "community-b-channel",
            &creator,
        )
        .await;

        let a = get_channel(&pool, CommunityId::from_uuid(community_a), channel_id)
            .await
            .expect("community A channel should resolve");
        let b = get_channel(&pool, CommunityId::from_uuid(community_b), channel_id)
            .await
            .expect("community B channel should resolve");

        assert_eq!(a.name, "community-a-channel");
        assert_eq!(b.name, "community-b-channel");

        let listed_a = list_channels(&pool, CommunityId::from_uuid(community_a), None)
            .await
            .expect("list community A channels");
        assert!(listed_a
            .iter()
            .any(|row| row.id == channel_id && row.name == "community-a-channel"));
        assert!(!listed_a
            .iter()
            .any(|row| row.id == channel_id && row.name == "community-b-channel"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_unarchive_expired_ephemeral_channel_renews_ttl_deadline() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner_pk = random_pubkey();
        ensure_user(&pool, community, &owner_pk)
            .await
            .expect("ensure owner");

        let channel = create_test_channel(
            &pool,
            community_id,
            "test-unarchive-renews-ttl",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner_pk,
            Some(60),
        )
        .await
        .expect("create ephemeral channel");

        sqlx::query(
            "UPDATE channels SET archived_at = NOW(), ttl_deadline = NOW() - interval '1 second' WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id)
        .bind(channel.id)
        .execute(&pool)
        .await
        .expect("expire and archive channel");

        unarchive_channel(&pool, community, channel.id)
            .await
            .expect("unarchive expired ephemeral channel");

        let channel = get_channel(&pool, community, channel.id)
            .await
            .expect("reload channel");
        assert!(
            channel.archived_at.is_none(),
            "channel should be unarchived"
        );
        assert!(
            channel.ttl_deadline.expect("ttl deadline") > Utc::now(),
            "unarchive should renew ttl_deadline into the future"
        );

        let reaped = reap_expired_ephemeral_channels(&pool)
            .await
            .expect("run reaper");
        assert!(
            !reaped
                .iter()
                .any(|row| row.community_id == community && row.channel_id == channel.id),
            "reaper should not immediately rearchive renewed channel"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn reap_expired_ephemeral_channels_returns_row_community_and_host() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let expected_host: String =
            sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("load community host");
        let owner_pk = random_pubkey();
        ensure_user(&pool, community, &owner_pk)
            .await
            .expect("ensure owner");
        let channel = create_test_channel(
            &pool,
            community_id,
            "test-reaper-host-provenance",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner_pk,
            Some(60),
        )
        .await
        .expect("create ephemeral channel");

        sqlx::query(
            "UPDATE channels SET ttl_deadline = NOW() - interval '1 second' WHERE community_id = $1 AND id = $2",
        )
        .bind(community_id)
        .bind(channel.id)
        .execute(&pool)
        .await
        .expect("expire channel");

        let reaped = reap_expired_ephemeral_channels(&pool)
            .await
            .expect("run reaper");
        assert!(
            reaped.iter().any(|row| {
                row.community_id == community
                    && row.host == expected_host
                    && row.channel_id == channel.id
            }),
            "reaper should carry the archived row's community id and host"
        );
    }
}
