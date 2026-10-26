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
use buzz_core::{CommunityId, StoredEvent};
use buzz_datastore_tracing::datastore_span;

// Re-export the canonical enum definitions from buzz-core.
// These live in core (zero I/O deps) so the SDK can share them
// without pulling in sqlx/tokio.
pub use buzz_core::channel::{
    decide_member_put, ChannelJoinRole, ChannelType, ChannelVisibility, MemberPut, MemberPutDenial,
    MemberRole,
};

// Keep the established channel module paths compatible while membership SQL
// and invariants live in their dedicated store module.
pub use crate::channel_members::{
    add_member, get_accessible_channel_ids, get_accessible_channels, get_bot_members,
    get_member_count, get_member_counts_bulk, get_member_role, get_members, get_members_bulk,
    get_users_bulk, is_member, list_large_channel_rosters_needing_reconciliation,
    lock_member_snapshot, membership_pairs, remove_member, verify_channel_roster_fence_behavior,
    verify_channel_roster_fence_catalog, AccessibleChannel, BotChannelEntry, BotMemberRecord,
    ChannelIdentity, LargeChannelRoster, LockedMemberSnapshot, MemberRecord, UserRecord,
};

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
    /// Labels published as `["t", <label>]` after the channel type tag.
    pub labels: Vec<String>,
    /// The role a new person gets (`"member"` or `"guest"`). Parse with
    /// [`buzz_core::channel::ChannelJoinRole::parse_fail_closed`].
    pub join_role: String,
    /// Whether the channel is left out of channel listings for non-members.
    /// It matters only while the channel is open: anyone can still read and
    /// join an unlisted open channel by naming it, and private channels are
    /// never listed to non-members.
    pub unlisted: bool,
}

impl ChannelRecord {
    /// The facts every group-state event publishes about this channel.
    pub fn identity(&self) -> ChannelIdentity {
        ChannelIdentity {
            channel_type: self.channel_type.clone(),
            labels: self.labels.clone(),
            created_by: self.created_by.clone(),
        }
    }
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
    labels: &[String],
    unlisted: bool,
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

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;

    sqlx::query(
        r#"
        INSERT INTO channels (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline, labels, unlisted)
        VALUES ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END, $9, $10)
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
    .bind(labels)
    .bind(unlisted)
    .execute(tx.conn())
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
    .execute(tx.conn())
    .await?;

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline, labels, join_role, unlisted
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(id)
    .fetch_one(tx.conn())
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
    labels: &[String],
) -> Result<(ChannelRecord, bool)> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;

    let was_created = insert_channel_rows(
        &mut tx,
        channel_id,
        NewChannel {
            name,
            channel_type,
            visibility,
            description,
            created_by,
            ttl_seconds,
            labels,
            unlisted: false,
            members: &[],
        },
    )
    .await?;

    let row = sqlx::query(
        r#"
        SELECT id, name, channel_type::text AS channel_type, visibility::text AS visibility,
               description, canvas,
               created_by, created_at, updated_at, archived_at, deleted_at,
               nip29_group_id, topic_required, max_members,
               topic, topic_set_by, topic_set_at,
               purpose, purpose_set_by, purpose_set_at,
               ttl_seconds, ttl_deadline, labels, join_role, unlisted
        FROM channels WHERE community_id = $1 AND id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?;

    let record = row_to_channel_record(row)?;
    tx.commit().await?;
    Ok((record, was_created))
}

/// A channel to create: the fields a kind:9007 sets.
pub struct NewChannel<'a> {
    /// Display name, canonicalized before storage.
    pub name: &'a str,
    /// Stream, forum, DM, …
    pub channel_type: ChannelType,
    /// Open or private.
    pub visibility: ChannelVisibility,
    /// The `about` text.
    pub description: Option<&'a str>,
    /// The creator's 32-byte pubkey; becomes the owner.
    pub created_by: &'a [u8],
    /// Ephemeral channel lifetime.
    pub ttl_seconds: Option<i32>,
    /// Labels validated by `buzz_core::channel::parse_channel_labels`.
    pub labels: &'a [String],
    /// Leave the channel out of listings for non-members from the start.
    pub unlisted: bool,
    /// 32-byte pubkeys added as ordinary members invited by the creator. The
    /// caller has authorized each addition.
    pub members: &'a [Vec<u8>],
}

/// What [`Db::create_channel_with_event`] did.
#[derive(Debug)]
pub enum ChannelCreate {
    /// The channel, its members and the create event committed together.
    Created(Box<StoredEvent>),
    /// The UUID was already taken and nothing was written. `resent` is true
    /// when the stored create that took it is this same event.
    Exists {
        /// This exact event is the channel's stored create.
        resent: bool,
    },
}

/// See [`Db::create_channel_with_event`].
pub(crate) async fn create_channel_with_event(
    pool: &PgPool,
    push_enqueue: Option<crate::runtime::push_enqueue::PushEnqueue>,
    community_id: CommunityId,
    channel_id: Uuid,
    channel: NewChannel<'_>,
    event: &nostr::Event,
) -> Result<ChannelCreate> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    tx.set_push_enqueue(push_enqueue);

    if !insert_channel_rows(&mut tx, channel_id, channel).await? {
        let resent = sqlx::query(
            "SELECT 1 FROM events WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_optional(tx.conn())
        .await?
        .is_some();
        return Ok(ChannelCreate::Exists { resent });
    }

    // The same writes as `Db::insert_event_with_thread_metadata`.
    let (stored, inserted) =
        crate::event::insert_event_with_thread_metadata_tx(&mut tx, event, Some(channel_id), None)
            .await?;
    if inserted {
        crate::insert_mentions_in_transaction(&mut tx, event, Some(channel_id)).await?;
    }
    tx.commit().await?;
    Ok(ChannelCreate::Created(Box::new(stored)))
}

/// Inserts the channel row, its owner and its initial members on `tx`.
/// Returns `false`, having written nothing, when `channel_id` already exists.
async fn insert_channel_rows(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    channel: NewChannel<'_>,
) -> Result<bool> {
    let NewChannel {
        name,
        channel_type,
        visibility,
        description,
        created_by,
        ttl_seconds,
        labels,
        unlisted,
        members,
    } = channel;
    let community_id = tx.community();
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

    let rows_affected = sqlx::query(
        r#"
        INSERT INTO channels (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline, labels, unlisted)
        VALUES ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8,
                CASE WHEN $8 IS NOT NULL THEN NOW() + ($8 || ' seconds')::interval ELSE NULL END, $9, $10)
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
    .bind(labels)
    .bind(unlisted)
    .execute(tx.conn())
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
        .execute(tx.conn())
        .await?;

        for member in members
            .iter()
            .filter(|member| member.as_slice() != created_by)
        {
            if member.len() != 32 {
                return Err(DbError::InvalidData(format!(
                    "pubkey must be 32 bytes, got {}",
                    member.len()
                )));
            }
            sqlx::query(
                r#"
                INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
                VALUES ($1, $2, $3, 'member', $4)
                ON CONFLICT (community_id, channel_id, pubkey) DO NOTHING
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(channel_id)
            .bind(member)
            .bind(created_by)
            .execute(tx.conn())
            .await?;
        }
    }

    Ok(was_created)
}

/// SQL that is true when `column` holds a legacy channel type
/// ([`buzz_core::channel::LEGACY_CHANNEL_TYPES`]): `<column> IN ('stream', …)`.
///
/// A query that a client did not ask to widen uses this, not
/// `<> 'system'`, so a channel type added later stays hidden (NIP-CL).
pub(crate) fn legacy_channel_type_sql(column: &str) -> String {
    let types = buzz_core::channel::LEGACY_CHANNEL_TYPES
        .iter()
        .map(|channel_type| format!("'{channel_type}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{column} IN ({types})")
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
               ttl_seconds, ttl_deadline, labels, join_role, unlisted
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let rows = sqlx::query(
        "UPDATE channels SET canvas = $1 WHERE community_id = $2 AND id = $3 AND deleted_at IS NULL",
    )
        .bind(canvas)
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(tx.conn())
        .await?;
    if rows.rows_affected() == 0 {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    tx.commit().await?;
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
                   ttl_seconds, ttl_deadline, labels, join_role, unlisted
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
                   ttl_seconds, ttl_deadline, labels, join_role, unlisted
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
        labels: row.try_get("labels")?,
        // Selected by every channel-record query. A missing column is an
        // error, never a silent `member`: the write gate depends on it.
        join_role: row.try_get("join_role")?,
        // Selected by every channel-record query, like `join_role`.
        unlisted: row.try_get("unlisted")?,
    })
}

/// The tags of a channel's relay-signed NIP-29 kind:39000 metadata event.
///
/// This is the single builder for kind:39000: the relay's discovery emitter
/// and the `buzz-admin` backfill both call it, so the published flags cannot
/// drift apart. `members` is used only for DM participant `p` tags.
///
/// The NIP-29 access flags follow the channel's settings:
/// - `private` (members-only read), or Buzz's explicit `public`;
/// - `restricted` (only a writing role may write) for private channels and
///   for open channels with `join_role = guest`; an unknown `join_role` fails
///   closed to `guest`;
/// - `closed` (join requests ignored) only when kind:9021 cannot self-join,
///   which is every non-open channel.
///
/// Buzz also publishes the explicit `["join_role", ...]` value, so clients
/// can tell what joining gives (show Join or Follow) without inferring it
/// from the flags, and a bare `["unlisted"]` flag when the channel is
/// unlisted. The flag is published as stored, whatever the visibility, so
/// clients see the setting; it only changes listings while the channel is
/// open.
pub fn group_metadata_tags(channel: &ChannelRecord, members: &[MemberRecord]) -> Vec<Vec<String>> {
    let tag = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let mut tags = vec![
        tag(&["d", &channel.id.to_string()]),
        tag(&["name", &channel.name]),
    ];
    if let Some(desc) = channel.description.as_deref().filter(|d| !d.is_empty()) {
        tags.push(tag(&["about", desc]));
    }
    let open = channel.visibility == ChannelVisibility::Open.as_str();
    if open {
        // Explicit "public" tag complements NIP-29's absence-of-"private"
        // convention, making channel visibility self-describing for clients.
        tags.push(tag(&["public"]));
    } else {
        tags.push(tag(&["private"]));
    }
    if channel.unlisted {
        tags.push(tag(&["unlisted"]));
    }
    let join_role = ChannelJoinRole::parse_fail_closed(&channel.join_role);
    if !open || join_role == ChannelJoinRole::Guest {
        tags.push(tag(&["restricted"]));
    }
    // NIP-29 hidden tag: hide metadata from non-members. Set for DMs and
    // for private system channels. Not a security boundary — access control
    // is handled by channel-scoped storage.
    if channel.channel_type == ChannelType::System.as_str() && !open {
        tags.push(tag(&["hidden"]));
    }
    if channel.channel_type == ChannelType::Dm.as_str() {
        tags.push(tag(&["hidden"]));
        // Include participant pubkeys in kind:39000 for DMs so clients can
        // resolve display names without a separate kind:39002 fetch.
        for m in members {
            tags.push(tag(&["p", &hex::encode(&m.pubkey)]));
        }
    }
    if !open {
        tags.push(tag(&["closed"]));
    }
    // Channel type tag so clients can distinguish stream/forum/dm without
    // inference, then one `t` per label, then the creator. The same tags lead
    // 39001 and 39002 (see `buzz_core::channel::group_state_identity_tags`).
    tags.push(tag(&["t", &channel.channel_type]));
    for label in &channel.labels {
        tags.push(tag(&["t", label]));
    }
    tags.push(tag(&["P", &hex::encode(&channel.created_by)]));
    // Creation time, so readers can pick the oldest of two channels. The
    // event's own created_at changes on every edit.
    tags.push(tag(&[
        buzz_core::channel::CHANNEL_CREATED_AT_TAG,
        &channel.created_at.timestamp().to_string(),
    ]));
    tags.push(tag(&["join_role", join_role.as_str()]));
    // Optional topic / purpose for richer client UX
    if let Some(topic) = channel.topic.as_deref().filter(|t| !t.is_empty()) {
        tags.push(tag(&["topic", topic]));
    }
    if let Some(purpose) = channel.purpose.as_deref().filter(|p| !p.is_empty()) {
        tags.push(tag(&["purpose", purpose]));
    }
    // Archived state — clients use this to hide channels from the sidebar.
    if channel.archived_at.is_some() {
        tags.push(tag(&["archived", "true"]));
    }
    // Ephemeral channel TTL — clients use this to show countdown timers.
    if let Some(ttl) = channel.ttl_seconds {
        tags.push(tag(&["ttl", &ttl.to_string()]));
    }
    if let Some(deadline) = channel.ttl_deadline {
        tags.push(tag(&["ttl_deadline", &deadline.to_rfc3339()]));
    }
    tags
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
    /// Replacement label set (empty clears it), or `None` to leave unchanged.
    /// Callers validate with `buzz_core::channel::parse_channel_labels`.
    pub labels: Option<Vec<String>>,
    /// New join role (`"member"`/`"guest"`), or `None` to leave unchanged.
    pub join_role: Option<String>,
    /// New unlisted flag, or `None` to leave unchanged. It decides listing,
    /// not admission, so it does not take the membership lock.
    pub unlisted: Option<bool>,
}

impl ChannelUpdate {
    /// True when no field is set.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.visibility.is_none()
            && self.ttl_seconds.is_none()
            && self.labels.is_none()
            && self.join_role.is_none()
            && self.unlisted.is_none()
    }

    /// True when the update changes a setting that decides admission:
    /// visibility or `join_role`.
    pub fn changes_admission(&self) -> bool {
        self.visibility.is_some() || self.join_role.is_some()
    }
}

/// Updates channel metadata dynamically.
///
/// At least one field must be provided; returns `InvalidData` otherwise.
/// Returns the updated `ChannelRecord` on success.
pub async fn update_channel(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    updates: ChannelUpdate,
) -> Result<ChannelRecord> {
    if updates.is_empty() {
        return Err(DbError::InvalidData(
            "at least one field must be provided for update".to_string(),
        ));
    }

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked = lock_channel_for_settings_write(&mut tx, channel_id, &updates).await?;
    update_channel_on(&mut locked, updates).await?;
    tx.commit().await?;

    get_channel_with_operation(
        pool,
        community_id,
        channel_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
}

#[derive(Clone, Copy)]
enum TtlLock {
    Shared,
    Exclusive,
}

/// Takes the per-channel TTL advisory lock (see [`channel_ttl_lock_key`]) on `tx`.
async fn lock_channel_ttl(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    mode: TtlLock,
) -> Result<()> {
    let sql = match mode {
        TtlLock::Shared => "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))",
        TtlLock::Exclusive => "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
    };
    sqlx::query(sql)
        .bind(channel_ttl_lock_key(tx.community(), channel_id))
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// The only way to write a channel's settings. Only
/// [`lock_channel_for_settings_write`] makes one, after it takes the locks
/// on the transaction it borrows, and every settings write is a method on
/// it. The borrow binds the locks to that transaction: a writer cannot use
/// them with another transaction or after its commit, skip a lock, or take
/// the locks in a different order.
pub(crate) struct ChannelSettingsWriter<'tx> {
    tx: &'tx mut crate::AdmittedTx,
    channel_id: Uuid,
    admission: bool,
    ttl: TtlLock,
    previous_visibility: String,
    archived: bool,
}

impl ChannelSettingsWriter<'_> {
    /// The channel's visibility when the lock was taken (`"open"` or `"private"`).
    pub(crate) fn previous_visibility(&self) -> &str {
        &self.previous_visibility
    }

    /// Whether the channel was archived when the lock was taken. The row
    /// lock keeps this current until the transaction ends.
    pub(crate) fn archived(&self) -> bool {
        self.archived
    }

    /// The locked transaction, for writes that are not channel settings
    /// (for example, storing the event that requested the change).
    pub(crate) fn tx(&mut self) -> &mut crate::AdmittedTx {
        self.tx
    }

    /// Refuse `updates` if this lock does not cover them. This catches a
    /// caller that took the lock for one set of changes and then writes
    /// another set.
    fn check_covers(&self, updates: &ChannelUpdate) -> Result<()> {
        let covered = (self.admission || !updates.changes_admission())
            && (matches!(self.ttl, TtlLock::Exclusive) || updates.ttl_seconds.is_none());
        if covered {
            Ok(())
        } else {
            Err(DbError::InvalidData(
                "channel settings write is not covered by its lock".into(),
            ))
        }
    }
}

/// Takes, on `tx`, every lock a write to `channel_id`'s settings needs, in
/// the one order that all channel writers use:
///
/// 1. the channel's membership lock, when `updates` changes visibility or
///    `join_role`. Those two settings decide admission, and `put_member` and
///    a huddle join decide admission under this lock. So an admission either
///    sees the old setting and commits first, or sees the new one.
/// 2. the channel's TTL lock: EXCLUSIVE for a TTL change, SHARED otherwise.
///    A TTL change can switch the event-commit fast path
///    (`event_follow_up::refresh_channel_ttls`, which reads `ttl_seconds`
///    under the SHARED lock). With the EXCLUSIVE lock, a concurrent event
///    either sees the new TTL or commits before this change.
/// 3. the channel row, `FOR NO KEY UPDATE`.
///
/// Roster publication holds the membership lock and then, at commit, takes
/// the TTL lock and updates the row. A huddle join takes the membership
/// lock, then the TTL lock, then the row. A writer that took these locks in
/// any other order could wait for one of them while it waits for this one.
///
/// Returns `ChannelNotFound` when the channel is missing or deleted.
pub(crate) async fn lock_channel_for_settings_write<'tx>(
    tx: &'tx mut crate::AdmittedTx,
    channel_id: Uuid,
    updates: &ChannelUpdate,
) -> Result<ChannelSettingsWriter<'tx>> {
    let ttl = if updates.ttl_seconds.is_some() {
        TtlLock::Exclusive
    } else {
        TtlLock::Shared
    };
    lock_channel_row(tx, channel_id, updates.changes_admission(), ttl).await
}

/// Takes the locks that deleting `channel_id` needs, in the same order as
/// [`lock_channel_for_settings_write`]. A delete ends admission, so it takes
/// the membership lock: a membership write or a huddle join either commits
/// first or, under the lock, finds the channel deleted. A delete does not
/// change the TTL, so the TTL lock is SHARED.
async fn lock_channel_for_delete(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
) -> Result<ChannelSettingsWriter<'_>> {
    lock_channel_row(tx, channel_id, true, TtlLock::Shared).await
}

async fn lock_channel_row(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    admission: bool,
    ttl: TtlLock,
) -> Result<ChannelSettingsWriter<'_>> {
    let community_id = tx.community();
    if admission {
        crate::channel_members::acquire_channel_membership_lock(
            tx.conn(),
            community_id,
            channel_id,
        )
        .await?;
    }
    lock_channel_ttl(tx, channel_id, ttl).await?;
    // `FOR NO KEY UPDATE`, like a huddle join: it orders this write against
    // every other settings write and a huddle join, but not against the
    // `KEY SHARE` lock that a membership INSERT takes for its foreign key.
    let (previous_visibility, archived): (String, bool) = sqlx::query_as(
        "SELECT visibility::text, archived_at IS NOT NULL FROM channels \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL FOR NO KEY UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?
    .ok_or(DbError::ChannelNotFound(channel_id))?;
    Ok(ChannelSettingsWriter {
        tx,
        channel_id,
        admission,
        ttl,
        previous_visibility,
        archived,
    })
}

/// Writes the set fields of `updates` to the locked channel row on `tx`.
/// The caller owns the commit.
async fn update_channel_on(
    w: &mut ChannelSettingsWriter<'_>,
    mut updates: ChannelUpdate,
) -> Result<()> {
    w.check_covers(&updates)?;
    let channel_id = w.channel_id;
    let tx = &mut *w.tx;
    let community_id = tx.community();
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
    if updates.join_role.is_some() {
        set_parts.push(format!("join_role = ${param_idx}"));
        param_idx += 1;
    }
    if updates.unlisted.is_some() {
        set_parts.push(format!("unlisted = ${param_idx}"));
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
    if updates.labels.is_some() {
        set_parts.push(format!("labels = ${param_idx}"));
        param_idx += 1;
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
    if let Some(ref join_role) = updates.join_role {
        q = q.bind(join_role);
    }
    if let Some(unlisted) = updates.unlisted {
        q = q.bind(unlisted);
    }
    if let Some(ref ttl) = updates.ttl_seconds {
        q = q.bind(*ttl);
    }
    if let Some(ref labels) = updates.labels {
        q = q.bind(labels);
    }
    q = q.bind(community_id.as_uuid());
    q = q.bind(channel_id);

    let result = q.execute(tx.conn()).await?;
    if result.rows_affected() == 0 {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    Ok(())
}

/// Sets the topic for a channel, recording who set it and when.
pub async fn set_topic(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    topic: &str,
    set_by: &[u8],
) -> Result<()> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked =
        lock_channel_for_settings_write(&mut tx, channel_id, &ChannelUpdate::default()).await?;
    set_topic_on(&mut locked, topic, set_by).await?;
    tx.commit().await?;
    Ok(())
}

async fn set_topic_on(w: &mut ChannelSettingsWriter<'_>, topic: &str, set_by: &[u8]) -> Result<()> {
    let channel_id = w.channel_id;
    let tx = &mut *w.tx;
    let result = sqlx::query(
        "UPDATE channels SET topic = $1, topic_set_by = $2, topic_set_at = NOW() \
         WHERE community_id = $3 AND id = $4 AND deleted_at IS NULL",
    )
    .bind(topic)
    .bind(set_by)
    .bind(tx.community().as_uuid())
    .bind(channel_id)
    .execute(tx.conn())
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked =
        lock_channel_for_settings_write(&mut tx, channel_id, &ChannelUpdate::default()).await?;
    set_purpose_on(&mut locked, purpose, set_by).await?;
    tx.commit().await?;
    Ok(())
}

async fn set_purpose_on(
    w: &mut ChannelSettingsWriter<'_>,
    purpose: &str,
    set_by: &[u8],
) -> Result<()> {
    let channel_id = w.channel_id;
    let tx = &mut *w.tx;
    let result = sqlx::query(
        "UPDATE channels SET purpose = $1, purpose_set_by = $2, purpose_set_at = NOW() \
         WHERE community_id = $3 AND id = $4 AND deleted_at IS NULL",
    )
    .bind(purpose)
    .bind(set_by)
    .bind(tx.community().as_uuid())
    .bind(channel_id)
    .execute(tx.conn())
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked =
        lock_channel_for_settings_write(&mut tx, channel_id, &ChannelUpdate::default()).await?;
    set_archived_on(&mut locked, true).await?;
    tx.commit().await?;
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked =
        lock_channel_for_settings_write(&mut tx, channel_id, &ChannelUpdate::default()).await?;
    set_archived_on(&mut locked, false).await?;
    tx.commit().await?;
    Ok(())
}

/// Archives (`archive = true`) or restores the channel on `tx`. Refuses with
/// `AccessDenied` when the channel is already in that state. Restoring an
/// ephemeral channel restarts its TTL deadline.
async fn set_archived_on(w: &mut ChannelSettingsWriter<'_>, archive: bool) -> Result<()> {
    let channel_id = w.channel_id;
    let tx = &mut *w.tx;
    let community_id = tx.community();
    let row = sqlx::query(
        "SELECT archived_at FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(tx.conn())
    .await?;

    match row {
        None => return Err(DbError::ChannelNotFound(channel_id)),
        Some(r) => {
            let archived_at: Option<DateTime<Utc>> = r.try_get("archived_at")?;
            if archive && archived_at.is_some() {
                return Err(DbError::AccessDenied(
                    "channel is already archived".to_string(),
                ));
            }
            if !archive && archived_at.is_none() {
                return Err(DbError::AccessDenied("channel is not archived".to_string()));
            }
        }
    }

    let sql = if archive {
        "UPDATE channels SET archived_at = NOW() \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL AND archived_at IS NULL"
    } else {
        "UPDATE channels SET archived_at = NULL, \
             ttl_deadline = CASE \
                 WHEN ttl_seconds IS NOT NULL THEN NOW() + (ttl_seconds || ' seconds')::interval \
                 ELSE ttl_deadline \
             END \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL AND archived_at IS NOT NULL"
    };
    sqlx::query(sql)
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// The channel changes named by one kind:9002 event. `None` leaves a value
/// unchanged.
#[derive(Default)]
pub struct ChannelMetadataEdit {
    /// Name, description, visibility, TTL and labels.
    pub columns: ChannelUpdate,
    /// New topic, recorded as set by the event's author.
    pub topic: Option<String>,
    /// New purpose, recorded as set by the event's author.
    pub purpose: Option<String>,
    /// `Some(true)` archives the channel; `Some(false)` restores it.
    pub archived: Option<bool>,
}

/// The result of [`Db::apply_channel_metadata_event`].
pub struct ChannelMetadataWrite {
    /// The stored kind:9002 event.
    pub stored: StoredEvent,
    /// `false` when the event was already stored. Nothing was changed then:
    /// its changes committed with it the first time.
    pub inserted: bool,
    /// The channel's visibility before this event (`"open"` or `"private"`).
    pub previous_visibility: String,
}

pub(crate) async fn apply_channel_metadata_event(
    pool: &PgPool,
    push_enqueue: Option<crate::runtime::push_enqueue::PushEnqueue>,
    community_id: CommunityId,
    channel_id: Uuid,
    event: &nostr::Event,
    edit: ChannelMetadataEdit,
    system_messages: &[nostr::Event],
) -> Result<ChannelMetadataWrite> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    tx.set_push_enqueue(push_enqueue);

    // The row lock also orders two copies of the same event: the second
    // waits, then sees the first stored.
    let mut locked = lock_channel_for_settings_write(&mut tx, channel_id, &edit.columns).await?;
    let previous_visibility = locked.previous_visibility().to_owned();

    // The same writes as `Db::insert_event_with_thread_metadata`.
    let (stored, inserted) = crate::event::insert_event_with_thread_metadata_tx(
        locked.tx(),
        event,
        Some(channel_id),
        None,
    )
    .await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(ChannelMetadataWrite {
            stored,
            inserted,
            previous_visibility,
        });
    }
    // Decide the archive state here, after the duplicate check and under
    // the row lock, not during validation. A resend that races the first
    // commit then sees that commit as a duplicate (above), never as a
    // channel already in the requested state.
    match (locked.archived(), edit.archived) {
        (true, Some(false)) | (false, None | Some(true)) => {}
        (true, _) => return Err(DbError::AccessDenied("channel is archived".to_string())),
        (false, Some(false)) => {
            return Err(DbError::AccessDenied("channel is not archived".to_string()));
        }
    }
    crate::insert_mentions_in_transaction(locked.tx(), event, Some(channel_id)).await?;

    let ChannelMetadataEdit {
        columns,
        topic,
        purpose,
        archived,
    } = edit;
    let actor = event.pubkey.to_bytes();
    if !columns.is_empty() {
        update_channel_on(&mut locked, columns).await?;
    }
    if let Some(topic) = topic {
        set_topic_on(&mut locked, &topic, &actor).await?;
    }
    if let Some(purpose) = purpose {
        set_purpose_on(&mut locked, &purpose, &actor).await?;
    }
    if let Some(archive) = archived {
        set_archived_on(&mut locked, archive).await?;
    }
    // The same writes as `Db::insert_event`.
    for message in system_messages {
        let (_, message_inserted) =
            crate::event::insert_event_in_transaction(locked.tx(), message, Some(channel_id))
                .await?;
        if message_inserted {
            crate::insert_mentions_in_transaction(locked.tx(), message, Some(channel_id)).await?;
        }
    }
    tx.commit().await?;
    Ok(ChannelMetadataWrite {
        stored,
        inserted,
        previous_visibility,
    })
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
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut writer = match lock_channel_for_delete(&mut tx, channel_id).await {
        Ok(writer) => writer,
        Err(DbError::ChannelNotFound(_)) => return Ok(false),
        Err(error) => return Err(error),
    };
    soft_delete_on(&mut writer).await?;
    tx.commit().await?;
    Ok(true)
}

async fn soft_delete_on(w: &mut ChannelSettingsWriter<'_>) -> Result<()> {
    if !w.admission {
        return Err(DbError::InvalidData(
            "channel delete is not covered by its lock".into(),
        ));
    }
    let channel_id = w.channel_id;
    let community_id = w.tx.community();
    sqlx::query("UPDATE channels SET deleted_at = NOW() WHERE community_id = $1 AND id = $2")
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(w.tx.conn())
        .await?;
    Ok(())
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
    let communities: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT ch.community_id FROM channels AS ch \
         JOIN communities AS c ON ch.community_id = c.id \
         WHERE ch.ttl_seconds IS NOT NULL \
           AND ch.ttl_deadline < NOW() \
           AND ch.archived_at IS NULL \
           AND ch.deleted_at IS NULL \
           AND c.archived_at IS NULL \
         ORDER BY ch.community_id",
    )
    .fetch_all(&mut *connection)
    .await?;
    drop(connection);
    let reaped = crate::sweep_admitted_communities(
        pool,
        communities,
        crate::observability::WriterOperation::Maintenance,
        "ephemeral_channel_reap",
        |id, mut tx| async move {
            // This archive takes no advisory lock, unlike
            // `lock_channel_for_settings_write`. It does not need one: it is
            // a single UPDATE whose WHERE clause holds every condition for
            // the archive. The UPDATE waits for the row lock of any writer
            // that holds the row (a TTL refresh, a settings write, a huddle
            // join). Under READ COMMITTED, PostgreSQL then checks the WHERE
            // clause again against the version that writer committed. So a
            // refresh that moved the deadline wins, a channel archived or
            // deleted meanwhile is skipped, and a join that held the row
            // commits first. Do not split this into a read and a write.
            let rows = sqlx::query(
                "UPDATE channels AS ch SET archived_at = NOW() \
                 FROM communities AS c \
                 WHERE ch.community_id = $1 AND ch.community_id = c.id \
                   AND ch.ttl_seconds IS NOT NULL AND ch.ttl_deadline < NOW() \
                   AND ch.archived_at IS NULL AND ch.deleted_at IS NULL \
                   AND c.archived_at IS NULL \
                 RETURNING ch.community_id, c.host, ch.id",
            )
            .bind(id)
            .fetch_all(tx.conn())
            .await?;
            let parsed = rows
                .into_iter()
                .map(|row| {
                    Ok(ReapedEphemeralChannel {
                        community_id: CommunityId::from_uuid(row.try_get("community_id")?),
                        host: row.try_get("host")?,
                        channel_id: row.try_get("id")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            tx.commit().await?;
            Ok(parsed)
        },
    )
    .await;
    Ok(reaped.into_iter().flatten().collect())
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
            &[],
            false,
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
            &[],
        )
        .await
    }

    /// Creates `channel_id` from its kind:9007 `event`: the channel, its
    /// owner, its initial members and the event itself commit in one
    /// transaction, so a channel row always has its create event beside it.
    ///
    /// A concurrent copy of the same create waits on the first one's channel
    /// insert, then sees either the committed event (`Exists { resent: true }`)
    /// or, if the first rolled back, an untaken UUID.
    #[datastore_span(name = "create_channel_with_event", system = "postgresql")]
    pub async fn create_channel_with_event(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        channel: NewChannel<'_>,
        event: &nostr::Event,
    ) -> Result<ChannelCreate> {
        create_channel_with_event(
            &self.pool,
            self.push_enqueue.clone(),
            community_id,
            channel_id,
            channel,
            event,
        )
        .await
    }

    /// Stores the kind:9002 `event` for `channel_id`, applies `edit`, and
    /// stores the relay-signed `system_messages` that announce it, all in one
    /// transaction. If any write fails, nothing is stored, so a client can
    /// resend the same event. A resend of a stored event changes nothing and
    /// returns `inserted: false`.
    #[datastore_span(name = "apply_channel_metadata_event", system = "postgresql")]
    pub async fn apply_channel_metadata_event(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        event: &nostr::Event,
        edit: ChannelMetadataEdit,
        system_messages: &[nostr::Event],
    ) -> Result<ChannelMetadataWrite> {
        apply_channel_metadata_event(
            &self.pool,
            self.push_enqueue.clone(),
            community_id,
            channel_id,
            event,
            edit,
            system_messages,
        )
        .await
    }

    /// Creates a channel with a relay-chosen UUID, an initial label set and
    /// an initial unlisted flag.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "create_labeled_channel", system = "postgresql")]
    pub async fn create_labeled_channel(
        &self,
        community_id: CommunityId,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        description: Option<&str>,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
        labels: &[String],
        unlisted: bool,
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
            labels,
            unlisted,
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

/// The per-channel TTL advisory lock key. Event commits take it SHARED
/// (`event_follow_up::refresh_channel_ttls`); TTL transitions in
/// [`update_channel`] and [`apply_channel_metadata_event`] take it EXCLUSIVE. Both sides build it here, so the keys
/// cannot drift.
pub(crate) fn channel_ttl_lock_key(community: CommunityId, channel: Uuid) -> String {
    format!("buzz_channel_ttl:{}:{channel}", community.as_uuid())
}

#[cfg(test)]
mod group_metadata_tag_tests {
    use super::*;

    fn channel(visibility: &str, join_role: &str) -> ChannelRecord {
        let now = Utc::now();
        ChannelRecord {
            id: Uuid::nil(),
            name: "news".into(),
            channel_type: "stream".into(),
            visibility: visibility.into(),
            description: None,
            canvas: None,
            created_by: vec![1; 32],
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
            nip29_group_id: None,
            topic_required: false,
            max_members: None,
            topic: None,
            topic_set_by: None,
            topic_set_at: None,
            purpose: None,
            purpose_set_by: None,
            purpose_set_at: None,
            ttl_seconds: None,
            ttl_deadline: None,
            labels: Vec::new(),
            join_role: join_role.into(),
            unlisted: false,
        }
    }

    fn flags(channel: &ChannelRecord) -> Vec<&'static str> {
        let tags = group_metadata_tags(channel, &[]);
        ["private", "public", "restricted", "closed"]
            .into_iter()
            .filter(|flag| tags.iter().any(|t| t == &vec![flag.to_string()]))
            .collect()
    }

    /// The NIP-29 access flags follow visibility and join_role. Open channels
    /// never publish `closed`: kind:9021 self-join works there.
    #[test]
    fn access_flags_follow_settings() {
        let cases: &[(&str, &str, &[&str])] = &[
            ("open", "member", &["public"]),
            ("open", "guest", &["public", "restricted"]),
            ("private", "member", &["private", "restricted", "closed"]),
            ("private", "guest", &["private", "restricted", "closed"]),
            // An unknown join_role value fails closed to `guest`.
            ("open", "bogus", &["public", "restricted"]),
        ];
        for (visibility, join_role, expected) in cases {
            assert_eq!(
                flags(&channel(visibility, join_role)),
                *expected,
                "{visibility}/{join_role}"
            );
        }
    }

    #[test]
    fn publishes_the_explicit_join_role_value() {
        let tags = group_metadata_tags(&channel("open", "guest"), &[]);
        assert!(tags.contains(&vec!["join_role".to_string(), "guest".to_string()]));
        let tags = group_metadata_tags(&channel("open", "member"), &[]);
        assert!(tags.contains(&vec!["join_role".to_string(), "member".to_string()]));
        // An unknown stored value is published as the value the relay enforces.
        let tags = group_metadata_tags(&channel("open", "bogus"), &[]);
        assert!(tags.contains(&vec!["join_role".to_string(), "guest".to_string()]));
    }

    /// The type `t` tag comes first among `t` tags, then the creator `P`,
    /// as on 39001 and 39002; kind:39000 also carries the creation time.
    #[test]
    fn publishes_the_type_creator_and_creation_time() {
        let channel = channel("open", "member");
        let tags = group_metadata_tags(&channel, &[]);
        let at = |name: &str| tags.iter().position(|t| t[0] == name).expect(name);
        assert_eq!(tags[at("t")], vec!["t", "stream"]);
        assert_eq!(at("P"), at("t") + 1);
        assert_eq!(tags[at("P")], vec!["P".to_string(), "01".repeat(32)]);
        assert_eq!(
            tags[at("created_at")],
            vec![
                "created_at".to_string(),
                channel.created_at.timestamp().to_string()
            ]
        );
    }

    /// Labels follow the type `t` tag and come before the creator `P`, the
    /// same order as `buzz_core::channel::group_state_identity_tags`.
    #[test]
    fn publishes_labels_between_the_type_and_the_creator() {
        let mut channel = channel("open", "member");
        channel.labels = vec!["workspace".into(), "agent-attention".into()];
        let tags = group_metadata_tags(&channel, &[]);
        let identity: Vec<&Vec<String>> =
            tags.iter().filter(|t| t[0] == "t" || t[0] == "P").collect();
        let expected = buzz_core::channel::group_state_identity_tags(
            &channel.channel_type,
            &channel.labels,
            &channel.created_by,
        )
        .expect("identity tags")
        .into_iter()
        .map(|tag| tag.to_vec())
        .collect::<Vec<_>>();
        assert_eq!(identity, expected.iter().collect::<Vec<_>>());
    }

    /// `unlisted` is a bare flag, published as stored for either visibility,
    /// and absent for a listed channel. It never adds NIP-29 `hidden`: an
    /// unlisted channel's metadata stays readable by id.
    #[test]
    fn publishes_the_unlisted_flag_as_stored() {
        let unlisted = vec!["unlisted".to_string()];
        let hidden = vec!["hidden".to_string()];
        for visibility in ["open", "private"] {
            let mut channel = channel(visibility, "member");
            assert!(!group_metadata_tags(&channel, &[]).contains(&unlisted));
            channel.unlisted = true;
            let tags = group_metadata_tags(&channel, &[]);
            assert!(tags.contains(&unlisted), "{visibility}");
            assert!(!tags.contains(&hidden), "{visibility}");
        }
    }

    /// Private system channels are hidden, like DMs; open ones are not.
    #[test]
    fn hides_private_system_channels() {
        let hidden = |visibility: &str| {
            let mut channel = channel(visibility, "member");
            channel.channel_type = "system".into();
            group_metadata_tags(&channel, &[]).contains(&vec!["hidden".to_string()])
        };
        assert!(hidden("private"));
        assert!(!hidden("open"));
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
    async fn join_role_defaults_to_member_and_updates_round_trip() {
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
            "test-join-role-round-trip",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner_pk,
            None,
        )
        .await
        .expect("create channel");
        assert_eq!(channel.join_role, "member");

        let updated = update_channel(
            &pool,
            community,
            channel.id,
            ChannelUpdate {
                join_role: Some("guest".into()),
                ..Default::default()
            },
        )
        .await
        .expect("set join_role");
        assert_eq!(updated.join_role, "guest");
        assert_eq!(
            get_channel(&pool, community, channel.id)
                .await
                .expect("reload")
                .join_role,
            "guest"
        );

        // The column CHECK rejects values the relay does not know.
        assert!(update_channel(
            &pool,
            community,
            channel.id,
            ChannelUpdate {
                join_role: Some("admin".into()),
                ..Default::default()
            },
        )
        .await
        .is_err());
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

    fn create_event(creator: &Keys, channel_id: Uuid) -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::Custom(9007), "")
            .tags([
                nostr::Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
                nostr::Tag::parse(["name", "start"]).expect("name tag"),
            ])
            .sign_with_keys(creator)
            .expect("sign 9007")
    }

    fn new_channel<'a>(created_by: &'a [u8], members: &'a [Vec<u8>]) -> NewChannel<'a> {
        NewChannel {
            name: "start",
            channel_type: ChannelType::Stream,
            visibility: ChannelVisibility::Private,
            description: None,
            created_by,
            ttl_seconds: None,
            labels: &[],
            unlisted: false,
            members,
        }
    }

    /// Holds a create open on its own transaction (channel row and event both
    /// written, not committed) while the same create runs again beside it.
    async fn resend_beside_open_create(commit_first: bool) -> ChannelCreate {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let creator = Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        let members = vec![random_pubkey()];
        let channel_id = Uuid::new_v4();
        let event = create_event(&creator, channel_id);

        let mut first = crate::begin_community_write_transaction(
            &pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await
        .expect("begin first create");
        assert!(insert_channel_rows(
            &mut first,
            channel_id,
            new_channel(&creator_bytes, &members)
        )
        .await
        .expect("first create's rows"));
        crate::event::insert_event_with_thread_metadata_tx(
            &mut first,
            &event,
            Some(channel_id),
            None,
        )
        .await
        .expect("first create's event");

        let resend = tokio::spawn({
            let (pool, event, creator_bytes, members) = (
                pool.clone(),
                event.clone(),
                creator_bytes.clone(),
                members.clone(),
            );
            async move {
                create_channel_with_event(
                    &pool,
                    None,
                    community,
                    channel_id,
                    new_channel(&creator_bytes, &members),
                    &event,
                )
                .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !resend.is_finished(),
            "the resend waits for the open create instead of answering early"
        );

        if commit_first {
            first.commit().await.expect("commit first create");
        } else {
            drop(first);
        }
        let outcome = resend.await.expect("resend task").expect("resend outcome");
        let roster = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT pubkey FROM channel_members WHERE community_id = $1 AND channel_id = $2 \
             AND removed_at IS NULL ORDER BY pubkey",
        )
        .bind(community.as_uuid())
        .bind(channel_id)
        .fetch_all(&pool)
        .await
        .expect("roster");
        let mut expected = vec![creator_bytes, members[0].clone()];
        expected.sort();
        assert_eq!(
            roster, expected,
            "the channel has its owner and member once"
        );
        outcome
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_resend_beside_an_open_create_confirms_it_once_committed() {
        let outcome = resend_beside_open_create(true).await;
        assert!(
            matches!(outcome, ChannelCreate::Exists { resent: true }),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_resend_beside_a_failed_create_creates_the_channel() {
        let outcome = resend_beside_open_create(false).await;
        assert!(matches!(outcome, ChannelCreate::Created(_)), "{outcome:?}");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_create_whose_event_fails_leaves_no_channel() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let creator = Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        let channel_id = Uuid::new_v4();
        // An ephemeral kind is refused by the event insert, after the rows.
        let unstorable = nostr::EventBuilder::new(nostr::Kind::Custom(20_000), "")
            .sign_with_keys(&creator)
            .expect("sign");
        let members = [random_pubkey()];
        create_channel_with_event(
            &pool,
            None,
            community,
            channel_id,
            new_channel(&creator_bytes, &members),
            &unstorable,
        )
        .await
        .expect_err("the event insert fails");
        assert!(
            get_channel(&pool, community, channel_id).await.is_err(),
            "the channel rolls back with its event"
        );
        let retry = create_channel_with_event(
            &pool,
            None,
            community,
            channel_id,
            new_channel(&creator_bytes, &members),
            &create_event(&creator, channel_id),
        )
        .await
        .expect("retry");
        assert!(matches!(retry, ChannelCreate::Created(_)), "{retry:?}");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn another_create_for_a_taken_channel_writes_nothing() {
        let pool = setup_pool().await;
        let community = CommunityId::from_uuid(make_test_community(&pool).await);
        let creator = Keys::generate();
        let creator_bytes = creator.public_key().to_bytes().to_vec();
        let channel_id = Uuid::new_v4();
        let first = create_channel_with_event(
            &pool,
            None,
            community,
            channel_id,
            new_channel(&creator_bytes, &[]),
            &create_event(&creator, channel_id),
        )
        .await
        .expect("first create");
        assert!(matches!(first, ChannelCreate::Created(_)), "{first:?}");

        let other = Keys::generate();
        let other_bytes = other.public_key().to_bytes().to_vec();
        let intruder = random_pubkey();
        let second = create_channel_with_event(
            &pool,
            None,
            community,
            channel_id,
            new_channel(&other_bytes, std::slice::from_ref(&intruder)),
            &create_event(&other, channel_id),
        )
        .await
        .expect("second create");
        assert!(
            matches!(second, ChannelCreate::Exists { resent: false }),
            "{second:?}"
        );
        let members = crate::channel_members::get_members(&pool, community, channel_id)
            .await
            .expect("members");
        assert_eq!(members.len(), 1, "only the first creator is a member");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn soft_delete_waits_for_membership_lock() {
        let pool = setup_pool().await;
        let community_id = make_test_community(&pool).await;
        let community = CommunityId::from_uuid(community_id);
        let owner = random_pubkey();
        let channel = create_test_channel(
            &pool,
            community_id,
            "delete-vs-roster",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            &owner,
            None,
        )
        .await
        .expect("create channel");

        // A roster publication holds the membership lock. A delete ends
        // admission, so it must wait, like any membership write.
        let snapshot =
            crate::channel_members::lock_member_snapshot(&pool, community, channel.id, &[9u8; 32])
                .await
                .expect("capture locked roster");
        let delete_pool = crate::test_support::named_pool("delete-vs-roster").await;
        let channel_id = channel.id;
        let delete =
            tokio::spawn(
                async move { soft_delete_channel(&delete_pool, community, channel_id).await },
            );
        crate::test_support::wait_for_lock_wait(&pool, "delete-vs-roster").await;
        assert!(
            !delete.is_finished(),
            "delete must wait for the roster fence"
        );
        snapshot.release().await.expect("release roster fence");
        assert!(delete.await.expect("delete task").expect("delete"));

        assert!(
            !soft_delete_channel(&pool, community, channel.id)
                .await
                .expect("second delete"),
            "a second delete finds no live channel"
        );
    }
}
